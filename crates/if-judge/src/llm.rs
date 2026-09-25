use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use if_agent::{ChatMessage, PromptContext, Provider, StreamTerminal};
use if_domain::{JudgmentOutput, JudgmentUsage};
use serde_json::Value;

use crate::{Judge, JudgeBackend, JudgeError, JudgeRequest, JudgeResponse, QuestionSpec};

const SYSTEM: &str = "你是一个判定器，只对给定的问题输出概率判断，不创作任何内容。\n\
state 是你能看到的全部信息；不要假设 state 以外的事实。\n\
每个问题独立判断，互不参考。\n\
只输出一个 JSON 对象，不要任何解释或代码块标记。对象的键是问题键，值按问题类型：\n\
- noul：{\"p\": 命题为真的概率，0 到 1}\n\
- choice：{\"probabilities\": {选项ID: 概率, …}}，覆盖全部选项，总和为 1\n\
- score：{\"probabilities\": {\"0\": 概率, \"1\": 概率, …}}，按等级索引，覆盖全部等级，总和为 1";

/// 用 LLM 充当判定者（D11：Jev 不可用时由用户手动切换）。
/// 判定记录里的 `model` 是该 LLM 的模型名，便于事后区分后端。
pub struct LlmJudge {
    provider: Box<dyn Provider>,
}

impl std::fmt::Debug for LlmJudge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmJudge").field("provider", &self.provider.label()).finish()
    }
}

impl LlmJudge {
    pub fn new(provider: Box<dyn Provider>) -> Self {
        LlmJudge { provider }
    }

    fn prompt(request: &JudgeRequest) -> PromptContext {
        let questions: serde_json::Map<String, Value> = request
            .questions
            .iter()
            .map(|q| (q.key.clone(), serde_json::to_value(&q.spec).expect("QuestionSpec 总能序列化")))
            .collect();
        let body = serde_json::json!({ "state": request.view.state, "questions": questions });
        PromptContext {
            system_sections: vec![SYSTEM.to_owned()],
            messages: vec![ChatMessage::user_text(body.to_string())],
        }
    }
}

impl Judge for LlmJudge {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError> {
        request.validate()?;
        let started = Instant::now();
        let output = match self.provider.stream_turn(&Self::prompt(request), &[], &mut |_| {}, cancel) {
            StreamTerminal::Done(output) => output,
            StreamTerminal::Aborted(_) => return Err(JudgeError::Cancelled),
            StreamTerminal::Error(failed) => {
                let message = failed.error.message;
                return Err(if failed.error.retryable {
                    JudgeError::Transient(message)
                } else if message.starts_with("HTTP 401") || message.starts_with("HTTP 403") {
                    JudgeError::Auth { status: message[5..8].parse().unwrap_or(401), message }
                } else {
                    JudgeError::Backend(message)
                });
            }
        };
        let latency_ms = started.elapsed().as_millis() as u64;
        let answers = parse_answers(&output.message.text())?;
        let raw = request
            .questions
            .iter()
            .filter_map(|q| Some((q.key.clone(), to_output(&q.spec, answers.get(&q.key)?)?)))
            .collect();
        let usage = JudgmentUsage {
            input_tokens: output.usage.input_tokens,
            output_tokens: output.usage.output_tokens,
            cost_usd: 0.0,
        };
        Ok(JudgeResponse::assemble(request, self.provider.model().to_owned(), raw, usage, latency_ms))
    }

    fn backend(&self) -> JudgeBackend {
        JudgeBackend::Llm { model: self.provider.model().to_owned() }
    }
}

/// 容忍代码块标记和前后的多余文字：取第一个 `{` 到最后一个 `}`。
fn parse_answers(text: &str) -> Result<serde_json::Map<String, Value>, JudgeError> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err(JudgeError::Malformed("LLM 判定输出里没有 JSON 对象".into()));
    };
    match serde_json::from_str::<Value>(&text[start..=end]) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(JudgeError::Malformed("LLM 判定输出不是 JSON 对象".into())),
        Err(e) => Err(JudgeError::Malformed(format!("LLM 判定输出 JSON 无效：{e}"))),
    }
}

/// 把一个答案转成与 Jev 同形的输出。不合规的答案返回 `None`，由策略层按缺失处理。
fn to_output(spec: &QuestionSpec, answer: &Value) -> Option<JudgmentOutput> {
    let unit = |v: &Value| v.as_f64().filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
    match spec {
        QuestionSpec::Noul { .. } => Some(JudgmentOutput::Noul { noul: unit(&answer["p"])? }),
        QuestionSpec::Choice { criteria, .. } => {
            let probabilities = normalized(criteria.0.keys().cloned(), &answer["probabilities"])?;
            let (choice, confidence) = probabilities
                .iter()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(k, p)| (k.clone(), *p))?;
            Some(JudgmentOutput::Choice { choice, probabilities, confidence })
        }
        QuestionSpec::Score { criteria, .. } => {
            let probabilities = normalized((0..criteria.0.len()).map(|i| i.to_string()), &answer["probabilities"])?;
            let score = probabilities.iter().map(|(k, p)| k.parse::<f64>().unwrap_or(0.0) * p).sum();
            let confidence = probabilities.values().copied().fold(0.0, f64::max);
            let legend = criteria.0.iter().enumerate().map(|(i, s)| (i.to_string(), s.clone())).collect();
            Some(JudgmentOutput::Score { score, legend, probabilities, confidence })
        }
    }
}

/// 读出全部键的概率并归一化；缺键记 0，全为 0 或含非法值则不合规。
fn normalized(keys: impl Iterator<Item = String>, raw: &Value) -> Option<BTreeMap<String, f64>> {
    let obj = raw.as_object()?;
    let mut out = BTreeMap::new();
    for k in keys {
        let p = match obj.get(&k) {
            None => 0.0,
            Some(v) => v.as_f64().filter(|p| p.is_finite() && *p >= 0.0)?,
        };
        out.insert(k, p);
    }
    let total: f64 = out.values().sum();
    if total <= 0.0 {
        return None;
    }
    out.values_mut().for_each(|p| *p /= total);
    Some(out)
}
