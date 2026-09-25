//! 事件、事件类型与补丁（docs/03 §2、§4）。
//!
//! 唯一真相是事件日志；世界状态是它的投影（docs/03 §1）。
//! 事件只追加，不修改、不删除。

use serde::{Deserialize, Serialize};

use crate::id::{
    BeatId, EventId, LoreId, PropositionId, SceneId, SubjectId, TendencyId, ThreadId, TurnId,
    WorldLineId,
};
use crate::narrative::{Beat, LoreEntry, Scene, Tendency, TendencyStatus, Thread, ThreadStage};
use crate::rule::WorldRule;
use crate::state::{Belief, Claim, DependsOn, Fact, Observation};
use crate::subject::{Proposition, Subject, Tier};
use crate::value::{Lock, WorldTime};

// ---------------------------------------------------------------- IF 相关的枚举

/// IF 的类型（docs/01 §5）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfKind {
    /// 主体此刻的状态、关系、意图。
    State,
    /// 主体此刻相信什么。只写入 Belief 层。
    Belief,
    /// 世界从现在起如何运作。
    Rule,
    /// 刚发生或正在发生的事。事件本身 L3，后果 L0。
    Occurrence,
    /// 一直为真、但未必有人知道的事。
    Truth,
    /// 与已展示的过去矛盾的事。
    Retcon,
}

/// 时间锚点（裁定卡字段，docs/01 §9.2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeAnchor {
    /// 此刻
    Now,
    /// 过去某时
    Past,
    /// 一直如此
    Always,
}

/// 作用范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfScope {
    Individual,
    Group,
    Region,
    Global,
}

/// 回溯型 IF 的消化策略（docs/01 §12）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestionStrategy {
    /// 过去被替换，下游重新判定，已展示正文标记为旧世界残影。
    Rewrite,
    /// 过去的事件保留，补充新事实改变其意义。
    Reinterpret,
    /// 同改写，但指定角色保留旧记忆（Belief ≠ Reality）。
    Anomaly,
}

/// 一次 IF 注入的完整记录。它本身也是事件，核心命题在投影中享有 Canon 地位。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IfInjection {
    /// 用户原文。
    pub input: String,
    pub kind: IfKind,
    /// 核心命题的自然语言表述。
    pub core: String,
    /// 核心命题落到哪个命题上。新命题时由引擎先发 `proposition_created`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_prop: Option<PropositionId>,
    pub time_anchor: TimeAnchor,
    pub scope: IfScope,
    pub lock: Lock,
    /// 仅回溯型必填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digestion: Option<DigestionStrategy>,
    /// 不承诺项：人们常会默认、但这条 IF 并不保证的推论（docs/01 §8）。
    #[serde(default)]
    pub non_commitments: Vec<String>,
    /// 世界线偏移 = −ln(max(p, p_min))（docs/01 §15）。
    pub world_line_shift: f64,
    /// 若这条 IF 由导演指令改写而来，记录改写文本。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewritten_from: Option<String>,
    /// 裁定卡是否被自动确认（默认关闭，docs/01 §9.2）。
    #[serde(default)]
    pub auto_confirmed: bool,
}

/// 回溯执行的记录（docs/03 §7）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Retcon {
    pub strategy: DigestionStrategy,
    /// 被改写的那个过去时刻。
    pub rewritten_at: WorldTime,
    /// 执行前自动保存的备份分支。
    pub backup_line: WorldLineId,
    /// 影响分析找出的下游依赖。
    #[serde(default)]
    pub affected: Vec<DependsOn>,
}

// ---------------------------------------------------------------- 因果引用

/// 事件的因果来源：IF 注入、其他事件，或趋势爆发。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CausedBy {
    Injection { event: EventId },
    Event { event: EventId },
    Tendency { tendency: TendencyId },
}

// ---------------------------------------------------------------- 补丁

/// 事件对投影的修改（docs/03 §2）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "patch", rename_all = "snake_case")]
pub enum Patch {
    WorldCreated {
        settings: crate::rule::WorldSettings,
        label: String,
    },
    SubjectCreated(Box<Subject>),
    SubjectShaped {
        subject: SubjectId,
    },
    /// 命题的创建。docs/03 §4 的事件类型表漏了这一项——没有它命题就无处诞生，
    /// 而命题是事实与信念的落点，必须独立建事件。
    PropositionCreated(Box<Proposition>),
    RuleAdded(Box<WorldRule>),
    RuleChanged(Box<WorldRule>),
    LoreAdded(Box<LoreEntry>),
    LoreSuperseded {
        lore: LoreId,
        by: EventId,
    },
    IfInjected(Box<IfInjection>),
    IfConflictResolved {
        note: String,
    },
    FactSet(Box<Fact>),
    FactEnded {
        prop: PropositionId,
        at: WorldTime,
    },
    BeliefSet(Box<Belief>),
    ClaimMade(Box<Claim>),
    SceneStarted(Box<Scene>),
    BeatDisplayed(Box<Beat>),
    SceneCompleted {
        scene: SceneId,
        at: WorldTime,
    },
    TimeAdvanced {
        to: WorldTime,
    },
    ThreadOpened(Box<Thread>),
    ThreadStageChanged {
        thread: ThreadId,
        stage: ThreadStage,
        pressure: f64,
    },
    TendencyCreated(Box<Tendency>),
    TendencyUpdated {
        tendency: TendencyId,
        pressure: f64,
        status: TendencyStatus,
    },
    BackgroundSettled {
        subject: SubjectId,
        text: String,
        from: WorldTime,
        to: WorldTime,
    },
    UserObserved(Box<Observation>),
    RetconApplied(Box<Retcon>),
    RetconRepaired {
        target: DependsOn,
        note: String,
    },
    /// 模拟分辨率的升降级（docs/09 §1.1）。
    TierChanged {
        subject: SubjectId,
        tier: Tier,
    },
}

impl EventType {
    /// 稳定字符串，写进 SQLite 的 `event_type` 列。改这里等于改存储格式。
    pub const fn as_str(self) -> &'static str {
        match self {
            EventType::WorldCreated => "world_created",
            EventType::SubjectCreated => "subject_created",
            EventType::SubjectShaped => "subject_shaped",
            EventType::PropositionCreated => "proposition_created",
            EventType::RuleAdded => "rule_added",
            EventType::RuleChanged => "rule_changed",
            EventType::LoreAdded => "lore_added",
            EventType::LoreSuperseded => "lore_superseded",
            EventType::IfInjected => "if_injected",
            EventType::IfConflictResolved => "if_conflict_resolved",
            EventType::FactSet => "fact_set",
            EventType::FactEnded => "fact_ended",
            EventType::BeliefSet => "belief_set",
            EventType::ClaimMade => "claim_made",
            EventType::SceneStarted => "scene_started",
            EventType::BeatDisplayed => "beat_displayed",
            EventType::SceneCompleted => "scene_completed",
            EventType::TimeAdvanced => "time_advanced",
            EventType::ThreadOpened => "thread_opened",
            EventType::ThreadStageChanged => "thread_stage_changed",
            EventType::TendencyCreated => "tendency_created",
            EventType::TendencyUpdated => "tendency_updated",
            EventType::TendencyErupted => "tendency_erupted",
            EventType::BackgroundSettled => "background_settled",
            EventType::UserObserved => "user_observed",
            EventType::RetconApplied => "retcon_applied",
            EventType::RetconRepaired => "retcon_repaired",
            EventType::TierChanged => "tier_changed",
        }
    }
}

impl Patch {
    /// 投影里可被观测到的「主要动作」涉及的主体，供视图编译与索引使用。
    pub fn subject(&self) -> Option<&SubjectId> {
        match self {
            Patch::SubjectCreated(s) => Some(&s.id),
            Patch::SubjectShaped { subject } | Patch::TierChanged { subject, .. } => Some(subject),
            Patch::BeliefSet(b) => Some(&b.holder),
            Patch::ClaimMade(c) => Some(&c.speaker),
            Patch::BackgroundSettled { subject, .. } => Some(subject),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- 事件类型

/// 事件类型（docs/03 §4）。它由 `Patch` 派生，不单独存储，避免两处不一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    WorldCreated,
    SubjectCreated,
    SubjectShaped,
    PropositionCreated,
    RuleAdded,
    RuleChanged,
    LoreAdded,
    LoreSuperseded,
    IfInjected,
    IfConflictResolved,
    FactSet,
    FactEnded,
    BeliefSet,
    ClaimMade,
    SceneStarted,
    BeatDisplayed,
    SceneCompleted,
    TimeAdvanced,
    ThreadOpened,
    ThreadStageChanged,
    TendencyCreated,
    TendencyUpdated,
    TendencyErupted,
    BackgroundSettled,
    UserObserved,
    RetconApplied,
    RetconRepaired,
    TierChanged,
}

impl Patch {
    pub fn event_type(&self) -> EventType {
        match self {
            Patch::WorldCreated { .. } => EventType::WorldCreated,
            Patch::SubjectCreated(_) => EventType::SubjectCreated,
            Patch::SubjectShaped { .. } => EventType::SubjectShaped,
            Patch::PropositionCreated(_) => EventType::PropositionCreated,
            Patch::RuleAdded(_) => EventType::RuleAdded,
            Patch::RuleChanged(_) => EventType::RuleChanged,
            Patch::LoreAdded(_) => EventType::LoreAdded,
            Patch::LoreSuperseded { .. } => EventType::LoreSuperseded,
            Patch::IfInjected(_) => EventType::IfInjected,
            Patch::IfConflictResolved { .. } => EventType::IfConflictResolved,
            Patch::FactSet(_) => EventType::FactSet,
            Patch::FactEnded { .. } => EventType::FactEnded,
            Patch::BeliefSet(_) => EventType::BeliefSet,
            Patch::ClaimMade(_) => EventType::ClaimMade,
            Patch::SceneStarted(_) => EventType::SceneStarted,
            Patch::BeatDisplayed(_) => EventType::BeatDisplayed,
            Patch::SceneCompleted { .. } => EventType::SceneCompleted,
            Patch::TimeAdvanced { .. } => EventType::TimeAdvanced,
            Patch::ThreadOpened(_) => EventType::ThreadOpened,
            Patch::ThreadStageChanged { .. } => EventType::ThreadStageChanged,
            Patch::TendencyCreated(_) => EventType::TendencyCreated,
            Patch::TendencyUpdated { status, .. } => match status {
                TendencyStatus::Erupted => EventType::TendencyErupted,
                _ => EventType::TendencyUpdated,
            },
            Patch::BackgroundSettled { .. } => EventType::BackgroundSettled,
            Patch::UserObserved(_) => EventType::UserObserved,
            Patch::RetconApplied(_) => EventType::RetconApplied,
            Patch::RetconRepaired { .. } => EventType::RetconRepaired,
            Patch::TierChanged { .. } => EventType::TierChanged,
        }
    }
}

// ---------------------------------------------------------------- 事件

/// 事件（docs/03 §2）。
///
/// 两处与文档的字面差异，都是为了消除冗余或不一致：
/// 1. 文档里的 `type` 字段在这里由 `payload.event_type()` 派生，不单独存字段——
///    否则 `type` 和 `payload` 可能对不上，而事件日志是唯一真相，不能自相矛盾。
/// 2. `depends_on` / `caused_by` 用枚举而非裸字符串，回溯修复要靠它们做传递闭包。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    /// 首次写入时所在的世界线。
    pub line: WorldLineId,
    /// 追加顺序，全局单调递增。
    pub seq: u64,
    /// 在世界中发生的时间。**可以早于前一条事件**——回溯型 IF 就写进过去。
    pub world_time: WorldTime,
    /// 叙述顺序。只有上屏的事件才有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative_order: Option<u64>,
    pub payload: Patch,
    #[serde(default)]
    pub caused_by: Vec<CausedBy>,
    #[serde(default)]
    pub depends_on: Vec<DependsOn>,
    pub turn: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<BeatId>,
}

impl Event {
    pub fn event_type(&self) -> EventType {
        self.payload.event_type()
    }

    /// 这个事件是否上屏（有叙述顺序）。
    pub fn is_displayed(&self) -> bool {
        self.narrative_order.is_some()
    }

    /// 是否是写入过去的事件。回溯修复的产物都落在过去。
    pub fn writes_to_past(&self, current: WorldTime) -> bool {
        self.world_time.is_before(current)
    }

    /// 是否可被回溯修复撤销。L3 事实不参与撤销（docs/03 §7）。
    pub fn is_retcon_immune(&self) -> bool {
        matches!(&self.payload, Patch::FactSet(f) if f.lock.is_axiom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subject::{PropositionKind, ValueType};

    fn prop(id: &str) -> Proposition {
        Proposition {
            id: PropositionId::new(id),
            key: format!("{id}.state"),
            text: "某状态".into(),
            subjects: vec![],
            kind: PropositionKind::State,
            value_type: ValueType::Bool,
            internal: false,
        }
    }

    fn event(id: &str, seq: u64, payload: Patch) -> Event {
        Event {
            id: EventId::new(id),
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

    #[test]
    fn event_type_is_derived_from_payload() {
        let e = event("evt_1", 1, Patch::PropositionCreated(Box::new(prop("p_1"))));
        assert_eq!(e.event_type(), EventType::PropositionCreated);
    }

    #[test]
    fn tendency_patch_reports_eruption_as_its_own_type() {
        let mut t = Tendency::from_failed_candidate("tnd_1", "x", 0.5, "evt_1");
        t.status = TendencyStatus::Erupted;
        let e = event(
            "evt_2",
            2,
            Patch::TendencyUpdated {
                tendency: t.id.clone(),
                pressure: t.pressure,
                status: t.status,
            },
        );
        assert_eq!(e.event_type(), EventType::TendencyErupted);
    }

    #[test]
    fn l3_facts_are_retcon_immune() {
        let mut f = Fact::new("p_1", true, WorldTime::EPOCH, Lock::L3, "evt_1");
        let e = event("evt_1", 1, Patch::FactSet(Box::new(f.clone())));
        assert!(e.is_retcon_immune());
        f.lock = Lock::L1;
        let e2 = event("evt_2", 2, Patch::FactSet(Box::new(f)));
        assert!(!e2.is_retcon_immune());
    }

    #[test]
    fn past_writes_are_detectable() {
        let mut e = event(
            "evt_9",
            9,
            Patch::FactEnded {
                prop: PropositionId::new("p_1"),
                at: WorldTime::from_days(1),
            },
        );
        e.world_time = WorldTime::from_days(1);
        assert!(e.writes_to_past(WorldTime::from_days(10)));
        assert!(!e.writes_to_past(WorldTime::from_days(1)));
    }
}
