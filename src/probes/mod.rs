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
use crate::report::{BillingRound, Group, ProbeResult};
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
/// The locks are not buying mutual exclusion so much as `Sync`. This used to be
/// `RefCell`, which made every probe future `!Send` and therefore impossible to
/// `await` from a multi-threaded runtime: an embedded caller (a request handler
/// spawning a verification) could not hold the future at all. Nothing may hold
/// one of these guards across an `.await`; each site below takes it, reads or
/// pushes, and drops it in the same expression. That rule was a tidiness
/// convention while steps ran one at a time and is load bearing now that they
/// may not — see [`run_steps`].
pub struct Ctx {
    pub client: Client,
    pub depth: Depth,
    pub lang: Lang,
    pub claimed_model: String,
    /// The run's seed. Every random payload is derived from it and from the id
    /// of the step that asks — see [`Ctx::rng_for`].
    pub seed: u64,
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
        Self::with_seed(client, depth, lang, claimed_model, Rng::new().next_u64())
    }

    /// The same, on a caller-chosen seed.
    ///
    /// Replaces the `with_rng` of earlier releases, which handed the whole run
    /// one shared generator. That worked exactly as long as the steps ran in a
    /// fixed order: draw order *was* step order, so a seed reproduced a run.
    /// Under [`RunConfig::concurrency`](crate::engine::RunConfig::concurrency)
    /// it does not — whichever step wins the race draws first — and a seed that
    /// reproduces a different set of questions each time is not a seed, it is
    /// decoration. The generator is therefore per step now, and the id is half
    /// of what seeds it.
    pub fn with_seed(
        client: Client,
        depth: Depth,
        lang: Lang,
        claimed_model: String,
        seed: u64,
    ) -> Self {
        Self {
            client,
            depth,
            lang,
            claimed_model,
            seed,
            perf: Mutex::new(Vec::new()),
            billing: Mutex::new(Vec::new()),
            headers: Mutex::new(Vec::new()),
            message_ids: Mutex::new(Vec::new()),
            raw_bodies: Mutex::new(Vec::new()),
            reachable: Mutex::new(true),
        }
    }

    /// A generator belonging to one step, and to one run.
    ///
    /// Two steps never share a stream, so the payloads a seed produces do not
    /// depend on the order the scheduler happened to run them in, or on whether
    /// an earlier step was skipped. Same seed and same step id rebuild the same
    /// questions on any machine — which is what makes the seed in the report
    /// worth recording, and a contested verdict answerable question by question.
    ///
    /// A step that draws more than once must therefore hold on to what this
    /// returns rather than calling again per draw, or every draw is the first
    /// draw. Steps that fan their requests out concurrently have to generate
    /// everything up front for the same reason.
    pub fn rng_for(&self, step_id: &str) -> Rng {
        // FNV-1a, inlined: the requirement is that the mixing never changes
        // between releases, which no hasher from the standard library promises.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in step_id.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Rng::from_seed(self.seed ^ h)
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
    /// Which section of the report this step's results land in. Also selectable
    /// — see [`Selection::only`] — so `identity` addresses the whole family of
    /// identity steps without naming each one.
    pub group: Group,
    pub subject: Subject,
    /// How many results this step contributes, for progress totals. Steps whose
    /// output depends on what earlier steps observed report their maximum.
    pub results: usize,
    /// Runs regardless of the caller's subject filter, because the rest of the
    /// suite is meaningless without it.
    pub always: bool,
    /// Must have the run to itself: nothing else may be in flight while it
    /// runs. See [`run_steps`] for why anything measuring a clock has to be.
    pub exclusive: bool,
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
    /// Whether this probe needs the run to itself. See [`ProbeSpec::exclusive`].
    ///
    /// `false` is right for anything whose evidence is the content of a reply.
    /// Say `true` only if the measurement is a clock reading, or if the probe
    /// reads shared state that a step running beside it could still be writing.
    fn exclusive(&self) -> bool {
        false
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
///
/// The identity family is seven steps rather than one. It was one until 0.5.0,
/// and the cost of that was invisible until somebody wanted a cheaper run: the
/// unit of selection was the whole family, so a caller who wanted the model's
/// self-report and its capability profile had to buy four steps of self-reported
/// trivia along with them — and, because a step is also the unit of scheduling,
/// had to run all twelve requests one after another. Splitting them changes
/// nothing about what any of them asks.
pub fn registry() -> Vec<ProbeSpec> {
    use Group::{Consistency, Contract, Identity, Perf, Stream};
    use Subject::{Endpoint, Model};
    let spec = |id, group, subject, results, run| ProbeSpec {
        id,
        group,
        subject,
        results,
        always: false,
        exclusive: false,
        run,
    };
    vec![
        ProbeSpec {
            id: "preflight",
            group: Contract,
            subject: Endpoint,
            results: 1,
            always: true,
            // Nothing may run beside it, because nothing may run *before* its
            // answer: every later step is conditional on the endpoint being
            // there at all.
            exclusive: true,
            run: one!(contract::preflight),
        },
        spec(
            "model_catalog",
            Contract,
            Endpoint,
            1,
            one!(contract::model_catalog),
        ),
        spec(
            "response_schema",
            Contract,
            Endpoint,
            1,
            one!(contract::response_schema),
        ),
        spec(
            "model_echo",
            Contract,
            Endpoint,
            1,
            one!(contract::model_echo),
        ),
        spec(
            "missing_version",
            Contract,
            Endpoint,
            1,
            one!(contract::missing_version),
        ),
        spec(
            "missing_auth",
            Contract,
            Endpoint,
            1,
            one!(contract::missing_auth),
        ),
        spec(
            "invalid_model",
            Contract,
            Endpoint,
            1,
            one!(contract::invalid_model),
        ),
        spec(
            "error_envelope",
            Contract,
            Endpoint,
            1,
            one!(contract::error_envelope),
        ),
        spec(
            "stop_reason_enum",
            Contract,
            Endpoint,
            1,
            one!(contract::stop_reason_enum),
        ),
        // Truncation, stop sequences and system-prompt adherence are asked of
        // the generator, not of the transport: a relay forwards the parameter
        // and it is the model that honours or ignores it.
        spec(
            "max_tokens_truncation",
            Contract,
            Model,
            1,
            one!(contract::max_tokens_truncation),
        ),
        spec(
            "stop_sequence",
            Contract,
            Model,
            1,
            one!(contract::stop_sequence),
        ),
        spec(
            "system_adherence",
            Contract,
            Model,
            1,
            one!(contract::system_adherence),
        ),
        spec("sse_format", Stream, Endpoint, 1, one!(stream::sse_format)),
        spec(
            "stream_not_empty",
            Stream,
            Endpoint,
            1,
            one!(stream::stream_not_empty),
        ),
        spec(
            "stream_usage",
            Stream,
            Endpoint,
            1,
            one!(stream::stream_usage),
        ),
        // Every billing signal is read out of the `usage` block the endpoint
        // reports. Behind a relay that block is the relay's accounting.
        spec("billing", Group::Billing, Endpoint, 7, many!(billing::run)),
        // ── identity ───────────────────────────────────────────────────────
        // What it says it is. One request, and the only one of these that can
        // reach a family verdict on its own.
        spec("self_id", Identity, Model, 1, one!(identity::self_id)),
        // A second, differently-phrased ask, purely to corroborate `self_id`.
        spec(
            "meta_creator",
            Identity,
            Model,
            1,
            one!(identity::meta_creator),
        ),
        // Self-reported context window and cutoff. Both are fingerprints — the
        // answers are strikingly consistent within a checkpoint — and neither
        // is scored: they describe, they do not judge.
        spec(
            "context_claim",
            Identity,
            Model,
            1,
            one!(identity::context_claim),
        ),
        spec(
            "cutoff_claim",
            Identity,
            Model,
            1,
            one!(identity::cutoff_claim),
        ),
        // Four requests, and the most expensive thing in the family that is not
        // the battery. Cross-checks demonstrated knowledge against the model's
        // own claimed cutoff.
        spec(
            "world_knowledge",
            Identity,
            Model,
            1,
            one!(identity::world_knowledge),
        ),
        // The battery and the tier it implies stay one step: the estimate is a
        // reading of the battery's own result, and a caller who took one without
        // the other would get a tier fitted to no measurements.
        spec(
            "capability",
            Identity,
            Model,
            2,
            many!(identity::capability),
        ),
        spec("verbosity", Identity, Model, 1, one!(identity::verbosity)),
        // ── consistency ────────────────────────────────────────────────────
        spec(
            "signature_drift",
            Consistency,
            Model,
            1,
            one!(consistency::signature_drift),
        ),
        spec(
            "cache_replay",
            Consistency,
            Model,
            1,
            one!(consistency::cache_replay),
        ),
        spec(
            "request_id_unique",
            Consistency,
            Endpoint,
            1,
            one!(consistency::request_id_unique),
        ),
        ProbeSpec {
            id: "perf",
            group: Perf,
            subject: Model,
            results: 4,
            always: false,
            // The one step whose measurement is a clock reading. Anything else
            // in flight is load this run put there itself, and a latency figure
            // that includes our own queueing describes the run rather than the
            // endpoint — then travels on into whatever the caller does with
            // `PerfSummary`.
            exclusive: true,
            run: many!(perf::run),
        },
        spec("channel", Group::Channel, Endpoint, 3, many!(channel::run)),
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
    ///
    /// Drops caller-supplied probes as well as built-in ones, so a custom probe
    /// found to be misbehaving can be turned off by id without a deploy. That
    /// is why replacing a built-in step with a custom one of the same id is
    /// [`replacing`](Self::replacing) rather than a `skip` plus a `with`.
    pub skip: Vec<String>,
    /// Built-in step ids a caller-supplied probe is standing in for.
    ///
    /// Unlike `skip` this applies to the registry only, which is the whole
    /// point: the replacement is allowed to answer to the id it replaced.
    pub replaced: Vec<String>,
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
            .field("replaced", &self.replaced)
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

    /// The smallest set that still supports a verdict about the model.
    ///
    /// [`model_only`](Self::model_only) with the redundant and the merely
    /// descriptive taken out. Nine requests at [`Depth::Fast`] against
    /// twenty-one, and — because the identity family is no longer one
    /// indivisible step — a shape the scheduler can actually overlap.
    ///
    /// # What it keeps, and why those
    ///
    /// * `preflight` — without it nothing else means anything.
    /// * `self_id` — the family check. This is the one that catches a lane sold
    ///   as one vendor's model and served by another's, which is the cheat that
    ///   does not need to be subtle.
    /// * `capability` — graded questions generated per run, and the only thing
    ///   here that a downgrade cannot answer its way around.
    /// * `verbosity` and `perf` — how much it writes and how fast it generates.
    ///   Weak on their own and the two most useful axes there are for anyone
    ///   comparing one endpoint against many others serving the same model.
    /// * `cache_replay` — a hard gate, and it catches being charged for
    ///   inference that never ran.
    ///
    /// # What it drops, and what that costs
    ///
    /// `meta_creator` only ever corroborated `self_id`; `context_claim` and
    /// `cutoff_claim` are unscored description; `signature_drift` looks for
    /// fan-out across backends, which behind a relay describes the relay.
    /// `world_knowledge` is four requests, asks for the cutoff a second time,
    /// and measures the training corpus — a cheap model with a large corpus
    /// passes it and an expensive one having a bad day fails it.
    ///
    /// The real loss is the three contract steps that survive a relay
    /// (`max_tokens_truncation`, `stop_sequence`, `system_adherence`). Each is
    /// one short request and each is a *strong* reverse-channel signal: they are
    /// how a reconstructed endpoint, one that forwards a prompt to a web session
    /// and cannot honour an API parameter it never received, gives itself away.
    /// A caller who has not otherwise established what is on the far end should
    /// add them back:
    ///
    /// ```
    /// # use llm_verify::probes::Selection;
    /// let sel = Selection::turbo().plus(["max_tokens_truncation", "stop_sequence", "system_adherence"]);
    /// ```
    ///
    /// # Replacing the battery rather than paying for two
    ///
    /// A caller with a private bank of graded questions — the published ones
    /// are readable by the endpoint being probed, which is the ceiling on what
    /// they can prove — should spend the budget there instead of on both:
    ///
    /// ```
    /// # use llm_verify::probes::Selection;
    /// # fn demo(bank: std::sync::Arc<dyn llm_verify::probes::Probe>) {
    /// let sel = Selection::turbo().replacing("capability", bank);
    /// # }
    /// ```
    ///
    /// [`replacing`](Self::replacing) rather than `minus` then `with` — the
    /// latter reads correctly and silently drops both, for the reason spelled
    /// out there.
    ///
    /// Three of turbo's nine requests are the battery, so that trades them for
    /// however many the bank asks. The private probe is then responsible for
    /// emitting a `capability` result and a `tier_estimate` one, or the identity
    /// view has no capability measurement to read — see
    /// [`identity::tier_result`](crate::probes::identity::tier_result), which is
    /// public so that the thresholds stay in one place.
    pub fn turbo() -> Self {
        Selection {
            only: ["self_id", "capability", "verbosity", "cache_replay", "perf"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Default::default()
        }
    }

    /// Add steps to an `only` list. Ids or group keys, as [`Selection::only`].
    ///
    /// A no-op on a selection that has no `only` list, because that one already
    /// includes everything — adding to it could only ever narrow it, which is
    /// the opposite of what the name says.
    pub fn plus<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if !self.only.is_empty() {
            self.only
                .extend(ids.into_iter().map(|s| s.as_ref().to_string()));
        }
        self
    }

    /// Drop steps. Ids or group keys, as [`Selection::skip`].
    ///
    /// Works on any selection, unlike [`plus`](Self::plus): removing is
    /// unambiguous whether or not there is an `only` list to remove from.
    pub fn minus<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.skip
            .extend(ids.into_iter().map(|s| s.as_ref().to_string()));
        self
    }

    /// Append a caller-supplied probe.
    pub fn with(mut self, p: std::sync::Arc<dyn Probe>) -> Self {
        self.extra.push(p);
        self
    }

    /// Drop a built-in step and install a caller's probe in its place.
    ///
    /// # Why this is one call
    ///
    /// The obvious spelling — `.minus(["capability"]).with(bank)` — is wrong in
    /// a way that reports success. [`skip`](Self::skip) applies to custom
    /// probes too, deliberately, so that a misbehaving one can be switched off
    /// by id; and a probe standing in for a built-in step naturally carries
    /// that step's id, because carrying it is what makes everything downstream
    /// keep working. So the skip removes both, the run comes back with the step
    /// simply absent, and nothing anywhere says so. The verdict still
    /// assembles, the report still renders, and the graded questions the whole
    /// exercise existed to ask were never sent.
    ///
    /// Found by counting requests against a stub, which is the only way it
    /// could have been found.
    pub fn replacing(mut self, id: &str, p: std::sync::Arc<dyn Probe>) -> Self {
        self.replaced.push(id.to_string());
        self.extra.push(p);
        self
    }

    /// Whether a name in `only`/`skip` addresses this step — by its own id, or
    /// by the key of the group it belongs to.
    ///
    /// Group matching is what keeps `skip: ["identity"]` meaning what it meant
    /// before 0.5.0 split that step into seven. It is also the more useful thing
    /// to write: a caller who wants "no identity probing" wants the family, not
    /// a list they have to keep in step with each release.
    fn names(spec: &ProbeSpec, pattern: &str) -> bool {
        pattern == spec.id || pattern == spec.group.key()
    }

    fn keeps(&self, spec: &ProbeSpec) -> bool {
        if spec.always {
            return true;
        }
        if self.skip.iter().any(|s| Self::names(spec, s))
            || self.replaced.iter().any(|s| Self::names(spec, s))
        {
            return false;
        }
        if !self.subjects.is_empty() && !self.subjects.contains(&spec.subject) {
            return false;
        }
        if !self.only.is_empty() && !self.only.iter().any(|s| Self::names(spec, s)) {
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
    run_with_extra(ctx, specs, &[], cancel, Schedule::paced(pace), on_event).await
}

/// When the steps run relative to one another.
///
/// The two fields are mutually exclusive in practice and the constructors say
/// so, because they exist for opposite reasons. [`Pace`] spreads a run out to
/// make it hard to recognise; overlapping compresses it into the smallest burst
/// the endpoint will tolerate. A caller who asked for both would be asking to be
/// unobtrusive quickly.
#[derive(Debug, Clone, Copy, Default)]
pub struct Schedule {
    pub pace: Option<Pace>,
    /// Steps that may be in flight at once. `0` and `1` both mean sequential.
    ///
    /// This is a *step* count, and steps are uneven — the capability battery is
    /// a dozen requests and `stop_sequence` is one — so it does not bound how
    /// much traffic reaches the endpoint. That bound is
    /// [`Client::with_limit`](crate::client::Client::with_limit), and a caller
    /// probing an endpoint it does not own wants both.
    pub concurrency: usize,
}

impl Schedule {
    /// One step at a time, in registry order. What every release before 0.5.0
    /// did, and still the default.
    pub fn sequential() -> Self {
        Schedule::default()
    }

    pub fn paced(pace: Option<Pace>) -> Self {
        Schedule {
            pace,
            concurrency: 0,
        }
    }

    /// Overlap up to `n` steps. Ignored while a [`Pace`] is set.
    pub fn concurrent(n: usize) -> Self {
        Schedule {
            pace: None,
            concurrency: n,
        }
    }

    fn overlaps(&self) -> bool {
        self.pace.is_none() && self.concurrency > 1
    }
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
    fn exclusive(&self) -> bool {
        match self {
            Step::Built(s) => s.exclusive,
            Step::Custom(p) => p.exclusive(),
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
    schedule: Schedule,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Vec<ProbeResult> {
    let steps: Vec<Step<'_>> = specs
        .iter()
        .map(Step::Built)
        .chain(extra.iter().map(Step::Custom))
        .collect();
    run_steps(ctx, &steps, cancel, schedule, on_event).await
}

/// Run the steps, one at a time or several at once.
///
/// # The two schedules
///
/// Sequential is what every release before 0.5.0 did and is still the default,
/// because it is the only one that can be paced and the only one under which a
/// [`Perf`](Group::Perf) reading means anything without further care.
///
/// Overlapping exists for the run somebody is *waiting on* — a marketplace
/// admitting a listing while a seller watches a progress dialog, where thirty
/// sequential round trips to a model that thinks before it answers is minutes.
/// Two things make it safe to do without changing any probe's answer:
///
/// * **Exclusive steps.** [`ProbeSpec::exclusive`] marks a step that must have
///   the run to itself. `preflight` is one because everything after it is
///   conditional on its answer; `perf` is one because its measurement is a
///   clock, and a latency figure taken while this run had three other requests
///   in the air describes the run rather than the endpoint. Registry order is
///   preserved across them: consecutive non-exclusive steps overlap with each
///   other and with nothing on the far side of an exclusive one.
/// * **Per-step generators.** See [`Ctx::rng_for`]. Draw order no longer
///   depends on scheduling, so a seed reproduces a run either way.
///
/// What does change is [`Event`] timing: a step announces `Started` when it is
/// admitted to the window rather than when the one before it finished, so
/// several may be outstanding at once and their `Finished` events interleave. A
/// display that tracks "the current step" should expect to hold more than one.
/// `done`/`total` still advance monotonically, and results are re-ordered into
/// registry order before returning, so nothing reading the report can tell.
async fn run_steps(
    ctx: &Ctx,
    specs: &[Step<'_>],
    cancel: &Cancel,
    schedule: Schedule,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Vec<ProbeResult> {
    let total: usize = specs.iter().map(|s| s.results()).sum();
    // Indexed by position in `specs`, so an overlapping wave can be flattened
    // back into registry order however its steps finished.
    let mut collected: Vec<Vec<ProbeResult>> = vec![Vec::new(); specs.len()];
    let mut done = 0usize;
    let mut first = true;
    let mut i = 0usize;

    while i < specs.len() {
        if cancel.is_cancelled() {
            break;
        }
        // How many steps start together.
        //
        // A rolling window rather than fixed batches, and the difference is not
        // academic: batching in groups of `concurrency` makes every group wait
        // for its own slowest member before the next one starts, so a run whose
        // steps are uneven — and they are, one of them is a whole battery —
        // spends most of its time with idle permits. Taking every consecutive
        // non-exclusive step instead lets a step that finishes early free its
        // permit for one further down the list. What bounds the actual traffic
        // is the client's permit pool, not this number.
        let wave = if schedule.overlaps() && !specs[i].exclusive() {
            specs[i..]
                .iter()
                .take_while(|s| !s.exclusive())
                .count()
                .max(1)
        } else {
            1
        };

        // Before the wave, never after the last one: a run that ended minutes
        // ago but has not returned is a run whose caller thinks it is still
        // going.
        if let (false, Some(p)) = (first, schedule.pace) {
            let span = p.max.saturating_sub(p.min);
            let jitter = if span.is_zero() {
                std::time::Duration::ZERO
            } else {
                // Drawn from a stream of its own so that pacing — which is a
                // decision about *when*, made by the caller — cannot shift the
                // payloads any probe asks. Before 0.5.0 it shared the run's one
                // generator, so the same seed produced different questions
                // depending on whether the run was paced.
                let r = ctx.rng_for("__pace").next_u64();
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

        // Nothing is spawned: the futures borrow `ctx` and are polled together
        // on this task. A step is admitted as soon as a slot frees rather than
        // when the whole group finishes, so a short step behind a long one does
        // not wait for it.
        use futures_util::StreamExt;
        let end = i + wave;
        let cap = if wave == 1 {
            1
        } else {
            schedule.concurrency.max(1)
        };
        let mut running = futures_util::stream::FuturesUnordered::new();
        let mut next = i;
        loop {
            while next < end && running.len() < cap {
                let at = next;
                on_event(Event::Started {
                    id: specs[at].id(),
                    done,
                    total,
                });
                running.push(async move { (at, specs[at].run(ctx).await) });
                next += 1;
            }
            let Some((at, results)) = running.next().await else {
                break;
            };
            for r in results {
                collected[at].push(r);
                done += 1;
                on_event(Event::Finished {
                    result: collected[at].last().unwrap(),
                    done,
                    total,
                });
            }
        }

        // Everything downstream would just report the same connection failure.
        if !ctx.is_reachable() {
            break;
        }
        i += wave;
    }

    collected.into_iter().flatten().collect()
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

    /// The two steps that cannot share a run, and the reason each cannot.
    ///
    /// `perf` is the one that would fail quietly. Overlap it and its numbers
    /// come out worse the more concurrency the caller asked for — and those
    /// numbers do not stay in the report, they go on to
    /// `PerfSummary::tps_mean`/`ttft_p50`, which anyone comparing endpoints
    /// against a population reads as a property of the model.
    #[test]
    fn the_clock_reading_and_the_gate_run_alone() {
        let exclusive: Vec<&str> = registry()
            .iter()
            .filter(|s| s.exclusive)
            .map(|s| s.id)
            .collect();
        assert_eq!(exclusive, vec!["preflight", "perf"]);
    }

    /// Whatever else concurrency changed, it must not have changed what a seed
    /// asks. The report records the seed so a seller can be shown the questions
    /// they were scored on; if scheduling could move them, that record is a
    /// fiction.
    #[test]
    fn a_step_s_questions_depend_on_the_seed_and_its_own_id_only() {
        let ctx = |seed| {
            Ctx::with_seed(
                Client::with_http(crate::client::Endpoint::default(), reqwest::Client::new()),
                Depth::Fast,
                Lang::En,
                "m".into(),
                seed,
            )
        };
        let a = ctx(0xC0FFEE);
        let b = ctx(0xC0FFEE);
        // Same seed, same step: identical, and drawing for some other step in
        // between cannot disturb it — which is exactly what a shared generator
        // could not promise once the steps stopped running in a fixed order.
        let first = a.rng_for("capability").hex(8);
        let _ = a.rng_for("cache_replay").hex(8);
        let _ = a.rng_for("stop_sequence").hex(8);
        assert_eq!(a.rng_for("capability").hex(8), first);
        assert_eq!(b.rng_for("capability").hex(8), first);

        // Two steps never share a stream, or one of them asking a question
        // fewer would shift every question after it.
        assert_ne!(a.rng_for("cache_replay").hex(8), first);
        // And a different run asks differently.
        assert_ne!(ctx(0xC0FFEF).rng_for("capability").hex(8), first);
    }

    #[test]
    fn a_schedule_overlaps_only_when_it_can() {
        assert!(!Schedule::sequential().overlaps());
        assert!(!Schedule::concurrent(1).overlaps());
        assert!(Schedule::concurrent(4).overlaps());
        // Pacing wins. Spreading a run out to be hard to spot and compressing
        // it into the smallest possible burst cannot both be had.
        let paced = Schedule {
            pace: Some(Pace {
                min: std::time::Duration::from_secs(20),
                max: std::time::Duration::from_secs(180),
            }),
            concurrency: 8,
        };
        assert!(!paced.overlaps());
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
