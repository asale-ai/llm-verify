// SPDX-License-Identifier: Apache-2.0
//! How many requests a run actually costs.
//!
//! Every probe is a real call to a real endpoint, and for an embedder reselling
//! somebody else's capacity that is real money and real quota off a stranger's
//! subscription. "Roughly a dozen" is not good enough to size a sampling rate
//! against, and the number is not something you can read off the source —
//! several steps loop on `Depth`, several skip themselves depending on what
//! earlier ones saw.
//!
//! So it is measured here, against a stub that answers everything, and pinned.
//! The point of the pin is not the exact figure: it is that adding a probe to
//! the suite cannot quietly multiply what every caller pays without somebody
//! having to update this file and notice.

use llm_verify::probes::{Cancel, Depth, Selection};
use llm_verify::{engine, Endpoint, Protocol};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A plausible Anthropic response, so probes proceed rather than bailing.
///
/// Content matters less than well-formedness: a probe that gets an unparseable
/// body records a failure and moves on, which is fine for counting, but one
/// that gets a 401 or a 404 makes `preflight` declare the endpoint unreachable
/// and the whole run stops after a single request.
const BODY: &str = r#"{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-4-5","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":40,"output_tokens":8}}"#;

/// Serve canned responses and count what arrives.
async fn stub() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                // One request per connection is enough: the client does not
                // pipeline, and reqwest reopens as needed.
                let Ok(n) = sock.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                seen.fetch_add(1, Ordering::Relaxed);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    BODY.len(),
                    BODY
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), hits)
}

async fn count(selection: Selection, depth: Depth) -> usize {
    let (base_url, hits) = stub().await;
    let mut cfg = engine::RunConfig::new(Endpoint {
        base_url,
        api_key: "k".into(),
        protocol: Protocol::Anthropic,
        model: "claude-opus-4-5".into(),
        ..Default::default()
    })
    .depth(depth)
    // Fixed, so the count is the same on every machine and every run. The
    // payloads a seed produces do not change how many there are, but a probe
    // that branches on a generated value could, and a flaky budget test is
    // worse than none.
    .seed(0xA11CE);
    cfg.selection = selection;
    let report = engine::run(cfg, &Cancel::new(), &mut |_| {}).await.unwrap();
    // The client's own ledger and the wire agree, or one of them is lying.
    assert_eq!(
        report.request_count as usize,
        hits.load(Ordering::Relaxed),
        "the reported request count must match what the endpoint actually received"
    );
    report.request_count as usize
}

/// The configuration a marketplace samples with: relay-surviving probes only,
/// shallowest depth.
///
/// This is the per-sample bill. Multiply by the sampling rate and the lane
/// count before turning sampling on.
#[tokio::test]
async fn model_only_at_fast_depth_stays_within_its_budget() {
    let n = count(Selection::model_only(), Depth::Fast).await;
    println!("model_only + fast = {n} requests");
    assert!(
        (8..=22).contains(&n),
        "a sampling run costs {n} requests; if that is intended, update this bound \
         and whoever is paying for it"
    );
}

/// The turbo configuration, which is the one a seller waits on.
///
/// Under half of `model_only`, and the half it drops was chosen probe by probe
/// — see [`Selection::turbo`]. Pinned tightly on purpose: turbo exists to be
/// cheap, and a step quietly added back into it would be a run no longer worth
/// having a second preset for.
#[tokio::test]
async fn turbo_at_fast_depth_is_under_half_of_model_only() {
    let turbo = count(Selection::turbo(), Depth::Fast).await;
    let sample = count(Selection::model_only(), Depth::Fast).await;
    println!("turbo + fast = {turbo} requests, model_only = {sample}");
    assert!(
        (8..=10).contains(&turbo),
        "turbo costs {turbo} requests; if that is intended, update this bound and the \
         doc comment on `Selection::turbo` that quotes it"
    );
    assert!(
        turbo * 2 <= sample,
        "turbo ({turbo}) is not meaningfully cheaper than model_only ({sample})"
    );
}

/// Handing the grading to a private bank has to *replace* the published
/// battery, not sit beside it.
///
/// The mistake this guards is paying for both: an embedder appends its own
/// probe, forgets that `capability` is still in the selection, and every run
/// quietly asks two sets of graded questions. Three requests is a third of
/// turbo, and nothing in the report would look wrong.
#[tokio::test]
async fn dropping_the_published_battery_refunds_its_requests() {
    let with = count(Selection::turbo(), Depth::Fast).await;
    let without = count(Selection::turbo().minus(["capability"]), Depth::Fast).await;
    println!("turbo = {with} requests, turbo minus capability = {without}");
    assert_eq!(
        with - without,
        3,
        "the battery is three requests at Depth::Fast"
    );
}

/// A probe standing in for a built-in step actually runs.
///
/// The bug this exists for shipped nowhere but came within one measurement of
/// it. Replacing the published battery with a private one was written the
/// obvious way — `minus(["capability"])` to drop the built-in, `with(bank)` to
/// add the replacement — and `skip` applies to custom probes too, on purpose,
/// so it removed both. The run came back a step short with every other probe
/// passing: no error, no warning, a verdict assembled out of a capability
/// measurement nobody took.
///
/// Nothing about the report would have said so. Only the request count does.
#[tokio::test]
async fn a_replacement_probe_is_not_deleted_by_the_step_it_replaces() {
    struct Bank;
    impl llm_verify::probes::Probe for Bank {
        // The same id as the step it stands in for, which is the entire point:
        // everything downstream looks capability measurements up by name.
        fn id(&self) -> &str {
            "capability"
        }
        fn run<'a>(
            &'a self,
            ctx: &'a llm_verify::probes::Ctx,
        ) -> llm_verify::probes::ProbeFuture<'a> {
            Box::pin(async move {
                let req = llm_verify::protocol::ChatRequest::new(
                    &ctx.client.endpoint.model,
                    "a question the endpoint has never seen",
                );
                let _ = ctx.client.chat(&req).await;
                vec![llm_verify::report::ProbeResult::new(
                    "capability",
                    "battery",
                    llm_verify::report::Group::Identity,
                )
                .pass("ok")]
            })
        }
    }

    let base = count(Selection::turbo().minus(["capability"]), Depth::Fast).await;
    let replaced = count(
        Selection::turbo().replacing("capability", Arc::new(Bank)),
        Depth::Fast,
    )
    .await;
    assert_eq!(
        replaced,
        base + 1,
        "the replacement probe never issued its request — it was dropped by the \
         skip that removed the step it replaces"
    );
}

/// Concurrency is a schedule, not a shortcut.
///
/// Overlapping the steps must change how long a run takes and nothing else.
/// The bill is the one thing an embedder reselling somebody else's capacity
/// cannot check by looking, so it is checked here: same selection, same depth,
/// same number of requests, whatever the schedule.
#[tokio::test]
async fn overlapping_the_steps_does_not_change_what_is_asked() {
    let sequential = count(Selection::turbo(), Depth::Fast).await;
    let (base_url, hits) = stub().await;
    let cfg = engine::RunConfig::new(Endpoint {
        base_url,
        api_key: "k".into(),
        protocol: Protocol::Anthropic,
        model: "claude-opus-4-5".into(),
        ..Default::default()
    })
    .turbo(4)
    .seed(0xA11CE);
    let report = engine::run(cfg, &Cancel::new(), &mut |_| {}).await.unwrap();
    assert_eq!(report.request_count as usize, hits.load(Ordering::Relaxed));
    assert_eq!(
        report.request_count as usize, sequential,
        "the same selection asked a different number of questions when overlapped"
    );
    // And the results still arrive in registry order, so anything reading the
    // report positionally rather than by id cannot tell either.
    let ids: Vec<&str> = report.results.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.first(), Some(&"preflight"));
    assert!(
        ids.iter().position(|i| *i == "self_id") < ids.iter().position(|i| *i == "ttft"),
        "identity results should still precede the perf ones: {ids:?}"
    );
}

/// What a drift-triggered recheck costs.
///
/// `Kind::Recheck` probes at `forensic`, and unlike the other two it is not
/// started by anyone: a lane that drifts from its population triggers one on
/// its own. An automatic path that spends real money needs a known ceiling,
/// and this is the only place that number exists.
#[tokio::test]
async fn a_forensic_recheck_has_a_known_ceiling() {
    let n = count(Selection::model_only(), Depth::Forensic).await;
    println!("model_only + forensic = {n} requests");
    assert!(
        n <= 70,
        "an automatically-triggered recheck costs {n} requests; if that is intended, \
         raise this bound deliberately — nobody asks for this one"
    );
}

/// The whole suite, for reference — what the CLI costs a user probing their own
/// endpoint, where nobody else's quota is involved.
#[tokio::test]
async fn the_full_suite_is_several_times_the_cost_of_a_sample() {
    let full = count(Selection::all(), Depth::Balanced).await;
    let sample = count(Selection::model_only(), Depth::Fast).await;
    println!("full + balanced = {full} requests, sample = {sample}");
    assert!(
        full > sample * 2,
        "the sampling configuration is supposed to be the cheap one ({sample} vs {full})"
    );
}
