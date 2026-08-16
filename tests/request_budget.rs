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
