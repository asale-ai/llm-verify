// SPDX-License-Identifier: Apache-2.0
//! Wire-format abstraction over the two chat protocols that matter in the
//! resale market: OpenAI `/v1/chat/completions` and Anthropic `/v1/messages`.
//!
//! Everything else worth testing (Azure, Bedrock relays, OpenRouter, LiteLLM,
//! New-API, One-API, vLLM, Ollama …) speaks one of these two on the wire, so
//! two adapters cover the field.

use serde_json::{json, Value};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    OpenAI,
    Anthropic,
}

impl Protocol {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "openai" | "oai" | "chat" | "chat-completions" => Some(Self::OpenAI),
            "anthropic" | "claude" | "messages" => Some(Self::Anthropic),
            _ => None,
        }
    }

    pub fn chat_path(&self) -> &'static str {
        match self {
            Self::OpenAI => "/chat/completions",
            Self::Anthropic => "/messages",
        }
    }

    pub fn models_path(&self) -> &'static str {
        "/models"
    }

    /// Anthropic exposes an authoritative token counter; OpenAI does not.
    /// This is the difference between auditing billing and estimating it.
    pub fn count_tokens_path(&self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("/messages/count_tokens"),
            Self::OpenAI => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── request ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    /// `(role, content)` pairs; role is `user` or `assistant`.
    pub messages: Vec<(String, String)>,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub stop_sequences: Vec<String>,
    pub stream: bool,
    /// Raw tool definitions in the target protocol's own shape.
    pub tools: Option<Value>,
    /// Arbitrary extra top-level fields, merged last. Used by probes that need
    /// to poke at parameter handling.
    pub extra: Vec<(String, Value)>,
}

impl ChatRequest {
    pub fn new(model: &str, user: &str) -> Self {
        Self {
            model: model.to_string(),
            system: None,
            messages: vec![("user".into(), user.into())],
            max_tokens: 256,
            temperature: None,
            stop_sequences: Vec::new(),
            stream: false,
            tools: None,
            extra: Vec::new(),
        }
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = Some(t);
        self
    }

    pub fn system(mut self, s: &str) -> Self {
        self.system = Some(s.to_string());
        self
    }

    pub fn stop(mut self, seqs: &[&str]) -> Self {
        self.stop_sequences = seqs.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn stream(mut self, on: bool) -> Self {
        self.stream = on;
        self
    }

    pub fn model_id(mut self, m: &str) -> Self {
        self.model = m.to_string();
        self
    }

    /// The text this request sends, used for local token estimation.
    pub fn prompt_text(&self) -> String {
        let mut s = self.system.clone().unwrap_or_default();
        for (_, c) in &self.messages {
            s.push('\n');
            s.push_str(c);
        }
        s
    }

    pub fn to_body(&self, proto: Protocol) -> Value {
        let mut body = match proto {
            Protocol::OpenAI => {
                let mut msgs: Vec<Value> = Vec::with_capacity(self.messages.len() + 1);
                if let Some(sys) = &self.system {
                    msgs.push(json!({"role": "system", "content": sys}));
                }
                for (role, content) in &self.messages {
                    msgs.push(json!({"role": role, "content": content}));
                }
                let mut b = json!({
                    "model": self.model,
                    "messages": msgs,
                    "max_tokens": self.max_tokens,
                });
                if let Some(t) = self.temperature {
                    b["temperature"] = json!(t);
                }
                if !self.stop_sequences.is_empty() {
                    b["stop"] = json!(self.stop_sequences);
                }
                if self.stream {
                    b["stream"] = json!(true);
                    // Without this many OpenAI-compatible servers omit usage
                    // from the stream entirely, which would look like a
                    // missing-usage anomaly rather than a config difference.
                    b["stream_options"] = json!({"include_usage": true});
                }
                if let Some(tools) = &self.tools {
                    b["tools"] = tools.clone();
                }
                b
            }
            Protocol::Anthropic => {
                let msgs: Vec<Value> = self
                    .messages
                    .iter()
                    .map(|(role, content)| json!({"role": role, "content": content}))
                    .collect();
                let mut b = json!({
                    "model": self.model,
                    "messages": msgs,
                    "max_tokens": self.max_tokens,
                });
                if let Some(sys) = &self.system {
                    b["system"] = json!(sys);
                }
                if let Some(t) = self.temperature {
                    b["temperature"] = json!(t);
                }
                if !self.stop_sequences.is_empty() {
                    b["stop_sequences"] = json!(self.stop_sequences);
                }
                if self.stream {
                    b["stream"] = json!(true);
                }
                if let Some(tools) = &self.tools {
                    b["tools"] = tools.clone();
                }
                b
            }
        };
        for (k, v) in &self.extra {
            body[k] = v.clone();
        }
        body
    }
}

// ── response ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_create_tokens: u32,
    pub cache_read_tokens: u32,
    /// False when the payload carried no usage block at all, which is itself a
    /// finding — we must not silently report that as "0 tokens billed".
    pub present: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub id: String,
    pub model: String,
    pub role: String,
    pub object_type: String,
    pub text: String,
    pub stop_reason: String,
    pub usage: Usage,
    pub tool_calls: Vec<String>,
}

impl ChatResponse {
    pub fn parse(proto: Protocol, v: &Value) -> Self {
        match proto {
            Protocol::Anthropic => Self::parse_anthropic(v),
            Protocol::OpenAI => Self::parse_openai(v),
        }
    }

    fn parse_anthropic(v: &Value) -> Self {
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        if let Some(blocks) = v.get("content").and_then(|c| c.as_array()) {
            for b in blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                            text.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        if let Some(n) = b.get("name").and_then(|n| n.as_str()) {
                            tool_calls.push(n.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        let u = v.get("usage");
        Self {
            id: str_at(v, "id"),
            model: str_at(v, "model"),
            role: str_at(v, "role"),
            object_type: str_at(v, "type"),
            text,
            stop_reason: str_at(v, "stop_reason"),
            usage: Usage {
                input_tokens: u32_at(u, "input_tokens"),
                output_tokens: u32_at(u, "output_tokens"),
                cache_create_tokens: u32_at(u, "cache_creation_input_tokens"),
                cache_read_tokens: u32_at(u, "cache_read_input_tokens"),
                present: u.is_some(),
            },
            tool_calls,
        }
    }

    fn parse_openai(v: &Value) -> Self {
        let choice = v
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first());
        let msg = choice.and_then(|c| c.get("message"));
        let text = msg
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        let tool_calls = msg
            .and_then(|m| m.get("tool_calls"))
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        t.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let u = v.get("usage");
        // OpenAI nests the cache hit count one level deeper than Anthropic.
        let cached = u
            .and_then(|u| u.get("prompt_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as u32;
        Self {
            id: str_at(v, "id"),
            model: str_at(v, "model"),
            role: msg
                .map(|m| str_at(m, "role"))
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "assistant".into()),
            object_type: str_at(v, "object"),
            text,
            stop_reason: choice
                .map(|c| str_at(c, "finish_reason"))
                .unwrap_or_default(),
            usage: Usage {
                input_tokens: u32_at(u, "prompt_tokens"),
                output_tokens: u32_at(u, "completion_tokens"),
                cache_create_tokens: 0,
                cache_read_tokens: cached,
                present: u.is_some(),
            },
            tool_calls,
        }
    }

    /// Canonical stop reasons for the protocol, normalised across both.
    pub fn stop_reason_is_known(&self, proto: Protocol) -> bool {
        let r = self.stop_reason.trim();
        if r.is_empty() {
            return false;
        }
        match proto {
            Protocol::Anthropic => matches!(
                r,
                "end_turn" | "max_tokens" | "stop_sequence" | "tool_use" | "pause_turn" | "refusal"
            ),
            Protocol::OpenAI => matches!(
                r,
                "stop" | "length" | "tool_calls" | "function_call" | "content_filter"
            ),
        }
    }

    /// True when the response signals "output was cut at the token ceiling".
    pub fn stopped_at_limit(&self, proto: Protocol) -> bool {
        match proto {
            Protocol::Anthropic => self.stop_reason == "max_tokens",
            Protocol::OpenAI => self.stop_reason == "length",
        }
    }

    /// True when the response signals "a stop sequence terminated the output".
    pub fn stopped_at_sequence(&self, proto: Protocol) -> bool {
        match proto {
            Protocol::Anthropic => self.stop_reason == "stop_sequence",
            // OpenAI collapses stop-sequence and natural end into "stop".
            Protocol::OpenAI => self.stop_reason == "stop",
        }
    }

    pub fn id_prefix_ok(&self, proto: Protocol) -> bool {
        let id = self.id.to_ascii_lowercase();
        match proto {
            Protocol::Anthropic => id.starts_with("msg_"),
            // OpenAI-compatible servers vary a lot here; only require non-empty.
            Protocol::OpenAI => !id.is_empty(),
        }
    }
}

fn str_at(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn u32_at(v: Option<&Value>, k: &str) -> u32 {
    v.and_then(|v| v.get(k))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32
}

// ── error envelope ─────────────────────────────────────────────────────────

/// Whether an error body matches the protocol's documented envelope.
/// A hand-rolled proxy usually gets this subtly wrong.
pub fn error_envelope_ok(proto: Protocol, v: &Value) -> bool {
    match proto {
        Protocol::Anthropic => {
            v.get("type").and_then(|t| t.as_str()) == Some("error")
                && v.get("error")
                    .map(|e| !str_at(e, "type").is_empty() && !str_at(e, "message").is_empty())
                    .unwrap_or(false)
        }
        Protocol::OpenAI => v
            .get("error")
            .map(|e| !str_at(e, "message").is_empty())
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_parses_aliases_and_rejects_junk() {
        assert_eq!(Protocol::parse("Claude"), Some(Protocol::Anthropic));
        assert_eq!(Protocol::parse(" openai "), Some(Protocol::OpenAI));
        assert_eq!(Protocol::parse("gemini"), None);
    }

    #[test]
    fn only_anthropic_offers_authoritative_token_counting() {
        assert!(Protocol::Anthropic.count_tokens_path().is_some());
        assert!(Protocol::OpenAI.count_tokens_path().is_none());
    }

    #[test]
    fn openai_body_lifts_system_into_messages() {
        let b = ChatRequest::new("gpt-4o", "hi")
            .system("be terse")
            .to_body(Protocol::OpenAI);
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert!(b.get("system").is_none());
    }

    #[test]
    fn anthropic_body_keeps_system_top_level() {
        let b = ChatRequest::new("claude", "hi")
            .system("be terse")
            .stop(&["END"])
            .to_body(Protocol::Anthropic);
        assert_eq!(b["system"], "be terse");
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["stop_sequences"][0], "END");
    }

    #[test]
    fn openai_stream_requests_usage_explicitly() {
        let b = ChatRequest::new("m", "hi")
            .stream(true)
            .to_body(Protocol::OpenAI);
        assert_eq!(b["stream_options"]["include_usage"], true);
    }

    #[test]
    fn parses_anthropic_response_with_tool_use() {
        let v = json!({
            "id": "msg_01ABC", "type": "message", "role": "assistant",
            "model": "claude-x", "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "let me check"},
                {"type": "tool_use", "name": "get_weather", "input": {}}
            ],
            "usage": {"input_tokens": 12, "output_tokens": 5,
                      "cache_read_input_tokens": 3}
        });
        let r = ChatResponse::parse(Protocol::Anthropic, &v);
        assert_eq!(r.text, "let me check");
        assert_eq!(r.tool_calls, vec!["get_weather"]);
        assert_eq!(r.usage.input_tokens, 12);
        assert_eq!(r.usage.cache_read_tokens, 3);
        assert!(r.usage.present);
        assert!(r.id_prefix_ok(Protocol::Anthropic));
        assert!(r.stop_reason_is_known(Protocol::Anthropic));
    }

    #[test]
    fn parses_openai_response_and_nested_cache_tokens() {
        let v = json!({
            "id": "chatcmpl-9", "object": "chat.completion", "model": "gpt-4o",
            "choices": [{"finish_reason": "length",
                         "message": {"role": "assistant", "content": "hello"}}],
            "usage": {"prompt_tokens": 8, "completion_tokens": 2,
                      "prompt_tokens_details": {"cached_tokens": 4}}
        });
        let r = ChatResponse::parse(Protocol::OpenAI, &v);
        assert_eq!(r.text, "hello");
        assert_eq!(r.usage.cache_read_tokens, 4);
        assert!(r.stopped_at_limit(Protocol::OpenAI));
        assert!(!r.stopped_at_limit(Protocol::Anthropic));
    }

    #[test]
    fn missing_usage_block_is_distinguishable_from_zero() {
        let v = json!({"id": "x", "content": [], "type": "message"});
        let r = ChatResponse::parse(Protocol::Anthropic, &v);
        assert!(!r.usage.present);
        assert_eq!(r.usage.input_tokens, 0);
    }

    #[test]
    fn unknown_stop_reasons_are_rejected_per_protocol() {
        let r = ChatResponse {
            stop_reason: "length".into(),
            ..Default::default()
        };
        assert!(r.stop_reason_is_known(Protocol::OpenAI));
        assert!(!r.stop_reason_is_known(Protocol::Anthropic));
        assert!(!ChatResponse::default().stop_reason_is_known(Protocol::OpenAI));
    }

    #[test]
    fn error_envelope_requires_documented_shape() {
        let good =
            json!({"type": "error", "error": {"type": "invalid_request_error", "message": "bad"}});
        assert!(error_envelope_ok(Protocol::Anthropic, &good));
        let missing_type = json!({"type": "error", "error": {"message": "bad"}});
        assert!(!error_envelope_ok(Protocol::Anthropic, &missing_type));
        assert!(error_envelope_ok(
            Protocol::OpenAI,
            &json!({"error": {"message": "bad"}})
        ));
        assert!(!error_envelope_ok(
            Protocol::OpenAI,
            &json!({"detail": "bad"})
        ));
    }
}
