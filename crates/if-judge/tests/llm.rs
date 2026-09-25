use std::sync::atomic::AtomicBool;

use if_agent::provider::scripted::{ScriptedProvider, ScriptedTurn};
use if_domain::{JudgmentOutput, ViewKind, ViewRef};
use if_judge::{CompiledView, Judge, JudgeBackend, JudgeError, JudgeRequest, LlmJudge, Question, QuestionSpec};

fn request() -> JudgeRequest {
    let mut r = JudgeRequest::new(CompiledView {
        meta: ViewRef { kind: ViewKind::God, holder: None, hash: "b3:x".into() },
        state: serde_json::json!("雨城"),
    });
    let q = |key: &str, spec| Question { key: key.into(), template: "t@1".into(), target: key.into(), spec };
    r.push(q("n", QuestionSpec::noul("会下雨吗？")));
    r.push(q("c", QuestionSpec::choice("做什么？", [("go", "去"), ("stay", "留")])));
    r.push(q("s", QuestionSpec::score("紧张？", ["低", "中", "高"])));
    r
}

fn judge(reply: &str) -> (LlmJudge, std::sync::Arc<ScriptedProvider>) {
    let provider = std::sync::Arc::new(ScriptedProvider::new([ScriptedTurn::text(reply)]));
    (LlmJudge::new(Box::new(ArcProvider(provider.clone()))), provider)
}

/// 让测试能在交给 judge 之后继续检查 provider 收到的 prompt。
struct ArcProvider(std::sync::Arc<ScriptedProvider>);
impl if_agent::Provider for ArcProvider {
    fn label(&self) -> String {
        self.0.label()
    }
    fn model(&self) -> &str {
        "llm-test"
    }
    fn stream_turn(
        &self,
        prompt: &if_agent::PromptContext,
        tools: &[if_agent::ToolSpec],
        on_event: &mut dyn FnMut(if_agent::ProviderEvent),
        cancel: &AtomicBool,
    ) -> if_agent::StreamTerminal {
        self.0.stream_turn(prompt, tools, on_event, cancel)
    }
}

#[test]
fn llm_answers_are_converted_to_jev_shaped_outputs() {
    let reply = "```json\n{\"n\":{\"p\":0.7},\"c\":{\"probabilities\":{\"go\":3,\"stay\":1}},\"s\":{\"probabilities\":{\"0\":0,\"1\":0.5,\"2\":0.5}}}\n```";
    let (judge, provider) = judge(reply);
    let resp = judge.judge(&request(), &AtomicBool::new(false)).unwrap();
    assert_eq!(resp.model, "llm-test");
    assert!(resp.missing.is_empty());
    assert_eq!(resp.answers["n"], JudgmentOutput::Noul { noul: 0.7 });
    match &resp.answers["c"] {
        JudgmentOutput::Choice { choice, probabilities, confidence } => {
            assert_eq!(choice, "go");
            assert_eq!(probabilities["stay"], 0.25, "分布被归一化");
            assert_eq!(*confidence, 0.75);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(resp.answers["s"].as_normalized(), Some(0.75));
    assert_eq!(judge.backend(), JudgeBackend::Llm { model: "llm-test".into() });

    let prompt = &provider.prompts()[0];
    let sent = prompt.messages[0].text();
    assert!(sent.contains("\"state\":\"雨城\""));
    assert!(sent.contains("\"criteria\":{\"go\""), "choice 的 criteria 仍是对象");
    assert!(prompt.system_text().contains("只输出一个 JSON 对象"));
}

#[test]
fn invalid_answers_become_missing_not_errors() {
    let (judge, _) = judge("{\"n\":{\"p\":1.5},\"c\":{\"probabilities\":{\"go\":0,\"stay\":0}}}");
    let resp = judge.judge(&request(), &AtomicBool::new(false)).unwrap();
    assert_eq!(resp.missing, vec!["n", "c", "s"]);
}

#[test]
fn non_json_reply_is_malformed() {
    let (judge, _) = judge("我认为大概会下雨。");
    assert!(matches!(judge.judge(&request(), &AtomicBool::new(false)), Err(JudgeError::Malformed(_))));
}

#[test]
fn provider_errors_are_classified() {
    let p = ScriptedProvider::new([
        ScriptedTurn::Error { message: "HTTP 503: busy".into(), retryable: true },
        ScriptedTurn::Error { message: "HTTP 401: bad key".into(), retryable: false },
        ScriptedTurn::Error { message: "HTTP 404: no model".into(), retryable: false },
    ]);
    let judge = LlmJudge::new(Box::new(p));
    let cancel = AtomicBool::new(false);
    assert!(matches!(judge.judge(&request(), &cancel), Err(JudgeError::Transient(_))));
    assert!(matches!(judge.judge(&request(), &cancel), Err(JudgeError::Auth { status: 401, .. })));
    assert!(matches!(judge.judge(&request(), &cancel), Err(JudgeError::Backend(_))));
}
