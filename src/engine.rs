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
use crate::util::Rng;
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
        }
    }

    /// Probe only what survives a relay — see [`probes::Subject`].
    pub fn model_only(mut self) -> Self {
        self.selection = Selection::model_only();
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
    let ctx = Ctx::with_rng(
        client,
        cfg.depth,
        cfg.lang,
        claimed_model.clone(),
        Rng::from_seed(seed),
    );

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
    let results = probes::run_with_extra(&ctx, &specs, &extra, cancel, cfg.pace, on_event).await;

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

    #[test]
    fn model_only_drops_endpoint_steps_but_keeps_preflight() {
        let specs = Selection::model_only().resolve();
        let ids: Vec<&str> = specs.iter().map(|s| s.id).collect();
        assert!(
            ids.contains(&"preflight"),
            "the run is meaningless without it"
        );
        assert!(ids.contains(&"identity"));
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
        let ids: Vec<&str> = sel.resolve().iter().map(|s| s.id).collect();
        assert!(ids.contains(&"identity"));
        assert!(!ids.contains(&"perf"));
    }
}
