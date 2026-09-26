//! 连通性诊断：在回合流程接入之前，让用户用真实的 LLM 与 Jev 验证配置（AGENTS.md：真实验证由用户执行）。
//!
//! 所有调用都在阻塞线程里跑；流式增量通过 `diag://llm` 事件推给前端。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use if_agent::{build_provider, Block, ChatMessage, PromptContext, Provider, ProviderEvent, ProviderSettings, StreamTerminal, ToolSpec};
use if_domain::{JudgmentOutput, ViewKind, ViewRef};
use if_judge::{CompiledView, JevJudge, Judge, JudgeError, JudgeRequest, LlmJudge, Question};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::if_parser::{IfDraft, ParsedIfKind, ParsedTimeAnchor};

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

/// 模型要填的那部分 IF 草案——**这份结构就是工具 schema 的对应物**。
///
/// 刻意**不含**三样东西，它们都不该问模型：
///
/// - `input`：用户的原始输入。问模型要，拿回来的是改写过措辞的复述，
///   而裁定卡上要显示的正是用户自己写下的那句话——引擎回填。
/// - `suggested_lock`：docs/01 §6 是「类型 → 等级」的**确定性表**，
///   由 `kind` 经 `ParsedIfKind::default_lock` 推导。真机上模型在这里答错过
///   （规则型给出了 `L3`，而表里是 `L2`）。
/// - `rewrite_candidates`：只有确定性解析器会产；模型路径留空（前端目前也不渲染）。
///
/// 于是有一条可以精确到**集合相等**的不变式：schema 的 `properties` ≡ `ModelDraft` 的字段。
/// 由 `the_tool_schema_matches_the_struct_the_model_fills` 盯着。
#[derive(Debug, Serialize, Deserialize)]
struct ModelDraft {
    normalized: String,
    is_directive: bool,
    kind: ParsedIfKind,
    time_anchor: ParsedTimeAnchor,
    scope: String,
    core: String,
    non_commitments: Vec<String>,
    warnings: Vec<String>,
}

/// 结构模型要填的 IF 草案 schema。
///
/// ⚠️ **这份 schema 必须与 `ModelDraft` 逐字段对齐**：少一个字段，模型就永远不会
/// 返回它，反序列化当场失败，而用户看到的是一句「missing field `xxx`」——
/// 只有开发者看得懂、也不知道能做什么。真机上发生过一次（`input` 缺席）。
/// 由 `the_tool_schema_matches_the_struct_the_model_fills` 盯着。
///
/// 每个字段都带 `description`，`kind` / `time_anchor` / `scope` 还带 `enum`——
/// 这不是装饰：第一版一个说明都没有，模型只能猜，于是规则型猜出 `L3`、
/// 「所有人」猜成 `individual`。**枚举值就是契约，把含义写在模型看得见的地方。**
fn if_draft_tool() -> ToolSpec {
    ToolSpec {
        name: "submit_if_draft".into(),
        description: "把用户输入解释为一条待确认的 IF 裁定卡。忠实解释，允许自然语言，不要推演后果；不要复述用户输入，也不要自行决定锁定等级。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "normalized": { "type": "string", "description": "把用户输入规整成一句以「IF」开头的断言。只调整措辞，不要添加后果。" },
                "is_directive": { "type": "boolean", "description": "输入是「让某人做某事」这类导演指令（而非断言）时为 true。" },
                "kind": {
                    "type": "string",
                    "enum": ["state", "belief", "rule", "occurrence", "truth", "retcon", "unknown"],
                    "description": "state 状态型（主体此刻的状态 / 关系 / 意图）· belief 认知型（此刻相信什么，事实不变）· rule 规则型（世界从现在起如何运作）· occurrence 事件型（刚发生或正在发生）· truth 真相型（一直为真，但未必有人知道）· retcon 回溯型（与已展示的过去矛盾）。判据见 docs/01 §5；拿不准写 unknown。"
                },
                "time_anchor": { "type": "string", "enum": ["now", "past", "always"], "description": "now 从此刻起 · past 已经发生 · always 一直如此。" },
                "scope": { "type": "string", "enum": ["individual", "group", "region", "global"], "description": "作用范围（docs/01 §5）：individual 单个主体 · group 一群人 · region 一个地域 · global 整个世界——含「所有人」「全人类」这类说法。" },
                "core": { "type": "string", "description": "这条 IF 的核心命题，一句话。不要展开后果。" },
                "non_commitments": { "type": "array", "items": { "type": "string" }, "description": "这条 IF **不承诺**的后果，逐条列出。" },
                "warnings": { "type": "array", "items": { "type": "string" }, "description": "解释上的歧义、边界不明确、判定标准不明之处，逐条列出。" }
            },
            "required": ["normalized", "is_directive", "kind", "time_anchor", "scope", "core", "non_commitments", "warnings"],
            "additionalProperties": false
        }),
        parallel_safe: false,
    }
}

/// 把模型返回的工具参数补成一份完整的 `IfDraft`。
///
/// 补的三样都是**引擎才有资格决定**的：`input`（用户原话）、`suggested_lock`
/// （docs/01 §6 的表）、`rewrite_candidates`（确定性解析器的事）。
fn draft_from_tool_args(args: Value, input: &str) -> Result<IfDraft, String> {
    let received: Vec<String> = args
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    let model: ModelDraft = serde_json::from_value(args).map_err(|error| {
        let seen = if received.is_empty() {
            "模型没有返回任何字段".to_owned()
        } else {
            format!("模型实际返回了：{}", received.join(" / "))
        };
        format!("结构模型返回的 IF 草案无效：{error}（{seen}；可重试一次，或换用更严格的结构模型）")
    })?;
    Ok(IfDraft {
        input: input.to_owned(),
        is_directive: model.is_directive,
        normalized: model.normalized,
        suggested_lock: model.kind.default_lock().to_owned(),
        kind: model.kind,
        time_anchor: model.time_anchor,
        scope: model.scope,
        core: model.core,
        non_commitments: model.non_commitments,
        warnings: model.warnings,
        rewrite_candidates: Vec::new(),
    })
}

pub fn parse_if_with_model(settings: ProviderSettings, input: String, cancel: Arc<AtomicBool>) -> Result<IfDraft, String> {
    if settings.model.trim().is_empty() {
        return Err("结构模型尚未配置，无法进行 IF 解析".into());
    }
    let provider = build_provider(settings);
    parse_if_with(provider.as_ref(), input, cancel)
}

/// 真正干活的那一层：只认 `Provider`，与「provider 从哪来」解耦。
///
/// 拆出来是为了能测**整条路径**（发问 → 读 `tool_uses()` → 补齐 `input` → 反序列化）：
/// 真机上炸的正是这一段，只测 `draft_from_tool_args` 抓不到它。测试用
/// `if_agent::provider::scripted::ScriptedProvider`，不联网、不花额度。
fn parse_if_with(provider: &dyn Provider, input: String, cancel: Arc<AtomicBool>) -> Result<IfDraft, String> {
    let prompt = PromptContext { system_sections: vec!["你是 IF 世界的结构解析器。忠实解释用户输入，允许自然语言，不要擅自添加后果。必须调用 submit_if_draft。".into()], messages: vec![ChatMessage::user_text(input.clone())] };
    let mut events = |_event: ProviderEvent| {};
    let output = match provider.stream_turn(&prompt, &[if_draft_tool()], &mut events, &cancel) {
        StreamTerminal::Done(output) => output,
        StreamTerminal::Aborted(_) => return Err("IF 解析已取消".into()),
        StreamTerminal::Error(error) => return Err(error.error.message),
    };
    let (_, _, args) = output.message.tool_uses().into_iter().next().ok_or_else(|| "结构模型未调用 submit_if_draft".to_owned())?;
    draft_from_tool_args(args.clone(), &input)
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

#[cfg(test)]
mod tests {
    use super::*;
    use if_agent::provider::scripted::{ScriptedProvider, ScriptedTurn};

    fn never() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// 模型**按 schema 如实返回**的一份参数。注意它里面**没有** `input`、
    /// 也没有 `suggested_lock`——这两个引擎自己填，schema 里根本没有它们。
    fn schema_clean_args() -> Value {
        json!({
            "normalized": "IF 所有人从此无法说谎",
            "is_directive": false,
            "kind": "rule",
            "time_anchor": "now",
            "scope": "global",
            "core": "所有人无法说谎",
            "non_commitments": ["角色是否意识到这条规则"],
            "warnings": []
        })
    }

    /// 真机回归：`input` 不在工具 schema 里，模型当然不会返回它，而
    /// `IfDraft::input` 是必填字段——第一版直接 `from_value`，于是**每一次**
    /// `IF xxx` 都失败在「missing field `input`」，裁定卡永远出不来。
    #[test]
    fn a_model_draft_that_omits_input_still_parses() {
        let draft = draft_from_tool_args(schema_clean_args(), "IF 所有人从此无法说谎")
            .expect("引擎应当补上 input 再解析");
        assert_eq!(draft.input, "IF 所有人从此无法说谎");
        assert_eq!(draft.core, "所有人无法说谎");
        assert_eq!(draft.kind, ParsedIfKind::Rule);
        // 锁定等级由 `kind` 推出（docs/01 §6），不是模型给的。
        assert_eq!(draft.suggested_lock, "L2");
        assert_eq!(draft.rewrite_candidates, Vec::<String>::new());
    }

    /// 锁定等级是「类型 → 等级」的全函数（docs/01 §6），引擎自己算。
    /// 真机上规则型拿到过 `L3`——而 `L2` 与 `L3` 在冲突让位时规则不同。
    #[test]
    fn the_lock_level_comes_from_the_kind_not_from_the_model() {
        for (kind, expected) in [
            ("state", "L1"),
            ("belief", "L1"),
            ("rule", "L2"),
            ("occurrence", "L3"),
            ("truth", "L3"),
            ("retcon", "L3"),
            ("unknown", "L0"),
        ] {
            let mut args = schema_clean_args();
            args["kind"] = json!(kind);
            let draft = draft_from_tool_args(args, "IF x").unwrap();
            assert_eq!(draft.suggested_lock, expected, "类型 {kind} 默认锁定等级");
        }
    }

    /// 模型没资格决定锁定等级：它连字段都没得填（`additionalProperties: false`），
    /// 就算多嘴塞一个也不作数。
    #[test]
    fn a_lock_level_smuggled_in_by_the_model_is_ignored() {
        let mut args = schema_clean_args();
        args["suggested_lock"] = json!("L3");
        let draft = draft_from_tool_args(args, "IF x").unwrap();
        assert_eq!(draft.suggested_lock, "L2", "规则型就该是 L2，模型说了不算");
    }

    /// `input` 是用户的原话，不是模型该复述的东西：模型改写措辞也只当没看见。
    #[test]
    fn the_engine_keeps_the_users_own_words_over_whatever_the_model_echoes() {
        let mut args = schema_clean_args();
        args["input"] = json!("IF 所有人都不许说谎");
        let draft = draft_from_tool_args(args, "IF 所有人从此无法说谎").unwrap();
        assert_eq!(draft.input, "IF 所有人从此无法说谎");
    }

    /// 其它字段真缺了，报错要说清楚「模型给了什么」——用户至少知道能重试 / 换模型。
    #[test]
    fn a_missing_field_reports_what_the_model_did_send() {
        let error = draft_from_tool_args(json!({ "normalized": "IF x", "is_directive": false }), "IF x")
            .unwrap_err();
        assert!(error.contains("missing field"), "应当指出缺了哪个字段：{error}");
        assert!(error.contains("normalized"), "应当列出模型实际返回的字段：{error}");
        assert!(error.contains("重试"), "应当给出用户可以做的动作：{error}");
    }

    /// 加字段时的护栏，而且现在是**集合相等**、没有豁免表：
    /// 工具 schema 的 `properties` 必须与 `ModelDraft` 的字段**完全相同**。
    ///
    /// - schema 少一个 → 模型永远不返回它 → 反序列化当场失败（真机出过一次）；
    /// - schema 多一个 → 模型照着返回会被 `additionalProperties: false` 挡下；
    /// - 引擎自己填的那三个（`input` / `suggested_lock` / `rewrite_candidates`）
    ///   压根不该出现在 schema 里——不是「可以不填」，是**不归模型管**。
    #[test]
    fn the_tool_schema_matches_the_struct_the_model_fills() {
        let tool = if_draft_tool();
        let properties = tool.schema["properties"].as_object().expect("schema 必须有 properties");
        let required: Vec<&str> = tool.schema["required"]
            .as_array()
            .expect("schema 必须有 required")
            .iter()
            .map(|value| value.as_str().expect("required 必须是字符串数组"))
            .collect();

        let sample = serde_json::to_value(ModelDraft {
            normalized: "IF 所有人从此无法说谎".into(),
            is_directive: false,
            kind: ParsedIfKind::Rule,
            time_anchor: ParsedTimeAnchor::Now,
            scope: "global".into(),
            core: "所有人无法说谎".into(),
            non_commitments: vec![],
            warnings: vec![],
        })
        .unwrap();
        let fields: Vec<String> = sample.as_object().unwrap().keys().cloned().collect();

        for field in &fields {
            assert!(properties.contains_key(field), "`ModelDraft::{field}` 不在工具 schema 里——模型永远不会返回它");
            assert!(required.contains(&field.as_str()), "`{field}` 在 schema 里但没进 required——模型可能不返回它");
        }
        for declared in properties.keys() {
            assert!(fields.contains(declared), "工具 schema 声明了 `ModelDraft` 没有的字段 `{declared}`");
        }
        // 非必填字段会逼调用方处理「模型没给」的分支，而 `additionalProperties: false`
        // 的 strict 模式要求两者一致——所以这里要求 properties 与 required 完全同一集合。
        assert_eq!(properties.len(), required.len(), "schema 里不应有非必填字段");

        for engine_owned in ["input", "suggested_lock", "rewrite_candidates"] {
            assert!(!properties.contains_key(engine_owned), "`{engine_owned}` 归引擎管，不该向模型索要");
        }

        // 每个字段都要有 `description`——第一版一个都没有，模型只能靠猜，
        // 于是「所有人」猜成 `individual`、规则型猜成 `L3`。
        for (name, spec) in properties {
            assert!(
                spec["description"].as_str().is_some_and(|text| !text.trim().is_empty()),
                "工具 schema 的 `{name}` 缺少 description——模型只能猜"
            );
        }
    }

    // ---- 整条模型路径：发问 → 读 tool_uses → 补齐 input → 反序列化 ----
    // 真机上炸的就是这一段。只测 `draft_from_tool_args` 覆盖不到「调用方怎么拿到 args」，
    // 所以这里真的走一遍 provider。

    #[test]
    fn the_whole_model_path_turns_a_tool_call_into_a_draft() {
        let provider = ScriptedProvider::new([ScriptedTurn::tool("submit_if_draft", schema_clean_args())]);
        let draft = parse_if_with(&provider, "IF 所有人从此无法说谎".into(), never()).expect("真机上的主路径");
        assert_eq!(draft.input, "IF 所有人从此无法说谎");
        assert_eq!(draft.core, "所有人无法说谎");
        assert_eq!(provider.prompts().len(), 1, "应当恰好发问一次");
    }

    /// 模型只说了话、没调工具 → 报「未调用」，而不是拿一个空 args 去反序列化。
    #[test]
    fn a_model_that_never_calls_the_tool_says_so() {
        let provider = ScriptedProvider::new([ScriptedTurn::text("我觉得这条更像规则。")]);
        let error = parse_if_with(&provider, "IF x".into(), never()).unwrap_err();
        assert!(error.contains("submit_if_draft"), "{error}");
    }

    /// 上游挂了就把上游的报错透出去——不要伪装成「草案无效」，那会把排查方向带偏。
    #[test]
    fn a_provider_failure_is_not_reported_as_a_bad_draft() {
        let provider = ScriptedProvider::new([ScriptedTurn::Error { message: "上游 502".into(), retryable: true }]);
        assert_eq!(parse_if_with(&provider, "IF x".into(), never()).unwrap_err(), "上游 502");
    }

    /// 已经取消的任务不该再解析结果。
    #[test]
    fn a_cancelled_run_stops_before_the_draft_is_built() {
        let provider = ScriptedProvider::new([ScriptedTurn::tool("submit_if_draft", schema_clean_args())]);
        assert_eq!(parse_if_with(&provider, "IF x".into(), Arc::new(AtomicBool::new(true))).unwrap_err(), "IF 解析已取消");
    }
}
