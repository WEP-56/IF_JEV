//! 回放、世界线与快照的集成测试（docs/12 §8）。
//!
//! 这些测试全部只用公开 API，跑的是真实的 SQLite——包括内存库与文件库。

use if_domain::event::{EventType, Patch};
use if_domain::id::{PropositionId, SceneId, SubjectId, ThreadId, TurnId, WorldLineId};
use if_domain::narrative::{Beat, Scene, ScenePlan, Thread, ThreadStage};
use if_domain::rule::{WorldSettings, L1_PROTECTION_SCENES};
use if_domain::state::{Belief, Fact};
use if_domain::subject::{Proposition, PropositionKind, Subject, SubjectKind, ValueType};
use if_domain::value::{Lock, Value, Visibility, WorldTime};
use if_domain::worldline::WorldLineKind;
use if_store::{EventDraft, Store, SNAPSHOT_INTERVAL};

const MAIN: &str = "wl_main";

fn main_line() -> WorldLineId {
    WorldLineId::new(MAIN)
}

fn prop(id: &str, key: &str, internal: bool) -> Proposition {
    Proposition {
        id: PropositionId::new(id),
        key: key.to_owned(),
        text: key.to_owned(),
        subjects: vec![SubjectId::new("c_lin")],
        kind: PropositionKind::State,
        value_type: ValueType::Bool,
        internal,
    }
}

fn subject(id: &str, name: &str) -> Subject {
    Subject::new(id, SubjectKind::Character, name)
}

fn scene_plan(pov: &str, present: &[&str]) -> ScenePlan {
    ScenePlan {
        goal: "表现林夏的犹豫".into(),
        pov: SubjectId::new(pov),
        focus: vec![SubjectId::new("c_gu")],
        present: present.iter().map(|s| SubjectId::new(*s)).collect(),
        time_span: "当晚".into(),
        required_beats: vec!["顾言收拾行李".into()],
        stop_condition: "顾言开始怀疑林夏改变行动的真正原因".into(),
        forbidden_resolutions: vec!["不得表白".into()],
        reveal_allowed: vec![],
        proposed_changes: vec![],
    }
}

/// 建一个「雨城」世界的开头，返回主世界线。
fn seed_world(store: &mut Store) -> WorldLineId {
    let settings = WorldSettings {
        seed: 77492,
        mechanic_step: if_domain::rule::MechanicStep::Day,
        ..Default::default()
    };
    store.create_world("雨城", settings).unwrap();

    let line = main_line();
    let turn = TurnId::new("turn_0001");

    let drafts = vec![
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::EPOCH,
            Patch::SubjectCreated(Box::new(subject("c_lin", "林夏"))),
        ),
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::EPOCH,
            Patch::SubjectCreated(Box::new(subject("c_gu", "顾言"))),
        ),
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::EPOCH,
            Patch::PropositionCreated(Box::new(prop("p_feeling", "c_lin.feeling.c_gu", true))),
        ),
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::EPOCH,
            Patch::PropositionCreated(Box::new(prop("p_leave", "c_gu.will_leave", false))),
        ),
        // IF 注入：林夏爱上顾言（状态型，L1，保护 3 个场景）
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::from_hours(1),
            Patch::FactSet(Box::new(
                Fact::new(
                    "p_feeling",
                    "爱",
                    WorldTime::from_hours(1),
                    Lock::L1,
                    "evt_if",
                )
                .with_visibility(Visibility::Private)
                .with_protection(L1_PROTECTION_SCENES),
            )),
        ),
        // 顾言不知道自己被爱上（Belief 与 Reality 分离）
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::from_hours(1),
            Patch::ThreadOpened(Box::new(Thread {
                id: ThreadId::new("thr_leave"),
                title: "顾言会不会离开".into(),
                question: "顾言最终会离开雨城吗".into(),
                stakes: "离开意味着两个孩子再也见不到".into(),
                subjects: vec![SubjectId::new("c_gu")],
                stage: ThreadStage::Seeded,
                protected_until: Some(ThreadStage::Climax),
                pressure: 0.2,
                last_advanced: 0,
                cadence: 3.0,
            })),
        ),
    ];
    store.append_batch(drafts).unwrap();
    line
}

/// 走一个场景：开始 → 两个节拍 → 结束。
fn run_scene(store: &mut Store, line: &WorldLineId, turn: &TurnId, scene_no: u64) {
    let scene_id = SceneId::new(format!("scene_{scene_no}"));
    let started = WorldTime::from_hours(2 * scene_no as i64);

    store
        .append(EventDraft::new(
            line.clone(),
            turn.clone(),
            started,
            Patch::SceneStarted(Box::new(Scene {
                id: scene_id.clone(),
                index: scene_no - 1,
                plan: scene_plan("c_lin", &["c_lin", "c_gu"]),
                started_at: started,
                completed_at: None,
            })),
        ))
        .unwrap();

    for beat_no in 0..2u32 {
        let beat = Beat {
            id: if_domain::id::BeatId::new(format!("beat_{scene_no}_{beat_no}")),
            scene: scene_id.clone(),
            index: beat_no,
            text: format!("第 {scene_no} 场第 {beat_no} 节拍"),
            plan_beat: Some(0),
            judgments: vec![],
            displayed_at: Some(1_700_000_000_000 + beat_no as i64),
        };
        let order = store.load_projection(line).unwrap().narrative_order + 1;
        store
            .append(
                EventDraft::new(
                    line.clone(),
                    turn.clone(),
                    started.plus_minutes(5 * (beat_no as i64 + 1)),
                    Patch::BeatDisplayed(Box::new(beat)),
                )
                .displayed(order)
                .in_scene(scene_id.clone()),
            )
            .unwrap();
    }

    store
        .append(EventDraft::new(
            line.clone(),
            turn.clone(),
            started.plus_minutes(30),
            Patch::SceneCompleted {
                scene: scene_id,
                at: started.plus_minutes(30),
            },
        ))
        .unwrap();
}

// ---------------------------------------------------------------- 测试

#[test]
fn replay_of_the_same_events_yields_the_same_projection() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    for scene in 1..=3 {
        run_scene(&mut store, &line, &TurnId::new("turn_0002"), scene);
    }

    let a = store.load_projection(&line).unwrap();
    let b = store.load_projection(&line).unwrap();
    assert_eq!(a, b, "同一事件序列必须得到同一个投影");

    // 序列化也必须逐字节一致——键序固定是确定性的前提（docs/12 §7）
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );

    // 换个新开的库，从同一份日志重放，结果仍应一致
    let c = store.load_projection_from_scratch(&line).unwrap();
    assert_eq!(a, c);
}

#[test]
fn world_seed_is_persisted_and_part_of_the_projection() {
    let mut store = Store::open_in_memory().unwrap();
    seed_world(&mut store);
    assert_eq!(store.world_seed().unwrap(), Some(77492));
    let projection = store.load_projection(&main_line()).unwrap();
    assert_eq!(projection.world_seed, Some(77492));
}

#[test]
fn events_are_append_only_and_heads_never_point_into_the_future() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    let before = store.event_count().unwrap();
    assert!(before > 0);

    // 头指针不能跑到还不存在的序号
    let err = store.set_head(&line, 99_999).unwrap_err();
    assert!(format!("{err}").contains("头指针"));

    // 追加只增不减
    run_scene(&mut store, &line, &TurnId::new("turn_0002"), 1);
    assert!(store.event_count().unwrap() > before);

    // seq 连续递增，id 与 seq 对应
    let events = store.events_for_line(&line).unwrap();
    for (i, event) in events.iter().enumerate() {
        assert_eq!(event.seq, i as u64 + 1);
        assert_eq!(event.id.as_str(), format!("evt_{:04}", i + 1));
    }
}

#[test]
fn snapshot_fold_equals_full_fold() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    let turn = TurnId::new("turn_0002");

    // 把事件数推过快照间隔，逼出一次自动快照
    let filler = Patch::IfConflictResolved {
        note: "填充事件".into(),
    };
    let current = store.event_count().unwrap();
    for i in current..SNAPSHOT_INTERVAL {
        store
            .append(EventDraft::new(
                line.clone(),
                turn.clone(),
                WorldTime::from_minutes(i as i64),
                filler.clone(),
            ))
            .unwrap();
    }
    assert_eq!(store.event_count().unwrap(), SNAPSHOT_INTERVAL);
    assert!(
        store.latest_snapshot(&line).unwrap().is_some(),
        "到达快照间隔后应自动存一份快照"
    );

    // 快照之后继续写，让「快照 + 增量」和「全量折叠」两条路都非空
    run_scene(&mut store, &line, &turn, 1);

    let via_snapshot = store.load_projection(&line).unwrap();
    let from_scratch = store.load_projection_from_scratch(&line).unwrap();
    assert_eq!(
        via_snapshot, from_scratch,
        "从快照续折必须与全量折叠等价"
    );
}

#[test]
fn fork_inherits_history_but_isolates_later_events() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    let turn = TurnId::new("turn_0002");
    run_scene(&mut store, &line, &turn, 1);

    let fork_point = store.world_line(&line).unwrap().unwrap().head_seq;
    let branch = store
        .fork(&line, fork_point, WorldLineKind::Branch, "换一种走法")
        .unwrap();

    // 分支上写一个新事实
    store
        .append(EventDraft::new(
            branch.id.clone(),
            turn.clone(),
            WorldTime::from_days(2),
            Patch::FactSet(Box::new(
                Fact::new(
                    "p_leave",
                    true,
                    WorldTime::from_days(2),
                    Lock::L0,
                    "evt_branch",
                )
                .with_visibility(Visibility::Public),
            )),
        ))
        .unwrap();

    let child = store.load_projection(&branch.id).unwrap();
    let parent = store.load_projection(&line).unwrap();

    // 分支继承了分叉点之前的全部历史
    assert!(child.subject(&SubjectId::new("c_lin")).is_some());
    assert_eq!(child.beats.len(), parent.beats.len());
    // 但分支上写的事实不会渗回主线
    assert_eq!(child.fact_value(&PropositionId::new("p_leave")), Some(&Value::Bool(true)));
    assert!(parent.fact_value(&PropositionId::new("p_leave")).is_none());
    assert!(child.anchor.seq > parent.anchor.seq);
}

#[test]
fn rollback_moves_the_head_and_keeps_the_retired_events_on_an_abandoned_line() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    let turn = TurnId::new("turn_0002");
    run_scene(&mut store, &line, &turn, 1);

    let keep_until = store.world_line(&line).unwrap().unwrap().head_seq;
    run_scene(&mut store, &line, &turn, 2);
    let after_two_scenes = store.event_count().unwrap();

    let fresh = store.rollback_to(&line, keep_until).unwrap();

    // 旧线降级为 abandoned，并且**事件一条都没丢**
    let lines = store.world_lines().unwrap();
    assert_eq!(lines.get(&line).unwrap().kind, WorldLineKind::Abandoned);
    assert_eq!(store.event_count().unwrap(), after_two_scenes);

    // 新线的头指针回到回滚点
    assert_eq!(lines.get(&fresh.id).unwrap().head_seq, keep_until);
    let rolled_back = store.load_projection(&fresh.id).unwrap();
    assert_eq!(rolled_back.scenes.len(), 1, "第二场应当被退回");
    assert_eq!(store.active_line().unwrap(), Some(fresh.id.clone()));

    // 被退回的事件仍能在旧线上看到——这正是「保留分支」的意义
    let abandoned = store.load_projection(&line).unwrap();
    assert_eq!(abandoned.scenes.len(), 2);
}

#[test]
fn retcon_backup_creates_a_frozen_line() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    run_scene(&mut store, &line, &TurnId::new("turn_0002"), 1);

    let backup = store.backup_before_retcon(&line).unwrap();
    assert_eq!(backup.kind, WorldLineKind::RetconBackup);
    assert_eq!(backup.parent.as_ref().unwrap().line, line);

    // 备份分支不接受新事件
    let err = store
        .append(EventDraft::new(
            backup.id.clone(),
            TurnId::new("turn_0003"),
            WorldTime::from_days(9),
            Patch::IfConflictResolved { note: "x".into() },
        ))
        .unwrap_err();
    assert!(format!("{err}").contains("不接受新事件"));
}

#[test]
fn a_bad_event_in_a_batch_writes_nothing() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    let before = store.event_count().unwrap();

    let turn = TurnId::new("turn_0009");
    let result = store.append_batch(vec![
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::from_days(3),
            Patch::FactSet(Box::new(Fact::new(
                "p_leave",
                true,
                WorldTime::from_days(3),
                Lock::L0,
                "evt_ok",
            ))),
        ),
        // 这条引用了不存在的命题，整批必须回滚
        EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::from_days(3),
            Patch::FactSet(Box::new(Fact::new(
                "p_does_not_exist",
                true,
                WorldTime::from_days(3),
                Lock::L0,
                "evt_bad",
            ))),
        ),
    ]);

    assert!(result.is_err());
    assert_eq!(
        store.event_count().unwrap(),
        before,
        "批次里任何一条失败，整批都不能落盘"
    );
}

#[test]
fn projection_keeps_fact_belief_and_claim_separate() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    let turn = TurnId::new("turn_0002");

    // 顾言声称林夏要走了（claim），这不会改动事实层
    let claimed = if_domain::state::Claim {
        id: if_domain::id::ClaimId::new("claim_1"),
        speaker: SubjectId::new("c_gu"),
        audience: vec![SubjectId::new("c_lin")],
        prop: PropositionId::new("p_leave"),
        claimed_value: Value::Bool(true),
        sincerity: if_domain::state::Sincerity::Unknown,
        world_time: WorldTime::from_days(1),
        source: if_domain::id::EventId::new("evt_claim"),
    };
    store
        .append(EventDraft::new(
            line.clone(),
            turn.clone(),
            WorldTime::from_days(1),
            Patch::ClaimMade(Box::new(claimed)),
        ))
        .unwrap();

    let projection = store.load_projection(&line).unwrap();
    assert_eq!(projection.claims.len(), 1);
    // 声称没有写进事实层
    assert!(projection.fact(&PropositionId::new("p_leave")).is_none());

    // 顾言的信念里也没有这件事——信念只能由 belief_set 写入
    assert!(projection
        .belief(&SubjectId::new("c_gu"), &PropositionId::new("p_leave"))
        .is_none());

    // 玩家自己的信念可以单独建立
    store
        .append(EventDraft::new(
            line.clone(),
            turn,
            WorldTime::from_days(1),
            Patch::BeliefSet(Box::new(Belief::witnessed(
                SubjectId::user(),
                "p_leave",
                false,
                WorldTime::from_days(1),
                "evt_user",
            ))),
        ))
        .unwrap();
    let projection = store.load_projection(&line).unwrap();
    assert!(projection
        .user_belief(&PropositionId::new("p_leave"))
        .is_some());
}

#[test]
fn beats_keep_their_display_order_and_world_time_is_monotonic() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);
    run_scene(&mut store, &line, &TurnId::new("turn_0002"), 1);
    run_scene(&mut store, &line, &TurnId::new("turn_0002"), 2);

    let projection = store.load_projection(&line).unwrap();
    assert_eq!(projection.beats.len(), 4);
    let orders: Vec<u64> = store
        .events_for_line(&line)
        .unwrap()
        .iter()
        .filter(|e| e.event_type() == EventType::BeatDisplayed)
        .filter_map(|e| e.narrative_order)
        .collect();
    let mut sorted = orders.clone();
    sorted.sort_unstable();
    assert_eq!(orders, sorted);
    assert_eq!(projection.narrative_order, 4);

    // 世界时间只前进
    assert!(projection.world_time >= WorldTime::from_hours(1));
    assert!(projection.current_scene_index() == 2);
}

/// 世界播种要在**写入之前**算出这批事件的 ID（`Subject::created_by` / `LoreEntry::source`
/// 存的是引入它的事件 ID）。这条规则是那个前提，不能悄悄改。
#[test]
fn event_ids_follow_next_seq() {
    let mut store = Store::open_in_memory().unwrap();
    let line = seed_world(&mut store);

    let first = store.next_seq().unwrap();
    assert_eq!(first, store.event_count().unwrap() + 1);
    let last = first - 1;

    let turn = TurnId::new("turn_0002");
    let drafts: Vec<_> = (0..3)
        .map(|i| {
            EventDraft::new(
                line.clone(),
                turn.clone(),
                WorldTime::from_days(1),
                Patch::IfConflictResolved {
                    note: format!("第 {i} 条"),
                },
            )
        })
        .collect();
    let written = store.append_batch(drafts).unwrap();

    for (i, event) in written.iter().enumerate() {
        let seq = first + i as u64;
        assert_eq!(event.seq, seq);
        assert_eq!(event.id.as_str(), format!("evt_{seq:04}"));
    }
    // 下一批接着往后发号，不重号
    assert_eq!(store.next_seq().unwrap(), first + 3);

    // 回滚只挪头指针、不删事件，所以号也不会被回收——
    // 这正是 `next_seq` 用 `max_seq` 而不是 `head_seq` 的原因。
    let rolled_back = store.rollback_to(&line, last).unwrap();
    assert_eq!(rolled_back.head_seq, last);
    assert_eq!(store.next_seq().unwrap(), first + 3);
}

#[test]
fn world_file_survives_reopen() {
    let dir = std::env::temp_dir().join(format!("if_store_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rain.ifworld");
    let _ = std::fs::remove_file(&path);

    {
        let mut store = Store::open(&path).unwrap();
        let line = seed_world(&mut store);
        run_scene(&mut store, &line, &TurnId::new("turn_0002"), 1);
        let projection = store.load_projection(&line).unwrap();
        assert_eq!(projection.beats.len(), 2);
    }

    // 重新打开同一个文件，投影应当一致
    let store = Store::open(&path).unwrap();
    let projection = store.load_projection(&main_line()).unwrap();
    assert_eq!(projection.beats.len(), 2);
    assert_eq!(store.world_seed().unwrap(), Some(77492));
    assert!(projection.subject(&SubjectId::new("c_lin")).is_some());

    let _ = std::fs::remove_file(&path);
}
