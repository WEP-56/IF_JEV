//! 世界规则及其编译产物，以及每个世界单独的设置（docs/01 §11、docs/02 §6、docs/10 §5）。

use serde::{Deserialize, Serialize};

use crate::id::{EventId, PropositionId, RuleId, SubjectId};
use crate::value::{Lock, WorldTime};

/// L1 保护期长度，按场景计（已确认，docs/01 §7）【初始值 3】。
pub const L1_PROTECTION_SCENES: u64 = 3;

/// 不变量：数据层面必须始终成立的条件，由引擎机械校验。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invariant {
    pub id: String,
    pub text: String,
    pub check: InvariantCheck,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InvariantCheck {
    /// 每条 Claim 的内容必须与说话者当时的 Belief 一致（例：「所有人无法说谎」）。
    ClaimsMatchSpeakerBelief,
    /// 某个数值命题在生效期内保持不变（例：「十年内所有人不会衰老」）。
    NumericUnchanged {
        prop: PropositionId,
        tolerance: f64,
    },
    /// 编译不了的部分留给 Jev，这里只记问题模板 ID。
    SemanticOnly { template: String },
}

/// 机制：随世界时间更新数值的公式（docs/02 §6）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mechanic {
    pub id: String,
    pub target: PropositionId,
    pub per: MechanicPeriod,
    /// 每个机制步的增量范围，取值由命运骰子决定（docs/06 §8）。
    pub delta: NumericRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

impl Mechanic {
    /// 用命运骰子取到的 `u ∈ [0,1)` 算出本步的实际增量（docs/06 §8）：
    /// `value = min + u × (max − min)`
    pub fn delta_for(&self, u: f64) -> f64 {
        self.delta.sample(u)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanicPeriod {
    /// 每个机制步长执行一次。
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NumericRange {
    pub min: f64,
    pub max: f64,
}

impl NumericRange {
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    pub fn sample(&self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        self.min + u * (self.max - self.min)
    }
}

/// 触发器：条件满足时生成候选或趋势（docs/02 §6）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    pub id: String,
    pub when: Condition,
    pub produces: TriggerOutput,
    /// 只触发一次（例：水位越过警戒线）。
    #[serde(default)]
    pub once: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerOutput {
    pub kind: TriggerProduces,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerProduces {
    Candidate,
    Tendency,
}

/// 条件：对（数值）命题的比较，可以组合。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    Compare {
        prop: PropositionId,
        cmp: CompareOp,
        value: f64,
    },
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Not { not: Box<Condition> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl CompareOp {
    pub fn test(self, lhs: f64, rhs: f64) -> bool {
        match self {
            CompareOp::Lt => lhs < rhs,
            CompareOp::Le => lhs <= rhs,
            CompareOp::Gt => lhs > rhs,
            CompareOp::Ge => lhs >= rhs,
            CompareOp::Eq => (lhs - rhs).abs() < f64::EPSILON,
            CompareOp::Ne => (lhs - rhs).abs() >= f64::EPSILON,
        }
    }
}

impl Condition {
    /// 求值。`lookup` 给出命题当前的数值；命题不存在或不是数值时返回 `None`，
    /// 而 `None` 一律当作**条件不成立**——缺数据不该触发事件。
    pub fn evaluate<F>(&self, lookup: &F) -> bool
    where
        F: Fn(&PropositionId) -> Option<f64>,
    {
        match self {
            Condition::Compare { prop, cmp, value } => {
                lookup(prop).is_some_and(|current| cmp.test(current, *value))
            }
            Condition::All { all } => all.iter().all(|c| c.evaluate(lookup)),
            Condition::Any { any } => any.iter().any(|c| c.evaluate(lookup)),
            Condition::Not { not } => !not.evaluate(lookup),
        }
    }
}

/// 世界规则（docs/02 §6）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldRule {
    pub id: RuleId,
    pub text: String,
    /// 规则型默认 L2，真相型的核心命题是 L3（docs/01 §5）。
    pub lock: Lock,
    pub source: EventId,
    pub active: ActiveWindow,
    /// 适用的主体或地域；空表示全局。
    #[serde(default)]
    pub scope: Vec<SubjectId>,
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub mechanics: Vec<Mechanic>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    /// Jev 语义约束：问题模板 ID + 参数。
    #[serde(default)]
    pub constraints: Vec<String>,
    /// 边界解释，由用户在裁定卡上确认（docs/01 §11）。
    #[serde(default)]
    pub boundaries: Vec<String>,
}

impl WorldRule {
    pub fn is_active_at(&self, at: WorldTime) -> bool {
        self.active.covers(at)
    }

    /// 规则是否作用于给定主体（空 scope 表示全局）。
    pub fn applies_to(&self, subject: &SubjectId) -> bool {
        self.scope.is_empty() || self.scope.iter().any(|s| s == subject)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveWindow {
    pub from: WorldTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<WorldTime>,
}

impl ActiveWindow {
    pub const fn from_now(from: WorldTime) -> Self {
        Self { from, to: None }
    }

    pub fn covers(&self, at: WorldTime) -> bool {
        self.from <= at && self.to.map_or(true, |end| at < end)
    }
}

/// 机制步长（世界设置，对应前端原来的「时间粒度」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanicStep {
    Hour,
    Day,
    Week,
    Month,
}

impl MechanicStep {
    pub const fn minutes(self) -> i64 {
        match self {
            MechanicStep::Hour => 60,
            MechanicStep::Day => 60 * 24,
            MechanicStep::Week => 60 * 24 * 7,
            MechanicStep::Month => 60 * 24 * 30,
        }
    }

    /// 世界时间段编号，用于命运骰子的决策键（docs/03 §8）。
    /// 粒度等于机制步长，所以两条世界线更容易共用同一颗骰子。
    pub fn bucket(self, at: WorldTime) -> i64 {
        at.minutes().div_euclid(self.minutes())
    }

    /// 决策键里的时间标记，例如 `D7`。
    pub fn bucket_label(self, at: WorldTime) -> String {
        let tag = match self {
            MechanicStep::Hour => "H",
            MechanicStep::Day => "D",
            MechanicStep::Week => "W",
            MechanicStep::Month => "M",
        };
        format!("{}{}", tag, self.bucket(at))
    }
}

/// 叙事视角。默认第三人称有限视角，每个场景一个视角人物（已确认，docs/08 §2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrativePov {
    ThirdLimited,
    FirstPerson,
    Omniscient,
}

/// 模式。v1 只做沙盒，挑战模式预留（docs/01 §16）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sandbox,
    Challenge,
}

/// 世界设置，每个世界单独保存（docs/10 §5）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSettings {
    /// 命运骰子的种子。它属于世界，不属于全局设置。
    pub seed: u64,
    pub narrative_style: String,
    pub narrative_pov: NarrativePov,
    pub mechanic_step: MechanicStep,
    /// 导演风格：一组权重的模板名（docs/06 §6）。
    pub director_style: String,
    pub mode: Mode,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            seed: 0,
            narrative_style: "文学叙事".into(),
            narrative_pov: NarrativePov::ThirdLimited,
            mechanic_step: MechanicStep::Day,
            director_style: "均衡".into(),
            mode: Mode::Sandbox,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_sampling_hits_endpoints() {
        let r = NumericRange::new(0.4, 1.2);
        assert!((r.sample(0.0) - 0.4).abs() < 1e-9);
        assert!((r.sample(1.0) - 1.2).abs() < 1e-9);
        assert!((r.sample(0.5) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn condition_missing_prop_is_false() {
        let c = Condition::Compare {
            prop: PropositionId::new("p_river"),
            cmp: CompareOp::Gt,
            value: 28.5,
        };
        // 命题不存在 → 不触发
        assert!(!c.evaluate(&|_| None));
        assert!(c.evaluate(&|_| Some(29.0)));
        assert!(!c.evaluate(&|_| Some(28.0)));
    }

    #[test]
    fn composite_conditions() {
        let leak = Condition::Any {
            any: vec![
                Condition::Compare {
                    prop: PropositionId::new("p_a"),
                    cmp: CompareOp::Gt,
                    value: 1.0,
                },
                Condition::Not {
                    not: Box::new(Condition::Compare {
                        prop: PropositionId::new("p_b"),
                        cmp: CompareOp::Le,
                        value: 5.0,
                    }),
                },
            ],
        };
        assert!(leak.evaluate(&|p| if p.as_str() == "p_a" { Some(2.0) } else { Some(9.0) }));
        assert!(!leak.evaluate(&|_| Some(0.0)));
    }

    #[test]
    fn mechanic_step_bucketing_is_stable_across_world_lines() {
        let step = MechanicStep::Day;
        let day7_early = WorldTime::from_days(7).plus_minutes(1);
        let day7_late = WorldTime::from_days(7).plus_minutes(1439);
        assert_eq!(step.bucket_label(day7_early), "D7");
        assert_eq!(step.bucket_label(day7_late), "D7");
        assert_eq!(step.bucket_label(WorldTime::from_days(8)), "D8");
    }

    #[test]
    fn empty_scope_means_global() {
        let rule = WorldRule {
            id: RuleId::new("rule_1"),
            text: "所有人无法说谎".into(),
            lock: Lock::L2,
            source: EventId::new("evt_1"),
            active: ActiveWindow::from_now(WorldTime::EPOCH),
            scope: vec![],
            invariants: vec![],
            mechanics: vec![],
            triggers: vec![],
            constraints: vec![],
            boundaries: vec![],
        };
        assert!(rule.applies_to(&SubjectId::new("c_lin")));
        assert!(rule.is_active_at(WorldTime::from_days(100)));
    }
}
