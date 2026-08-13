// SPDX-License-Identifier: Apache-2.0
//! Channel provenance — which relays this request actually passed through.
//!
//! Costs nothing extra: every signal is read out of response headers, message
//! IDs and bodies that earlier probes already collected. Relay software is
//! chatty about itself in headers, so a three-tier classifier gets a long way.
//!
//! Tier 1 — a vendor-exclusive header prefix or ID format. Decisive.
//! Tier 2 — shared infrastructure, scored across several weaker signals.
//! Tier 3 — a native-looking ID with nothing else. Inferred transparent relay.

use super::Ctx;
use crate::report::{Group, ProbeResult};
use std::collections::BTreeMap;

const G: Group = Group::Channel;

/// `(display name, id-prefix, header-prefix, exact-header)`. A match on any
/// populated field is decisive for that vendor.
const TIER1: &[(&str, &str, &str, &str)] = &[
    ("OpenRouter", "gen-", "", "x-generation-id"),
    ("Cloudflare AI Gateway", "", "cf-aig-", ""),
    ("Azure AI Foundry", "", "", "apim-request-id"),
    ("LiteLLM", "", "x-litellm-", ""),
    ("Helicone", "", "helicone-", ""),
    ("Portkey", "", "x-portkey-", ""),
    ("Kong Gateway", "", "x-kong-", ""),
    ("Alibaba DashScope", "", "x-dashscope-", ""),
    ("New-API", "", "", "x-new-api-version"),
    ("One-API", "", "", "x-oneapi-request-id"),
    ("Fastly", "", "", "x-served-by"),
];

pub async fn run(ctx: &Ctx) -> Vec<ProbeResult> {
    let headers = merge_headers(&ctx.headers.borrow());
    let ids = ctx.message_ids.borrow().clone();
    let bodies = ctx.raw_bodies.borrow().join("\n");
    vec![
        classify(&headers, &ids, &bodies),
        official_headers(&headers),
        multi_hop(&headers, &ids),
    ]
}

/// Union of every header seen this run. A relay that only stamps some
/// responses still gets caught.
fn merge_headers(all: &[BTreeMap<String, String>]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for h in all {
        for (k, v) in h {
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub label: String,
    pub confidence: f64,
    pub tier: u8,
    pub evidence: Vec<String>,
    pub hops: Vec<String>,
}

/// Pure classifier, separated from the probe wrapper so it is directly testable.
pub fn classify_signals(
    headers: &BTreeMap<String, String>,
    ids: &[String],
    body: &str,
) -> Classification {
    let mut evidence = Vec::new();
    let mut hops = Vec::new();

    // ── Tier 1 ─────────────────────────────────────────────────────────────
    for (name, id_prefix, header_prefix, exact) in TIER1 {
        let mut hit: Option<String> = None;
        if !id_prefix.is_empty() && ids.iter().any(|i| i.starts_with(id_prefix)) {
            hit = Some(format!("消息 ID 前缀 {id_prefix}"));
        }
        if hit.is_none() && !header_prefix.is_empty() {
            if let Some(k) = headers.keys().find(|k| k.starts_with(header_prefix)) {
                hit = Some(format!("响应头 {k}"));
            }
        }
        if hit.is_none() && !exact.is_empty() && headers.contains_key(*exact) {
            hit = Some(format!("响应头 {exact}"));
        }
        if let Some(why) = hit {
            hops.push(name.to_string());
            evidence.push(format!("{name} — {why}"));
        }
    }
    if let Some(first) = hops.first() {
        return Classification {
            label: first.clone(),
            confidence: 1.0,
            tier: 1,
            evidence,
            hops,
        };
    }

    // ── Tier 2 ─────────────────────────────────────────────────────────────
    let mut scores: BTreeMap<&str, f64> = BTreeMap::new();
    let mut bump = |k: &'static str, w: f64, why: String, ev: &mut Vec<String>| {
        *scores.entry(k).or_insert(0.0) += w;
        ev.push(why);
    };

    if let Some(k) = headers.keys().find(|k| k.starts_with("x-amzn-bedrock-")) {
        bump("AWS Bedrock", 1.0, format!("响应头 {k}"), &mut evidence);
    }
    if ids.iter().any(|i| i.starts_with("msg_bdrk_")) {
        bump(
            "AWS Bedrock",
            1.0,
            "消息 ID 前缀 msg_bdrk_".into(),
            &mut evidence,
        );
    }
    if body.contains("bedrock-2023-05-31") {
        bump(
            "AWS Bedrock",
            0.9,
            "body 含 bedrock-2023-05-31".into(),
            &mut evidence,
        );
    }
    if ids.iter().any(|i| i.starts_with("msg_vrtx_")) {
        bump(
            "Google Vertex",
            1.0,
            "消息 ID 前缀 msg_vrtx_".into(),
            &mut evidence,
        );
    }
    if body.contains("vertex-2023-10-16") {
        bump(
            "Google Vertex",
            0.9,
            "body 含 vertex-2023-10-16".into(),
            &mut evidence,
        );
    }
    if let Some(k) = headers.keys().find(|k| k.starts_with("x-goog-")) {
        bump("Google Vertex", 1.0, format!("响应头 {k}"), &mut evidence);
    }
    if headers
        .get("server")
        .map(|s| s.to_ascii_lowercase().contains("google"))
        .unwrap_or(false)
    {
        // A server banner is self-reported and trivially spoofed; weight it low.
        bump(
            "Google Vertex",
            0.5,
            "Server 头含 google".into(),
            &mut evidence,
        );
    }
    if headers.contains_key("x-amz-apigw-id") || headers.contains_key("apigw-requestid") {
        bump(
            "AWS API Gateway",
            0.8,
            "响应头 x-amz-apigw-id".into(),
            &mut evidence,
        );
    }
    if headers.keys().any(|k| {
        k.starts_with("anthropic-ratelimit-")
            || k.starts_with("anthropic-priority-")
            || k.starts_with("anthropic-fast-")
    }) {
        bump(
            "Anthropic 官方",
            0.95,
            "响应头 anthropic-ratelimit-* / priority-*".into(),
            &mut evidence,
        );
    }
    if headers
        .get("request-id")
        .map(|r| r.starts_with("req_"))
        .unwrap_or(false)
    {
        bump(
            "Anthropic 官方",
            0.6,
            "request-id 前缀 req_".into(),
            &mut evidence,
        );
    }
    if headers.contains_key("openai-organization") || headers.contains_key("openai-processing-ms") {
        bump("OpenAI 官方", 0.9, "响应头 openai-*".into(), &mut evidence);
    }

    if let Some((&winner, &score)) = scores
        .iter()
        // Deterministic tie-break by name so the same inputs always classify
        // the same way.
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap().then(b.0.cmp(a.0)))
    {
        if score > 0.0 {
            hops = scores.keys().map(|k| k.to_string()).collect();
            return Classification {
                label: winner.to_string(),
                confidence: score.min(1.0),
                tier: 2,
                evidence,
                hops,
            };
        }
    }

    // ── Tier 3 ─────────────────────────────────────────────────────────────
    let native_anthropic = ids.iter().any(|i| i.starts_with("msg_01") && i.len() >= 20);
    if native_anthropic {
        evidence.push("原生格式的 Anthropic 消息 ID，但没有任何官方响应头".into());
        return Classification {
            label: "透明中继".into(),
            confidence: 0.5,
            tier: 3,
            evidence,
            hops: vec!["透明中继".into()],
        };
    }
    Classification {
        label: "未知代理".into(),
        confidence: 0.0,
        tier: 3,
        evidence,
        hops: Vec::new(),
    }
}

fn classify(headers: &BTreeMap<String, String>, ids: &[String], body: &str) -> ProbeResult {
    let c = classify_signals(headers, ids, body);
    let mut p = ProbeResult::new("channel_signature", "渠道签名识别", G)
        .weight(2)
        .neutral()
        .metric("channel", c.label.clone())
        .metric("tier", c.tier)
        .metric("confidence", c.confidence)
        .metric("headers_seen", headers.len())
        .metric("hops", c.hops.join(" → "));
    for e in &c.evidence {
        p = p.finding(e.clone());
    }
    match c.tier {
        1 => p.pass(format!("确定性识别：{}", c.label)),
        2 => p.pass(format!(
            "{}（置信度 {:.0}%）",
            c.label,
            c.confidence * 100.0
        )),
        _ if c.confidence > 0.0 => p.warn(format!("推断为{}", c.label)),
        _ => p.warn("没有任何渠道特征，来源无法确定"),
    }
}

fn official_headers(headers: &BTreeMap<String, String>) -> ProbeResult {
    let p = ProbeResult::new("official_headers", "官方响应头指纹", G).weight(1);
    let markers: Vec<&str> = [
        "anthropic-ratelimit-requests-limit",
        "anthropic-ratelimit-tokens-limit",
        "openai-organization",
        "openai-processing-ms",
        "x-ratelimit-limit-requests",
        "cf-ray",
        "request-id",
    ]
    .iter()
    .filter(|m| headers.contains_key(**m) || headers.keys().any(|k| k.starts_with(*m)))
    .copied()
    .collect();

    let p = p
        .metric("marker_count", markers.len())
        .metric("markers", markers.join(", "));

    if markers.len() >= 3 {
        p.pass(format!("检出 {} 项官方特征头", markers.len()))
    } else if markers.is_empty() {
        p.warn("没有任何官方特征响应头")
            .finding("中转层通常会剥掉这些头；这本身不证明是假的，但少了一条佐证")
    } else {
        p.warn(format!("只有 {} 项官方特征头", markers.len()))
    }
}

fn multi_hop(headers: &BTreeMap<String, String>, ids: &[String]) -> ProbeResult {
    let p = ProbeResult::new("multi_hop", "多跳转发检测", G)
        .weight(1)
        .neutral();
    let mut vendors: Vec<String> = Vec::new();
    for (name, id_prefix, header_prefix, exact) in TIER1 {
        let hit = (!id_prefix.is_empty() && ids.iter().any(|i| i.starts_with(id_prefix)))
            || (!header_prefix.is_empty() && headers.keys().any(|k| k.starts_with(header_prefix)))
            || (!exact.is_empty() && headers.contains_key(*exact));
        if hit {
            vendors.push(name.to_string());
        }
    }
    // A generic forwarding header is one more hop even without a vendor name.
    let generic = ["via", "x-forwarded-for", "x-forwarded-host", "forwarded"]
        .iter()
        .filter(|k| headers.contains_key(**k))
        .count();

    let p = p
        .metric("vendor_hops", vendors.len())
        .metric("generic_forward_headers", generic)
        .metric("vendors", vendors.join(" → "));

    if vendors.len() >= 2 {
        p.fail(format!(
            "检出 {} 层中转：{}",
            vendors.len(),
            vendors.join(" → ")
        ))
        .finding("请求在到达模型前被转发了不止一次，每一跳都有机会改写内容与计费")
    } else if vendors.len() == 1 && generic > 0 {
        p.warn(format!("{} + {generic} 个通用转发头", vendors[0]))
    } else if vendors.len() == 1 {
        p.pass(format!("单层中转：{}", vendors[0]))
    } else {
        p.pass("未检出多跳转发")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn tier1_id_prefix_is_decisive() {
        let c = classify_signals(&hdrs(&[]), &["gen-abc123".into()], "");
        assert_eq!(c.label, "OpenRouter");
        assert_eq!(c.tier, 1);
        assert_eq!(c.confidence, 1.0);
    }

    #[test]
    fn tier1_header_prefix_is_decisive() {
        let c = classify_signals(&hdrs(&[("cf-aig-cache-status", "MISS")]), &[], "");
        assert_eq!(c.label, "Cloudflare AI Gateway");
        assert_eq!(c.tier, 1);

        let c = classify_signals(&hdrs(&[("x-litellm-version", "1.0")]), &[], "");
        assert_eq!(c.label, "LiteLLM");

        let c = classify_signals(&hdrs(&[("x-new-api-version", "0.6")]), &[], "");
        assert_eq!(c.label, "New-API");
    }

    #[test]
    fn tier2_scores_bedrock_across_signals() {
        let c = classify_signals(
            &hdrs(&[("x-amzn-bedrock-input-token-count", "12")]),
            &["msg_bdrk_01xyz".into()],
            "\"anthropic_version\":\"bedrock-2023-05-31\"",
        );
        assert_eq!(c.label, "AWS Bedrock");
        assert_eq!(c.tier, 2);
        assert_eq!(c.confidence, 1.0, "multiple signals clamp at 1.0");
        assert!(c.evidence.len() >= 3);
    }

    #[test]
    fn tier2_recognises_anthropic_official() {
        let c = classify_signals(
            &hdrs(&[
                ("anthropic-ratelimit-requests-limit", "50"),
                ("request-id", "req_011AB"),
            ]),
            &["msg_01ABCDEFGHIJKLMNOPQRSTU".into()],
            "",
        );
        assert_eq!(c.label, "Anthropic 官方");
        assert_eq!(c.tier, 2);
    }

    #[test]
    fn tier3_infers_transparent_relay_from_native_id_alone() {
        let c = classify_signals(&hdrs(&[]), &["msg_01ABCDEFGHIJKLMNOPQRSTU".into()], "");
        assert_eq!(c.label, "透明中继");
        assert_eq!(c.tier, 3);
        assert_eq!(c.confidence, 0.5);
    }

    #[test]
    fn no_signals_at_all_yields_unknown_not_a_guess() {
        let c = classify_signals(&hdrs(&[("content-type", "application/json")]), &[], "");
        assert_eq!(c.label, "未知代理");
        assert_eq!(c.confidence, 0.0);
        assert!(c.hops.is_empty());
    }

    #[test]
    fn a_short_msg_01_id_does_not_count_as_native() {
        // Guards against a relay minting "msg_01" + a few chars to look native.
        let c = classify_signals(&hdrs(&[]), &["msg_01short".into()], "");
        assert_eq!(c.label, "未知代理");
    }

    #[test]
    fn multi_hop_flags_two_vendors() {
        let r = multi_hop(
            &hdrs(&[("x-litellm-version", "1"), ("helicone-id", "h1")]),
            &[],
        );
        assert_eq!(r.status, crate::report::Status::Fail);
        assert_eq!(r.metric_f64("vendor_hops"), Some(2.0));
    }

    #[test]
    fn single_vendor_hop_is_not_a_failure() {
        let r = multi_hop(&hdrs(&[("x-litellm-version", "1")]), &[]);
        assert_eq!(r.status, crate::report::Status::Pass);
    }

    #[test]
    fn merge_headers_unions_across_responses() {
        let merged = merge_headers(&[
            hdrs(&[("a", "1")]),
            hdrs(&[("b", "2")]),
            hdrs(&[("a", "override-ignored")]),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged["a"], "1", "first value wins");
    }

    #[test]
    fn classification_is_deterministic_for_tied_tier2_scores() {
        let h = hdrs(&[("x-amz-apigw-id", "x"), ("request-id", "req_1")]);
        let a = classify_signals(&h, &[], "");
        let b = classify_signals(&h, &[], "");
        assert_eq!(a.label, b.label);
    }
}
