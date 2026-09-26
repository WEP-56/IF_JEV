//! 第 12–13 步：预演 / 观察 / 提交的对账，以及回合的产物（docs/02 §11、docs/03 §2）。
//!
//! 这一段是整个流程里**唯一**会产出可写事件的地方。三条对账规则全部来自 docs/02 §11，
//! 这里一行不改：
//!
//! 1. Observed 全部提交——它们已经展示给玩家了，撤不回来。
//! 2. Proposed 的**内在状态**（感情、意图、信念、倾向），只要没有被正文否定，
//!    可以在正文之外提交。
//! 3. Proposed 的**外在事件**（行动、可见的变化），必须在正文中出现过才提交；
//!    没出现的退回候选池，或转为趋势。
//! 4. 正文与 Proposed 矛盾时，**以正文为准**，该 Proposed 作废。
//!
//! 第 3 条是这套设计里最容易被忽略的一条：没有它，「骰子说发生了」就会直接变成世界事实，
//! 而玩家从没在正文里见过它——世界开始自己编故事。
//!
//! 第 4 条之后再补一条**去重**：同一条变化常常既是 Observed（从正文抽出来的）又是
//! 「正文里确实发生了的预演」，但只能提交一次，且留下的是**锁定最强**的那一条——
//! 按插入顺序去重会把 IF 带来的 L2 静默降成 L0。
//!
//! ## 事件 ID 的分配契约（改动前必读）
//!
//! 有些补丁的载荷里存着**引入它的事件 ID**（`Fact::source`、`Tendency::contributors`），
//! 而这个 ID 只有写入那一刻才定得下来。所以这里沿用播种（`if-app::seed`）的做法：
//! [`DraftCursor`] 从调用方给的 `first_seq` 起**顺序发号**，
//! 与 `if_store::Store::append_batch` 的发号规则一一对应（第 `i` 条拿到 `first_seq + i`）。
//!
//! **调用方必须把 `finish()` 的草稿按原顺序交给 `append_batch`**，中间不能插队——
//! 这是 `if-store::tests::event_ids_follow_next_seq` 与
//! `if-app::seed::tests::plan_matches_store_allocation` 两侧钉住的同一条契约。

use std::collections::BTreeMap;

use if_domain::event::{EventDraft, Patch};
use if_domain::id::{
    CandidateId, EventId, PropositionId, SceneId, TendencyId, ThreadId, TurnId, WorldLineId,
};
use if_domain::narrative::{Beat, ScenePlan, Tendency, TendencyStatus, ThreadStage};
use if_domain::subject::Proposition;
use if_domain::turn::Candidate;
use if_domain::value::{Lock, Value, WorldTime};

use if_domain::projection::Projection;

/// 一条**预演**变化：掷骰通过、用来指导写作。
#[derive(Clone, Debug, PartialEq)]
pub struct ProposedChange {
    /// 它是哪个候选的产物。
    pub candidate: CandidateId,
    pub prop: PropositionId,
    pub value: Value,
    /// 内在状态（感情、意图、信念、倾向）。它决定第 2 条与第 3 条哪条适用。
    pub internal: bool,
    /// 提交时用的锁定等级。世界自然推演是 L0；IF 直接带来的改变由调用方另行指定。
    pub lock: Lock,
    /// 人读的来源说明（候选内容），用于审计与界面展示。
    pub reason: String,
}

/// 一条**观察**变化：从已展示正文里抽取、经 Jev 校对的内容。
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedChange {
    pub prop: PropositionId,
    pub value: Value,
    pub internal: bool,
    /// 正文里的依据，供审计。
    pub text: String,
}

/// 提交的理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeSource {
    /// 已经在正文里出现过（Observed）。
    Observed,
    /// 内在状态，正文没否定它。
    Internal,
    /// 外在事件，正文里确实发生了。
    Displayed,
}

/// 一条要写进事件日志的变化。
#[derive(Clone, Debug, PartialEq)]
pub struct CommittedChange {
    pub prop: PropositionId,
    pub value: Value,
    pub lock: Lock,
    pub internal: bool,
    pub source: ChangeSource,
}

/// 对账结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reconciliation {
    /// 提交的变化，按命题 ID 排序。
    pub committed: Vec<CommittedChange>,
    /// 退回候选池的变化（外在、且正文里没出现）。
    pub returned: Vec<ProposedChange>,
    /// 被正文否定的变化（同命题、不同值）。
    pub voided: Vec<ProposedChange>,
    pub warnings: Vec<String>,
}

/// 按 docs/02 §11 的四条规则对账。
pub fn reconcile(proposed: &[ProposedChange], observed: &[ObservedChange]) -> Reconciliation {
    let mut result = Reconciliation::default();
    // 命题 → 正文里出现的值。
    let mut from_text: BTreeMap<&PropositionId, &Value> = BTreeMap::new();
    for change in observed {
        from_text.insert(&change.prop, &change.value);
    }

    for change in observed {
        result.committed.push(CommittedChange {
            prop: change.prop.clone(),
            value: change.value.clone(),
            // 正文里出现过的变化由世界推演产生，因此是 L0；
            // 要升到 L2/L3 只有一条路——用户显式注入 IF（docs/01 §6）。
            lock: Lock::L0,
            internal: change.internal,
            source: ChangeSource::Observed,
        });
    }

    for change in proposed {
        match from_text.get(&change.prop) {
            // 规则 4：正文与预演矛盾，以正文为准。值相同则下面按规则 2 / 3 走。
            Some(value) if *value != &change.value => {
                result.voided.push(change.clone());
                result.warnings.push(format!(
                    "正文里的 {} 与预演不一致，以正文为准（预演作废）",
                    change.prop
                ));
            }
            _ => {
                if change.internal {
                    // 规则 2：内在状态可以在正文之外提交。
                    result.committed.push(CommittedChange {
                        prop: change.prop.clone(),
                        value: change.value.clone(),
                        lock: change.lock,
                        internal: true,
                        source: ChangeSource::Internal,
                    });
                } else if from_text.contains_key(&change.prop) {
                    // 规则 3 的另一半：外在事件，且正文里确实出现了。
                    result.committed.push(CommittedChange {
                        prop: change.prop.clone(),
                        value: change.value.clone(),
                        lock: change.lock,
                        internal: false,
                        source: ChangeSource::Displayed,
                    });
                } else {
                    // 规则 3：外在事件没在正文里出现，退回候选池或转为趋势。
                    result.returned.push(change.clone());
                }
            }
        }
    }

    // 同一命题只提交一次。留下的必须是**锁定最强**的那一条——同一条变化常常既是
    // Observed（从正文抽出来的）又是「正文里确实发生了的预演」，而按插入顺序去重
    // 会留下先插进去的 Observed（`Lock::L0`），把 IF 带来的 L2 静默降成自由状态。
    // 锁定相同时以「正文里确实发生了」为准：那是更具体的那句话。
    result.committed.sort_by(|a, b| {
        a.prop
            .cmp(&b.prop)
            .then_with(|| b.lock.rank().cmp(&a.lock.rank()))
            .then_with(|| b.source.cmp(&a.source))
    });
    result.committed.dedup_by(|a, b| a.prop == b.prop);
    result.returned.sort_by(|a, b| a.prop.cmp(&b.prop));
    result.voided.sort_by(|a, b| a.prop.cmp(&b.prop));
    result
}

/// 把一个放行的候选翻成预演变化。
///
/// 值的推导是**确定性的且保守的**：只有命题的 `value_type` 能唯一确定取值时才产出，
/// 其余写进 `warnings` 让调用方去补。不猜——猜错就是静默改写世界前提。
pub fn proposed_from(
    candidate: &Candidate,
    projection: &Projection,
    selected_option: Option<&str>,
) -> (Vec<ProposedChange>, Vec<String>) {
    let mut changes = Vec::new();
    let mut warnings = Vec::new();

    for prop in &candidate.affects {
        let Some(proposition) = projection.propositions.get(prop) else {
            warnings.push(format!(
                "候选 {} 影响了不存在的命题 {}，跳过",
                candidate.id, prop
            ));
            continue;
        };
        let value = match (&proposition.value_type, selected_option) {
            (if_domain::subject::ValueType::Bool, _) => Some(Value::Bool(true)),
            (if_domain::subject::ValueType::Enum { values }, Some(option)) => {
                if values.iter().any(|value| value == option) {
                    Some(Value::Text(option.to_owned()))
                } else {
                    warnings.push(format!(
                        "候选 {} 抽中的选项「{option}」不在命题 {} 的取值域里，跳过",
                        candidate.id, prop
                    ));
                    None
                }
            }
            _ => {
                warnings.push(format!(
                    "命题 {} 的取值无法由候选 {} 唯一确定，本回合不提交它",
                    prop, candidate.id
                ));
                None
            }
        };
        if let Some(value) = value {
            changes.push(ProposedChange {
                candidate: candidate.id.clone(),
                prop: prop.clone(),
                value,
                internal: candidate.internal,
                lock: Lock::L0,
                reason: candidate.content.clone(),
            });
        }
    }

    (changes, warnings)
}

// ---------------------------------------------------------------- 提交

/// 场景部分的提交内容。
#[derive(Clone, Debug, PartialEq)]
pub struct SceneCommit {
    pub id: SceneId,
    /// 场景序号。L1 保护期与出场平衡都按它算，所以由调用方给，不由这里猜。
    pub index: u64,
    pub plan: ScenePlan,
    pub started_at: WorldTime,
    pub completed_at: Option<WorldTime>,
    /// 已放行的节拍，按放行顺序。
    pub beats: Vec<Beat>,
}

/// 故事线阶段变化。
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadUpdate {
    pub thread: ThreadId,
    pub stage: ThreadStage,
    pub pressure: f64,
}

/// 一个回合要写进事件日志的全部内容。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnCommit {
    /// 需要新建的命题。**必须排在任何引用它们的 `FactSet` 之前**。
    pub propositions: Vec<Proposition>,
    pub changes: Vec<CommittedChange>,
    pub tendencies: Vec<Tendency>,
    /// 已有趋势的推动 / 爆发。
    pub tendency_updates: Vec<(TendencyId, f64, TendencyStatus)>,
    pub threads: Vec<ThreadUpdate>,
    pub scene: Option<SceneCommit>,
    /// 世界时间推进到哪一刻。
    pub time: Option<WorldTime>,
    /// 节拍上屏的现实时刻。审计用，**不参与投影**（docs/02 §10）——
    /// 所以这里由调用方给，读时钟会让同一份产物在两次运行里不一样。
    pub displayed_at: Option<if_domain::narrative::Timestamp>,
}

/// 顺序发号的草稿游标。
///
/// 与 `if-app::seed` 里的 `Writer` 是同一条契约的两种用法：**先算号、再写**，
/// 因为有些补丁的载荷里存着引入它的事件 ID。
///
/// 它还负责给草稿盖上「属于哪个场景 / 哪个节拍」——这两个字段 `Store` 是**会落库
/// 并读回**的（`events.scene` / `events.beat`），漏填不是少个注释，是少了可查询的维度。
#[derive(Clone, Debug)]
pub struct DraftCursor {
    line: WorldLineId,
    turn: TurnId,
    seq: u64,
    narrative_order: u64,
    scene: Option<SceneId>,
    beat: Option<if_domain::id::BeatId>,
    drafts: Vec<EventDraft>,
}

impl DraftCursor {
    /// `first_seq` 取 `Store::next_seq()`——第 `i` 条草稿会拿到 `first_seq + i`。
    pub fn new(
        line: impl Into<WorldLineId>,
        turn: impl Into<TurnId>,
        first_seq: u64,
        narrative_order: u64,
    ) -> Self {
        Self {
            line: line.into(),
            turn: turn.into(),
            seq: first_seq,
            narrative_order,
            scene: None,
            beat: None,
            drafts: Vec::new(),
        }
    }

    /// 之后推的草稿都记在这个场景下。`None` = 离开场景。
    pub fn enter_scene(&mut self, scene: Option<SceneId>) {
        self.scene = scene;
    }

    /// 之后推的草稿都记在这个节拍下。`None` = 不在任何节拍里。
    pub fn enter_beat(&mut self, beat: Option<if_domain::id::BeatId>) {
        self.beat = beat;
    }

    /// 下一条草稿将拿到的 ID。需要「先算号再写」时用它。
    pub fn peek(&self) -> EventId {
        EventId::numbered(self.seq)
    }

    /// 推一条草稿，把 ID 交给构造闭包（补丁里要存它时用）。
    pub fn push(&mut self, at: WorldTime, build: impl FnOnce(&EventId) -> Patch) -> EventId {
        let id = self.peek();
        self.seq += 1;
        let patch = build(&id);
        let draft = self.tag(EventDraft::new(
            self.line.clone(),
            self.turn.clone(),
            at,
            patch,
        ));
        self.drafts.push(draft);
        id
    }

    /// 推一条**上屏**的草稿：消费一个叙述顺序（docs/03 §4）。
    pub fn push_displayed(
        &mut self,
        at: WorldTime,
        build: impl FnOnce(&EventId, u64) -> Patch,
    ) -> EventId {
        let id = self.peek();
        self.seq += 1;
        let narrative_order = self.narrative_order;
        self.narrative_order += 1;
        let patch = build(&id, narrative_order);
        let draft = self.tag(
            EventDraft::new(self.line.clone(), self.turn.clone(), at, patch)
                .displayed(narrative_order),
        );
        self.drafts.push(draft);
        id
    }

    /// 当前叙述顺序游标（下一个上屏事件会拿到它）。
    pub fn narrative_order(&self) -> u64 {
        self.narrative_order
    }

    pub fn finish(self) -> Vec<EventDraft> {
        self.drafts
    }

    /// 盖上场景 / 节拍。
    fn tag(&self, mut draft: EventDraft) -> EventDraft {
        if let Some(scene) = &self.scene {
            draft.scene = Some(scene.clone());
        }
        if let Some(beat) = &self.beat {
            draft.beat = Some(beat.clone());
        }
        draft
    }
}

/// 把一回合的决定写成一串草稿，返回 `(草稿, 本回合提交的事件 ID)`。
///
///
/// 顺序是固定的，且**命题先于引用它的事实**：
///
/// ```text
/// scene_started → beat_displayed* → proposition_created* → fact_set*
/// → tendency_created* → tendency_updated* → thread_stage_changed*
/// → time_advanced → scene_completed
/// ```
pub fn drafts(
    commit: &TurnCommit,
    cursor: &mut DraftCursor,
) -> (Vec<EventDraft>, Vec<EventId>) {
    let at = commit.time.unwrap_or_else(|| {
        commit
            .scene
            .as_ref()
            .map_or(WorldTime::EPOCH, |scene| scene.started_at)
    });
    let mut committed: Vec<EventId> = Vec::new();

    if let Some(scene) = &commit.scene {
        cursor.enter_scene(Some(scene.id.clone()));
        let id = cursor.push(at, |_| {
            Patch::SceneStarted(Box::new(if_domain::narrative::Scene {
                id: scene.id.clone(),
                index: scene.index,
                plan: scene.plan.clone(),
                started_at: scene.started_at,
                completed_at: None,
            }))
        });
        committed.push(id);
    }

    for beat in commit.scene.iter().flat_map(|scene| scene.beats.iter()) {
        let mut beat = beat.clone();
        beat.displayed_at = commit.displayed_at;
        cursor.enter_beat(Some(beat.id.clone()));
        let id = cursor.push_displayed(at, |_, _| Patch::BeatDisplayed(Box::new(beat.clone())));
        committed.push(id);
    }
    cursor.enter_beat(None);

    for proposition in &commit.propositions {
        let id = cursor.push(at, |_| Patch::PropositionCreated(Box::new(proposition.clone())));
        committed.push(id);
    }

    for change in &commit.changes {
        let prop = change.prop.clone();
        let value = change.value.clone();
        let lock = change.lock;
        let internal = change.internal;
        let id = cursor.push(at, move |event| {
            let mut fact = if_domain::state::Fact::new(prop, value, at, lock, event.clone());
            // 正文里出现过的变化是**可感知的**；内在变化只有相关主体知道。
            fact.visibility = if internal {
                if_domain::value::Visibility::Private
            } else {
                if_domain::value::Visibility::Public
            };
            Patch::FactSet(Box::new(fact))
        });
        committed.push(id);
    }

    for tendency in &commit.tendencies {
        let id = cursor.push(at, |event| {
            let mut tendency = tendency.clone();
            if tendency.contributors.is_empty() {
                tendency.contributors.push(event.clone());
            }
            Patch::TendencyCreated(Box::new(tendency))
        });
        committed.push(id);
    }

    for (tendency, pressure, status) in &commit.tendency_updates {
        let id = cursor.push(at, |_| Patch::TendencyUpdated {
            tendency: tendency.clone(),
            pressure: *pressure,
            status: *status,
        });
        committed.push(id);
    }

    for update in &commit.threads {
        let id = cursor.push(at, |_| Patch::ThreadStageChanged {
            thread: update.thread.clone(),
            stage: update.stage,
            pressure: update.pressure,
        });
        committed.push(id);
    }

    if let Some(to) = commit.time {
        let id = cursor.push(to, |_| Patch::TimeAdvanced { to });
        committed.push(id);
    }

    if let Some(scene) = &commit.scene {
        if let Some(completed_at) = scene.completed_at {
            let id = cursor.push(completed_at, |_| Patch::SceneCompleted {
                scene: scene.id.clone(),
                at: completed_at,
            });
            committed.push(id);
        }
    }

    (committed_drain(cursor), committed)
}

/// `drafts` 把草稿攒在游标里，这里取出来交给调用方。
///
/// 单独一段是因为 `finish` 会消费游标，而同一个回合里调用方可能还要接着推
/// （后台结算、预生成）。真要继续推就再调一次 `drafts`。
fn committed_drain(cursor: &mut DraftCursor) -> Vec<EventDraft> {
    std::mem::take(&mut cursor.drafts)
}

#[cfg(test)]
mod tests;
