//! 投影：把事件折叠成当前世界（docs/03 §1、§5）。
//!
//! 这里只有**纯计算**——不碰存储。`if-store` 负责把事件读出来喂给 [`Projection::apply`]，
//! 并在需要时存取快照。这样折叠逻辑不依赖 SQLite，单元测试不用起数据库。
//!
//! 确定性要求（docs/12 §7）：所有集合都用 `BTreeMap` / `Vec`，序列化键序固定；
//! 同一事件序列永远得到同一个投影。

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::event::{Event, IfInjection, Patch};
use crate::id::{
    EventId, LoreId, PropositionId, RuleId, SceneId, SubjectId, TendencyId, ThreadId, WorldLineId,
};
use crate::narrative::{Beat, LoreEntry, LoreStatus, Scene, Tendency, TendencyStatus, Thread};
use crate::rule::{WorldRule, WorldSettings};
use crate::state::{Belief, Claim, DependsOn, Fact, Observation};
use crate::subject::{Proposition, Subject};
use crate::value::{Lock, Value, Visibility, WorldTime};

/// 投影是从哪折叠出来的。存档与调试用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAnchor {
    pub line: WorldLineId,
    /// 已折叠到的 `seq`（含）。
    pub seq: u64,
}

impl ProjectionAnchor {
    pub fn genesis(line: impl Into<WorldLineId>) -> Self {
        Self {
            line: line.into(),
            seq: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    MissingProposition(PropositionId),
    MissingSubject(SubjectId),
    MissingThread(ThreadId),
    MissingTendency(TendencyId),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectionError::MissingProposition(p) => write!(f, "命题不存在: {p}"),
            ProjectionError::MissingSubject(s) => write!(f, "主体不存在: {s}"),
            ProjectionError::MissingThread(t) => write!(f, "故事线不存在: {t}"),
            ProjectionError::MissingTendency(t) => write!(f, "趋势不存在: {t}"),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// 当前世界。全部由事件折叠而来，引擎从不直接修改它（docs/02 开头）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    pub anchor: ProjectionAnchor,
    pub subjects: BTreeMap<SubjectId, Subject>,
    pub propositions: BTreeMap<PropositionId, Proposition>,
    /// 每个命题当前有效的事实。
    pub facts: BTreeMap<PropositionId, Fact>,
    /// 已经结束的事实，保留供回溯修复与审计。
    pub fact_history: Vec<Fact>,
    pub rules: BTreeMap<RuleId, WorldRule>,
    pub threads: BTreeMap<ThreadId, Thread>,
    pub tendencies: BTreeMap<TendencyId, Tendency>,
    pub lore: BTreeMap<LoreId, LoreEntry>,
    pub scenes: BTreeMap<SceneId, Scene>,
    /// 已展示的节拍，按叙述顺序。
    pub beats: Vec<Beat>,
    /// JSON 的对象键只能是字符串，所以序列化为按键排序的 `[key, belief]` 列表。
    #[serde(with = "belief_map")]
    pub beliefs: BTreeMap<BeliefKey, Belief>,
    pub claims: Vec<Claim>,
    pub observations: Vec<Observation>,
    pub injections: BTreeMap<EventId, IfInjection>,
    pub world_time: WorldTime,
    /// 叙述顺序的游标。
    pub narrative_order: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<WorldSettings>,
    /// 世界种子。命运骰子要它（docs/03 §8）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_seed: Option<u64>,
}

/// 信念的键：持有者 + 命题。玩家用 `SubjectId::user()`。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BeliefKey {
    pub holder: SubjectId,
    pub prop: PropositionId,
}

mod belief_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serializer};

    use super::BeliefKey;
    use crate::state::Belief;

    pub fn serialize<S: Serializer>(map: &BTreeMap<BeliefKey, Belief>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(map.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<BeliefKey, Belief>, D::Error> {
        Ok(Vec::<(BeliefKey, Belief)>::deserialize(d)?.into_iter().collect())
    }
}

impl BeliefKey {
    pub fn new(holder: impl Into<SubjectId>, prop: impl Into<PropositionId>) -> Self {
        Self {
            holder: holder.into(),
            prop: prop.into(),
        }
    }
}

impl Projection {
    /// 一条还没有任何事件的世界线。
    pub fn genesis(line: impl Into<WorldLineId>) -> Self {
        Self {
            anchor: ProjectionAnchor::genesis(line),
            subjects: BTreeMap::new(),
            propositions: BTreeMap::new(),
            facts: BTreeMap::new(),
            fact_history: Vec::new(),
            rules: BTreeMap::new(),
            threads: BTreeMap::new(),
            tendencies: BTreeMap::new(),
            lore: BTreeMap::new(),
            scenes: BTreeMap::new(),
            beats: Vec::new(),
            beliefs: BTreeMap::new(),
            claims: Vec::new(),
            observations: Vec::new(),
            injections: BTreeMap::new(),
            world_time: WorldTime::EPOCH,
            narrative_order: 0,
            settings: None,
            world_seed: None,
        }
    }

    /// 把一条事件折进投影。这是唯一的状态变更入口。
    pub fn apply(&mut self, event: &Event) -> Result<(), ProjectionError> {
        // 叙述顺序只由事件决定，补丁本身不带它。
        if let Some(order) = event.narrative_order {
            self.narrative_order = self.narrative_order.max(order);
        }
        // 世界时间单调前进；回溯修复写的是过去，不能把游标拽回去。
        if event.world_time > self.world_time {
            self.world_time = event.world_time;
        }
        self.anchor.seq = self.anchor.seq.max(event.seq);

        match &event.payload {
            Patch::WorldCreated { settings, .. } => {
                self.world_seed = Some(settings.seed);
                self.settings = Some(settings.clone());
            }
            Patch::SubjectCreated(subject) => {
                self.subjects.insert(subject.id.clone(), subject.as_ref().clone());
            }
            Patch::SubjectShaped { subject } => {
                let entry = self
                    .subjects
                    .get_mut(subject)
                    .ok_or_else(|| ProjectionError::MissingSubject(subject.clone()))?;
                entry.shaped = true;
            }
            Patch::PropositionCreated(prop) => {
                self.propositions
                    .insert(prop.id.clone(), prop.as_ref().clone());
            }
            Patch::RuleAdded(rule) => {
                self.rules.insert(rule.id.clone(), rule.as_ref().clone());
            }
            Patch::RuleChanged(rule) => {
                self.rules.insert(rule.id.clone(), rule.as_ref().clone());
            }
            Patch::LoreAdded(entry) => {
                self.lore.insert(entry.id.clone(), entry.as_ref().clone());
            }
            Patch::LoreSuperseded { lore, .. } => {
                if let Some(entry) = self.lore.get_mut(lore) {
                    entry.status = LoreStatus::Superseded;
                }
            }
            Patch::IfInjected(injection) => {
                self.injections.insert(event.id.clone(), injection.as_ref().clone());
            }
            Patch::IfConflictResolved { .. } => {}
            Patch::FactSet(fact) => {
                self.require_proposition(&fact.prop)?;
                // 同一命题同一时刻只有一个有效值：新的写入把旧的结束掉。
                if let Some(previous) = self.facts.get_mut(&fact.prop) {
                    if previous.valid_to.is_none() {
                        previous.valid_to = Some(fact.valid_from);
                    }
                    let ended = previous.clone();
                    self.fact_history.push(ended);
                }
                self.facts.insert(fact.prop.clone(), fact.as_ref().clone());
            }
            Patch::FactEnded { prop, at } => {
                if let Some(current) = self.facts.get_mut(prop) {
                    if current.valid_to.is_none() {
                        current.valid_to = Some(*at);
                        let ended = current.clone();
                        self.fact_history.push(ended);
                    }
                }
            }
            Patch::BeliefSet(belief) => {
                self.require_proposition(&belief.prop)?;
                if !belief.holder.is_user() {
                    self.require_subject(&belief.holder)?;
                }
                let key = BeliefKey::new(belief.holder.clone(), belief.prop.clone());
                self.beliefs.insert(key, belief.as_ref().clone());
            }
            Patch::ClaimMade(claim) => {
                self.require_proposition(&claim.prop)?;
                self.require_subject(&claim.speaker)?;
                self.claims.push(claim.as_ref().clone());
            }
            Patch::SceneStarted(scene) => {
                self.scenes.insert(scene.id.clone(), scene.as_ref().clone());
            }
            Patch::BeatDisplayed(beat) => {
                self.beats.push(beat.as_ref().clone());
            }
            Patch::SceneCompleted { scene, at } => {
                if let Some(entry) = self.scenes.get_mut(scene) {
                    entry.completed_at = Some(*at);
                }
            }
            Patch::TimeAdvanced { to } => {
                self.world_time = self.world_time.max(*to);
            }
            Patch::ThreadOpened(thread) => {
                self.threads.insert(thread.id.clone(), thread.as_ref().clone());
            }
            Patch::ThreadStageChanged {
                thread,
                stage,
                pressure,
            } => {
                let scene_index = self.current_scene_index();
                let entry = self
                    .threads
                    .get_mut(thread)
                    .ok_or_else(|| ProjectionError::MissingThread(thread.clone()))?;
                entry.stage = *stage;
                entry.pressure = *pressure;
                entry.last_advanced = scene_index;
            }
            Patch::TendencyCreated(tendency) => {
                self.tendencies
                    .insert(tendency.id.clone(), tendency.as_ref().clone());
            }
            Patch::TendencyUpdated {
                tendency,
                pressure,
                status,
            } => {
                let entry = self
                    .tendencies
                    .get_mut(tendency)
                    .ok_or_else(|| ProjectionError::MissingTendency(tendency.clone()))?;
                entry.pressure = *pressure;
                entry.status = *status;
            }
            Patch::BackgroundSettled { .. } => {
                // 后台事件不展示，但可以带起趋势——具体联动由引擎发起后续事件。
            }
            Patch::UserObserved(observation) => {
                self.observations.push(observation.as_ref().clone());
            }
            Patch::RetconApplied(retcon) => {
                // 影响分析写进记录，实际撤销由后续修复事件表达。
                let _ = &retcon.affected;
            }
            Patch::RetconRepaired { .. } => {}
            Patch::TierChanged { subject, tier } => {
                let entry = self
                    .subjects
                    .get_mut(subject)
                    .ok_or_else(|| ProjectionError::MissingSubject(subject.clone()))?;
                entry.tier = *tier;
            }
        }
        Ok(())
    }

    /// 按顺序折叠一批事件。
    pub fn fold_into<'a, I>(&mut self, events: I) -> Result<(), ProjectionError>
    where
        I: IntoIterator<Item = &'a Event>,
    {
        for event in events {
            self.apply(event)?;
        }
        Ok(())
    }

    /// 从空投影折叠出一份世界。
    pub fn fold<'a, I>(line: impl Into<WorldLineId>, events: I) -> Result<Self, ProjectionError>
    where
        I: IntoIterator<Item = &'a Event>,
    {
        let mut projection = Projection::genesis(line);
        projection.fold_into(events)?;
        Ok(projection)
    }

    // ------------------------------------------------------------ 查询辅助

    /// 已开始的场景数量。L1 保护期按场景序号计算（docs/01 §7）。
    pub fn current_scene_index(&self) -> u64 {
        self.scenes.len() as u64
    }

    pub fn current_scene(&self) -> Option<&Scene> {
        self.scenes.values().max_by_key(|s| s.index)
    }

    pub fn fact(&self, prop: &PropositionId) -> Option<&Fact> {
        self.facts.get(prop)
    }

    pub fn fact_value(&self, prop: &PropositionId) -> Option<&Value> {
        self.facts.get(prop).map(|f| &f.value)
    }

    pub fn belief(&self, holder: &SubjectId, prop: &PropositionId) -> Option<&Belief> {
        self.beliefs.get(&BeliefKey::new(holder.clone(), prop.clone()))
    }

    /// 玩家自己的认知投影。有限上帝视角就是它（docs/02 §4.2）。
    pub fn user_belief(&self, prop: &PropositionId) -> Option<&Belief> {
        self.belief(&SubjectId::user(), prop)
    }

    pub fn subject(&self, id: &SubjectId) -> Option<&Subject> {
        self.subjects.get(id)
    }

    pub fn proposition(&self, id: &PropositionId) -> Option<&Proposition> {
        self.propositions.get(id)
    }

    /// 仍然活跃（未消散、未爆发）的趋势。
    pub fn latent_tendencies(&self) -> impl Iterator<Item = &Tendency> {
        self.tendencies
            .values()
            .filter(|t| t.status == TendencyStatus::Latent)
    }

    /// 当前生效的规则。
    pub fn active_rules(&self) -> impl Iterator<Item = &WorldRule> {
        let now = self.world_time;
        self.rules.values().filter(move |r| r.is_active_at(now))
    }

    /// 作用在某主体上的生效规则。
    pub fn rules_for(&self, subject: &SubjectId) -> impl Iterator<Item = &WorldRule> + '_ {
        let subject = subject.clone();
        self.active_rules().filter(move |r| r.applies_to(&subject))
    }

    /// 某命题当前的事实值——只有 L1 保护期内才拒绝「无触发事件」的改动，
    /// 这条查询供裁决层判断（docs/01 §7）。
    pub fn is_protected(&self, prop: &PropositionId) -> bool {
        let scene = self.current_scene_index();
        self.facts
            .get(prop)
            .is_some_and(|f| f.is_protected_at(scene))
    }

    /// 公开事实的数量，供快速体检与测试断言。
    pub fn public_fact_count(&self) -> usize {
        self.facts
            .values()
            .filter(|f| f.visibility == Visibility::Public)
            .count()
    }

    /// 断言某命题存在。
    fn require_proposition(&self, prop: &PropositionId) -> Result<(), ProjectionError> {
        self.propositions
            .contains_key(prop)
            .then_some(())
            .ok_or_else(|| ProjectionError::MissingProposition(prop.clone()))
    }

    fn require_subject(&self, subject: &SubjectId) -> Result<(), ProjectionError> {
        self.subjects
            .contains_key(subject)
            .then_some(())
            .ok_or_else(|| ProjectionError::MissingSubject(subject.clone()))
    }
}

impl Default for Projection {
    fn default() -> Self {
        Projection::genesis(WorldLineId::new("wl_main"))
    }
}

/// 一份「世界尚未创建」的投影，`fact_history` 里那些 L3 事实不会被回溯撤销。
pub fn retcon_immune_count(history: &[Fact], facts: &BTreeMap<PropositionId, Fact>) -> usize {
    history
        .iter()
        .chain(facts.values())
        .filter(|f| f.lock.is_axiom())
        .count()
}

/// 把 `DependsOn` 解析成投影里的取值，供回溯影响分析使用（docs/03 §7）。
pub fn resolve_dependency<'a>(projection: &'a Projection, dep: &DependsOn) -> Option<&'a Value> {
    match dep {
        DependsOn::Fact { prop } => projection.fact_value(prop),
        DependsOn::Belief { holder, prop } => projection.belief(holder, prop).map(|b| &b.value),
        DependsOn::Event { .. } => None,
    }
}

/// 一条事实是否受锁定等级保护，不被普通推演改写。
pub fn lock_allows_world_change(lock: Lock) -> bool {
    matches!(lock, Lock::L0 | Lock::L1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventType, Patch};
    use crate::id::{SceneId, TurnId};
    use crate::narrative::{ScenePlan, ThreadStage};
    use crate::subject::{PropositionKind, SubjectKind, ValueType};

    fn ev(seq: u64, payload: Patch) -> Event {
        Event {
            id: EventId::numbered(seq),
            line: WorldLineId::new("wl_main"),
            seq,
            world_time: WorldTime::from_days(seq as i64),
            narrative_order: None,
            payload,
            caused_by: vec![],
            depends_on: vec![],
            turn: TurnId::new("turn_1"),
            scene: None,
            beat: None,
        }
    }

    fn prop(id: &str) -> Proposition {
        Proposition {
            id: PropositionId::new(id),
            key: format!("k.{id}"),
            text: id.into(),
            subjects: vec![],
            kind: PropositionKind::State,
            value_type: ValueType::Bool,
            internal: false,
        }
    }

    fn subject(id: &str) -> Subject {
        Subject::new(id, SubjectKind::Character, id)
    }

    #[test]
    fn facts_are_single_valued_per_proposition() {
        let mut p = Projection::genesis("wl_main");
        p.apply(&ev(1, Patch::PropositionCreated(Box::new(prop("p_go")))))
            .unwrap();
        p.apply(&ev(
            2,
            Patch::FactSet(Box::new(Fact::new(
                "p_go",
                true,
                WorldTime::from_days(0),
                Lock::L1,
                "evt_0002",
            ))),
        ))
        .unwrap();
        p.apply(&ev(
            3,
            Patch::FactSet(Box::new(Fact::new(
                "p_go",
                false,
                WorldTime::from_days(3),
                Lock::L1,
                "evt_0003",
            ))),
        ))
        .unwrap();

        assert_eq!(p.fact_value(&PropositionId::new("p_go")), Some(&Value::Bool(false)));
        // 旧值被自动结束并进了历史
        assert_eq!(p.fact_history.len(), 1);
        assert_eq!(p.fact_history[0].valid_to, Some(WorldTime::from_days(3)));
    }

    #[test]
    fn fold_is_deterministic_for_the_same_event_sequence() {
        let events: Vec<Event> = vec![
            ev(
                1,
                Patch::WorldCreated {
                    settings: WorldSettings {
                        seed: 77492,
                        ..Default::default()
                    },
                    label: "雨城".into(),
                },
            ),
            ev(2, Patch::SubjectCreated(Box::new(subject("c_lin")))),
            ev(3, Patch::PropositionCreated(Box::new(prop("p_go")))),
            ev(
                4,
                Patch::FactSet(Box::new(
                    Fact::new("p_go", true, WorldTime::from_days(1), Lock::L1, "evt_0004")
                        .with_visibility(Visibility::Public),
                )),
            ),
            ev(
                5,
                Patch::BeliefSet(Box::new(Belief::witnessed(
                    "c_lin",
                    "p_go",
                    true,
                    WorldTime::from_days(1),
                    "evt_0005",
                ))),
            ),
        ];

        let a = Projection::fold("wl_main", &events).unwrap();
        let b = Projection::fold("wl_main", &events).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.world_seed, Some(77492));
        assert_eq!(a.public_fact_count(), 1);

        // 序列化也必须一致——键序固定是确定性的前提
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn narrative_order_is_a_brand_new_cursor() {
        let mut p = Projection::genesis("wl_main");
        let mut e = ev(1, Patch::IfConflictResolved { note: "x".into() });
        e.narrative_order = Some(7);
        p.apply(&e).unwrap();
        assert_eq!(p.narrative_order, 7);
        // 回放中的旧事件不会把游标拽回去
        let mut older = ev(2, Patch::IfConflictResolved { note: "y".into() });
        older.narrative_order = Some(3);
        p.apply(&older).unwrap();
        assert_eq!(p.narrative_order, 7);
    }

    #[test]
    fn retcon_writing_to_past_does_not_move_the_clock_back() {
        let mut p = Projection::genesis("wl_main");
        p.apply(&ev(1, Patch::TimeAdvanced { to: WorldTime::from_days(10) }))
            .unwrap();
        let mut past = ev(2, Patch::IfConflictResolved { note: "改写过去".into() });
        past.world_time = WorldTime::from_days(2);
        p.apply(&past).unwrap();
        assert_eq!(p.world_time, WorldTime::from_days(10));
    }

    #[test]
    fn missing_references_are_rejected() {
        let mut p = Projection::genesis("wl_main");
        let err = p
            .apply(&ev(
                1,
                Patch::FactSet(Box::new(Fact::new(
                    "p_nope",
                    true,
                    WorldTime::EPOCH,
                    Lock::L0,
                    "evt_0001",
                ))),
            ))
            .unwrap_err();
        assert_eq!(
            err,
            ProjectionError::MissingProposition(PropositionId::new("p_nope"))
        );
    }

    #[test]
    fn user_holder_needs_no_subject_row() {
        let mut p = Projection::genesis("wl_main");
        p.apply(&ev(1, Patch::PropositionCreated(Box::new(prop("p_go")))))
            .unwrap();
        // 玩家不是 Subject 表里的一行
        p.apply(&ev(
            2,
            Patch::BeliefSet(Box::new(Belief::witnessed(
                SubjectId::user(),
                "p_go",
                true,
                WorldTime::EPOCH,
                "evt_0002",
            ))),
        ))
        .unwrap();
        assert!(p.user_belief(&PropositionId::new("p_go")).is_some());
    }

    #[test]
    fn thread_stage_change_stamps_current_scene() {
        let mut p = Projection::genesis("wl_main");
        p.apply(&ev(
            1,
            Patch::ThreadOpened(Box::new(Thread {
                id: ThreadId::new("thr_1"),
                title: "顾言会不会离开".into(),
                question: "他会离开吗".into(),
                stakes: String::new(),
                subjects: vec![],
                stage: ThreadStage::Seeded,
                protected_until: Some(ThreadStage::Climax),
                pressure: 0.2,
                last_advanced: 0,
                cadence: 3.0,
            })),
        ))
        .unwrap();

        let plan = ScenePlan {
            goal: "开场".into(),
            pov: SubjectId::new("c_lin"),
            focus: vec![],
            present: vec![SubjectId::new("c_lin")],
            time_span: "当晚".into(),
            required_beats: vec![],
            stop_condition: "".into(),
            forbidden_resolutions: vec![],
            reveal_allowed: vec![],
            proposed_changes: vec![],
        };
        p.apply(&ev(
            2,
            Patch::SceneStarted(Box::new(Scene {
                id: SceneId::new("scene_1"),
                index: 0,
                plan,
                started_at: WorldTime::EPOCH,
                completed_at: None,
            })),
        ))
        .unwrap();
        assert_eq!(p.current_scene_index(), 1);

        p.apply(&ev(
            3,
            Patch::ThreadStageChanged {
                thread: ThreadId::new("thr_1"),
                stage: ThreadStage::Developing,
                pressure: 0.4,
            },
        ))
        .unwrap();
        let t = p.threads.get(&ThreadId::new("thr_1")).unwrap();
        assert_eq!(t.stage, ThreadStage::Developing);
        // last_advanced 自动打上当前场景序号
        assert_eq!(t.last_advanced, 1);
    }

    #[test]
    fn l1_protection_is_visible_to_the_resolver() {
        let mut p = Projection::genesis("wl_main");
        p.apply(&ev(1, Patch::PropositionCreated(Box::new(prop("p_go")))))
            .unwrap();
        p.apply(&ev(
            2,
            Patch::FactSet(Box::new(
                Fact::new("p_go", true, WorldTime::EPOCH, Lock::L1, "evt_0002")
                    .with_protection(crate::rule::L1_PROTECTION_SCENES),
            )),
        ))
        .unwrap();
        assert!(p.is_protected(&PropositionId::new("p_go")));
    }

    #[test]
    fn patch_event_type_matches_projection_effect() {
        let e = ev(1, Patch::PropositionCreated(Box::new(prop("p_x"))));
        assert_eq!(e.event_type(), EventType::PropositionCreated);
    }
}
