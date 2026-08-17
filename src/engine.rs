// SPDX-License-Identifier: Apache-2.0
//! One entry point for a whole verification run.
//!
//! Everything the CLI does after argument parsing happens here, so an embedder
//! gets the same run the command line gets — same probes, same order, same
//! verdict logic — without reimplementing the wiring. That equivalence is the
//! point: a marketplace that gates listings on this must be able to say its
//! gate and its published tool agree, and the only way to guarantee that is for
//! there to be one implementation.

use crate::client::{Client, Endpoint};
use crate::i18n::Lang;
use crate::probes::{self, Cancel, Ctx, Depth, Event, Pace, Selection};
use crate::report::Report;
use crate::verdict;
use anyhow::Result;

/// Everything a run needs.
#[derive(Clone)]
pub struct RunConfig {
    pub endpoint: Endpoint,
    /// The model the vendor claims to serve, when it differs from the id being
    /// requested. Defaults to `endpoint.model`.
    pub claimed_model: Option<String>,
    pub depth: Depth,
    pub lang: Lang,
    pub selection: Selection,
    /// `None` draws one from the clock — see [`Rng::from_seed`] for why an
    /// embedder should choose its own instead.
    pub seed: Option<u64>,
    /// Reuse the caller's HTTP client. See [`Client::with_http`].
    pub http: Option<reqwest::Client>,
    /// Spread the run out instead of issuing it as a burst. See [`Pace`].
    pub pace: Option<Pace>,
    /// Overlap the steps instead of running them one at a time. See
    /// [`RunConfig::concurrency`].
    pub concurrency: usize,
    /// Requests this run may have in flight at once, whatever the schedule.
    /// See [`RunConfig::max_in_flight`].
    pub max_in_flight: usize,
}

impl RunConfig {
    pub fn new(endpoint: Endpoint) -> Self {
        RunConfig {
            endpoint,
            claimed_model: None,
            depth: Depth::Balanced,
            lang: Lang::En,
            selection: Selection::all(),
            seed: None,
            http: None,
            pace: None,
            concurrency: 1,
            max_in_flight: 0,
        }
    }

    /// Probe only what survives a relay — see [`probes::Subject`].
    pub fn model_only(mut self) -> Self {
        self.selection = Selection::model_only();
        self
    }

    /// The cheapest run that still supports a verdict about the model.
    ///
    /// [`Selection::turbo`] with the steps overlapped and the traffic capped.
    /// Twelve requests at [`Depth::Fast`], in roughly four round trips instead
    /// of twenty-one, which is the difference between a run somebody can watch
    /// finish and one they give up on.
    ///
    /// `in_flight` is the only number here worth thinking about, and it is a
    /// statement about the endpoint rather than about this crate: how many
    /// simultaneous requests it will answer without queueing or refusing. Set
    /// it too high and the run measures the endpoint's saturation instead of
    /// its behaviour — and, on anything with a per-account concurrency budget,
    /// spends that budget on being examined. When in doubt, three.
    pub fn turbo(mut self, in_flight: usize) -> Self {
        self.selection = Selection::turbo();
        self.depth = Depth::Fast;
        self.concurrency = in_flight.max(1);
        self.max_in_flight = in_flight;
        self
    }

    /// How many steps may be in flight at once. `1` is sequential, the default,
    /// and what every release before 0.5.0 did.
    ///
    /// Ignored while a [`Pace`] is set: pacing exists to make a run hard to
    /// pick out of ordinary traffic, and overlapping exists to compress it into
    /// the smallest possible burst. A caller asking for both is asking to be
    /// unobtrusive quickly, and pacing wins.
    ///
    /// Steps marked [`exclusive`](probes::ProbeSpec::exclusive) still run alone
    /// — `preflight` because everything after it depends on its answer, `perf`
    /// because a latency measurement taken alongside this run's own traffic
    /// measures this run.
    pub fn concurrency(mut self, n: usize) -> Self {
        self.concurrency = n.max(1);
        self
    }

    /// Cap the requests in flight, independently of how many steps are.
    ///
    /// Steps are uneven — the capability battery is a dozen requests and
    /// `stop_sequence` is one — so a step count does not bound what the
    /// endpoint sees. This does. `0` leaves it uncapped.
    pub fn max_in_flight(mut self, n: usize) -> Self {
        self.max_in_flight = n;
        self
    }

    pub fn depth(mut self, d: Depth) -> Self {
        self.depth = d;
        self
    }

    pub fn lang(mut self, l: Lang) -> Self {
        self.lang = l;
        self
    }

    pub fn seed(mut self, s: u64) -> Self {
        self.seed = Some(s);
        self
    }

    pub fn claimed_model(mut self, m: impl Into<String>) -> Self {
        self.claimed_model = Some(m.into());
        self
    }

    pub fn http(mut self, c: reqwest::Client) -> Self {
        self.http = Some(c);
        self
    }

    /// Wait a random interval between steps — see [`Pace`].
    pub fn pace(mut self, min: std::time::Duration, max: std::time::Duration) -> Self {
        self.pace = Some(Pace { min, max });
        self
    }
}

/// Run the suite and assemble the report.
///
/// Progress arrives through `on_event`; pass `&mut |_| {}` to ignore it.
/// `cancel` is checked between steps — see [`Cancel`].
pub async fn run(
    cfg: RunConfig,
    cancel: &Cancel,
    on_event: &mut (dyn FnMut(Event<'_>) + Send),
) -> Result<Report> {
    let started_at = crate::util::iso8601_utc();
    let t0 = crate::util::now_ms();

    let seed = cfg.seed.unwrap_or_else(|| {
        // Same source `Rng::new` uses, surfaced so the report can record it.
        (crate::util::now_ms() as u64) ^ 0x9E37_79B9_7F4A_7C15
    });
    let claimed_model = cfg
        .claimed_model
        .clone()
        .unwrap_or_else(|| cfg.endpoint.model.clone());
    let protocol = cfg.endpoint.protocol;
    let model = cfg.endpoint.model.clone();
    let base_url = cfg.endpoint.base_url.clone();
    let host = cfg.endpoint.host();

    let client = match cfg.http.clone() {
        Some(http) => Client::with_http(cfg.endpoint.clone(), http),
        None => Client::new(cfg.endpoint.clone())?,
    };
    let client = client.with_limit(cfg.max_in_flight);
    let ctx = Ctx::with_seed(client, cfg.depth, cfg.lang, claimed_model.clone(), seed);

    let specs = cfg.selection.resolve();
    // Custom probes are named here alongside the built-in steps. A report that
    // listed only the public suite would understate what the run actually
    // asked — and the whole point of the private ones is that the list is the
    // only place they are visible.
    let steps: Vec<String> = specs
        .iter()
        .map(|s| s.id.to_string())
        .chain(
            cfg.selection
                .resolve_extra()
                .iter()
                .map(|p| p.id().to_string()),
        )
        .collect();
    let extra = cfg.selection.resolve_extra();
    let schedule = probes::Schedule {
        pace: cfg.pace,
        concurrency: cfg.concurrency,
    };
    let results = probes::run_with_extra(&ctx, &specs, &extra, cancel, schedule, on_event).await;

    let l = cfg.lang;
    let identity = verdict::build_identity(&results, &claimed_model, l);
    let billing = verdict::build_billing(&results, &model, l);
    let channel = verdict::build_channel(&results, l);
    let v = verdict::decide(&results, &identity, &billing, &channel, protocol, l);
    let perf = probes::perf::summarize(&ctx.perf.lock().unwrap());

    let skipped = results
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                crate::report::Status::Skip | crate::report::Status::Error
            )
        })
        .map(|r| t!(l, "{} ({}) — {}", "{}（{}）：{}", r.label, r.id, r.summary))
        .collect();

    Ok(Report {
        schema_version: crate::report::schema_version(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        lang: l,
        started_at,
        finished_at: crate::util::iso8601_utc(),
        duration_ms: (crate::util::now_ms() - t0) as u64,
        host,
        base_url,
        protocol,
        model,
        claimed_model,
        depth: cfg.depth.as_str().to_string(),
        seed,
        steps,
        request_count: ctx.client.requests(),
        results,
        verdict: v,
        identity,
        billing,
        channel,
        perf,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine has to be awaitable from a multi-threaded runtime, which is
    /// the whole reason `Ctx` holds locks instead of `RefCell`s. A regression
    /// here is a compile error rather than a test failure, which is the point:
    /// this exists so that reintroducing a `!Send` field cannot pass CI.
    #[test]
    fn run_future_is_send() {
        fn assert_send<T: Send>(_: T) {}
        let cfg = RunConfig::new(Endpoint {
            base_url: "https://example.invalid".into(),
            model: "m".into(),
            ..Default::default()
        });
        let cancel = Cancel::new();
        let mut sink = |_: Event<'_>| {};
        assert_send(run(cfg, &cancel, &mut sink));
    }

    fn ids(sel: Selection) -> Vec<&'static str> {
        sel.resolve().iter().map(|s| s.id).collect()
    }

    #[test]
    fn model_only_drops_endpoint_steps_but_keeps_preflight() {
        let ids = ids(Selection::model_only());
        assert!(
            ids.contains(&"preflight"),
            "the run is meaningless without it"
        );
        assert!(ids.contains(&"self_id"));
        assert!(ids.contains(&"capability"));
        assert!(ids.contains(&"perf"));
        // These read the endpoint's own contract and accounting, which behind a
        // relay belong to the relay.
        assert!(!ids.contains(&"billing"));
        assert!(!ids.contains(&"channel"));
        assert!(!ids.contains(&"missing_auth"));
    }

    #[test]
    fn skip_wins_over_an_explicit_include() {
        let sel = Selection {
            only: vec!["identity".into(), "perf".into()],
            skip: vec!["perf".into()],
            ..Default::default()
        };
        let ids = ids(sel);
        assert!(ids.contains(&"self_id"));
        assert!(!ids.contains(&"perf"));
    }

    /// Splitting the identity step into seven must not have taken away the way
    /// callers already addressed it. `skip: ["identity"]` meant "no identity
    /// probing" before 0.5.0 and has to keep meaning that, because the
    /// alternative is every existing caller silently starting to run steps they
    /// had turned off.
    #[test]
    fn a_group_key_still_addresses_the_whole_family() {
        let without = ids(Selection {
            skip: vec!["identity".into()],
            ..Default::default()
        });
        for gone in ["self_id", "meta_creator", "world_knowledge", "capability"] {
            assert!(!without.contains(&gone), "{gone} survived skip: [identity]");
        }
        assert!(without.contains(&"preflight"));

        let only_identity = ids(Selection {
            only: vec!["identity".into()],
            ..Default::default()
        });
        assert!(only_identity.contains(&"self_id") && only_identity.contains(&"verbosity"));
        assert!(!only_identity.contains(&"cache_replay"));
    }

    #[test]
    fn turbo_keeps_every_probe_a_verdict_rests_on() {
        let kept_ids = ids(Selection::turbo());
        // The family gate, the capability measurement and the tier it implies,
        // the two population axes, and the one hard gate that survives a relay.
        for kept in [
            "preflight",
            "self_id",
            "capability",
            "verbosity",
            "perf",
            "cache_replay",
        ] {
            assert!(kept_ids.contains(&kept), "turbo dropped {kept}");
        }
        // Corroboration, description and a fan-out check that describes the
        // relay rather than the model.
        for dropped in [
            "meta_creator",
            "context_claim",
            "cutoff_claim",
            "world_knowledge",
            "signature_drift",
        ] {
            assert!(!kept_ids.contains(&dropped), "turbo kept {dropped}");
        }
        // And nothing whose subject is the endpoint, which turbo inherits from
        // being a strict subset of `model_only`.
        let relayed = ids(Selection::model_only());
        for id in &kept_ids {
            assert!(relayed.contains(id), "{id} is not in model_only");
        }
    }

    #[test]
    fn plus_puts_the_endpoint_contract_checks_back() {
        let ids = ids(Selection::turbo().plus(["stop_sequence", "system_adherence"]));
        assert!(ids.contains(&"stop_sequence"));
        assert!(ids.contains(&"system_adherence"));
        assert!(ids.contains(&"self_id"), "and keeps what turbo had");
    }

    /// `plus` on a selection with no `only` list must not narrow it. An empty
    /// `only` means "everything", so extending it would turn a full run into a
    /// two-step one — the exact opposite of what the name promises.
    #[test]
    fn plus_on_an_unfiltered_selection_is_a_no_op() {
        let before = ids(Selection::all()).len();
        let after = ids(Selection::all().plus(["self_id"])).len();
        assert_eq!(before, after);
    }
}
