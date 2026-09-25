use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use if_domain::{JudgmentId, JudgmentOutput, TurnId, ViewKind, ViewRef};
use if_judge::{
    CompiledView, Judge, JudgeError, JevConfig, JevJudge, JudgeRequest, Question, QuestionSpec, RetryPolicy,
    Retrying, StubJudge,
};

fn view() -> CompiledView {
    CompiledView {
        meta: ViewRef { kind: ViewKind::Pov, holder: Some("c_gu".into()), hash: "b3:test".into() },
        state: serde_json::json!({ "视角人物": "顾言" }),
    }
}

fn q(key: &str, template: &str, spec: QuestionSpec) -> Question {
    Question { key: key.into(), template: template.into(), target: key.split('.').next().unwrap().into(), spec }
}

fn sample_request() -> JudgeRequest {
    let mut r = JudgeRequest::new(view());
    r.push(q("cand_004.notices", "q.perception.notices@1", QuestionSpec::noul("顾言会注意到吗？")));
    r.push(q(
        "cand_007.action",
        "q.outcome.choice@1",
        QuestionSpec::choice("顾言接下来做什么？", [("leave", "离开"), ("stay", "留下")]),
    ));
    r.push(q(
        "scene_1.tension",
        "q.scene.tension@1",
        QuestionSpec::score("紧张程度？", ["明显缓和", "略微缓和", "持平", "略微升高", "明显升高"]),
    ));
    r
}

const OK_BODY: &str = r#"{
  "model": "typesafe/jev-1.13-20260917",
  "answers": {
    "cand_004.notices": { "type": "noul", "noul": 0.58 },
    "cand_007.action": { "type": "choice", "choice": "stay",
      "probabilities": { "leave": 0.3, "stay": 0.7 }, "confidence": 0.63 },
    "scene_1.tension": { "type": "score", "score": 3.73,
      "legend": { "0": "明显缓和", "1": "略微缓和", "2": "持平", "3": "略微升高", "4": "明显升高" },
      "probabilities": { "0": 0, "1": 0.01, "2": 0.02, "3": 0.2, "4": 0.77 }, "confidence": 0.78 }
  },
  "usage": { "input_tokens": 1168, "output_tokens": 544, "cost": 0.00004906 },
  "id": "gen-dec-x", "provider": "TypeSafe"
}"#;

/// 极简 HTTP 服务：按顺序返回脚本里的 (status, body)，并记录收到的请求体。
fn serve(script: Vec<(u16, String)>) -> (String, Arc<std::sync::Mutex<Vec<serde_json::Value>>>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/alpha/decisions", listener.local_addr().unwrap());
    let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
    let hits = Arc::new(AtomicUsize::new(0));
    let (b, h) = (bodies.clone(), hits.clone());
    std::thread::spawn(move || {
        for (status, body) in script {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut buf = vec![0; len];
            reader.read_exact(&mut buf).unwrap();
            b.lock().unwrap().push(serde_json::from_slice(&buf).unwrap());
            h.fetch_add(1, Ordering::SeqCst);
            let resp = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
        }
    });
    (url, bodies, hits)
}

fn jev(url: String) -> JevJudge {
    let mut cfg = JevConfig::new("test-key");
    cfg.endpoint = url;
    cfg.timeout = Duration::from_secs(5);
    JevJudge::new(cfg)
}

#[test]
fn wire_request_uses_record_for_choice_and_array_for_score() {
    let (url, bodies, _) = serve(vec![(200, OK_BODY.into())]);
    jev(url).judge(&sample_request(), &AtomicBool::new(false)).unwrap();
    let body = &bodies.lock().unwrap()[0];
    assert_eq!(body["model"], "typesafe/jev-1.13");
    assert_eq!(body["state"]["视角人物"], "顾言");
    let qs = &body["questions"];
    assert_eq!(qs["cand_004.notices"]["type"], "noul");
    assert!(qs["cand_004.notices"].get("true_means").is_none());
    assert!(qs["cand_007.action"]["criteria"].is_object());
    assert!(qs["scene_1.tension"]["criteria"].is_array());
    assert_eq!(qs["scene_1.tension"]["criteria"][4], "明显升高");
}

#[test]
fn response_is_parsed_and_records_use_echoed_snapshot() {
    let (url, _, _) = serve(vec![(200, OK_BODY.into())]);
    let req = sample_request();
    let resp = jev(url).judge(&req, &AtomicBool::new(false)).unwrap();
    assert_eq!(resp.model, "typesafe/jev-1.13-20260917");
    assert!(resp.missing.is_empty());
    assert_eq!(resp.answers["cand_004.notices"], JudgmentOutput::Noul { noul: 0.58 });
    assert!((resp.usage.cost_usd - 0.00004906).abs() < 1e-12);

    let mut n = 0;
    let records = resp.to_judgments(&req, &TurnId::new("turn_1"), || {
        n += 1;
        JudgmentId::new(format!("jdg_{n}"))
    });
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|j| j.model == "typesafe/jev-1.13-20260917"));
    assert_eq!(records[0].template, "q.perception.notices@1");
    assert_eq!(records[0].target, "cand_004");
    assert_eq!(records[0].usage.input_tokens, 1168);
    assert_eq!(records[1].usage.input_tokens, 0, "请求级用量只记一次");
}

#[test]
fn missing_and_mismatched_answers_are_reported() {
    let body = r#"{ "model": "m", "answers": {
        "cand_004.notices": { "type": "choice", "choice": "a", "probabilities": {"a": 1}, "confidence": 1 },
        "cand_007.action": { "type": "choice", "choice": "zzz", "probabilities": {"leave": 1}, "confidence": 1 },
        "unknown": { "type": "noul", "noul": 0.1 } } }"#;
    let (url, _, _) = serve(vec![(200, body.into())]);
    let resp = jev(url).judge(&sample_request(), &AtomicBool::new(false)).unwrap();
    assert!(resp.answers.is_empty());
    assert_eq!(resp.missing, vec!["cand_004.notices", "cand_007.action", "scene_1.tension"]);
}

#[test]
fn status_400_is_invalid_with_path_and_not_retried() {
    let err = r#"{"error":{"message":"expected record, received array","metadata":{"issues":[{"path":["questions","q1","criteria"]}]}}}"#;
    let (url, _, hits) = serve(vec![(400, err.into())]);
    let judge = Retrying::new(jev(url), RetryPolicy { retries: 2, base_delay: Duration::from_millis(1) });
    match judge.judge(&sample_request(), &AtomicBool::new(false)) {
        Err(JudgeError::Invalid { path, message }) => {
            assert_eq!(path, "questions.q1.criteria");
            assert!(message.contains("expected record"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn status_401_is_auth() {
    let (url, _, _) = serve(vec![(401, r#"{"error":{"message":"bad key"}}"#.into())]);
    assert!(matches!(
        jev(url).judge(&sample_request(), &AtomicBool::new(false)),
        Err(JudgeError::Auth { status: 401, .. })
    ));
}

#[test]
fn transient_errors_are_retried_then_succeed() {
    let (url, _, hits) = serve(vec![(503, "{}".into()), (500, "{}".into()), (200, OK_BODY.into())]);
    let judge = Retrying::new(jev(url), RetryPolicy { retries: 2, base_delay: Duration::from_millis(1) });
    assert!(judge.judge(&sample_request(), &AtomicBool::new(false)).is_ok());
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[test]
fn transient_errors_give_up_after_retries() {
    let (url, _, hits) = serve(vec![(503, "{}".into()), (503, "{}".into()), (503, "{}".into())]);
    let judge = Retrying::new(jev(url), RetryPolicy { retries: 2, base_delay: Duration::from_millis(1) });
    assert!(matches!(judge.judge(&sample_request(), &AtomicBool::new(false)), Err(JudgeError::Transient(_))));
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[test]
fn local_validation_rejects_before_network() {
    let judge = jev("http://127.0.0.1:9/never".into());
    let cancel = AtomicBool::new(false);

    let empty = JudgeRequest::new(view());
    assert!(matches!(judge.judge(&empty, &cancel), Err(JudgeError::Invalid { .. })));

    let mut one_option = JudgeRequest::new(view());
    one_option.push(q("x", "t@1", QuestionSpec::choice("?", [("a", "A")])));
    assert!(matches!(judge.judge(&one_option, &cancel), Err(JudgeError::Invalid { path, .. }) if path == "questions.x.criteria"));

    let mut too_many = JudgeRequest::new(view());
    too_many.push(q("s", "t@1", QuestionSpec::score("?", (0..11).map(|i| i.to_string()))));
    assert!(matches!(judge.judge(&too_many, &cancel), Err(JudgeError::Invalid { .. })));

    let mut dup = JudgeRequest::new(view());
    dup.push(q("d", "t@1", QuestionSpec::noul("?")));
    dup.push(q("d", "t@1", QuestionSpec::noul("?")));
    assert!(matches!(judge.judge(&dup, &cancel), Err(JudgeError::Invalid { .. })));
}

#[test]
fn cancelled_before_send() {
    let judge = jev("http://127.0.0.1:9/never".into());
    assert!(matches!(judge.judge(&sample_request(), &AtomicBool::new(true)), Err(JudgeError::Cancelled)));
}

#[test]
fn stub_scripts_and_defaults() {
    let stub = StubJudge::new()
        .noul_for_template("q.perception.notices", 0.9)
        .drop_key("scene_1.tension");
    let req = sample_request();
    let resp = stub.judge(&req, &AtomicBool::new(false)).unwrap();
    assert_eq!(resp.answers["cand_004.notices"], JudgmentOutput::Noul { noul: 0.9 });
    match &resp.answers["cand_007.action"] {
        JudgmentOutput::Choice { choice, probabilities, .. } => {
            assert_eq!(choice, "leave");
            assert_eq!(probabilities["stay"], 0.5);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(resp.missing, vec!["scene_1.tension"]);
    assert_eq!(stub.requests().len(), 1);

    let mut score_only = JudgeRequest::new(view());
    score_only.push(q("s", "q.scene.tension@1", QuestionSpec::score("?", ["a", "b", "c", "d", "e"])));
    let resp = StubJudge::new().judge(&score_only, &AtomicBool::new(false)).unwrap();
    assert_eq!(resp.answers["s"].as_normalized(), Some(0.5));
}
