// SPDX-License-Identifier: Apache-2.0
//! Turns probe results into a conclusion.
//!
//! Two rules shape everything here.
//!
//! First, the two axes are independent: *is it genuine* and *where did it come
//! from* are different questions. A real model behind a relay is still a real
//! model, and conflating the two produces the most common false accusation in
//! this space.
//!
//! Second, a false accusation against an honest provider costs far more than a
//! miss. So every uncertain path degrades toward "cannot tell" rather than
//! toward "guilty", and every conclusion carries the trace that produced it.

use crate::i18n::Lang;
use crate::probes::identity::{tier_from_model_id, Tier};
use crate::report::*;
use std::collections::BTreeMap;

/// Group weights for the 0–100 score. Contract and billing dominate because
/// they are the checks with the least interpretive slack.
fn group_weight(g: Group) -> f64 {
    match g {
        Group::Contract => 0.26,
        Group::Billing => 0.22,
        Group::Identity => 0.18,
        Group::Stream => 0.12,
        Group::Consistency => 0.12,
        Group::Perf => 0.06,
        Group::Channel => 0.04,
    }
}

fn find<'a>(results: &'a [ProbeResult], id: &str) -> Option<&'a ProbeResult> {
    results.iter().find(|r| r.id == id)
}

fn status_of(results: &[ProbeResult], id: &str) -> Option<Status> {
    find(results, id).map(|r| r.status)
}

fn failed(results: &[ProbeResult], id: &str) -> bool {
    status_of(results, id) == Some(Status::Fail)
}

fn passed(results: &[ProbeResult], id: &str) -> bool {
    status_of(results, id) == Some(Status::Pass)
}

/// Signals that, taken together, describe an endpoint reconstructed from a
/// web session rather than resold from a real API key.
///
/// Weighted, because they are not equal evidence. A missing capability — no
/// tool calls, no prompt caching, no token counter — is something a real API
/// key always has and a scraped session never does. A missing vendor header or
/// a non-canonical error body is true of *every* relay, including entirely
/// honest ones, so on its own it must not push anyone into this bucket.
fn reverse_signals(
    results: &[ProbeResult],
    protocol: crate::protocol::Protocol,
    l: Lang,
) -> (Vec<String>, f64) {
    const STRONG: f64 = 1.0;
    const WEAK: f64 = 0.5;

    let mut out = Vec::new();
    let mut weight = 0.0;
    let add = |w: f64, msg: String, out: &mut Vec<String>, total: &mut f64| {
        out.push(msg);
        *total += w;
    };

    // Strong: capabilities a genuine API key would expose.
    if find(results, "stream_usage")
        .and_then(|r| r.metric_bool("usage_absent"))
        .unwrap_or(false)
    {
        add(
            STRONG,
            t!(
                l,
                "the stream reports no usage at all",
                "流式响应完全不上报 usage"
            ),
            &mut out,
            &mut weight,
        );
    }
    if failed(results, "usage_present") {
        add(
            STRONG,
            t!(
                l,
                "usage is missing or permanently zero",
                "usage 字段缺失或恒为零"
            ),
            &mut out,
            &mut weight,
        );
    }
    if failed(results, "max_tokens") {
        add(
            STRONG,
            t!(l, "max_tokens has no effect", "max_tokens 未生效"),
            &mut out,
            &mut weight,
        );
    }
    if failed(results, "stop_sequence") {
        add(
            STRONG,
            t!(l, "stop_sequences has no effect", "stop_sequences 未生效"),
            &mut out,
            &mut weight,
        );
    }
    if failed(results, "system_adherence") {
        add(
            STRONG,
            t!(l, "the system prompt is dropped", "System Prompt 被丢弃"),
            &mut out,
            &mut weight,
        );
    }
    if failed(results, "sse_format") || failed(results, "stream_body") {
        add(
            STRONG,
            t!(
                l,
                "streaming does not follow the protocol",
                "流式传输不符合规范"
            ),
            &mut out,
            &mut weight,
        );
    }
    // Only meaningful where the protocol actually defines the route.
    if protocol == crate::protocol::Protocol::Anthropic {
        if let Some(r) = find(results, "token_recount") {
            if r.metric_bool("authoritative") == Some(false) {
                add(
                    STRONG,
                    t!(
                        l,
                        "the count_tokens endpoint is unavailable",
                        "count_tokens 端点不可用"
                    ),
                    &mut out,
                    &mut weight,
                );
            }
        }
    }

    // Weak: true of ordinary relays too, so they can corroborate but never
    // convict on their own.
    if matches!(
        status_of(results, "error_envelope"),
        Some(Status::Warn | Status::Fail)
    ) {
        add(
            WEAK,
            t!(
                l,
                "the error object does not match the protocol envelope",
                "错误对象不符合协议规范"
            ),
            &mut out,
            &mut weight,
        );
    }
    if matches!(status_of(results, "official_headers"), Some(Status::Warn)) {
        add(
            WEAK,
            t!(l, "vendor marker headers are absent", "缺少官方特征响应头"),
            &mut out,
            &mut weight,
        );
    }

    (out, weight)
}

/// Conditions that override the weighted score entirely. Each one is a fact
/// about the endpoint that no amount of good behaviour elsewhere excuses.
fn hard_gates(results: &[ProbeResult], identity: &Identity, l: Lang) -> Vec<GateHit> {
    let mut hits = Vec::new();
    let mut gate = |name: String, probe: &str, reason: String| {
        hits.push(GateHit {
            name,
            probe: probe.to_string(),
            reason,
        });
    };

    if find(results, "invalid_model")
        .and_then(|r| r.metric_bool("silent_fallback"))
        .unwrap_or(false)
    {
        gate(
            t!(l, "Silent fallback", "静默 fallback"),
            "invalid_model",
            t!(l, "A model that cannot exist was still served, so the model name does not determine the backend", "请求一个不存在的模型也返回了内容，说明模型名不决定后端"),
        );
    }
    if find(results, "missing_auth")
        .and_then(|r| r.metric_bool("shared_pool_signal"))
        .unwrap_or(false)
    {
        gate(
            t!(l, "Shared-pool forwarding", "共享池裸转发"),
            "missing_auth",
            t!(
                l,
                "An answer came back with no API key, so your key is not the upstream credential",
                "不带 API Key 也能取得回答，你的 Key 并非上游凭据"
            ),
        );
    }
    if identity.tier_severity >= 2 {
        gate(
            t!(l, "Tier downgrade", "档位降级"),
            "tier_estimate",
            t!(l, "Measured capability sits two or more tiers below the claim, which sampling noise cannot explain", "实测能力档位与宣称相差两档以上，无法用采样噪声解释"),
        );
    }
    if failed(results, "wrapper_marker") {
        gate(
            t!(l, "Third-party wrapper injection", "第三方壳注入"),
            "wrapper_marker",
            t!(l, "Third-party wrapper markers appear in the response; that layer rewrites your prompt and the reply", "响应中检出第三方包装层标记，它会改写你的 prompt 与响应"),
        );
    }
    if failed(results, "cache_replay") {
        gate(
            t!(l, "Cache replay", "缓存回放"),
            "cache_replay",
            t!(l, "Repeated high-temperature samples came back word-for-word identical; these are not freshly generated", "高温度下多次采样逐字相同，返回的不是真实生成结果"),
        );
    }
    if failed(results, "input_inflation") {
        gate(
            t!(l, "Hidden prompt injection", "隐藏 prompt 注入"),
            "input_inflation",
            t!(l, "A minimal prompt is billed as many tokens; every request pays for injected content", "最小 prompt 被计入大量 token，每次请求都在为注入内容付费"),
        );
    }
    if failed(results, "id_unique") {
        gate(
            t!(l, "Response replay", "响应重放"),
            "id_unique",
            t!(
                l,
                "Message IDs repeat, so responses are being reused",
                "消息 ID 出现重复，响应被复用"
            ),
        );
    }
    hits
}

fn score(results: &[ProbeResult]) -> (f64, BTreeMap<String, f64>) {
    let mut per_group: BTreeMap<String, f64> = BTreeMap::new();
    let mut total = 0.0;
    let mut total_weight = 0.0;

    for g in Group::ALL {
        let scored: Vec<&ProbeResult> = results
            .iter()
            .filter(|r| r.group == g && !r.neutral && r.status.scored())
            .collect();
        if scored.is_empty() {
            continue;
        }
        let earned: f64 = scored
            .iter()
            .map(|r| r.status.credit() * r.weight as f64)
            .sum();
        let possible: f64 = scored.iter().map(|r| r.weight as f64).sum();
        let pct = if possible > 0.0 {
            earned / possible * 100.0
        } else {
            0.0
        };
        per_group.insert(g.key().to_string(), (pct * 10.0).round() / 10.0);
        let w = group_weight(g);
        total += pct * w;
        total_weight += w;
    }

    let score = if total_weight > 0.0 {
        total / total_weight
    } else {
        0.0
    };
    ((score * 10.0).round() / 10.0, per_group)
}

/// Assemble the identity view from the surface and capability probes.
pub fn build_identity(results: &[ProbeResult], claimed_model: &str, l: Lang) -> Identity {
    let mut id = Identity {
        claimed_model: claimed_model.to_string(),
        claimed_family: crate::probes::identity::family_from_model_id(claimed_model),
        claimed_tier: tier_from_model_id(claimed_model).map(|t| t.as_str().to_string()),
        status: IdentityStatus::Insufficient,
        ..Default::default()
    };

    if let Some(r) = find(results, "self_id") {
        let observed = r
            .metrics
            .get("observed_family")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let hits = r.metric_f64("marker_hits").unwrap_or(0.0);
        id.observed_family = observed;
        // More independent markers in the same answer means a stronger call,
        // but self-report never exceeds medium confidence on its own.
        id.family_confidence = (hits / 3.0).clamp(0.0, 1.0) * 0.7;
        if let Some(e) = &r.evidence {
            id.evidence
                .push(t!(l, "Self-identification: {e}", "自述身份：{e}"));
        }
    }
    if let Some(r) = find(results, "meta_creator") {
        if let Some(f) = r.metrics.get("observed_family").and_then(|v| v.as_str()) {
            if !f.is_empty() {
                // A second, independently-phrased question agreeing raises
                // confidence; disagreeing lowers it.
                if id.observed_family.as_deref() == Some(f) {
                    id.family_confidence = (id.family_confidence + 0.25).min(0.95);
                    id.evidence.push(t!(
                        l,
                        "The creator answer also points to {f}",
                        "创造者自述同样指向 {f}"
                    ));
                } else if id.observed_family.is_some() {
                    id.family_confidence *= 0.6;
                    id.evidence
                        .push(t!(l, "The creator answer points to {f}, disagreeing with the self-identification", "创造者自述指向 {f}，与身份自述不一致"));
                } else {
                    id.observed_family = Some(f.to_string());
                    id.family_confidence = 0.4;
                }
            }
        }
    }

    if let Some(r) = find(results, "capability") {
        for (k, band) in [
            ("easy_accuracy", "easy"),
            ("medium_accuracy", "medium"),
            ("hard_accuracy", "hard"),
        ] {
            if let Some(v) = r.metric_f64(k) {
                id.accuracy_by_difficulty.insert(band.to_string(), v);
            }
        }
    }
    if let Some(r) = find(results, "tier_estimate") {
        for t in Tier::ALL {
            if let Some(v) = r.metric_f64(&format!("score_{}", t.as_str())) {
                id.tier_scores.insert(t.as_str().to_string(), v);
            }
        }
        id.estimated_tier = r
            .metrics
            .get("estimated_tier")
            .and_then(|v| v.as_str())
            .map(String::from);
        id.tier_confidence = r.metric_f64("fit").unwrap_or(0.0);
        id.tier_severity = r.metric_f64("tier_severity").unwrap_or(0.0) as u8;
        id.tier_questions = r.metric_f64("questions_asked").unwrap_or(0.0) as u32;
        id.tier_margin = r.metric_f64("margin").unwrap_or(0.0);
        // A narrow margin means the tier profiles overlapped; withdraw the
        // severity claim rather than assert a downgrade we cannot support.
        if r.metric_f64("margin").unwrap_or(1.0) < 0.06 {
            id.tier_severity = 0;
            id.evidence.push(
                t!(
                    l,
                    "Tier candidates were too close; no downgrade asserted",
                    "档位候选过于接近，未断言降级"
                )
                .to_string(),
            );
        }
        if let Some(t) = &id.estimated_tier {
            id.evidence.push(t!(
                l,
                "Measured capability tier: {t} (fit {:.2})",
                "能力实测档位：{t}（拟合 {:.2}）",
                id.tier_confidence
            ));
        }
    }
    if let Some(r) = find(results, "world_knowledge") {
        if matches!(r.status, Status::Fail | Status::Warn) {
            id.evidence
                .push(t!(l, "Knowledge cutoff: {}", "知识截止：{}", r.summary));
        }
    }
    if let Some(r) = find(results, "tps") {
        if let (Some(band), Some(claimed)) = (
            r.metrics.get("speed_band").and_then(|v| v.as_str()),
            tier_from_model_id(claimed_model),
        ) {
            let implied = match band {
                "large" => Some(Tier::Large),
                "small" => Some(Tier::Small),
                _ => None,
            };
            if let Some(implied) = implied {
                if implied != claimed && (claimed.rank() - implied.rank()).abs() >= 2 {
                    id.evidence.push(t!(
                        l,
                        "Throughput points to {}, which disagrees with the claimed {}",
                        "吞吐速度指向{}，与宣称的{}不符",
                        implied.label(l),
                        claimed.label(l)
                    ));
                }
            }
        }
    }

    // ── status ─────────────────────────────────────────────────────────────
    let family_match = match (&id.claimed_family, &id.observed_family) {
        (Some(c), Some(o)) => Some(c == o),
        _ => None,
    };
    let tier_known = id.estimated_tier.is_some() && id.tier_confidence > 0.0;

    id.status = match (family_match, tier_known) {
        (Some(false), _) => IdentityStatus::FamilyMismatch,
        (Some(true), true) if id.tier_severity >= 2 => IdentityStatus::TierMismatch,
        (Some(true), true) if id.tier_severity == 0 => IdentityStatus::Match,
        // One step apart is within what a single sampling run can produce.
        (Some(true), true) => IdentityStatus::FamilyOnly,
        (Some(true), false) => IdentityStatus::FamilyOnly,
        (None, true) if id.tier_severity >= 2 => IdentityStatus::TierMismatch,
        (None, _) => IdentityStatus::Insufficient,
    };

    // A family verdict resting on a weak self-report is not a family verdict.
    if id.status == IdentityStatus::FamilyMismatch && id.family_confidence < 0.35 {
        id.status = IdentityStatus::Ambiguous;
        id.evidence.push(
            t!(
                l,
                "The family signal was too weak; no family mismatch asserted",
                "家族信号强度不足，未断言家族不符"
            )
            .to_string(),
        );
    }
    id
}

pub fn build_billing(results: &[ProbeResult], model: &str, l: Lang) -> BillingAudit {
    let mut b = BillingAudit {
        method: t!(l, "not measured", "未测量"),
        pricing_source: t!(l, "built-in price table", "内置定价表"),
        ..Default::default()
    };

    if let Some(r) = find(results, "token_recount") {
        b.method = r
            .metrics
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or(ts!(l, "unknown", "未知"))
            .to_string();
        b.billed_input = r.metric_f64("billed_input").unwrap_or(0.0) as u32;
        b.honest_input = r.metric_f64("honest_input").unwrap_or(0.0) as u32;
    }
    if let Some(r) = find(results, "output_inflation") {
        b.billed_output = r.metric_f64("billed_output").unwrap_or(0.0) as u32;
        b.honest_output = r.metric_f64("estimated_output").unwrap_or(0.0) as u32;
    }
    if let Some(r) = find(results, "cost_ratio") {
        b.input_ratio = r.metric_f64("ratio").unwrap_or(1.0);
        b.billed_input = r
            .metric_f64("billed_input_total")
            .unwrap_or(b.billed_input as f64) as u32;
        b.honest_input = r
            .metric_f64("honest_input_total")
            .unwrap_or(b.honest_input as f64) as u32;
        b.billed_cost_usd = r.metric_f64("billed_cost_usd").unwrap_or(0.0);
        b.honest_cost_usd = r.metric_f64("honest_cost_usd").unwrap_or(0.0);
        b.cost_ratio = if b.honest_cost_usd > 0.0 {
            b.billed_cost_usd / b.honest_cost_usd
        } else {
            b.input_ratio
        };
    }
    if crate::pricing::lookup(model).is_none() {
        b.pricing_source = t!(
            l,
            "No price entry for {model}; no currency figure applied",
            "定价表无 {model} 条目，未折算金额"
        );
    }
    for id in [
        "input_inflation",
        "output_inflation",
        "usage_present",
        "cost_ratio",
        "token_recount",
    ] {
        if let Some(r) = find(results, id) {
            if matches!(r.status, Status::Fail | Status::Warn) {
                b.anomalies.push(format!("{}：{}", r.label, r.summary));
            }
        }
    }
    b
}

pub fn build_channel(results: &[ProbeResult], l: Lang) -> ChannelSignature {
    let mut c = ChannelSignature {
        key: crate::probes::channel::UNKNOWN_PROXY.to_string(),
        display: crate::probes::channel::display(crate::probes::channel::UNKNOWN_PROXY, l),
        ..Default::default()
    };
    if let Some(r) = find(results, "channel_signature") {
        if let Some(k) = r.metrics.get("channel").and_then(|v| v.as_str()) {
            c.key = k.to_string();
            c.display = crate::probes::channel::display(k, l);
        }
        c.confidence = r.metric_f64("confidence").unwrap_or(0.0);
        c.tier = r.metric_f64("tier").unwrap_or(3.0) as u8;
        c.evidence = r.findings.clone();
    }
    if let Some(r) = find(results, "multi_hop") {
        if let Some(v) = r.metrics.get("vendors").and_then(|v| v.as_str()) {
            if !v.is_empty() {
                c.all_hops = v.split(" → ").map(String::from).collect();
            }
        }
    }
    c
}

/// Reverse-channel weight at or above which the endpoint is classified as a
/// reconstructed channel. Two strong signals, or one strong plus two weak.
const REVERSE_THRESHOLD: f64 = 2.0;

fn classify_channel(
    results: &[ProbeResult],
    channel: &ChannelSignature,
    reverse_weight: f64,
) -> Channel {
    // Enough reverse weight outranks a vendor label: relay software in front
    // of a reconstructed backend still stamps its own headers.
    if reverse_weight >= REVERSE_THRESHOLD {
        return Channel::ReverseProxy;
    }
    // Routed on the classifier's stable key, never on its display name — a
    // translated label must not be able to change the verdict.
    use crate::probes::channel as ch;
    match channel.key.as_str() {
        ch::ANTHROPIC_OFFICIAL | ch::OPENAI_OFFICIAL => Channel::Official,
        ch::AWS_BEDROCK | ch::GOOGLE_VERTEX | ch::AZURE_FOUNDRY | ch::AWS_APIGATEWAY => {
            Channel::Cloud
        }
        ch::UNKNOWN_PROXY => {
            if passed(results, "official_headers") {
                Channel::Subscription
            } else {
                Channel::Unknown
            }
        }
        _ => Channel::Proxy,
    }
}

pub fn decide(
    results: &[ProbeResult],
    identity: &Identity,
    billing: &BillingAudit,
    channel_sig: &ChannelSignature,
    protocol: crate::protocol::Protocol,
    l: Lang,
) -> Verdict {
    let mut trace = Vec::new();
    let mut signals = Vec::new();

    let (score, group_scores) = score(results);
    trace.push(t!(l, "Weighted score {score:.1}", "加权分 {score:.1}"));

    let errored = results.iter().filter(|r| r.status == Status::Error).count();
    let total = results.len().max(1);
    let coverage_gap = errored as f64 / total as f64;
    trace.push(t!(
        l,
        "Coverage gap {:.0}% ({errored}/{total} probes could not run)",
        "覆盖缺口 {:.0}%（{errored}/{total} 个探针未能执行）",
        coverage_gap * 100.0
    ));

    let gates = hard_gates(results, identity, l);
    for g in &gates {
        trace.push(t!(
            l,
            "Hard gate tripped: {} — {}",
            "硬门禁命中：{} — {}",
            g.name,
            g.reason
        ));
        signals.push(format!("{}：{}", g.name, g.reason));
    }

    let (reverse, reverse_weight) = reverse_signals(results, protocol, l);
    if !reverse.is_empty() {
        trace.push(t!(l, "{} reverse-channel signal(s) (weight {reverse_weight:.1}, threshold {REVERSE_THRESHOLD:.1}): {}", "逆向信号 {} 项（加权 {reverse_weight:.1}，判定门槛 {REVERSE_THRESHOLD:.1}）：{}",
            reverse.len(),
            reverse.join("、")
        ));
    }
    let channel = classify_channel(results, channel_sig, reverse_weight);
    trace.push(t!(l, "Origin: {}", "来源判定：{}", channel.label(l)));

    // ── authenticity ───────────────────────────────────────────────────────
    let unreachable = status_of(results, "preflight")
        .map(|s| matches!(s, Status::Fail | Status::Error))
        .unwrap_or(true);

    let authenticity = if unreachable {
        trace.push(t!(
            l,
            "Connectivity preflight did not pass; no verdict possible",
            "连通性预检未通过，无法判定"
        ));
        Authenticity::Inconclusive
    } else if failed(results, "model_echo") && identity.status == IdentityStatus::FamilyMismatch {
        trace.push(t!(
            l,
            "The echoed model and the self-identification both disagree",
            "model 回显与身份自述同时对不上"
        ));
        Authenticity::Counterfeit
    } else if identity.status == IdentityStatus::FamilyMismatch && identity.family_confidence >= 0.5
    {
        trace.push(t!(
            l,
            "Identity fingerprints point at a different model family",
            "身份指纹指向另一个模型家族"
        ));
        Authenticity::Counterfeit
    } else if !gates.is_empty() {
        trace.push(t!(
            l,
            "A hard gate tripped; verdict forced to suspicious",
            "命中硬门禁，直接判存疑"
        ));
        Authenticity::Suspicious
    } else if reverse_weight >= REVERSE_THRESHOLD {
        trace.push(t!(
            l,
            "Reverse-channel signal weight {reverse_weight:.1} reached the threshold",
            "逆向渠道信号加权 {reverse_weight:.1} 达到门槛"
        ));
        Authenticity::Suspicious
    } else if coverage_gap > 0.25 {
        trace.push(t!(
            l,
            "Over a quarter of probes could not run; not enough data",
            "超过四分之一的探针未能执行，数据不足"
        ));
        Authenticity::Inconclusive
    } else if score >= 90.0 && billing.input_ratio <= 1.05 && channel == Channel::Official {
        Authenticity::Authentic
    } else if score >= 85.0 && billing.input_ratio <= 1.15 {
        if channel == Channel::Official || channel == Channel::Cloud {
            Authenticity::Authentic
        } else {
            trace.push(t!(
                l,
                "Behaviour is clean but the request is relayed",
                "行为干净但经过转发"
            ));
            Authenticity::ThirdParty
        }
    } else if score >= 70.0 {
        if billing.input_ratio > 1.15 {
            trace.push(t!(
                l,
                "Billing ratio {:.2}x runs high",
                "计费倍率 {:.2}× 偏高",
                billing.input_ratio
            ));
            Authenticity::AuthenticDegraded
        } else {
            Authenticity::ThirdParty
        }
    } else if score >= 50.0 {
        Authenticity::Suspicious
    } else {
        trace.push(t!(l, "Weighted score below 50", "加权分低于 50"));
        Authenticity::Suspicious
    };

    // ── confidence ─────────────────────────────────────────────────────────
    let mut confidence: f64 = match authenticity {
        Authenticity::Inconclusive => 0.30,
        Authenticity::Counterfeit => 0.80,
        Authenticity::Authentic => 0.88,
        _ => 0.70,
    };
    if coverage_gap > 0.05 {
        confidence -= 0.15;
        trace.push(t!(
            l,
            "Confidence lowered because of the coverage gap",
            "因覆盖缺口降低置信度"
        ));
    }
    if !gates.is_empty() {
        // Gates are the least ambiguous evidence we have.
        confidence += 0.08;
    }
    confidence = confidence.clamp(0.15, 0.95);

    for id in ["model_echo", "input_inflation", "cache_replay", "multi_hop"] {
        if let Some(r) = find(results, id) {
            if r.status == Status::Fail {
                signals.push(format!("{}：{}", r.label, r.summary));
            }
        }
    }
    for r in &reverse {
        signals.push(t!(l, "Reverse-channel signal: {r}", "逆向信号：{r}"));
    }

    Verdict {
        authenticity,
        channel,
        score,
        confidence: (confidence * 100.0).round() / 100.0,
        hard_gate_hits: gates,
        signals,
        trace,
        group_scores,
        coverage_gap: (coverage_gap * 1000.0).round() / 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Lang = Lang::En;
    use crate::protocol::Protocol;

    fn p(id: &str, g: Group, s: Status, w: u32) -> ProbeResult {
        let mut r = ProbeResult::new(id, id, g).weight(w);
        r.status = s;
        r
    }

    #[test]
    fn score_ignores_neutral_and_unscored_probes() {
        let results = vec![
            p("a", Group::Contract, Status::Pass, 1),
            p("b", Group::Contract, Status::Fail, 1),
            p("c", Group::Contract, Status::Skip, 5), // must not drag the score
            p("d", Group::Contract, Status::Pass, 1).neutral(), // must not lift it
        ];
        let (_, groups) = score(&results);
        assert_eq!(groups["contract"], 50.0);
    }

    #[test]
    fn warn_is_worth_half_a_pass() {
        let results = vec![p("a", Group::Billing, Status::Warn, 1)];
        let (_, groups) = score(&results);
        assert_eq!(groups["billing"], 50.0);
    }

    #[test]
    fn weight_shifts_the_group_score() {
        let results = vec![
            p("a", Group::Contract, Status::Pass, 3),
            p("b", Group::Contract, Status::Fail, 1),
        ];
        let (_, groups) = score(&results);
        assert_eq!(groups["contract"], 75.0);
    }

    #[test]
    fn silent_fallback_trips_a_hard_gate() {
        let mut r = p("invalid_model", Group::Contract, Status::Fail, 3);
        r = r.metric("silent_fallback", true);
        let gates = hard_gates(&[r], &Identity::default(), L);
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].name, "Silent fallback");
    }

    #[test]
    fn tier_severity_two_trips_a_gate_but_one_does_not() {
        let id2 = Identity {
            tier_severity: 2,
            ..Default::default()
        };
        assert_eq!(hard_gates(&[], &id2, L).len(), 1);
        let id1 = Identity {
            tier_severity: 1,
            ..Default::default()
        };
        assert!(
            hard_gates(&[], &id1, L).is_empty(),
            "one tier apart is within sampling noise and must not accuse"
        );
    }

    #[test]
    fn reverse_signals_only_count_count_tokens_for_anthropic() {
        let r = p("token_recount", Group::Billing, Status::Pass, 1).metric("authoritative", false);
        assert!(
            reverse_signals(std::slice::from_ref(&r), Protocol::Anthropic, L)
                .0
                .iter()
                .any(|s| s.contains("count_tokens"))
        );
        // OpenAI has no such route, so its absence is not evidence of anything.
        assert!(reverse_signals(&[r], Protocol::OpenAI, L).0.is_empty());
    }

    #[test]
    fn two_reverse_signals_outrank_a_vendor_label() {
        let sig = ChannelSignature {
            key: crate::probes::channel::ANTHROPIC_OFFICIAL.into(),
            ..Default::default()
        };
        assert_eq!(classify_channel(&[], &sig, 0.0), Channel::Official);
        assert_eq!(classify_channel(&[], &sig, 2.0), Channel::ReverseProxy);
        // Two weak signals alone total 1.0 and must not convict.
        assert_eq!(classify_channel(&[], &sig, 1.0), Channel::Official);
    }

    #[test]
    fn unknown_vendor_with_official_headers_reads_as_subscription() {
        let sig = ChannelSignature {
            key: crate::probes::channel::UNKNOWN_PROXY.into(),
            ..Default::default()
        };
        let with_headers = vec![p("official_headers", Group::Channel, Status::Pass, 1)];
        assert_eq!(
            classify_channel(&with_headers, &sig, 0.0),
            Channel::Subscription
        );
        assert_eq!(classify_channel(&[], &sig, 0.0), Channel::Unknown);
    }

    #[test]
    fn family_mismatch_on_a_weak_signal_degrades_to_ambiguous() {
        // The core anti-false-accusation rule: a single low-confidence
        // self-report must never produce a "wrong family" verdict.
        let results = vec![p("self_id", Group::Identity, Status::Fail, 2)
            .metric("observed_family", "openai")
            .metric("marker_hits", 1)];
        let id = build_identity(&results, "claude-opus-4-5", L);
        assert!(id.family_confidence < 0.35);
        assert_eq!(id.status, IdentityStatus::Ambiguous);
    }

    #[test]
    fn corroborated_family_mismatch_is_asserted() {
        let results = vec![
            p("self_id", Group::Identity, Status::Fail, 2)
                .metric("observed_family", "openai")
                .metric("marker_hits", 3),
            p("meta_creator", Group::Identity, Status::Pass, 1).metric("observed_family", "openai"),
        ];
        let id = build_identity(&results, "claude-opus-4-5", L);
        assert_eq!(id.observed_family.as_deref(), Some("openai"));
        assert!(id.family_confidence >= 0.5);
        assert_eq!(id.status, IdentityStatus::FamilyMismatch);
    }

    #[test]
    fn disagreeing_creator_answer_lowers_family_confidence() {
        let results = vec![
            p("self_id", Group::Identity, Status::Pass, 2)
                .metric("observed_family", "anthropic")
                .metric("marker_hits", 3),
            p("meta_creator", Group::Identity, Status::Pass, 1).metric("observed_family", "openai"),
        ];
        let id = build_identity(&results, "claude-opus-4-5", L);
        assert!(id.family_confidence < 0.7);
        assert!(id.evidence.iter().any(|e| e.contains("disagreeing")));
    }

    #[test]
    fn narrow_tier_margin_withdraws_the_severity_claim() {
        let results = vec![p("tier_estimate", Group::Identity, Status::Fail, 3)
            .metric("estimated_tier", "small")
            .metric("fit", 0.8)
            .metric("tier_severity", 2)
            .metric("margin", 0.01)];
        let id = build_identity(&results, "claude-opus-4-5", L);
        assert_eq!(id.tier_severity, 0, "an overlapping margin must not accuse");
        assert!(hard_gates(&results, &id, L).is_empty());
    }

    #[test]
    fn wide_tier_margin_keeps_the_severity_claim() {
        let results = vec![p("tier_estimate", Group::Identity, Status::Fail, 3)
            .metric("estimated_tier", "small")
            .metric("fit", 0.8)
            .metric("tier_severity", 2)
            .metric("margin", 0.3)];
        let id = build_identity(&results, "claude-opus-4-5", L);
        assert_eq!(id.tier_severity, 2);
        assert_eq!(id.status, IdentityStatus::TierMismatch);
    }

    #[test]
    fn unreachable_endpoint_is_inconclusive_not_counterfeit() {
        let results = vec![p("preflight", Group::Contract, Status::Fail, 3)];
        let v = decide(
            &results,
            &Identity::default(),
            &BillingAudit::default(),
            &ChannelSignature::default(),
            Protocol::Anthropic,
            L,
        );
        assert_eq!(v.authenticity, Authenticity::Inconclusive);
    }

    #[test]
    fn clean_official_endpoint_reads_as_authentic() {
        let mut results = vec![p("preflight", Group::Contract, Status::Pass, 3)];
        for g in [
            Group::Contract,
            Group::Billing,
            Group::Identity,
            Group::Stream,
        ] {
            for i in 0..4 {
                results.push(p(&format!("{}{i}", g.key()), g, Status::Pass, 2));
            }
        }
        let billing = BillingAudit {
            input_ratio: 1.0,
            ..Default::default()
        };
        let sig = ChannelSignature {
            key: crate::probes::channel::ANTHROPIC_OFFICIAL.into(),
            ..Default::default()
        };
        let v = decide(
            &results,
            &Identity::default(),
            &billing,
            &sig,
            Protocol::Anthropic,
            L,
        );
        assert_eq!(v.authenticity, Authenticity::Authentic);
        assert_eq!(v.channel, Channel::Official);
        assert!(v.score >= 90.0);
    }

    #[test]
    fn a_clean_relay_is_third_party_not_suspicious() {
        let mut results = vec![p("preflight", Group::Contract, Status::Pass, 3)];
        for g in [
            Group::Contract,
            Group::Billing,
            Group::Identity,
            Group::Stream,
        ] {
            for i in 0..4 {
                results.push(p(&format!("{}{i}", g.key()), g, Status::Pass, 2));
            }
        }
        let sig = ChannelSignature {
            key: "OpenRouter".into(),
            ..Default::default()
        };
        let v = decide(
            &results,
            &Identity::default(),
            &BillingAudit {
                input_ratio: 1.02,
                ..Default::default()
            },
            &sig,
            Protocol::Anthropic,
            L,
        );
        assert_eq!(v.authenticity, Authenticity::ThirdParty);
        assert_eq!(v.channel, Channel::Proxy);
    }

    #[test]
    fn high_coverage_gap_forces_inconclusive() {
        let mut results = vec![p("preflight", Group::Contract, Status::Pass, 3)];
        for i in 0..6 {
            results.push(p(&format!("e{i}"), Group::Contract, Status::Error, 1));
        }
        for i in 0..3 {
            results.push(p(&format!("ok{i}"), Group::Contract, Status::Pass, 1));
        }
        let v = decide(
            &results,
            &Identity::default(),
            &BillingAudit::default(),
            &ChannelSignature::default(),
            Protocol::Anthropic,
            L,
        );
        assert_eq!(v.authenticity, Authenticity::Inconclusive);
        assert!(v.coverage_gap > 0.25);
    }

    #[test]
    fn confidence_stays_inside_its_band() {
        let v = decide(
            &[p("preflight", Group::Contract, Status::Error, 3)],
            &Identity::default(),
            &BillingAudit::default(),
            &ChannelSignature::default(),
            Protocol::Anthropic,
            L,
        );
        assert!((0.15..=0.95).contains(&v.confidence), "{}", v.confidence);
    }
}

#[cfg(test)]
mod reverse_weight_tests {
    use super::*;
    use crate::protocol::Protocol;

    const L: Lang = Lang::En;

    fn p(id: &str, s: Status) -> ProbeResult {
        let mut r = ProbeResult::new(id, id, Group::Contract);
        r.status = s;
        r
    }

    #[test]
    fn weak_signals_alone_stay_below_the_threshold() {
        // The exact shape of an honest relay: a non-JSON error body and no
        // vendor headers. It must not be labelled a reverse channel.
        let results = vec![
            p("error_envelope", Status::Warn),
            p("official_headers", Status::Warn),
        ];
        let (list, weight) = reverse_signals(&results, Protocol::OpenAI, L);
        assert_eq!(list.len(), 2);
        assert!(weight < REVERSE_THRESHOLD, "weight was {weight}");
    }

    #[test]
    fn two_capability_absences_reach_the_threshold() {
        let results = vec![
            p("usage_present", Status::Fail),
            p("max_tokens", Status::Fail),
        ];
        let (_, weight) = reverse_signals(&results, Protocol::OpenAI, L);
        assert!(weight >= REVERSE_THRESHOLD, "weight was {weight}");
    }

    #[test]
    fn one_strong_plus_two_weak_also_reaches_it() {
        let results = vec![
            p("system_adherence", Status::Fail),
            p("error_envelope", Status::Warn),
            p("official_headers", Status::Warn),
        ];
        let (_, weight) = reverse_signals(&results, Protocol::OpenAI, L);
        assert!(weight >= REVERSE_THRESHOLD, "weight was {weight}");
    }
}
