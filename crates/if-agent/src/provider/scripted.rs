//! 按脚本返回预设回复的 provider，用于回合流程测试（docs/12 §8"模拟的 provider"）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::Value;

use super::{FailedTurn, Provider, ProviderError, ProviderEvent, StreamTerminal, TurnOutput};
use crate::context::PromptContext;
use crate::message::{Block, ChatMessage, Role, StopReason, Usage};
use crate::tools::ToolSpec;

#[derive(Debug, Clone)]
pub enum ScriptedTurn {
    /// 文本回复。每个分片发一次 `TextDelta`，便于测试流式切分。
    Text(Vec<String>),
    /// 若干工具调用，(名字, 参数)。ID 自动编号。
    ToolCalls(Vec<(String, Value)>),
    /// 一次失败。`retryable` 决定 agent loop 是否重试。
    Error { message: String, retryable: bool },
}

impl ScriptedTurn {
    pub fn text(text: impl Into<String>) -> Self {
        ScriptedTurn::Text(vec![text.into()])
    }

    pub fn tool(name: impl Into<String>, args: Value) -> Self {
        ScriptedTurn::ToolCalls(vec![(name.into(), args)])
    }
}

#[derive(Debug, Default)]
pub struct ScriptedProvider {
    turns: Mutex<VecDeque<ScriptedTurn>>,
    prompts: Mutex<Vec<PromptContext>>,
    call_counter: Mutex<u64>,
}

impl ScriptedProvider {
    pub fn new(turns: impl IntoIterator<Item = ScriptedTurn>) -> Self {
        ScriptedProvider { turns: Mutex::new(turns.into_iter().collect()), ..Default::default() }
    }

    /// 收到的每次 prompt，按顺序。
    pub fn prompts(&self) -> Vec<PromptContext> {
        self.prompts.lock().expect("poisoned").clone()
    }

    pub fn remaining(&self) -> usize {
        self.turns.lock().expect("poisoned").len()
    }
}

impl Provider for ScriptedProvider {
    fn label(&self) -> String {
        "scripted".into()
    }

    fn model(&self) -> &str {
        "scripted"
    }

    fn stream_turn(
        &self,
        prompt: &PromptContext,
        _tools: &[ToolSpec],
        on_event: &mut dyn FnMut(ProviderEvent),
        cancel: &AtomicBool,
    ) -> StreamTerminal {
        self.prompts.lock().expect("poisoned").push(prompt.clone());
        if cancel.load(Ordering::Relaxed) {
            return StreamTerminal::Aborted(FailedTurn::aborted());
        }
        let Some(turn) = self.turns.lock().expect("poisoned").pop_front() else {
            return StreamTerminal::Error(FailedTurn::from_error(ProviderError::fatal("脚本已用完")));
        };
        let (blocks, stop) = match turn {
            ScriptedTurn::Text(chunks) => {
                for c in &chunks {
                    on_event(ProviderEvent::TextDelta(c.clone()));
                }
                (vec![Block::Text(chunks.concat())], StopReason::EndTurn)
            }
            ScriptedTurn::ToolCalls(calls) => {
                let mut counter = self.call_counter.lock().expect("poisoned");
                let blocks = calls
                    .into_iter()
                    .map(|(name, input)| {
                        *counter += 1;
                        on_event(ProviderEvent::ToolCallBegun { name: name.clone() });
                        Block::ToolUse { id: format!("call_{}", *counter), name, input }
                    })
                    .collect();
                (blocks, StopReason::ToolUse)
            }
            ScriptedTurn::Error { message, retryable } => {
                let error = if retryable { ProviderError::retryable(message) } else { ProviderError::fatal(message) };
                return StreamTerminal::Error(FailedTurn::from_error(error));
            }
        };
        StreamTerminal::Done(TurnOutput {
            message: ChatMessage { role: Role::Assistant, blocks },
            usage: Usage { input_tokens: 1, output_tokens: 1, cache: None },
            stop,
            prompt_fingerprint: None,
        })
    }
}
