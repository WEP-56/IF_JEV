use super::*;
use crate::testsupport as f;
use if_domain::event::{Event, EventDraft, Patch};
use if_domain::id::{PropositionId, SubjectId};
use if_domain::turn::Candidate;
use if_domain::value::{Lock, Value};
use if_judge::StubJudge;
use if_store::Store;

const LINE: &str = "wl_main";
const SCENE: &str = "scene_0001";
const DISPLAYED_AT: if_domain::narrative::Timestamp = 1_800_000_000_000;

// ---------------------------------------------------------------- 夹具

/// 让全流程能跑通的判定桩。
///
/// 「触发类」模板的方向是 `HigherFlags`（概率越高越可疑），而桩对未配置的问题返回 0.5，
/// 那在默认阈值下已经算违规——所以干净的一回合必须显式把它们压到 0。
fn judge() -> StubJudge {
    StubJudge::new()
        .noul_for_template("q.beat.violates_fact", 0.0)
        .noul_for_template("q.beat.violates_rule", 0.0)
        .noul_for_template("q.beat.knowledge_leak", 0.0)
        .noul_for_template("q.beat.forbidden_resolution", 0.0)
        .noul_for_template("q.beat.reveals_secret", 0.0)
        // 约束门：符合人物、不用到他不知道的信息
        .noul_for_template("q.cand.in_character", 0.9)
        .noul_for_template("q.cand.knowledge_gap", 0.1)
        // 停止条件达成 → 场景演一拍就收束
        .noul_for_template("q.beat.stop_reached", 0.9)
}

fn candidate() -> Candidate {
    Candidate {
        id: CandidateId::new("cand_01"),
        key: Some("cand_01.decision".into()),
        subject: Some(SubjectId::new(f::LIN)),
        content: "顾言把那封信的事说出口".into(),
        internal: false,
        shape: if_domain::turn::CandidateShape::Exclusive {
            options: vec!["拖延".into(), "摊牌".into()],
        },
        depends_on: Vec::new(),
        based_on: Vec::new(),
        affects: vec![PropositionId::new(f::P_EVIDENCE)],
    }
}

fn scene_proposal() -> SceneProposal {
    let lin = SubjectId::new(f::LIN);
    let gu = SubjectId::new(f::GU);
    SceneProposal::new(SceneId::new(SCENE), "雨夜的廊下，顾言开口")
        .cast([lin.clone(), gu.clone()], [lin, gu])
}

fn beats() -> Vec<BeatProposal> {
    vec![
        BeatProposal::new(1, "顾言开口了。"),
        BeatProposal::new(2, "林夏没有回答。"),
    ]
}

/// 从**事件**里长出那个世界。
///
/// 不用 `f::projection()` 直接插入：回合级重放要验的是「事件 → 投影 → 回合 → 事件」
/// 这条闭环，输入若是一份手搓的投影，折起来的东西就没被验过。
fn seed_world(store: &mut Store) {
    let world = f::projection();
    let line = WorldLineId::new(LINE);
    let turn = TurnId::numbered(0);
    let at = WorldTime::from_days(7);
    let first = store.next_seq().expect("内存库取号不会失败");

    let mut drafts: Vec<EventDraft> = Vec::new();
    for subject in world.subjects.values() {
        drafts.push(EventDraft::new(
            line.clone(),
            turn.clone(),
            at,
            Patch::SubjectCreated(Box::new(subject.clone())),
        ));
    }
    for proposition in world.propositions.values() {
        drafts.push(EventDraft::new(
            line.clone(),
            turn.clone(),
            at,
            Patch::PropositionCreated(Box::new(proposition.clone())),
        ));
    }
    for fact in world.facts.values() {
        drafts.push(EventDraft::new(
            line.clone(),
            turn.clone(),
            at,
            Patch::FactSet(Box::new(fact.clone())),
        ));
    }
    for rule in world.rules.values() {
        drafts.push(EventDraft::new(
            line.clone(),
            turn.clone(),
            at,
            Patch::RuleAdded(Box::new(rule.clone())),
        ));
    }
    for thread in world.threads.values() {
        drafts.push(EventDraft::new(
            line.clone(),
            turn.clone(),
            at,
            Patch::ThreadOpened(Box::new(thread.clone())),
        ));
    }

    // 「引入它的事件 ID」只有写入那一刻才定得下来，所以按批内位置回填——
    // 与 `if-app::seed` 是同一条契约（见 `commit` 模块文档）。
    for (index, draft) in drafts.iter_mut().enumerate() {
        let id = EventId::numbered(first + index as u64);
        match &mut draft.payload {
            Patch::SubjectCreated(subject) => subject.created_by = id,
            Patch::FactSet(fact) => fact.source = id,
            Patch::RuleAdded(rule) => rule.source = id,
            _ => {}
        }
    }

    store.append_batch(drafts).expect("播种世界");
}

struct Rig {
    store: Store,
    projection: Projection,
    settings: WorldSettings,
    ctx: TurnContext,
}

fn seeded_rig() -> Rig {
    let mut store = Store::open_in_memory().expect("内存库");
    store
        .create_world("测试世界", f::settings())
        .expect("建世界");
    seed_world(&mut store);
    let projection = store
        .load_projection(&WorldLineId::new(LINE))
        .expect("折叠投影");
    Rig {
        store,
        projection,
        settings: f::settings(),
        ctx: f::ctx(1),
    }
}

fn open_turn(rig: &Rig, judge: &StubJudge) -> Opening {
    open(
        judge,
        OpenRequest {
            projection: &rig.projection,
            settings: &rig.settings,
            ctx: &rig.ctx,
            candidates: vec![candidate()],
            scenes: vec![scene_proposal()],
            source_event: None,
            triggered: Vec::new(),
            temperature: None,
            salt: String::new(),
        },
    )
    .expect("第一段不该失败")
}

fn resolve_turn(rig: &mut Rig, judge: &StubJudge, opening: &Opening) -> SceneOutcome {
    let scene = opening
        .scene()
        .expect("只有一个场景候选，必然抽中")
        .id
        .clone();
    let outcome = resolve(
        judge,
        ResolveRequest {
            projection: &rig.projection,
            settings: &rig.settings,
            ctx: &rig.ctx,
            opening,
            scene,
            plan: f::plan("让顾言开口"),
            beats: beats(),
            observed: vec![ObservedChange {
                prop: PropositionId::new(f::P_EVIDENCE),
                value: Value::Bool(true),
                internal: false,
                text: "信还在她手上。".into(),
            }],
            new_propositions: Vec::new(),
            tendencies: Vec::new(),
            first_seq: rig.store.next_seq().expect("取号"),
            displayed_at: Some(DISPLAYED_AT),
            time: Some(WorldTime::from_days(8)),
        },
    )
    .expect("第二段不该失败");

    rig.store
        .append_batch(outcome.drafts.clone())
        .expect("写入回合");
    outcome
}

fn play(judge: &StubJudge) -> (Rig, Opening, SceneOutcome) {
    let mut rig = seeded_rig();
    let opening = open_turn(&rig, judge);
    let outcome = resolve_turn(&mut rig, judge, &opening);
    (rig, opening, outcome)
}

// ---------------------------------------------------------------- 引擎注入

#[test]
fn a_protected_thread_forbids_its_own_resolution() {
    let projection = f::projection();
    let mut plan = f::plan("让顾言开口");
    let notes = inject_constraints(&projection, &mut plan);

    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains(f::T_SHIELD));
    assert_eq!(plan.forbidden_resolutions.len(), 1);
    assert!(plan.forbidden_resolutions[0].contains("不得在本场景得到最终回答"));
}

#[test]
fn injecting_twice_does_not_duplicate_the_forbidden_list() {
    let projection = f::projection();
    let mut plan = f::plan("让顾言开口");
    inject_constraints(&projection, &mut plan);
    let second = inject_constraints(&projection, &mut plan);

    assert!(second.is_empty(), "已经在计划里的禁止项不再注入");
    assert_eq!(plan.forbidden_resolutions.len(), 1);
}

#[test]
fn a_thread_past_its_protection_adds_nothing() {
    let mut projection = f::projection();
    // 已经到高潮：保护期结束，引擎不再替它说话
    projection
        .threads
        .get_mut(&if_domain::id::ThreadId::new(f::T_SHIELD))
        .unwrap()
        .stage = if_domain::narrative::ThreadStage::Climax;
    let mut plan = f::plan("让顾言开口");
    assert!(inject_constraints(&projection, &mut plan).is_empty());
    assert!(plan.forbidden_resolutions.is_empty());
}

// ---------------------------------------------------------------- 两段驱动

#[test]
fn a_full_turn_runs_end_to_end() {
    let (rig, opening, outcome) = play(&judge());

    assert!(opening.scene().is_some(), "唯一候选必然抽中");
    assert!(outcome.completed(), "停止条件 0.9，第一拍就该收束");
    assert_eq!(outcome.beats.admitted.len(), 1);
    assert_eq!(outcome.beats.admitted[0].displayed_at, Some(DISPLAYED_AT));

    // 引擎把受保护线的禁止项写进了真正生效的那份计划
    assert_eq!(outcome.injections.len(), 1);
    assert_eq!(outcome.plan.forbidden_resolutions.len(), 1);

    // 同一份改动只提交一次：观察到的那条 + 预演里的同一条
    assert_eq!(outcome.reconciliation.committed.len(), 1);

    let projection = rig
        .store
        .load_projection(&WorldLineId::new(LINE))
        .expect("折叠");
    assert_eq!(projection.scenes.len(), 1);
    assert_eq!(projection.beats.len(), 1);
    assert_eq!(projection.world_time, WorldTime::from_days(8));
    assert_eq!(
        projection
            .facts
            .get(&PropositionId::new(f::P_EVIDENCE))
            .map(|fact| fact.value.clone()),
        Some(Value::Bool(true))
    );
    assert_eq!(
        projection
            .facts
            .get(&PropositionId::new(f::P_EVIDENCE))
            .map(|fact| fact.lock),
        Some(Lock::L0),
        "世界推演出来的变化是 L0——要升到 L2/L3 只能靠用户注入 IF"
    );
}

/// `events.scene` / `events.beat` 是**会落库并读回**的字段：回合写出去的事件
/// 必须带得上「属于哪个场景、哪一拍」，否则事后按场景查事件就是一片空。
#[test]
fn a_turn_stamps_its_scene_and_beat_on_every_event_it_writes() {
    let (rig, _, outcome) = play(&judge());
    let events = rig
        .store
        .events_for_line(&WorldLineId::new(LINE))
        .expect("读事件");
    let written = &events[events.len() - outcome.drafts.len()..];

    assert_eq!(written.len(), outcome.committed.len());
    for (event, id) in written.iter().zip(&outcome.committed) {
        assert_eq!(&event.id, id, "事件 ID 与 committed 必须一一对应");
        assert_eq!(
            event.scene.as_ref().map(|scene| scene.as_str()),
            Some(SCENE),
            "{} 没带上场景",
            event.id
        );
    }

    let displayed: Vec<&Event> = written
        .iter()
        .filter(|event| matches!(event.payload, Patch::BeatDisplayed(_)))
        .collect();
    assert_eq!(displayed.len(), 1);
    assert_eq!(
        displayed[0].beat.as_ref().map(|beat| beat.as_str()),
        Some("scene_0001_b01")
    );
    assert!(displayed[0].is_displayed(), "上屏事件必须有叙述顺序");
}

#[test]
fn resolve_refuses_a_scene_that_was_not_the_one_drawn() {
    let rig = seeded_rig();
    let judge = judge();
    let opening = open_turn(&rig, &judge);

    let error = resolve(
        &judge,
        ResolveRequest {
            projection: &rig.projection,
            settings: &rig.settings,
            ctx: &rig.ctx,
            opening: &opening,
            scene: SceneId::new("scene_9999"),
            plan: f::plan("让顾言开口"),
            beats: beats(),
            observed: Vec::new(),
            new_propositions: Vec::new(),
            tendencies: Vec::new(),
            first_seq: rig.store.next_seq().expect("取号"),
            displayed_at: None,
            time: None,
        },
    )
    .unwrap_err();

    assert!(matches!(error, PipelineError::Invalid(_)), "{error:?}");
    assert!(error.to_string().contains("第一段抽中的"));
}

#[test]
fn an_illegal_plan_stops_the_turn_before_any_beat_is_checked() {
    let rig = seeded_rig();
    let judge = judge();
    let opening = open_turn(&rig, &judge);
    let mut plan = f::plan("让顾言开口");
    // 视角人物不在场：领域层的硬校验必须在问判定之前就拦下
    plan.present = vec![SubjectId::new(f::GU)];

    let error = resolve(
        &judge,
        ResolveRequest {
            projection: &rig.projection,
            settings: &rig.settings,
            ctx: &rig.ctx,
            opening: &opening,
            scene: SceneId::new(SCENE),
            plan,
            beats: beats(),
            observed: Vec::new(),
            new_propositions: Vec::new(),
            tendencies: Vec::new(),
            first_seq: rig.store.next_seq().expect("取号"),
            displayed_at: None,
            time: None,
        },
    )
    .unwrap_err();

    assert!(matches!(error, PipelineError::ScenePlan(_)), "{error:?}");
}

#[test]
fn a_scene_that_leaves_the_present_set_empty_is_refused() {
    let mut plan = f::plan("让顾言开口");
    plan.present.clear();
    assert!(plan.validate().is_err());
}

// ---------------------------------------------------------------- 判定记录的号

/// 一个回合要经过三段判定。三段各自从 1 发号的话，`Beat::judgments` 里的
/// `jdg_0001` 会在 `TurnRecord` 里同时命中影响裁决的第一条——审计就查错了记录。
#[test]
fn judgment_ids_are_unique_across_the_whole_turn() {
    let (_, opening, outcome) = play(&judge());
    let record = record(
        TurnId::numbered(1),
        LINE,
        TurnKind::If,
        WorldTime::from_days(7),
        Some("让顾言开口".into()),
        Some(&opening),
        Some(&outcome),
    );

    assert!(record.judgments.len() > outcome.beats.judgments.len());
    let mut ids = std::collections::BTreeSet::new();
    for judgment in &record.judgments {
        assert!(
            ids.insert(judgment.id.as_str().to_owned()),
            "判定 ID 重复：{}",
            judgment.id
        );
    }

    // 节拍上记的每一个 ID 都要能在回合记录里唯一指回
    for beat in &outcome.beats.admitted {
        assert!(!beat.judgments.is_empty());
        for id in &beat.judgments {
            assert!(ids.contains(id.as_str()), "节拍上的判定 {id} 不在回合记录里");
        }
    }
}

/// 顺序：影响裁决（含约束门与发生类）→ 场景选择 → 逐节拍检查。
#[test]
fn the_audit_cursor_runs_across_all_three_stages_in_order() {
    let (_, opening, outcome) = play(&judge());

    let impact = opening
        .impact
        .judgments
        .iter()
        .map(|judgment| judgment.id.clone())
        .collect::<Vec<_>>();
    let scenes = opening
        .scenes
        .judgments
        .iter()
        .map(|judgment| judgment.id.clone())
        .collect::<Vec<_>>();
    let beats = outcome
        .beats
        .judgments
        .iter()
        .map(|judgment| judgment.id.clone())
        .collect::<Vec<_>>();

    assert_eq!(impact[0].as_str(), "jdg_0001");
    assert_eq!(scenes[0].as_str(), "jdg_0004", "场景选择接着影响裁决发号");
    assert_eq!(beats[0].as_str(), "jdg_0007", "节拍检查接着场景选择发号");
    assert_eq!(opening.audit.issued(), 6);
}

// ---------------------------------------------------------------- 重放

/// 把一条世界线重新折叠成投影。
fn folded(rig: &Rig) -> Projection {
    rig.store
        .load_projection(&WorldLineId::new(LINE))
        .expect("折叠")
}

/// 同一事件序列 + 同一裁决 → 同一个投影（docs/12 §7）。
///
/// 这条闭环是「可重放」这个产品承诺的最小可验证形式：它同时验了骰子的确定性、
/// 事件草稿的可折叠性，以及折叠结果只由事件决定。
#[test]
fn replaying_the_same_turn_reconstructs_the_same_projection() {
    let (once, _, _) = play(&judge());
    let (twice, _, _) = play(&judge());
    let once = folded(&once);
    let twice = folded(&twice);

    assert_eq!(once, twice);
    // 逐字节相同——连序列化的键序都必须是确定的
    assert_eq!(
        serde_json::to_string(&once).expect("投影可序列化"),
        serde_json::to_string(&twice).expect("投影可序列化")
    );
    // 而且它确实不是空世界：重放验的是「折出来的东西一样」，不是「都没折出来」
    assert_eq!(once.scenes.len(), 1);
    assert_eq!(once.beats.len(), 1);
}

#[test]
fn the_same_opening_is_drawn_twice() {
    let judge = judge();
    let rig = seeded_rig();
    let opening = open_turn(&rig, &judge);
    let again = open_turn(&rig, &judge);
    assert_eq!(opening.scenes.die, again.scenes.die);
    assert_eq!(opening.scenes.decision_key, again.scenes.decision_key);
    assert_eq!(
        opening.scenes.scene_id().map(|scene| scene.as_str()),
        again.scenes.scene_id().map(|scene| scene.as_str())
    );
}

// ---------------------------------------------------------------- 回合记录

#[test]
fn the_record_collects_both_halves() {
    let (_, opening, outcome) = play(&judge());
    let record = record(
        TurnId::numbered(1),
        LINE,
        TurnKind::If,
        WorldTime::from_days(7),
        Some("让顾言开口".into()),
        Some(&opening),
        Some(&outcome),
    );

    assert_eq!(record.id.as_str(), TurnId::numbered(1).as_str());
    assert_eq!(record.line.as_str(), LINE);
    assert_eq!(record.input.as_deref(), Some("让顾言开口"));
    assert_eq!(record.candidates.len(), 1);
    assert_eq!(record.committed, outcome.committed);
    assert_eq!(
        record.scene_plan.as_ref().map(|plan| plan.goal.as_str()),
        Some("让顾言开口")
    );
    assert_eq!(
        record
            .scene_plan
            .as_ref()
            .map(|plan| plan.forbidden_resolutions.len()),
        Some(1),
        "记录里存的是注入之后的计划"
    );
    // 判定记录按 ID 排序，保证同一输入产出同一份记录
    let mut sorted = record.judgments.clone();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(record.judgments, sorted);
}

#[test]
fn a_turn_without_an_opening_still_records_what_it_has() {
    let record = record(
        TurnId::numbered(2),
        LINE,
        TurnKind::Continue,
        WorldTime::from_days(7),
        None,
        None,
        None,
    );
    assert!(record.candidates.is_empty());
    assert!(record.judgments.is_empty());
    assert!(record.committed.is_empty());
    assert!(record.scene_plan.is_none());
}
