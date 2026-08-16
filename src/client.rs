// SPDX-License-Identifier: Apache-2.0
//! HTTP transport. Deliberately low-level: several probes work by *removing*
//! things a well-behaved client would always send (the auth header, the API
//! version header) or by sending a body that is not valid JSON, so nothing
//! here may quietly normalise a request on our behalf.

use crate::protocol::{ChatRequest, ChatResponse, Protocol};
use crate::util::now_ms;
use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

/// Sent on every request so operators can identify the traffic in their logs.
const UA: &str = concat!("llm-verify/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub protocol: Protocol,
    pub model: String,
    pub anthropic_version: String,
    pub timeout: Duration,
    /// Sent on every request, ahead of any per-probe `extra_headers`.
    ///
    /// The CLI leaves this empty. An embedder uses it for whatever its own
    /// front door requires — routing headers, a tenant id, a marker that says
    /// which of its callers this run belongs to. It is deliberately *not*
    /// merged into `auth_headers`' omit logic: a probe that removes the API key
    /// to see how the endpoint answers must still be routed to it.
    pub headers: Vec<(String, String)>,
}

impl Default for Endpoint {
    fn default() -> Self {
        Endpoint {
            base_url: String::new(),
            api_key: String::new(),
            protocol: Protocol::OpenAI,
            model: String::new(),
            anthropic_version: "2023-06-01".to_string(),
            timeout: Duration::from_secs(120),
            headers: Vec::new(),
        }
    }
}

impl Endpoint {
    /// Join the base URL with a protocol path, inserting `/v1` only when the
    /// base does not already carry a version segment. Both
    /// `https://api.anthropic.com` and `https://relay.example/api/v1` are
    /// common in the wild and must both resolve correctly.
    pub fn url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let last = base.rsplit('/').next().unwrap_or("");
        let versioned = last.len() >= 2
            && last.starts_with('v')
            && last[1..2].chars().all(|c| c.is_ascii_digit());
        if versioned {
            format!("{base}{path}")
        } else {
            format!("{base}/v1{path}")
        }
    }

    pub fn host(&self) -> String {
        self.base_url
            .split("://")
            .nth(1)
            .unwrap_or(&self.base_url)
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

/// Knobs that let a probe deviate from a well-formed request on purpose.
#[derive(Debug, Clone, Default)]
pub struct RequestOpts {
    pub omit_auth: bool,
    pub omit_version: bool,
    /// Replaces the serialised body verbatim — used to send malformed JSON.
    pub raw_body: Option<Vec<u8>>,
    pub extra_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub duration_ms: u64,
}

impl RawResponse {
    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.body).ok()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(|s| s.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One parsed Server-Sent Event.
#[derive(Debug, Clone)]
pub struct SseEvent {
    /// `event:` field, empty for data-only streams such as OpenAI's.
    pub name: String,
    pub data: String,
    /// Milliseconds from request send to this event arriving.
    pub at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct StreamResult {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub events: Vec<SseEvent>,
    /// Time to the first event carrying actual content text — not role
    /// assignments, not `message_start`, not pings. This is what a user feels
    /// as lag before the answer starts appearing.
    pub ttft_ms: Option<u64>,
    pub total_ms: u64,
    pub text: String,
    pub saw_done_sentinel: bool,
    pub content_type: String,
    /// Bytes received, so an empty keep-alive stream is distinguishable from
    /// a stream that failed to open at all.
    pub bytes: usize,
    pub usage: Option<crate::protocol::Usage>,
    pub error: Option<String>,
}

impl StreamResult {
    pub fn event_names(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|e| {
                if e.name.is_empty() {
                    "data".to_string()
                } else {
                    e.name.clone()
                }
            })
            .collect()
    }
}

pub struct Client {
    http: reqwest::Client,
    pub endpoint: Endpoint,
    /// Every request the run has made, for the report's request ledger.
    ///
    /// Atomic rather than `Cell` so a probe future stays `Send`. Probes run
    /// sequentially and nothing here is contended; what the atomic buys is the
    /// ability to `await` this engine from a multi-threaded runtime — an
    /// embedder's request handler cannot hold a `!Send` future.
    pub request_count: std::sync::atomic::AtomicU32,
}

impl Client {
    pub fn new(endpoint: Endpoint) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(endpoint.timeout)
            .connect_timeout(Duration::from_secs(15))
            .user_agent(UA)
            // Redirects would let a relay bounce our probe to a different
            // host without us noticing which one actually answered.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self::with_http(endpoint, http))
    }

    /// Build on a caller-supplied HTTP client.
    ///
    /// An embedder generally already has one, configured for its own network:
    /// a proxy it must egress through, a connection pool it wants shared, a
    /// root store it pins. Rebuilding that here would either duplicate the
    /// configuration or quietly ignore it.
    ///
    /// The caller owns the timeout and the redirect policy that come with the
    /// client it passes. Both matter to what the probes mean — a client that
    /// follows redirects can be bounced to a different host mid-run without
    /// the report saying so — so a caller that has no strong opinion should
    /// use [`Client::new`].
    pub fn with_http(endpoint: Endpoint, http: reqwest::Client) -> Self {
        Self {
            http,
            endpoint,
            request_count: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Requests issued so far.
    pub fn requests(&self) -> u32 {
        self.request_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn count_request(&self) {
        self.request_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn auth_headers(&self, opts: &RequestOpts) -> Vec<(String, String)> {
        let mut h = vec![("content-type".to_string(), "application/json".to_string())];
        h.extend(self.endpoint.headers.iter().cloned());
        if !opts.omit_auth {
            match self.endpoint.protocol {
                Protocol::Anthropic => {
                    h.push(("x-api-key".into(), self.endpoint.api_key.clone()));
                    // Many Anthropic-compatible relays only accept Bearer.
                    h.push((
                        "authorization".into(),
                        format!("Bearer {}", self.endpoint.api_key),
                    ));
                }
                Protocol::OpenAI => {
                    h.push((
                        "authorization".into(),
                        format!("Bearer {}", self.endpoint.api_key),
                    ));
                }
            }
        }
        if self.endpoint.protocol == Protocol::Anthropic && !opts.omit_version {
            h.push((
                "anthropic-version".into(),
                self.endpoint.anthropic_version.clone(),
            ));
        }
        h.extend(opts.extra_headers.iter().cloned());
        h
    }

    /// POST a JSON body and return the raw response without interpreting it.
    pub async fn post_raw(
        &self,
        path: &str,
        body: &Value,
        opts: &RequestOpts,
    ) -> Result<RawResponse> {
        self.count_request();
        let url = self.endpoint.url(path);
        let payload = match &opts.raw_body {
            Some(b) => b.clone(),
            None => serde_json::to_vec(body)?,
        };

        let mut req = self.http.post(&url).body(payload);
        for (k, v) in self.auth_headers(opts) {
            req = req.header(k, v);
        }

        let started = now_ms();
        let resp = req
            .send()
            .await
            .with_context(|| format!("POST {url} failed"))?;
        let status = resp.status().as_u16();
        let headers = collect_headers(resp.headers());
        let body = resp.text().await.unwrap_or_default();
        Ok(RawResponse {
            status,
            headers,
            body,
            duration_ms: (now_ms() - started) as u64,
        })
    }

    pub async fn get_raw(&self, path: &str, opts: &RequestOpts) -> Result<RawResponse> {
        self.count_request();
        let url = self.endpoint.url(path);
        let mut req = self.http.get(&url);
        for (k, v) in self.auth_headers(opts) {
            req = req.header(k, v);
        }
        let started = now_ms();
        let resp = req
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;
        let status = resp.status().as_u16();
        let headers = collect_headers(resp.headers());
        let body = resp.text().await.unwrap_or_default();
        Ok(RawResponse {
            status,
            headers,
            body,
            duration_ms: (now_ms() - started) as u64,
        })
    }

    /// Send a chat request and parse it. Returns both the parsed view and the
    /// raw response, because several probes assert on headers and status.
    pub async fn chat(&self, req: &ChatRequest) -> Result<(ChatResponse, RawResponse)> {
        self.chat_with(req, &RequestOpts::default()).await
    }

    pub async fn chat_with(
        &self,
        req: &ChatRequest,
        opts: &RequestOpts,
    ) -> Result<(ChatResponse, RawResponse)> {
        let proto = self.endpoint.protocol;
        let raw = self
            .post_raw(proto.chat_path(), &req.to_body(proto), opts)
            .await?;
        if !raw.ok() {
            return Err(anyhow!(
                "HTTP {} from {}: {}",
                raw.status,
                self.endpoint.host(),
                crate::util::truncate(raw.body.trim(), 240)
            ));
        }
        let v = raw.json().ok_or_else(|| {
            anyhow!(
                "response body was not JSON: {}",
                crate::util::truncate(&raw.body, 200)
            )
        })?;
        Ok((ChatResponse::parse(proto, &v), raw))
    }

    /// Stream a chat request, timing the first content-bearing event.
    pub async fn stream(&self, req: &ChatRequest) -> Result<StreamResult> {
        self.count_request();
        let proto = self.endpoint.protocol;
        let body = req.clone().stream(true).to_body(proto);
        let url = self.endpoint.url(proto.chat_path());

        let mut http_req = self.http.post(&url).body(serde_json::to_vec(&body)?);
        for (k, v) in self.auth_headers(&RequestOpts::default()) {
            http_req = http_req.header(k, v);
        }
        http_req = http_req.header("accept", "text/event-stream");

        let started = now_ms();
        let resp = http_req
            .send()
            .await
            .with_context(|| format!("POST {url} (stream) failed"))?;

        let mut out = StreamResult {
            status: resp.status().as_u16(),
            headers: collect_headers(resp.headers()),
            ..Default::default()
        };
        out.content_type = out.headers.get("content-type").cloned().unwrap_or_default();

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    out.error = Some(format!("stream aborted: {e}"));
                    break;
                }
            };
            out.bytes += chunk.len();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // Events are separated by a blank line; keep the trailing partial.
            while let Some(idx) = find_event_boundary(&buf) {
                let (raw_event, rest) = buf.split_at(idx);
                let raw_event = raw_event.to_string();
                buf = rest.trim_start_matches(['\r', '\n']).to_string();
                if let Some(ev) = parse_sse_block(&raw_event, (now_ms() - started) as u64) {
                    self.absorb_event(proto, ev, &mut out);
                }
            }
        }
        // Flush a final event that arrived without a trailing blank line.
        if !buf.trim().is_empty() {
            if let Some(ev) = parse_sse_block(&buf, (now_ms() - started) as u64) {
                self.absorb_event(proto, ev, &mut out);
            }
        }
        out.total_ms = (now_ms() - started) as u64;
        Ok(out)
    }

    fn absorb_event(&self, proto: Protocol, ev: SseEvent, out: &mut StreamResult) {
        if ev.data.trim() == "[DONE]" {
            out.saw_done_sentinel = true;
            out.events.push(ev);
            return;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&ev.data) {
            if let Some(delta) = extract_delta_text(proto, &v) {
                if !delta.is_empty() {
                    if out.ttft_ms.is_none() {
                        out.ttft_ms = Some(ev.at_ms);
                    }
                    out.text.push_str(&delta);
                }
            }
            if let Some(u) = extract_stream_usage(proto, &v) {
                // Later usage frames supersede earlier ones; Anthropic sends a
                // partial at message_start and the real totals at message_delta.
                out.usage = Some(match out.usage.take() {
                    Some(prev) => crate::protocol::Usage {
                        input_tokens: if u.input_tokens > 0 {
                            u.input_tokens
                        } else {
                            prev.input_tokens
                        },
                        output_tokens: if u.output_tokens > 0 {
                            u.output_tokens
                        } else {
                            prev.output_tokens
                        },
                        cache_create_tokens: u.cache_create_tokens.max(prev.cache_create_tokens),
                        cache_read_tokens: u.cache_read_tokens.max(prev.cache_read_tokens),
                        present: true,
                    },
                    None => u,
                });
            }
            if let Some(err) = v.get("error") {
                out.error = Some(crate::util::truncate(&err.to_string(), 200));
            }
        }
        out.events.push(ev);
    }

    /// Anthropic's authoritative token counter. `None` when the protocol has
    /// no such route; `Err` when the route exists but the endpoint refused.
    pub async fn count_tokens(&self, req: &ChatRequest) -> Option<Result<u32>> {
        let path = self.endpoint.protocol.count_tokens_path()?;
        let mut body = req.to_body(self.endpoint.protocol);
        // count_tokens rejects generation-only fields.
        for k in ["max_tokens", "temperature", "stream", "stop_sequences"] {
            if let Some(o) = body.as_object_mut() {
                o.remove(k);
            }
        }
        Some(
            match self.post_raw(path, &body, &RequestOpts::default()).await {
                Err(e) => Err(e),
                Ok(raw) if !raw.ok() => Err(anyhow!(
                    "count_tokens returned HTTP {}: {}",
                    raw.status,
                    crate::util::truncate(raw.body.trim(), 160)
                )),
                Ok(raw) => raw
                    .json()
                    .and_then(|v| v.get("input_tokens").and_then(|t| t.as_u64()))
                    .map(|t| t as u32)
                    .ok_or_else(|| anyhow!("count_tokens response had no input_tokens field")),
            },
        )
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let raw = self
            .get_raw(
                self.endpoint.protocol.models_path(),
                &RequestOpts::default(),
            )
            .await?;
        if !raw.ok() {
            return Err(anyhow!("HTTP {} from /models", raw.status));
        }
        let v = raw.json().ok_or_else(|| anyhow!("/models was not JSON"))?;
        let arr = v
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| anyhow!("/models had no data array"))?;
        Ok(arr
            .iter()
            .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
            .collect())
    }
}

// ── SSE parsing ────────────────────────────────────────────────────────────

fn collect_headers(h: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    h.iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}

/// Index just past the first `\n\n` (or `\r\n\r\n`) in the buffer.
fn find_event_boundary(buf: &str) -> Option<usize> {
    let a = buf.find("\n\n").map(|i| i + 2);
    let b = buf.find("\r\n\r\n").map(|i| i + 4);
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

fn parse_sse_block(block: &str, at_ms: u64) -> Option<SseEvent> {
    let mut name = String::new();
    let mut data = String::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("event:") {
            name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if name.is_empty() && data.is_empty() {
        return None;
    }
    Some(SseEvent { name, data, at_ms })
}

/// The incremental text carried by one streamed frame, if any.
fn extract_delta_text(proto: Protocol, v: &Value) -> Option<String> {
    match proto {
        Protocol::Anthropic => {
            if v.get("type").and_then(|t| t.as_str()) != Some("content_block_delta") {
                return None;
            }
            v.get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .map(String::from)
        }
        Protocol::OpenAI => v
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|t| t.as_str())
            .map(String::from),
    }
}

fn extract_stream_usage(proto: Protocol, v: &Value) -> Option<crate::protocol::Usage> {
    let u = match proto {
        Protocol::Anthropic => v
            .get("usage")
            .or_else(|| v.get("message").and_then(|m| m.get("usage")))?,
        Protocol::OpenAI => v.get("usage").filter(|u| !u.is_null())?,
    };
    let get = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    Some(match proto {
        Protocol::Anthropic => crate::protocol::Usage {
            input_tokens: get("input_tokens"),
            output_tokens: get("output_tokens"),
            cache_create_tokens: get("cache_creation_input_tokens"),
            cache_read_tokens: get("cache_read_input_tokens"),
            present: true,
        },
        Protocol::OpenAI => crate::protocol::Usage {
            input_tokens: get("prompt_tokens"),
            output_tokens: get("completion_tokens"),
            cache_create_tokens: 0,
            cache_read_tokens: u
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0) as u32,
            present: true,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ep(base: &str) -> Endpoint {
        Endpoint {
            base_url: base.into(),
            api_key: "k".into(),
            protocol: Protocol::Anthropic,
            model: "m".into(),
            anthropic_version: "2023-06-01".into(),
            timeout: Duration::from_secs(1),
            headers: Vec::new(),
        }
    }

    #[test]
    fn url_inserts_v1_only_when_absent() {
        assert_eq!(
            ep("https://api.anthropic.com").url("/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            ep("https://relay.example/api/v1").url("/messages"),
            "https://relay.example/api/v1/messages"
        );
        assert_eq!(
            ep("https://relay.example/api/v1/").url("/messages"),
            "https://relay.example/api/v1/messages"
        );
        // A path segment that merely starts with "v" is not a version.
        assert_eq!(
            ep("https://relay.example/vendor").url("/messages"),
            "https://relay.example/vendor/v1/messages"
        );
        assert_eq!(
            ep("https://x.dev/v1beta").url("/messages"),
            "https://x.dev/v1beta/messages"
        );
    }

    #[test]
    fn host_extracts_authority() {
        assert_eq!(ep("https://api.example.com/v1").host(), "api.example.com");
        assert_eq!(ep("http://localhost:8080").host(), "localhost:8080");
    }

    #[test]
    fn event_boundary_prefers_the_earliest_terminator() {
        assert_eq!(find_event_boundary("a\n\nb"), Some(3));
        assert_eq!(find_event_boundary("a\r\n\r\nb"), Some(5));
        assert_eq!(find_event_boundary("no terminator"), None);
    }

    #[test]
    fn parses_named_and_data_only_events() {
        let named = parse_sse_block("event: message_start\ndata: {\"a\":1}\n", 5).unwrap();
        assert_eq!(named.name, "message_start");
        assert_eq!(named.data, "{\"a\":1}");

        let data_only = parse_sse_block("data: [DONE]\n", 9).unwrap();
        assert!(data_only.name.is_empty());
        assert_eq!(data_only.data, "[DONE]");

        assert!(parse_sse_block(": keep-alive comment\n", 0).is_none());
    }

    #[test]
    fn multiline_data_fields_are_joined() {
        let ev = parse_sse_block("data: line1\ndata: line2\n", 0).unwrap();
        assert_eq!(ev.data, "line1\nline2");
    }

    #[test]
    fn delta_text_extracted_per_protocol() {
        let a =
            json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": "hi"}});
        assert_eq!(
            extract_delta_text(Protocol::Anthropic, &a).as_deref(),
            Some("hi")
        );
        // message_start carries no content and must not start the TTFT clock.
        let start = json!({"type": "message_start", "message": {"usage": {"input_tokens": 4}}});
        assert!(extract_delta_text(Protocol::Anthropic, &start).is_none());

        let o = json!({"choices": [{"delta": {"content": "yo"}}]});
        assert_eq!(
            extract_delta_text(Protocol::OpenAI, &o).as_deref(),
            Some("yo")
        );
        // A role-only opening frame must not count as first content either.
        let role = json!({"choices": [{"delta": {"role": "assistant"}}]});
        assert!(extract_delta_text(Protocol::OpenAI, &role).is_none());
    }

    #[test]
    fn stream_usage_read_from_both_shapes() {
        let start = json!({"type": "message_start", "message": {"usage": {"input_tokens": 7}}});
        let u = extract_stream_usage(Protocol::Anthropic, &start).unwrap();
        assert_eq!(u.input_tokens, 7);

        let oai = json!({"usage": {"prompt_tokens": 3, "completion_tokens": 11}});
        let u = extract_stream_usage(Protocol::OpenAI, &oai).unwrap();
        assert_eq!(u.output_tokens, 11);

        // OpenAI sends `"usage": null` on every non-final frame.
        assert!(extract_stream_usage(Protocol::OpenAI, &json!({"usage": null})).is_none());
    }

    #[test]
    fn raw_response_header_lookup_is_case_insensitive() {
        let r = RawResponse {
            status: 200,
            headers: [("request-id".to_string(), "req_1".to_string())].into(),
            body: String::new(),
            duration_ms: 0,
        };
        assert_eq!(r.header("Request-Id"), Some("req_1"));
        assert!(r.ok());
    }
}
