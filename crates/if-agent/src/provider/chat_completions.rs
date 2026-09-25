//! OpenAI Chat Completions 适配器（`/v1/chat/completions`，stream 模式）。IF 新增。
//!
//! 中转服务、本地模型、Ollama 和大多数"OpenAI 兼容"接口只支持这个格式（docs/05 §3.1）。
//! 兼容接口的实现参差不齐，所以解析尽量宽松：
//!
//! - 工具调用按 `tool_calls[].index` 拼接；`id` 与 `name` 只在第一个分片出现；
//! - `reasoning_content`（DeepSeek 等）与 `reasoning`（部分中转）都当作思考内容，
//!   并在后续请求里以 `reasoning_content` 回传（DeepSeek 的思考模式在工具调用轮次要求回传）；
//! - 用量来自 `stream_options.include_usage` 的末尾分片；不支持的服务就记 0；
//! - 流结束的判据：见过 `finish_reason`，且随后收到 `[DONE]` 或连接正常关闭。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use super::{
    args_to_string, http_agent, parse_args, post_sse, sse::SseReader, Provider, ProviderError,
    ProviderEvent, ProviderSettings, StreamTerminal, TurnOutput,
};
use crate::context::PromptContext;
use crate::message::{Block, CacheUsage, ChatMessage, Role, StopReason, Usage};
use crate::tools::ToolSpec;

const KIND: &str = "chat_completions";

pub struct ChatCompletionsProvider {
    settings: ProviderSettings,
    agent: ureq::Agent,
}

impl ChatCompletionsProvider {
    pub fn new(settings: ProviderSettings) -> Self {
        ChatCompletionsProvider { settings, agent: http_agent() }
    }

    pub(crate) fn build_body(&self, prompt: &PromptContext, tools: &[ToolSpec]) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        let system = prompt.system_text();
        if !system.is_empty() {
            messages.push(json!({"role": "system", "content": system}));
        }
        for m in &prompt.messages {
            match m.role {
                Role::Assistant => {
                    let text = m.text();
                    let reasoning: String = m
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            Block::Thinking { text, provider_kind, .. } if provider_kind.as_deref() == Some(KIND) => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect();
                    let calls: Vec<Value> = m
                        .tool_uses()
                        .into_iter()
                        .map(|(id, name, input)| {
                            json!({ "id": id, "type": "function", "function": { "name": name, "arguments": args_to_string(input) } })
                        })
                        .collect();
                    if text.is_empty() && calls.is_empty() {
                        continue;
                    }
                    let mut msg = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
                    if !calls.is_empty() {
                        msg["tool_calls"] = Value::Array(calls);
                    }
                    if !reasoning.is_empty() {
                        msg["reasoning_content"] = json!(reasoning);
                    }
                    messages.push(msg);
                }
                Role::User => {
                    // role=tool 的消息必须紧跟在带 tool_calls 的 assistant 之后，所以先发工具结果。
                    for b in &m.blocks {
                        if let Block::ToolResult { tool_use_id, content, is_error } = b {
                            let content = if *is_error { format!("ERROR: {content}") } else { content.clone() };
                            messages.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": content }));
                        }
                    }
                    let text = m.text();
                    if !text.is_empty() {
                        messages.push(json!({"role": "user", "content": text}));
                    }
                }
            }
        }

        let mut body = json!({
            "model": self.settings.model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if let Some(n) = self.settings.max_tokens {
            body["max_tokens"] = json!(n);
        }
        if let Some(t) = self.settings.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(effort) = &self.settings.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(
                super::sorted_tools(tools)
                    .into_iter()
                    .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema } }))
                    .collect(),
            );
        }
        if self.settings.prompt_cache {
            body["prompt_cache_key"] = json!(super::prompt_cache_key(&self.settings, prompt, tools));
        }
        body
    }
}

impl Provider for ChatCompletionsProvider {
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
            let url = super::url_join(&self.settings.base_url, "v1/chat/completions");
            let mut headers = Vec::new();
            if !self.settings.api_key.is_empty() {
                headers.push(("authorization", format!("Bearer {}", self.settings.api_key)));
            }
            let body = self.build_body(prompt, tools);
            let fingerprint = super::prompt_fingerprint(&self.settings, prompt, tools);
            let reader = post_sse(&self.agent, &url, &headers, &body)?;
            parse_stream(SseReader::new(reader), on_event, cancel, fingerprint)
        })
    }
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

pub(crate) fn parse_stream<R: std::io::Read>(
    mut sse: SseReader<R>,
    on_event: &mut dyn FnMut(ProviderEvent),
    cancel: &AtomicBool,
    fingerprint: String,
) -> Result<Option<TurnOutput>, ProviderError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls: BTreeMap<u64, PartialCall> = BTreeMap::new();
    let mut usage = Usage::default();
    let mut finish: Option<String> = None;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let Some(ev) = sse.next_event().map_err(|e| ProviderError::retryable(format!("读取流失败: {e}")))? else {
            break;
        };
        if ev.data.trim() == "[DONE]" {
            break;
        }
        if ev.data.trim().is_empty() {
            continue;
        }
        let data: Value =
            serde_json::from_str(&ev.data).map_err(|e| ProviderError::fatal(format!("流事件 JSON 无效: {e}")))?;
        if let Some(err) = data.get("error").filter(|e| !e.is_null()) {
            let msg = err["message"].as_str().map(str::to_owned).unwrap_or_else(|| err.to_string());
            return Err(ProviderError::fatal(format!("API 流错误: {msg}")));
        }
        if let Some(u) = data.get("usage").filter(|u| u.is_object()) {
            let cached = u["prompt_tokens_details"]["cached_tokens"].as_u64();
            usage = Usage {
                input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
                output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
                cache: cached.map(|read| CacheUsage { read_tokens: read, write_tokens: 0 }),
            };
        }
        let Some(choice) = data["choices"].as_array().and_then(|c| c.first()) else {
            continue;
        };
        let delta = &choice["delta"];
        if let Some(piece) = delta["content"].as_str().filter(|s| !s.is_empty()) {
            text.push_str(piece);
            on_event(ProviderEvent::TextDelta(piece.to_owned()));
        }
        for key in ["reasoning_content", "reasoning"] {
            if let Some(piece) = delta[key].as_str().filter(|s| !s.is_empty()) {
                reasoning.push_str(piece);
                on_event(ProviderEvent::ThinkingDelta(piece.to_owned()));
                break;
            }
        }
        for (pos, tc) in delta["tool_calls"].as_array().into_iter().flatten().enumerate() {
            let index = tc["index"].as_u64().unwrap_or(pos as u64);
            let entry = calls.entry(index).or_default();
            if let Some(id) = tc["id"].as_str().filter(|s| !s.is_empty()) {
                entry.id = id.to_owned();
            }
            if let Some(name) = tc["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                if entry.name.is_empty() {
                    on_event(ProviderEvent::ToolCallBegun { name: name.to_owned() });
                }
                entry.name.push_str(name);
            }
            if let Some(args) = tc["function"]["arguments"].as_str() {
                entry.args.push_str(args);
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            finish = Some(reason.to_owned());
        }
    }

    let Some(finish) = finish else {
        return Err(ProviderError::retryable("流在 finish_reason 之前结束"));
    };
    let mut blocks = Vec::new();
    if !reasoning.is_empty() {
        blocks.push(Block::Thinking { text: reasoning, provider_kind: Some(KIND.to_owned()), raw: None });
    }
    if !text.is_empty() {
        blocks.push(Block::Text(text));
    }
    for (i, call) in calls.into_values().enumerate() {
        // 少数兼容实现不给 id；补一个稳定的，保证工具结果能配对。
        let id = if call.id.is_empty() { format!("call_{i}") } else { call.id };
        blocks.push(Block::ToolUse { id, name: call.name, input: parse_args(&call.args) });
    }
    let has_calls = blocks.iter().any(|b| matches!(b, Block::ToolUse { .. }));
    let stop = match finish.as_str() {
        "length" => StopReason::MaxTokens,
        _ if has_calls => StopReason::ToolUse,
        "stop" | "tool_calls" | "function_call" => StopReason::EndTurn,
        other => StopReason::Other(other.to_owned()),
    };
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

    fn parse(stream: &str) -> Result<Option<TurnOutput>, ProviderError> {
        parse_stream(SseReader::new(stream.as_bytes()), &mut |_| {}, &AtomicBool::new(false), "fp".into())
    }

    #[test]
    fn request_body_orders_tool_messages_before_user_text() {
        let mut prompt = PromptContext::default();
        prompt.system_sections.push("sys".into());
        prompt.messages.push(ChatMessage::user_text("开始"));
        prompt.messages.push(ChatMessage {
            role: Role::Assistant,
            blocks: vec![
                Block::Thinking { text: "想".into(), provider_kind: Some(KIND.into()), raw: None },
                Block::ToolUse { id: "c1".into(), name: "f".into(), input: json!({"a":1}) },
            ],
        });
        prompt.messages.push(ChatMessage {
            role: Role::User,
            blocks: vec![
                Block::Text("另外".into()),
                Block::ToolResult { tool_use_id: "c1".into(), content: "r".into(), is_error: false },
            ],
        });
        let mut s = test_settings(ApiKind::ChatCompletions);
        s.temperature = Some(0.7);
        s.prompt_cache = false;
        let tools = vec![ToolSpec { name: "f".into(), description: "d".into(), schema: json!({"type":"object"}), parallel_safe: false }];
        let body = ChatCompletionsProvider::new(s).build_body(&prompt, &tools);
        let msgs = body["messages"].as_array().unwrap();
        let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool", "user"]);
        assert_eq!(msgs[2]["content"], Value::Null);
        assert_eq!(msgs[2]["reasoning_content"], "想");
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["arguments"], "{\"a\":1}");
        assert_eq!(msgs[3]["tool_call_id"], "c1");
        assert_eq!(body["tools"][0]["function"]["name"], "f");
        assert_eq!(body["temperature"], 0.7);
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn parses_fragmented_tool_calls_and_usage() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"嗯\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"propose\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"x\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"function\":{\"name\":\"look\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"2}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":4,\"prompt_tokens_details\":{\"cached_tokens\":8}}}\n\n",
            "data: [DONE]\n\n",
        );
        let out = parse(stream).unwrap().unwrap();
        assert_eq!(out.stop, StopReason::ToolUse);
        let uses = out.message.tool_uses();
        assert_eq!(uses.len(), 2);
        assert_eq!((uses[0].0, uses[0].1, uses[0].2), ("call_a", "propose", &json!({"x":2})));
        assert_eq!(uses[1].1, "look");
        assert_eq!(out.usage.input_tokens, 11);
        assert_eq!(out.usage.cache.unwrap().read_tokens, 8);
        assert!(matches!(&out.message.blocks[0], Block::Thinking { text, .. } if text == "嗯"));
    }

    #[test]
    fn plain_text_and_length_stop() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"雨\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"夜\"},\"finish_reason\":\"length\"}]}\n\n",
        );
        let out = parse(stream).unwrap().unwrap();
        assert_eq!(out.message.text(), "雨夜");
        assert_eq!(out.stop, StopReason::MaxTokens);
    }

    #[test]
    fn missing_finish_is_retryable_and_error_payload_is_fatal() {
        let err = parse("data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: [DONE]\n\n").unwrap_err();
        assert!(err.retryable);
        let err = parse("data: {\"error\":{\"message\":\"context too long\"}}\n\n").unwrap_err();
        assert!(!err.retryable);
        assert!(err.message.contains("context too long"));
    }
}
