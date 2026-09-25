//! 统一消息模型。各 provider 适配器负责在它与厂商报文之间双向转换，
//! agent loop 与 host 不需要知道用的是哪家 API。形状最接近 Anthropic 的 content blocks。

use serde::{Deserialize, Serialize};

/// 没有 System / Tool 角色：系统提示在 [`crate::PromptContext`] 里，
/// 工具结果是 User 消息里的 [`Block::ToolResult`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Text(String),
    /// 思考 / 推理内容。`provider_kind` + `raw` 保存厂商私有数据，只由产生它的适配器回传
    /// （例如 Responses 在 `store: false` 时必须原样回传 reasoning item）。
    Thinking {
        text: String,
        provider_kind: Option<String>,
        raw: Option<serde_json::Value>,
    },
    /// 模型发起的工具调用。参数不是合法 JSON 时以 `Value::String(原文)` 存入，
    /// 让工具层报错、模型自愈。
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

impl ChatMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        ChatMessage { role: Role::User, blocks: vec![Block::Text(text.into())] }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        ChatMessage { role: Role::Assistant, blocks: vec![Block::Text(text.into())] }
    }

    pub fn empty_assistant() -> Self {
        ChatMessage { role: Role::Assistant, blocks: Vec::new() }
    }

    pub fn tool_uses(&self) -> Vec<(&str, &str, &serde_json::Value)> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
                _ => None,
            })
            .collect()
    }

    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache: Option<CacheUsage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheUsage {
    pub read_tokens: u64,
    pub write_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        if let Some(other_cache) = other.cache {
            let cache = self.cache.get_or_insert_with(CacheUsage::default);
            cache.read_tokens += other_cache.read_tokens;
            cache.write_tokens += other_cache.write_tokens;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}
