use super::*;

#[test]
fn template_ids_are_versioned_and_unique() {
    assert_eq!(template_id("q.behavior.occurs"), "q.behavior.occurs@1");
    assert_eq!(template_id("q.behavior.occurs@1"), "q.behavior.occurs@1");
    assert_eq!(template_id("q.behavior.occurs@7"), "q.behavior.occurs@7");

    let mut seen = std::collections::BTreeSet::new();
    for template in TEMPLATES {
        assert!(template.contains('@'), "{template} 缺版本后缀");
        assert_eq!(template_id(normalize(template)), *template, "{template}");
        assert!(seen.insert(*template), "{template} 重复");
    }
    assert_eq!(seen.len(), TEMPLATES.len());
}

fn normalize(template: &str) -> &str {
    template.split('@').next().unwrap_or(template)
}

/// 每个构造器产出的问题都必须能通过本地预校验——否则就是等一个必然的 400（docs/14 §7.2）。
#[test]
fn every_builder_produces_a_valid_question() {
    let questions = vec![
        behavior_occurs("cand_1", "顾言", "推迟行程"),
        world_occurs("cand_2", "钟声停止"),
        perception_notices("cand_3", "顾言", "林夏换了座位", "林夏"),
        outcome_choice(
            "cand_4",
            "林夏听到表白",
            &[
                ("avoid".to_owned(), "回避".to_owned()),
                ("accept".to_owned(), "接受".to_owned()),
            ],
        ),
        cand_in_character("cand_5", "顾言", "当场翻脸"),
        cand_knowledge_gap("cand_6", "顾言", "提起林夏的秘密"),
        scene_fit(0, "雨夜的站台"),
        scene_tension(0, "雨夜的站台"),
        scene_advances_thread(0, "雨夜的站台", "顾言会不会离开", "他会离开吗"),
        scene_repetitive(0, "雨夜的站台", "站台对峙、雨中对峙"),
        scene_resolves_thread(0, "雨夜的站台", "顾言会不会离开", "他会离开吗"),
        beat_violates_fact(1, "city.no_lie", "城市里没有人能说谎"),
        beat_violates_rule(1, "rule_curfew", "宵禁", "入夜后坊门落锁"),
        beat_knowledge_leak(1, "顾言", "林夏最近避着他"),
        beat_forbidden_resolution(1, 0, "顾言当场离开"),
        beat_reveals_secret(1, "p_lin_crown", "林夏是王储"),
        beat_stop_reached(1, "顾言开始怀疑", &[]),
        beat_goal_done(1, "表现林夏的犹豫"),
        tendency_push("tnd_1", "顾言开始怀疑林夏"),
        thread_stage("thr_1", "顾言会不会离开", "他会离开吗"),
    ];
    for question in questions {
        question
            .validate()
            .unwrap_or_else(|e| panic!("{} 校验失败：{e}", question.template));
    }
}

/// 发生类必须写成「会 / 不会」，不能用「是否合理」代替（docs/06 §1 硬规则一）。
/// 这条规则无法机械校验措辞质量，但可以钉住构造器用的确实是 Noul + true/false 定义。
#[test]
fn occurrence_questions_are_noul_with_both_poles_named() {
    for question in [
        behavior_occurs("cand_1", "顾言", "推迟行程"),
        world_occurs("cand_2", "钟声停止"),
        perception_notices("cand_3", "顾言", "林夏换了座位", "林夏"),
    ] {
        match question.spec {
            QuestionSpec::Noul { true_means, false_means, instructions } => {
                assert!(true_means.is_some(), "{} 缺 true_means", question.template);
                assert!(false_means.is_some(), "{} 缺 false_means", question.template);
                assert!(instructions.contains("会"), "{} 的措辞没有落在「会不会」上", question.template);
            }
            other => panic!("{} 应当是 Noul，实际 {other:?}", question.template),
        }
    }
}

/// 两个约束门的**方向相反**，写错只会静默放过越界候选（docs/06 §1）。
/// 构造层管不了方向（方向在 if-policy 的阈值表里），但可以钉住它们确实是两条独立的问题。
#[test]
fn the_two_constraint_gates_ask_different_things() {
    let character = cand_in_character("cand_7", "顾言", "当场翻脸");
    let knowledge = cand_knowledge_gap("cand_7", "顾言", "当场翻脸");
    assert_ne!(character.key, knowledge.key);
    assert_ne!(character.template, knowledge.template);
    // 同一候选的两条问题共用一个 target，判定记录据此归并
    assert_eq!(character.target, knowledge.target);
}

#[test]
fn candidate_questions_key_on_the_candidate() {
    let question = behavior_occurs("cand_004", "顾言", "推迟行程");
    assert_eq!(question.key, "cand_004.occurs");
    assert_eq!(question.target, "cand_004");
    // 问题文本要同时带上主体与候选，否则判定者无从判断
    let instructions = question.spec.instructions();
    assert!(instructions.contains("顾言"));
    assert!(instructions.contains("推迟行程"));
}

#[test]
fn choice_criteria_is_a_record_and_score_criteria_is_a_list() {
    // 两种原语的 criteria 形态相反，写错直接 400（docs/14 §2）
    let choice = outcome_choice(
        "cand_8",
        "林夏听到表白",
        &[
            ("avoid".to_owned(), "回避".to_owned()),
            ("accept".to_owned(), "接受".to_owned()),
        ],
    );
    match &choice.spec {
        QuestionSpec::Choice { criteria, .. } => {
            assert_eq!(criteria.0.len(), 2);
            assert!(criteria.0.contains_key("avoid"));
        }
        other => panic!("期望 Choice，实际 {other:?}"),
    }

    let score = scene_tension(0, "雨夜的站台");
    match &score.spec {
        QuestionSpec::Score { criteria, .. } => {
            assert_eq!(criteria.0.len(), TENSION_LEVELS.len());
            assert_eq!(criteria.0[0], "明显缓和");
            assert_eq!(criteria.0[4], "明显升高");
        }
        other => panic!("期望 Score，实际 {other:?}"),
    }
}

/// 检查类问题**可以**说出秘密——检查视图本来就是为查泄露存在的（docs/08 §1）。
/// 受约束的是叙事视图与否决理由（docs/05 §2.5），不在这里。
#[test]
fn check_questions_may_name_the_secret() {
    let question = beat_reveals_secret(2, "p_lin_crown", "林夏是失踪的王储");
    assert!(question.spec.instructions().contains("林夏是失踪的王储"));
    assert_eq!(question.template, Q_BEAT_REVEALS_SECRET);
}

/// 节拍问题按 `beat_<n>.<检查项>.<对象>` 命名，同节拍的多个检查项共用一个 `target`。
///
/// 后缀是必须的：一个节拍要查的事实/规则/秘密都是**多条**，只写 `beat_3.fact`
/// 会让第二个覆盖第一个，`JudgeRequest::validate` 直接判为键重复并拒绝整个请求。
#[test]
fn beat_questions_are_namespaced_by_beat_index() {
    let fact = beat_violates_fact(3, "city.no_lie", "宵禁");
    let rule = beat_violates_rule(3, "rule_curfew", "没人能说谎", "只能沉默");
    assert_eq!(fact.key, "beat_3.fact.city.no_lie");
    assert_eq!(rule.key, "beat_3.rule.rule_curfew");
    assert_eq!(fact.target, rule.target);
    assert_eq!(fact.target, "beat_3");
}

/// 同一节拍的多条同类检查必须拿到**互不相同**的键（docs/07 §2 R1）。
#[test]
fn repeated_checks_on_one_beat_never_collide() {
    let questions = vec![
        beat_violates_fact(2, "c_lin.evidence", "那封信在林夏手上"),
        beat_violates_fact(2, "world.rain", "外面在下雨"),
        beat_violates_rule(2, "rule_night", "夜里宫门落锁", "仅限皇城宫门"),
        beat_violates_rule(2, "rule_curfew", "宵禁", ""),
        beat_forbidden_resolution(2, 0, "顾言当众承认"),
        beat_forbidden_resolution(2, 1, "林夏交出信"),
        beat_reveals_secret(2, "c_gu.secret", "顾言是失踪的王储"),
        beat_reveals_secret(2, "c_lin.evidence", "那封信在林夏手上"),
        beat_knowledge_leak(2, "林夏", "外面在下雨"),
        beat_knowledge_leak(2, "顾言", "外面在下雨"),
    ];
    let mut keys = std::collections::BTreeSet::new();
    for question in &questions {
        assert!(keys.insert(question.key.clone()), "键重复：{}", question.key);
    }
    assert_eq!(keys.len(), questions.len());
}

/// 停止条件的问题要带上此前已放行的节拍，否则「到这一节拍为止」无从判断。
#[test]
fn stop_reached_carries_the_prior_beats() {
    let empty = beat_stop_reached(0, "顾言开始怀疑", &[]);
    assert!(empty.spec.instructions().contains("第一个节拍"));
    let with_prior = beat_stop_reached(
        2,
        "顾言开始怀疑",
        &["雨落下来".to_owned(), "他抬头看天".to_owned()],
    );
    let text = with_prior.spec.instructions();
    assert!(text.contains("雨落下来"));
    assert!(text.contains("他抬头看天"));
}

#[test]
fn tendency_push_levels_match_the_domain_deltas() {
    assert_eq!(PUSH_LEVELS.len(), if_domain::TENDENCY_PUSH_DELTAS.len());
    let question = tendency_push("tnd_1", "顾言开始怀疑林夏");
    match &question.spec {
        QuestionSpec::Score { criteria, .. } => assert_eq!(criteria.0.len(), PUSH_LEVELS.len()),
        other => panic!("期望 Score，实际 {other:?}"),
    }
}
