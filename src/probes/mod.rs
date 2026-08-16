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
use crate::i18n::Lang;
use crate::report::{BillingRound, ProbeResult};
use crate::util::Rng;
use std::collections::BTreeMap;
use std::sync::Mutex;

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
    ///
    /// Measured on a live endpoint, two per band was not enough: the same
    /// model scored `hard 0/2` on one run and `1/2` on the next, which moved
    /// the fitted tier by a whole step. The abstention gates caught it — the
    /// second run degraded to a warning instead of accusing — but a real
    /// downgrade can be missed that way. Three narrows the swing at the cost
    /// of three extra requests; `forensic` is still the setting to reach for
    /// when the answer has to hold up.
    pub fn tier_questions(&self) -> usize {
        match self {
            Self::Fast => 1,
            Self::Balanced => 3,
            Self::Forensic => 5,
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

/// Shared state.
///
/// Probes run sequentially and nothing here is ever contended, so the lock is
/// not buying mutual exclusion — it is buying `Sync`. This used to be
/// `RefCell`, which made every probe future `!Send` and therefore impossible to
/// `await` from a multi-threaded runtime: an embedded caller (a request handler
/// spawning a verification) could not hold the future at all. Nothing may hold
/// one of these guards across an `.await`; each site below takes it, reads or
/// pushes, and drops it in the same expression.
pub struct Ctx {
    pub client: Client,
    pub depth: Depth,
    pub lang: Lang,
    pub claimed_model: String,
    pub rng: Mutex<Rng>,
    pub perf: Mutex<Vec<PerfSample>>,
    pub billing: Mutex<Vec<BillingRound>>,
    /// Response headers from every successful call, for channel classification.
    pub headers: Mutex<Vec<BTreeMap<String, String>>>,
    pub message_ids: Mutex<Vec<String>>,
    pub raw_bodies: Mutex<Vec<String>>,
    /// Set by the preflight probe; when false the rest of the run is pointless.
    pub reachable: Mutex<bool>,
}

impl Ctx {
    pub fn new(client: Client, depth: Depth, lang: Lang, claimed_model: String) -> Self {
        Self::with_rng(client, depth, lang, claimed_model, Rng::new())
    }

    /// The same, on a caller-chosen random source — see [`Rng::from_seed`].
    pub fn with_rng(
        client: Client,
        depth: Depth,
        lang: Lang,
        claimed_model: String,
        rng: Rng,
    ) -> Self {
        Self {
            client,
            depth,
            lang,
            claimed_model,
            rng: Mutex::new(rng),
            perf: Mutex::new(Vec::new()),
            billing: Mutex::new(Vec::new()),
            headers: Mutex::new(Vec::new()),
            message_ids: Mutex::new(Vec::new()),
            raw_bodies: Mutex::new(Vec::new()),
            reachable: Mutex::new(true),
        }
    }

    /// Record everything a later probe might want from a raw response.
    pub fn observe(&self, raw: &crate::client::RawResponse, id: &str) {
        self.headers.lock().unwrap().push(raw.headers.clone());
        if !id.is_empty() {
            self.message_ids.lock().unwrap().push(id.to_string());
        }
        let mut bodies = self.raw_bodies.lock().unwrap();
        if bodies.len() < 12 {
            bodies.push(crate::util::truncate(&raw.body, 4000));
        }
    }

    pub fn add_perf(&self, sample: PerfSample) {
        self.perf.lock().unwrap().push(sample);
    }

    /// Whether the endpoint answered the preflight probe at all.
    pub fn is_reachable(&self) -> bool {
        *self.reachable.lock().unwrap()
    }

    pub fn set_reachable(&self, v: bool) {
        *self.reachable.lock().unwrap() = v;
    }
}

/// What a step is actually measuring — and therefore whether its answer
/// survives a relay.
///
/// This is the distinction that matters to anyone probing an endpoint that is
/// not the vendor's own. Ask "is this endpoint's error envelope well formed"
/// through three hops and you have measured the hop nearest you; ask "does the
/// text coming back read like the model it claims to be" and you have measured
/// whatever generated the tokens, however many hops away it sits, because the
/// tokens themselves are the evidence.
///
/// A marketplace verifying a seller reachable only through its own gateway must
/// run [`Subject::Model`] steps and skip the rest: the endpoint steps would all
/// be describing the gateway, identically for every seller, at the cost of a
/// real request each. Running the whole suite there is not merely wasteful —
/// the contract steps deliberately send malformed and unauthenticated requests,
/// which is exactly the traffic pattern that gets a seller's upstream account
/// flagged for abuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// The HTTP endpoint itself: its contract, its error shapes, its headers,
    /// and the usage numbers *it* reports. Behind a relay, these describe the
    /// relay.
    Endpoint,
    /// The model generating the text. Survives any number of relay hops.
    Model,
}

pub type ProbeFuture<'a> = futures_util::future::BoxFuture<'a, Vec<ProbeResult>>;

/// One entry in the suite.
///
/// A step may emit several [`ProbeResult`]s — `identity` alone produces seven —
/// so the unit of *selection* is coarser than the unit of *reporting*. That is
/// deliberate: the multi-result steps share state and sampling between their
/// parts, and letting a caller pick half of one would silently change what the
/// other half means.
pub struct ProbeSpec {
    /// Stable across releases. Callers persist these, filter on them, and
    /// localise labels from them, so renaming one is a breaking change.
    pub id: &'static str,
    pub subject: Subject,
    /// How many results this step contributes, for progress totals. Steps whose
    /// output depends on what earlier steps observed report their maximum.
    pub results: usize,
    /// Runs regardless of the caller's subject filter, because the rest of the
    /// suite is meaningless without it.
    pub always: bool,
    pub run: for<'a> fn(&'a Ctx) -> ProbeFuture<'a>,
}

/// A probe supplied by the caller rather than by this crate.
///
/// # Why this exists
///
/// Everything in [`registry`] is published. That is right for a tool whose
/// value is that anyone can audit what it asks — and it is a problem for
/// anyone using it to police a marketplace, because the endpoint being probed
/// can read the suite too. Timing can be disguised (see [`Pace`]); the
/// questions themselves cannot be, once they are in a public repository.
///
/// So a caller in that position keeps its own bank of probes, out of the
/// public tree, rotated often enough that a fingerprint of last month's
/// questions is worth nothing. This trait is where those plug in. They share
/// the run's [`Ctx`] — the same client, the same seeded RNG, the same
/// observation buffers — so a custom probe is indistinguishable from a
/// built-in one in the report and in the traffic.
///
/// The engine never inspects them beyond scheduling. A custom probe that
/// returns [`Status::Fail`](crate::report::Status::Fail) moves the score
/// exactly as a built-in one does; if that is not wanted, mark the result
/// neutral.
pub trait Probe: Send + Sync {
    /// Stable id, as [`ProbeSpec::id`]. Namespace it (`acme.tokenizer`) so it
    /// cannot collide with a step this crate adds later.
    fn id(&self) -> &str;
    /// Whether this probe's evidence survives a relay. See [`Subject`].
    fn subject(&self) -> Subject {
        Subject::Model
    }
    /// Results this probe contributes, for progress totals.
    fn results(&self) -> usize {
        1
    }
    fn run<'a>(&'a self, ctx: &'a Ctx) -> ProbeFuture<'a>;
}

/// Wrap a single-result step so it fits the registry's shape.
macro_rules! one {
    ($f:path) => {
        (|ctx: &Ctx| Box::pin(async move { vec![$f(ctx).await] }) as ProbeFuture<'_>)
            as for<'a> fn(&'a Ctx) -> ProbeFuture<'a>
    };
}

/// Wrap a step that already returns several results.
macro_rules! many {
    ($f:path) => {
        (|ctx: &Ctx| Box::pin($f(ctx)) as ProbeFuture<'_>) as for<'a> fn(&'a Ctx) -> ProbeFuture<'a>
    };
}

/// Every step in the suite, in execution order.
///
/// Contract first: if the channel is rewriting requests, later fingerprint
/// results are unreliable and the verdict layer needs to know that before it
/// reads them. Perf and channel last, because both read what every earlier step
/// observed rather than issuing much of their own.
pub fn registry() -> Vec<ProbeSpec> {
    use Subject::{Endpoint, Model};
    let spec = |id, subject, results, run| ProbeSpec {
        id,
        subject,
        results,
        always: false,
        run,
    };
    vec![
        ProbeSpec {
            id: "preflight",
            subject: Endpoint,
            results: 1,
            always: true,
            run: one!(contract::preflight),
        },
        spec("model_catalog", Endpoint, 1, one!(contract::model_catalog)),
        spec(
            "response_schema",
            Endpoint,
            1,
            one!(contract::response_schema),
        ),
        spec("model_echo", Endpoint, 1, one!(contract::model_echo)),
        spec(
            "missing_version",
            Endpoint,
            1,
            one!(contract::missing_version),
        ),
        spec("missing_auth", Endpoint, 1, one!(contract::missing_auth)),
        spec("invalid_model", Endpoint, 1, one!(contract::invalid_model)),
        spec(
            "error_envelope",
            Endpoint,
            1,
            one!(contract::error_envelope),
        ),
        spec(
            "stop_reason_enum",
            Endpoint,
            1,
            one!(contract::stop_reason_enum),
        ),
        // Truncation, stop sequences and system-prompt adherence are asked of
        // the generator, not of the transport: a relay forwards the parameter
        // and it is the model that honours or ignores it.
        spec(
            "max_tokens_truncation",
            Model,
            1,
            one!(contract::max_tokens_truncation),
        ),
        spec("stop_sequence", Model, 1, one!(contract::stop_sequence)),
        spec(
            "system_adherence",
            Model,
            1,
            one!(contract::system_adherence),
        ),
        spec("sse_format", Endpoint, 1, one!(stream::sse_format)),
        spec(
            "stream_not_empty",
            Endpoint,
            1,
            one!(stream::stream_not_empty),
        ),
        spec("stream_usage", Endpoint, 1, one!(stream::stream_usage)),
        // Every billing signal is read out of the `usage` block the endpoint
        // reports. Behind a relay that block is the relay's accounting.
        spec("billing", Endpoint, 7, many!(billing::run)),
        spec("identity", Model, 8, many!(identity::run)),
        spec(
            "signature_drift",
            Model,
            1,
            one!(consistency::signature_drift),
        ),
        spec("cache_replay", Model, 1, one!(consistency::cache_replay)),
        spec(
            "request_id_unique",
            Endpoint,
            1,
            one!(consistency::request_id_unique),
        ),
        spec("perf", Model, 4, many!(perf::run)),
        spec("channel", Endpoint, 3, many!(channel::run)),
    ]
}

/// Which steps to run.
#[derive(Clone, Default)]
pub struct Selection {
    /// Subjects to keep. Empty means all of them.
    pub subjects: Vec<Subject>,
    /// Step ids to keep. Empty means all of them; applied after `subjects`.
    pub only: Vec<String>,
    /// Step ids to drop, applied last and winning over both fields above.
    pub skip: Vec<String>,
    /// Caller-supplied probes, appended after the built-in steps. See [`Probe`].
    ///
    /// Not filtered by `subjects`/`only`/`skip`: the caller assembled this list
    /// itself and already decided what belongs in it. `skip` still removes one
    /// by id, so a probe found to be misbehaving can be turned off without a
    /// deploy.
    pub extra: Vec<std::sync::Arc<dyn Probe>>,
}

impl std::fmt::Debug for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selection")
            .field("subjects", &self.subjects)
            .field("only", &self.only)
            .field("skip", &self.skip)
            .field(
                "extra",
                &self.extra.iter().map(|p| p.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Selection {
    /// Everything — what the CLI runs against a vendor's own endpoint.
    pub fn all() -> Self {
        Self::default()
    }

    /// Only what survives a relay. See [`Subject`].
    pub fn model_only() -> Self {
        Selection {
            subjects: vec![Subject::Model],
            ..Default::default()
        }
    }

    /// Append a caller-supplied probe.
    pub fn with(mut self, p: std::sync::Arc<dyn Probe>) -> Self {
        self.extra.push(p);
        self
    }

    fn keeps(&self, spec: &ProbeSpec) -> bool {
        if spec.always {
            return true;
        }
        if self.skip.iter().any(|s| s == spec.id) {
            return false;
        }
        if !self.subjects.is_empty() && !self.subjects.contains(&spec.subject) {
            return false;
        }
        if !self.only.is_empty() && !self.only.iter().any(|s| s == spec.id) {
            return false;
        }
        true
    }

    /// The registry, filtered.
    pub fn resolve(&self) -> Vec<ProbeSpec> {
        registry().into_iter().filter(|s| self.keeps(s)).collect()
    }

    /// Caller-supplied probes that survived `skip`.
    pub fn resolve_extra(&self) -> Vec<std::sync::Arc<dyn Probe>> {
        self.extra
            .iter()
            .filter(|p| !self.skip.iter().any(|s| s == p.id()))
            .cloned()
            .collect()
    }
}

/// What the caller is told as the run proceeds.
///
/// Both variants carry the running total so a progress bar can be drawn without
/// the caller tracking state. `total` is an upper bound — several steps skip
/// parts of themselves when the endpoint does not offer what they need — so a
/// run legitimately finishes below it.
pub enum Event<'a> {
    /// A step is about to issue its requests. This is the one a UI wants: the
    /// gap before a slow step's first result is where a progress display
    /// otherwise looks frozen.
    Started {
        id: &'a str,
        done: usize,
        total: usize,
    },
    Finished {
        result: &'a ProbeResult,
        done: usize,
        total: usize,
    },
}

/// Cooperative cancellation.
///
/// Checked between steps rather than inside them: a step that has already paid
/// for its requests may as well report what they showed, and tearing one down
/// mid-flight would leave the shared context half-written for whatever runs
/// next. Worst-case latency is therefore one step, not one request.
#[derive(Clone)]
pub struct Cancel {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Wakes a paced run that is asleep between steps. Without it, cancelling
    /// a run spread over half an hour would take up to one full interval to
    /// take effect.
    notify: std::sync::Arc<tokio::sync::Notify>,
}

impl Default for Cancel {
    fn default() -> Self {
        Cancel {
            flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            notify: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Resolves once cancelled, or immediately if it already is.
    pub async fn wait(&self) {
        if self.is_cancelled() {
            return;
        }
        self.notify.notified().await;
    }
}

/// How long to wait between steps, drawn uniformly from the range.
///
/// The CLI leaves this unset and the suite runs as fast as the endpoint
/// answers, which is what somebody probing their own endpoint wants.
///
/// It exists for the caller probing *somebody else's*. Back to back, a run is a
/// recognisable object: a fixed number of requests, in a fixed order, arriving
/// in a burst that looks like nothing else that endpoint serves. An operator
/// who wants a run to be hard to pick out of ordinary traffic has to spread it,
/// and spreading it is the caller's decision because only the caller knows what
/// ordinary traffic there looks like.
///
/// This raises the cost of recognition; it does not eliminate it. The payloads
/// are still drawn from a published suite.
#[derive(Debug, Clone, Copy)]
pub struct Pace {
    pub min: std::time::Duration,
    pub max: std::time::Duration,
}

/// Run a selected suite.
pub async fn run_selected(
    ctx: &Ctx,
    specs: &[ProbeSpec],
    cancel: &Cancel,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Vec<ProbeResult> {
    run_paced(ctx, specs, cancel, None, on_event).await
}

/// Run a selected suite, optionally spacing the steps out. See [`Pace`].
pub async fn run_paced(
    ctx: &Ctx,
    specs: &[ProbeSpec],
    cancel: &Cancel,
    pace: Option<Pace>,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Vec<ProbeResult> {
    run_with_extra(ctx, specs, &[], cancel, pace, on_event).await
}

/// One step, whether it came from the registry or from the caller.
///
/// The two are deliberately indistinguishable from here on: same context, same
/// pacing, same events, same place in the report. A custom probe that ran
/// differently from a built-in one would be a custom probe the endpoint could
/// pick out, which defeats the reason for having private ones at all.
enum Step<'a> {
    Built(&'a ProbeSpec),
    Custom(&'a std::sync::Arc<dyn Probe>),
}

impl Step<'_> {
    fn id(&self) -> &str {
        match self {
            Step::Built(s) => s.id,
            Step::Custom(p) => p.id(),
        }
    }
    fn results(&self) -> usize {
        match self {
            Step::Built(s) => s.results,
            Step::Custom(p) => p.results(),
        }
    }
    fn run<'a>(&'a self, ctx: &'a Ctx) -> ProbeFuture<'a> {
        match self {
            Step::Built(s) => (s.run)(ctx),
            Step::Custom(p) => p.run(ctx),
        }
    }
}

/// Run the registry steps and then the caller's own. See [`Probe`].
pub async fn run_with_extra(
    ctx: &Ctx,
    specs: &[ProbeSpec],
    extra: &[std::sync::Arc<dyn Probe>],
    cancel: &Cancel,
    pace: Option<Pace>,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Vec<ProbeResult> {
    let steps: Vec<Step<'_>> = specs
        .iter()
        .map(Step::Built)
        .chain(extra.iter().map(Step::Custom))
        .collect();
    run_steps(ctx, &steps, cancel, pace, on_event).await
}

async fn run_steps(
    ctx: &Ctx,
    specs: &[Step<'_>],
    cancel: &Cancel,
    pace: Option<Pace>,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Vec<ProbeResult> {
    let total: usize = specs.iter().map(|s| s.results()).sum();
    let mut out: Vec<ProbeResult> = Vec::new();
    let mut first = true;
    for spec in specs {
        if cancel.is_cancelled() {
            break;
        }
        // Before the step, never after the last one: a run that ended minutes
        // ago but has not returned is a run whose caller thinks it is still
        // going.
        if let (false, Some(p)) = (first, pace) {
            let span = p.max.saturating_sub(p.min);
            let jitter = if span.is_zero() {
                std::time::Duration::ZERO
            } else {
                let r = ctx.rng.lock().unwrap().next_u64();
                std::time::Duration::from_millis(r % (span.as_millis() as u64).max(1))
            };
            let wait = p.min + jitter;
            // Cancellation has to win over the wait, or cancelling a paced run
            // takes as long as letting it finish.
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = cancel.wait() => break,
            }
        }
        first = false;
        on_event(Event::Started {
            id: spec.id(),
            done: out.len(),
            total,
        });
        for r in spec.run(ctx).await {
            out.push(r);
            let done = out.len();
            on_event(Event::Finished {
                result: out.last().unwrap(),
                done,
                total,
            });
        }
        // Everything downstream would just report the same connection failure.
        if !ctx.is_reachable() {
            break;
        }
    }
    out
}

/// Upper bound on results from the full suite. Derived, so adding a step cannot
/// leave it stale — it used to be a hand-maintained `40`.
pub fn probe_count() -> usize {
    registry().iter().map(|s| s.results).sum()
}

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
