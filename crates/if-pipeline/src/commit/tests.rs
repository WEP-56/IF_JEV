use super::*;
use crate::testsupport as f;
use if_domain::id::BeatId;
use if_domain::narrative::{Beat, Timestamp};
use if_domain::subject::{PropositionKind, ValueType};

// ---------------------------------------------------------------- 对账

fn proposed(prop: &str, value: Value, internal: bool) -> ProposedChange {
    ProposedChange {
        candidate: CandidateId::new("cand_1"),
        prop: PropositionId::new(prop),
        value,
        internal,
        lock: Lock::L2,
        reason: "候选内容".into(),
    }
}

fn observed(prop: &str, value: Value, internal: bool) -> ObservedChange {
    ObservedChange {
        prop: PropositionId::new(prop),
        value,
        internal,
        text: "正文里的一句话".into(),
    }
}

/// 规则 1：已经展示给玩家的东西撤不回来。
#[test]
fn everything_observed_is_committed() {
    let result = reconcile(
        &[],
        &[
            observed("p_a", Value::Bool(true), false),
            observed("p_b", Value::Text("走了".into()), true),
        ],
    );
    assert_eq!(result.committed.len(), 2);
    assert!(result.returned.is_empty());
    assert!(result.voided.is_empty());
    assert!(result.committed.iter().all(|change| change.source == ChangeSource::Observed));
    // 正文里的变化由世界推演产生，只有用户注入 IF 才可能升到 L2/L3
    assert!(result.committed.iter().all(|change| change.lock == Lock::L0));
}

/// 规则 2：内在状态（感情、意图、信念）可以在正文之外提交。
#[test]
fn internal_state_commits_without_appearing_in_the_text() {
    let result = reconcile(
        &[proposed("p_mood", Value::Bool(true), true)],
        &[],
    );
    assert_eq!(result.committed.len(), 1);
    assert_eq!(result.committed[0].source, ChangeSource::Internal);
    assert_eq!(result.committed[0].lock, Lock::L2, "内在变化保留预演给的锁定");
    assert!(result.returned.is_empty(), "内在状态不进候选池");
}

/// 规则 3：外在事件必须在正文里出现过。
///
/// 没有这一条，「骰子说发生了」就会直接变成世界事实，而玩家从没在正文里见过它。
#[test]
fn an_external_change_that_never_appeared_is_returned_to_the_pool() {
    let result = reconcile(&[proposed("p_gate", Value::Bool(true), false)], &[]);
    assert!(result.committed.is_empty());
    assert_eq!(result.returned.len(), 1);
    assert_eq!(result.returned[0].prop.as_str(), "p_gate");
}

#[test]
fn an_external_change_that_did_appear_is_committed_as_displayed() {
    let result = reconcile(
        &[proposed("p_gate", Value::Bool(true), false)],
        &[observed("p_gate", Value::Bool(true), false)],
    );
    assert_eq!(result.committed.len(), 1);
    assert_eq!(result.committed[0].source, ChangeSource::Displayed);
    assert!(!result.committed[0].internal);
    // 正文里出现过一次，就该只提交一次
    assert!(result.returned.is_empty());
}

/// 规则 4：正文与预演矛盾时**以正文为准**，该预演作废。
#[test]
fn the_text_wins_when_it_contradicts_the_proposal() {    let result = reconcile(
        &[proposed("p_gate", Value::Bool(true), false)],
        &[observed("p_gate", Value::Bool(false), false)],
    );
    assert_eq!(result.voided.len(), 1);
    assert!(result.returned.is_empty(), "作废不等于退回候选池");
    assert!(result.warnings.iter().any(|w| w.contains("以正文为准")));
    // 提交的是正文里的那个值
    assert_eq!(result.committed.len(), 1);
    assert_eq!(result.committed[0].value, Value::Bool(false));
}

/// 内在状态与正文矛盾时同样以正文为准——「他心里其实不想走」被正文写成了想走。
#[test]
fn the_text_also_wins_over_an_internal_conflict() {
    let result = reconcile(
        &[proposed("p_mood", Value::Bool(true), true)],
        &[observed("p_mood", Value::Bool(false), false)],
    );
    assert_eq!(result.voided.len(), 1);
    assert_eq!(result.committed.len(), 1);
    assert_eq!(result.committed[0].source, ChangeSource::Observed);
}

/// 同一条变化既在正文里出现、又带着更高的锁定（用户 IF 直接引起的）时，
/// 提交的必须是**锁定更强**的那一条。
///
/// 按插入顺序去重会留下先插进去的 Observed（`Lock::L0`），把 IF 的 L2 静默降成自由状态——
/// 世界从此可以被自然推演改掉用户亲手锚定的事。
#[test]
fn the_strongest_lock_survives_when_the_text_confirms_a_change() {
    let mut proposal = proposed("p_gate", Value::Bool(true), false);
    proposal.lock = Lock::L2;
    let result = reconcile(
        &[proposal],
        &[observed("p_gate", Value::Bool(true), false)],
    );

    assert_eq!(result.committed.len(), 1, "同一命题只提交一次");
    assert_eq!(result.committed[0].lock, Lock::L2);
    assert_eq!(result.committed[0].source, ChangeSource::Displayed);
}

#[test]
fn the_result_is_sorted_by_proposition() {
    let result = reconcile(
        &[
            proposed("p_c", Value::Bool(true), true),
            proposed("p_a", Value::Bool(true), true),
            proposed("p_b", Value::Bool(true), false),
        ],
        &[],
    );
    let committed: Vec<&str> = result
        .committed
        .iter()
        .map(|change| change.prop.as_str())
        .collect();
    assert_eq!(committed, vec!["p_a", "p_c"]);
    assert_eq!(result.returned.len(), 1);
    assert_eq!(result.returned[0].prop.as_str(), "p_b");
}

// ---------------------------------------------------------------- 预演推导

fn candidate(affects: &[&str], internal: bool) -> Candidate {
    Candidate {
        id: CandidateId::new("cand_1"),
        key: Some("cand_1.decision".into()),
        subject: None,
        content: "门锁上了".into(),
        internal,
        shape: if_domain::turn::CandidateShape::Occurs,
        depends_on: Vec::new(),
        based_on: Vec::new(),
        affects: affects
            .iter()
            .map(|id| PropositionId::new(*id))
            .collect(),
    }
}

#[test]
fn a_bool_proposition_is_set_to_true_and_nothing_else_is_guessed() {
    let projection = f::projection();
    let (changes, warnings) = proposed_from(&candidate(&[f::P_EVIDENCE], false), &projection, None);

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].value, Value::Bool(true));
    assert_eq!(changes[0].prop.as_str(), f::P_EVIDENCE);
    assert!(!changes[0].internal);
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// 标量（如心情 0–1）推不出唯一取值，就**不猜**——写进 warnings 让调用方补。
#[test]
fn a_scalar_proposition_is_left_to_the_caller() {
    let projection = f::projection();
    let (changes, warnings) = proposed_from(&candidate(&[f::P_MOOD], true), &projection, None);
    assert!(changes.is_empty());
    assert!(warnings.iter().any(|w| w.contains("无法由候选") && w.contains("唯一确定")));
}

#[test]
fn an_unknown_proposition_is_reported_and_skipped() {
    let projection = f::projection();
    let (changes, warnings) = proposed_from(&candidate(&["p_missing"], false), &projection, None);
    assert!(changes.is_empty());
    assert!(warnings.iter().any(|w| w.contains("不存在的命题")));
}

#[test]
fn an_enum_takes_the_selected_option_when_it_is_in_range() {
    let mut projection = f::projection();
    projection.propositions.insert(
        PropositionId::new("p_route"),
        Proposition {
            id: PropositionId::new("p_route"),
            key: "world.route".into(),
            text: "他走哪条路".into(),
            subjects: Vec::new(),
            kind: PropositionKind::State,
            value_type: ValueType::Enum {
                values: vec!["东门".into(), "西门".into()],
            },
            internal: false,
        },
    );

    let (changes, warnings) =
        proposed_from(&candidate(&["p_route"], false), &projection, Some("西门"));
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].value, Value::Text("西门".into()));
    assert!(warnings.is_empty());

    // 抽中的选项不在取值域里 → 不提交，报出来
    let (changes, warnings) =
        proposed_from(&candidate(&["p_route"], false), &projection, Some("北门"));
    assert!(changes.is_empty());
    assert!(warnings.iter().any(|w| w.contains("不在命题")));

    // 枚举没有抽中选项时也不能瞎填
    let (changes, warnings) = proposed_from(&candidate(&["p_route"], false), &projection, None);
    assert!(changes.is_empty());
    assert!(warnings.iter().any(|w| w.contains("无法由候选") && w.contains("唯一确定")));
}

// ---------------------------------------------------------------- 事件草稿

const SCENE: &str = "scene_0001";

fn beat(index: u32) -> Beat {
    Beat {
        id: BeatId::new(format!("{SCENE}_b{index:02}")),
        scene: SceneId::new(SCENE),
        index,
        text: format!("第 {index} 拍。"),
        plan_beat: None,
        judgments: vec![],
        displayed_at: None,
    }
}

fn commit() -> TurnCommit {
    TurnCommit {
        propositions: vec![Proposition {
            id: PropositionId::new("p_new"),
            key: "world.new".into(),
            text: "新命题".into(),
            subjects: Vec::new(),
            kind: PropositionKind::State,
            value_type: ValueType::Bool,
            internal: false,
        }],
        changes: vec![
            CommittedChange {
                prop: PropositionId::new(f::P_RAIN),
                value: Value::Bool(true),
                lock: Lock::L0,
                internal: false,
                source: ChangeSource::Displayed,
            },
            CommittedChange {
                prop: PropositionId::new(f::P_MOOD),
                value: Value::Bool(true),
                lock: Lock::L0,
                internal: true,
                source: ChangeSource::Internal,
            },
        ],
        tendencies: vec![Tendency::latent(
            TendencyId::new("tnd_1"),
            "顾言开始怀疑",
            0.6,
        )],
        tendency_updates: vec![(TendencyId::new("tnd_old"), 0.8, TendencyStatus::Latent)],
        threads: vec![ThreadUpdate {
            thread: ThreadId::new(f::T_SHIELD),
            stage: ThreadStage::Escalating,
            pressure: 0.7,
        }],
        scene: Some(SceneCommit {
            id: SceneId::new(SCENE),
            index: 1,
            plan: f::plan("让顾言开口"),
            started_at: WorldTime::from_days(7),
            completed_at: Some(WorldTime::from_days(8)),
            beats: vec![beat(1), beat(2)],
        }),
        time: Some(WorldTime::from_days(8)),
        displayed_at: Some(1_800_000_000_000),
    }
}

fn label(patch: &Patch) -> &'static str {
    match patch {
        Patch::SceneStarted(_) => "scene_started",
        Patch::BeatDisplayed(_) => "beat_displayed",
        Patch::PropositionCreated(_) => "proposition_created",
        Patch::FactSet(_) => "fact_set",
        Patch::TendencyCreated(_) => "tendency_created",
        Patch::TendencyUpdated { .. } => "tendency_updated",
        Patch::ThreadStageChanged { .. } => "thread_stage_changed",
        Patch::TimeAdvanced { .. } => "time_advanced",
        Patch::SceneCompleted { .. } => "scene_completed",
        other => panic!("不认识的事件：{other:?}"),
    }
}

fn write(commit: &TurnCommit, first_seq: u64) -> (Vec<EventDraft>, Vec<EventId>) {
    let mut cursor = DraftCursor::new("wl_main", TurnId::numbered(1), first_seq, 3);
    drafts(commit, &mut cursor)
}

/// 顺序是契约：命题必须排在任何引用它的事实之前，收束排在最末。
#[test]
fn drafts_are_written_in_a_fixed_order() {
    let (drafts, _) = write(&commit(), 1);
    let order: Vec<&str> = drafts.iter().map(|draft| label(&draft.payload)).collect();
    assert_eq!(
        order,
        vec![
            "scene_started",
            "beat_displayed",
            "beat_displayed",
            "proposition_created",
            "fact_set",
            "fact_set",
            "tendency_created",
            "tendency_updated",
            "thread_stage_changed",
            "time_advanced",
            "scene_completed",
        ]
    );
}

/// 与 `Store::append_batch` 是同一条契约：第 `i` 条拿到 `first_seq + i`。
#[test]
fn drafts_are_numbered_from_first_seq() {
    let (drafts, committed) = write(&commit(), 7);
    assert_eq!(committed.len(), drafts.len());
    for (index, id) in committed.iter().enumerate() {
        assert_eq!(id.as_str(), EventId::numbered(7 + index as u64).as_str());
    }
    assert_eq!(committed[0].as_str(), "evt_0007");
}

#[test]
fn beat_displayed_consumes_a_narrative_order() {
    let (drafts, _) = write(&commit(), 1);
    let displayed: Vec<&EventDraft> = drafts
        .iter()
        .filter(|draft| matches!(draft.payload, Patch::BeatDisplayed(_)))
        .collect();
    assert_eq!(displayed.len(), 2);
    // 游标从 3 起
    assert_eq!(displayed[0].narrative_order, Some(3));
    assert_eq!(displayed[1].narrative_order, Some(4));
    // 其余事件不占叙述顺序
    assert!(drafts
        .iter()
        .filter(|draft| !matches!(draft.payload, Patch::BeatDisplayed(_)))
        .all(|draft| draft.narrative_order.is_none()));
}

/// `events.scene` / `events.beat` 是**会落库并读回**的字段，漏填就是丢了可查询的维度。
#[test]
fn drafts_carry_their_scene_and_beat() {
    let (drafts, _) = write(&commit(), 1);
    for draft in &drafts {
        assert_eq!(
            draft.scene.as_ref().map(|scene| scene.as_str()),
            Some(SCENE),
            "{:?} 没带上场景",
            label(&draft.payload)
        );
    }
    let beats: Vec<Option<&str>> = drafts
        .iter()
        .filter(|draft| matches!(draft.payload, Patch::BeatDisplayed(_)))
        .map(|draft| draft.beat.as_ref().map(|beat| beat.as_str()))
        .collect();
    assert_eq!(beats, vec![Some("scene_0001_b01"), Some("scene_0001_b02")]);
    assert!(drafts
        .iter()
        .filter(|draft| !matches!(draft.payload, Patch::BeatDisplayed(_)))
        .all(|draft| draft.beat.is_none()));
}

#[test]
fn the_scene_started_payload_matches_the_plan_that_was_used() {
    let (drafts, _) = write(&commit(), 1);
    match &drafts[0].payload {
        Patch::SceneStarted(scene) => {
            assert_eq!(scene.id.as_str(), SCENE);
            assert_eq!(scene.index, 1);
            assert_eq!(scene.started_at, WorldTime::from_days(7));
            assert_eq!(scene.completed_at, None, "开场事件里场景还没结束");
            assert_eq!(scene.plan.goal, "让顾言开口");
        }
        other => panic!("第一条应当是 scene_started，实际 {other:?}"),
    }
}

#[test]
fn a_displayed_beat_carries_its_timestamp() {
    let (drafts, _) = write(&commit(), 1);
    let beat = drafts
        .iter()
        .find_map(|draft| match &draft.payload {
            Patch::BeatDisplayed(beat) => Some(beat.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(beat.displayed_at, Some(1_800_000_000_000 as Timestamp));
}

/// `Tendency::contributors` 存的是**引入它的事件 ID**，所以只有写入那一刻才定得下来。
#[test]
fn a_new_tendency_takes_its_own_event_id_as_a_contributor() {
    let (drafts, committed) = write(&commit(), 1);
    let (index, tendency) = drafts
        .iter()
        .enumerate()
        .find_map(|(index, draft)| match &draft.payload {
            Patch::TendencyCreated(tendency) => Some((index, tendency.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(tendency.contributors.len(), 1);
    assert_eq!(tendency.contributors[0].as_str(), committed[index].as_str());

    // 已经挂过贡献事件的趋势不会被改写
    let mut existing = commit();
    existing.tendencies[0].contributors.push(EventId::new("evt_9000"));
    let (drafts, _) = write(&existing, 1);
    let tendency = drafts
        .iter()
        .find_map(|draft| match &draft.payload {
            Patch::TendencyCreated(tendency) => Some(tendency.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(tendency.contributors.len(), 1);
    assert_eq!(tendency.contributors[0].as_str(), "evt_9000");
}

#[test]
fn time_advanced_is_written_at_the_new_time() {
    let (drafts, _) = write(&commit(), 1);
    let advanced = drafts
        .iter()
        .find(|draft| matches!(draft.payload, Patch::TimeAdvanced { .. }))
        .unwrap();
    match &advanced.payload {
        Patch::TimeAdvanced { to } => assert_eq!(*to, WorldTime::from_days(8)),
        other => panic!("实际 {other:?}"),
    }
    assert_eq!(advanced.world_time, WorldTime::from_days(8));
}

#[test]
fn scene_completed_is_only_written_when_the_scene_closes() {
    let mut open_scene = commit();
    open_scene.scene.as_mut().unwrap().completed_at = None;
    let (drafts, _) = write(&open_scene, 1);
    assert!(!drafts
        .iter()
        .any(|draft| matches!(draft.payload, Patch::SceneCompleted { .. })));
    // 仍然有开场与被展示的节拍
    assert_eq!(drafts.len(), 10);
}

#[test]
fn the_whole_commit_can_be_written_without_a_scene() {
    // 后台结算 / 纯时间推进的回合没有场景，但也不是错误
    let mut plain = commit();
    plain.scene = None;
    let (drafts, committed) = write(&plain, 1);
    assert!(drafts
        .iter()
        .all(|draft| !matches!(draft.payload, Patch::BeatDisplayed(_))));
    assert_eq!(committed.len(), drafts.len());
    assert!(drafts.iter().all(|draft| draft.scene.is_none()));
}

#[test]
fn writing_twice_from_the_same_commit_yields_the_same_drafts() {
    let (once, _) = write(&commit(), 1);
    let (twice, _) = write(&commit(), 1);
    assert_eq!(once, twice);
}

/// 游标可以接着推：回合本体写完之后还有后台结算、预生成。
#[test]
fn the_cursor_keeps_numbering_after_a_drain() {
    let mut cursor = DraftCursor::new("wl_main", TurnId::numbered(1), 1, 3);
    let (first, _) = drafts(&commit(), &mut cursor);
    assert_eq!(cursor.peek().as_str(), "evt_0012");
    assert_eq!(cursor.narrative_order(), 5, "两拍各占掉一个叙述序号");

    let id = cursor.push(WorldTime::from_days(9), |_| Patch::BackgroundSettled {
        subject: if_domain::id::SubjectId::new(f::LIN),
        text: "他睡了很久".into(),
        from: WorldTime::from_days(8),
        to: WorldTime::from_days(9),
    });
    assert_eq!(id.as_str(), "evt_0012");
    assert_eq!(first.len(), 11);
    assert_eq!(cursor.finish().len(), 1, "当场推的那条要被取走");
}
