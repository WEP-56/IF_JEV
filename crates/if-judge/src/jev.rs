use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use if_domain::{JudgmentOutput, JudgmentUsage};
use serde::{Deserialize, Serialize};

use crate::{Judge, JudgeBackend, JudgeError, JudgeRequest, JudgeResponse};

pub const DEFAULT_ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
/// 请求写别名；记录用响应回显的快照（docs/07 §6）。
pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";

#[derive(Clone)]
pub struct JevConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for JevConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevConfig")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl JevConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        JevConfig {
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            api_key: api_key.into(),
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub struct JevJudge {
    config: JevConfig,
    agent: ureq::Agent,
}

impl JevJudge {
    pub fn new(config: JevConfig) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(config.timeout).build();
        JevJudge { config, agent }
    }

    /// 探活：直接对 decisions 端点发一个最小 noul（docs/14 §7.8），不要用模型列表。
    pub fn probe(&self) -> Result<String, JudgeError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "state": "探活",
            "questions": { "ping": { "type": "noul", "instructions": "天是蓝的吗？" } }
        });
        Ok(self.post(&body)?.model)
    }

    fn post(&self, body: &serde_json::Value) -> Result<WireResponse, JudgeError> {
        let result = self
            .agent
            .post(&self.config.endpoint)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .send_json(body);
        match result {
            Ok(resp) => {
                let text = resp.into_string().map_err(|e| JudgeError::Transient(e.to_string()))?;
                serde_json::from_str(&text).map_err(|e| JudgeError::Malformed(e.to_string()))
            }
            Err(ureq::Error::Status(status, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                Err(classify_status(status, &text))
            }
            Err(ureq::Error::Transport(t)) => Err(JudgeError::Transient(t.to_string())),
        }
    }
}

impl Judge for JevJudge {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError> {
        request.validate()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(JudgeError::Cancelled);
        }
        let body = wire_request(&self.config.model, request);
        let started = Instant::now();
        let wire = self.post(&body)?;
        let latency_ms = started.elapsed().as_millis() as u64;
        // ureq 的阻塞调用无法中途打断；取消只能丢弃结果。
        if cancel.load(Ordering::Relaxed) {
            return Err(JudgeError::Cancelled);
        }
        Ok(wire.into_response(request, latency_ms))
    }

    fn backend(&self) -> JudgeBackend {
        JudgeBackend::Jev { version: self.config.model.clone() }
    }
}

pub(crate) fn wire_request(model: &str, request: &JudgeRequest) -> serde_json::Value {
    let questions: serde_json::Map<String, serde_json::Value> = request
        .questions
        .iter()
        .map(|q| (q.key.clone(), serde_json::to_value(&q.spec).expect("QuestionSpec 总能序列化")))
        .collect();
    serde_json::json!({
        "model": model,
        "state": request.view.state,
        "questions": questions,
    })
}

fn classify_status(status: u16, body: &str) -> JudgeError {
    let message = error_message(body);
    match status {
        400 | 422 => JudgeError::Invalid { path: error_path(body).unwrap_or_default(), message },
        401 | 403 => JudgeError::Auth { status, message },
        408 | 429 | 500..=599 => JudgeError::Transient(format!("HTTP {status}：{message}")),
        _ => JudgeError::Malformed(format!("意外的 HTTP {status}：{message}")),
    }
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(500).collect())
}

/// 从 zod 风格的错误里取出第一个 `path`，拼成 `questions.<id>.criteria`。
/// 错误体的具体嵌套未完全实测，这里宽松地在整棵 JSON 里找第一个字符串数组形态的 `path`。
fn error_path(body: &str) -> Option<String> {
    fn find(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::Array(parts)) = map.get("path") {
                    let joined: Vec<String> = parts
                        .iter()
                        .map(|p| match p {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    if !joined.is_empty() {
                        return Some(joined.join("."));
                    }
                }
                map.values().find_map(find)
            }
            serde_json::Value::Array(items) => items.iter().find_map(find),
            serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok().as_ref().and_then(find),
            _ => None,
        }
    }
    find(&serde_json::from_str(body).ok()?)
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireResponse {
    pub model: String,
    #[serde(default)]
    pub answers: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub usage: WireUsage,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cost: f64,
}

impl WireResponse {
    pub(crate) fn into_response(self, request: &JudgeRequest, latency_ms: u64) -> JudgeResponse {
        // 单个答案解析失败不连累整批：记为缺失，交给策略层（docs/06 §9）。
        let raw = self
            .answers
            .into_iter()
            .filter_map(|(k, v)| parse_answer(v).map(|out| (k, out)))
            .collect();
        let usage = JudgmentUsage {
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cost_usd: self.usage.cost,
        };
        JudgeResponse::assemble(request, self.model, raw, usage, latency_ms)
    }
}

fn parse_answer(v: serde_json::Value) -> Option<JudgmentOutput> {
    let out: JudgmentOutput = serde_json::from_value(v).ok()?;
    let in_unit = |p: f64| p.is_finite() && (0.0..=1.0).contains(&p);
    let ok = match &out {
        JudgmentOutput::Noul { noul } => in_unit(*noul),
        JudgmentOutput::Choice { choice, probabilities, confidence } => {
            probabilities.contains_key(choice) && in_unit(*confidence) && probabilities.values().all(|p| in_unit(*p))
        }
        JudgmentOutput::Score { score, confidence, probabilities, .. } => {
            score.is_finite() && *score >= 0.0 && in_unit(*confidence) && probabilities.values().all(|p| in_unit(*p))
        }
    };
    ok.then_some(out)
}
