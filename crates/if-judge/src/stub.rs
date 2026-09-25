use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use if_domain::{JudgmentOutput, JudgmentUsage};

use crate::{Judge, JudgeBackend, JudgeError, JudgeRequest, JudgeResponse, Question, QuestionSpec};

/// 按脚本返回固定判定的测试桩（docs/12 §8）。
///
/// 匹配顺序：问题键 → 模板 ID（不含版本）→ 默认值。未配置的 noul 返回 0.5，
/// choice 平均分布并选第一个选项，score 返回中间等级。
#[derive(Debug, Default)]
pub struct StubJudge {
    by_key: BTreeMap<String, JudgmentOutput>,
    by_template: BTreeMap<String, JudgmentOutput>,
    missing: Vec<String>,
    log: Mutex<Vec<JudgeRequest>>,
}

impl StubJudge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn noul_for_key(mut self, key: impl Into<String>, p: f64) -> Self {
        self.by_key.insert(key.into(), JudgmentOutput::Noul { noul: p });
        self
    }

    /// `template` 不含版本，如 `q.beat.violates_fact`。
    pub fn noul_for_template(mut self, template: impl Into<String>, p: f64) -> Self {
        self.by_template.insert(template.into(), JudgmentOutput::Noul { noul: p });
        self
    }

    pub fn output_for_key(mut self, key: impl Into<String>, out: JudgmentOutput) -> Self {
        self.by_key.insert(key.into(), out);
        self
    }

    pub fn output_for_template(mut self, template: impl Into<String>, out: JudgmentOutput) -> Self {
        self.by_template.insert(template.into(), out);
        self
    }

    /// 让某个问题键在响应里缺失，用来测试 docs/06 §9 的降级。
    pub fn drop_key(mut self, key: impl Into<String>) -> Self {
        self.missing.push(key.into());
        self
    }

    /// 已收到的请求，按顺序。
    pub fn requests(&self) -> Vec<JudgeRequest> {
        self.log.lock().expect("stub log poisoned").clone()
    }

    fn answer(&self, q: &Question) -> JudgmentOutput {
        let template = q.template.split('@').next().unwrap_or(&q.template);
        if let Some(out) = self.by_key.get(&q.key).or_else(|| self.by_template.get(template)) {
            return out.clone();
        }
        match &q.spec {
            QuestionSpec::Noul { .. } => JudgmentOutput::Noul { noul: 0.5 },
            QuestionSpec::Choice { criteria, .. } => {
                let p = 1.0 / criteria.0.len() as f64;
                JudgmentOutput::Choice {
                    choice: criteria.0.keys().next().cloned().unwrap_or_default(),
                    probabilities: criteria.0.keys().map(|k| (k.clone(), p)).collect(),
                    confidence: 0.5,
                }
            }
            QuestionSpec::Score { criteria, .. } => {
                let mid = (criteria.0.len() - 1) / 2;
                JudgmentOutput::Score {
                    score: mid as f64,
                    legend: criteria.0.iter().enumerate().map(|(i, s)| (i.to_string(), s.clone())).collect(),
                    probabilities: (0..criteria.0.len())
                        .map(|i| (i.to_string(), if i == mid { 1.0 } else { 0.0 }))
                        .collect(),
                    confidence: 1.0,
                }
            }
        }
    }
}

impl Judge for StubJudge {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError> {
        request.validate()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(JudgeError::Cancelled);
        }
        self.log.lock().expect("stub log poisoned").push(request.clone());
        let raw = request
            .questions
            .iter()
            .filter(|q| !self.missing.contains(&q.key))
            .map(|q| (q.key.clone(), self.answer(q)))
            .collect();
        Ok(JudgeResponse::assemble(request, "stub".to_owned(), raw, JudgmentUsage::default(), 0))
    }

    fn backend(&self) -> JudgeBackend {
        JudgeBackend::Stub
    }
}
