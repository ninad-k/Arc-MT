use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which API a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Ollama,
}

/// One `[[llm.providers]]` entry from `config/jarvis.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Name used to select this provider, e.g. `"local"`, `"claude"`.
    pub name: String,
    pub kind: ProviderKind,
    pub model: String,
    /// Override the API base URL. Defaults: `https://api.anthropic.com`,
    /// `https://api.openai.com/v1`, `http://127.0.0.1:11434`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Name of the env var holding the API key, e.g. `"ANTHROPIC_API_KEY"`.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Ollama only: how long the model stays loaded after a request, as an Ollama duration
    /// (`"30m"`, `"2h"`, `"-1"` for always). Default `"30m"`, so the next question does not
    /// pay for loading the model again. Ignored by the other providers.
    #[serde(default)]
    pub keep_alive: Option<String>,
    /// Anthropic only: when Claude declines a request, re-run it on another Claude model
    /// (`fallbacks: "default"`, billed at that model's rate). Default off.
    #[serde(default)]
    pub refusal_fallback: bool,
}

fn default_timeout_secs() -> u64 {
    60
}
fn default_max_tokens() -> u32 {
    1024
}

/// A tool the model may call. `parameters` is a JSON Schema object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A tool call requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider call id (Ollama has none: the provider generates one).
    pub id: String,
    pub name: String,
    /// Parsed JSON arguments (always an object; `{}` when the model sent none).
    pub arguments: Value,
}

/// Conversation history in a provider-neutral shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        text: String,
    },
    /// A user message with one image (Anthropic sends both; other providers send the text only,
    /// with a note that an image could not be shown).
    UserImage {
        text: String,
        /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
        media_type: String,
        data_base64: String,
    },
    Assistant {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    /// Result of executing a tool call. `content` is usually JSON text.
    ToolResult {
        call_id: String,
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    /// Overrides `ProviderConfig::max_tokens` when set.
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// How hard the model should think (`low`..`max`). Anthropic only; others ignore it.
    #[serde(default)]
    pub effort: Option<String>,
    /// Mark the system prompt for prompt caching (Anthropic only). Set it only when the system
    /// prompt is the same on every request: a prompt that changes (a clock) would pay the cache
    /// write price every time and never be read back.
    #[serde(default)]
    pub cache_system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// The model declined the request (Anthropic safety classifiers).
    Refusal,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    /// All prompt tokens, including those read from or written to the prompt cache.
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Of `input_tokens`, read from the prompt cache (billed at the cache-read price).
    #[serde(default)]
    pub cache_read_tokens: u32,
    /// Of `input_tokens`, written to the prompt cache (billed at the cache-write price).
    #[serde(default)]
    pub cache_write_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Assistant text, if any (may be present alongside tool calls).
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    pub usage: Option<Usage>,
}
