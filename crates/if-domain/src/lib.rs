//! # if-domain
//!
//! IF 的领域类型。全部来自 [docs/02 世界模型](https://github.com/WEP-56/IF_JEV/blob/main/docs/02-世界模型.md)
//! 与 [docs/03 事件与世界线](https://github.com/WEP-56/IF_JEV/blob/main/docs/03-事件与世界线.md)。
//!
//! 三条贯穿性的设计约束：
//!
//! 1. **事件日志是唯一真相。** 世界状态是 [`projection::Projection`] —— 把事件折叠出来的结果，
//!    没有任何代码可以直接修改它（[docs/03 §1]）。
//! 2. **事实与认知分离。** [`state::Fact`]（Reality）、[`state::Belief`]（Belief）、
//!    [`state::Claim`]（Claim）是三类独立记录（[docs/02 §3–§5]）。
//! 3. **确定性。** 所有集合用 `BTreeMap`，序列化键序固定；同一事件序列永远得到同一个投影
//!    （[docs/12 §7]）。这是命运骰子和重放能成立的前提。
//!
//! 本 crate 只依赖 `serde`：投影是纯计算，不碰存储与网络，因此折叠逻辑可以直接单元测试。
//!
//! [docs/02 §3–§5]: https://github.com/WEP-56/IF_JEV/blob/main/docs/02-世界模型.md
//! [docs/03 §1]: https://github.com/WEP-56/IF_JEV/blob/main/docs/03-事件与世界线.md
//! [docs/12 §7]: https://github.com/WEP-56/IF_JEV/blob/main/docs/12-工程架构.md

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod event;
pub mod id;
pub mod narrative;
pub mod projection;
pub mod rule;
pub mod state;
pub mod subject;
pub mod turn;
pub mod value;
pub mod worldline;

pub use event::{
    CausedBy, DigestionStrategy, Event, EventType, IfInjection, IfKind, IfScope, Patch,
    Retcon, TimeAnchor,
};
pub use id::{
    BeatId, CandidateId, ClaimId, EventId, JudgmentId, LoreId, PropositionId, RuleId, SceneId,
    SubjectId, TendencyId, ThreadId, TurnId, WorldLineId, USER_HOLDER,
};
pub use narrative::{
    tendency_push_delta, Beat, LoreEntry, LoreSection, LoreStatus, LoreVisibility, RevealTarget,
    Scene, ScenePlan, ScenePlanError, SecondaryLogic, Tendency, TendencyStatus, Thread, ThreadStage,
    MAX_BEATS_PER_SCENE, MAX_BEAT_RETRIES, TENDENCY_DECAY_PER_SCENE, TENDENCY_DISSOLVE_THRESHOLD,
    TENDENCY_ERUPT_THRESHOLD, TENDENCY_PUSH_DELTAS, TENDENCY_WATCH_THRESHOLD,
};
pub use projection::{BeliefKey, Projection, ProjectionAnchor, ProjectionError};
pub use rule::{
    ActiveWindow, CompareOp, Condition, Invariant, InvariantCheck, Mechanic, MechanicPeriod,
    MechanicStep, Mode, NarrativePov, NumericRange, Trigger, TriggerOutput, TriggerProduces,
    WorldRule, WorldSettings, L1_PROTECTION_SCENES,
};
pub use state::{Belief, Claim, DependsOn, Fact, Observation, ObservationTarget, Sincerity};
pub use subject::{Proposition, PropositionKind, Subject, SubjectKind, Tier, ValueType};
pub use turn::{
    Candidate, CandidateShape, Judgment, JudgmentOutput, JudgmentUsage, Primitive, Resolution,
    ResolutionOutcome, ResolutionPolicy, TaskFinish, TaskRecord, ToolCallRecord, TurnKind,
    TurnMetrics, TurnRecord, ViewKind, ViewRef,
};
pub use value::{Lock, Value, Visibility, WorldTime};
pub use worldline::{
    common_ancestor, covers, lineage, ParentRef, Segment, WorldLine, WorldLineError, WorldLineKind,
};
