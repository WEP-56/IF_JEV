//! Anthropic Messages API 适配器，从 Onemore 移植。
//!
//! - `max_tokens` 必填，没配就用默认值；
//! - 消息必须 user / assistant 严格交替且 content 非空，所以合并相邻同角色消息、跳过空消息；
//! - 流式的工具参数以 `input_json_delta.partial_json` 碎片下发，拼完才能解析；
//! - IF 新增：`prompt_cache` 开启时，system 以内容块数组发送并在末尾加 `cache_control`，
//!   让视图编译出的稳定前缀命中缓存（docs/08）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use super::{
    args_to_object, http_agent, parse_args, post_sse, sse::SseReader, Provider, ProviderError,
    ProviderEvent, ProviderSettings, StreamTerminal, TurnOutput,
};
use crate::context::PromptContext;
use crate::message::{Block, CacheUsage, ChatMessage, Role, StopReason, Usage};
use crate::tools::ToolSpec;

const DEFAULT_MAX_TOKENS: u64 = 8192;
const API_VERSION: &str = "2023-06-01";
const KIND: &str = "anthropic";

pub struct AnthropicProvider {
    settings: ProviderSettings,
    agent: ureq::Agent,
}

impl AnthropicProvider {
    pub fn new(settings: ProviderSettings) -> Self {
        AnthropicProvider { settings, agent: http_agent() }
    }

    pub(crate) fn build_body(&self, prompt: &PromptContext, tools: &[ToolSpec]) -> Value {
        let thinking = self.settings.reasoning_effort.is_some();
        let mut messages: Vec<Value> = Vec::new();
        for m in &prompt.messages {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let mut content: Vec<Value> = Vec::new();
            for b in &m.blocks {
                match b {
                    Block::Text(t) if !t.is_empty() => content.push(json!({"type": "text", "text": t})),
                    Block::Text(_) => {}
                    Block::Thinking { text, provider_kind, raw: Some(raw) }
                        if thinking
                            && provider_kind.as_deref() == Some(KIND)
                            && raw.get("signature").and_then(Value::as_str).is_some() =>
                    {
                        content.push(json!({ "type": "thinking", "thinking": text, "signature": raw["signature"] }));
                    }
                    Block::Thinking { .. } => {}
                    Block::ToolUse { id, name, input } => {
                        content.push(json!({ "type": "tool_use", "id": id, "name": name, "input": args_to_object(input) }));
                    }
                    Block::ToolResult { tool_use_id, content: c, is_error } => {
                        let mut o = json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": c });
                        if *is_error {
                            o["is_error"] = json!(true);
                        }
                        content.push(o);
                    }
                }
            }
            if content.is_empty() {
                continue;
            }
            match messages.last_mut() {
                Some(last) if last["role"] == role => {
                    if let Some(arr) = last["content"].as_array_mut() {
                        arr.extend(content);
                    }
                }
                _ => messages.push(json!({"role": role, "content": content})),
            }
        }

        let mut body = json!({
            "model": self.settings.model,
            "max_tokens": self.settings.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "messages": messages,
            "stream": true,
        });
        if let Some(effort) = &self.settings.reasoning_effort {
            body["thinking"] = json!({"type": "adaptive"});
            body["output_config"] = json!({"effort": effort});
        } else if let Some(t) = self.settings.temperature {
            // 开启 thinking 时 API 不接受自定义温度。
            body["temperature"] = json!(t);
        }
        let system = prompt.system_text();
        if !system.is_empty() {
            body["system"] = if self.settings.prompt_cache {
                json!([{ "type": "text", "text": system, "cache_control": {"type": "ephemeral"} }])
            } else {
                json!(system)
            };
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(
                super::sorted_tools(tools)
                    .into_iter()
                    .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema }))
                    .collect(),
            );
        }
        body
    }
}

enum Partial {
    Text(String),
    Thinking { text: String, signature: String },
    ToolUse { id: String, name: String, args: String },
}

impl Partial {
    fn finish(self) -> Block {
        match self {
            Partial::Text(t) => Block::Text(t),
            Partial::Thinking { text, signature } => Block::Thinking {
                text,
                provider_kind: Some(KIND.to_owned()),
                raw: (!signature.is_empty()).then(|| json!({"signature": signature})),
            },
            Partial::ToolUse { id, name, args } => Block::ToolUse { id, name, input: parse_args(&args) },
        }
    }
}

fn cache_usage(usage: &Value) -> Option<CacheUsage> {
    let read = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
    let write = usage.get("cache_creation_input_tokens").and_then(Value::as_u64);
    (read.is_some() || write.is_some())
        .then(|| CacheUsage { read_tokens: read.unwrap_or(0), write_tokens: write.unwrap_or(0) })
}

impl Provider for AnthropicProvider {
    fn label(&self) -> String {
        super::label(&self.settings)
    }

    fn model(&self) -> &str {
        &self.settings.model
    }

    fn stream_turn(
        &self,
        prompt: &PromptContext,
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(ProviderEvent),
        cancel: &AtomicBool,
    ) -> StreamTerminal {
        super::run_stream(|| {
            let url = super::url_join(&self.settings.base_url, "v1/messages");
            let headers = vec![("x-api-key", self.settings.api_key.clone()), ("anthropic-version", API_VERSION.to_owned())];
            let body = self.build_body(prompt, tools);
            let fingerprint = super::prompt_fingerprint(&self.settings, prompt, tools);
            let reader = post_sse(&self.agent, &url, &headers, &body)?;
            parse_stream(SseReader::new(reader), on_event, cancel, fingerprint)
        })
    }
}

pub(crate) fn parse_stream<R: std::io::Read>(
    mut sse: SseReader<R>,
    on_event: &mut dyn FnMut(ProviderEvent),
    cancel: &AtomicBool,
    fingerprint: String,
) -> Result<Option<TurnOutput>, ProviderError> {
    let mut partials: BTreeMap<usize, Partial> = BTreeMap::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut usage = Usage::default();
    let mut stop = StopReason::EndTurn;
    let mut saw_terminal = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let Some(ev) = sse.next_event().map_err(|e| ProviderError::retryable(format!("读取流失败: {e}")))? else {
            break;
        };
        if ev.data == "[DONE]" {
            return Err(ProviderError::fatal("流在终止事件前收到 [DONE]"));
        }
        let data: Value =
            serde_json::from_str(&ev.data).map_err(|e| ProviderError::fatal(format!("流事件 JSON 无效: {e}")))?;
        let index = data["index"].as_u64().unwrap_or(0) as usize;
        match data["type"].as_str().unwrap_or("") {
            "message_start" => {
                let u = &data["message"]["usage"];
                usage.input_tokens = u["input_tokens"].as_u64().unwrap_or(0);
                usage.output_tokens = u["output_tokens"].as_u64().unwrap_or(0);
                usage.cache = cache_usage(u);
                // Anthropic 分开报未缓存、缓存读、缓存写三桶；归一成完整输入量。
                if let Some(cache) = usage.cache {
                    usage.input_tokens += cache.read_tokens + cache.write_tokens;
                }
            }
            "content_block_start" => {
                let cb = &data["content_block"];
                match cb["type"].as_str().unwrap_or("") {
                    "text" => {
                        partials.insert(index, Partial::Text(String::new()));
                    }
                    "thinking" => {
                        partials.insert(
                            index,
                            Partial::Thinking {
                                text: cb["thinking"].as_str().unwrap_or("").to_owned(),
                                signature: cb["signature"].as_str().unwrap_or("").to_owned(),
                            },
                        );
                    }
                    "tool_use" => {
                        let name = cb["name"].as_str().unwrap_or("").to_owned();
                        on_event(ProviderEvent::ToolCallBegun { name: name.clone() });
                        partials.insert(
                            index,
                            Partial::ToolUse { id: cb["id"].as_str().unwrap_or("").to_owned(), name, args: String::new() },
                        );
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let delta = &data["delta"];
                match (delta["type"].as_str().unwrap_or(""), partials.get_mut(&index)) {
                    ("text_delta", p) => {
                        let piece = delta["text"].as_str().unwrap_or("");
                        if let Some(Partial::Text(buf)) = p {
                            buf.push_str(piece);
                        }
                        on_event(ProviderEvent::TextDelta(piece.to_owned()));
                    }
                    ("thinking_delta", p) => {
                        let piece = delta["thinking"].as_str().unwrap_or("");
                        if let Some(Partial::Thinking { text, .. }) = p {
                            text.push_str(piece);
                        }
                        on_event(ProviderEvent::ThinkingDelta(piece.to_owned()));
                    }
                    ("signature_delta", Some(Partial::Thinking { signature, .. })) => {
                        signature.push_str(delta["signature"].as_str().unwrap_or(""));
                    }
                    ("input_json_delta", Some(Partial::ToolUse { args, .. })) => {
                        args.push_str(delta["partial_json"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(p) = partials.remove(&index) {
                    blocks.push(p.finish());
                }
            }
            "message_delta" => {
                if let Some(r) = data["delta"]["stop_reason"].as_str() {
                    stop = match r {
                        "end_turn" | "stop_sequence" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        other => StopReason::Other(other.to_owned()),
                    };
                }
                if let Some(n) = data["usage"]["output_tokens"].as_u64() {
                    usage.output_tokens = n;
                }
            }
            "message_stop" => {
                saw_terminal = true;
                break;
            }
            "error" => {
                return Err(ProviderError::fatal(format!(
                    "API 流错误: {}",
                    data["error"]["message"].as_str().unwrap_or("未知")
                )));
            }
            _ => {}
        }
    }
    if !saw_terminal {
        return Err(ProviderError::retryable("流在终止事件前结束"));
    }
    blocks.extend(partials.into_values().map(Partial::finish));
    Ok(Some(TurnOutput {
        message: ChatMessage { role: Role::Assistant, blocks },
        usage,
        stop,
        prompt_fingerprint: Some(fingerprint),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{test_settings, ApiKind};

    fn provider() -> AnthropicProvider {
        AnthropicProvider::new(test_settings(ApiKind::AnthropicMessages))
    }

    fn tool_turn_prompt() -> PromptContext {
        let mut prompt = PromptContext::default();
        prompt.system_sections.push("sys".into());
        prompt.messages.push(ChatMessage::user_text("你好"));
        prompt.messages.push(ChatMessage {
            role: Role::Assistant,
            blocks: vec![
                Block::Text("我看看".into()),
                Block::ToolUse { id: "toolu_1".into(), name: "lookup_subject".into(), input: json!({"name":"顾言"}) },
            ],
        });
        prompt.messages.push(ChatMessage {
            role: Role::User,
            blocks: vec![Block::ToolResult { tool_use_id: "toolu_1".into(), content: "内容".into(), is_error: false }],
        });
        prompt.messages.push(ChatMessage::user_text("继续"));
        prompt
    }

    #[test]
    fn request_body_shape_and_merging() {
        let tools = vec![ToolSpec {
            name: "lookup_subject".into(),
            description: "查".into(),
            schema: json!({"type":"object"}),
            parallel_safe: true,
        }];
        let body = provider().build_body(&tool_turn_prompt(), &tools);
        assert_eq!(body["system"][0]["text"], "sys");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["messages"][1]["content"][1]["type"], "tool_use");
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["content"].as_array().unwrap().len(), 2, "工具结果与新输入合并成一条 user");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn temperature_and_effort_are_exclusive() {
        let mut s = test_settings(ApiKind::AnthropicMessages);
        s.temperature = Some(0.3);
        s.prompt_cache = false;
        let body = AnthropicProvider::new(s.clone()).build_body(&tool_turn_prompt(), &[]);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["system"], "sys");
        s.reasoning_effort = Some("high".into());
        let body = AnthropicProvider::new(s).build_body(&tool_turn_prompt(), &[]);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["output_config"]["effort"], "high");
    }

    #[test]
    fn parses_text_and_tool_stream() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":90}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"好\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"f\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"1}\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let mut events = Vec::new();
        let out = parse_stream(SseReader::new(stream.as_bytes()), &mut |e| events.push(e), &AtomicBool::new(false), "fp".into())
            .unwrap()
            .unwrap();
        assert_eq!(out.stop, StopReason::ToolUse);
        assert_eq!(out.usage.input_tokens, 100);
        assert_eq!(out.usage.output_tokens, 7);
        assert_eq!(out.message.text(), "好");
        assert_eq!(out.message.tool_uses()[0].2, &json!({"a":1}));
        assert!(events.contains(&ProviderEvent::ToolCallBegun { name: "f".into() }));
    }

    #[test]
    fn truncated_stream_is_retryable() {
        let stream = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\n";
        let err = parse_stream(SseReader::new(stream.as_bytes()), &mut |_| {}, &AtomicBool::new(false), "fp".into()).unwrap_err();
        assert!(err.retryable);
    }
}
