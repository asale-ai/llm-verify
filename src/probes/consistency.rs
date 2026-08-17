// SPDX-License-Identifier: Apache-2.0
//! Cross-request consistency.
//!
//! One request shows what an endpoint *can* do. Repeating it shows whether the
//! same thing is answering every time. Two failure modes matter here: a pool
//! that round-robins across different backends, and a cache that replays a
//! stored answer instead of running the model at all.

use super::Ctx;
use crate::protocol::ChatRequest;
use crate::report::{Group, ProbeResult};
use crate::util::now_ms;
use std::collections::BTreeMap;

const G: Group = Group::Consistency;

/// A coarse signature of "which system answered", built only from structural
/// traits that must not vary between identical requests. Content is excluded
/// on purpose — models are allowed to word things differently.
/// Response time is deliberately excluded. Ordinary queueing moves a request
/// across any bucket boundary you pick, so including it reports normal jitter
/// as backend drift — the `jitter` probe already measures that properly.
fn signature(resp: &crate::protocol::ChatResponse, raw: &crate::client::RawResponse) -> String {
    format!(
        "model={}|stop={}|role={}|type={}|id_ok={}|req_id={}|input={}",
        resp.model,
        resp.stop_reason,
        resp.role,
        resp.object_type,
        !resp.id.is_empty(),
        raw.header("request-id")
            .map(|r| !r.is_empty())
            .unwrap_or(false),
        resp.usage.input_tokens,
    )
}

pub async fn signature_drift(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "signature_drift",
        ts!(l, "Cross-request signature drift", "跨请求签名漂移"),
        G,
    )
    .weight(2);
    let runs = ctx.depth.repeats().max(2);
    let t0 = now_ms();

    let req = ChatRequest::new(&ctx.client.endpoint.model, "Reply with the single word: OK")
        .max_tokens(16)
        .temperature(0.0);

    let mut sigs: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors = 0usize;
    for _ in 0..runs {
        match ctx.client.chat(&req).await {
            Ok((resp, raw)) => {
                ctx.observe(&raw, &resp.id);
                *sigs.entry(signature(&resp, &raw)).or_insert(0) += 1;
            }
            Err(_) => errors += 1,
        }
    }

    let took = (now_ms() - t0) as u64;
    let variants = sigs.len();
    let dominant = sigs.values().copied().max().unwrap_or(0);
    let drift = if runs > 0 {
        (runs - dominant) as f64 / runs as f64 * 100.0
    } else {
        0.0
    };

    let mut p = p
        .metric("runs", runs)
        .metric("variants", variants)
        .metric("drift_pct", (drift * 10.0).round() / 10.0)
        .metric("errors", errors);

    if variants > 1 {
        for (sig, n) in sigs.iter().take(4) {
            p = p.finding(format!("{n}× {}", crate::util::truncate(sig, 110)));
        }
    }

    if sigs.is_empty() {
        return p
            .error(t!(
                l,
                "Every consistency request failed",
                "所有一致性请求都失败了"
            ))
            .took(took);
    }
    if variants == 1 {
        p.pass(t!(
            l,
            "{runs} requests produced an identical signature",
            "{runs} 次请求签名完全一致"
        ))
        .took(took)
    } else if drift < 35.0 {
        p.warn(t!(
            l,
            "{variants} signature variants, {drift:.0}% drift",
            "{variants} 种签名变体，漂移 {drift:.0}%"
        ))
        .took(took)
    } else {
        p.fail(t!(l, "{variants} signature variants, {drift:.0}% drift", "{variants} 种签名变体，漂移 {drift:.0}%"))
            .finding(t!(l, "The same request returns structurally different responses; there is probably more than one backend", "同一请求得到结构不同的响应，后端很可能不止一个"))
            .took(took)
    }
}

pub async fn cache_replay(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "cache_replay",
        ts!(l, "Cache replay detection", "缓存回放检测"),
        G,
    )
    .weight(2);
    let runs = ctx.depth.repeats().max(2);
    let t0 = now_ms();

    // Temperature 1 with an open-ended prompt: identical text across runs is
    // not something a model does, but a cache does it every time.
    let nonce = ctx.rng_for("cache_replay").hex(6);
    let req = ChatRequest::new(
        &ctx.client.endpoint.model,
        &format!(
            "Invent one short surprising sentence about the number {nonce}. \
             Be creative and different each time."
        ),
    )
    .max_tokens(80)
    .temperature(1.0);

    let mut texts: Vec<String> = Vec::new();
    for _ in 0..runs {
        if let Ok((resp, raw)) = ctx.client.chat(&req).await {
            ctx.observe(&raw, &resp.id);
            texts.push(resp.text.trim().to_string());
        }
    }

    let took = (now_ms() - t0) as u64;
    if texts.len() < 2 {
        return p
            .skip(t!(
                l,
                "Only {} sample(s) collected; too few to compare",
                "只拿到 {} 个样本，不足以比较",
                texts.len()
            ))
            .took(took);
    }

    let mut uniq = texts.clone();
    uniq.sort();
    uniq.dedup();
    let all_identical = uniq.len() == 1;
    let p = p
        .metric("samples", texts.len())
        .metric("unique_responses", uniq.len())
        .evidence(crate::util::truncate(&texts[0], 200))
        .took(took);

    // A model at temperature 1 will repeat itself sometimes; that is sampling,
    // not caching. Only near-total collapse is evidence of a replayed answer.
    if all_identical && !texts[0].is_empty() {
        p.fail(t!(l, "{} answers at temperature 1 were word-for-word identical", "temperature=1 下 {} 次回答逐字相同", texts.len()))
            .finding(t!(l, "That is not model behaviour, it is a cache replaying — you may be paying for requests that never ran", "这不是模型的行为，是缓存在回放——你可能在为没有真正推理的请求付费"))
    } else if uniq.len() * 2 <= texts.len() {
        p.warn(t!(l, "Only {} distinct answers across {} samples; over half repeat", "{} 个样本里只有 {} 种不同回答，重复率过半",
            texts.len(),
            uniq.len()
        ))
        .finding(t!(l, "Not enough to call it a cache, but repetition runs higher than normal sampling at this temperature", "尚不足以断定是缓存，但重复率高于同温度下的正常采样"))
    } else {
        p.pass(t!(
            l,
            "{} samples produced {} distinct answers; generation is real",
            "{} 次采样得到 {} 种不同回答，确认是真实生成",
            texts.len(),
            uniq.len()
        ))
    }
}

pub async fn request_id_unique(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "id_unique",
        ts!(l, "Message ID uniqueness", "消息 ID 唯一性"),
        G,
    )
    .weight(1);
    let ids = ctx.message_ids.lock().unwrap().clone();
    let non_empty: Vec<&String> = ids.iter().filter(|i| !i.is_empty()).collect();

    if non_empty.len() < 2 {
        return p.skip(t!(
            l,
            "Fewer than two message IDs collected",
            "收集到的消息 ID 不足 2 个"
        ));
    }
    let mut uniq: Vec<&String> = non_empty.clone();
    uniq.sort();
    uniq.dedup();

    let p = p
        .metric("ids_seen", non_empty.len())
        .metric("unique_ids", uniq.len());

    if uniq.len() == non_empty.len() {
        p.pass(t!(
            l,
            "All {} message IDs are unique",
            "{} 个消息 ID 全部唯一",
            non_empty.len()
        ))
    } else {
        let dupes = non_empty.len() - uniq.len();
        p.fail(t!(l, "{dupes} of {} message IDs are duplicates", "{} 个消息 ID 中有 {dupes} 个重复", non_empty.len()))
            .finding(t!(l, "Duplicate message IDs mean responses are being replayed rather than generated each time", "重复的消息 ID 意味着响应被重放，而不是每次真实生成"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::RawResponse;
    use crate::protocol::{ChatResponse, Usage};

    fn resp(model: &str, stop: &str, input: u32) -> ChatResponse {
        ChatResponse {
            id: "msg_01x".into(),
            model: model.into(),
            role: "assistant".into(),
            object_type: "message".into(),
            text: "whatever".into(),
            stop_reason: stop.into(),
            usage: Usage {
                input_tokens: input,
                output_tokens: 3,
                present: true,
                ..Default::default()
            },
            tool_calls: vec![],
        }
    }

    fn raw(ms: u64) -> RawResponse {
        RawResponse {
            status: 200,
            headers: [("request-id".to_string(), "req_1".to_string())].into(),
            body: String::new(),
            duration_ms: ms,
        }
    }

    #[test]
    fn signature_ignores_wording_but_catches_structure() {
        let mut a = resp("m", "end_turn", 10);
        let mut b = a.clone();
        // Different generated text must not change the signature.
        a.text = "hello there".into();
        b.text = "hi".into();
        assert_eq!(signature(&a, &raw(600)), signature(&b, &raw(700)));

        // A different model behind the same request must change it.
        let c = resp("other-model", "end_turn", 10);
        assert_ne!(signature(&a, &raw(600)), signature(&c, &raw(600)));

        // So must a different stop reason or input accounting.
        let d = resp("m", "max_tokens", 10);
        assert_ne!(signature(&a, &raw(600)), signature(&d, &raw(600)));
        let e = resp("m", "end_turn", 999);
        assert_ne!(signature(&a, &raw(600)), signature(&e, &raw(600)));
    }

    #[test]
    fn response_time_never_affects_the_signature() {
        // Queueing routinely spans an order of magnitude on a healthy
        // endpoint; treating that as drift produced false positives.
        let r = resp("m", "end_turn", 10);
        assert_eq!(signature(&r, &raw(120)), signature(&r, &raw(9_000)));
    }
}
