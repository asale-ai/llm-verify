// SPDX-License-Identifier: Apache-2.0
//! Endpoints that reject non-default `temperature`.
//!
//! Some relays serve a model that accepts only the provider's default sampling
//! value, and answer any request that names another one with
//! `400 ... does not support non-default temperature`. That is a property of
//! the endpoint, not a verdict about the model, but it aborts the preflight and
//! therefore the whole run.
//!
//! These tests pin the two ways out: the preflight learns the constraint and
//! retries without the field, and `no_temperature()` skips the refused attempt
//! entirely. A `400` that is about something else must still fail and must not
//! be retried — otherwise a config mistake would be masked as a retry storm.

use llm_verify::probes::{Cancel, Depth, Selection};
use llm_verify::report::Status;
use llm_verify::{engine, Endpoint, Protocol};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A plausible Anthropic response, so the preflight marks the endpoint reachable.
const OK_BODY: &str = r#"{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-4-5","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":40,"output_tokens":8}}"#;

/// The exact refusal reported by the relay that motivated this change.
const TEMPERATURE_400: &str = r#"{"error":{"message":"claude-opus-4-5 does not support non-default temperature","type":"invalid_request_error"},"type":"error"}"#;

/// A different `400`, which must not trigger the temperature fallback.
const OTHER_400: &str = r#"{"error":{"message":"model not found","type":"invalid_request_error"},"type":"error"}"#;

#[derive(Clone, Copy)]
enum Mode {
    RejectTemperature,
    RejectOther,
}

/// Read one request, headers and body. Content-length decides when it ends;
/// a single `read` is not enough in principle, and a flaky test here would
/// look like a temperature-fallback regression.
async fn read_http(sock: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n".as_slice()) {
            let header = String::from_utf8_lossy(&buf[..pos]);
            let len = header
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    if k.eq_ignore_ascii_case("content-length") {
                        v.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            if buf.len() >= pos + 4 + len {
                break;
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// Serve canned responses and record every request body.
async fn stub(mode: Mode) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let Some(request) = read_http(&mut sock).await else {
                    return;
                };
                let body = request
                    .split_once("\r\n\r\n")
                    .map(|(_, b)| b.to_string())
                    .unwrap_or_default();
                seen.lock().unwrap().push(body.clone());

                let (status, reason, payload) = match mode {
                    Mode::RejectTemperature if body.contains("\"temperature\"") => {
                        (400, "Bad Request", TEMPERATURE_400.to_string())
                    }
                    Mode::RejectOther => (400, "Bad Request", OTHER_400.to_string()),
                    _ => (200, "OK", OK_BODY.to_string()),
                };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), bodies)
}

/// A run that only needs the preflight: tiny, and it is the step under test.
fn config(base_url: String) -> engine::RunConfig {
    let mut cfg = engine::RunConfig::new(Endpoint {
        base_url,
        api_key: "k".into(),
        protocol: Protocol::Anthropic,
        model: "claude-opus-4-5".into(),
        ..Default::default()
    })
    .depth(Depth::Fast);
    cfg.selection = Selection {
        only: vec!["preflight".into()],
        ..Default::default()
    };
    cfg
}

fn preflight(report: &llm_verify::report::Report) -> &llm_verify::report::ProbeResult {
    report
        .results
        .iter()
        .find(|r| r.id == "preflight")
        .expect("preflight must be in the report")
}

#[tokio::test]
async fn retries_without_temperature_after_the_matching_400() {
    let (base_url, bodies) = stub(Mode::RejectTemperature).await;
    let report = engine::run(config(base_url), &Cancel::new(), &mut |_| {})
        .await
        .expect("run must not fail at the transport layer");

    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        2,
        "expected one refused attempt and one retry, got {bodies:?}"
    );
    assert!(
        bodies[0].contains("\"temperature\""),
        "first attempt must still name a temperature: {}",
        bodies[0]
    );
    assert!(
        !bodies[1].contains("\"temperature\""),
        "retry must omit temperature: {}",
        bodies[1]
    );

    let p = preflight(&report);
    assert_eq!(p.status, Status::Pass, "preflight should succeed after retry");
    assert!(
        p.findings
            .iter()
            .any(|f| f.to_ascii_lowercase().contains("temperature")),
        "the report must say sampling was not pinned: {:?}",
        p.findings
    );
    assert_eq!(
        p.metrics.get("temperature_omitted"),
        Some(&serde_json::Value::Bool(true))
    );
}

#[tokio::test]
async fn no_temperature_skips_the_refused_attempt() {
    let (base_url, bodies) = stub(Mode::RejectTemperature).await;
    let report = engine::run(
        config(base_url).no_temperature(),
        &Cancel::new(),
        &mut |_| {},
    )
    .await
    .expect("run must not fail at the transport layer");

    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        1,
        "a caller that already knows must not spend a refused request: {bodies:?}"
    );
    assert!(
        !bodies[0].contains("\"temperature\""),
        "temperature must be absent from the first request: {}",
        bodies[0]
    );

    let p = preflight(&report);
    assert_eq!(p.status, Status::Pass);
    assert_eq!(
        p.metrics.get("temperature_omitted"),
        Some(&serde_json::Value::Bool(true))
    );
}

#[tokio::test]
async fn a_400_that_is_not_about_temperature_is_not_retried() {
    let (base_url, bodies) = stub(Mode::RejectOther).await;
    let report = engine::run(config(base_url), &Cancel::new(), &mut |_| {})
        .await
        .expect("run must not fail at the transport layer");

    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        1,
        "unrelated 400s must not be retried: {bodies:?}"
    );

    let p = preflight(&report);
    assert_eq!(p.status, Status::Fail);
    assert!(
        p.metrics.get("temperature_omitted").is_none(),
        "no omission happened, so no such metric should be claimed"
    );
}
