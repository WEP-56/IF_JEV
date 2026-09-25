//! 稳定标识符。
//!
//! 所有 ID 都是 `newtype(String)`，serde 上透明成裸字符串，方便直接读写 JSON 与 SQLite。
//! 名字可以改，ID 不变（见 docs/02 §1）。

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal, $doc:literal) => {
        $(#[$meta])*
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 该 ID 类别的推荐前缀。
            pub const PREFIX: &'static str = $prefix;

            pub fn new(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// 生成带前缀的序号 ID，例如 `evt_0192`（零填充 4 位，超出则自然变长）。
            pub fn numbered(n: u64) -> Self {
                Self(format!("{}{:04}", $prefix, n))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

id_type!(SubjectId, "c_", "主体 ID，如 `c_lin`。特殊值 `user` 表示玩家自己。");
id_type!(PropositionId, "p_", "命题 ID。规范键另外放在 `Proposition::key`。");
id_type!(EventId, "evt_", "事件 ID，如 `evt_0192`。");
id_type!(WorldLineId, "wl_", "世界线 ID。");
id_type!(TurnId, "turn_", "回合 ID。");
id_type!(SceneId, "scene_", "场景 ID。");
id_type!(BeatId, "beat_", "节拍 ID。");
id_type!(ThreadId, "thr_", "故事线 ID。");
id_type!(TendencyId, "tnd_", "趋势 ID。");
id_type!(LoreId, "lore_", "设定条目 ID。");
id_type!(RuleId, "rule_", "世界规则 ID。");
id_type!(ClaimId, "claim_", "声称 ID。");
id_type!(JudgmentId, "jdg_", "判定记录 ID。");
id_type!(CandidateId, "cand_", "候选 ID，由引擎分配（docs/05 §5.2）。");

/// 玩家在认知层里的主体 ID。玩家的认知投影就是有限上帝视角（docs/02 §4.2）。
pub const USER_HOLDER: &str = "user";

impl SubjectId {
    /// 玩家本人。
    pub fn user() -> Self {
        Self(USER_HOLDER.to_owned())
    }

    pub fn is_user(&self) -> bool {
        self.0 == USER_HOLDER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_ids_are_zero_padded() {
        assert_eq!(EventId::numbered(192).as_str(), "evt_0192");
        assert_eq!(BeatId::numbered(7).as_str(), "beat_0007");
        // 超出填充宽度时自然变长，不截断
        assert_eq!(EventId::numbered(12345).as_str(), "evt_12345");
    }

    #[test]
    fn serde_is_transparent() {
        let id = SubjectId::new("c_lin");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"c_lin\"");
        let back: SubjectId = serde_json::from_str("\"c_lin\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn user_holder_is_a_subject_id() {
        assert!(SubjectId::user().is_user());
        assert!(!SubjectId::new("c_lin").is_user());
    }
}
