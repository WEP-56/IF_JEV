//! T-extract：正文回收（docs/04 第 12 步、docs/05 §4）。
//!
//! 玩家在读正文的时候，后台从**已经展示的**节拍里抽出「实际上发生了什么」，
//! 交给 [`crate::commit::reconcile`] 与预演对账（docs/02 §11）。
//! 对账的规矩是：正文里确实发生了的变化，锁定等级按最强者保留——
//! 所以这里抽错了会被 IF 的 `L2` 盖过去，而抽漏了会让 IF 的作用悄悄失效。
//!
//! ⚠️ **v1 只抽事实**（`ObservedChange`）。docs/05 §4 列的三类——事实、声称、新主体——
//! 里后两类在领域层没有对应的对账路径（`commit` 只处理命题变化），
//! 所以工具 schema 里只声明事实这一种，而不是声明三种再丢掉两种。
//! `q.extract.faithful` / `q.extract.kind` 的 Jev 校对同样还没接：
//! 判定模板表（[`crate::question`]）里目前没有 `q.extract.*`。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

use if_agent::{AgentEvent, Provider, ToolError, ToolOutput, ToolSpec};
use if_domain::narrative::Beat;
use if_domain::projection::Projection;
use if_domain::turn::ViewKind;
use if_domain::value::Value as DomainValue;

use crate::beats::BeatProposal;
use crate::commit::ObservedChange;

use super::host::{run_proposal_task, TaskInputs, TaskKind, Workspace};
use super::resolver::Resolver;
use super::{decode, view_block, view_json, TaskPrompt};

pub const TOOL: &str = "record_observation";

/// 模型要填的那部分回收结果。与工具 schema **集合相等**。
///
/// `value` 是一个 JSON 标量——schema 里声明成 `["boolean","number","string"]`，
/// 本地校验器支持 `type` 写成数组（docs/05 §3.1），所以不必拆成三个字段再让模型猜该填哪个。
#[derive(Debug, Deserialize, Serialize)]
struct ModelObservation {
    prop: String,
    value: Value,
    internal: bool,
    text: String,
}

pub fn tool() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: "登记正文里实际发生的一处改变。每处改变一次调用；没有改变就不要调用。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "prop": {
                    "type": "string",
                    "description": "改变落在哪个命题上：写命题键（视图里 facts[].key 的值）。视图里没有的键收不进来——**不要编造**，找不到对应命题就别记这一条。"
                },
                "value": {
                    "type": ["boolean", "number", "string"],
                    "description": "这个命题现在的值。真/假写 true|false，程度写数字，文本写字符串。"
                },
                "internal": {
                    "type": "boolean",
                    "description": "true = 这是内在变化（心理、认知、态度）；false = 外在变化（行为、世界状态）。"
                },
                "text": {
                    "type": "string",
                    "minLength": 1,
                    "description": "正文里对应的原话，一到两句，用于回看核对。"
                }
            },
            "required": ["prop", "value", "internal", "text"],
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
    let model: ModelObservation = decode(TOOL, args)?;

    let prop = resolver.proposition(&model.prop).ok_or_else(|| {
        ToolError::invalid_arguments(format!(
            "命题 {:?} 不在视图里。请只回收视图里已有的命题；不确定就别记这一条。",
            model.prop
        ))
    })?;

    let value = match &model.value {
        Value::Bool(flag) => DomainValue::Bool(*flag),
        Value::Number(number) => DomainValue::Number(number.as_f64().unwrap_or_default()),
        Value::String(text) => DomainValue::Text(text.clone()),
        other => {
            return Err(ToolError::invalid_arguments(format!(
                "value 只能是 true/false、数字或字符串，收到的是 {other}"
            )))
        }
    };

    out.observations.push(ObservedChange {
        prop,
        value,
        internal: model.internal,
        text: model.text.trim().to_owned(),
    });
    Ok(ToolOutput::text(format!(
        "已回收第 {} 条观察。",
        out.observations.len()
    )))
}

/// 组装 T-extract 的提示词：检查视图 + 已经展示的节拍原文。
pub fn prompt(
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    displayed: &[BeatProposal],
) -> TaskPrompt {
    let system = "你是正文回收器。\n\
        读已经展示给玩家的正文，如实记下**实际上发生了**的改变。\n\
        \n\
        规则：\n\
        - 只记正文里真的写出来、已经发生的；打算做、想做、可能做都不算。\n\
        - 每处改变一次 record_observation；一条都没有就不要调用工具，直接说明「没有可回收的变化」。\n\
        - prop 必须是视图里已有的命题键；找不到对应命题就别记。\n\
        - text 抄正文里的原话，不要改写。\n\
        - 不要判断这件事重不重要、也不要补正文里没写的因果。"
        .to_owned();

    let rendered: String = displayed
        .iter()
        .map(|beat| format!("［第 {} 拍］{}", beat.index, beat.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let json = view_json(
        projection,
        &ctx.view(ViewKind::Check).task("T-extract").beat_text(rendered.clone()),
    );

    TaskPrompt::new(
        system,
        format!(
            "已展示的正文：\n{rendered}\n\n{}",
            view_block("检查视图", &json)
        ),
    )
}

/// 跑一次 T-extract。`displayed` 是**已经放行**的节拍。
///
/// 一场戏什么都没改时返回空表，且**不算失败**（[`TaskKind::allows_empty`]）——
/// 人只是说了句话，那就是没有可回收的变化。
pub fn run(
    provider: &dyn Provider,
    projection: &Projection,
    ctx: &crate::context::TurnContext,
    displayed: &[Beat],
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<(Vec<ObservedChange>, u32), super::TaskError> {
    if displayed.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let beats: Vec<BeatProposal> = displayed
        .iter()
        .map(|beat| BeatProposal::new(beat.index, beat.text.clone()))
        .collect();
    let resolver = Resolver::new(projection);
    let task_prompt = prompt(projection, ctx, &beats);
    let outcome = run_proposal_task(
        provider,
        TaskKind::Extract,
        task_prompt,
        &resolver,
        TaskInputs {
            accepted_candidates: Vec::new(),
            displayed_beats: beats.len() as u32,
            first_scene: 0,
        },
        cancel,
        emit,
    )?;
    Ok((outcome.workspace.observations, outcome.rounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::assert_schema_matches_struct;

    fn sample() -> Value {
        json!({
            "prop": "c_lin.evidence",
            "value": false,
            "internal": false,
            "text": "信已经不在她手上了。"
        })
    }

    #[test]
    fn the_tool_schema_matches_the_struct_the_model_fills() {
        assert_schema_matches_struct::<ModelObservation>(&tool(), &sample());
    }

    #[test]
    fn an_observation_lands_on_the_proposition_it_names() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let output = accept(&sample(), &resolver, &mut out, &TaskInputs::default()).unwrap();
        assert_eq!(output.model_text, "已回收第 1 条观察。");
        let observed = &out.observations[0];
        assert_eq!(observed.prop.as_str(), "p_lin_evidence");
        assert_eq!(observed.value, DomainValue::Bool(false));
        assert!(!observed.internal);
        assert_eq!(observed.text, "信已经不在她手上了。");
    }

    #[test]
    fn all_three_scalar_shapes_survive_the_mapping() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        for (raw, expected) in [
            (json!(true), DomainValue::Bool(true)),
            (json!(0.7), DomainValue::Number(0.7)),
            (json!("雨停了"), DomainValue::Text("雨停了".into())),
        ] {
            let mut args = sample();
            args["value"] = raw;
            accept(&args, &resolver, &mut out, &TaskInputs::default()).unwrap();
            assert_eq!(out.observations.last().unwrap().value, expected);
        }
    }

    /// 抽到一个视图里没有的命题，说明模型在编。要当场拒掉并告诉它怎么改，
    /// 而不是把它当成一条「世界之外的变化」写进对账。
    #[test]
    fn an_invented_proposition_is_rejected_with_a_fixable_message() {
        let projection = crate::testsupport::projection();
        let resolver = Resolver::new(&projection);
        let mut out = Workspace::default();

        let mut args = sample();
        args["prop"] = json!("c_lin.something_new");
        let error = accept(&args, &resolver, &mut out, &TaskInputs::default()).unwrap_err();
        assert!(error.message.contains("不在视图里"), "{}", error.message);
        assert!(out.observations.is_empty());
    }
}
