use crate::tools::ToolError;

/// agent loop 向外发出的事件。host 把它们转成回合进度或 Tauri 事件。
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    AssistantDelta(String),
    ThinkingDelta(String),
    /// 模型开始生成一次工具调用，参数还在流式拼接中。
    ToolCallPending { name: String },
    AssistantMessage(String),
    ToolCallStarted { id: String, name: String, summary: String },
    ToolCallFinished { id: String, name: String, output: String, error: Option<ToolError> },
    RetryScheduled { attempt: u32, max_retries: u32, delay_ms: u64, error: String },
    RetryStarted { attempt: u32, max_retries: u32 },
    Notice(String),
    Error(String),
}
