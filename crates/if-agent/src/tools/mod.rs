//! 工具基础类型。与 Onemore 的区别：
//!
//! - [`Tool`] 对执行上下文 `C` 泛型化：IF 的上下文是"视图读取句柄 + 提议效果记录"，
//!   由 `if-pipeline` 定义，这里不预设 workspace 或 plan（docs/05 §3.1）。
//! - 新增 [`ToolErrorCode::JudgeRejected`]，与 `HookRejected` 区分（docs/05 §6）。
//! - schema 校验补上 `items`、`minItems`、`maxItems`（见 [`schema`]）。

pub mod schema;

use serde_json::Value;

/// 单个工具结果的上限（docs/05 §5.1）。
pub const RESULT_MAX_CHARS: usize = 24_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
    /// 只读工具可以在 scoped threads 中并行执行。
    pub parallel_safe: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// 唯一进入 ToolResult 和模型上下文的正文。
    pub model_text: String,
    /// 不进入模型的结构化信息。
    pub details: Option<Value>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        ToolOutput { model_text: text.into(), details: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorCode {
    UnknownTool,
    InvalidArguments,
    NotFound,
    TruncatedInput,
    Aborted,
    /// 提议未通过 Jev 审查。说明文字受视角隔离约束，不得说出秘密（docs/05 §2.5）。
    JudgeRejected,
    HookRejected,
    ExecutionFailed,
    Internal,
}

impl ToolErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolErrorCode::UnknownTool => "unknown_tool",
            ToolErrorCode::InvalidArguments => "invalid_arguments",
            ToolErrorCode::NotFound => "not_found",
            ToolErrorCode::TruncatedInput => "truncated_input",
            ToolErrorCode::Aborted => "aborted",
            ToolErrorCode::JudgeRejected => "judge_rejected",
            ToolErrorCode::HookRejected => "hook_rejected",
            ToolErrorCode::ExecutionFailed => "execution_failed",
            ToolErrorCode::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        ToolError { code, message: message.into(), details: None }
    }

    pub fn invalid_arguments(message: impl Into<String>) -> Self {
        ToolError::new(ToolErrorCode::InvalidArguments, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        ToolError::new(ToolErrorCode::NotFound, message)
    }

    pub fn judge_rejected(message: impl Into<String>) -> Self {
        ToolError::new(ToolErrorCode::JudgeRejected, message)
    }

    pub fn execution(message: impl Into<String>) -> Self {
        ToolError::new(ToolErrorCode::ExecutionFailed, message)
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ToolError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub output: ToolOutput,
    pub error: Option<ToolError>,
}

impl ToolOutcome {
    pub fn success(output: ToolOutput) -> Self {
        normalize(ToolOutcome { output, error: None })
    }

    pub fn failure(error: ToolError) -> Self {
        normalize(ToolOutcome {
            output: ToolOutput { model_text: error.message.clone(), details: error.details.clone() },
            error: Some(error),
        })
    }

    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

fn normalize(mut outcome: ToolOutcome) -> ToolOutcome {
    if let Some(error) = outcome.error.as_mut() {
        error.message = bound(&error.message);
        outcome.output.model_text = error.message.clone();
    } else {
        outcome.output.model_text = bound(&outcome.output.model_text);
    }
    outcome
}

/// 超长时保留头尾、截掉中间。
fn bound(text: &str) -> String {
    let count = text.chars().count();
    if count <= RESULT_MAX_CHARS {
        return text.to_owned();
    }
    let keep = RESULT_MAX_CHARS / 2 - 32;
    let head: String = text.chars().take(keep).collect();
    let tail: String = text.chars().skip(count - keep).collect();
    format!("{head}\n…[已截断 {} 字符]…\n{tail}", count - 2 * keep)
}

/// 工具。`Send + Sync`：只读工具可能在 scoped threads 中并发执行。
pub trait Tool<C: ?Sized>: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn execute(&self, args: &Value, ctx: &mut C) -> Result<ToolOutput, ToolError>;
}

pub struct ToolRegistry<C: ?Sized> {
    tools: Vec<Box<dyn Tool<C>>>,
}

impl<C: ?Sized> std::fmt::Debug for ToolRegistry<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.tools.iter().map(|t| t.spec().name)).finish()
    }
}

impl<C: ?Sized> Default for ToolRegistry<C> {
    fn default() -> Self {
        ToolRegistry { tools: Vec::new() }
    }
}

impl<C: ?Sized> ToolRegistry<C> {
    pub fn new(tools: Vec<Box<dyn Tool<C>>>) -> Self {
        let registry = ToolRegistry { tools };
        debug_assert!(registry.names_unique(), "工具名必须唯一");
        registry
    }

    pub fn push(&mut self, tool: Box<dyn Tool<C>>) {
        self.tools.push(tool);
        debug_assert!(self.names_unique(), "工具名必须唯一");
    }

    fn names_unique(&self) -> bool {
        let mut names: Vec<String> = self.tools.iter().map(|t| t.spec().name).collect();
        let len = names.len();
        names.sort();
        names.dedup();
        names.len() == len
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool<C>> {
        self.tools.iter().find(|t| t.spec().name == name).map(|t| t.as_ref())
    }

    /// 查找工具并校验参数。非法参数永远到不了 `execute`（docs/12 §8）。
    pub fn prepare(&self, name: &str, args: &Value) -> Result<(&dyn Tool<C>, ToolSpec), ToolError> {
        let Some(tool) = self.get(name) else {
            let available: Vec<String> = self.tools.iter().map(|t| t.spec().name).collect();
            return Err(ToolError::new(
                ToolErrorCode::UnknownTool,
                format!("未知工具 {name:?}。可用工具：{}", available.join("、")),
            ));
        };
        let spec = tool.spec();
        if let Value::String(raw) = args {
            return Err(ToolError::invalid_arguments(format!(
                "工具 {} 的参数不是合法 JSON：{}",
                spec.name,
                raw.chars().take(200).collect::<String>()
            )));
        }
        if let Err(errors) = schema::validate(&spec.schema, args) {
            return Err(ToolError {
                code: ToolErrorCode::InvalidArguments,
                message: format!("工具 {} 参数校验失败：{}", spec.name, errors.join("；")),
                details: Some(serde_json::json!({ "errors": errors })),
            });
        }
        Ok((tool, spec))
    }

    pub fn execute(&self, name: &str, args: &Value, ctx: &mut C) -> ToolOutcome {
        match self.prepare(name, args) {
            Ok((tool, _)) => match tool.execute(args, ctx) {
                Ok(output) => ToolOutcome::success(output),
                Err(error) => ToolOutcome::failure(error),
            },
            Err(error) => ToolOutcome::failure(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Echo;
    impl Tool<Vec<String>> for Echo {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "echo".into(),
                description: "回显".into(),
                schema: json!({
                    "type": "object",
                    "properties": { "items": { "type": "array", "items": { "type": "string" }, "minItems": 1 } },
                    "required": ["items"],
                    "additionalProperties": false
                }),
                parallel_safe: true,
            }
        }
        fn execute(&self, args: &Value, ctx: &mut Vec<String>) -> Result<ToolOutput, ToolError> {
            ctx.push(args.to_string());
            Ok(ToolOutput::text("ok"))
        }
    }

    #[test]
    fn schema_failures_never_reach_execute() {
        let registry: ToolRegistry<Vec<String>> = ToolRegistry::new(vec![Box::new(Echo)]);
        let mut calls = Vec::new();
        for bad in [json!({}), json!({"items": []}), json!({"items": [1]}), json!({"items": ["a"], "x": 1}), json!("{broken")] {
            let outcome = registry.execute("echo", &bad, &mut calls);
            assert_eq!(outcome.error.unwrap().code, ToolErrorCode::InvalidArguments, "{bad}");
        }
        assert!(calls.is_empty());
        let ok = registry.execute("echo", &json!({"items": ["a"]}), &mut calls);
        assert!(!ok.is_error());
        assert_eq!(calls.len(), 1);
        assert_eq!(registry.execute("nope", &json!({}), &mut calls).error.unwrap().code, ToolErrorCode::UnknownTool);
    }

    #[test]
    fn long_output_is_bounded() {
        let out = ToolOutcome::success(ToolOutput::text("字".repeat(RESULT_MAX_CHARS * 2)));
        assert!(out.output.model_text.chars().count() <= RESULT_MAX_CHARS);
        assert!(out.output.model_text.contains("已截断"));
    }
}
