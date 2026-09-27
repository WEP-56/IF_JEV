//! T-impact：影响候选（docs/04 第 6 步、docs/05 §4）。
//!
//! 输入是**已经锁定的 IF** 与当前投影；产物是候选，直接喂
//! [`crate::candidates::adjudicate`]。模型只提出「可能发生什么」，
//! 「到底会不会发生」由引擎掷骰（docs/06）——两件事分开，才有得复现。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

use if_agent::{AgentEvent, Provider, ToolError, ToolOutput, ToolSpec};
use if_domain::projection::Projection;
use if_domain::turn::{Candidate, CandidateShape, ViewKind};

use super::host::{run_proposal_task, TaskInputs, TaskKind, TaskOutcome, Workspace};
use super::resolver::Resolver;
use super::{decode, view_block, view_json, TaskPrompt};

pub const TOOL: &str = "propose_candidate";

/// 模型要填的那部分候选。**集合相等**：schema 的 `properties` 就是这些字段。
///
/// 刻意**不含三样**：
///
/// - `id`：候选 ID 由引擎发号，写在工具回执里（docs/05 §5.2）。
///   让模型自己编号，两条世界线上就会给出不同的号，而候选 ID 是回合内分配的。
/// - `key`（稳定决策键）：它必须跨世界线不变，而模型每次的措辞都可能不一样。
///   [`Candidate::decision_key`] 已经有一条确定性回退——用首个被影响命题的规范键，
///   那比让模型复述一遍可靠得多。
/// - 概率：命运骰子只认世界种子与决策键，模型说什么都不进这道算式。
#[derive(Debug, Deserialize, Serialize)]
struct ModelCandidate {
    subject: String,
    content: String,
    internal: bool,
    shape: String,
    options: Vec<String>,
    depends_on: Vec<String>,
    based_on: Vec<String>,
    affects: Vec<String>,
}

pub fn tool() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: "登记一条影响候选：这条 IF 会引出的一件具体的事。每次调用只登记一条。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "候选作用于谁：写角色名（视图里的名字）。世界级事件（天气、时间、场所）留空字符串。"
                },
                "content": {
                    "type": "string",
                    "minLength": 1,
                    "description": "一件具体的事，一句话。写「会发生什么」，不要写理由、不要写后果链。"
                },
                "internal": {
                    "type": "boolean",
                    "description": "true = 内在（心理、认知、态度、意图）；false = 外在（行为、世界状态）。"
                },
                "shape": {
                    "type": "string",
                    "enum": ["occurs", "exclusive"],
                    "description": "occurs = 这件事会不会发生；exclusive = 几种互斥的结果里哪一个发生（必须在 options 里列出至少两项）。"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "仅 exclusive 使用：互斥的选项，至少两项。shape 为 occurs 时留空数组。"
                },
                "depends_on": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "只有先发生它、这条才谈得上的候选 ID（如 cand_0001，从工具回执里读）。没有依赖就留空数组。"
                },
                "based_on": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "这条候选依据的认知或事实：写命题键（视图里 propositions[].key 的值）。判断者只能看这些。"
                },
                "affects": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "这条候选会改变哪个命题：写命题键。至少写一个——它同时是这条候选的稳定标识。"
                }
            },
            "required": ["subject", "content", "internal", "shape", "options", "depends_on", "based_on", "affects"],
            "additionalProperties": false
        }),
        parallel_safe: false,
    }
}

/// 收下一条候选。认不出的名字与 ID 记成警告而不是抛错——
/// 但**绝不静默丢掉**：那会让「模型写错了」看起来像「世界本来就没有」。
pub(crate) fn accept(
    args: &Value,
    resolver: &Resolver<'_>,
    out: &mut Workspace,
    _inputs: &TaskInputs,
) -> Result<ToolOutput, ToolError> {
    let model: ModelCandidate = decode(TOOL, args)?;
    let id = out.next_candidate_id();
    let known = out.candidate_ids();

    let mut notes: Vec<String> = Vec::new();

    let subject = if model.subject.trim().is_empty() {
        None
    } else {
        match resolver.subject(&model.subject) {
            Some(subject) => Some(subject),
            None => {
                notes.push(format!("候选 {id} 的主体 {:?} 不在世界里，按世界级事件处理", model.subject));
                None
            }
        }
    };

    let shape = match model.shape.as_str() {
        "exclusive" if model.options.len() >= 2 => CandidateShape::Exclusive {
            options: model.options.clone(),
        },
        "exclusive" => {
            notes.push(format!(
                "候选 {id} 声称是互斥组，却只给了 {} 个选项；按发生类处理",
                model.options.len()
            ));
            CandidateShape::Occurs
        }
        _ => CandidateShape::Occurs,
    };

    let depends_on: Vec<_> = model
        .depends_on
        .iter()
        .filter_map(|raw| {
            let id = if_domain::id::CandidateId::new(raw.trim());
            if known.contains(&id) {
                Some(id)
            } else {
                notes.push(format!("候选 {id} 依赖的 {:?} 不是本回合已登记的候选，已丢弃", raw));
                None
            }
        })
        .collect();

    let based_on = resolver.propositions(&model.based_on);
    let affects = resolver.propositions(&model.affects);

    if !based_on.unknown.is_empty() {
        notes.push(format!(
            "候选 {id} 依据的命题 {} 不在视图里，已丢弃",
            based_on.unknown.join("、")
        ));
    }
    if !affects.unknown.is_empty() {
        notes.push(format!(
            "候选 {id} 影响的命题 {} 不在视图里，已丢弃",
            affects.unknown.join("、")
        ));
    }
    if affects.ids.is_empty() {
        notes.push(format!("候选 {id} 没有指明影响哪个命题，它的稳定决策键会缺失"));
    }
    if model.content.trim().is_empty() {
        notes.push(format!("候选 {id} 没有内容"));
    }

    out.warnings.extend(notes);
    out.candidates.push(Candidate {
        id: id.clone(),
        key: None,
        subject,
        content: model.content.trim().to_owned(),
        internal: model.internal,
        shape,
        depends_on,
        based_on: based_on.ids,
        affects: affects.ids,
    });

    Ok(ToolOutput::text(format!("已登记为 {id}。")))
}

/// 组装 T-impact 的提示词：稳定前缀（系统段）在前，本回合的内容在后（docs/08）。
pub fn prompt(
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    input: Option<&str>,
) -> TaskPrompt {
    let system = "你是 IF 世界的影响推演器。\n\
        给定一条刚刚被用户锁定为世界前提的反事实断言，以及当前世界状态，\
        你要提出这条断言会引出的**具体候选**。\n\
        \n\
        规则：\n\
        - 一条候选 = 一件具体的事，一句话。不要写解释、不要写后果的后果。\n\
        - 每次调用 propose_candidate 只登记一条；多条就多次调用。\n\
        - 每个受强烈影响的主体至少给一条；世界层面的后果（时间、天气、场所）也要给。\n\
        - 几种结果**只能有一个发生**时用 exclusive，并在 options 里列出至少两项。\n\
          「会不会发生」这种二分用 occurs。\n\
        - 不要判断概率、不要决定哪些会发生——那是引擎的事。\n\
        - 不要编造视图里没有的角色或命题；依据与影响都写视图里的命题键。"
        .to_owned();

    let mut request = ctx.view(ViewKind::God).task("T-impact");
    if let Some(input) = input {
        request = request.user_input(input);
    }
    let json = view_json(projection, &request);

    let head = match input {
        Some(input) => format!("用户注入并已锁定的 IF：{input}\n\n"),
        None => "本回合没有新的 IF；世界按既有前提继续推演。\n\n".to_owned(),
    };
    let user = format!("{head}{}", view_block("世界视图", &json));
    TaskPrompt::new(system, user)
}

/// 跑一次 T-impact。
pub fn run(
    provider: &dyn Provider,
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    input: Option<&str>,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<TaskOutcome, super::TaskError> {
    let resolver = Resolver::new(projection);
    let task_prompt = prompt(projection, ctx, input);
    run_proposal_task(
        provider,
        TaskKind::Impact,
        task_prompt,
        &resolver,
        TaskInputs::default(),
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
            "subject": "林夏",
            "content": "林夏当晚去宫门找顾言",
            "internal": false,
            "shape": "occurs",
            "options": [],
            "depends_on": [],
            "based_on": ["c_lin.evidence"],
            "affects": ["c_lin.mood"]
        })
    }

    #[test]
    fn the_tool_schema_matches_the_struct_the_model_fills() {
        assert_schema_matches_struct::<ModelCandidate>(&tool(), &sample());
    }

    #[test]
    fn a_candidate_is_registered_with_an_engine_issued_id() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let output = accept(&sample(), &resolver, &mut out, &TaskInputs::default()).unwrap();
        assert_eq!(output.model_text, "已登记为 cand_0001。");
        assert_eq!(out.candidates.len(), 1);

        let candidate = &out.candidates[0];
        assert_eq!(candidate.id.as_str(), "cand_0001");
        assert_eq!(candidate.subject.as_ref().map(|s| s.as_str()), Some("c_lin"));
        // 决策键留给引擎推导：`affects` 里的命题键就是它的来源。
        assert_eq!(candidate.key, None);
        assert_eq!(candidate.affects.len(), 1);
        assert_eq!(resolver.key_of(&candidate.affects[0]).as_deref(), Some("c_lin.mood"));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);

        // 第二次调用拿到的是下一个号，不是复用。
        accept(&sample(), &resolver, &mut out, &TaskInputs::default()).unwrap();
        assert_eq!(out.candidates[1].id.as_str(), "cand_0002");
    }

    /// 认不出的名字、不属于本回合的依赖、只给一个选项的互斥组——都要留下痕迹。
    #[test]
    fn sloppy_proposals_are_recorded_as_warnings_not_silently_accepted() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["subject"] = json!("路人甲");
        args["shape"] = json!("exclusive");
        args["options"] = json!(["只给一项"]);
        args["depends_on"] = json!(["cand_0099"]);
        args["based_on"] = json!(["不存在的键"]);
        args["affects"] = json!([]);

        accept(&args, &resolver, &mut out, &TaskInputs::default()).unwrap();
        let warnings = out.warnings.join("\n");
        assert!(warnings.contains("路人甲"), "{warnings}");
        assert!(warnings.contains("按发生类处理"), "{warnings}");
        assert!(warnings.contains("cand_0099"), "{warnings}");
        assert!(warnings.contains("不存在的键"), "{warnings}");
        assert!(warnings.contains("稳定决策键会缺失"), "{warnings}");
        assert!(matches!(out.candidates[0].shape, CandidateShape::Occurs));
    }

    /// 互斥组真的会被原样收下——否则 docs/06 §1 的互斥类策略永远走不到。
    #[test]
    fn an_exclusive_candidate_keeps_its_options() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["shape"] = json!("exclusive");
        args["options"] = json!(["答应", "拒绝", "沉默"]);
        accept(&args, &resolver, &mut out, &TaskInputs::default()).unwrap();
        assert_eq!(
            out.candidates[0].options(),
            Some(["答应".to_owned(), "拒绝".to_owned(), "沉默".to_owned()].as_slice())
        );
    }
}
