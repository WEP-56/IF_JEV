//! 任务层的测试（docs/12 §9：**Judge 测试桩 + 模拟的 provider**）。
//!
//! 这里不联网、不花额度：`ScriptedProvider` 按脚本回答，`StubJudge` 按模板给概率。
//! 但走的是**真机同一条代码路径**——任务 host、工具参数校验、`if-pipeline` 的编排、
//! 事件草稿的生成全都在里面。所以这一条测试红，就是真机上会红。

use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use if_agent::provider::scripted::{ScriptedProvider, ScriptedTurn};
use if_agent::ToolSpec;
use if_judge::StubJudge;

use crate::tasks::driver::{run_if_turn, TurnProviders, TurnRequest};
use crate::tasks::{impact, plan, scenes as scenes_task, TaskKind};
use crate::testsupport;

// ---------------------------------------------------------------- 共用的护栏

/// 工具 schema 的 `properties` 必须与「模型要填的那个结构体」**集合相等**。
///
/// 这不是形式主义：schema 少一个字段，模型永远不会返回它，反序列化当场失败，
/// 用户只看到一句「missing field `x`」——真机上发生过一次（`IfDraft::input`）。
/// 每个提议工具都挂一份这个断言，加字段时想忘都忘不掉。
pub(crate) fn assert_schema_matches_struct<T>(tool: &ToolSpec, sample: &Value)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let parsed: T = serde_json::from_value(sample.clone())
        .unwrap_or_else(|error| panic!("{} 的样本不是它要填的结构：{error}", tool.name));
    let value = serde_json::to_value(&parsed).expect("结构体应当可以序列化");
    let mut fields: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("{} 的结构体必须是 object", tool.name))
        .keys()
        .cloned()
        .collect();
    fields.sort();

    assert_eq!(
        fields,
        crate::tasks::schema_properties(tool),
        "{} 的字段与工具 schema 不一致",
        tool.name
    );
    assert_eq!(
        fields,
        crate::tasks::schema_required(tool),
        "{} 的字段必须全部进 required——非必填字段会逼调用方处理「模型没给」的分支",
        tool.name
    );

    let properties = tool.schema["properties"].as_object().expect("properties");
    for (name, spec) in properties {
        assert!(
            spec["description"].as_str().is_some_and(|text| !text.trim().is_empty()),
            "{} 的 `{name}` 缺少 description——模型只能猜",
            tool.name
        );
    }

    // 样本本身必须过 schema 校验，否则「模型照 schema 返回」这句话就是假的。
    if let Err(errors) = if_agent::tools::schema::validate(&tool.schema, sample) {
        panic!("样本没能通过 {} 的 schema 校验：{errors:?}", tool.name);
    }
}

/// 一个「干净回合」的判定桩：约束全过、节拍全干净。
///
/// 默认桩的答案是 0.5，而 `q.cand.knowledge_gap` / `q.beat.*` 的方向是
/// **越高越可疑**（`HigherFlags`）——不显式压下去，整个回合会一条正文都放行不了。
/// 这不是桩坏了，是方向本来如此（见 `if-policy::threshold` 的表）。
fn clean_judge() -> StubJudge {
    let mut judge = StubJudge::new();
    for template in [
        "q.cand.knowledge_gap",
        "q.scene.resolves_thread",
        "q.beat.violates_fact",
        "q.beat.violates_rule",
        "q.beat.knowledge_leak",
        "q.beat.forbidden_resolution",
        "q.beat.reveals_secret",
        "q.beat.stop_reached",
    ] {
        judge = judge.noul_for_template(template, 0.0);
    }
    judge = judge.noul_for_template("q.cand.in_character", 1.0);
    judge = judge.noul_for_template("q.behavior.occurs", 1.0);
    judge
}

// ---------------------------------------------------------------- 夹具

fn impact_args() -> Value {
    json!({
        "subject": "林夏",
        "content": "林夏当晚去宫门找顾言",
        "internal": false,
        "shape": "occurs",
        "options": [],
        "depends_on": [],
        "based_on": ["c_lin.evidence"],
        "affects": ["c_lin.evidence"]
    })
}

fn scene_args() -> Value {
    json!({
        "summary": "雨夜宫门外，林夏把信交给顾言",
        "threads": ["thr_shield"],
        "resolves": [],
        "erupts": [],
        "focus": ["林夏"],
        "present": ["林夏", "顾言"]
    })
}

/// 刻意**不留 required_beats**：这样被拦下来的节拍可以跳过，而不是止损整个场景。
fn plan_args() -> Value {
    json!({
        "goal": "顾言说出他知道的事",
        "pov": "林夏",
        "focus": ["顾言"],
        "present": ["林夏", "顾言"],
        "time_span": "当晚",
        "required_beats": [],
        "stop_condition": "顾言把话说完",
        "reveal_facts": [],
        "reveal_lore": [],
        "proposed_changes": []
    })
}

const BEAT_1: &str = "第一拍：雨落下来。";
const BEAT_2: &str = "第二拍：她说了一句不该说的话。";
const BEAT_3: &str = "第三拍：他转身走了。";

fn script() -> Vec<ScriptedTurn> {
    vec![
        ScriptedTurn::tool("propose_candidate", impact_args()),
        ScriptedTurn::text("已提交候选。"),
        ScriptedTurn::tool("propose_scene", scene_args()),
        ScriptedTurn::text("已提交场景候选。"),
        ScriptedTurn::tool("submit_scene_plan", plan_args()),
        ScriptedTurn::text("已提交场景计划。"),
        ScriptedTurn::text(format!("{BEAT_1}<<<BEAT>>>{BEAT_2}<<<BEAT>>>{BEAT_3}")),
        ScriptedTurn::tool(
            "record_observation",
            json!({
                "prop": "c_lin.evidence",
                "value": false,
                "internal": false,
                "text": "他转身走了。"
            }),
        ),
        ScriptedTurn::text("已回收。"),
    ]
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

// ---------------------------------------------------------------- 整条回合

/// 一条从「模型提议」到「事件草稿」的完整 IF 回合，不联网。
#[test]
fn a_whole_if_turn_runs_from_proposals_to_event_drafts() {
    let provider = ScriptedProvider::new(script());
    let judge = clean_judge();
    let projection = testsupport::projection();
    let settings = testsupport::settings();
    let ctx = testsupport::ctx(0);
    let cancel = never();
    let mut events = Vec::new();

    let outcome = run_if_turn(
        TurnProviders::single(&provider),
        &judge,
        TurnRequest {
            projection: &projection,
            settings: &settings,
            ctx: &ctx,
            input: Some("IF 林夏当晚去宫门".into()),
            source_event: None,
            first_seq: 1,
            displayed_at: Some(1_700_000_000),
            time: None,
        },
        &cancel,
        &mut |event| events.push(event),
    )
    .expect("干净回合应当跑通");

    // ---- 提议确实进了引擎
    assert_eq!(outcome.scene_proposal().map(|scene| scene.id.as_str()), Some("scene_0001"));
    let scene = outcome.scene.as_ref().expect("有场景");
    assert_eq!(scene.plan.pov.as_str(), "c_lin");
    assert!(!scene.plan.present.is_empty());

    // ---- 五个任务都跑过，名字与轮数都记下来了
    let names: Vec<&str> = outcome.tasks.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec!["T-impact", "T-scenes", "T-plan", "T-render", "T-extract"]
    );

    // ---- 正文切成了节拍，三拍都干净，所以三拍都放行
    let admitted: Vec<&str> = scene
        .beats
        .admitted
        .iter()
        .map(|beat| beat.text.as_str())
        .collect();
    assert_eq!(admitted, vec![BEAT_1, BEAT_2, BEAT_3]);

    // ---- 事件草稿与提交
    assert!(!scene.drafts.is_empty(), "一个回合至少要写出事件");
    assert_eq!(scene.drafts.len(), scene.committed.len());
    assert_eq!(scene.committed.len(), scene.drafts.len());

    // ---- 判定记录在整个回合里唯一（三段判定共用同一个游标）
    let mut ids: Vec<String> = outcome
        .opening
        .impact
        .judgments
        .iter()
        .chain(outcome.opening.scenes.judgments.iter())
        .chain(scene.beats.judgments.iter())
        .map(|judgment| judgment.id.as_str().to_owned())
        .collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "判定 ID 在回合里必须唯一");

    // ---- 提交的事件都盖上了场景与节拍（否则事后按场景查事件是一片空）
    for draft in &scene.drafts {
        assert_eq!(draft.scene.as_ref().map(|s| s.as_str()), Some("scene_0001"));
        assert!(draft.turn == ctx.turn);
    }
}

/// **T-extract 只能看见已经放行的节拍。**
///
/// 这一条是两段驱动之间那道缝的全部意义：从被拦下来的节拍里回收，
/// 等于把一件没人看见的事写进历史。所以这里让第 2 拍违规，
/// 再从模型收到的提示词里确认它**没见过**那一拍。
#[test]
fn the_extractor_only_ever_sees_the_beats_that_were_admitted() {
    let provider = ScriptedProvider::new(script());
    let judge = clean_judge().noul_for_key("beat_2.fact.p_lin_evidence", 1.0);
    let projection = testsupport::projection();
    let settings = testsupport::settings();
    let ctx = testsupport::ctx(0);
    let cancel = never();

    let outcome = run_if_turn(
        TurnProviders::single(&provider),
        &judge,
        TurnRequest {
            projection: &projection,
            settings: &settings,
            ctx: &ctx,
            input: Some("IF 林夏当晚去宫门".into()),
            source_event: None,
            first_seq: 1,
            displayed_at: None,
            time: None,
        },
        &cancel,
        &mut |_| {},
    )
    .expect("回合应当跑通");

    let scene = outcome.scene.as_ref().expect("有场景");
    let admitted: Vec<&str> = scene.beats.admitted.iter().map(|beat| beat.text.as_str()).collect();
    assert_eq!(admitted, vec![BEAT_1, BEAT_3], "违规的第 2 拍不该上屏");

    // 最后一个提示词就是 T-extract 的那一次。
    let prompts = provider.prompts();
    let extract_prompt = prompts.last().expect("至少有 T-extract 的一次发问");
    let user = extract_prompt
        .messages
        .iter()
        .map(|message| message.text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(user.contains(BEAT_1), "已放行的第 1 拍应当交给回收器");
    assert!(user.contains(BEAT_3), "已放行的第 3 拍应当交给回收器");
    assert!(
        !user.contains(BEAT_2),
        "被拦下来的第 2 拍绝不能进回收器的视野"
    );
}

/// 任务名与工具名对得上：回执里说的、`TurnRecord` 里记的、给模型看的必须是同一套。
#[test]
fn the_task_catalogue_names_its_own_tools() {
    assert_eq!(TaskKind::Impact.as_str(), "T-impact");
    assert_eq!(TaskKind::Impact.tool().name, impact::TOOL);
    assert_eq!(TaskKind::Scenes.tool().name, scenes_task::TOOL);
    assert_eq!(TaskKind::Plan.tool().name, plan::TOOL);
    assert_eq!(TaskKind::Extract.tool().name, crate::tasks::extract::TOOL);
    assert_eq!(crate::tasks::RENDER_TASK, "T-render");
    // 只有 T-extract 允许空手而归。
    assert!(TaskKind::Extract.allows_empty());
    assert!(!TaskKind::Impact.allows_empty());
    assert!(!TaskKind::Scenes.allows_empty());
    assert!(!TaskKind::Plan.allows_empty());
}
