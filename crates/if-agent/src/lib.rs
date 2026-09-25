//! # if-agent
//!
//! 从 Onemore（`ai-agent-example/`）移植的 agent 底层（docs/05 §3.1）：
//!
//! - [`agent_loop`]：`run_agent_loop` 与 [`AgentLoopHost`] 挂载点。IF 的 `IfTaskHost` 在
//!   `if-pipeline` 中实现，runtime 由 Jev 主导。
//! - [`provider`]：Anthropic Messages、OpenAI Responses，以及新增的 OpenAI Chat Completions。
//! - [`message`]：厂商无关的消息模型。
//! - [`tools`]：工具声明、注册表、JSON Schema 子集校验（补上数组关键字）、`JudgeRejected`。
//!
//! 去掉了编码类工具、权限、skills、planning、compaction、TUI、RPC 与图片输入。

#![forbid(unsafe_code)]

pub mod agent_loop;
pub mod context;
pub mod event;
pub mod message;
pub mod provider;
pub mod tools;

pub use agent_loop::{
    run_agent_loop, AgentLoopCallbacks, AgentLoopHost, AgentLoopOutcome, RetryPolicy, ToolCall,
    ToolTurnResult,
};
pub use context::PromptContext;
pub use event::AgentEvent;
pub use message::{Block, CacheUsage, ChatMessage, Role, StopReason, Usage};
pub use provider::{
    build_provider, ApiKind, FailedTurn, Provider, ProviderError, ProviderEvent, ProviderSettings,
    StreamTerminal, TurnOutput,
};
pub use tools::{Tool, ToolError, ToolErrorCode, ToolOutcome, ToolOutput, ToolRegistry, ToolSpec};
