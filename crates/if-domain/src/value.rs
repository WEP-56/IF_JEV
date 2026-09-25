//! 值与时间，以及跨模块共用的小枚举。

use std::fmt;

use serde::{Deserialize, Serialize};

/// 世界时间：自世界纪元起经过的**分钟数**（已确认，见 docs/02 §12）。
///
/// 内部一律用整数，显示格式由世界设定决定（例如「第 7 天 02:17」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldTime(i64);

impl WorldTime {
    pub const EPOCH: WorldTime = WorldTime(0);

    pub const fn from_minutes(minutes: i64) -> Self {
        WorldTime(minutes)
    }

    pub const fn minutes(self) -> i64 {
        self.0
    }

    pub const fn from_hours(hours: i64) -> Self {
        WorldTime(hours * 60)
    }

    pub const fn from_days(days: i64) -> Self {
        WorldTime(days * 60 * 24)
    }

    pub fn plus_minutes(self, minutes: i64) -> Self {
        WorldTime(self.0 + minutes)
    }

    pub fn saturating_sub(self, other: WorldTime) -> i64 {
        self.0.saturating_sub(other.0)
    }

    /// 世界时间是否严格早于另一时刻。回溯修复靠它判断「写回过去」。
    pub fn is_before(self, other: WorldTime) -> bool {
        self.0 < other.0
    }
}

impl fmt::Display for WorldTime {
    /// 默认展示成「第 N 天 HH:MM」；世界设定可以覆盖这个格式。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.0;
        let day = total.div_euclid(60 * 24);
        let rem = total.rem_euclid(60 * 24);
        write!(f, "第 {} 天 {:02}:{:02}", day, rem / 60, rem % 60)
    }
}

/// 命题的取值。具体含义由 `Proposition::value_type` 决定（docs/02 §12）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(v) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{v}"),
            Value::Number(v) => write!(f, "{v}"),
            Value::Text(v) => f.write_str(v),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Number(v as f64)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

/// 锁定等级（docs/01 §6）。等级越高越难改变。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Lock {
    /// 自由：世界自然推演。
    L0,
    /// 惯性：保护期内只能被强证据改变。
    L1,
    /// 锚定：只有用户 IF 能改。
    L2,
    /// 公理：只有用户显式的 IF，回溯修复时永不撤销。
    L3,
}

impl Lock {
    /// 数值权重，方便做优先级比较。L3 = 3，L0 = 0。
    pub const fn rank(self) -> u8 {
        match self {
            Lock::L0 => 0,
            Lock::L1 => 1,
            Lock::L2 => 2,
            Lock::L3 => 3,
        }
    }

    /// 该等级的锁定是否在回溯修复中被视为不可撤销。
    pub const fn is_axiom(self) -> bool {
        matches!(self, Lock::L3)
    }
}

impl fmt::Display for Lock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Lock::L0 => "L0",
            Lock::L1 => "L1",
            Lock::L2 => "L2",
            Lock::L3 => "L3",
        })
    }
}

/// 事实的可见性（docs/02 §3）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// 被刻意隐藏。
    Secret,
    /// 只有相关主体知道。
    Private,
    /// 在场者都能感知。
    Public,
}

impl Visibility {
    /// 是否能默认传入某个角色的视图。只有 public 可以直接进叙事视图。
    pub const fn is_public(self) -> bool {
        matches!(self, Visibility::Public)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_time_displays_day_and_clock() {
        assert_eq!(WorldTime::from_minutes(0).to_string(), "第 0 天 00:00");
        assert_eq!(WorldTime::from_hours(26).to_string(), "第 1 天 02:00");
        assert_eq!(WorldTime::from_days(7).plus_minutes(137).to_string(), "第 7 天 02:17");
    }

    #[test]
    fn world_time_handles_negative_as_days_ago() {
        // 回溯型 IF 会把 world_time 写进过去，允许负值。
        assert_eq!(WorldTime::from_minutes(-60).minutes(), -60);
    }

    #[test]
    fn lock_ordering_follows_priority() {
        assert!(Lock::L3 > Lock::L2);
        assert!(Lock::L2 > Lock::L1);
        assert!(Lock::L1 > Lock::L0);
        assert!(Lock::L3.is_axiom());
        assert!(!Lock::L2.is_axiom());
    }

    #[test]
    fn value_round_trips() {
        for v in [Value::Bool(true), Value::Number(0.5), Value::Text("雨".into())] {
            let s = serde_json::to_string(&v).unwrap();
            let back: Value = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }
}
