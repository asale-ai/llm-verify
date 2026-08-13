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
//!
//! Channels are identified by a **stable key**, never by their display name.
//! The verdict layer routes on those keys, so a translated label can never
//! change how an endpoint gets classified.

use super::Ctx;
use crate::i18n::Lang;
use crate::report::{Group, ProbeResult};
use std::collections::BTreeMap;

const G: Group = Group::Channel;

// ── stable channel keys ────────────────────────────────────────────────────

pub const ANTHROPIC_OFFICIAL: &str = "anthropic-official";
pub const OPENAI_OFFICIAL: &str = "openai-official";
pub const AWS_BEDROCK: &str = "aws-bedrock";
pub const GOOGLE_VERTEX: &str = "google-vertex";
pub const AWS_APIGATEWAY: &str = "aws-apigateway";
pub const AZURE_FOUNDRY: &str = "azure-foundry";
pub const TRANSPARENT_RELAY: &str = "transparent-relay";
pub const UNKNOWN_PROXY: &str = "unknown-proxy";

/// Human-readable name for a channel key.
///
/// Most relay vendors are proper nouns and read identically in both languages;
/// only the descriptive keys need translating.
pub fn display(key: &str, lang: Lang) -> String {
    match key {
        ANTHROPIC_OFFICIAL => t!(lang, "Anthropic (first-party)", "Anthropic 官方"),
        OPENAI_OFFICIAL => t!(lang, "OpenAI (first-party)", "OpenAI 官方"),
        AWS_BEDROCK => "AWS Bedrock".to_string(),
        GOOGLE_VERTEX => "Google Vertex".to_string(),
        AWS_APIGATEWAY => "AWS API Gateway".to_string(),
        AZURE_FOUNDRY => "Azure AI Foundry".to_string(),
        TRANSPARENT_RELAY => t!(lang, "Transparent relay", "透明中继"),
        UNKNOWN_PROXY => t!(lang, "Unknown proxy", "未知代理"),
        other => other.to_string(),
    }
}

/// `(key, id-prefix, header-prefix, exact-header)`. A match on any populated
/// field is decisive for that vendor.
const TIER1: &[(&str, &str, &str, &str)] = &[
    ("OpenRouter", "gen-", "", "x-generation-id"),
    ("Cloudflare AI Gateway", "", "cf-aig-", ""),
    (AZURE_FOUNDRY, "", "", "apim-request-id"),
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
        classify(&headers, &ids, &bodies, ctx.lang),
        official_headers(&headers, ctx.lang),
        multi_hop(&headers, &ids, ctx.lang),
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
    /// Stable key, never localised.
    pub key: String,
    pub confidence: f64,
    pub tier: u8,
    /// Evidence sentences, already in the caller's language.
    pub evidence: Vec<String>,
    pub hops: Vec<String>,
}

/// Pure classifier, separated from the probe wrapper so it is directly testable.
pub fn classify_signals(
    headers: &BTreeMap<String, String>,
    ids: &[String],
    body: &str,
    l: Lang,
) -> Classification {
    let mut evidence = Vec::new();
    let mut hop_keys: Vec<&str> = Vec::new();

    // ── Tier 1 ─────────────────────────────────────────────────────────────
    for (key, id_prefix, header_prefix, exact) in TIER1 {
        let mut hit: Option<String> = None;
        if !id_prefix.is_empty() && ids.iter().any(|i| i.starts_with(id_prefix)) {
            hit = Some(t!(
                l,
                "message ID prefix {id_prefix}",
                "消息 ID 前缀 {id_prefix}"
            ));
        }
        if hit.is_none() && !header_prefix.is_empty() {
            if let Some(k) = headers.keys().find(|k| k.starts_with(header_prefix)) {
                hit = Some(t!(l, "response header {k}", "响应头 {k}"));
            }
        }
        if hit.is_none() && !exact.is_empty() && headers.contains_key(*exact) {
            hit = Some(t!(l, "response header {exact}", "响应头 {exact}"));
        }
        if let Some(why) = hit {
            hop_keys.push(key);
            evidence.push(format!("{} — {why}", display(key, l)));
        }
    }
    if let Some(first) = hop_keys.first() {
        return Classification {
            key: first.to_string(),
            confidence: 1.0,
            tier: 1,
            evidence,
            hops: hop_keys.iter().map(|k| display(k, l)).collect(),
        };
    }

    // ── Tier 2 ─────────────────────────────────────────────────────────────
    let mut scores: BTreeMap<&str, f64> = BTreeMap::new();
    let mut bump = |k: &'static str, w: f64, why: String, ev: &mut Vec<String>| {
        *scores.entry(k).or_insert(0.0) += w;
        ev.push(why);
    };

    if let Some(k) = headers.keys().find(|k| k.starts_with("x-amzn-bedrock-")) {
        bump(
            AWS_BEDROCK,
            1.0,
            t!(l, "response header {k}", "响应头 {k}"),
            &mut evidence,
        );
    }
    if ids.iter().any(|i| i.starts_with("msg_bdrk_")) {
        bump(
            AWS_BEDROCK,
            1.0,
            t!(l, "message ID prefix msg_bdrk_", "消息 ID 前缀 msg_bdrk_"),
            &mut evidence,
        );
    }
    if body.contains("bedrock-2023-05-31") {
        bump(
            AWS_BEDROCK,
            0.9,
            t!(
                l,
                "body contains bedrock-2023-05-31",
                "body 含 bedrock-2023-05-31"
            ),
            &mut evidence,
        );
    }
    if ids.iter().any(|i| i.starts_with("msg_vrtx_")) {
        bump(
            GOOGLE_VERTEX,
            1.0,
            t!(l, "message ID prefix msg_vrtx_", "消息 ID 前缀 msg_vrtx_"),
            &mut evidence,
        );
    }
    if body.contains("vertex-2023-10-16") {
        bump(
            GOOGLE_VERTEX,
            0.9,
            t!(
                l,
                "body contains vertex-2023-10-16",
                "body 含 vertex-2023-10-16"
            ),
            &mut evidence,
        );
    }
    if let Some(k) = headers.keys().find(|k| k.starts_with("x-goog-")) {
        bump(
            GOOGLE_VERTEX,
            1.0,
            t!(l, "response header {k}", "响应头 {k}"),
            &mut evidence,
        );
    }
    if headers
        .get("server")
        .map(|s| s.to_ascii_lowercase().contains("google"))
        .unwrap_or(false)
    {
        // A server banner is self-reported and trivially spoofed; weight it low.
        bump(
            GOOGLE_VERTEX,
            0.5,
            t!(l, "Server header contains google", "Server 头含 google"),
            &mut evidence,
        );
    }
    if headers.contains_key("x-amz-apigw-id") || headers.contains_key("apigw-requestid") {
        bump(
            AWS_APIGATEWAY,
            0.8,
            t!(l, "response header x-amz-apigw-id", "响应头 x-amz-apigw-id"),
            &mut evidence,
        );
    }
    if headers.keys().any(|k| {
        k.starts_with("anthropic-ratelimit-")
            || k.starts_with("anthropic-priority-")
            || k.starts_with("anthropic-fast-")
    }) {
        bump(
            ANTHROPIC_OFFICIAL,
            0.95,
            t!(
                l,
                "response headers anthropic-ratelimit-* / priority-*",
                "响应头 anthropic-ratelimit-* / priority-*"
            ),
            &mut evidence,
        );
    }
    if headers
        .get("request-id")
        .map(|r| r.starts_with("req_"))
        .unwrap_or(false)
    {
        bump(
            ANTHROPIC_OFFICIAL,
            0.6,
            t!(l, "request-id prefixed req_", "request-id 前缀 req_"),
            &mut evidence,
        );
    }
    if headers.contains_key("openai-organization") || headers.contains_key("openai-processing-ms") {
        bump(
            OPENAI_OFFICIAL,
            0.9,
            t!(l, "response headers openai-*", "响应头 openai-*"),
            &mut evidence,
        );
    }

    if let Some((&winner, &score)) = scores
        .iter()
        // Deterministic tie-break by key so the same inputs always classify
        // the same way.
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap().then(b.0.cmp(a.0)))
    {
        if score > 0.0 {
            return Classification {
                key: winner.to_string(),
                confidence: score.min(1.0),
                tier: 2,
                evidence,
                hops: scores.keys().map(|k| display(k, l)).collect(),
            };
        }
    }

    // ── Tier 3 ─────────────────────────────────────────────────────────────
    let native_anthropic = ids.iter().any(|i| i.starts_with("msg_01") && i.len() >= 20);
    if native_anthropic {
        evidence.push(t!(
            l,
            "A native-format Anthropic message ID with no vendor headers at all",
            "原生格式的 Anthropic 消息 ID，但没有任何官方响应头"
        ));
        return Classification {
            key: TRANSPARENT_RELAY.to_string(),
            confidence: 0.5,
            tier: 3,
            evidence,
            hops: vec![display(TRANSPARENT_RELAY, l)],
        };
    }
    Classification {
        key: UNKNOWN_PROXY.to_string(),
        confidence: 0.0,
        tier: 3,
        evidence,
        hops: Vec::new(),
    }
}

fn classify(
    headers: &BTreeMap<String, String>,
    ids: &[String],
    body: &str,
    l: Lang,
) -> ProbeResult {
    let c = classify_signals(headers, ids, body, l);
    let name = display(&c.key, l);
    let mut p = ProbeResult::new(
        "channel_signature",
        ts!(l, "Channel signature", "渠道签名识别"),
        G,
    )
    .weight(2)
    .neutral()
    .metric("channel", c.key.clone())
    .metric("channel_display", name.clone())
    .metric("tier", c.tier)
    .metric("confidence", c.confidence)
    .metric("headers_seen", headers.len())
    .metric("hops", c.hops.join(" -> "));
    for e in &c.evidence {
        p = p.finding(e.clone());
    }
    match c.tier {
        1 => p.pass(t!(l, "Identified decisively: {name}", "确定性识别：{name}")),
        2 => p.pass(t!(
            l,
            "{name} ({:.0}% confidence)",
            "{name}（置信度 {:.0}%）",
            c.confidence * 100.0
        )),
        _ if c.confidence > 0.0 => p.warn(t!(l, "Inferred as {name}", "推断为{name}")),
        _ => p.warn(t!(
            l,
            "No channel markers at all; the origin cannot be determined",
            "没有任何渠道特征，来源无法确定"
        )),
    }
}

fn official_headers(headers: &BTreeMap<String, String>, l: Lang) -> ProbeResult {
    let p = ProbeResult::new(
        "official_headers",
        ts!(l, "Vendor header fingerprint", "官方响应头指纹"),
        G,
    )
    .weight(1);
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
        p.pass(t!(
            l,
            "{} vendor marker headers found",
            "检出 {} 项官方特征头",
            markers.len()
        ))
    } else if markers.is_empty() {
        p.warn(t!(
            l,
            "No vendor marker headers at all",
            "没有任何官方特征响应头"
        ))
        .finding(t!(
            l,
            "Relays commonly strip these. It does not prove anything is fake, \
             but one corroborating signal is gone",
            "中转层通常会剥掉这些头；这本身不证明是假的，但少了一条佐证"
        ))
    } else {
        p.warn(t!(
            l,
            "Only {} vendor marker header(s)",
            "只有 {} 项官方特征头",
            markers.len()
        ))
    }
}

fn multi_hop(headers: &BTreeMap<String, String>, ids: &[String], l: Lang) -> ProbeResult {
    let p = ProbeResult::new(
        "multi_hop",
        ts!(l, "Multi-hop forwarding", "多跳转发检测"),
        G,
    )
    .weight(1)
    .neutral();
    let mut vendors: Vec<String> = Vec::new();
    for (key, id_prefix, header_prefix, exact) in TIER1 {
        let hit = (!id_prefix.is_empty() && ids.iter().any(|i| i.starts_with(id_prefix)))
            || (!header_prefix.is_empty() && headers.keys().any(|k| k.starts_with(header_prefix)))
            || (!exact.is_empty() && headers.contains_key(*exact));
        if hit {
            vendors.push(display(key, l));
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
        .metric("vendors", vendors.join(" -> "));

    if vendors.len() >= 2 {
        p.fail(t!(
            l,
            "{} relay hops detected: {}",
            "检出 {} 层中转：{}",
            vendors.len(),
            vendors.join(" -> ")
        ))
        .finding(t!(
            l,
            "The request is forwarded more than once before reaching the model; \
             every hop can rewrite content and billing",
            "请求在到达模型前被转发了不止一次，每一跳都有机会改写内容与计费"
        ))
    } else if vendors.len() == 1 && generic > 0 {
        p.warn(t!(
            l,
            "{} plus {generic} generic forwarding header(s)",
            "{} + {generic} 个通用转发头",
            vendors[0]
        ))
    } else if vendors.len() == 1 {
        p.pass(t!(l, "Single relay hop: {}", "单层中转：{}", vendors[0]))
    } else {
        p.pass(t!(l, "No multi-hop forwarding detected", "未检出多跳转发"))
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

    const L: Lang = Lang::En;

    #[test]
    fn tier1_id_prefix_is_decisive() {
        let c = classify_signals(&hdrs(&[]), &["gen-abc123".into()], "", L);
        assert_eq!(c.key, "OpenRouter");
        assert_eq!(c.tier, 1);
        assert_eq!(c.confidence, 1.0);
    }

    #[test]
    fn tier1_header_prefix_is_decisive() {
        let c = classify_signals(&hdrs(&[("cf-aig-cache-status", "MISS")]), &[], "", L);
        assert_eq!(c.key, "Cloudflare AI Gateway");
        assert_eq!(c.tier, 1);

        let c = classify_signals(&hdrs(&[("x-litellm-version", "1.0")]), &[], "", L);
        assert_eq!(c.key, "LiteLLM");

        let c = classify_signals(&hdrs(&[("x-new-api-version", "0.6")]), &[], "", L);
        assert_eq!(c.key, "New-API");
    }

    #[test]
    fn tier2_scores_bedrock_across_signals() {
        let c = classify_signals(
            &hdrs(&[("x-amzn-bedrock-input-token-count", "12")]),
            &["msg_bdrk_01xyz".into()],
            "\"anthropic_version\":\"bedrock-2023-05-31\"",
            L,
        );
        assert_eq!(c.key, AWS_BEDROCK);
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
            L,
        );
        assert_eq!(c.key, ANTHROPIC_OFFICIAL);
        assert_eq!(c.tier, 2);
    }

    #[test]
    fn tier3_infers_transparent_relay_from_native_id_alone() {
        let c = classify_signals(&hdrs(&[]), &["msg_01ABCDEFGHIJKLMNOPQRSTU".into()], "", L);
        assert_eq!(c.key, TRANSPARENT_RELAY);
        assert_eq!(c.tier, 3);
        assert_eq!(c.confidence, 0.5);
    }

    #[test]
    fn no_signals_at_all_yields_unknown_not_a_guess() {
        let c = classify_signals(&hdrs(&[("content-type", "application/json")]), &[], "", L);
        assert_eq!(c.key, UNKNOWN_PROXY);
        assert_eq!(c.confidence, 0.0);
        assert!(c.hops.is_empty());
    }

    #[test]
    fn a_short_msg_01_id_does_not_count_as_native() {
        // Guards against a relay minting "msg_01" + a few chars to look native.
        let c = classify_signals(&hdrs(&[]), &["msg_01short".into()], "", L);
        assert_eq!(c.key, UNKNOWN_PROXY);
    }

    #[test]
    fn the_key_never_changes_with_the_display_language() {
        // The verdict layer routes on this key, so a translation must not be
        // able to alter how an endpoint is classified.
        let h = hdrs(&[("anthropic-ratelimit-requests-limit", "50")]);
        let en = classify_signals(&h, &[], "", Lang::En);
        let zh = classify_signals(&h, &[], "", Lang::Zh);
        assert_eq!(en.key, zh.key);
        assert_eq!(en.tier, zh.tier);
        assert_eq!(en.confidence, zh.confidence);
        // ...while the evidence itself is localised.
        assert_ne!(en.evidence, zh.evidence);
    }

    #[test]
    fn vendor_names_are_proper_nouns_and_do_not_translate() {
        assert_eq!(display("LiteLLM", Lang::Zh), "LiteLLM");
        assert_eq!(display("OpenRouter", Lang::En), "OpenRouter");
        assert_eq!(display(AWS_BEDROCK, Lang::Zh), "AWS Bedrock");
        // Descriptive keys do translate.
        assert_ne!(
            display(TRANSPARENT_RELAY, Lang::En),
            display(TRANSPARENT_RELAY, Lang::Zh)
        );
    }

    #[test]
    fn multi_hop_flags_two_vendors() {
        let r = multi_hop(
            &hdrs(&[("x-litellm-version", "1"), ("helicone-id", "h1")]),
            &[],
            L,
        );
        assert_eq!(r.status, crate::report::Status::Fail);
        assert_eq!(r.metric_f64("vendor_hops"), Some(2.0));
    }

    #[test]
    fn single_vendor_hop_is_not_a_failure() {
        let r = multi_hop(&hdrs(&[("x-litellm-version", "1")]), &[], L);
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
        assert_eq!(
            classify_signals(&h, &[], "", L).key,
            classify_signals(&h, &[], "", L).key
        );
    }
}
