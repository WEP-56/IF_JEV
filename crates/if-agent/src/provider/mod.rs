//! Provider trait 与三个 API 适配器。
//!
//! | 概念     | Messages（Anthropic）  | Responses（OpenAI）       | Chat Completions（OpenAI 兼容） |
//! |----------|------------------------|---------------------------|---------------------------------|
//! | 端点     | /v1/messages           | /v1/responses             | /v1/chat/completions            |
//! | 系统提示 | 顶层 system            | 顶层 instructions         | 首条 system 消息                |
//! | 工具调用 | tool_use 内容块        | function_call 输出项      | assistant.tool_calls            |
//! | 工具结果 | user 内 tool_result    | function_call_output      | role = tool 的消息              |
//! | 流结束   | message_stop           | response.completed        | finish_reason + [DONE]          |
//!
//! Chat Completions 是 IF 新增的：中转服务、本地模型、Ollama 与大多数"OpenAI 兼容"接口只支持它
//! （docs/05 §3.1）。

pub mod anthropic;
pub mod chat_completions;
pub mod responses;
pub mod scripted;
pub mod sse;

use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::context::PromptContext;
use crate::message::{ChatMessage, StopReason, Usage};
use crate::tools::ToolSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKind {
    AnthropicMessages,
    OpenAiResponses,
    ChatCompletions,
}

/// 一个模型槽位的配置（docs/11 §6：结构模型与叙事模型至少两个槽位）。
/// 密钥由 `if-app` 从系统钥匙串取出后填入，不序列化。
#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub name: String,
    pub api: ApiKind,
    pub base_url: String,
    #[serde(skip)]
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// 如 `low` / `medium` / `high`；`None` 表示不发送。
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 发送 prompt cache 提示：Anthropic 为 system 末尾的 `cache_control`，
    /// OpenAI 为 `prompt_cache_key`。不少兼容接口会拒绝未知字段，所以可以关掉。
    #[serde(default = "default_true")]
    pub prompt_cache: bool,
    /// Responses 专用：`store: false` 并回传加密 reasoning。只有 OpenAI 官方支持。
    #[serde(default)]
    pub replay_encrypted_reasoning: bool,
}

fn default_true() -> bool {
    true
}

impl std::fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("name", &self.name)
            .field("api", &self.api)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("reasoning_effort", &self.reasoning_effort)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallBegun { name: String },
}

#[derive(Debug, Clone)]
pub struct TurnOutput {
    pub message: ChatMessage,
    pub usage: Usage,
    pub stop: StopReason,
    pub prompt_fingerprint: Option<String>,
}

/// `retryable` 标记网络、限流、服务端故障；agent loop 只在尚未收到任何流事件时重试。
#[derive(Debug, Clone)]
pub struct ProviderError {
    pub message: String,
    pub retryable: bool,
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    pub fn fatal(message: impl Into<String>) -> Self {
        ProviderError { message: message.into(), retryable: false, retry_after: None }
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        ProviderError { message: message.into(), retryable: true, retry_after: None }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub trait Provider: Send + Sync {
    fn label(&self) -> String;
    fn model(&self) -> &str;

    /// 每次调用必然终止于 Done、Error 或 Aborted。
    fn stream_turn(
        &self,
        prompt: &PromptContext,
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(ProviderEvent),
        cancel: &AtomicBool,
    ) -> StreamTerminal;
}

#[derive(Debug)]
pub enum StreamTerminal {
    Done(TurnOutput),
    Error(FailedTurn),
    Aborted(FailedTurn),
}

#[derive(Debug)]
pub struct FailedTurn {
    pub message: ChatMessage,
    pub error: ProviderError,
}

impl FailedTurn {
    pub fn from_error(error: ProviderError) -> Self {
        FailedTurn { message: ChatMessage::empty_assistant(), error }
    }

    pub fn aborted() -> Self {
        FailedTurn::from_error(ProviderError::fatal("模型调用已取消"))
    }
}

pub fn build_provider(settings: ProviderSettings) -> Box<dyn Provider> {
    match settings.api {
        ApiKind::AnthropicMessages => Box::new(anthropic::AnthropicProvider::new(settings)),
        ApiKind::OpenAiResponses => Box::new(responses::ResponsesProvider::new(settings)),
        ApiKind::ChatCompletions => Box::new(chat_completions::ChatCompletionsProvider::new(settings)),
    }
}

pub(crate) fn label(settings: &ProviderSettings) -> String {
    match &settings.reasoning_effort {
        Some(effort) => format!("{} / {} / effort={effort}", settings.name, settings.model),
        None => format!("{} / {}", settings.name, settings.model),
    }
}

pub(crate) fn run_stream<F>(f: F) -> StreamTerminal
where
    F: FnOnce() -> Result<Option<TurnOutput>, ProviderError>,
{
    match f() {
        Ok(Some(output)) => StreamTerminal::Done(output),
        Ok(None) => StreamTerminal::Aborted(FailedTurn::aborted()),
        Err(error) => StreamTerminal::Error(FailedTurn::from_error(error)),
    }
}

// ---------------------------------------------------------------- prompt 指纹与缓存键

pub(crate) fn sorted_tools(tools: &[ToolSpec]) -> Vec<&ToolSpec> {
    let mut tools: Vec<&ToolSpec> = tools.iter().collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

fn canonical_tools(tools: &[ToolSpec]) -> Vec<Value> {
    sorted_tools(tools)
        .into_iter()
        .map(|t| serde_json::json!({ "name": t.name, "description": t.description, "schema": t.schema }))
        .collect()
}

fn sha256_hex(value: &Value) -> String {
    Sha256::digest(value.to_string().as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// 整个请求的语义指纹，写进任务记录，用于审计与回归。
pub(crate) fn prompt_fingerprint(settings: &ProviderSettings, prompt: &PromptContext, tools: &[ToolSpec]) -> String {
    let semantic = serde_json::json!({
        "version": 1,
        "api": settings.api,
        "model": settings.model,
        "reasoning_effort": settings.reasoning_effort,
        "temperature": settings.temperature,
        "system": prompt.system_text(),
        "tools": canonical_tools(tools),
        "messages": prompt.messages,
    });
    format!("sha256:{}", sha256_hex(&semantic))
}

/// 只由稳定前缀（system + 工具）决定，不含对话。OpenAI 限 64 字符。
pub(crate) fn prompt_cache_key(settings: &ProviderSettings, prompt: &PromptContext, tools: &[ToolSpec]) -> String {
    let prefix = serde_json::json!({
        "version": 1,
        "api": settings.api,
        "model": settings.model,
        "system": prompt.system_text(),
        "tools": canonical_tools(tools),
    });
    const KEY_PREFIX: &str = "if:v1:";
    let hash = sha256_hex(&prefix);
    format!("{KEY_PREFIX}{}", &hash[..64 - KEY_PREFIX.len()])
}

// ---------------------------------------------------------------- HTTP

/// 带超时与代理的 HTTP agent。代理读 HTTPS_PROXY / HTTP_PROXY / ALL_PROXY。
pub(crate) fn http_agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        // SSE 是长连接；推理模型思考期间可能长时间无数据。
        .timeout_read(Duration::from_secs(300))
        .timeout_write(Duration::from_secs(60));
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                if let Ok(proxy) = ureq::Proxy::new(v.trim()) {
                    builder = builder.proxy(proxy);
                }
                break;
            }
        }
    }
    builder.build()
}

pub(crate) fn post_sse(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, String)],
    body: &Value,
) -> Result<Box<dyn Read + Send + Sync + 'static>, ProviderError> {
    let mut req = agent.post(url).set("content-type", "application/json");
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.send_string(&body.to_string()) {
        Ok(resp) => Ok(resp.into_reader()),
        Err(ureq::Error::Status(code, resp)) => {
            let retry_after = retry_after_hint(code, resp.header("retry-after-ms"), resp.header("retry-after"));
            let text = resp.into_string().unwrap_or_default();
            Err(ProviderError {
                message: format!("HTTP {code}: {}", extract_api_error(&text)),
                retryable: matches!(code, 408 | 429) || code >= 500,
                retry_after,
            })
        }
        Err(ureq::Error::Transport(t)) => Err(ProviderError::retryable(format!("网络错误: {t}"))),
    }
}

/// 只在 Retry-After 语义成立的状态码上采信；网关 502 页上的 Retry-After 是 CDN 模板噪声。
fn retry_after_hint(status: u16, ms: Option<&str>, secs: Option<&str>) -> Option<Duration> {
    if !matches!(status, 408 | 429 | 503) {
        return None;
    }
    ms.and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .or_else(|| secs.and_then(|v| v.parse().ok()).map(Duration::from_secs))
}

/// 从错误响应体里挖出人话；HTML 错误页只保留 `<title>`。
fn extract_api_error(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        for pointer in ["/error/message", "/message", "/error"] {
            if let Some(s) = v.pointer(pointer).and_then(Value::as_str) {
                return s.to_owned();
            }
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "（空响应体）".into();
    }
    let head = trimmed.chars().take(256).collect::<String>().to_lowercase();
    if head.contains("<html") || head.starts_with("<!doctype html") {
        let lower = trimmed.to_lowercase();
        let title = lower.find("<title").and_then(|open| {
            let start = trimmed[open..].find('>')? + open + 1;
            let end = lower[start..].find("</title>")? + start;
            Some(collapse(&trimmed[start..end]))
        });
        return match title {
            Some(t) if !t.is_empty() => format!("{t}（HTML 错误页，正文已省略）"),
            _ => "（HTML 错误页，已省略）".into(),
        };
    }
    collapse(trimmed).chars().take(400).collect()
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 兼容 base_url 带不带 `/v1`。`versioned_path` 形如 `v1/messages`。
pub(crate) fn url_join(base: &str, versioned_path: &str) -> String {
    let b = base.trim_end_matches('/');
    if b.ends_with("/v1") {
        format!("{b}/{}", versioned_path.trim_start_matches("v1/"))
    } else {
        format!("{b}/{versioned_path}")
    }
}

pub(crate) fn args_to_string(input: &Value) -> String {
    match input {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 空文本按 `{}`；解析失败保留原文为 `Value::String`，让工具层报错、模型自愈。
pub(crate) fn parse_args(raw: &str) -> Value {
    let t = raw.trim();
    if t.is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(t).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

pub(crate) fn args_to_object(input: &Value) -> Value {
    match input {
        Value::Object(_) => input.clone(),
        other => serde_json::json!({ "_raw": args_to_string(other) }),
    }
}

#[cfg(test)]
pub(crate) fn test_settings(api: ApiKind) -> ProviderSettings {
    ProviderSettings {
        name: "test".into(),
        api,
        base_url: "https://example.invalid".into(),
        api_key: "k".into(),
        model: "m".into(),
        max_tokens: None,
        temperature: None,
        reasoning_effort: None,
        prompt_cache: true,
        replay_encrypted_reasoning: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolSpec {
        ToolSpec { name: name.into(), description: "d".into(), schema: serde_json::json!({"type":"object"}), parallel_safe: true }
    }

    #[test]
    fn url_join_handles_v1() {
        assert_eq!(url_join("https://api.anthropic.com", "v1/messages"), "https://api.anthropic.com/v1/messages");
        assert_eq!(url_join("https://api.openai.com/v1/", "v1/responses"), "https://api.openai.com/v1/responses");
    }

    #[test]
    fn parse_args_variants() {
        assert_eq!(parse_args(""), serde_json::json!({}));
        assert_eq!(parse_args("{\"a\":1}"), serde_json::json!({"a":1}));
        assert!(matches!(parse_args("{broken"), Value::String(_)));
    }

    #[test]
    fn cache_key_depends_only_on_stable_prefix() {
        let s = test_settings(ApiKind::OpenAiResponses);
        let mut first = PromptContext::default();
        first.system_sections.push("stable".into());
        first.messages.push(ChatMessage::user_text("a"));
        let mut second = first.clone();
        second.messages.push(ChatMessage::user_text("b"));
        let tools = vec![tool("z"), tool("a")];
        let reversed = vec![tool("a"), tool("z")];
        let key = prompt_cache_key(&s, &first, &tools);
        assert_eq!(key.chars().count(), 64);
        assert_eq!(key, prompt_cache_key(&s, &second, &reversed));
        assert_ne!(prompt_fingerprint(&s, &first, &tools), prompt_fingerprint(&s, &second, &tools));
        assert_eq!(prompt_fingerprint(&s, &first, &tools), prompt_fingerprint(&s, &first, &reversed));
    }

    #[test]
    fn retry_after_only_on_rate_limit_statuses() {
        assert_eq!(retry_after_hint(429, Some("2500"), None), Some(Duration::from_millis(2500)));
        assert_eq!(retry_after_hint(503, None, Some("7")), Some(Duration::from_secs(7)));
        assert_eq!(retry_after_hint(502, Some("60000"), Some("60")), None);
    }

    #[test]
    fn html_error_pages_reduce_to_title() {
        let page = "<!DOCTYPE html><html><head><title>x | 502: Bad gateway</title></head><body>long</body></html>";
        assert_eq!(extract_api_error(page), "x | 502: Bad gateway（HTML 错误页，正文已省略）");
        assert_eq!(extract_api_error(r#"{"error":{"message":"quota"}}"#), "quota");
        assert_eq!(extract_api_error("a\n  b"), "a b");
    }

    #[test]
    fn settings_debug_redacts_key_and_serde_skips_it() {
        let mut s = test_settings(ApiKind::ChatCompletions);
        s.api_key = "sk-secret".into();
        assert!(!format!("{s:?}").contains("sk-secret"));
        assert!(!serde_json::to_string(&s).unwrap().contains("sk-secret"));
    }
}
