use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::provider::scripted::{ScriptedProvider, ScriptedTurn};

fn fast_retry() -> RetryPolicy {
    RetryPolicy { max_attempts: 3, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(2), ..RetryPolicy::default() }
}

/// 按 IF 的方式使用挂载点：工具调用直接记录，模型想停时检查完成条件。
#[derive(Default)]
struct TaskHost {
    proposals: Vec<serde_json::Value>,
    needed: usize,
    stop_reminders: u32,
    finished: Option<bool>,
    usage: Usage,
}

impl AgentLoopHost for TaskHost {
    fn prepare_prompt(
        &mut self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _emit: &mut dyn FnMut(AgentEvent),
    ) -> anyhow::Result<PromptContext> {
        Ok(PromptContext { system_sections: vec!["视图".into()], messages: messages.to_vec() })
    }

    fn record_usage(&mut self, usage: Usage, _emit: &mut dyn FnMut(AgentEvent)) {
        self.usage.add(usage);
    }

    fn execute_tool_turn(
        &mut self,
        messages: &[ChatMessage],
        turn: &TurnOutput,
        calls: &[ToolCall],
        _emit: &mut dyn FnMut(AgentEvent),
        _cancel: &AtomicBool,
    ) -> anyhow::Result<ToolTurnResult> {
        let mut results = Vec::new();
        for call in calls {
            let (content, is_error) = if call.arguments["ok"] == json!(true) {
                self.proposals.push(call.arguments.clone());
                (format!("已登记为 cand_{:03}", self.proposals.len()), false)
            } else {
                ("judge_rejected: 需要用到角色不知道的信息".to_owned(), true)
            };
            results.push(Block::ToolResult { tool_use_id: call.id.clone(), content, is_error });
        }
        let mut committed = messages.to_vec();
        committed.push(turn.message.clone());
        committed.push(ChatMessage { role: Role::User, blocks: results });
        let done = self.proposals.len() >= self.needed + 10;
        Ok(ToolTurnResult { messages: committed, cancelled: false, stop_after_commit: done.then(|| "完成".into()) })
    }

    fn intercept_stop(
        &mut self,
        messages: &[ChatMessage],
        turn: &TurnOutput,
        _emit: &mut dyn FnMut(AgentEvent),
    ) -> anyhow::Result<Option<Vec<ChatMessage>>> {
        if self.proposals.len() >= self.needed {
            return Ok(None);
        }
        self.stop_reminders += 1;
        let mut committed = messages.to_vec();
        committed.push(turn.message.clone());
        committed.push(ChatMessage::user_text(format!("还需要 {} 条通过的提议", self.needed - self.proposals.len())));
        Ok(Some(committed))
    }

    fn finish(&mut self, cancelled: bool, _emit: &mut dyn FnMut(AgentEvent)) {
        self.finished = Some(cancelled);
    }
}

fn run(model: &ScriptedProvider, host: &mut TaskHost, cancel: &AtomicBool) -> (AgentLoopOutcome, Vec<AgentEvent>) {
    let mut events = Vec::new();
    let mut emit = |e| events.push(e);
    let outcome = run_agent_loop(
        model,
        vec![ChatMessage::user_text("开始任务")],
        &[],
        AgentLoopCallbacks::new(host, &mut emit, cancel).max_turns(6).retry_policy(fast_retry()),
    );
    (outcome, events)
}

#[test]
fn host_forces_continuation_until_completion_condition_holds() {
    let model = ScriptedProvider::new([
        ScriptedTurn::tool("propose_candidate", json!({"ok": false})),
        ScriptedTurn::text("我做完了"),
        ScriptedTurn::tool("propose_candidate", json!({"ok": true})),
        ScriptedTurn::text("完成"),
    ]);
    let mut host = TaskHost { needed: 1, ..Default::default() };
    let (outcome, _) = run(&model, &mut host, &AtomicBool::new(false));
    assert!(!outcome.cancelled);
    assert_eq!(host.proposals.len(), 1);
    assert_eq!(host.stop_reminders, 1);
    assert_eq!(host.finished, Some(false));
    assert_eq!(host.usage.input_tokens, 4);
    assert_eq!(model.remaining(), 0);
    // 否决理由与继续理由都进入了模型可见的对话
    let last_prompt = model.prompts().pop().unwrap();
    let transcript = serde_json::to_string(&last_prompt.messages).unwrap();
    assert!(transcript.contains("judge_rejected"));
    assert!(transcript.contains("还需要 1 条"));
    assert_eq!(last_prompt.system_sections, vec!["视图"]);
}

#[test]
fn stop_after_commit_ends_the_task() {
    let model = ScriptedProvider::new([ScriptedTurn::tool("p", json!({"ok": true})), ScriptedTurn::text("不该被调用")]);
    // 已有 9 条，再登记 1 条就达到 `needed + 10`，host 要求提交后停止。
    let mut host = TaskHost { proposals: vec![json!({}); 9], ..Default::default() };
    let (_, events) = run(&model, &mut host, &AtomicBool::new(false));
    assert_eq!(model.remaining(), 1);
    assert!(events.contains(&AgentEvent::Notice("完成".into())));
}

#[test]
fn retryable_errors_before_streaming_are_retried() {
    let model = ScriptedProvider::new([
        ScriptedTurn::Error { message: "HTTP 503".into(), retryable: true },
        ScriptedTurn::text("好"),
    ]);
    let mut host = TaskHost::default();
    let (outcome, events) = run(&model, &mut host, &AtomicBool::new(false));
    assert_eq!(outcome.messages.last().unwrap().text(), "好");
    assert!(events.iter().any(|e| matches!(e, AgentEvent::RetryScheduled { attempt: 1, .. })));
}

#[test]
fn fatal_errors_stop_without_committing() {
    let model = ScriptedProvider::new([ScriptedTurn::Error { message: "HTTP 400".into(), retryable: false }]);
    let mut host = TaskHost::default();
    let (outcome, events) = run(&model, &mut host, &AtomicBool::new(false));
    assert_eq!(outcome.messages.len(), 1);
    assert!(events.contains(&AgentEvent::Error("HTTP 400".into())));
    assert_eq!(host.finished, Some(false));
}

#[test]
fn cancellation_is_reported_to_host() {
    let model = ScriptedProvider::new([ScriptedTurn::text("x")]);
    let mut host = TaskHost::default();
    let (outcome, _) = run(&model, &mut host, &AtomicBool::new(true));
    assert!(outcome.cancelled);
    assert_eq!(host.finished, Some(true));
    assert_eq!(model.remaining(), 1);
}

#[test]
fn max_turns_bounds_a_model_that_never_satisfies_the_host() {
    let model = ScriptedProvider::new((0..10).map(|_| ScriptedTurn::text("完成了")));
    let mut host = TaskHost { needed: 5, ..Default::default() };
    let (_, events) = run(&model, &mut host, &AtomicBool::new(false));
    assert_eq!(model.remaining(), 4, "max_turns = 6");
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Notice(n) if n.contains("上限"))));
}
