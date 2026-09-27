//! T-scenes：场景候选（docs/04 第 8 步、docs/05 §4）。
//!
//! 产物直接喂 [`crate::scenes::choose`]。模型在这里的职责是**给出可选的路**，
//! 不是选路：受保护故事线的硬否决与导演评分的骰子都在引擎那一侧。
//!
//! ⚠️ **v1 缺一项**：docs/05 §4 要求每个场景带「走向类别」（顺势 / 逆转 / 慢热 /
//! 第三方介入 / 升级），完成条件是「覆盖至少 3 类」。`SceneProposal` 没有这个字段，
//! 而它属于领域类型——补它要先改 `if-domain` 并同步全部构造点。当前只在提示词里
//! 要求「尽量覆盖不同走向」，完成条件退化为「候选数 ≥ 1」。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

use if_agent::{AgentEvent, Provider, ToolError, ToolOutput, ToolSpec};
use if_domain::projection::Projection;
use if_domain::turn::{Candidate, ViewKind};

use crate::scenes::SceneProposal;

use super::host::{run_proposal_task, TaskInputs, TaskKind, TaskOutcome, Workspace};
use super::resolver::Resolver;
use super::{decode, view_block, view_json, TaskPrompt};

pub const TOOL: &str = "propose_scene";

/// 模型要填的那部分场景候选。与工具 schema **集合相等**。
///
/// 同样不含 `id`：场景 ID 由引擎发号（`scene_0001`…），并且它得和第一段
/// [`crate::scenes::choose`] 记下来的那个 ID 一致——详见 [`crate::turn::resolve`] 的校验。
#[derive(Debug, Deserialize, Serialize)]
struct ModelScene {
    summary: String,
    threads: Vec<String>,
    resolves: Vec<String>,
    erupts: Vec<String>,
    focus: Vec<String>,
    present: Vec<String>,
}

pub fn tool() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: "登记一个场景候选：接下来这一场戏可以怎么演。每次调用只登记一个。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "minLength": 1,
                    "description": "这个场景演什么：一句话，写清楚谁在哪儿做什么。"
                },
                "threads": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "本场景推进的故事线 ID（视图里 threads 的键）。没有就留空数组。"
                },
                "resolves": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "本场景会**收束**的故事线 ID。受保护的故事线会被引擎直接否决，别硬写。"
                },
                "erupts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "本场景中爆发的趋势 ID。没有就留空数组。"
                },
                "focus": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "焦点角色名。一到两个。"
                },
                "present": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "在场角色名，至少一个。焦点角色必须在场。"
                }
            },
            "required": ["summary", "threads", "resolves", "erupts", "focus", "present"],
            "additionalProperties": false
        }),
        parallel_safe: false,
    }
}

pub(crate) fn accept(
    args: &Value,
    resolver: &Resolver<'_>,
    out: &mut Workspace,
    _inputs: &TaskInputs,
) -> Result<ToolOutput, ToolError> {
    let model: ModelScene = decode(TOOL, args)?;
    let id = out.next_scene_id();

    let mut notes: Vec<String> = Vec::new();
    let threads = resolver.threads(&model.threads);
    let resolves = resolver.threads(&model.resolves);
    let erupts = resolver.tendencies(&model.erupts);
    let focus = resolver.subjects(&model.focus);
    let present = resolver.subjects(&model.present);

    for (label, unknown) in [
        ("推进的故事线", &threads.unknown),
        ("收束的故事线", &resolves.unknown),
        ("爆发的趋势", &erupts.unknown),
        ("焦点角色", &focus.unknown),
        ("在场角色", &present.unknown),
    ] {
        if !unknown.is_empty() {
            notes.push(format!("场景 {id} 的{label} {} 不在世界里，已丢弃", unknown.join("、")));
        }
    }
    if present.ids.is_empty() {
        notes.push(format!("场景 {id} 没有任何在场角色，演不起来"));
    }
    for subject in &focus.ids {
        if !present.ids.contains(subject) {
            notes.push(format!("场景 {id} 的焦点角色 {subject} 不在场"));
        }
    }

    out.warnings.extend(notes);
    out.scenes.push(
        SceneProposal::new(id.clone(), model.summary.trim())
            .threads(threads.ids)
            .resolves(resolves.ids)
            .erupts(erupts.ids)
            .cast(focus.ids, present.ids),
    );

    Ok(ToolOutput::text(format!("已登记为 {id}。")))
}

/// 组装 T-scenes 的提示词。`accepted` 是第一段裁决放行的候选——
/// 场景要「实现」的那些预演变化，模型得先看见它们。
pub fn prompt(
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    accepted: &[Candidate],
) -> TaskPrompt {
    let system = "你是 IF 世界的导演。\n\
        给定当前世界状态与本回合已经确定会发生的事，提出**几个都能演、方向不同**的场景候选。\n\
        \n\
        规则：\n\
        - 每个候选一场戏：写清楚谁在哪儿、做什么、这一场推进或收束哪条线。\n\
        - 候选之间要**走向不同**（顺势推进、突然逆转、慢慢升温、第三方介入、事态升级……），\n\
          不要给四个差不多的版本。\n\
        - 每次调用 propose_scene 只登记一个。\n\
        - 焦点角色必须在场。\n\
        - 受保护的故事线不许收束；写进 resolves 会被直接否决。\n\
        - 不要选路、不要排优先级——选哪一条由引擎掷骰决定。"
        .to_owned();

    let mut request = ctx.view(ViewKind::Director).task("T-scenes");
    if let Some(input) = ctx.user_input.as_deref() {
        request = request.user_input(input);
    }
    let json = view_json(projection, &request);

    let mut head = String::new();
    if accepted.is_empty() {
        head.push_str("本回合没有任何候选被裁决放行；场景可以只推进既有故事线。\n\n");
    } else {
        head.push_str("本回合已经确定会发生的事：\n");
        for candidate in accepted {
            let who = candidate
                .subject
                .as_ref()
                .and_then(|subject| projection.subjects.get(subject))
                .map(|subject| subject.name.as_str())
                .unwrap_or("世界");
            let kind = if candidate.internal { "内在" } else { "外在" };
            head.push_str(&format!("- {}（{kind}）：{}\n", who, candidate.content));
        }
        head.push('\n');
    }

    TaskPrompt::new(system, format!("{head}{}", view_block("世界视图", &json)))
}

/// 跑一次 T-scenes。`accepted` 是裁决放行的候选——场景要演的就是它们。
///
/// `first_scene` 是场景 ID 的发号起点：世界里已经有几场戏（`TurnContext::scene_index`）。
/// 场景 ID 会进投影，重号就是覆盖，所以起点不能每回合都从头数。
pub fn run(
    provider: &dyn Provider,
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    accepted: &[Candidate],
    first_scene: u64,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<TaskOutcome, super::TaskError> {
    let resolver = Resolver::new(projection);
    let task_prompt = prompt(projection, ctx, accepted);
    run_proposal_task(
        provider,
        TaskKind::Scenes,
        task_prompt,
        &resolver,
        TaskInputs {
            first_scene,
            ..TaskInputs::default()
        },
        cancel,
        emit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::assert_schema_matches_struct;

    fn sample() -> Value {
        json!({
            "summary": "雨夜宫门外，林夏把信交给顾言",
            "threads": ["thr_shield"],
            "resolves": [],
            "erupts": [],
            "focus": ["林夏"],
            "present": ["林夏", "顾言"]
        })
    }

    #[test]
    fn the_tool_schema_matches_the_struct_the_model_fills() {
        assert_schema_matches_struct::<ModelScene>(&tool(), &sample());
    }

    #[test]
    fn names_are_resolved_and_ids_are_issued_by_the_engine() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let output = accept(&sample(), &resolver, &mut out, &TaskInputs::default()).unwrap();
        assert_eq!(output.model_text, "已登记为 scene_0001。");
        let scene = &out.scenes[0];
        assert_eq!(scene.id.as_str(), "scene_0001");
        assert_eq!(scene.threads.len(), 1);
        assert_eq!(scene.present.len(), 2);
        assert_eq!(scene.focus.len(), 1);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn a_scene_that_names_nobody_present_is_flagged() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["present"] = json!([]);
        args["focus"] = json!(["查无此人"]);
        accept(&args, &resolver, &mut out, &TaskInputs::default()).unwrap();

        let warnings = out.warnings.join("\n");
        assert!(warnings.contains("没有任何在场角色"), "{warnings}");
        assert!(warnings.contains("查无此人"), "{warnings}");
    }

    /// 焦点不在场要报出来：`ScenePlan::validate` 在 T-plan 那一侧会硬拦，
    /// 但那时已经白跑一轮模型了，早一步提醒便宜得多。
    #[test]
    fn a_focus_subject_outside_the_cast_is_flagged() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["present"] = json!(["林夏"]);
        args["focus"] = json!(["顾言"]);
        accept(&args, &resolver, &mut out, &TaskInputs::default()).unwrap();
        assert!(out.warnings.join("\n").contains("不在场"), "{:?}", out.warnings);
    }

    /// 场景 ID 从调用方给的起点往后发，**不是**每回合都从 `scene_0001` 重来。
    ///
    /// 重来的后果不是「难看」而是「丢数据」：场景 ID 是 `Projection.scenes` 的键，
    /// 也是每条事件 `scene` 字段的值——第二场戏会把第一场覆盖掉，投影里只剩一场。
    /// 所以这条契约必须钉住。
    #[test]
    fn scene_ids_start_where_the_world_left_off() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::starting_at(2);
        let inputs = TaskInputs::default();

        accept(&sample(), &resolver, &mut out, &inputs).unwrap();
        assert_eq!(out.scenes[0].id.as_str(), "scene_0003");
        // 同一回合里的第二个提议接着往下发，不撞号。
        accept(&sample(), &resolver, &mut out, &inputs).unwrap();
        assert_eq!(out.scenes[1].id.as_str(), "scene_0004");
    }
}
