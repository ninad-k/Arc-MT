//! Anthropic Messages API provider (`POST {base}/v1/messages`).
//!
//! Notes on current API behaviour (checked against the Anthropic API reference):
//! - `temperature`/`top_p`/`top_k` are rejected (HTTP 400) by current models, so
//!   `ChatRequest::temperature` is intentionally NOT sent.
//! - Newer models think by default and return `thinking` blocks that must be replayed
//!   unchanged during a tool loop. The neutral [`Message`] type has no slot for them, so the
//!   provider keeps them in a small private cache keyed by tool-call id and re-attaches them
//!   when the assistant turn is sent back.
//! - Forced `tool_choice` is rejected by current models; it is never sent (default `auto`).
//! - The last tool carries `cache_control`, and so does the system prompt when the request says
//!   it is fixed (`cache_system`), so the fixed part of a request is read from the prompt cache
//!   (about a tenth of the input price).
//! - `refusal_fallback` sends `fallbacks: "default"` (beta `server-side-fallback-2026-07-01`):
//!   a declined request is re-run on another Claude model inside the same call.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::util::{self, non_empty, post_json, schema_or_empty};
use crate::{
    ChatRequest, ChatResponse, LlmError, LlmProvider, Message, ProviderConfig, StopReason,
    ToolCall, Usage,
};

/// Where requests go when the config sets no `base_url`.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const THINKING_CACHE_CAP: usize = 256;
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Anthropic Messages API client.
pub struct AnthropicProvider {
    name: String,
    model: String,
    url: String,
    max_tokens: u32,
    client: reqwest::Client,
    thinking: Option<Value>,
    effort: Option<String>,
    refusal_fallback: bool,
    /// (first tool-call id of an assistant turn, thinking blocks to replay before it).
    thinking_cache: Mutex<VecDeque<(String, Vec<Value>)>>,
}

impl AnthropicProvider {
    /// Build from config, reading the API key from `config.api_key_env`.
    pub fn from_config(config: &ProviderConfig) -> Result<Self, LlmError> {
        let key = util::read_api_key(config)?;
        Self::new(config, &key)
    }

    /// Build with an explicit API key (used by tests and `from_config`).
    pub fn new(config: &ProviderConfig, api_key: &str) -> Result<Self, LlmError> {
        let mut base = util::resolve_base_url(config, DEFAULT_BASE_URL)?;
        util::require_secure_transport(config, &base)?;
        if let Some(stripped) = base.strip_suffix("/v1") {
            base = stripped.to_string();
        }
        let mut headers = HeaderMap::new();
        let (k, v) = util::secret_header("x-api-key", api_key, &config.name)?;
        headers.insert(k, v);
        headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if config.refusal_fallback {
            headers.insert("anthropic-beta", HeaderValue::from_static(FALLBACK_BETA));
        }
        Ok(Self {
            name: config.name.clone(),
            model: config.model.clone(),
            url: format!("{base}/v1/messages"),
            max_tokens: config.max_tokens,
            client: util::build_client(config, headers)?,
            thinking: None,
            effort: None,
            refusal_fallback: config.refusal_fallback,
            thinking_cache: Mutex::new(VecDeque::new()),
        })
    }

    /// Send an explicit `thinking` config, e.g. `{"type":"adaptive"}` or, on Claude Sonnet 5.5,
    /// `{"type":"between_tools"}` for the lowest-latency setting. Default: not sent (model default).
    pub fn with_thinking(mut self, thinking: Value) -> Self {
        self.thinking = Some(thinking);
        self
    }

    /// Send `output_config.effort` (`low`..`max`) when the request names none. Default: not
    /// sent (model default).
    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    fn build_body(&self, req: &ChatRequest) -> Value {
        let mut body = json!({
            "model": self.model,
            "max_tokens": req.max_tokens.unwrap_or(self.max_tokens),
            "messages": self.build_messages(&req.messages),
        });
        // Render order is tools, then system, then messages: marking the system prompt caches
        // both, and marking the last tool also covers a request without a system prompt.
        if let Some(system) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
            body["system"] = if req.cache_system {
                json!([{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}])
            } else {
                json!(system)
            };
        }
        if !req.tools.is_empty() {
            let last = req.tools.len() - 1;
            body["tools"] = req
                .tools
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut tool = json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": schema_or_empty(&t.parameters),
                    });
                    if i == last {
                        tool["cache_control"] = json!({"type": "ephemeral"});
                    }
                    tool
                })
                .collect();
        }
        if let Some(t) = &self.thinking {
            body["thinking"] = t.clone();
        }
        if let Some(e) = req.effort.as_ref().or(self.effort.as_ref()) {
            body["output_config"] = json!({"effort": e});
        }
        if self.refusal_fallback {
            body["fallbacks"] = json!("default");
        }
        body
    }

    fn build_messages(&self, messages: &[Message]) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        // Pending user turn: tool_result blocks must precede text in one user message, and
        // consecutive results (parallel tool calls) share a single message.
        let mut results: Vec<Value> = Vec::new();
        let mut texts: Vec<Value> = Vec::new();
        let flush = |out: &mut Vec<Value>, results: &mut Vec<Value>, texts: &mut Vec<Value>| {
            if results.is_empty() && texts.is_empty() {
                return;
            }
            let mut content = std::mem::take(results);
            content.append(texts);
            out.push(json!({"role": "user", "content": content}));
        };
        for m in messages {
            match m {
                Message::User { text } => texts.push(json!({"type": "text", "text": text})),
                Message::UserImage {
                    text,
                    media_type,
                    data_base64,
                } => {
                    texts.push(json!({"type": "image", "source": {
                        "type": "base64", "media_type": media_type, "data": data_base64}}));
                    texts.push(json!({"type": "text", "text": text}));
                }
                Message::ToolResult {
                    call_id,
                    content,
                    is_error,
                    ..
                } => {
                    let mut block = json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content,
                    });
                    if *is_error {
                        block["is_error"] = json!(true);
                    }
                    results.push(block);
                }
                Message::Assistant { text, tool_calls } => {
                    flush(&mut out, &mut results, &mut texts);
                    let mut content: Vec<Value> = self.cached_thinking(tool_calls);
                    if let Some(t) = text.as_deref().filter(|t| !t.is_empty()) {
                        content.push(json!({"type": "text", "text": t}));
                    }
                    for c in tool_calls {
                        content.push(json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.name,
                            "input": c.arguments,
                        }));
                    }
                    if !content.is_empty() {
                        out.push(json!({"role": "assistant", "content": content}));
                    }
                }
            }
        }
        flush(&mut out, &mut results, &mut texts);
        out
    }

    fn cached_thinking(&self, calls: &[ToolCall]) -> Vec<Value> {
        if calls.is_empty() {
            return Vec::new();
        }
        let cache = self
            .thinking_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        calls
            .iter()
            .find_map(|c| {
                cache
                    .iter()
                    .find(|(id, _)| *id == c.id)
                    .map(|(_, b)| b.clone())
            })
            .unwrap_or_default()
    }

    fn remember_thinking(&self, first_call_id: &str, blocks: Vec<Value>) {
        if blocks.is_empty() {
            return;
        }
        let mut cache = self
            .thinking_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.retain(|(id, _)| id != first_call_id);
        cache.push_back((first_call_id.to_string(), blocks));
        while cache.len() > THINKING_CACHE_CAP {
            cache.pop_front();
        }
    }

    fn parse_response(&self, v: &Value) -> Result<ChatResponse, LlmError> {
        let content = v
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| LlmError::Decode("anthropic: response has no `content` array".into()))?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        let mut thinking_blocks = Vec::new();
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).ok_or_else(|| {
                        LlmError::Decode("anthropic: tool_use block without `id`".into())
                    })?;
                    let name = block.get("name").and_then(Value::as_str).ok_or_else(|| {
                        LlmError::Decode("anthropic: tool_use block without `name`".into())
                    })?;
                    let arguments = util::object_arguments(
                        block.get("input").cloned().unwrap_or(Value::Null),
                        name,
                    )?;
                    tool_calls.push(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments,
                    });
                }
                Some("thinking") | Some("redacted_thinking") => thinking_blocks.push(block.clone()),
                _ => {}
            }
        }
        if let Some(first) = tool_calls.first() {
            self.remember_thinking(&first.id, thinking_blocks);
        }
        let stop_reason = match v.get("stop_reason").and_then(Value::as_str) {
            Some("end_turn") | Some("stop_sequence") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some("refusal") => StopReason::Refusal,
            Some(other) => StopReason::Other(other.to_string()),
            None if !tool_calls.is_empty() => StopReason::ToolUse,
            None => StopReason::EndTurn,
        };
        let usage = v.get("usage").map(|u| {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            // Total prompt tokens, including prompt-cache reads/writes.
            let input =
                n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
            let clamp = |x: u64| x.min(u32::MAX as u64) as u32;
            Usage {
                input_tokens: clamp(input),
                output_tokens: clamp(n("output_tokens")),
                cache_read_tokens: clamp(n("cache_read_input_tokens")),
                cache_write_tokens: clamp(n("cache_creation_input_tokens")),
            }
        });
        Ok(ChatResponse {
            text: non_empty(text),
            tool_calls,
            stop_reason,
            usage,
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let body = self.build_body(request);
        tracing::debug!(
            provider = %self.name,
            model = %self.model,
            messages = request.messages.len(),
            tools = request.tools.len(),
            "anthropic chat request"
        );
        let v = post_json(&self.client, &self.name, &self.url, &body).await?;
        self.parse_response(&v)
    }
}
