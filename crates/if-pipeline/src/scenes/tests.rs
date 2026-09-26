use super::*;
use crate::testsupport as f;
use if_domain::id::TendencyId;
use if_domain::narrative::{Thread, ThreadStage};
use if_domain::value::WorldTime;
use if_judge::StubJudge;

/// 一条不受保护的故事线，用来让「未受保护」那条分支也能被走到。
fn unprotected_thread(projection: &mut Projection, id: &str) {
    projection.threads.insert(
        ThreadId::new(id),
        Thread {
            id: ThreadId::new(id),
            title: "那封信会不会被交出去".into(),
            question: "林夏最终会把信交出去吗".into(),
            stakes: "交出去意味着背叛".into(),
            subjects: vec![SubjectId::new(f::LIN)],
            stage: ThreadStage::Developing,
            protected_until: None,
            pressure: 0.6,
            last_advanced: 0,
            cadence: 3.0,
        },
    );
}

#[test]
fn a_scene_that_resolves_a_protected_thread_is_vetoed_before_any_judgment() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let judge = StubJudge::new();
    let scenes = vec![SceneProposal::new(f::scene_id("scene_1"), "顾言当众承认自己是王储")
        .resolves([ThreadId::new(f::T_SHIELD)])];
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes),
        &mut Audit::new(),
    )
    .unwrap();
    assert!(choice.is_empty(), "受保护的线必须被硬否决");
    assert_eq!(choice.vetoed.len(), 1);
    assert!(choice.vetoed[0].reason.contains("不得在本场景得到最终回答"));
    assert!(judge.requests().is_empty(), "被否决的场景不该再花一次判定");
}

#[test]
fn one_veto_does_not_take_the_other_candidates_with_it() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let judge = StubJudge::new().noul_for_template("q.scene.fit", 0.8);
    let scenes = vec![
        SceneProposal::new(f::scene_id("scene_1"), "顾言当众承认自己是王储")
            .resolves([ThreadId::new(f::T_SHIELD)]),
        SceneProposal::new(f::scene_id("scene_2"), "林夏独自在廊下站了很久"),
    ];
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes),
        &mut Audit::new(),
    )
    .unwrap();
    assert_eq!(choice.vetoed.len(), 1);
    assert_eq!(choice.scene_id().map(|id| id.as_str()), Some("scene_2"));
}

#[test]
fn an_unprotected_thread_scores_as_a_penalty_instead_of_a_veto() {
    let mut projection = f::projection();
    unprotected_thread(&mut projection, "thr_letter");
    let ctx = f::ctx(1);
    // 会收束未受保护的线：不该被否决，但会被扣分。
    let judge = StubJudge::new()
        .noul_for_template("q.scene.fit", 0.9)
        .noul_for_template("q.scene.resolves_thread", 0.9);
    let scene = SceneProposal::new(f::scene_id("scene_1"), "林夏把信交了出去")
        .resolves([ThreadId::new("thr_letter")]);
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, vec![scene]),
        &mut Audit::new(),
    )
    .unwrap();
    assert!(choice.vetoed.is_empty());
    let score = choice.chosen.as_ref().expect("只有一个候选，必然选中");
    assert!(
        (score.signals.premature_resolution - 0.9).abs() < 1e-9,
        "过早收束要进评分"
    );
    // 0.15 的温度下，这一项扣分足以让总分明显偏低
    assert!(score.score < 1.0);
}

#[test]
fn thread_pressure_and_overdue_come_from_the_engine() {
    let projection = f::projection();
    let ctx = f::ctx(6);
    let judge = StubJudge::new();
    let scene = SceneProposal::new(f::scene_id("scene_1"), "顾言被叫去问话")
        .threads([ThreadId::new(f::T_SHIELD)]);
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, vec![scene]),
        &mut Audit::new(),
    )
    .unwrap();
    let signals = choice.chosen.unwrap().signals;
    assert!((signals.thread_pressure - 0.5).abs() < 1e-9);
    // 受保护的线仍在推进区间里：cadence 3，上次推进在第 0 个场景
    assert!(signals.overdue_bonus > 0.0);
}

#[test]
fn a_declared_eruption_that_is_not_high_pressure_is_reported() {
    let mut projection = f::projection();
    let mut calm = f::tendency("tnd_calm", "林夏开始怀疑顾言", 0.2);
    calm.threshold = if_domain::TENDENCY_ERUPT_THRESHOLD;
    projection.tendencies.insert(TendencyId::new("tnd_calm"), calm);
    let ctx = f::ctx(1);
    let judge = StubJudge::new();
    let scene = SceneProposal::new(f::scene_id("scene_1"), "林夏问起那封信")
        .erupts([TendencyId::new("tnd_calm")]);
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, vec![scene]),
        &mut Audit::new(),
    )
    .unwrap();
    assert_eq!(choice.chosen.unwrap().signals.tendency_bonus, 0.0);
    assert!(choice.warnings.iter().any(|w| w.contains("还没到爆发阈值")));
}

#[test]
fn a_high_pressure_tendency_gives_the_scene_its_bonus() {
    let mut projection = f::projection();
    // `Tendency::latent` 只把概率折算成初始压力（×0.5），所以这里要显式把压力抬到阈值之上
    let mut hot = f::tendency("tnd_hot", "顾言快藏不住了", 0.9);
    hot.threshold = if_domain::TENDENCY_ERUPT_THRESHOLD;
    hot.pressure = if_domain::TENDENCY_ERUPT_THRESHOLD;
    projection.tendencies.insert(TendencyId::new("tnd_hot"), hot);
    let ctx = f::ctx(1);
    let judge = StubJudge::new();
    let scene = SceneProposal::new(f::scene_id("scene_1"), "顾言说漏了嘴")
        .erupts([TendencyId::new("tnd_hot")]);
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, vec![scene]),
        &mut Audit::new(),
    )
    .unwrap();
    assert_eq!(choice.chosen.unwrap().signals.tendency_bonus, 1.0);
}

#[test]
fn probabilities_sum_to_one_and_every_candidate_is_accounted_for() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let judge = StubJudge::new().noul_for_template("q.scene.fit", 0.7);
    let scenes = vec![
        SceneProposal::new(f::scene_id("scene_1"), "林夏独自在廊下"),
        SceneProposal::new(f::scene_id("scene_2"), "顾言被叫去问话"),
        SceneProposal::new(f::scene_id("scene_3"), "两人在雨中相遇"),
    ];
    let choice = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes),
        &mut Audit::new(),
    )
    .unwrap();
    assert_eq!(choice.road_not_taken.len(), 2);
    let total: f64 = choice.road_not_taken.iter().map(|s| s.probability).sum::<f64>()
        + choice.chosen.as_ref().unwrap().probability;
    assert!((total - 1.0).abs() < 1e-9, "{total}");
}

#[test]
fn the_same_input_yields_the_same_scene() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let scenes = vec![
        SceneProposal::new(f::scene_id("scene_1"), "林夏独自在廊下"),
        SceneProposal::new(f::scene_id("scene_2"), "顾言被叫去问话"),
    ];
    let once = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes.clone()),
        &mut Audit::new(),
    )
    .unwrap();
    let twice = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes),
        &mut Audit::new(),
    )
    .unwrap();
    assert_eq!(once.scene_id(), twice.scene_id());
    assert_eq!(once.die, twice.die);
    assert_eq!(once.decision_key, twice.decision_key);
}

#[test]
fn a_reroll_salt_changes_the_die_but_stays_reproducible() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let scenes = vec![SceneProposal::new(f::scene_id("scene_1"), "林夏独自在廊下")];
    let plain = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes.clone()),
        &mut Audit::new(),
    )
    .unwrap();
    let salted = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes.clone()).salt("reroll-1"),
        &mut Audit::new(),
    )
    .unwrap();
    let again = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes).salt("reroll-1"),
        &mut Audit::new(),
    )
    .unwrap();
    assert_ne!(plain.die, salted.die, "换盐值必须换骰子");
    assert_eq!(salted.die, again.die, "同一个盐值必须可复现");
}

#[test]
fn the_narrative_order_is_what_makes_the_selection_key() {
    let projection = f::projection();
    let judge = StubJudge::new();
    let scenes = vec![SceneProposal::new(f::scene_id("scene_1"), "林夏独自在廊下")];
    let first = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &f::ctx(1), scenes.clone()),
        &mut Audit::new(),
    )
    .unwrap();
    let later = choose(
        &judge,
        SceneRequest::new(&projection, &f::settings(), &f::ctx(4), scenes),
        &mut Audit::new(),
    )
    .unwrap();
    // 场景选择发生在时间被推进之前，所以键用叙述序号，不用世界时间
    assert_eq!(first.decision_key.as_deref(), Some("scene_select@3"));
    assert_eq!(later.decision_key.as_deref(), Some("scene_select@3"));
}

#[test]
fn no_candidates_at_all_is_not_an_error() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let choice = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, Vec::new()),
        &mut Audit::new(),
    )
    .unwrap();
    assert!(choice.is_empty());
    assert!(choice.warnings.iter().any(|w| w.contains("没有可用的场景候选")));
}

#[test]
fn a_scene_resolving_an_unknown_thread_only_warns() {
    let projection = f::projection();
    let ctx = f::ctx(1);
    let scenes = vec![SceneProposal::new(f::scene_id("scene_1"), "林夏独自在廊下")
        .resolves([ThreadId::new("thr_missing")])];
    let choice = choose(
        &StubJudge::new(),
        SceneRequest::new(&projection, &f::settings(), &ctx, scenes),
        &mut Audit::new(),
    )
    .unwrap();
    assert!(choice.vetoed.is_empty());
    assert!(choice.warnings.iter().any(|w| w.contains("没有这条线")));
}

#[test]
fn the_relation_between_scene_count_and_appearances_is_computed_from_the_projection() {
    let mut projection = f::projection();
    // 一个已经演过的场景，林夏在场、顾言不在。
    // 注意 `f::plan` 的 present 是「林夏 + 顾言」，这里必须换成只有林夏的那一份，
    // 否则测的是「两个人各出场一次」。
    let alone = if_domain::narrative::ScenePlan {
        present: vec![SubjectId::new(f::LIN)],
        ..f::plan("林夏独自在廊下")
    };
    projection.scenes.insert(
        SceneId::new("scene_0001"),
        if_domain::narrative::Scene {
            id: SceneId::new("scene_0001"),
            index: 0,
            plan: alone,
            started_at: WorldTime::from_days(6),
            completed_at: Some(WorldTime::from_days(7)),
        },
    );
    let appearances = recent_appearances(&projection, 6);
    assert_eq!(appearances.get(&SubjectId::new(f::LIN)), Some(&1));
    assert_eq!(appearances.get(&SubjectId::new(f::GU)), None);
    // 焦点角色里两个都缺额：林夏出场一次（缺额 1−1/6），顾言零次（缺额 1）
    let balance = director::character_balance(
        &[SubjectId::new(f::LIN), SubjectId::new(f::GU)],
        &appearances,
        6,
    );
    assert!((balance - (1.0 - 1.0 / 6.0 + 1.0) / 2.0).abs() < 1e-9);
}
