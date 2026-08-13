// SPDX-License-Identifier: Apache-2.0
//! The data model every probe writes into and every output format reads from.

use crate::protocol::Protocol;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    /// The endpoint does not offer what this probe needs. Not a defect.
    Skip,
    /// The probe itself could not run (network, timeout). Coverage gap.
    Error,
}

impl Status {
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
            Self::Skip => "–",
            Self::Error => "?",
        }
    }

    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Pass => "通过",
            Self::Warn => "警告",
            Self::Fail => "失败",
            Self::Skip => "跳过",
            Self::Error => "错误",
        }
    }

    pub fn css(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skip => "skip",
            Self::Error => "err",
        }
    }

    /// Whether this outcome counts toward the weighted score at all.
    pub fn scored(&self) -> bool {
        matches!(self, Self::Pass | Self::Warn | Self::Fail)
    }

    pub fn credit(&self) -> f64 {
        match self {
            Self::Pass => 1.0,
            Self::Warn => 0.5,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Contract,
    Stream,
    Billing,
    Channel,
    Perf,
    Identity,
    Consistency,
}

impl Group {
    pub const ALL: [Group; 7] = [
        Group::Contract,
        Group::Stream,
        Group::Billing,
        Group::Channel,
        Group::Perf,
        Group::Identity,
        Group::Consistency,
    ];

    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Contract => "协议契约",
            Self::Stream => "流式传输",
            Self::Billing => "计量计费",
            Self::Channel => "渠道溯源",
            Self::Perf => "性能速度",
            Self::Identity => "模型身份",
            Self::Consistency => "跨请求一致性",
        }
    }

    pub fn blurb_zh(&self) -> &'static str {
        match self {
            Self::Contract => "这是不是一条正牌 API 通道",
            Self::Stream => "流式响应是否符合协议，有没有空 body",
            Self::Billing => "计量数字可信吗，有没有多收钱",
            Self::Channel => "这条链路上有哪些中转",
            Self::Perf => "首字延迟、吞吐与抖动",
            Self::Identity => "背后跑的是不是它声称的那个模型",
            Self::Consistency => "多次请求的行为是否一致",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Stream => "stream",
            Self::Billing => "billing",
            Self::Channel => "channel",
            Self::Perf => "perf",
            Self::Identity => "identity",
            Self::Consistency => "consistency",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub id: String,
    pub label: String,
    pub group: Group,
    pub status: Status,
    /// Relative importance inside the weighted score.
    pub weight: u32,
    /// Neutral probes gather evidence for the verdict but never move the score.
    pub neutral: bool,
    pub summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, Value>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

impl ProbeResult {
    pub fn new(id: &str, label: &str, group: Group) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            group,
            status: Status::Pass,
            weight: 1,
            neutral: false,
            summary: String::new(),
            findings: Vec::new(),
            metrics: BTreeMap::new(),
            duration_ms: 0,
            evidence: None,
        }
    }

    pub fn weight(mut self, w: u32) -> Self {
        self.weight = w;
        self
    }

    pub fn neutral(mut self) -> Self {
        self.neutral = true;
        self
    }

    pub fn pass(mut self, summary: impl Into<String>) -> Self {
        self.status = Status::Pass;
        self.summary = summary.into();
        self
    }

    pub fn warn(mut self, summary: impl Into<String>) -> Self {
        self.status = Status::Warn;
        self.summary = summary.into();
        self
    }

    pub fn fail(mut self, summary: impl Into<String>) -> Self {
        self.status = Status::Fail;
        self.summary = summary.into();
        self
    }

    pub fn skip(mut self, summary: impl Into<String>) -> Self {
        self.status = Status::Skip;
        self.summary = summary.into();
        self
    }

    pub fn error(mut self, summary: impl Into<String>) -> Self {
        self.status = Status::Error;
        self.summary = summary.into();
        self
    }

    pub fn finding(mut self, f: impl Into<String>) -> Self {
        self.findings.push(f.into());
        self
    }

    pub fn metric(mut self, k: &str, v: impl Into<Value>) -> Self {
        self.metrics.insert(k.to_string(), v.into());
        self
    }

    pub fn evidence(mut self, e: impl Into<String>) -> Self {
        let e = e.into();
        if !e.trim().is_empty() {
            self.evidence = Some(crate::util::truncate(e.trim(), 600));
        }
        self
    }

    pub fn took(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    pub fn metric_f64(&self, k: &str) -> Option<f64> {
        self.metrics.get(k).and_then(|v| v.as_f64())
    }

    pub fn metric_bool(&self, k: &str) -> Option<bool> {
        self.metrics.get(k).and_then(|v| v.as_bool())
    }
}

// ── verdict ────────────────────────────────────────────────────────────────

/// Axis 1 — is the response genuine?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authenticity {
    Authentic,
    AuthenticDegraded,
    ThirdParty,
    Suspicious,
    Counterfeit,
    Inconclusive,
}

impl Authenticity {
    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Authentic => "正品",
            Self::AuthenticDegraded => "正品（有瑕疵）",
            Self::ThirdParty => "第三方转发",
            Self::Suspicious => "存疑",
            Self::Counterfeit => "假冒",
            Self::Inconclusive => "无法判定",
        }
    }

    pub fn desc_zh(&self) -> &'static str {
        match self {
            Self::Authentic => "协议契约、计量与身份信号全部对齐，可以放心使用。",
            Self::AuthenticDegraded => {
                "模型本身看起来是真的，但链路上存在注入、计量偏差或能力缺失。"
            }
            Self::ThirdParty => {
                "行为基本正常，但缺少官方特征、或计量倍率偏高，是经过转发的真模型。"
            }
            Self::Suspicious => "多项异常同时出现，可能被篡改、降级或经过逆向渠道。",
            Self::Counterfeit => "模型回显与自我认同同时对不上，背后大概率不是它声称的模型。",
            Self::Inconclusive => "连通性或数据覆盖不足，不足以下判断。",
        }
    }

    pub fn css(&self) -> &'static str {
        match self {
            Self::Authentic => "v-good",
            Self::AuthenticDegraded | Self::ThirdParty => "v-mid",
            Self::Suspicious => "v-warn",
            Self::Counterfeit => "v-bad",
            Self::Inconclusive => "v-none",
        }
    }
}

/// Axis 2 — where did it come from? Independent of axis 1: a real model
/// behind a relay is still a real model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    Official,
    Cloud,
    Subscription,
    Proxy,
    ReverseProxy,
    Unknown,
}

impl Channel {
    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Official => "官方直连",
            Self::Cloud => "云平台",
            Self::Subscription => "订阅号",
            Self::Proxy => "普通中转",
            Self::ReverseProxy => "逆向渠道",
            Self::Unknown => "无法确定",
        }
    }

    pub fn desc_zh(&self) -> &'static str {
        match self {
            Self::Official => "响应头带官方特征，看起来是直连厂商 API。",
            Self::Cloud => "经由 AWS Bedrock / Google Vertex 等云平台转售。",
            Self::Subscription => "功能完整但缺少官方响应头，像是订阅账号导出的接口。",
            Self::Proxy => "功能正常但没有官方特征，是一层普通中转。",
            Self::ReverseProxy => "多项官方能力缺失，特征符合从网页端逆向出来的接口。",
            Self::Unknown => "信号不足，无法确定链路来源。",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub authenticity: Authenticity,
    pub channel: Channel,
    /// 0–100 weighted score across the scored groups.
    pub score: f64,
    /// 0–1. Reflects both signal strength and coverage.
    pub confidence: f64,
    pub hard_gate_hits: Vec<GateHit>,
    pub signals: Vec<String>,
    /// Every step the decision took, so a user can audit the conclusion.
    pub trace: Vec<String>,
    pub group_scores: BTreeMap<String, f64>,
    /// Fraction of probes that errored out — high values force a downgrade.
    pub coverage_gap: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateHit {
    pub name: String,
    pub probe: String,
    pub reason: String,
}

// ── identity ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStatus {
    /// Family and tier both line up with the claim.
    Match,
    /// Family lines up; tier could not be confirmed.
    FamilyOnly,
    /// Family lines up but measured capability points at a different tier.
    TierMismatch,
    /// Fingerprints point at a different family than claimed.
    FamilyMismatch,
    /// Signals contradict each other.
    Ambiguous,
    #[default]
    Insufficient,
}

impl IdentityStatus {
    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Match => "相符",
            Self::FamilyOnly => "家族相符，档位未验证",
            Self::TierMismatch => "家族相符，但档位被降级",
            Self::FamilyMismatch => "家族不符",
            Self::Ambiguous => "信号矛盾",
            Self::Insufficient => "数据不足",
        }
    }

    pub fn css(&self) -> &'static str {
        match self {
            Self::Match => "v-good",
            Self::FamilyOnly => "v-mid",
            Self::TierMismatch | Self::FamilyMismatch => "v-bad",
            Self::Ambiguous => "v-warn",
            Self::Insufficient => "v-none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Identity {
    pub claimed_model: String,
    pub claimed_family: Option<String>,
    pub claimed_tier: Option<String>,
    pub observed_family: Option<String>,
    pub family_confidence: f64,
    pub estimated_tier: Option<String>,
    pub tier_confidence: f64,
    /// 0 = aligned, 1 = one step apart, 2 = two or more (e.g. Opus vs Haiku).
    pub tier_severity: u8,
    pub status: IdentityStatus,
    pub evidence: Vec<String>,
    pub tier_scores: BTreeMap<String, f64>,
    pub accuracy_by_difficulty: BTreeMap<String, f64>,
    /// How many capability questions the tier estimate rests on, and how far
    /// the winning hypothesis beat the runner-up. Both belong in the report:
    /// a tier call from a handful of questions with a narrow margin is a much
    /// weaker claim than the same call from a wide one.
    pub tier_questions: u32,
    pub tier_margin: f64,
}

// ── billing ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Default)]
pub struct BillingAudit {
    pub rounds: Vec<BillingRound>,
    /// `authoritative` when the endpoint's own count_tokens answered,
    /// `estimated` when we fell back to the local heuristic.
    pub method: String,
    pub billed_input: u32,
    pub billed_output: u32,
    pub honest_input: u32,
    pub honest_output: u32,
    pub input_ratio: f64,
    pub billed_cost_usd: f64,
    pub honest_cost_usd: f64,
    pub cost_ratio: f64,
    pub pricing_source: String,
    pub anomalies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BillingRound {
    pub probe: String,
    pub billed_input: u32,
    pub honest_input: u32,
    pub billed_output: u32,
    pub ratio: f64,
}

// ── channel ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Default)]
pub struct ChannelSignature {
    pub label: String,
    pub display: String,
    pub confidence: f64,
    pub tier: u8,
    pub evidence: Vec<String>,
    /// Every relay vendor we saw a signature for; more than one means the
    /// request passed through more than one hop.
    pub all_hops: Vec<String>,
}

// ── perf ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Default)]
pub struct PerfSummary {
    pub samples: usize,
    pub ttft_ms: Vec<f64>,
    pub latency_ms: Vec<f64>,
    pub tps: Vec<f64>,
    pub ttft_p50: f64,
    pub ttft_p95: f64,
    pub latency_p50: f64,
    pub latency_p95: f64,
    pub tps_mean: f64,
    pub latency_cv: f64,
}

// ── whole run ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub tool_version: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub host: String,
    pub base_url: String,
    pub protocol: Protocol,
    pub model: String,
    pub claimed_model: String,
    pub depth: String,
    pub request_count: u32,
    pub results: Vec<ProbeResult>,
    pub verdict: Verdict,
    pub identity: Identity,
    pub billing: BillingAudit,
    pub channel: ChannelSignature,
    pub perf: PerfSummary,
    /// Probes that never ran, and why — so "not tested" is never read as "passed".
    pub skipped: Vec<String>,
}

impl Report {
    pub fn by_group(&self, g: Group) -> Vec<&ProbeResult> {
        self.results.iter().filter(|r| r.group == g).collect()
    }

    pub fn count(&self, s: Status) -> usize {
        self.results.iter().filter(|r| r.status == s).count()
    }

    /// Exit code contract: 0 clean, 1 failing score, 2 hard gate tripped.
    pub fn exit_code(&self) -> i32 {
        if !self.verdict.hard_gate_hits.is_empty() {
            return 2;
        }
        if matches!(
            self.verdict.authenticity,
            Authenticity::Counterfeit | Authenticity::Suspicious
        ) || self.verdict.score < 60.0
        {
            return 1;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_credit_matches_scoring_rules() {
        assert_eq!(Status::Pass.credit(), 1.0);
        assert_eq!(Status::Warn.credit(), 0.5);
        assert_eq!(Status::Fail.credit(), 0.0);
        // Skip and Error must not be worth partial credit, and must not
        // count in the denominator either.
        assert_eq!(Status::Skip.credit(), 0.0);
        assert!(!Status::Skip.scored());
        assert!(!Status::Error.scored());
        assert!(Status::Fail.scored());
    }

    #[test]
    fn probe_builder_chains() {
        let p = ProbeResult::new("x", "测试", Group::Contract)
            .weight(3)
            .fail("boom")
            .finding("detail")
            .metric("n", 4)
            .took(120);
        assert_eq!(p.status, Status::Fail);
        assert_eq!(p.weight, 3);
        assert_eq!(p.metric_f64("n"), Some(4.0));
        assert_eq!(p.duration_ms, 120);
        assert_eq!(p.findings.len(), 1);
    }

    #[test]
    fn evidence_is_trimmed_and_empty_is_dropped() {
        let p = ProbeResult::new("x", "l", Group::Contract).evidence("   ");
        assert!(p.evidence.is_none());
        let p = ProbeResult::new("x", "l", Group::Contract).evidence("  hi  ");
        assert_eq!(p.evidence.as_deref(), Some("hi"));
    }

    fn report_with(auth: Authenticity, score: f64, gates: Vec<GateHit>) -> Report {
        Report {
            tool_version: "t".into(),
            started_at: String::new(),
            finished_at: String::new(),
            duration_ms: 0,
            host: String::new(),
            base_url: String::new(),
            protocol: Protocol::Anthropic,
            model: String::new(),
            claimed_model: String::new(),
            depth: "balanced".into(),
            request_count: 0,
            results: vec![],
            verdict: Verdict {
                authenticity: auth,
                channel: Channel::Unknown,
                score,
                confidence: 0.5,
                hard_gate_hits: gates,
                signals: vec![],
                trace: vec![],
                group_scores: BTreeMap::new(),
                coverage_gap: 0.0,
            },
            identity: Identity::default(),
            billing: BillingAudit::default(),
            channel: ChannelSignature::default(),
            perf: PerfSummary::default(),
            skipped: vec![],
        }
    }

    #[test]
    fn exit_code_prioritises_hard_gates() {
        let gate = GateHit {
            name: "g".into(),
            probe: "p".into(),
            reason: "r".into(),
        };
        assert_eq!(
            report_with(Authenticity::Authentic, 99.0, vec![gate]).exit_code(),
            2
        );
        assert_eq!(
            report_with(Authenticity::Counterfeit, 99.0, vec![]).exit_code(),
            1
        );
        assert_eq!(
            report_with(Authenticity::Authentic, 42.0, vec![]).exit_code(),
            1
        );
        assert_eq!(
            report_with(Authenticity::Authentic, 88.0, vec![]).exit_code(),
            0
        );
        assert_eq!(
            report_with(Authenticity::ThirdParty, 88.0, vec![]).exit_code(),
            0,
            "a relayed real model is not a failure"
        );
    }
}
