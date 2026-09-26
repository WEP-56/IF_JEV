//! 连通性诊断：在回合流程接入之前，让用户用真实的 LLM 与 Jev 验证配置（AGENTS.md：真实验证由用户执行）。
//!
//! 所有调用都在阻塞线程里跑；流式增量通过 `diag://llm` 事件推给前端。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use if_agent::{build_provider, Block, ChatMessage, PromptContext, ProviderEvent, ProviderSettings, StreamTerminal, ToolSpec};
use if_domain::{JudgmentOutput, ViewKind, ViewRef};
use if_judge::{CompiledView, JevJudge, Judge, JudgeError, JudgeRequest, LlmJudge, Question};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::if_parser::IfDraft;

#[derive(Debug, Clone, Serialize)]
pub struct LlmEvent {
    pub run_id: String,
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallView {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct LlmTestResult {
    pub label: String,
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<ToolCallView>,
    pub stop: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub first_token_ms: Option<u64>,
    pub latency_ms: u64,
}

/// 工具调用测试用的工具：结构任务全靠工具参数输出，这一项不通过，回合流程就跑不起来。
fn probe_tool() -> ToolSpec {
    ToolSpec {
        name: "submit_if_rewrite".into(),
        description: "提交一条 IF 形式的改写建议。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "minLength": 1, "description": "以「IF」开头的一句话断言" },
                "kind": { "type": "string", "enum": ["state", "belief", "rule", "event", "truth"] },
                "not_committed": { "type": "array", "items": { "type": "string" }, "description": "这条 IF 不承诺的后果" }
            },
            "required": ["text", "kind"],
            "additionalProperties": false
        }),
        parallel_safe: false,
    }
}

pub fn parse_if_with_model(settings: ProviderSettings, input: String, cancel: Arc<AtomicBool>) -> Result<IfDraft, String> {
    if settings.model.trim().is_empty() {
        return Err("结构模型尚未配置，无法进行 IF 解析".into());
    }
    let provider = build_provider(settings);
    let tool = ToolSpec {
        name: "submit_if_draft".into(),
        description: "把用户输入解释为一条待确认的 IF 裁定卡。不要推演后果。".into(),
        schema: json!({"type":"object","properties":{
            "normalized":{"type":"string"},"is_directive":{"type":"boolean"},
            "kind":{"type":"string","enum":["state","belief","rule","occurrence","truth","retcon","unknown"]},
            "time_anchor":{"type":"string","enum":["now","past","always"]},
            "scope":{"type":"string"},"suggested_lock":{"type":"string","enum":["L0","L1","L2","L3"]},
            "core":{"type":"string"},"non_commitments":{"type":"array","items":{"type":"string"}},
            "warnings":{"type":"array","items":{"type":"string"}}
        },"required":["normalized","is_directive","kind","time_anchor","scope","suggested_lock","core","non_commitments","warnings"],"additionalProperties":false}),
        parallel_safe: false,
    };
    let prompt = PromptContext { system_sections: vec!["你是 IF 世界的结构解析器。忠实解释用户输入，允许自然语言，不要擅自添加后果。必须调用 submit_if_draft。".into()], messages: vec![ChatMessage::user_text(input.clone())] };
    let mut events = |_event: ProviderEvent| {};
    let output = match provider.stream_turn(&prompt, &[tool], &mut events, &cancel) {
        StreamTerminal::Done(output) => output,
        StreamTerminal::Aborted(_) => return Err("IF 解析已取消".into()),
        StreamTerminal::Error(error) => return Err(error.error.message),
    };
    let (_, _, args) = output.message.tool_uses().into_iter().next().ok_or_else(|| "结构模型未调用 submit_if_draft".to_owned())?;
    serde_json::from_value::<IfDraft>(args.clone()).map_err(|error| format!("结构模型返回的 IF 草案无效：{error}"))
}

pub fn test_llm(
    settings: ProviderSettings,
    run_id: String,
    prompt: String,
    with_tool: bool,
    cancel: Arc<AtomicBool>,
    emit: impl Fn(LlmEvent),
) -> Result<LlmTestResult, String> {
    if settings.model.trim().is_empty() {
        return Err("还没有填写模型名".into());
    }
    let provider = build_provider(settings);
    let tools = if with_tool { vec![probe_tool()] } else { Vec::new() };
    let system = if with_tool {
        "你在测试工具调用。必须调用 submit_if_rewrite 工具提交结果，不要只用文字回答。"
    } else {
        "你是一个中文小说作者。"
    };
    let ctx = PromptContext { system_sections: vec![system.into()], messages: vec![ChatMessage::user_text(prompt)] };

    let started = Instant::now();
    let mut first_token_ms = None;
    let mut on_event = |e: ProviderEvent| {
        first_token_ms.get_or_insert_with(|| started.elapsed().as_millis() as u64);
        let (kind, text) = match e {
            ProviderEvent::TextDelta(t) => ("text", t),
            ProviderEvent::ThinkingDelta(t) => ("thinking", t),
            ProviderEvent::ToolCallBegun { name } => ("tool", name),
        };
        emit(LlmEvent { run_id: run_id.clone(), kind, text });
    };
    let output = match provider.stream_turn(&ctx, &tools, &mut on_event, &cancel) {
        StreamTerminal::Done(output) => output,
        StreamTerminal::Aborted(_) => return Err("已取消".into()),
        StreamTerminal::Error(failed) => return Err(failed.error.message),
    };
    let thinking = output
        .message
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let tool_calls = output
        .message
        .tool_uses()
        .into_iter()
        .map(|(_, name, args)| ToolCallView { name: name.into(), arguments: args.clone() })
        .collect();
    Ok(LlmTestResult {
        label: provider.label(),
        text: output.message.text(),
        thinking,
        tool_calls,
        stop: format!("{:?}", output.stop),
        input_tokens: output.usage.input_tokens,
        output_tokens: output.usage.output_tokens,
        cache_read_tokens: output.usage.cache.map(|c| c.read_tokens).unwrap_or(0),
        first_token_ms,
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct JudgeTestInput {
    pub state: Value,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeTestResult {
    pub backend: String,
    pub model: String,
    pub answers: std::collections::BTreeMap<String, JudgmentOutput>,
    pub missing: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms: u64,
}

fn request(input: JudgeTestInput) -> JudgeRequest {
    let view = CompiledView { meta: ViewRef { kind: ViewKind::God, holder: None, hash: "diag".into() }, state: input.state };
    JudgeRequest { view, questions: input.questions }
}

pub fn run_judge(judge: &dyn Judge, backend: &str, input: JudgeTestInput, cancel: &AtomicBool) -> Result<JudgeTestResult, String> {
    let resp = judge.judge(&request(input), cancel).map_err(describe)?;
    Ok(JudgeTestResult {
        backend: backend.into(),
        model: resp.model,
        answers: resp.answers,
        missing: resp.missing,
        input_tokens: resp.usage.input_tokens,
        output_tokens: resp.usage.output_tokens,
        cost_usd: resp.usage.cost_usd,
        latency_ms: resp.latency_ms,
    })
}

pub fn jev(settings: &crate::settings::JevSettings, key: String) -> JevJudge {
    JevJudge::new(settings.config(key))
}

pub fn llm_judge(settings: ProviderSettings) -> LlmJudge {
    LlmJudge::new(build_provider(settings))
}

fn describe(e: JudgeError) -> String {
    match e {
        JudgeError::Invalid { path, message } if !path.is_empty() => format!("请求不合规（{path}）：{message}"),
        other => other.to_string(),
    }
}
