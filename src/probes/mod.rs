// SPDX-License-Identifier: Apache-2.0
//! Probe registry and the shared context every probe writes into.

pub mod billing;
pub mod channel;
pub mod consistency;
pub mod contract;
pub mod identity;
pub mod perf;
pub mod stream;

use crate::client::Client;
use crate::report::{BillingRound, ProbeResult};
use crate::util::Rng;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// How hard to push. Repeat-count driven: more samples buy tighter
/// consistency and jitter signals at linear cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// Cheapest useful pass. Single samples, no repeat probes.
    Fast,
    /// Default. Enough repeats to see jitter and cache replay.
    Balanced,
    /// Consistency-heavy. Use when building a case against a provider.
    Forensic,
}

impl Depth {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" => Some(Self::Fast),
            "balanced" | "default" => Some(Self::Balanced),
            "forensic" | "deep" => Some(Self::Forensic),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Forensic => "forensic",
        }
    }

    /// Repeats for consistency and jitter sampling.
    pub fn repeats(&self) -> usize {
        match self {
            Self::Fast => 1,
            Self::Balanced => 3,
            Self::Forensic => 6,
        }
    }

    /// Questions per difficulty band in the tier estimator.
    pub fn tier_questions(&self) -> usize {
        match self {
            Self::Fast => 1,
            Self::Balanced => 2,
            Self::Forensic => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PerfSample {
    pub probe: String,
    pub ttft_ms: Option<u64>,
    pub latency_ms: u64,
    pub output_tokens: u32,
}

impl PerfSample {
    /// Generation throughput, excluding the wait for the first token.
    /// Returns `None` when the sample cannot support the calculation.
    pub fn tps(&self) -> Option<f64> {
        let ttft = self.ttft_ms? as f64;
        let gen_ms = self.latency_ms as f64 - ttft;
        if gen_ms <= 0.0 || self.output_tokens == 0 {
            return None;
        }
        Some(self.output_tokens as f64 / (gen_ms / 1000.0))
    }
}

/// Shared state. Probes run sequentially, so `RefCell` is enough and keeps
/// the borrow discipline visible instead of hiding it behind a lock.
pub struct Ctx {
    pub client: Client,
    pub depth: Depth,
    pub claimed_model: String,
    pub rng: RefCell<Rng>,
    pub perf: RefCell<Vec<PerfSample>>,
    pub billing: RefCell<Vec<BillingRound>>,
    /// Response headers from every successful call, for channel classification.
    pub headers: RefCell<Vec<BTreeMap<String, String>>>,
    pub message_ids: RefCell<Vec<String>>,
    pub raw_bodies: RefCell<Vec<String>>,
    /// Set by the preflight probe; when false the rest of the run is pointless.
    pub reachable: RefCell<bool>,
}

impl Ctx {
    pub fn new(client: Client, depth: Depth, claimed_model: String) -> Self {
        Self {
            client,
            depth,
            claimed_model,
            rng: RefCell::new(Rng::new()),
            perf: RefCell::new(Vec::new()),
            billing: RefCell::new(Vec::new()),
            headers: RefCell::new(Vec::new()),
            message_ids: RefCell::new(Vec::new()),
            raw_bodies: RefCell::new(Vec::new()),
            reachable: RefCell::new(true),
        }
    }

    /// Record everything a later probe might want from a raw response.
    pub fn observe(&self, raw: &crate::client::RawResponse, id: &str) {
        self.headers.borrow_mut().push(raw.headers.clone());
        if !id.is_empty() {
            self.message_ids.borrow_mut().push(id.to_string());
        }
        if self.raw_bodies.borrow().len() < 12 {
            self.raw_bodies
                .borrow_mut()
                .push(crate::util::truncate(&raw.body, 4000));
        }
    }

    pub fn add_perf(&self, sample: PerfSample) {
        self.perf.borrow_mut().push(sample);
    }
}

/// Every probe in the suite, in execution order. Contract first: if the
/// channel is rewriting requests, later fingerprint results are unreliable
/// and the verdict layer needs to know that before it reads them.
pub async fn run_all(
    ctx: &Ctx,
    on_progress: &mut dyn FnMut(&ProbeResult, usize, usize),
) -> Vec<ProbeResult> {
    let mut out: Vec<ProbeResult> = Vec::new();
    let total = PROBE_COUNT;

    macro_rules! step {
        ($e:expr) => {{
            let r = $e;
            on_progress(&r, out.len() + 1, total);
            out.push(r);
        }};
        (many $e:expr) => {{
            for r in $e {
                on_progress(&r, out.len() + 1, total);
                out.push(r);
            }
        }};
    }

    step!(contract::preflight(ctx).await);
    if !*ctx.reachable.borrow() {
        // Everything downstream would just report the same connection failure.
        return out;
    }

    step!(contract::model_catalog(ctx).await);
    step!(contract::response_schema(ctx).await);
    step!(contract::model_echo(ctx).await);
    step!(contract::missing_version(ctx).await);
    step!(contract::missing_auth(ctx).await);
    step!(contract::invalid_model(ctx).await);
    step!(contract::error_envelope(ctx).await);
    step!(contract::stop_reason_enum(ctx).await);
    step!(contract::max_tokens_truncation(ctx).await);
    step!(contract::stop_sequence(ctx).await);
    step!(contract::system_adherence(ctx).await);

    step!(stream::sse_format(ctx).await);
    step!(stream::stream_not_empty(ctx).await);
    step!(stream::stream_usage(ctx).await);

    step!(many billing::run(ctx).await);

    step!(many identity::run(ctx).await);

    step!(consistency::signature_drift(ctx).await);
    step!(consistency::cache_replay(ctx).await);
    step!(consistency::request_id_unique(ctx).await);

    // Perf and channel read what every other probe already observed, so they
    // must run last.
    step!(many perf::run(ctx).await);
    step!(many channel::run(ctx).await);

    out
}

/// Kept in sync with `run_all` so the progress line can show a total.
pub const PROBE_COUNT: usize = 40;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_parses_and_scales_repeats_monotonically() {
        assert_eq!(Depth::parse("FAST"), Some(Depth::Fast));
        assert_eq!(Depth::parse("deep"), Some(Depth::Forensic));
        assert_eq!(Depth::parse("nonsense"), None);
        assert!(Depth::Fast.repeats() < Depth::Balanced.repeats());
        assert!(Depth::Balanced.repeats() < Depth::Forensic.repeats());
    }

    #[test]
    fn tps_excludes_the_wait_for_first_token() {
        let s = PerfSample {
            probe: "p".into(),
            ttft_ms: Some(1000),
            latency_ms: 3000,
            output_tokens: 100,
        };
        // 100 tokens over the 2s of actual generation, not the full 3s.
        assert_eq!(s.tps(), Some(50.0));
    }

    #[test]
    fn tps_is_none_when_the_sample_cannot_support_it() {
        let base = PerfSample {
            probe: "p".into(),
            ttft_ms: Some(500),
            latency_ms: 1500,
            output_tokens: 10,
        };
        assert!(base.tps().is_some());

        // No streaming, so no TTFT to subtract.
        let mut s = base.clone();
        s.ttft_ms = None;
        assert!(s.tps().is_none());

        // Zero output tokens would divide by a meaningless numerator.
        let mut s = base.clone();
        s.output_tokens = 0;
        assert!(s.tps().is_none());

        // Whole response arrived in the first frame: no generation window.
        let mut s = base.clone();
        s.latency_ms = 500;
        assert!(s.tps().is_none());
    }
}
