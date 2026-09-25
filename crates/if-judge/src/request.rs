use std::collections::{BTreeMap, BTreeSet};

use if_domain::{Judgment, JudgmentId, JudgmentOutput, JudgmentUsage, TurnId, ViewRef};
use serde::{Deserialize, Serialize};

use crate::{JudgeError, Question};

/// 已编译的视图：state 本体加上视图元数据（种类、持有者、hash）。
/// 编译与 hash 由 `if-views` 负责；这里只承载。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledView {
    pub meta: ViewRef,
    /// 发给判定者的 state：字符串、对象或数组，内容为文本（docs/14 §2）。
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeRequest {
    pub view: CompiledView,
    pub questions: Vec<Question>,
}

impl JudgeRequest {
    pub fn new(view: CompiledView) -> Self {
        JudgeRequest { view, questions: Vec::new() }
    }

    pub fn push(&mut self, question: Question) -> &mut Self {
        self.questions.push(question);
        self
    }

    pub fn validate(&self) -> Result<(), JudgeError> {
        if self.questions.is_empty() {
            return Err(JudgeError::invalid("questions", "至少需要一个问题"));
        }
        let mut seen = BTreeSet::new();
        for q in &self.questions {
            q.validate()?;
            if !seen.insert(q.key.as_str()) {
                return Err(JudgeError::invalid(format!("questions.{}", q.key), "问题键重复"));
            }
        }
        Ok(())
    }

    pub fn question(&self, key: &str) -> Option<&Question> {
        self.questions.iter().find(|q| q.key == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JudgeBackend {
    Jev { version: String },
    Llm { model: String },
    Stub,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeResponse {
    /// 响应回显的固定快照版本（docs/14 §1）。
    pub model: String,
    /// 问题键 → 判定。只收录原语与问题一致的答案。
    pub answers: BTreeMap<String, JudgmentOutput>,
    /// 缺失或原语不符的问题键。策略层按 docs/06 §9 处理：约束类视为不通过，发生类视为不发生。
    pub missing: Vec<String>,
    pub usage: JudgmentUsage,
    pub latency_ms: u64,
}

impl JudgeResponse {
    /// 把答案对齐回问题，并按 `answers` 中的原语过滤；不在请求里的键丢弃。
    pub(crate) fn assemble(
        request: &JudgeRequest,
        model: String,
        raw: BTreeMap<String, JudgmentOutput>,
        usage: JudgmentUsage,
        latency_ms: u64,
    ) -> Self {
        let mut answers = BTreeMap::new();
        let mut missing = Vec::new();
        let mut raw = raw;
        for q in &request.questions {
            match raw.remove(&q.key) {
                Some(out) if out.primitive() == q.spec.primitive() => {
                    answers.insert(q.key.clone(), out);
                }
                _ => missing.push(q.key.clone()),
            }
        }
        JudgeResponse { model, answers, missing, usage, latency_ms }
    }

    /// 生成判定记录（docs/07 §5）。请求级的 usage 与耗时记在第一条上，其余记 0，
    /// 这样按记录求和不会重复计费。
    pub fn to_judgments(
        &self,
        request: &JudgeRequest,
        turn: &TurnId,
        mut next_id: impl FnMut() -> JudgmentId,
    ) -> Vec<Judgment> {
        let mut first = true;
        request
            .questions
            .iter()
            .filter_map(|q| {
                let output = self.answers.get(&q.key)?.clone();
                let (usage, latency_ms) = if std::mem::take(&mut first) {
                    (self.usage, self.latency_ms)
                } else {
                    (JudgmentUsage::default(), 0)
                };
                Some(Judgment {
                    id: next_id(),
                    turn: turn.clone(),
                    template: q.template.clone(),
                    target: q.target.clone(),
                    view: request.view.meta.clone(),
                    model: self.model.clone(),
                    output,
                    usage,
                    latency_ms,
                })
            })
            .collect()
    }
}
