//! OpenAI Responses API 适配器，从 Onemore 移植（去掉了图片输入与原生网页搜索）。
//!
//! - 输入是 item 列表：消息、函数调用、函数结果、推理并列；
//! - `replay_encrypted_reasoning` 开启时以 `store: false` 无状态使用，并原样回传加密 reasoning item，
//!   否则下一轮带 function_call 的请求会被 400 拒绝；
//! - 只消费必要的流事件子集，未知事件一律忽略。

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

const KIND: &str = "openai_responses";

pub struct ResponsesProvider {
    settings: ProviderSettings,
    agent: ureq::Agent,
}

impl ResponsesProvider {
    pub fn new(settings: ProviderSettings) -> Self {
        ResponsesProvider { settings, agent: http_agent() }
    }

    pub(crate) fn build_body(&self, prompt: &PromptContext, tools: &[ToolSpec]) -> Value {
        let replay = self.settings.replay_encrypted_reasoning;
        let mut input: Vec<Value> = Vec::new();
        for m in &prompt.messages {
            for b in &m.blocks {
                match (m.role, b) {
                    (Role::User, Block::Text(t)) if !t.is_empty() => input.push(json!({
                        "type": "message", "role": "user",
                        "content": [{"type": "input_text", "text": t}],
                    })),
                    (Role::User, Block::ToolResult { tool_use_id, content, is_error }) => input.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_use_id,
                        "output": if *is_error { format!("ERROR: {content}") } else { content.clone() },
                    })),
                    (Role::Assistant, Block::Text(t)) if !t.is_empty() => input.push(json!({
                        "type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": t}],
                    })),
                    (Role::Assistant, Block::ToolUse { id, name, input: args }) => input.push(json!({
                        "type": "function_call", "call_id": id, "name": name, "arguments": args_to_string(args),
                    })),
                    (Role::Assistant, Block::Thinking { provider_kind, raw: Some(item), .. })
                        if replay && provider_kind.as_deref() == Some(KIND) =>
                    {
                        input.push(item.clone());
                    }
                    _ => {}
                }
            }
        }

        let mut body = json!({ "model": self.settings.model, "input": input, "stream": true });
        if replay {
            body["store"] = json!(false);
            body["include"] = json!(["reasoning.encrypted_content"]);
        }
        if let Some(effort) = &self.settings.reasoning_effort {
            body["reasoning"] = json!({"effort": effort});
        } else if let Some(t) = self.settings.temperature {
            body["temperature"] = json!(t);
        }
        let system = prompt.system_text();
        if !system.is_empty() {
            body["instructions"] = json!(system);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(
                super::sorted_tools(tools)
                    .into_iter()
                    .map(|t| json!({ "type": "function", "name": t.name, "description": t.description, "parameters": t.schema }))
                    .collect(),
            );
        }
        if let Some(n) = self.settings.max_tokens {
            body["max_output_tokens"] = json!(n);
        }
        if self.settings.prompt_cache {
            body["prompt_cache_key"] = json!(super::prompt_cache_key(&self.settings, prompt, tools));
        }
        body
    }
}

impl Provider for ResponsesProvider {
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
            let url = super::url_join(&self.settings.base_url, "v1/responses");
            let mut headers = Vec::new();
            if !self.settings.api_key.is_empty() {
                headers.push(("authorization", format!("Bearer {}", self.settings.api_key)));
            }
            let body = self.build_body(prompt, tools);
            let fingerprint = super::prompt_fingerprint(&self.settings, prompt, tools);
            let reader = post_sse(&self.agent, &url, &headers, &body)?;
            parse_stream(SseReader::new(reader), on_event, cancel, fingerprint, self.settings.replay_encrypted_reasoning)
        })
    }
}

enum Partial {
    Msg(String),
    Fc { call_id: String, name: String, args: String },
    Reasoning(String),
}

fn block_from_item(item: &Value, replay: bool) -> Option<Block> {
    match item["type"].as_str().unwrap_or("") {
        "message" => {
            let text: String = item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["type"] == "output_text")
                .filter_map(|p| p["text"].as_str())
                .collect();
            (!text.is_empty()).then_some(Block::Text(text))
        }
        "function_call" => Some(Block::ToolUse {
            id: item["call_id"].as_str().or_else(|| item["id"].as_str()).unwrap_or("").to_owned(),
            name: item["name"].as_str().unwrap_or("").to_owned(),
            input: parse_args(item["arguments"].as_str().unwrap_or("")),
        }),
        "reasoning" => {
            let mut summary: String =
                item["summary"].as_array().into_iter().flatten().filter_map(|p| p["text"].as_str()).collect();
            if summary.is_empty() {
                for key in ["text", "reasoning_text", "content"] {
                    if let Some(text) = item[key].as_str() {
                        summary.push_str(text);
                    }
                }
            }
            let replayable = replay && item.get("encrypted_content").and_then(Value::as_str).is_some();
            Some(Block::Thinking {
                text: summary,
                provider_kind: replayable.then(|| KIND.to_owned()),
                raw: replayable.then(|| item.clone()),
            })
        }
        _ => None,
    }
}

fn read_usage(u: &Value) -> Usage {
    let details = &u["input_tokens_details"];
    let read = details.get("cached_tokens").and_then(Value::as_u64);
    let write = details.get("cache_write_tokens").and_then(Value::as_u64);
    Usage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
        cache: (read.is_some() || write.is_some())
            .then(|| CacheUsage { read_tokens: read.unwrap_or(0), write_tokens: write.unwrap_or(0) }),
    }
}

pub(crate) fn parse_stream<R: std::io::Read>(
    mut sse: SseReader<R>,
    on_event: &mut dyn FnMut(ProviderEvent),
    cancel: &AtomicBool,
    fingerprint: String,
    replay: bool,
) -> Result<Option<TurnOutput>, ProviderError> {
    let mut partials: BTreeMap<u64, Partial> = BTreeMap::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut usage = Usage::default();
    let mut stop: Option<StopReason> = None;
    let mut saw_terminal = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let Some(ev) = sse.next_event().map_err(|e| ProviderError::retryable(format!("读取流失败: {e}")))? else {
            break;
        };
        if ev.data == "[DONE]" {
            return Err(ProviderError::fatal("流在 response terminal 事件前收到 [DONE]"));
        }
        let data: Value =
            serde_json::from_str(&ev.data).map_err(|e| ProviderError::fatal(format!("流事件 JSON 无效: {e}")))?;
        let index = data["output_index"].as_u64().unwrap_or(0);
        match data["type"].as_str().unwrap_or("") {
            "response.output_item.added" => {
                let item = &data["item"];
                match item["type"].as_str().unwrap_or("") {
                    "message" => {
                        partials.insert(index, Partial::Msg(String::new()));
                    }
                    "reasoning" => {
                        partials.insert(index, Partial::Reasoning(String::new()));
                    }
                    "function_call" => {
                        let name = item["name"].as_str().unwrap_or("").to_owned();
                        on_event(ProviderEvent::ToolCallBegun { name: name.clone() });
                        partials.insert(
                            index,
                            Partial::Fc { call_id: item["call_id"].as_str().unwrap_or("").to_owned(), name, args: String::new() },
                        );
                    }
                    _ => {}
                }
            }
            "response.output_text.delta" => {
                let piece = data["delta"].as_str().unwrap_or("");
                if let Some(Partial::Msg(buf)) = partials.get_mut(&index) {
                    buf.push_str(piece);
                }
                on_event(ProviderEvent::TextDelta(piece.to_owned()));
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let piece = data["delta"].as_str().unwrap_or("");
                if let Some(Partial::Reasoning(buf)) = partials.get_mut(&index) {
                    buf.push_str(piece);
                }
                on_event(ProviderEvent::ThinkingDelta(piece.to_owned()));
            }
            "response.function_call_arguments.delta" => {
                if let Some(Partial::Fc { args, .. }) = partials.get_mut(&index) {
                    args.push_str(data["delta"].as_str().unwrap_or(""));
                }
            }
            "response.output_item.done" => {
                partials.remove(&index);
                if let Some(b) = block_from_item(&data["item"], replay) {
                    blocks.push(b);
                }
            }
            "response.completed" => {
                saw_terminal = true;
                if let Some(u) = data["response"].get("usage").filter(|u| !u.is_null()) {
                    usage = read_usage(u);
                }
                break;
            }
            "response.incomplete" => {
                let resp = &data["response"];
                let reason = resp["incomplete_details"]["reason"].as_str().unwrap_or("unknown");
                if reason != "max_output_tokens" {
                    return Err(ProviderError::fatal(format!("响应未完成: {reason}")));
                }
                saw_terminal = true;
                if let Some(u) = resp.get("usage").filter(|u| !u.is_null()) {
                    usage = read_usage(u);
                }
                stop = Some(StopReason::MaxTokens);
                break;
            }
            "response.failed" => {
                return Err(ProviderError::fatal(format!(
                    "API 错误: {}",
                    data["response"]["error"]["message"].as_str().unwrap_or("response.failed（无详情）")
                )));
            }
            "error" => {
                return Err(ProviderError::fatal(format!("API 流错误: {}", data["message"].as_str().unwrap_or("未知"))));
            }
            _ => {}
        }
    }
    if !saw_terminal {
        return Err(ProviderError::retryable("流在 response terminal 事件前结束"));
    }
    for p in partials.into_values() {
        match p {
            Partial::Msg(t) if !t.is_empty() => blocks.push(Block::Text(t)),
            Partial::Fc { call_id, name, args } => blocks.push(Block::ToolUse { id: call_id, name, input: parse_args(&args) }),
            Partial::Reasoning(t) if !t.is_empty() => blocks.push(Block::Thinking { text: t, provider_kind: None, raw: None }),
            _ => {}
        }
    }
    let has_calls = blocks.iter().any(|b| matches!(b, Block::ToolUse { .. }));
    let stop = stop.unwrap_or(if has_calls { StopReason::ToolUse } else { StopReason::EndTurn });
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

    #[test]
    fn request_body_shape() {
        let mut prompt = PromptContext::default();
        prompt.system_sections.push("sys".into());
        prompt.messages.push(ChatMessage {
            role: Role::Assistant,
            blocks: vec![
                Block::Thinking { text: "t".into(), provider_kind: Some(KIND.into()), raw: Some(json!({"type":"reasoning","encrypted_content":"x"})) },
                Block::ToolUse { id: "c1".into(), name: "f".into(), input: json!({"a":1}) },
            ],
        });
        prompt.messages.push(ChatMessage {
            role: Role::User,
            blocks: vec![Block::ToolResult { tool_use_id: "c1".into(), content: "bad".into(), is_error: true }],
        });
        let mut s = test_settings(ApiKind::OpenAiResponses);
        let body = ResponsesProvider::new(s.clone()).build_body(&prompt, &[]);
        assert_eq!(body["instructions"], "sys");
        assert_eq!(body["input"][0]["type"], "function_call", "未开启回放时不回传 reasoning");
        assert_eq!(body["input"][1]["output"], "ERROR: bad");
        assert!(body["prompt_cache_key"].as_str().unwrap().starts_with("if:v1:"));
        s.replay_encrypted_reasoning = true;
        let body = ResponsesProvider::new(s).build_body(&prompt, &[]);
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][0]["type"], "reasoning");
    }

    #[test]
    fn parses_function_call_stream() {
        let stream = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"name\":\"f\",\"call_id\":\"c1\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"a\\\":1}\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"name\":\"f\",\"call_id\":\"c1\",\"arguments\":\"{\\\"a\\\":1}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"input_tokens_details\":{\"cached_tokens\":2}}}}\n\n",
        );
        let out = parse_stream(SseReader::new(stream.as_bytes()), &mut |_| {}, &AtomicBool::new(false), "fp".into(), false)
            .unwrap()
            .unwrap();
        assert_eq!(out.stop, StopReason::ToolUse);
        assert_eq!(out.usage.cache.unwrap().read_tokens, 2);
        assert_eq!(out.message.tool_uses()[0].1, "f");
    }
}
