//! 事实 / 信念 / 声称三层（docs/02 §3–§5），以及玩家的观测记录。

use serde::{Deserialize, Serialize};

use crate::id::{ClaimId, EventId, PropositionId, SubjectId};
use crate::subject::Proposition;
use crate::value::{Lock, Value, Visibility, WorldTime};

/// 「依据」引用。文档 02 写作 `depends_on: string[]`，这里升级成枚举：
/// 回溯修复要沿它做传递闭包（docs/03 §7），类型分清才不容易写错。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependsOn {
    Fact { prop: PropositionId },
    Event { event: EventId },
    Belief { holder: SubjectId, prop: PropositionId },
}

/// 事实：Reality 层（docs/02 §3）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub prop: PropositionId,
    pub value: Value,
    pub valid_from: WorldTime,
    /// 被后续事件结束时填写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<WorldTime>,
    pub lock: Lock,
    /// L1 保护期截止的场景序号（docs/01 §7）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_until: Option<u64>,
    pub visibility: Visibility,
    #[serde(default)]
    pub depends_on: Vec<DependsOn>,
    pub source: EventId,
}

impl Fact {
    pub fn new(
        prop: impl Into<PropositionId>,
        value: impl Into<Value>,
        valid_from: WorldTime,
        lock: Lock,
        source: impl Into<EventId>,
    ) -> Self {
        Self {
            prop: prop.into(),
            value: value.into(),
            valid_from,
            valid_to: None,
            lock,
            protected_until: None,
            visibility: Visibility::Private,
            depends_on: Vec::new(),
            source: source.into(),
        }
    }

    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn with_protection(mut self, until_scene: u64) -> Self {
        self.protected_until = Some(until_scene);
        self
    }

    /// 在给定世界时刻是否有效。同一命题同一时刻只有一个有效值（docs/02 §3）。
    pub fn is_valid_at(&self, at: WorldTime) -> bool {
        self.valid_from <= at && self.valid_to.map_or(true, |end| at < end)
    }

    /// 在给定场景序号上是否仍处于 L1 保护期。
    pub fn is_protected_at(&self, scene_index: u64) -> bool {
        self.lock == Lock::L1 && self.protected_until.map_or(false, |end| scene_index < end)
    }
}

/// 信念：Belief 层。没有记录即「一无所知」（docs/02 §4）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Belief {
    pub holder: SubjectId,
    pub prop: PropositionId,
    /// 相信的值，可以与事实不同。
    pub value: Value,
    /// 0–1，相信程度。
    pub belief: f64,
    /// 0–1，坚定程度：越高越难被新证据改变。
    pub certainty: f64,
    /// 亲历、被告知、推断。
    #[serde(default)]
    pub sources: Vec<EventId>,
    pub updated_at: WorldTime,
}

impl Belief {
    /// 亲历公开事件：直接获得信念，belief 高（docs/02 §4.1）。
    pub fn witnessed(
        holder: impl Into<SubjectId>,
        prop: impl Into<PropositionId>,
        value: impl Into<Value>,
        at: WorldTime,
        source: impl Into<EventId>,
    ) -> Self {
        Self {
            holder: holder.into(),
            prop: prop.into(),
            value: value.into(),
            belief: 0.95,
            certainty: 0.8,
            sources: vec![source.into()],
            updated_at: at,
        }
    }

    /// 被告知（Claim）时的更新公式（docs/02 §4.1）：
    /// `belief' = certainty × belief + (1 − certainty) × level`
    ///
    /// `level` 是可信度等级对应的数值（见 docs/06 §7）；`certainty` 越高越难被撼动。
    /// 返回新的 `Belief`，原值不动——投影只由事件驱动，这个函数是纯计算。
    pub fn after_claim(&self, level: f64, source: impl Into<EventId>, at: WorldTime) -> Self {
        let level = level.clamp(0.0, 1.0);
        let next = self.certainty * self.belief + (1.0 - self.certainty) * level;
        let mut sources = self.sources.clone();
        sources.push(source.into());
        Self {
            belief: next.clamp(0.0, 1.0),
            sources,
            updated_at: at,
            ..self.clone()
        }
    }

    /// 从零建立一条信念（此前一无所知，第一次被告知）。
    pub fn from_claim(
        holder: impl Into<SubjectId>,
        prop: impl Into<PropositionId>,
        value: impl Into<Value>,
        level: f64,
        source: impl Into<EventId>,
        at: WorldTime,
    ) -> Self {
        Self {
            holder: holder.into(),
            prop: prop.into(),
            value: value.into(),
            belief: level.clamp(0.0, 1.0),
            // 第一次听到某件事时，立场还不坚定。
            certainty: 0.3,
            sources: vec![source.into()],
            updated_at: at,
        }
    }
}

/// 声称的真诚度（docs/02 §5）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sincerity {
    Sincere,
    Lie,
    /// 字面为真但意在误导——需要 Jev 判定 `q.claim.misleading`。
    Mislead,
    Unknown,
}

impl Sincerity {
    /// 引擎能机械推出的部分：把声称的值和说话者当时的信念比较。
    ///
    /// 一致 → sincere，相反 → lie，其余交给 Jev（mislead / unknown）。
    pub fn derive(claimed: &Value, believed: Option<&Value>) -> Self {
        match believed {
            Some(b) if b == claimed => Sincerity::Sincere,
            Some(_) => Sincerity::Lie,
            None => Sincerity::Unknown,
        }
    }
}

/// 声称：Claim 层。它本身永远不直接写入事实层（docs/02 §5）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub id: ClaimId,
    pub speaker: SubjectId,
    #[serde(default)]
    pub audience: Vec<SubjectId>,
    pub prop: PropositionId,
    pub claimed_value: Value,
    pub sincerity: Sincerity,
    pub world_time: WorldTime,
    pub source: EventId,
}

/// 观测结果写入玩家认知投影，不改变世界（docs/01 §13）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub target: ObservationTarget,
    /// LLM 措辞后的观测文本。
    pub text: String,
    /// 这次观测揭示了哪些命题，玩家认知据此更新。
    #[serde(default)]
    pub revealed: Vec<PropositionId>,
    pub world_time: WorldTime,
    pub source: EventId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationTarget {
    Subject { subject: SubjectId },
    Proposition { prop: PropositionId },
    Thread { thread: crate::id::ThreadId },
    Location { subject: SubjectId },
}

/// 供投影快速判断「某命题的值能不能被某个角色看见」。
pub fn belief_is_visible_to(
    proposition: &Proposition,
    fact: &Fact,
    holder: &SubjectId,
    scene_index: u64,
) -> bool {
    if fact.visibility.is_public() {
        return true;
    }
    // 非公开事实只有相关主体能感知（docs/02 §3）。
    proposition.subjects.iter().any(|s| s == holder) && !fact.is_protected_at(scene_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_validity_window_is_half_open() {
        let f = Fact::new(
            "p_x",
            true,
            WorldTime::from_days(1),
            Lock::L1,
            "evt_0001",
        )
        .with_visibility(Visibility::Public);
        assert!(!f.is_valid_at(WorldTime::from_days(1).plus_minutes(-1)));
        assert!(f.is_valid_at(WorldTime::from_days(1)));
        let mut ended = f.clone();
        ended.valid_to = Some(WorldTime::from_days(2));
        assert!(ended.is_valid_at(WorldTime::from_days(2).plus_minutes(-1)));
        assert!(!ended.is_valid_at(WorldTime::from_days(2)));
    }

    #[test]
    fn l1_protection_expires_by_scene() {
        let f = Fact::new("p_x", 1.0, WorldTime::EPOCH, Lock::L1, "evt_0001")
            .with_protection(3);
        assert!(f.is_protected_at(0));
        assert!(f.is_protected_at(2));
        assert!(!f.is_protected_at(3));
        // 非 L1 的事实没有保护期
        let l2 = Fact::new("p_y", 1.0, WorldTime::EPOCH, Lock::L2, "evt_0001").with_protection(9);
        assert!(!l2.is_protected_at(0));
    }

    #[test]
    fn claim_update_follows_documented_formula() {
        let b = Belief {
            holder: SubjectId::new("c_gu"),
            prop: PropositionId::new("p_lin_betrayed"),
            value: Value::Bool(false),
            belief: 0.2,
            certainty: 0.5,
            sources: vec![],
            updated_at: WorldTime::EPOCH,
        };
        // 0.5 * 0.2 + 0.5 * 0.9 = 0.55
        let after = b.after_claim(0.9, "evt_0002", WorldTime::from_hours(1));
        assert!((after.belief - 0.55).abs() < 1e-9);
        assert_eq!(after.sources.len(), 1);
    }

    #[test]
    fn high_certainty_resists_new_evidence() {
        let mut b = Belief::from_claim("c_gu", "p_x", true, 0.5, "evt_1", WorldTime::EPOCH);
        b.certainty = 0.95;
        let after = b.after_claim(0.0, "evt_2", WorldTime::EPOCH);
        // 极度确信时，一条反证几乎撼不动
        assert!(after.belief > 0.45);
    }

    #[test]
    fn sincerity_is_derived_from_speaker_belief() {
        assert_eq!(
            Sincerity::derive(&Value::Bool(true), Some(&Value::Bool(true))),
            Sincerity::Sincere
        );
        assert_eq!(
            Sincerity::derive(&Value::Bool(true), Some(&Value::Bool(false))),
            Sincerity::Lie
        );
        assert_eq!(Sincerity::derive(&Value::Bool(true), None), Sincerity::Unknown);
    }
}
