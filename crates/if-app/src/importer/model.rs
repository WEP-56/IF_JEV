//! 导入结果的规范化 DTO。
//!
//! 原则（docs/10 §2、docs/13）：**忠实保留来源字段，不做静默改写**。
//! 已知字段映射为 IF 需要的结构，未知字段与来源信息保留为警告或原样附带，
//! 语义抽取（主体 / 事实 / 故事线）留给后续 T-parse，不在这里用 LLM。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 设定条目在 IF 上下文中的归段。映射依据 docs/13 §2 的 `position` 行。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoreSection {
    /// 世界层：世界书正文、示例前后、作者注释上下。
    #[default]
    World,
    /// 角色层：角色定义前后。
    Character,
    /// 场景层：@深度插入（IF 没有聊天深度的概念，统一收敛到场景段）。
    Scene,
}

/// 主 / 次关键词的组合逻辑，对应酒馆 `selectiveLogic` 0–3（docs/13 §2【已核实】）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoreLogic {
    /// 0 = AND ANY：主关键词命中，且任一次级关键词命中。
    #[default]
    AndAny,
    /// 1 = NOT ALL：主关键词命中，且并非所有次级关键词都命中。
    NotAll,
    /// 2 = NOT ANY：主关键词命中，且没有任何次级关键词命中。
    NotAny,
    /// 3 = AND ALL：主关键词命中，且所有次级关键词都命中。
    AndAll,
}

impl LoreLogic {
    /// 酒馆 `selectiveLogic` 数值 → 逻辑。未知值按 AND ANY 处理。
    pub fn from_world_info(value: i64) -> Self {
        match value {
            1 => Self::NotAll,
            2 => Self::NotAny,
            3 => Self::AndAll,
            _ => Self::AndAny,
        }
    }
}

/// @深度插入时的角色，对应酒馆 `role` 0–2（docs/13 §3.1【已核实】）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoreRole {
    System,
    User,
    Assistant,
}

impl LoreRole {
    pub fn from_world_info(value: i64) -> Self {
        match value {
            1 => Self::User,
            2 => Self::Assistant,
            _ => Self::System,
        }
    }
}

/// 世界书本身的元数据（酒馆世界书顶层字段）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LorebookMeta {
    pub name: String,
    pub description: String,
    /// 扫描最近 N 条消息；IF 侧换算为最近 N 个节拍。
    pub scan_depth: Option<f64>,
    /// token 预算；IF 侧换算为世界书预算裁剪。
    pub token_budget: Option<f64>,
    pub recursive_scanning: Option<bool>,
}

/// 角色卡 V3 的 `assets` 条目。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedAsset {
    pub asset_type: String,
    pub uri: String,
    pub name: String,
    pub ext: String,
}

/// 导入后的一条设定条目（世界书条目）。
///
/// 同时承载两套来源方言的字段（CCv3 `character_book` / `lorebook_v3` 一套，
/// 酒馆运行时 World Info 导出一套），无法映射到同一语义的保持原样并标注来源。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedLore {
    /// 条目名：优先 `comment`，其次 `name`，再次首个关键词，最后回落为用户 id。
    pub title: String,
    pub content: String,
    pub keys: Vec<String>,
    pub secondary_keys: Vec<String>,
    pub constant: bool,
    pub enabled: bool,
    pub order: i32,
    pub priority: Option<f64>,
    pub selective: bool,
    pub logic: LoreLogic,
    pub section: LoreSection,
    /// @深度插入的深度（position = 4 / `@@depth`）。
    pub depth: Option<f64>,
    pub role: Option<LoreRole>,
    /// 触发概率（酒馆为 0–100，保留原值不归一化）。
    pub probability: Option<f64>,
    pub use_probability: Option<bool>,
    pub group: Option<String>,
    pub group_weight: Option<f64>,
    pub group_override: Option<bool>,
    /// 定时效果，单位按消息数计（IF 侧准备按场景数换算）。
    pub sticky: Option<f64>,
    pub cooldown: Option<f64>,
    pub delay: Option<f64>,
    pub scan_depth: Option<f64>,
    pub case_sensitive: Option<bool>,
    pub match_whole_words: Option<bool>,
    /// V3 新增：`keys` 是否按正则匹配。
    pub use_regex: Option<bool>,
    pub exclude_recursion: Option<bool>,
    pub prevent_recursion: Option<bool>,
    pub delay_until_recursion: Option<bool>,
    /// 只对指定角色生效；仅当指定主体在场时激活。
    pub character_filter: Option<Value>,
    /// 需要向量检索才激活的条目（v1 不支持，记录以便审阅）。
    pub vectorized: bool,
    /// 从 content 中剥离出的 V3 装饰器（`@@depth` 等），原样保留供审阅。
    pub decorators: Vec<String>,
    /// 条目级 `extensions`，规范要求不得丢弃。
    pub extensions: Option<Value>,
    /// 来源条目的 id / uid，便于回指原文件。
    pub source_uid: Option<String>,
    /// 本条来自哪套方言：`ccv3` 或 `world_info`。
    pub source_dialect: String,
}

/// 导入后的一个角色。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedCharacter {
    pub name: String,
    /// V3：存在时 `{{char}}` 应替换为该值。
    pub nickname: Option<String>,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_message: String,
    pub alternate_greetings: Vec<String>,
    pub example_messages: String,
    pub aliases: Vec<String>,
    pub system_prompt: String,
    pub post_history_instructions: String,
    pub creator_notes: String,
    /// V3：仅用于群聊的额外开场（不用于单卡导入）。
    pub group_only_greetings: Vec<String>,
    pub assets: Vec<ImportedAsset>,
    pub source: Vec<String>,
    pub tags: Vec<String>,
    pub creator: Option<String>,
    pub character_version: Option<String>,
    pub creation_date: Option<i64>,
    pub modification_date: Option<i64>,
    pub extensions: Option<Value>,
}

/// 一次导入的完整结果。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedWorld {
    pub name: String,
    pub genre: String,
    pub summary: String,
    /// 来源格式标识：`chara_card_v2` / `chara_card_v3` / `lorebook_v3` / `world_info_json` / `chara_card_v1`。
    pub source_format: String,
    /// 文件层面来源：`json` / `png:ccv3` / `png:chara`。
    pub source_kind: String,
    /// 来源文件名（仅用于回指，不参与映射）。
    pub source_file: Option<String>,
    /// 角色卡顶层 `spec_version`。
    pub spec_version: Option<String>,
    pub characters: Vec<ImportedCharacter>,
    pub lore: Vec<ImportedLore>,
    pub lore_meta: LorebookMeta,
    /// 卡内出现的宏（`{{user}}` / `{{char}}` 等），供 D14 处理前审阅。
    pub macros: Vec<String>,
    /// 顶层字段名清单，便于用户核对未映射字段。
    pub source_fields: Vec<String>,
    /// 无法映射或需要人工确认的事项。
    pub warnings: Vec<String>,
}

impl ImportedWorld {
    pub fn push_warning(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.warnings.iter().any(|existing| existing == &text) {
            self.warnings.push(text);
        }
    }
}
