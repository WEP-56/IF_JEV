use super::*;
use crate::testsupport as f;
use if_domain::turn::ViewKind;
use if_judge::StubJudge;

const SCENE: &str = "scene_0001";

/// 让所有「触发类」检查判为清白的测试桩。
///
/// 这几条模板的方向是 `HigherFlags`——**概率越高越可疑**。测试桩对没配置的问题
/// 返回 0.5，那在默认阈值（0.3 / 0.35）下已经算违规，所以「干净的节拍」这个夹具
/// 必须显式把它们压到 0。`q.beat.stop_reached` 不配：默认 0.5 < 0.6，正好是「没达成」。
fn clean() -> StubJudge {
    StubJudge::new()
        .noul_for_template("q.beat.violates_fact", 0.0)
        .noul_for_template("q.beat.violates_rule", 0.0)
        .noul_for_template("q.beat.knowledge_leak", 0.0)
        .noul_for_template("q.beat.forbidden_resolution", 0.0)
        .noul_for_template("q.beat.reveals_secret", 0.0)
}

fn request<'a>(
    projection: &'a Projection,
    settings: &'a WorldSettings,
    ctx: &'a TurnContext,
    plan: &'a ScenePlan,
    beat: BeatProposal,
) -> BeatRequest<'a> {
    BeatRequest {
        projection,
        settings,
        ctx,
        plan,
        beat,
        prior: Vec::new(),
    }
}

fn one_beat_round(beats: Vec<BeatProposal>) -> (BeatRound, StubJudge) {
    one_beat_round_with(clean(), beats)
}

fn one_beat_round_with(judge: StubJudge, beats: Vec<BeatProposal>) -> (BeatRound, StubJudge) {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let round = run(
        &judge,
        BeatRoundRequest::new(&projection, &settings, &ctx, &plan, SCENE, beats),
        &mut Audit::new(),
    )
    .expect("节拍循环不该失败");
    (round, judge)
}

// ---------------------------------------------------------------- 检查本身

/// 每条能被挡下的模板都必须有一句**能说给模型听**的理由。
///
/// 这个测试钉的是一个已经踩过的坑：`reason_for` 拿「去掉版本的名字」去比带 `@1` 的模板
/// 常量，两边一个削了一个没削，全都不相等，于是所有否决理由静默退化成「未通过检查」——
/// 模型只知道「没过」，不知道「改哪儿」，只能瞎改。
#[test]
fn every_blocked_template_has_a_reason_for_the_model() {
    let mut seen = std::collections::BTreeSet::new();
    for (template, reason) in BLOCK_REASONS {
        assert!(seen.insert(question::template_name(template)), "{template} 重复");
        assert_eq!(reason_for(template), *reason, "{template} 的理由不对");
        // 带版本的写法也必须命中同一条
        assert_eq!(reason_for(&question::template_id(template)), *reason);
    }
    // 只有没登记的模板才落到兜底
    assert_eq!(reason_for("q.beat.not_a_thing@1"), "未通过检查");
    assert_eq!(reason_for("q.beat.not_a_thing"), "未通过检查");
}

#[test]
fn a_clean_beat_is_admitted() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean();
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "顾言终于开口，声音比平时低。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    assert!(verdict.admitted);
    assert!(verdict.blocks.is_empty());
    assert!(!verdict.stop_reached, "0.5 不到 0.6，不该判定场景结束");
    assert!(verdict.warnings.is_empty());

    // 只发一次请求，检出所有检查项：4 条事实 + 1 条规则 + 2 个在场角色 + 1 条秘密
    // + 停止条件 + 节拍目标
    let requests = judge.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].questions.len(), 10);
    assert_eq!(requests[0].view.meta.kind, ViewKind::Check);
    assert_eq!(requests[0].view.meta.holder, None, "检查视图没有持有者");
}

/// 检查项必须**逐条**发问：同一个节拍要查 4 条事实、1 条规则、1 条秘密，
/// 它们的键不能互相覆盖——否则 `validate()` 会判为重复并拒绝整个请求。
#[test]
fn every_check_item_gets_its_own_question_key() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean();
    check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "顾言终于开口。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    let ask = &judge.requests()[0];
    let keys: Vec<&str> = ask.questions.iter().map(|q| q.key.as_str()).collect();
    assert!(keys.contains(&"beat_1.fact.p_gu_secret"));
    assert!(keys.contains(&"beat_1.fact.p_lin_evidence"));
    assert!(keys.contains(&"beat_1.fact.p_lin_mood"));
    assert!(keys.contains(&"beat_1.fact.p_world_rain"));
    assert!(keys.contains(&"beat_1.rule.rule_night"));
    assert!(keys.contains(&"beat_1.leak.林夏"));
    assert!(keys.contains(&"beat_1.leak.顾言"));
    assert!(keys.contains(&"beat_1.secret.p_gu_secret"));
    assert!(keys.contains(&"beat_1.stop"));
    assert!(keys.contains(&"beat_1.goal"));

    let unique: std::collections::BTreeSet<&str> = keys.iter().copied().collect();
    assert_eq!(unique.len(), keys.len(), "问题键重复：{keys:?}");
}

#[test]
fn a_violating_beat_is_blocked_and_the_reason_names_the_visible_fact() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean().noul_for_template("q.beat.violates_fact", 0.9);
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "林夏把信烧了。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    assert!(!verdict.admitted);
    assert_eq!(verdict.blocks.len(), 4, "四条事实各挡一条");
    // 可见的那条能点名，模型才知道改哪里
    assert!(
        verdict
            .blocks
            .iter()
            .any(|block| block.reason.contains("那封信在林夏手上")),
        "{:?}",
        verdict.blocks
    );
    assert!(verdict.blocks.iter().all(|block| block.reason.contains("与已锁定的事实相矛盾")));
}

/// 否决理由回给的是**模型**，而模型会把理由原样写进下一稿。
/// 秘密一旦出现在理由里，第一次否决就把秘密交代了（docs/05 §2.5）。
#[test]
fn a_blocked_secret_never_appears_in_the_reason() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean().noul_for_template("q.beat.reveals_secret", 0.95);
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "顾言站在雨里，像在等人认领。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    assert!(!verdict.admitted);
    let block = verdict
        .blocks
        .iter()
        .find(|block| block.template.starts_with("q.beat.reveals_secret"))
        .expect("秘密检查必须挡下来");
    assert_eq!(block.reason, "涉及尚未批准揭示的内容");
    assert!(!block.reason.contains("顾言是失踪的王储"));
    // 问题本身是可以说出秘密的——否则判定者没法判断
    let ask = &judge.requests()[0];
    let question = ask.question("beat_1.secret.p_gu_secret").unwrap();
    assert!(question.spec.instructions().contains("顾言是失踪的王储"));
}

/// 秘密事实带来的「与事实矛盾」这一条也不许点名。
#[test]
fn a_secret_fact_block_does_not_name_it_either() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean().noul_for_template("q.beat.violates_fact", 0.9);
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "顾言把身世说了出来。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    assert!(
        verdict
            .blocks
            .iter()
            .all(|block| !block.reason.contains("顾言是失踪的王储")),
        "{:?}",
        verdict.blocks
    );
}

/// 缺判定按不通过处理（docs/06 §9）：宁可让模型重写一次，也不要让违规正文上屏。
#[test]
fn a_missing_judgment_blocks_the_beat() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean().drop_key("beat_1.fact.p_world_rain");
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "雨停了。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    assert!(!verdict.admitted);
    let block = verdict
        .blocks
        .iter()
        .find(|block| block.template.starts_with("q.beat.violates_fact"))
        .unwrap();
    assert_eq!(block.probability, None);
    assert!(verdict.warnings.iter().any(|w| w.contains("没拿到判定")));
    assert!(verdict.warnings.iter().any(|w| w.contains("按不通过处理")));
}

#[test]
fn goal_completion_is_reported_but_never_used_as_a_threshold() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    // 目标完成度给到满分：它只报原始概率，不据此放行或阻挡
    let judge = clean().noul_for_template("q.beat.goal_done", 1.0);
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(1, "顾言开口了。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();
    assert_eq!(verdict.goal_done, Some(1.0));
    assert!(verdict.admitted, "节拍目标不是阈值表项，不该挡住节拍");
}

#[test]
fn judgment_records_are_numbered_from_one() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean();
    let verdict = check(
        &judge,
        request(
            &projection,
            &settings,
            &ctx,
            &plan,
            BeatProposal::new(3, "顾言开口了。"),
        ),
        &mut Audit::new(),
    )
    .unwrap();

    assert_eq!(verdict.judgments.len(), 10);
    let ids: Vec<String> = verdict
        .judgments
        .iter()
        .map(|judgment| judgment.id.as_str().to_owned())
        .collect();
    assert_eq!(ids[0], "jdg_0001");
    assert_eq!(ids[9], "jdg_0010");
    assert_eq!(verdict.judgment_ids().len(), 10);
}

// ---------------------------------------------------------------- 循环

#[test]
fn the_stop_condition_ends_the_scene_and_later_beats_are_not_read() {
    let judge = clean().noul_for_template("q.beat.stop_reached", 0.9);
    let (round, judge) = one_beat_round_with(
        judge,
        vec![
            BeatProposal::new(1, "顾言把话说完。"),
            BeatProposal::new(2, "这一段不该被读到。"),
        ],
    );

    assert_eq!(round.stopped, Some(BeatStop::Condition));
    assert_eq!(round.admitted.len(), 1);
    assert_eq!(round.last_admitted(), Some(1));
    assert_eq!(judge.requests().len(), 1, "停止条件达成后不该再读后面的节拍");
}

#[test]
fn the_admitted_beat_carries_the_scene_name_and_its_judgment_ids() {
    let (round, _) = one_beat_round(vec![BeatProposal::new(3, "顾言开口了。")]);
    let beat = &round.admitted[0];
    assert_eq!(beat.id.as_str(), "scene_0001_b03");
    assert_eq!(beat.scene.as_str(), SCENE);
    assert_eq!(beat.index, 3);
    assert_eq!(beat.judgments.len(), 10);
    assert_eq!(round.displayed_text(), "顾言开口了。");
}

#[test]
fn a_required_beat_that_keeps_failing_stops_the_scene() {
    // 同一序号的三次尝试 = 首次 + 2 次重写（docs/04 §4.5）
    let attempts = vec![
        BeatProposal::new(1, "第一次尝试。").required(),
        BeatProposal::new(1, "第二次尝试。").required(),
        BeatProposal::new(1, "第三次尝试。").required(),
        BeatProposal::new(2, "后面的节拍。"),
    ];
    let judge = clean().noul_for_template("q.beat.violates_fact", 0.9);
    let (round, judge) = one_beat_round_with(judge, attempts);

    assert!(round.admitted.is_empty());
    assert_eq!(round.blocked.len(), 3, "每次尝试都留一条被否决的记录");
    assert_eq!(round.ledger.attempts(1), 3);
    assert!(matches!(
        round.stopped,
        Some(BeatStop::Rejected { index: 1, exhausted: true, .. })
    ));
    assert_eq!(judge.requests().len(), 3, "必需节拍用尽重试后不再往下读");
    assert_eq!(round.last_admitted(), None);
}

#[test]
fn a_non_required_beat_is_skipped_after_its_retries() {
    let judge = clean()
        .noul_for_template("q.beat.violates_fact", 0.0)
        .noul_for_key("beat_1.fact.p_lin_evidence", 0.9);
    let (round, _) = one_beat_round_with(
        judge,
        vec![
            BeatProposal::new(1, "第一次。"),
            BeatProposal::new(1, "第二次。"),
            BeatProposal::new(1, "第三次。"),
            BeatProposal::new(2, "顾言终于开口。"),
        ],
    );

    assert_eq!(round.admitted.len(), 1);
    assert_eq!(round.last_admitted(), Some(2));
    assert_eq!(round.blocked.len(), 3);
    assert!(
        round
            .warnings
            .iter()
            .any(|w| w.contains("作为非必需节拍跳过")),
        "{:?}",
        round.warnings
    );
    // 提议用完了，没有停止条件也没有触顶
    assert_eq!(round.stopped, Some(BeatStop::Exhausted));
}

#[test]
fn the_scene_stops_at_the_beat_limit() {
    let projection = f::projection();
    let settings = f::settings();
    let ctx = f::ctx(1);
    let plan = f::plan("让顾言开口");
    let judge = clean();
    let beats = (1..=5)
        .map(|index| BeatProposal::new(index, format!("第 {index} 拍。")))
        .collect();
    let mut request = BeatRoundRequest::new(&projection, &settings, &ctx, &plan, SCENE, beats);
    request.max_beats = 2;

    let round = run(&judge, request, &mut Audit::new()).unwrap();
    assert_eq!(round.admitted.len(), 2);
    assert_eq!(round.stopped, Some(BeatStop::Limit));
    assert_eq!(judge.requests().len(), 2, "触顶之后不该再检查第三个节拍");
}

#[test]
fn prior_beats_travel_with_the_stop_question() {
    let (_, judge) = one_beat_round(vec![
        BeatProposal::new(1, "雨落下来。"),
        BeatProposal::new(2, "他抬头看天。"),
    ]);

    let requests = judge.requests();
    assert_eq!(requests.len(), 2);
    let first = requests[0].question("beat_1.stop").unwrap();
    assert!(first.spec.instructions().contains("第一个节拍"));
    let second = requests[1].question("beat_2.stop").unwrap();
    assert!(second.spec.instructions().contains("雨落下来。"));
    assert!(!second.spec.instructions().contains("他抬头看天。"), "当前节拍不算进 prior");
}

#[test]
fn an_empty_proposal_list_finishes_without_asking_anything() {
    let (round, judge) = one_beat_round(Vec::new());
    assert!(round.admitted.is_empty());
    assert_eq!(round.stopped, Some(BeatStop::Exhausted));
    assert!(judge.requests().is_empty());
    assert_eq!(round.displayed_text(), "");
}

#[test]
fn the_same_beats_yield_the_same_round() {
    let beats = || {
        vec![
            BeatProposal::new(1, "雨落下来。"),
            BeatProposal::new(2, "他抬头看天。"),
        ]
    };
    let (once, _) = one_beat_round(beats());
    let (twice, _) = one_beat_round(beats());
    assert_eq!(once, twice);
}

#[test]
fn the_retry_ledger_counts_per_index() {
    let ledger = RetryLedger::default();
    assert_eq!(ledger.attempts(1), 0);
    assert!(ledger.can_retry(1));

    let (round, _) = one_beat_round_with(
        clean().noul_for_template("q.beat.violates_fact", 0.9),
        vec![
            BeatProposal::new(1, "第一次。"),
            BeatProposal::new(1, "第二次。"),
            BeatProposal::new(1, "第三次。"),
        ],
    );
    assert_eq!(round.ledger.attempts(1), 3);
    assert!(!round.ledger.can_retry(1));
    assert!(round.ledger.can_retry(2), "没见过的序号还能试");
}
