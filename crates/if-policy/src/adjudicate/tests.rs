use std::collections::BTreeMap;

use if_domain::{CandidateShape, SubjectId};

use super::*;
use crate::dice::candidate_key;

fn settings(seed: u64) -> WorldSettings {
    WorldSettings {
        seed,
        ..Default::default()
    }
}

fn occurs(id: &str, key: Option<&str>, affects: &[&str]) -> Candidate {
    Candidate {
        id: CandidateId::new(id),
        key: key.map(str::to_string),
        subject: Some(SubjectId::new("c_gu")),
        content: format!("候选 {id} 发生"),
        internal: false,
        shape: CandidateShape::Occurs,
        depends_on: vec![],
        based_on: vec![],
        affects: affects.iter().map(|p| PropositionId::new(*p)).collect(),
    }
}

fn exclusive(id: &str, key: &str, options: &[&str]) -> Candidate {
    Candidate {
        id: CandidateId::new(id),
        key: Some(key.to_string()),
        subject: Some(SubjectId::new("c_lin")),
        content: format!("选择点 {id}"),
        internal: true,
        shape: CandidateShape::Exclusive {
            options: options.iter().map(|o| (*o).to_string()).collect(),
        },
        depends_on: vec![],
        based_on: vec![],
        affects: vec![],
    }
}

fn dist(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

fn day7() -> WorldTime {
    WorldTime::from_days(7)
}

fn policy() -> Policy {
    Policy::new(&settings(20260926), day7(), 48)
}

fn id(raw: &str) -> CandidateId {
    CandidateId::new(raw)
}

/// 找一个骰子值大于 `floor` 的决策键。用来把「必然被否决」写死，而不是靠运气。
fn key_with_die_above(policy: &Policy, floor: f64) -> String {
    (0..500)
        .map(|i| candidate_key(&format!("k{i}"), MechanicStep::Day, day7()))
        .find(|key| policy.dice().roll(key) > floor)
        .expect("总能找到一个")
}

// ---------------------------------------------------------------- 确定性

#[test]
fn same_policy_and_input_give_the_same_result() {
    let input = AdjudicationInput::new(vec![
        occurs("cand_1", Some("c_gu.notices@D7"), &[]),
        occurs("cand_2", Some("c_gu.leaves@D7"), &[]),
    ])
    .with_probability("cand_1", 0.6)
    .with_probability("cand_2", 0.4)
    .from_event("evt_1");

    let a = policy().run(&input).unwrap();
    let b = policy().run(&input).unwrap();
    assert_eq!(a, b);
    // 换一个世界（不同种子）就得用另一颗骰子
    let other = Policy::new(&settings(99), day7(), 48).run(&input).unwrap();
    assert_ne!(a.resolutions, other.resolutions);
}

#[test]
fn the_die_is_what_decides_whether_an_occurrence_happens() {
    let policy = policy();
    let key = candidate_key("c_gu.notices", MechanicStep::Day, day7());
    let die = policy.dice().roll(&key);

    // p 越过 u 才翻转：p 在 u 之上 → 发生，之下 → 不发生
    let above = policy
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", Some(&key), &[])])
                .with_probability("cand_1", die + (1.0 - die) * 0.5),
        )
        .unwrap();
    assert!(above.resolution(&id("cand_1")).unwrap().accepted());

    let below = policy
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", Some(&key), &[])])
                .with_probability("cand_1", die * 0.5),
        )
        .unwrap();
    assert!(!below.resolution(&id("cand_1")).unwrap().accepted());

    // 两次记录的是同一颗骰子
    assert_eq!(
        above.resolution(&id("cand_1")).unwrap().die,
        below.resolution(&id("cand_1")).unwrap().die
    );
    // 阈值裁决永远不带骰子值
    let l1 = policy
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", Some(&key), &["p_secret"])])
                .with_probability("cand_1", 0.99)
                .protecting([PropositionId::new("p_secret")])
                .triggering("cand_1"),
        )
        .unwrap();
    assert!(l1.resolution(&id("cand_1")).unwrap().die.is_none());
}

// ---------------------------------------------------------------- 降级

#[test]
fn a_missing_occurrence_judgment_becomes_no_occurrence() {
    let result = policy()
        .run(&AdjudicationInput::new(vec![occurs(
            "cand_1",
            Some("c_gu.notices@D7"),
            &[],
        )]))
        .unwrap();
    let resolution = result.resolution(&id("cand_1")).unwrap();
    assert!(!resolution.accepted());
    // 没有概率就没有抽样：记降级，但不编一个骰子值
    assert!(resolution.die.is_none());
    assert_eq!(
        resolution.decision_key.as_deref(),
        Some("c_gu.notices@D7")
    );
    // 也不该凭空长出一条趋势
    assert!(result.tendencies.is_empty());
}

// ---------------------------------------------------------------- 观察带

#[test]
fn a_rejected_occurrence_in_the_watch_band_becomes_a_tendency() {
    let policy = policy();
    let key = key_with_die_above(&policy, 0.6);
    // p = 0.5：低于骰子（必然否决），高于 τ_watch（进观察带）
    let result = policy
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", Some(&key), &["p_notices"])])
                .with_probability("cand_1", 0.5)
                .from_event("evt_9"),
        )
        .unwrap();

    assert_eq!(result.rejected(), vec![&id("cand_1")]);
    assert_eq!(result.tendencies.len(), 1);
    let tendency = &result.tendencies[0];
    // 初始压力 = p × 0.5
    assert!((tendency.pressure - 0.25).abs() < 1e-9);
    assert_eq!(tendency.status, if_domain::TendencyStatus::Latent);
    // ID 从决策键推出来，重放不会漂移
    assert_eq!(tendency.id.as_str(), format!("tnd_{key}"));
    // 贡献事件接上了
    assert_eq!(tendency.contributors, vec![if_domain::EventId::new("evt_9")]);
    // 爆发后要对应的命题
    assert_eq!(tendency.target, "p_notices");
    // 层结果里也记了
    assert_eq!(result.layers[0].tendencies, vec![tendency.id.clone()]);
    // 能按候选找回那条趋势
    assert_eq!(result.tendency_of(&id("cand_1")), Some(tendency));
}

#[test]
fn a_rejected_occurrence_below_the_watch_band_leaves_nothing_behind() {
    let result = policy()
        .run(
            &AdjudicationInput::new(vec![occurs(
                "cand_1",
                Some("c_gu.notices@D7"),
                &[],
            )])
            .with_probability("cand_1", 0.0),
        )
        .unwrap();
    assert_eq!(result.rejected(), vec![&id("cand_1")]);
    assert!(result.tendencies.is_empty());
    assert_eq!(result.tendency_of(&id("cand_1")), None);
}

#[test]
fn an_accepted_occurrence_creates_no_tendency() {
    let result = policy()
        .run(
            &AdjudicationInput::new(vec![occurs(
                "cand_1",
                Some("c_gu.notices@D7"),
                &[],
            )])
            .with_probability("cand_1", 1.0),
        )
        .unwrap();
    assert_eq!(result.accepted(), vec![&id("cand_1")]);
    assert!(result.tendencies.is_empty());
}

// ---------------------------------------------------------------- 互斥组

#[test]
fn an_exclusive_group_is_drawn_once_and_members_share_the_outcome() {
    let key = "c_lin.reaction_to_feeling@D7";
    let members = vec![
        exclusive("cand_1", key, &["avoid", "protect", "both", "none"]),
        // 组员：同一个决策键，但没有带分布
        exclusive("cand_2", key, &["avoid", "protect", "both", "none"]),
    ];
    let result = policy()
        .run(
            &AdjudicationInput::new(members)
                .with_distribution("cand_1", dist(&[("avoid", 0.25), ("protect", 0.25), ("both", 0.25), ("none", 0.25)])),
        )
        .unwrap();

    let first = result.resolution(&id("cand_1")).unwrap();
    let second = result.resolution(&id("cand_2")).unwrap();
    assert_eq!(first.policy, ResolutionPolicy::SeededCategorical);
    let option = result.selected_option(&id("cand_1")).unwrap();
    assert!(["avoid", "protect", "both", "none"].contains(&option));
    // 组内候选不单独掷骰：选中的选项和骰子值都一样
    assert_eq!(result.selected_option(&id("cand_2")), Some(option));
    assert_eq!(first.die, second.die);
    assert!(first.die.is_some());
    // 互斥组总是放行（选中了某一项），不会长趋势
    assert_eq!(result.accepted().len(), 2);
    assert!(result.tendencies.is_empty());
}

#[test]
fn the_group_leader_need_not_be_sorted_first() {
    // 带分布的是 cand_2，但按 ID 排序 cand_1 在前。按键索引之后，
    // 谁先被裁决都不影响结果。
    let key = "c_lin.reaction@D7";
    let result = policy()
        .run(
            &AdjudicationInput::new(vec![
                exclusive("cand_1", key, &["a", "b"]),
                exclusive("cand_2", key, &["a", "b"]),
            ])
            .with_distribution("cand_2", dist(&[("a", 1.0), ("b", 0.0)])),
        )
        .unwrap();
    assert_eq!(result.selected_option(&id("cand_1")), Some("a"));
    assert_eq!(result.selected_option(&id("cand_2")), Some("a"));
}

#[test]
fn an_exclusive_group_without_a_distribution_selects_nothing() {
    let result = policy()
        .run(&AdjudicationInput::new(vec![exclusive(
            "cand_1",
            "c_lin.reaction@D7",
            &["a", "b"],
        )]))
        .unwrap();
    let resolution = result.resolution(&id("cand_1")).unwrap();
    assert!(!resolution.accepted());
    assert!(resolution.die.is_none());
    assert_eq!(resolution.policy, ResolutionPolicy::SeededCategorical);
    assert_eq!(result.selected_option(&id("cand_1")), None);
}

// ---------------------------------------------------------------- L1 保护期

#[test]
fn a_protected_change_without_a_trigger_is_refused_even_with_high_probability() {
    let result = policy()
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", Some("c_lin.alive@D7"), &["p_alive"])])
                .with_probability("cand_1", 0.99)
                .protecting([PropositionId::new("p_alive")]),
        )
        .unwrap();
    let resolution = result.resolution(&id("cand_1")).unwrap();
    assert!(!resolution.accepted());
    assert_eq!(resolution.policy, ResolutionPolicy::Threshold);
    assert!(resolution.die.is_none());
    // 否决的理由是「缺依据」，不是「概率不够」——不该长成趋势
    assert!(result.tendencies.is_empty());
}

#[test]
fn a_triggered_protected_change_needs_the_high_threshold() {
    let input = |probability: f64| {
        AdjudicationInput::new(vec![occurs(
            "cand_1",
            Some("c_lin.alive@D7"),
            &["p_alive"],
        )])
        .with_probability("cand_1", probability)
        .protecting([PropositionId::new("p_alive")])
        .triggering("cand_1")
    };
    // 0.85 才允许（docs/06 §2 表末行）
    assert!(!policy().run(&input(0.84)).unwrap().resolutions[0].accepted());
    assert!(policy().run(&input(0.85)).unwrap().resolutions[0].accepted());
}

#[test]
fn protected_state_is_gated_even_when_the_dice_would_allow_it() {
    // 找一个骰子值小于 0.5 的键：若这个候选走了抽样，p = 0.5 必然放行。
    let policy = policy();
    let key = (0..500)
        .map(|i| candidate_key(&format!("k{i}"), MechanicStep::Day, day7()))
        .find(|key| policy.dice().roll(key) < 0.5)
        .expect("总能找到一个");
    assert!(policy
        .dice()
        .sample(&key, 0.5, crate::dice::NO_SALT));

    let result = policy
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", Some(&key), &["p_alive"])])
                .with_probability("cand_1", 0.5)
                .protecting([PropositionId::new("p_alive")])
                .triggering("cand_1"),
        )
        .unwrap();
    // 受保护状态不看骰子：0.5 < 0.85 → 挡住。
    assert!(!result.resolutions[0].accepted());
    assert!(result.resolutions[0].die.is_none());
}

// ---------------------------------------------------------------- 决策键

#[test]
fn a_candidate_without_a_key_falls_back_to_its_affected_proposition() {
    let candidate = occurs("cand_1", None, &["p_notices"]);
    let result = policy()
        .run(
            &AdjudicationInput::new(vec![candidate])
                .with_probability("cand_1", 0.5)
                .with_proposition_key("p_notices", "c_gu.notices.c_lin_abnormal@D7"),
        )
        .unwrap();
    assert_eq!(
        result.resolutions[0].decision_key.as_deref(),
        Some("c_gu.notices.c_lin_abnormal@D7")
    );
    assert!(result.unstable.is_empty());
    assert!(result.warnings.is_empty());
}

#[test]
fn a_candidate_with_no_stable_key_is_flagged() {
    let result = policy()
        .run(
            &AdjudicationInput::new(vec![occurs("cand_1", None, &[])])
                .with_probability("cand_1", 0.5),
        )
        .unwrap();
    assert_eq!(result.unstable, [id("cand_1")].into_iter().collect());
    assert_eq!(result.resolutions[0].decision_key.as_deref(), Some("cand_1"));
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("稳定决策键"));
    // 快照测试式的一句话，改动措辞时会被迫看一眼
    assert!(result.warnings[0].contains("1 个候选"));
}

// ---------------------------------------------------------------- 分层

#[test]
fn resolutions_follow_causal_layer_order() {
    let mut downstream = occurs("cand_3", Some("c_gu.leaves@D7"), &[]);
    downstream.depends_on = vec![id("cand_2")];
    let mut middle = occurs("cand_2", Some("c_gu.packs@D7"), &[]);
    middle.depends_on = vec![id("cand_1")];
    let first = occurs("cand_1", Some("c_gu.notices@D7"), &[]);

    let result = policy()
        .run(
            &AdjudicationInput::new(vec![downstream, middle, first])
                .with_probability("cand_1", 1.0)
                .with_probability("cand_2", 1.0)
                .with_probability("cand_3", 1.0),
        )
        .unwrap();

    assert_eq!(result.layers.len(), 3);
    assert_eq!(result.layers[0].depth, 0);
    assert_eq!(result.layers[2].depth, 2);
    let order: Vec<&str> = result
        .resolutions
        .iter()
        .map(|r| r.target.as_str())
        .collect();
    assert_eq!(order, ["cand_1", "cand_2", "cand_3"]);
}

#[test]
fn candidates_beyond_the_depth_cap_become_tendencies() {
    let mut chain = vec![occurs("cand_1", Some("k1@D7"), &[])];
    for n in 2..=4 {
        let mut candidate = occurs(&format!("cand_{n}"), Some(&format!("k{n}@D7")), &["p_deep"]);
        candidate.depends_on = vec![id(&format!("cand_{}", n - 1))];
        chain.push(candidate);
    }
    let mut input = AdjudicationInput::new(chain);
    for n in 1..=4 {
        input = input.with_probability(format!("cand_{n}"), 1.0);
    }
    let result = policy().run(&input).unwrap();

    assert_eq!(result.layers.len(), crate::layers::MAX_CAUSAL_DEPTH);
    assert_eq!(result.deferred, vec![id("cand_4")]);
    // 推迟的候选不进裁决记录
    assert!(result.resolution(&id("cand_4")).is_none());
    // 但落在观察带内，所以转成了趋势而不是凭空消失（docs/06 §4）
    assert_eq!(result.tendencies.len(), 1);
    assert_eq!(result.tendencies[0].id.as_str(), "tnd_k4@D7");
    assert_eq!(result.tendencies[0].target, "p_deep");
}

#[test]
fn a_dependency_cycle_is_reported_not_guessed_at() {
    let mut a = occurs("cand_1", Some("a@D7"), &[]);
    a.depends_on = vec![id("cand_2")];
    let mut b = occurs("cand_2", Some("b@D7"), &[]);
    b.depends_on = vec![id("cand_1")];
    assert!(matches!(
        policy().run(&AdjudicationInput::new(vec![a, b])),
        Err(LayerError::Cycle { .. })
    ));
}

// ---------------------------------------------------------------- 阈值与方向

#[test]
fn strictness_moves_the_gate_without_moving_the_dice() {
    // 0.25 < 0.3：默认放行
    assert!(policy().passes("q.beat.violates_fact", 0.25).unwrap());
    // 严格度 2.0 把阈值压到 0.18 → 同一份正文现在被判为违反
    assert!(!policy()
        .with_strictness(2.0)
        .passes("q.beat.violates_fact", 0.25)
        .unwrap());

    // 严格度只影响阈值，不该动抽样：同一输入下骰子与结论都不变
    let input = AdjudicationInput::new(vec![
        occurs("cand_1", Some("c_gu.notices@D7"), &[]),
        occurs("cand_2", Some("c_gu.leaves@D7"), &[]),
    ])
    .with_probability("cand_1", 0.6)
    .with_probability("cand_2", 0.4);
    let loose = policy().with_strictness(0.5).run(&input).unwrap();
    let strict = policy().with_strictness(2.0).run(&input).unwrap();
    assert_eq!(loose.resolutions, strict.resolutions);
    assert_eq!(loose.tendencies, strict.tendencies);
}
