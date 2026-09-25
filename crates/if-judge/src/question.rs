use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::JudgeError;

/// Score 的最大等级数（docs/07 §1）。
pub const MAX_SCORE_LEVELS: usize = 10;

/// 一个问题：请求内的键 + 模板出处 + 原语参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// 请求内唯一的键，也是响应 `answers` 的键，如 `cand_004.notices`。
    pub key: String,
    /// 模板 ID@版本，如 `q.perception.notices@1`。
    pub template: String,
    /// 被判定的候选或对象 ID，写进判定记录。
    pub target: String,
    pub spec: QuestionSpec,
}

/// 原语参数。`choice` 与 `score` 的 criteria 形态相反（docs/14 §2），
/// 所以用两个不同的类型承载，而不是一个字段装两种形态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionSpec {
    Noul {
        instructions: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        true_means: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        false_means: Option<String>,
    },
    Choice { instructions: String, criteria: ChoiceCriteria },
    Score { instructions: String, criteria: ScoreCriteria },
}

/// 选项 ID → 选项说明。序列化为 JSON 对象。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChoiceCriteria(pub BTreeMap<String, String>);

/// 有序量表，索引 0 基。序列化为 JSON 数组。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScoreCriteria(pub Vec<String>);

impl QuestionSpec {
    pub fn noul(instructions: impl Into<String>) -> Self {
        QuestionSpec::Noul { instructions: instructions.into(), true_means: None, false_means: None }
    }

    pub fn choice<K: Into<String>, V: Into<String>>(
        instructions: impl Into<String>,
        options: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        QuestionSpec::Choice {
            instructions: instructions.into(),
            criteria: ChoiceCriteria(options.into_iter().map(|(k, v)| (k.into(), v.into())).collect()),
        }
    }

    pub fn score<S: Into<String>>(instructions: impl Into<String>, levels: impl IntoIterator<Item = S>) -> Self {
        QuestionSpec::Score {
            instructions: instructions.into(),
            criteria: ScoreCriteria(levels.into_iter().map(Into::into).collect()),
        }
    }

    pub fn instructions(&self) -> &str {
        match self {
            QuestionSpec::Noul { instructions, .. }
            | QuestionSpec::Choice { instructions, .. }
            | QuestionSpec::Score { instructions, .. } => instructions,
        }
    }

    pub fn primitive(&self) -> if_domain::Primitive {
        match self {
            QuestionSpec::Noul { .. } => if_domain::Primitive::Noul,
            QuestionSpec::Choice { .. } => if_domain::Primitive::Choice,
            QuestionSpec::Score { .. } => if_domain::Primitive::Score,
        }
    }
}

impl Question {
    /// 本地预校验，避免 400 白跑往返（docs/14 §7.2）。
    pub fn validate(&self) -> Result<(), JudgeError> {
        let path = |field: &str| format!("questions.{}.{field}", self.key);
        if self.key.trim().is_empty() {
            return Err(JudgeError::invalid("questions", "问题键为空"));
        }
        if self.spec.instructions().trim().is_empty() {
            return Err(JudgeError::invalid(path("instructions"), "instructions 为空"));
        }
        match &self.spec {
            QuestionSpec::Noul { .. } => {}
            QuestionSpec::Choice { criteria, .. } => {
                if criteria.0.len() < 2 {
                    return Err(JudgeError::invalid(path("criteria"), "choice 至少需要 2 个选项"));
                }
                if criteria.0.keys().any(|k| k.trim().is_empty()) {
                    return Err(JudgeError::invalid(path("criteria"), "choice 选项 ID 为空"));
                }
            }
            QuestionSpec::Score { criteria, .. } => {
                let n = criteria.0.len();
                if !(2..=MAX_SCORE_LEVELS).contains(&n) {
                    return Err(JudgeError::invalid(
                        path("criteria"),
                        format!("score 需要 2–{MAX_SCORE_LEVELS} 级，实际 {n} 级"),
                    ));
                }
            }
        }
        Ok(())
    }
}
