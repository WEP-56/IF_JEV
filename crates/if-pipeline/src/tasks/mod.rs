//! 五个 agent 任务（docs/05 §4）：把「提议」从模型那里拿回来。
//!
//! 这一层是 [`if_pipeline`](crate) 唯一的对外接口——也是唯一认识 LLM 的地方。
//! 底下那七个模块（`context` / `candidates` / `scenes` / `beats` / `commit` / `turn`）
//! 仍然不认识网络：它们收提议、问 Jev、出补丁。分工就是信条本身：
//!
//! ```text
//! 用户断言 → LLM 提出 → Jev 判断 → 引擎裁决提交 → LLM 叙述
//!             ^^^^^^^^                            ^^^^^^^^
//!             本模块（T-impact / T-scenes / T-plan / T-render / T-extract）
//! ```
//!
//! | 任务 | 视图 | 工具 | 产物 |
//! |---|---|---|---|
//! | [`TaskKind::Impact`] | 上帝视图 | `propose_candidate` | [`if_domain::turn::Candidate`] |
//! | [`TaskKind::Scenes`] | 导演视图 | `propose_scene` | [`crate::scenes::SceneProposal`] |
//! | [`TaskKind::Plan`] | 导演视图 | `submit_scene_plan` | [`if_domain::narrative::ScenePlan`] |
//! | [`render`] T-render | 叙事视图 | 无（正文走文本输出，用分隔标记切节拍） | [`crate::beats::BeatProposal`] |
//! | [`TaskKind::Extract`] | 检查视图 | `record_observation` | [`crate::commit::ObservedChange`] |
//!
//! 两条与真机教训一脉相承的规矩：
//!
//! 1. **有确定性依据的东西不问模型。** 候选 ID 由引擎发号；`ScenePlan.forbidden_resolutions`
//!    由 [`crate::turn::inject_constraints`] 注入；稳定的决策键由
//!    [`Candidate::decision_key`](if_domain::turn::Candidate::decision_key) 从命题键推导。
//!    模型少一个自由度，就少一类答错的地方（`suggested_lock` 那次就是这么翻车的）。
//! 2. **每个工具的 schema 与「模型要填的结构体」必须集合相等**，由各模块的
//!    `the_tool_schema_matches_the_struct_the_model_fills` 盯着。少一个字段，
//!    模型永远不返回它，反序列化当场失败，用户只看到一句看不懂的报错。
//!
//! 读类工具（`lookup_subject` / `read_recent` …，docs/05 §5.1）**v1 不提供**：
//! 当前视图直接进提示词，再开一层查询工具只是让同一份信息走两条路。
//! 等世界大到提示词装不下（真实卡 10 万字量级，docs/13 §6.4）再加。

mod driver;
mod extract;
mod host;
mod impact;
mod plan;
mod render;
mod resolver;
mod scenes;

#[cfg(test)]
mod tests;

pub use driver::{
    extract_observations, render_text, run_if_turn, TurnFailure, TurnOutcome, TurnProviders,
    TurnRequest,
};
pub use host::{run_proposal_task, ProposalHost, TaskInputs, TaskKind, TaskOutcome, Workspace};
pub use render::BEAT_MARKER;
pub use render::{beats_from_text, Rendered, TASK as RENDER_TASK};
pub use resolver::{Resolved, Resolver};

use serde::de::DeserializeOwned;
use serde_json::Value;

use if_agent::ToolError;
use if_domain::projection::Projection;
use if_views::ViewRequest;

/// 一次任务的提示词。
///
/// 系统段与本次内容是分开的两块：稳定前缀（系统段 + 工具）决定 prompt cache 的命中，
/// 而它对同一个任务永远是同一份（docs/08：稳定前缀在前、任务内容在后）。
/// 把它们合成一个字符串就等于把这条性质交给调用方的自觉。
#[derive(Clone, Debug)]
pub struct TaskPrompt {
    pub system: String,
    pub user: String,
}

impl TaskPrompt {
    pub fn new(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
        }
    }
}

/// 任务层可能出现的失败。
///
/// 与 [`crate::PipelineError`] 分开：那个说的是「编排编不下去了」，
/// 这个说的是「这一轮模型没把提议交上来」。后者要能降级
/// （docs/06 §9：拿不到提议就跳过这一回合、记一条警告），
/// 前者不该被降级成静默的「什么都没发生」。
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("任务 {task} 已取消")]
    Cancelled { task: &'static str },
    #[error("任务 {task} 的模型调用失败：{reason}")]
    Provider { task: &'static str, reason: String },
    #[error("任务 {task} 没有产出任何提议：{reason}")]
    Incomplete { task: &'static str, reason: String },
    #[error("任务 {task} 之后无法继续：{reason}")]
    FollowUp { task: &'static str, reason: String },
}

impl TaskError {
    pub fn task(&self) -> &'static str {
        match self {
            TaskError::Cancelled { task }
            | TaskError::Provider { task, .. }
            | TaskError::Incomplete { task, .. }
            | TaskError::FollowUp { task, .. } => task,
        }
    }
}

/// 把工具参数解成「模型要填的那个结构体」。
///
/// 报错照抄 `diagnostics::draft_from_tool_args` 的做法：**列出模型实际返回了哪些字段**。
/// 只报 `missing field 'x'` 的话，用户既不知道模型给了什么，也不知道能做什么。
pub(crate) fn decode<T: DeserializeOwned>(tool: &str, args: &Value) -> Result<T, ToolError> {
    let received: Vec<String> = args
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    serde_json::from_value(args.clone()).map_err(|error| {
        let seen = if received.is_empty() {
            "模型没有返回任何字段".to_owned()
        } else {
            format!("模型实际返回了：{}", received.join(" / "))
        };
        ToolError::invalid_arguments(format!(
            "{tool} 的参数无法解析：{error}（{seen}；可重试一次，或换用更严格的结构模型）"
        ))
    })
}

/// 编译一个视图并把它的状态序列化成提示词里的一段 JSON。
pub(crate) fn view_json(projection: &Projection, request: &ViewRequest) -> String {
    let compiled = if_views::compile(projection, request);
    serde_json::to_string(&compiled.state).unwrap_or_else(|_| "{}".to_owned())
}

/// 把一段 JSON 包成带标签的块，让模型知道这是**数据**而不是指令。
pub(crate) fn view_block(title: &str, json: &str) -> String {
    format!("<{title}>\n{json}\n</{title}>")
}

#[cfg(test)]
pub(crate) fn schema_properties(tool: &if_agent::ToolSpec) -> Vec<String> {
    let mut names: Vec<String> = tool.schema["properties"]
        .as_object()
        .expect("工具 schema 必须有 properties")
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) fn schema_required(tool: &if_agent::ToolSpec) -> Vec<String> {
    let mut names: Vec<String> = tool.schema["required"]
        .as_array()
        .expect("工具 schema 必须有 required")
        .iter()
        .map(|value| value.as_str().expect("required 必须是字符串数组").to_owned())
        .collect();
    names.sort();
    names
}
