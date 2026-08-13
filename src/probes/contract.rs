// SPDX-License-Identifier: Apache-2.0
//! Protocol contract probes — "is this a genuine API channel", independent of
//! which model sits behind it.
//!
//! Three of these work by *breaking* the request on purpose (drop the version
//! header, drop auth, name a model that cannot exist). A real upstream rejects
//! all three. An endpoint that answers anyway is either forwarding from a
//! shared pool or silently falling back to some other model.

use super::{Ctx, PerfSample};
use crate::client::RequestOpts;
use crate::protocol::{error_envelope_ok, ChatRequest, Protocol};
use crate::report::{Group, ProbeResult};
use crate::util::now_ms;
use serde_json::json;

const G: Group = Group::Contract;

/// A minimal, cheap request used wherever the content does not matter.
fn ping(ctx: &Ctx) -> ChatRequest {
    ChatRequest::new(&ctx.client.endpoint.model, "Reply with the single word: OK")
        .max_tokens(16)
        .temperature(0.0)
}

pub async fn preflight(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("preflight", "连通性预检", G).weight(3);
    let t0 = now_ms();
    let req = ping(ctx);

    let raw = match ctx
        .client
        .post_raw(
            ctx.client.endpoint.protocol.chat_path(),
            &req.to_body(ctx.client.endpoint.protocol),
            &RequestOpts::default(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            *ctx.reachable.borrow_mut() = false;
            return p
                .error(format!("无法连接：{e}"))
                .finding("后续探针已全部跳过——连不上时其它结论都没有意义")
                .took((now_ms() - t0) as u64);
        }
    };

    let took = (now_ms() - t0) as u64;
    let body_excerpt = crate::util::truncate(raw.body.trim(), 300);

    // Auth and model-name errors are terminal: every later probe would just
    // re-report the same thing and burn quota doing it.
    match raw.status {
        401 | 403 => {
            *ctx.reachable.borrow_mut() = false;
            return p
                .fail(format!("鉴权失败（HTTP {}）", raw.status))
                .finding("API Key 无效或没有该模型的权限")
                .evidence(body_excerpt)
                .took(took);
        }
        404 => {
            *ctx.reachable.borrow_mut() = false;
            return p
                .fail("HTTP 404：端点路径不存在")
                .finding(format!(
                    "实际请求的是 {}，确认 --base-url 是否需要带 /v1",
                    ctx.client
                        .endpoint
                        .url(ctx.client.endpoint.protocol.chat_path())
                ))
                .evidence(body_excerpt)
                .took(took);
        }
        s if s == 400 && raw.body.contains("model") => {
            *ctx.reachable.borrow_mut() = false;
            return p
                .fail("模型不存在或不被该端点接受")
                .finding(format!("请求的模型：{}", ctx.client.endpoint.model))
                .evidence(body_excerpt)
                .took(took);
        }
        429 => {
            return p
                .warn("HTTP 429：被限流，结果可能不完整")
                .evidence(body_excerpt)
                .took(took);
        }
        s if !(200..300).contains(&s) => {
            *ctx.reachable.borrow_mut() = false;
            return p
                .fail(format!("HTTP {s}"))
                .evidence(body_excerpt)
                .took(took);
        }
        _ => {}
    }

    ctx.observe(&raw, "");
    p.pass(format!("端点可达，{}ms", raw.duration_ms))
        .metric("status", raw.status)
        .metric("duration_ms", raw.duration_ms)
        .took(took)
}

/// Does the endpoint's own catalogue list the model it just served?
pub async fn model_catalog(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("model_catalog", "模型目录核验", G).weight(1);
    let t0 = now_ms();
    let models = match ctx.client.list_models().await {
        Ok(m) => m,
        Err(e) => {
            // Plenty of legitimate relays do not expose /models. Absent is not
            // guilty; it is simply one fewer corroborating signal.
            return p
                .skip(format!(
                    "/models 不可用：{}",
                    crate::util::truncate(&format!("{e}"), 80)
                ))
                .took((now_ms() - t0) as u64);
        }
    };
    let took = (now_ms() - t0) as u64;
    let target = &ctx.client.endpoint.model;
    let listed = models.iter().any(|m| models_equivalent(target, m));
    let p = p
        .metric("catalog_size", models.len())
        .metric("target_listed", listed);

    if models.is_empty() {
        p.warn("/models 返回了空目录").took(took)
    } else if listed {
        p.pass(format!("目录含 {} 个模型，包含目标模型", models.len()))
            .took(took)
    } else {
        p.warn(format!("目录里没有 {target}，但请求却成功了"))
            .finding("目录与实际可用模型不一致，可能是手工拼装的模型列表")
            .finding(format!(
                "目录示例：{}",
                models
                    .iter()
                    .take(6)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .took(took)
    }
}

/// Does a system prompt actually reach the model, or does a middle layer
/// overwrite it with its own?
///
/// The probe carries a run-unique fact *inside* the system prompt and then
/// asks for it back. That makes the result unfakeable in the useful direction:
/// a model that never received the system prompt cannot possibly produce the
/// token. Crucially it is also a benign, natural instruction — an earlier
/// version told the model to ignore the user's question entirely, and stronger
/// models correctly refused that as adversarial, which the probe then misread
/// as the middleware dropping the prompt.
pub async fn system_adherence(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("system_adherence", "System Prompt 生效", G).weight(2);
    let t0 = now_ms();
    let token = ctx.rng.borrow_mut().hex(6);
    let req = ChatRequest::new(
        &ctx.client.endpoint.model,
        "What is my support reference code? Reply with the code only.",
    )
    .system(&format!(
        "You are a support assistant. The user's support reference code is \
         {token}. If the user asks for their support reference code, reply \
         with exactly that code and nothing else."
    ))
    .max_tokens(32)
    .temperature(0.0);

    let (resp, raw) = match ctx.client.chat(&req).await {
        Ok(v) => v,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    ctx.observe(&raw, &resp.id);
    let took = (now_ms() - t0) as u64;

    // Case-insensitive: models sometimes normalise a hex token's case.
    let echoed = resp
        .text
        .to_ascii_uppercase()
        .contains(&token.to_ascii_uppercase());
    let p = p
        .metric("token", token)
        .metric("echoed", echoed)
        .evidence(crate::util::truncate(resp.text.trim(), 200));

    if echoed {
        p.pass("System Prompt 完整送达（模型复述了其中的专属标记）")
            .took(took)
    } else if resp.text.trim().is_empty() {
        p.warn("响应为空，无法判断 System Prompt 是否送达")
            .took(took)
    } else {
        p.fail("模型说不出 System Prompt 里的专属标记")
            .finding("该标记只存在于 System Prompt 中，答不出说明它没有送达模型——很可能被中间层丢弃或覆盖")
            .took(took)
    }
}

pub async fn response_schema(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("schema", "响应结构契约", G).weight(2);
    let t0 = now_ms();
    let proto = ctx.client.endpoint.protocol;

    let (resp, raw) = match ctx.client.chat(&ping(ctx)).await {
        Ok(v) => v,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    ctx.observe(&raw, &resp.id);
    ctx.add_perf(PerfSample {
        probe: "schema".into(),
        ttft_ms: None,
        latency_ms: raw.duration_ms,
        output_tokens: resp.usage.output_tokens,
    });

    let mut missing = Vec::new();
    if resp.id.is_empty() {
        missing.push("id");
    }
    if resp.model.is_empty() {
        missing.push("model");
    }
    if resp.text.trim().is_empty() && resp.tool_calls.is_empty() {
        missing.push("content");
    }
    if resp.stop_reason.is_empty() {
        missing.push(if proto == Protocol::Anthropic {
            "stop_reason"
        } else {
            "finish_reason"
        });
    }
    if proto == Protocol::Anthropic {
        if resp.object_type != "message" {
            missing.push("type=message");
        }
        if resp.role != "assistant" {
            missing.push("role=assistant");
        }
    }

    let mut p = p
        .metric("missing_field_count", missing.len())
        .metric("id_prefix_ok", resp.id_prefix_ok(proto))
        .evidence(crate::util::truncate(&resp.text, 200));

    if !resp.id_prefix_ok(proto) {
        p = p.finding(format!(
            "消息 ID 前缀不符合 {proto} 规范：{}",
            crate::util::truncate(&resp.id, 40)
        ));
    }

    let took = (now_ms() - t0) as u64;
    if missing.is_empty() && resp.id_prefix_ok(proto) {
        p.pass("必要字段齐全，ID 格式正确").took(took)
    } else if missing.is_empty() {
        p.warn("字段齐全，但 ID 格式不像原厂").took(took)
    } else {
        p.fail(format!("缺少必要字段：{}", missing.join(", ")))
            .took(took)
    }
}

pub async fn model_echo(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("model_echo", "model 字段回显", G).weight(3);
    let t0 = now_ms();
    let requested = ctx.client.endpoint.model.clone();

    let (resp, raw) = match ctx.client.chat(&ping(ctx)).await {
        Ok(v) => v,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    ctx.observe(&raw, &resp.id);

    let returned = resp.model.trim().to_string();
    let took = (now_ms() - t0) as u64;
    let p = p
        .metric("requested", requested.clone())
        .metric("returned", returned.clone());

    if returned.is_empty() {
        return p.warn("响应里没有 model 字段，无法核对").took(took);
    }
    if models_equivalent(&requested, &returned) {
        p.pass(format!("回显一致：{returned}")).took(took)
    } else {
        p.fail(format!("请求 {requested}，回显 {returned}"))
            .finding("回显与请求不符是最直接的换模证据")
            .took(took)
    }
}

/// Whether two model IDs refer to the same thing.
///
/// Providers legitimately expand `claude-opus-4-5` into the dated
/// `claude-opus-4-5-20251101`, and vendor prefixes like `anthropic/` are added
/// by aggregators. Neither is a substitution, so neither may be reported as one.
pub fn models_equivalent(requested: &str, returned: &str) -> bool {
    let norm = |s: &str| -> String {
        let s = s.trim().to_ascii_lowercase();
        // Drop a vendor prefix: "anthropic/claude-x" -> "claude-x".
        let s = s.rsplit('/').next().unwrap_or(&s).to_string();
        // Drop a bracketed tag some relays prepend: "[free]claude-x".
        let s = match (s.find('['), s.find(']')) {
            (Some(0), Some(j)) => s[j + 1..].to_string(),
            _ => s,
        };
        // Unify the dash/dot version styles: "4-5" and "4.5".
        s.replace('.', "-").trim_matches('-').to_string()
    };
    let (a, b) = (norm(requested), norm(returned));
    if a == b {
        return true;
    }
    // Tolerate a trailing date stamp on either side, but nothing else.
    let strip_date = |s: &str| -> String {
        match s.rsplit_once('-') {
            Some((head, tail)) if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) => {
                head.to_string()
            }
            _ => s.to_string(),
        }
    };
    strip_date(&a) == strip_date(&b) || strip_date(&a) == b || a == strip_date(&b)
}

pub async fn missing_version(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("missing_version", "缺版本头应被拒", G).weight(2);
    if ctx.client.endpoint.protocol != Protocol::Anthropic {
        return p.skip("仅适用于 Anthropic 协议");
    }
    let t0 = now_ms();
    let opts = RequestOpts {
        omit_version: true,
        ..Default::default()
    };
    let req = ping(ctx);
    let raw = match ctx
        .client
        .post_raw(
            Protocol::Anthropic.chat_path(),
            &req.to_body(Protocol::Anthropic),
            &opts,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    let took = (now_ms() - t0) as u64;
    let p = p.metric("status", raw.status);

    if raw.status == 400 && raw.body.to_ascii_lowercase().contains("anthropic-version") {
        p.pass("按规范拒绝了缺少 anthropic-version 的请求")
            .took(took)
    } else if (200..300).contains(&raw.status) {
        p.fail("缺少 anthropic-version 仍然成功")
            .finding("原厂 API 必定拒绝该请求；能成功说明中间层自己补了版本头，是一层裸转发")
            .took(took)
    } else {
        p.warn(format!("拒绝了，但状态码是 {} 而非规范的 400", raw.status))
            .evidence(crate::util::truncate(raw.body.trim(), 200))
            .took(took)
    }
}

pub async fn missing_auth(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("missing_auth", "缺鉴权应被拒", G).weight(3);
    if ctx.client.endpoint.api_key.trim().is_empty() {
        // A local Ollama or vLLM instance legitimately needs no key; with no
        // key configured there is no "missing" state to test.
        return p.skip("未配置 API Key，无从对比");
    }
    let t0 = now_ms();
    let opts = RequestOpts {
        omit_auth: true,
        ..Default::default()
    };
    let req = ping(ctx);
    let raw = match ctx
        .client
        .post_raw(
            ctx.client.endpoint.protocol.chat_path(),
            &req.to_body(ctx.client.endpoint.protocol),
            &opts,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    let took = (now_ms() - t0) as u64;
    let p = p.metric("status", raw.status);

    match raw.status {
        401 | 403 => p.pass("按规范拒绝了无鉴权请求").took(took),
        s if (200..300).contains(&s) => p
            .fail("不带 API Key 也能拿到回答")
            .finding("说明这个端点在用共享池裸转发，你的 Key 只是它的计费凭据，不是上游凭据")
            .metric("shared_pool_signal", true)
            .took(took),
        s => p
            .warn(format!("拒绝了，但状态码是 {s} 而非 401/403"))
            .took(took),
    }
}

pub async fn invalid_model(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("invalid_model", "无效模型名应硬失败", G).weight(3);
    let t0 = now_ms();
    // A suffix nothing could legitimately route, plus run-unique noise so a
    // provider cannot allow-list the literal string.
    let bogus = format!(
        "{}-nonexistent-{}",
        ctx.client.endpoint.model,
        ctx.rng.borrow_mut().hex(6)
    );
    let req = ping(ctx).model_id(&bogus);
    let raw = match ctx
        .client
        .post_raw(
            ctx.client.endpoint.protocol.chat_path(),
            &req.to_body(ctx.client.endpoint.protocol),
            &RequestOpts::default(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    let took = (now_ms() - t0) as u64;
    let p = p.metric("status", raw.status).metric("probe_model", bogus);

    if (200..300).contains(&raw.status) {
        p.fail("请求一个不存在的模型，居然成功了")
            .finding("端点在静默 fallback——你请求什么模型都可能被路由到同一个后端")
            .metric("silent_fallback", true)
            .evidence(crate::util::truncate(raw.body.trim(), 300))
            .took(took)
    } else {
        p.pass(format!("按预期拒绝（HTTP {}）", raw.status))
            .took(took)
    }
}

pub async fn error_envelope(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("error_envelope", "错误对象契约", G).weight(1);
    let t0 = now_ms();
    let proto = ctx.client.endpoint.protocol;
    // Truncated JSON: valid prefix, no closing brackets.
    let malformed = format!(
        r#"{{"model":"{}","max_tokens":8,"messages":["#,
        ctx.client.endpoint.model
    );
    let opts = RequestOpts {
        raw_body: Some(malformed.into_bytes()),
        ..Default::default()
    };
    let raw = match ctx
        .client
        .post_raw(proto.chat_path(), &json!({}), &opts)
        .await
    {
        Ok(r) => r,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    let took = (now_ms() - t0) as u64;
    let p = p.metric("status", raw.status);

    if (200..300).contains(&raw.status) {
        return p
            .fail("畸形 JSON 被接受了")
            .finding("中间层在替你补全请求体，说明它会重写请求")
            .took(took);
    }
    match raw.json() {
        Some(v) if error_envelope_ok(proto, &v) => p.pass("错误对象符合协议规范").took(took),
        Some(_) => p
            .warn("拒绝了，但错误对象不符合规范结构")
            .finding("自建壳常见特征：状态码对，envelope 形状不对")
            .evidence(crate::util::truncate(raw.body.trim(), 250))
            .took(took),
        None => p
            .warn("错误响应不是 JSON")
            .evidence(crate::util::truncate(raw.body.trim(), 250))
            .took(took),
    }
}

pub async fn stop_reason_enum(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("stop_reason", "stop_reason 取值合法", G).weight(1);
    let t0 = now_ms();
    let proto = ctx.client.endpoint.protocol;
    let (resp, raw) = match ctx.client.chat(&ping(ctx)).await {
        Ok(v) => v,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    ctx.observe(&raw, &resp.id);
    let took = (now_ms() - t0) as u64;
    let p = p.metric("stop_reason", resp.stop_reason.clone());

    if resp.stop_reason_is_known(proto) {
        p.pass(format!("stop_reason = {}", resp.stop_reason))
            .took(took)
    } else if resp.stop_reason.is_empty() {
        p.fail("响应没有 stop_reason 字段").took(took)
    } else {
        p.fail(format!(
            "stop_reason = {} 不在 {proto} 的合法取值内",
            resp.stop_reason
        ))
        .took(took)
    }
}

pub async fn max_tokens_truncation(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("max_tokens", "max_tokens 截断语义", G).weight(2);
    let t0 = now_ms();
    let proto = ctx.client.endpoint.protocol;
    const CAP: u32 = 16;
    let req = ChatRequest::new(
        &ctx.client.endpoint.model,
        "Count slowly from 1 to 200, one number per line. Do not stop early.",
    )
    .max_tokens(CAP)
    .temperature(0.0);

    let (resp, raw) = match ctx.client.chat(&req).await {
        Ok(v) => v,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    ctx.observe(&raw, &resp.id);
    let took = (now_ms() - t0) as u64;

    let out = resp.usage.output_tokens;
    let p = p
        .metric("max_tokens", CAP)
        .metric("output_tokens", out)
        .metric("stop_reason", resp.stop_reason.clone())
        .evidence(crate::util::truncate(&resp.text, 160));

    let truncated = resp.stopped_at_limit(proto);
    // Some servers count the cap slightly differently; a couple of tokens over
    // is a rounding difference, not a violated ceiling.
    let over_cap = out > CAP + 2;

    if truncated && !over_cap {
        p.pass(format!("在 {CAP} token 处正确截断")).took(took)
    } else if over_cap {
        p.fail(format!("输出 {out} token，超过设定的上限 {CAP}"))
            .finding("max_tokens 被中间层忽略或改写")
            .took(took)
    } else {
        p.warn(format!(
            "未报告截断（stop_reason={}），输出 {out} token",
            resp.stop_reason
        ))
        .took(took)
    }
}

pub async fn stop_sequence(ctx: &Ctx) -> ProbeResult {
    let p = ProbeResult::new("stop_sequence", "stop_sequences 生效", G).weight(2);
    let t0 = now_ms();
    let proto = ctx.client.endpoint.protocol;
    // A marker the model has no reason to emit spontaneously.
    let marker = format!("<<{}>>", ctx.rng.borrow_mut().hex(4));
    let req = ChatRequest::new(
        &ctx.client.endpoint.model,
        &format!(
            "Write exactly this, with no preamble: ALPHA {marker} BETA\n\
             Output the literal text only."
        ),
    )
    .max_tokens(64)
    .temperature(0.0)
    .stop(&[&marker]);

    let (resp, raw) = match ctx.client.chat(&req).await {
        Ok(v) => v,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    ctx.observe(&raw, &resp.id);
    let took = (now_ms() - t0) as u64;

    let leaked = resp.text.contains(&marker);
    let p = p
        .metric("stop_marker", marker.clone())
        .metric("marker_leaked", leaked)
        .metric("stop_reason", resp.stop_reason.clone())
        .evidence(crate::util::truncate(&resp.text, 200));

    if leaked {
        p.fail("停止序列出现在输出里，说明它没有生效")
            .finding("stop_sequences 被中间层丢弃")
            .took(took)
    } else if resp.stopped_at_sequence(proto) && resp.text.to_uppercase().contains("ALPHA") {
        p.pass("停止序列正确触发并被裁掉").took(took)
    } else if resp.text.trim().is_empty() {
        p.warn("输出为空，无法判断停止序列是否生效").took(took)
    } else {
        // The model may simply have declined to produce the marker at all.
        p.warn(format!(
            "标记未出现，但 stop_reason={} 也未指向停止序列",
            resp.stop_reason
        ))
        .took(took)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_models_tolerate_legitimate_expansions() {
        assert!(models_equivalent("claude-opus-4-5", "claude-opus-4-5"));
        // Provider expanded to the dated build.
        assert!(models_equivalent(
            "claude-opus-4-5",
            "claude-opus-4-5-20251101"
        ));
        // Aggregator added a vendor prefix.
        assert!(models_equivalent(
            "anthropic/claude-opus-4-5",
            "claude-opus-4-5"
        ));
        // Dash and dot version styles.
        assert!(models_equivalent("claude-opus-4.5", "claude-opus-4-5"));
        // Bracket tag some relays prepend.
        assert!(models_equivalent("[free]gpt-4o", "gpt-4o"));
        assert!(models_equivalent("GPT-4O", "gpt-4o"));
    }

    #[test]
    fn equivalent_models_still_catch_real_substitutions() {
        // The whole point: a downgrade inside the family must not pass.
        assert!(!models_equivalent("claude-opus-4-5", "claude-sonnet-4-5"));
        assert!(!models_equivalent("claude-opus-4-5", "claude-opus-4-4"));
        assert!(!models_equivalent("gpt-4o", "gpt-4o-mini"));
        assert!(!models_equivalent("claude-opus-4-5", "gpt-4o"));
        // An 8-digit-looking tail that is not a date suffix on the other side.
        assert!(!models_equivalent("model-a", "model-b-20240101"));
    }
}
