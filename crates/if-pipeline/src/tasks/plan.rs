//! T-plan：场景计划（docs/04 第 10 步、docs/05 §4）。
//!
//! 夹在 [`crate::turn::open`] 与 [`crate::turn::resolve`] 之间——**顺序不能拧反**：
//! 场景要先选出来，才谈得上给它写计划（见 [`crate::turn`] 的模块说明）。
//!
//! 计划里有一项**不问模型**：`forbidden_resolutions`。它由
//! [`crate::turn::inject_constraints`] 从受保护故事线直接注入（docs/04 §2.1），
//! 问模型等于把硬约束变成建议——它会照着写、也会忘。这与 `suggested_lock` 是同一课。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

use if_agent::{AgentEvent, Provider, ToolError, ToolOutput, ToolSpec};
use if_domain::id::{CandidateId, SubjectId};
use if_domain::narrative::{RevealTarget, ScenePlan};
use if_domain::projection::Projection;
use if_domain::turn::{Candidate, ViewKind};

use crate::scenes::SceneProposal;

use super::host::{run_proposal_task, TaskInputs, TaskKind, Workspace};
use super::resolver::Resolver;
use super::{decode, view_block, view_json, TaskPrompt};

pub const TOOL: &str = "submit_scene_plan";

/// 模型要填的那部分场景计划。与工具 schema **集合相等**。
///
/// 不含 `forbidden_resolutions`（引擎注入）、不含 `proposed_changes` 的自由引用
/// （只能是本回合被裁决放行的候选 ID，见 [`accept`]）。
#[derive(Debug, Deserialize, Serialize)]
struct ModelPlan {
    goal: String,
    pov: String,
    focus: Vec<String>,
    present: Vec<String>,
    time_span: String,
    required_beats: Vec<String>,
    stop_condition: String,
    reveal_facts: Vec<String>,
    reveal_lore: Vec<String>,
    proposed_changes: Vec<String>,
}

pub fn tool() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: "提交这个场景的执行计划。计划只有一份，重交会覆盖上一份。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "minLength": 1,
                    "description": "这场戏要达到什么：一句话。"
                },
                "pov": {
                    "type": "string",
                    "description": "视角角色名。必须有这个人，而且他必须在 present 里。"
                },
                "focus": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "这场戏的焦点角色名（不是视角人物也可以）。"
                },
                "present": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "在场角色名，至少一个，且必须包含 pov。"
                },
                "time_span": {
                    "type": "string",
                    "description": "这场戏覆盖多长时间，例如「当晚」「三天后到清晨」。"
                },
                "required_beats": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "**必须写到**的节拍，按先后顺序，每条一句话。没有就留空数组。"
                },
                "stop_condition": {
                    "type": "string",
                    "minLength": 1,
                    "description": "写到什么程度这场戏就结束：一句话，可判定。"
                },
                "reveal_facts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "本场景允许揭示的事实：写命题键。没有就留空数组。"
                },
                "reveal_lore": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "本场景允许揭示的设定条目：写条目标题或 ID。没有就留空数组。"
                },
                "proposed_changes": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "本场景要体现哪些已经确定的候选：写候选 ID（如 cand_0001）。没被裁决放行的候选不能写。"
                }
            },
            "required": [
                "goal", "pov", "focus", "present", "time_span", "required_beats",
                "stop_condition", "reveal_facts", "reveal_lore", "proposed_changes"
            ],
            "additionalProperties": false
        }),
        parallel_safe: false,
    }
}

pub(crate) fn accept(
    args: &Value,
    resolver: &Resolver<'_>,
    out: &mut Workspace,
    inputs: &TaskInputs,
) -> Result<ToolOutput, ToolError> {
    let model: ModelPlan = decode(TOOL, args)?;

    let mut notes: Vec<String> = Vec::new();

    // 视角人物是硬约束：`ScenePlan::validate` 会拦，但在这里先拦一道，
    // 报错里能带上「这个人在不在世界里」这种模型改得动的信息。
    let pov = resolver.subject(&model.pov).ok_or_else(|| {
        ToolError::invalid_arguments(format!(
            "视角人物 {:?} 不在世界里。请从视图的 subjects 里挑一个名字。",
            model.pov
        ))
    })?;

    let focus = resolver.subjects(&model.focus);
    let present = resolver.subjects(&model.present);
    for (label, unknown) in [
        ("焦点角色", &focus.unknown),
        ("在场角色", &present.unknown),
    ] {
        if !unknown.is_empty() {
            notes.push(format!("计划里的{label} {} 不在世界里，已丢弃", unknown.join("、")));
        }
    }

    let reveal_facts = resolver.propositions(&model.reveal_facts);
    if !reveal_facts.unknown.is_empty() {
        notes.push(format!(
            "计划里允许揭示的命题 {} 不在视图里，已丢弃",
            reveal_facts.unknown.join("、")
        ));
    }
    let mut reveal_allowed: Vec<RevealTarget> = reveal_facts
        .ids
        .into_iter()
        .map(|prop| RevealTarget::Fact { prop })
        .collect();
    for raw in &model.reveal_lore {
        match resolver.lore(raw) {
            Some(lore) => reveal_allowed.push(RevealTarget::Lore { lore }),
            None => notes.push(format!("计划里允许揭示的设定条目 {raw:?} 不在视图里，已丢弃")),
        }
    }

    let mut proposed_changes: Vec<CandidateId> = Vec::new();
    for raw in &model.proposed_changes {
        let id = CandidateId::new(raw.trim());
        if inputs.accepted_candidates.contains(&id) {
            if !proposed_changes.contains(&id) {
                proposed_changes.push(id);
            }
        } else {
            notes.push(format!(
                "计划的 proposed_changes 引用了{id}，但本回合没有放行这个候选，已丢弃"
            ));
        }
    }

    let plan = ScenePlan {
        goal: model.goal.trim().to_owned(),
        pov,
        focus: focus.ids,
        present: present.ids,
        time_span: model.time_span.trim().to_owned(),
        required_beats: model
            .required_beats
            .iter()
            .map(|beat| beat.trim().to_owned())
            .filter(|beat| !beat.is_empty())
            .collect(),
        stop_condition: model.stop_condition.trim().to_owned(),
        // 引擎注入，不问模型（docs/04 §2.1）。
        forbidden_resolutions: Vec::new(),
        reveal_allowed,
        proposed_changes,
    };
    // 两条领域层硬校验：present 非空、视角人物在场。把领域层的错误原样交回给模型，
    // 它下一轮就知道该补什么——这比在引擎里悄悄补一个视角人物要好。
    plan.validate().map_err(|error| {
        ToolError::invalid_arguments(format!("场景计划不合规：{error}。请修正后重新提交。"))
    })?;

    out.warnings.extend(notes);
    out.plan = Some(plan);

    Ok(ToolOutput::text("已接受这份场景计划。"))
}

/// 组装 T-plan 的提示词：选中的场景 + 本回合确定会发生的事 + 导演视图。
pub fn prompt(
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    scene: &SceneProposal,
    accepted: &[Candidate],
) -> TaskPrompt {
    let system = "你是场景计划器。\n\
        在这一步，场景**已经选定**（不是你在选），你要把它写成一份可执行的计划。\n\
        \n\
        规则：\n\
        - pov 必须是真实存在的角色名，而且必须在 present 里；present 不能为空。\n\
        - required_beats 只写**必须写到**的节拍，按先后顺序。没有就留空。\n\
        - stop_condition 要可判定（「顾言把话说完」可以，「气氛变好」不行）。\n\
        - 允许揭示什么就写什么，其余内容正文里不许点破。\n\
        - proposed_changes 只能写上面列出的、已经确定会发生的那几个候选 ID。\n\
        - 不要写正文，不要写对白。"
        .to_owned();

    let mut request = ctx.view(ViewKind::Director).task("T-plan");
    // 计划是针对**这个**场景写的，所以视图的在场面收敛到场景提议给出的那一批
    // （和 `ViewRequest::scene_plan` 做的事一样，只是计划还没写出来）。
    if !scene.present.is_empty() {
        request.present = scene.present.clone();
    }
    if !scene.focus.is_empty() {
        request.focus = scene.focus.clone();
    }
    if let Some(input) = ctx.user_input.as_deref() {
        request = request.user_input(input);
    }
    let json = view_json(projection, &request);

    let mut head = format!("选定的场景：{}\n", scene.summary);
    if !scene.focus.is_empty() {
        head.push_str(&format!(
            "场景提议给出的焦点：{}\n",
            names(projection, &scene.focus)
        ));
    }
    if !accepted.is_empty() {
        head.push_str("\n本回合已经确定会发生的事（proposed_changes 只能引用这些 ID）：\n");
        for candidate in accepted {
            head.push_str(&format!("- {}：{}\n", candidate.id, candidate.content));
        }
    }
    TaskPrompt::new(system, format!("{head}\n{}", view_block("世界视图", &json)))
}

fn names(projection: &Projection, subjects: &[SubjectId]) -> String {
    subjects
        .iter()
        .map(|subject| crate::context::subject_name(projection, subject))
        .collect::<Vec<_>>()
        .join("、")
}

/// 跑一次 T-plan，返回**引擎注入硬约束之后**才算数的那份计划由 [`crate::turn::resolve`] 产出；
/// 这里给的是模型的提议。
pub fn run(
    provider: &dyn Provider,
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    scene: &SceneProposal,
    accepted: &[Candidate],
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<ScenePlan, super::TaskError> {
    let resolver = Resolver::new(projection);
    let task_prompt = prompt(projection, ctx, scene, accepted);
    let inputs = TaskInputs {
        accepted_candidates: accepted.iter().map(|candidate| candidate.id.clone()).collect(),
        displayed_beats: 0,
        first_scene: 0,
    };
    let outcome = run_proposal_task(
        provider,
        TaskKind::Plan,
        task_prompt,
        &resolver,
        inputs,
        cancel,
        emit,
    )?;
    outcome.workspace.plan.ok_or_else(|| super::TaskError::Incomplete {
        task: TaskKind::Plan.as_str(),
        reason: "工具报了成功，但工作区里没有计划".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::assert_schema_matches_struct;
    use if_domain::id::PropositionId;

    fn sample() -> Value {
        json!({
            "goal": "顾言说出他知道的事",
            "pov": "林夏",
            "focus": ["顾言"],
            "present": ["林夏", "顾言"],
            "time_span": "当晚",
            "required_beats": ["顾言开口", "林夏没有回答"],
            "stop_condition": "顾言把话说完",
            "reveal_facts": [],
            "reveal_lore": [],
            "proposed_changes": []
        })
    }

    fn inputs() -> TaskInputs {
        TaskInputs {
            accepted_candidates: vec![CandidateId::new("cand_0001")],
            displayed_beats: 0,
            first_scene: 0,
        }
    }

    #[test]
    fn the_tool_schema_matches_the_struct_the_model_fills() {
        assert_schema_matches_struct::<ModelPlan>(&tool(), &sample());
    }

    #[test]
    fn a_plan_is_accepted_and_the_engine_owns_the_forbidden_list() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let output = accept(&sample(), &resolver, &mut out, &inputs()).unwrap();
        assert_eq!(output.model_text, "已接受这份场景计划。");
        let plan = out.plan.expect("计划应当被收下");
        assert_eq!(plan.pov.as_str(), "c_lin");
        assert_eq!(plan.present.len(), 2);
        // 禁止项由 `inject_constraints` 注入；这里必须是空的，
        // 否则「模型可以自己决定禁止什么」会变成一条悄悄生效的规则。
        assert!(plan.forbidden_resolutions.is_empty());
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    /// 视角人物不在场是最常见的一类失败：要**原样回报领域层的错误**，
    /// 让模型下一轮自己补，而不是引擎替它塞一个人进去。
    #[test]
    fn a_plan_whose_pov_is_absent_is_rejected_with_a_fixable_message() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["present"] = json!(["顾言"]);
        let error = accept(&args, &resolver, &mut out, &inputs()).unwrap_err();
        assert!(error.message.contains("视角人物不在"), "{}", error.message);
        assert!(error.message.contains("重新提交"), "{}", error.message);
        assert!(out.plan.is_none(), "被拒的计划不该留在工作区里");
    }

    #[test]
    fn an_unknown_pov_is_rejected_before_the_domain_layer_sees_it() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["pov"] = json!("查无此人");
        let error = accept(&args, &resolver, &mut out, &inputs()).unwrap_err();
        assert!(error.message.contains("不在世界里"), "{}", error.message);
    }

    /// 引一个没被放行的候选，等于让正文去演一件骰子说不会发生的事。
    #[test]
    fn a_change_that_was_never_adjudicated_is_dropped() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["proposed_changes"] = json!(["cand_0001", "cand_0042"]);
        accept(&args, &resolver, &mut out, &inputs()).unwrap();
        let plan = out.plan.expect("计划");
        assert_eq!(plan.proposed_changes, vec![CandidateId::new("cand_0001")]);
        assert!(out.warnings.join("\n").contains("cand_0042"), "{:?}", out.warnings);
    }

    #[test]
    fn reveal_targets_are_split_into_facts_and_lore() {
        let projection = crate::testsupport::projection_with_lore();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["reveal_facts"] = json!(["c_gu.secret"]);
        args["reveal_lore"] = json!(["大乾", "查无此条"]);
        accept(&args, &resolver, &mut out, &inputs()).unwrap();
        let plan = out.plan.expect("计划");
        assert_eq!(
            plan.reveal_allowed,
            vec![
                RevealTarget::Fact { prop: PropositionId::new("p_gu_secret") },
                RevealTarget::Lore { lore: if_domain::id::LoreId::new("lore_world_1") },
            ]
        );
        assert!(out.warnings.join("\n").contains("查无此条"), "{:?}", out.warnings);
    }
}
