//! 提议任务的 host（docs/05 §3 的 `IfTaskHost`）。
//!
//! 复用 Onemore 的 [`run_agent_loop`](if_agent::run_agent_loop)，只重写两个挂载点：
//!
//! - `execute_tool_turn`：校验参数 → 把提议落进**任务工作区**（`Workspace`）→
//!   把「已登记为 `cand_0004`」这类回执交还模型。任何工具都不直接写事件日志
//!   （docs/05 §2.1）。
//! - `intercept_stop`：模型不调工具就直接收尾时，用一句具体的话把它拉回来
//!   （docs/05 §2.2「模型不能自己决定任务结束」）。
//!
//! 判定（Jev）**不在这一层**。裁决策略在 [`crate::candidates`] / [`crate::scenes`] /
//! [`crate::beats`] 里按回合的固定顺序发生，那是「引擎裁决一次」的地方；
//! 宿主只负责把提议拿回来。这是刻意的分工：模型提出、Jev 判断、引擎裁决。

use std::sync::atomic::AtomicBool;

use anyhow::Result;
use if_agent::message::{Block, ChatMessage, Role};
use if_agent::provider::TurnOutput;
use if_agent::tools::{schema, ToolError, ToolErrorCode, ToolOutcome, ToolSpec};
use if_agent::{
    run_agent_loop, AgentEvent, AgentLoopCallbacks, AgentLoopHost, Provider, ToolCall,
    ToolTurnResult,
};

use if_domain::id::{CandidateId, SceneId};
use if_domain::narrative::ScenePlan;
use if_domain::turn::Candidate;

use crate::commit::ObservedChange;
use crate::scenes::SceneProposal;

use super::resolver::Resolver;
use super::{extract, impact, plan, scenes, TaskError};

/// 任务种类。工具、提示词与回执话术都挂在它上面。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Impact,
    Scenes,
    Plan,
    Extract,
}

impl TaskKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            TaskKind::Impact => "T-impact",
            TaskKind::Scenes => "T-scenes",
            TaskKind::Plan => "T-plan",
            TaskKind::Extract => "T-extract",
        }
    }

    /// 本任务唯一被允许调用的工具（docs/05 §4）。
    pub fn tool(self) -> ToolSpec {
        match self {
            TaskKind::Impact => impact::tool(),
            TaskKind::Scenes => scenes::tool(),
            TaskKind::Plan => plan::tool(),
            TaskKind::Extract => extract::tool(),
        }
    }

    /// 轮数上限【初始值】（docs/05 §4）。
    pub const fn max_rounds(self) -> u32 {
        match self {
            TaskKind::Impact => 4,
            TaskKind::Scenes => 4,
            TaskKind::Plan => 3,
            TaskKind::Extract => 4,
        }
    }

    /// 「什么都不交」是不是一个正常结果。
    ///
    /// 只有 T-extract 是：一场戏确实可能**什么都没改**（人只是说了句话），
    /// 那时模型不调工具才是对的。其余三个任务空手而归说明模型没干活，
    /// 要当成失败冒上去，不能假装世界推了一步。
    pub const fn allows_empty(self) -> bool {
        matches!(self, TaskKind::Extract)
    }

    /// 模型只说话不调工具时的拉回话术。
    fn reminder(self) -> String {
        match self {
            TaskKind::Impact => {
                "你还没有提交任何候选。不要再解释——**直接调用 propose_candidate 工具**，\
                 每一条候选调用一次；视图里没有命题时 affects / based_on 留空数组即可。"
                    .to_owned()
            }
            TaskKind::Scenes => {
                "你还没有提交任何场景候选。请为每个走向各调用一次 propose_scene。".to_owned()
            }
            TaskKind::Plan => {
                "你还没有提交场景计划。请调用 submit_scene_plan 提交一份完整的计划。".to_owned()
            }
            TaskKind::Extract => {
                "你还没有记录任何回收结果。请对正文里每一处已经发生的改变调用一次 record_observation；\
                 没有任何改变时，也要据此说明并提交一次空的 `text` 之外的结果。"
                    .to_owned()
            }
        }
    }
}

/// 任务工作区：提议的落脚处（docs/05 §2.1「提议的效果先进入任务工作区」）。
///
/// 它是**回合内的**：回合末由引擎对账、写进事件日志（[`crate::commit`]），
/// 之后就没有用了。所以这里没有 ID、没有持久化，只有模型说过什么。
#[derive(Debug, Default)]
pub struct Workspace {
    pub candidates: Vec<Candidate>,
    pub scenes: Vec<SceneProposal>,
    pub plan: Option<ScenePlan>,
    pub observations: Vec<ObservedChange>,
    /// 提议被收下时发现的偏差（认不出的名字、被丢掉的引用）。
    /// **不进事件日志**，随回合的 warnings 一起给用户看。
    pub warnings: Vec<String>,
    /// 成功登记的提议条数。用来判断「这一轮到底有没有产出」。
    pub collected: usize,
    next_candidate: u32,
    next_scene: u32,
}

impl Workspace {
    /// 从「世界里已经有几场戏」开始发场景号（见 [`TaskInputs::first_scene`]）。
    ///
    /// 候选 ID 不吃这个起点：它只在本回合的话题里出现，跨回合重号无害。
    pub fn starting_at(first_scene: u64) -> Self {
        Self {
            next_scene: first_scene as u32,
            ..Self::default()
        }
    }

    /// 候选 ID 由引擎发号（docs/05 §5.2）。模型只从工具回执里读到它。
    pub fn next_candidate_id(&mut self) -> CandidateId {
        self.next_candidate += 1;
        CandidateId::numbered(self.next_candidate as u64)
    }

    pub fn next_scene_id(&mut self) -> SceneId {
        self.next_scene += 1;
        SceneId::numbered(self.next_scene as u64)
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// 已经发出去的候选 ID，供 `depends_on` 校验。
    pub fn candidate_ids(&self) -> Vec<CandidateId> {
        self.candidates.iter().map(|candidate| candidate.id.clone()).collect()
    }
}

/// 任务额外的输入。多数任务不用，所以给了 `Default`。
#[derive(Clone, Debug, Default)]
pub struct TaskInputs {
    /// T-plan 用：本回合被裁决放行的候选 ID。`proposed_changes` 只能引用它们——
    /// 引一个没被放行的候选，等于让正文去演一件骰子说不会发生的事。
    pub accepted_candidates: Vec<CandidateId>,
    /// T-extract 用：已展示的节拍序号（1 基）。`beat` 只能落在这个范围里。
    pub displayed_beats: u32,
    /// T-scenes 用：场景 ID 的**发号起点**——世界里已经有几场戏（`TurnContext::scene_index`）。
    ///
    /// 候选 ID 是本回合内的话题，跨回合重号无害；**场景 ID 不是**：它会被写进
    /// `Projection.scenes`、也会盖在每条事件的 `scene` 字段上。如果每回合都从
    /// `scene_0001` 重发，第二场戏就会覆盖第一场——投影里只剩最后一场。
    /// 所以起点由**调用方**按世界现状给，而不是由任务层从 1 开始数。
    pub first_scene: u64,
}

/// 一次提议任务的产出。
#[derive(Debug)]
pub struct TaskOutcome {
    pub workspace: Workspace,
    /// 实际发生的模型轮数。写进 `TaskRecord.rounds`（docs/03 §3）。
    pub rounds: u32,
}

pub struct ProposalHost<'a> {
    kind: TaskKind,
    system: String,
    resolver: &'a Resolver<'a>,
    inputs: TaskInputs,
    tool: ToolSpec,
    out: Workspace,
    /// 模型「只说话、不调工具」的次数。上限由 [`TaskKind::max_rounds`] 给。
    nudges: u32,
    rounds: u32,
    /// 模型最后说的一句自然语言。收尾失败时用它解释「它到底想干什么」——
    /// 没有这一条，用户只看到「没有产出提议」，无从判断是模型答非所问还是契约有问题。
    last_text: String,
    /// 被调用了、但参数没通过校验的工具次数。
    /// **与「一次都没调用」是两回事**：前者说明 schema 或模型笔法有问题，后者说明模型没干活。
    rejected: u32,
    /// 最后一次工具参数报错，原样带给用户（照抄 `diagnostics::draft_from_tool_args` 的做法）。
    last_error: Option<String>,
}

impl<'a> std::fmt::Debug for ProposalHost<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProposalHost")
            .field("kind", &self.kind)
            .field("rounds", &self.rounds)
            .field("collected", &self.out.collected)
            .finish_non_exhaustive()
    }
}

impl<'a> ProposalHost<'a> {
    pub fn new(kind: TaskKind, system: String, resolver: &'a Resolver<'a>, inputs: TaskInputs) -> Self {
        // 发号起点由调用方给（见 `TaskInputs::first_scene`）：任务层不知道世界里已经有几场戏。
        let out = Workspace::starting_at(inputs.first_scene);
        Self {
            kind,
            system,
            resolver,
            inputs,
            tool: kind.tool(),
            out,
            nudges: 0,
            rounds: 0,
            last_text: String::new(),
            rejected: 0,
            last_error: None,
        }
    }

    pub fn into_workspace(self) -> Workspace {
        self.out
    }

    /// 这次任务发起了几次模型调用。写进 `TaskRecord.rounds`（docs/03 §3）。
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// 模型最后说的那句自然语言（可能是空的）。失败时带进错误信息。
    pub fn last_text(&self) -> &str {
        &self.last_text
    }

    /// 有过几次工具调用没通过参数校验。
    pub fn rejected(&self) -> u32 {
        self.rejected
    }

    /// 最后一次工具参数报错的原文。
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn run(&mut self, call: &ToolCall) -> ToolOutcome {
        if call.name != self.tool.name {
            return ToolOutcome::failure(ToolError::new(
                ToolErrorCode::UnknownTool,
                format!(
                    "本任务只能调用 {}，{:?} 不是可用工具。",
                    self.tool.name, call.name
                ),
            ));
        }
        // 参数先校验再执行（docs/05 §1：结构化输出靠工具参数，不再从自由文本里解析 JSON）。
        if let Err(errors) = schema::validate(&self.tool.schema, &call.arguments) {
            return ToolOutcome::failure(ToolError::invalid_arguments(format!(
                "{} 的参数不合规：{}",
                self.tool.name,
                errors.join("；")
            )));
        }
        let outcome = match self.kind {
            TaskKind::Impact => impact::accept(&call.arguments, self.resolver, &mut self.out, &self.inputs),
            TaskKind::Scenes => scenes::accept(&call.arguments, self.resolver, &mut self.out, &self.inputs),
            TaskKind::Plan => plan::accept(&call.arguments, self.resolver, &mut self.out, &self.inputs),
            TaskKind::Extract => extract::accept(&call.arguments, self.resolver, &mut self.out, &self.inputs),
        };
        match outcome {
            Ok(output) => {
                self.out.collected += 1;
                ToolOutcome::success(output)
            }
            Err(error) => ToolOutcome::failure(error),
        }
    }
}

impl AgentLoopHost for ProposalHost<'_> {
    fn prepare_prompt(
        &mut self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<if_agent::PromptContext> {
        self.rounds += 1;
        Ok(if_agent::PromptContext {
            system_sections: vec![self.system.clone()],
            messages: messages.to_vec(),
        })
    }

    fn execute_tool_turn(
        &mut self,
        messages: &[ChatMessage],
        turn: &TurnOutput,
        calls: &[ToolCall],
        emit: &mut dyn FnMut(AgentEvent),
        _cancel: &AtomicBool,
    ) -> Result<ToolTurnResult> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            emit(AgentEvent::ToolCallStarted {
                id: call.id.clone(),
                name: call.name.clone(),
                summary: call.arguments.to_string(),
            });
            let outcome = self.run(call);
            if outcome.is_error() {
                self.rejected += 1;
                self.last_error = Some(outcome.output.model_text.clone());
            }
            emit(AgentEvent::ToolCallFinished {
                id: call.id.clone(),
                name: call.name.clone(),
                output: outcome.output.model_text.clone(),
                error: outcome.error.clone(),
            });
            results.push(Block::ToolResult {
                tool_use_id: call.id.clone(),
                content: outcome.output.model_text.clone(),
                is_error: outcome.is_error(),
            });
        }
        // 整批原子提交：assistant 消息与所有工具结果一起进对话（docs/05 §3）。
        let mut committed = messages.to_vec();
        committed.push(turn.message.clone());
        committed.push(ChatMessage {
            role: Role::User,
            blocks: results,
        });
        Ok(ToolTurnResult {
            messages: committed,
            cancelled: false,
            stop_after_commit: None,
        })
    }

    /// 模型不能自己决定任务结束（docs/05 §2.2）：没产出提议就把话说明白，让它继续。
    fn intercept_stop(
        &mut self,
        messages: &[ChatMessage],
        turn: &TurnOutput,
        _emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<Option<Vec<ChatMessage>>> {
        // 无论拉不拉回，都记住模型最后说的那句话：失败时要把它带出去。
        let said = turn.message.text();
        if !said.trim().is_empty() {
            self.last_text = said;
        }
        if self.out.collected > 0 || self.nudges >= self.kind.max_rounds() {
            return Ok(None);
        }
        self.nudges += 1;
        let mut next = messages.to_vec();
        next.push(turn.message.clone());
        next.push(ChatMessage::user_text(self.kind.reminder()));
        Ok(Some(next))
    }
}

/// 跑一个提议任务：一次 `run_agent_loop`，返回它收下的东西。
///
/// 失败分三类，都如实报出来（不把上游故障伪装成「提议不合规」，那会把排查方向带偏）：
/// 取消、上游报错、以及「跑完了但什么都没收到」。
pub fn run_proposal_task(
    provider: &dyn Provider,
    kind: TaskKind,
    prompt: super::TaskPrompt,
    resolver: &Resolver<'_>,
    inputs: TaskInputs,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<TaskOutcome, TaskError> {
    let mut host = ProposalHost::new(kind, prompt.system, resolver, inputs);
    let messages = vec![ChatMessage::user_text(prompt.user)];
    let tools = vec![kind.tool()];

    let mut errors: Vec<String> = Vec::new();
    let outcome = {
        let mut forward = |event: AgentEvent| {
            if let AgentEvent::Error(message) = &event {
                errors.push(message.clone());
            }
            emit(event);
        };
        run_agent_loop(
            provider,
            messages,
            &tools,
            AgentLoopCallbacks::new(&mut host, &mut forward, cancel).max_turns(kind.max_rounds()),
        )
    };

    let task = kind.as_str();
    let rounds = host.rounds();
    let rejected = host.rejected();
    let last_error = host.last_error().map(str::to_owned);
    let last_text = host.last_text().trim().to_owned();
    let workspace = host.into_workspace();
    if outcome.cancelled {
        return Err(TaskError::Cancelled { task });
    }
    if !errors.is_empty() {
        return Err(TaskError::Provider {
            task,
            reason: errors.join("；"),
        });
    }
    if workspace.collected == 0 && !kind.allows_empty() {
        // 报错要说实话：**「一次都没调用」与「调用了但参数都没过」是两回事**，
        // 修法完全不同（前者是契约/提示词的问题，后者是 schema 或模型笔法的问题）。
        // 只回一句「没有产出提议」，等于让用户去猜（`IfDraft::input` 那次就是这么翻车的）。
        let tool = kind.tool().name;
        let mut reason = format!("模型在 {rounds} 轮里都没有成功登记任何 {tool}——");
        if rejected > 0 {
            reason.push_str(&format!("它调用了 {rejected} 次，但参数都没通过校验"));
        } else {
            reason.push_str("它一次都没有调用这个工具");
        }
        if let Some(error) = last_error {
            reason.push_str(&format!("；最后一次的参数错误：{error}"));
        }
        if !last_text.is_empty() {
            let said: String = last_text.chars().take(200).collect();
            reason.push_str(&format!("；模型最后说的是：{said}"));
        }
        return Err(TaskError::Incomplete { task, reason });
    }
    Ok(TaskOutcome { workspace, rounds })
}
