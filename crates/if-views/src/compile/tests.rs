//! 编译器测试：重点在**隔离**与**确定性**，不在字段铺满。

use if_domain::id::{EventId, LoreId, PropositionId, SubjectId};
use if_domain::narrative::{LoreEntry, LoreSection, LoreStatus, LoreVisibility, SecondaryLogic, Thread};
use if_domain::projection::Projection;
use if_domain::rule::{ActiveWindow, WorldRule};
use if_domain::state::{Belief, Fact};
use if_domain::subject::{Proposition, PropositionKind, Subject, SubjectKind, ValueType};
use if_domain::turn::ViewKind;
use if_domain::value::{Lock, Visibility, WorldTime};

use super::*;

fn subject(id: &str, name: &str) -> Subject {
    Subject::new(id, SubjectKind::Character, name)
}

fn prop(id: &str, key: &str, subjects: &[&str], internal: bool) -> Proposition {
    Proposition {
        id: PropositionId::new(id),
        key: key.to_owned(),
        text: format!("{key} 的说明"),
        subjects: subjects.iter().map(|s| SubjectId::new(*s)).collect(),
        kind: PropositionKind::State,
        value_type: ValueType::Bool,
        internal,
    }
}

fn world() -> Projection {
    let mut projection = Projection::genesis("wl_main");
    projection.world_time = WorldTime::from_days(7).plus_minutes(137);
    projection.narrative_order = 48;
    projection.subjects.insert(SubjectId::new("c_gu"), subject("c_gu", "顾言"));
    projection.subjects.insert(SubjectId::new("c_lin"), subject("c_lin", "林夏"));

    let open = prop("p_open", "c_gu.awaits", &["c_gu"], false);
    projection.propositions.insert(open.id.clone(), open.clone());
    projection.facts.insert(
        open.id.clone(),
        Fact::new(open.id.clone(), true, WorldTime::EPOCH, Lock::L2, "evt_1")
            .with_visibility(Visibility::Public),
    );

    let inner = prop("p_inner", "c_lin.affection", &["c_lin"], true);
    projection.propositions.insert(inner.id.clone(), inner.clone());
    projection.facts.insert(
        inner.id.clone(),
        Fact::new(inner.id.clone(), 0.7, WorldTime::EPOCH, Lock::L1, "evt_2")
            .with_visibility(Visibility::Private),
    );

    let hidden = prop("p_hidden", "c_lin.is_royal", &["c_lin"], false);
    projection.propositions.insert(hidden.id.clone(), hidden.clone());
    projection.facts.insert(
        hidden.id.clone(),
        Fact::new(hidden.id.clone(), true, WorldTime::EPOCH, Lock::L3, "evt_3")
            .with_visibility(Visibility::Secret),
    );

    projection.beliefs.insert(
        if_domain::projection::BeliefKey::new("c_gu", "p_inner"),
        Belief::witnessed("c_gu", "p_inner", true, WorldTime::EPOCH, "evt_4"),
    );

    projection.rules.insert(
        if_domain::id::RuleId::new("r_night"),
        WorldRule {
            id: if_domain::id::RuleId::new("r_night"),
            text: "夜里宵禁".into(),
            lock: Lock::L2,
            source: EventId::new("evt_5"),
            active: ActiveWindow::from_now(WorldTime::EPOCH),
            scope: vec![],
            invariants: vec![],
            mechanics: vec![],
            triggers: vec![],
            constraints: vec!["q.beat.violates_rule".into()],
            boundaries: vec![],
        },
    );

    projection.threads.insert(
        if_domain::id::ThreadId::new("t_leave"),
        Thread {
            id: if_domain::id::ThreadId::new("t_leave"),
            title: "顾言会不会离开".into(),
            question: "他会不会走".into(),
            stakes: "两人关系".into(),
            subjects: vec![SubjectId::new("c_gu")],
            stage: if_domain::narrative::ThreadStage::Developing,
            protected_until: Some(if_domain::narrative::ThreadStage::Climax),
            pressure: 0.5,
            last_advanced: 0,
            cadence: 3.0,
        },
    );

    projection
}

fn lore(section: LoreSection, visibility: LoreVisibility, known_by: &[&str]) -> LoreEntry {
    LoreEntry {
        id: LoreId::new("l_1"),
        title: "文风".into(),
        content: "冷静克制的句式".into(),
        keys: vec![],
        secondary_keys: vec![],
        logic: SecondaryLogic::AndAny,
        subjects: vec![],
        when: None,
        constant: true,
        order: 100,
        section,
        visibility,
        known_by: known_by.iter().map(|s| SubjectId::new(*s)).collect(),
        probability: None,
        status: LoreStatus::Active,
        source: EventId::new("evt_6"),
    }
}

// ---------------------------------------------------------------- 隔离

/// 核心断言：角色视图不得出现该角色无权知道的事实。
#[test]
fn pov_view_excludes_facts_the_holder_cannot_perceive() {
    let projection = world();
    let view = compile(
        &projection,
        &ViewRequest::new(ViewKind::Pov).holder("c_gu").scene_index(0),
    );
    let keys = json_keys(&view.state, "facts");
    assert!(keys.contains(&"c_gu.awaits".to_owned()), "{keys:?}");
    assert!(
        !keys.contains(&"c_lin.is_royal".to_owned()),
        "secret 事实泄漏进了顾言的视图：{keys:?}"
    );
    // 顾言对林夏的感情有信念（被告知），所以那条不是「无权知道」——
    // 但事实层依然不给，认知层给。两层分离是刻意的。
    assert!(!keys.contains(&"c_lin.affection".to_owned()), "{keys:?}");
    assert!(!key_list(&view.state, "beliefs").is_empty(), "信念层应该有记录");

    // 而林夏自己是能感知到的。
    let lin = compile(
        &projection,
        &ViewRequest::new(ViewKind::Pov).holder("c_lin").scene_index(0),
    );
    assert!(json_keys(&lin.state, "facts").contains(&"c_lin.affection".to_owned()));
}

#[test]
fn check_view_sees_secrets_but_narration_does_not() {
    let projection = world();
    let check = compile(
        &projection,
        &ViewRequest::new(ViewKind::Check)
            .scene_index(0)
            .present_hint(&["c_gu"]),
    );
    assert!(json_keys(&check.state, "facts").contains(&"c_lin.is_royal".to_owned()));
    assert!(string_list(&check.state, "unapproved_secrets").contains(&"p_hidden".to_owned()));

    let narration = compile(
        &projection,
        &ViewRequest::new(ViewKind::Narration)
            .scene_index(0)
            .present_hint(&["c_gu"]),
    );
    assert!(
        !json_keys(&narration.state, "facts").contains(&"c_lin.is_royal".to_owned()),
        "叙事视图泄漏了秘密事实"
    );
    assert!(
        string_list(&narration.state, "unapproved_secrets").is_empty(),
        "秘密清单不该进叙事视图（docs/04 §2.1）"
    );
}

#[test]
fn knowledge_boundaries_name_who_is_unaware() {
    let projection = world();
    let view = compile(
        &projection,
        &ViewRequest::new(ViewKind::Check)
            .scene_index(0)
            .present_hint(&["c_gu"]),
    );
    let boundaries = view.state["knowledge_boundaries"]
        .as_array()
        .expect("检查视图必须给出在场者的认知边界");
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0]["holder"], "c_gu");
    let unaware = boundaries[0]["unaware_of"]
        .as_array()
        .expect("顾言不知道秘密，边界里就该列出来");
    assert!(unaware.iter().any(|v| v == "c_lin.is_royal"), "{unaware:?}");
}

#[test]
fn pov_without_holder_yields_an_empty_view_not_an_omniscient_one() {
    let projection = world();
    let view = compile(&projection, &ViewRequest::new(ViewKind::Pov).scene_index(0));
    assert!(
        json_keys(&view.state, "facts").is_empty(),
        "缺持有者的角色视图必须为空"
    );
}

// ---------------------------------------------------------------- 确定性

#[test]
fn same_projection_same_request_same_fingerprint() {
    let projection = world();
    let request = ViewRequest::new(ViewKind::God).scene_index(3);
    let a = compile(&projection, &request);
    let b = compile(&projection, &request);
    assert_eq!(a.meta.hash, b.meta.hash);
    assert_eq!(a.state, b.state);
}

#[test]
fn different_views_get_different_fingerprints() {
    let projection = world();
    let god = compile(&projection, &ViewRequest::new(ViewKind::God));
    let director = compile(&projection, &ViewRequest::new(ViewKind::Director));
    let parse = compile(&projection, &ViewRequest::new(ViewKind::Parse));
    assert_ne!(god.meta.hash, director.meta.hash, "导演视图多了调度信号");
    assert_ne!(god.meta.hash, parse.meta.hash);
}

#[test]
fn different_holders_get_different_fingerprints() {
    let projection = world();
    let gu = compile(&projection, &ViewRequest::new(ViewKind::Pov).holder("c_gu").scene_index(0));
    let lin = compile(&projection, &ViewRequest::new(ViewKind::Pov).holder("c_lin").scene_index(0));
    assert_ne!(gu.meta.hash, lin.meta.hash);
    assert_eq!(gu.meta.holder, Some(SubjectId::new("c_gu")));
}

#[test]
fn scene_index_changes_the_fingerprint_of_protected_state() {
    let mut projection = world();
    // 给一条 L1 事实加上保护期，场景序号就会影响它是否可见。
    let p = projection.propositions.get(&PropositionId::new("p_inner")).cloned();
    if let (Some(proposition), Some(fact)) = (
        p,
        projection.facts.get(&PropositionId::new("p_inner")).cloned(),
    ) {
        projection
            .facts
            .insert(proposition.id.clone(), fact.with_protection(5));
    }
    let early = compile(
        &projection,
        &ViewRequest::new(ViewKind::Pov).holder("c_lin").scene_index(1),
    );
    let late = compile(
        &projection,
        &ViewRequest::new(ViewKind::Pov).holder("c_lin").scene_index(5),
    );
    assert_ne!(early.meta.hash, late.meta.hash);
}

// ---------------------------------------------------------------- 视图内容

#[test]
fn director_view_carries_engine_signals() {
    let projection = world();
    let view = compile(&projection, &ViewRequest::new(ViewKind::Director));
    let director = &view.state["director"];
    assert!(director.is_object(), "{:?}", view.state);
    assert!(director["overdue_threads"].is_array());
    assert_eq!(director["scene_count"], 0);
}

#[test]
fn narration_view_only_carries_style_lore() {
    let projection = world();
    let request = ViewRequest::new(ViewKind::Narration).scene_index(0).lore(vec![
        lore(LoreSection::Style, LoreVisibility::Public, &[]),
        lore(LoreSection::World, LoreVisibility::Public, &[]),
    ]);
    let view = compile(&projection, &request);
    let sections = view.state["lore"]
        .as_array()
        .expect("叙事视图要有文风条目")
        .iter()
        .map(|l| l["section"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(sections, vec!["style".to_owned()], "设定条目不该进叙事视图");
}

#[test]
fn secret_lore_is_hidden_from_outsiders() {
    let projection = world();
    let entry = lore(LoreSection::World, LoreVisibility::Secret, &["c_lin"]);
    let gu = compile(
        &projection,
        &ViewRequest::new(ViewKind::Pov)
            .holder("c_gu")
            .scene_index(0)
            .lore(vec![entry.clone()]),
    );
    assert!(view_lore_ids(&gu.state).is_empty(), "secret 条目不该给外人");
    let lin = compile(
        &projection,
        &ViewRequest::new(ViewKind::Pov)
            .holder("c_lin")
            .scene_index(0)
            .lore(vec![entry]),
    );
    assert_eq!(view_lore_ids(&lin.state), vec!["l_1".to_owned()]);
}

#[test]
fn parse_view_carries_the_user_input() {
    let projection = world();
    let view = compile(
        &projection,
        &ViewRequest::new(ViewKind::Parse).user_input("IF 王宫刚刚起火了"),
    );
    assert_eq!(view.state["user_input"], "IF 王宫刚刚起火了");
}

#[test]
fn budget_trims_history_before_rules() {
    let mut projection = world();
    for i in 0..400 {
        projection.beats.push(if_domain::narrative::Beat {
            id: if_domain::id::BeatId::new(format!("b_{i}")),
            scene: if_domain::id::SceneId::new("sc_1"),
            index: i as u32,
            text: "正文".repeat(30),
            plan_beat: None,
            judgments: vec![],
            displayed_at: None,
        });
    }
    let view = compile(
        &projection,
        &ViewRequest::new(ViewKind::God).budget(4_000),
    );
    assert_eq!(
        view.state["rules"].as_array().map(Vec::len),
        Some(1),
        "规则不该被裁掉"
    );
    let beats = view.state["recent_beats"].as_array().map(Vec::len).unwrap_or(0);
    assert!(beats < 400, "历史应该先被裁，实际 {beats}");
}

// ---------------------------------------------------------------- 辅助

fn json_keys(state: &serde_json::Value, section: &str) -> Vec<String> {
    state[section]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["key"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn key_list(state: &serde_json::Value, section: &str) -> Vec<String> {
    state[section]
        .as_array()
        .map(|items| items.to_vec())
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item["prop"].as_str().map(str::to_owned))
        .collect()
}

fn string_list(state: &serde_json::Value, section: &str) -> Vec<String> {
    state[section]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn view_lore_ids(state: &serde_json::Value) -> Vec<String> {
    state["lore"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|l| l["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
