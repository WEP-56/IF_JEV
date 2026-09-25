//! 主体与命题（docs/02 §1、§2）。

use serde::{Deserialize, Serialize};

use crate::id::{EventId, PropositionId, SubjectId};

/// 主体的类别（docs/02 §1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    Character,
    Group,
    Faction,
    Location,
    Item,
    Concept,
}

/// 模拟分辨率（docs/09 §1）。升降温条件见 docs/09 §1.1。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// 暂时无关：只跑机制与趋势，不模拟个体行动。
    Dormant,
    /// 与当前故事线直接相关：按场景或时间段批量结算。
    Active,
    /// 当前镜头：逐行动、逐认知模拟。
    Foreground,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    pub id: SubjectId,
    pub kind: SubjectKind,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 自然语言设定：外貌、性格、背景。
    #[serde(default)]
    pub profile: String,
    /// 说话方式与台词样例（可来自角色卡的 `mes_example`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    pub tier: Tier,
    /// 是否已定型。定型后核心设定升为 L2，只能经 IF 修改（docs/10 §6）。
    #[serde(default)]
    pub shaped: bool,
    pub created_by: EventId,
}

impl Subject {
    pub fn new(id: impl Into<SubjectId>, kind: SubjectKind, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            name: name.into(),
            aliases: Vec::new(),
            profile: String::new(),
            voice: None,
            tier: Tier::Active,
            shaped: false,
            // 由调用方在写入事件时覆盖成真实的事件 ID。
            created_by: EventId::new("evt_pending"),
        }
    }
}

/// 命题类别（docs/02 §2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropositionKind {
    /// 主体的状态。
    State,
    /// 两个主体之间的状态。
    Relation,
    /// 目标、计划、决定——角色自驱力的来源。
    Intention,
    /// 某事件是否发生过。
    Occurrence,
    /// 相对稳定的特质。
    Trait,
}

/// 取值类型。`enum` 与 `scalar` 携带自己的取值域。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValueType {
    Bool,
    Enum { values: Vec<String> },
    Scalar {
        min: f64,
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Text,
}

impl ValueType {
    /// 命题取值是否为数值型——机制的 `delta` 只能作用在数值命题上。
    pub const fn is_numeric(&self) -> bool {
        matches!(self, ValueType::Scalar { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposition {
    pub id: PropositionId,
    /// 规范键，如 `c_lin.feeling.c_gu`。界面显示用 `text`。
    pub key: String,
    /// 自然语言描述，如「林夏对顾言的感情」。
    pub text: String,
    #[serde(default)]
    pub subjects: Vec<SubjectId>,
    pub kind: PropositionKind,
    pub value_type: ValueType,
    /// 内在状态（情感、意图、信念倾向）为 true；外在状态为 false。
    /// 对账规则用它区分「可以只靠预演提交」和「必须在正文里出现过」（docs/02 §11）。
    pub internal: bool,
}

impl Proposition {
    /// 按 `<主体ID>.<属性>[.<对象ID>]` 生成规范键（docs/02 §2.1）。
    pub fn make_key(owner: &SubjectId, attribute: &str, object: Option<&SubjectId>) -> String {
        match object {
            Some(obj) => format!("{}.{}.{}", owner.as_str(), attribute, obj.as_str()),
            None => format!("{}.{}", owner.as_str(), attribute),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_has_expected_shape() {
        let lin = SubjectId::new("c_lin");
        let gu = SubjectId::new("c_gu");
        assert_eq!(Proposition::make_key(&lin, "mood", None), "c_lin.mood");
        assert_eq!(
            Proposition::make_key(&lin, "feeling", Some(&gu)),
            "c_lin.feeling.c_gu"
        );
    }

    #[test]
    fn internal_flag_distinguishes_state_from_action() {
        let p = Proposition {
            id: PropositionId::new("p_1"),
            key: "c_lin.feeling.c_gu".into(),
            text: "林夏对顾言的感情".into(),
            subjects: vec![SubjectId::new("c_lin"), SubjectId::new("c_gu")],
            kind: PropositionKind::Relation,
            value_type: ValueType::Enum {
                values: vec!["爱".into(), "朋友".into(), "无".into()],
            },
            internal: true,
        };
        assert!(p.internal);
        assert!(!p.value_type.is_numeric());
    }
}
