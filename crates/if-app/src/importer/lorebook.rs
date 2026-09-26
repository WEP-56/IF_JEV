//! 世界书条目解析。
//!
//! 需要同时吃下两套方言：
//!
//! 1. **CCv3 规范**（`character_book` / `lorebook_v3`）：
//!    `keys`、`secondary_keys`、`content`、`enabled`、`insertion_order`、
//!    `use_regex`、`constant`、`position: "before_char" | "after_char"`。
//! 2. **酒馆运行时 World Info 导出**：
//!    `key`、`keysecondary`、`content`、`disable`、`order`、`selectiveLogic`(0–3)、
//!    `position`(0–6 数字)、`depth`、`role`(0–2)、`probability`、`group` 等。
//!
//! 依据 docs/13 §2、§3.1，以及 CCv3 规范与 SillyTavern 文档的实测核对结果。

use serde_json::Value;

use super::model::{ImportedLore, LoreLogic, LoreRole, LoreSection};
use super::value::{flag, number, optional_flag, strings, text, to_i32};

/// 方言标记：CCv3 规范字段。
const DIALECT_CCV3: &str = "ccv3";
/// 方言标记：酒馆运行时 World Info 导出。
const DIALECT_WORLD_INFO: &str = "world_info";

/// 解析一组条目。接受数组（CCv3）或 `uid → entry` 对象（酒馆运行时导出）。
pub fn parse_entries(value: &Value) -> Vec<ImportedLore> {
    let pairs: Vec<(String, &Value)> = match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (index.to_string(), item))
            .collect(),
        Value::Object(map) => map.iter().map(|(key, item)| (key.clone(), item)).collect(),
        _ => Vec::new(),
    };
    pairs
        .into_iter()
        .filter_map(|(uid, entry)| parse_entry(&uid, entry))
        .collect()
}

/// 单条条目 → `ImportedLore`。非对象、或内容为空的条目直接跳过。
fn parse_entry(uid: &str, entry: &Value) -> Option<ImportedLore> {
    if !entry.is_object() {
        return None;
    }
    let raw_content = text(entry, "content");
    if raw_content.trim().is_empty() {
        return None;
    }
    let dialect = if entry.get("keys").is_some()
        || entry.get("insertion_order").is_some()
        || entry.get("enabled").is_some()
    {
        DIALECT_CCV3
    } else {
        DIALECT_WORLD_INFO
    };

    let decorators = Decorators::split(&raw_content);
    let keys = strings(entry.get("key").or_else(|| entry.get("keys")));
    let secondary_keys = strings(entry.get("keysecondary").or_else(|| entry.get("secondary_keys")));

    // `enabled`（CCv3）与 `disable`（酒馆）语义相反；缺少两者时按启用处理。
    let enabled = optional_flag(entry, "enabled").unwrap_or_else(|| !flag(entry, "disable"));

    let order = number(entry, "insertion_order")
        .or_else(|| number(entry, "order"))
        .unwrap_or(100.0);

    let logic = number(entry, "selectiveLogic")
        .map(|value| LoreLogic::from_world_info(value as i64))
        .unwrap_or_default();

    let position = entry.get("position");
    let mut section = section_from_position(position);
    if let Some(overridden) = decorators.section() {
        section = overridden;
    }

    let title = first_non_empty([
        text(entry, "comment"),
        text(entry, "name"),
        keys.first().cloned().unwrap_or_default(),
    ])
    .unwrap_or_else(|| format!("条目 {uid}"));

    let source_uid = text(entry, "uid");
    let source_uid = if source_uid.is_empty() {
        text(entry, "id")
    } else {
        source_uid
    };
    let source_uid = if source_uid.is_empty() {
        uid.to_owned()
    } else {
        source_uid
    };

    Some(ImportedLore {
        title,
        content: decorators.content,
        keys,
        secondary_keys,
        constant: flag(entry, "constant"),
        enabled,
        order: to_i32(order),
        priority: number(entry, "priority"),
        selective: flag(entry, "selective"),
        logic,
        section,
        depth: number(entry, "depth"),
        role: number(entry, "role").map(|value| LoreRole::from_world_info(value as i64)),
        probability: number(entry, "probability"),
        use_probability: optional_flag(entry, "useProbability"),
        group: non_empty(text(entry, "group")),
        group_weight: number(entry, "groupWeight"),
        group_override: optional_flag(entry, "groupOverride"),
        sticky: number(entry, "sticky"),
        cooldown: number(entry, "cooldown"),
        delay: number(entry, "delay"),
        scan_depth: number(entry, "scanDepth"),
        case_sensitive: optional_flag(entry, "case_sensitive")
            .or_else(|| optional_flag(entry, "caseSensitive")),
        match_whole_words: optional_flag(entry, "matchWholeWords"),
        use_regex: optional_flag(entry, "use_regex"),
        exclude_recursion: optional_flag(entry, "excludeRecursion"),
        prevent_recursion: optional_flag(entry, "preventRecursion"),
        delay_until_recursion: optional_flag(entry, "delayUntilRecursion"),
        character_filter: entry.get("characterFilter").cloned(),
        vectorized: flag(entry, "vectorized"),
        decorators: decorators.lines,
        extensions: entry.get("extensions").cloned(),
        source_uid: Some(source_uid),
        source_dialect: dialect.to_owned(),
    })
}

/// `position` → IF 三段（docs/13 §2）。
///
/// - 角色定义前后 → 角色段
/// - @深度 → 场景段
/// - 示例前后、作者注释上下 → 世界段
fn section_from_position(position: Option<&Value>) -> LoreSection {
    match position {
        Some(Value::String(raw)) => match raw.to_ascii_lowercase().as_str() {
            "before_char" | "after_char" => LoreSection::Character,
            "at_depth" | "depth" => LoreSection::Scene,
            // before_an / after_an / before_em / after_em 以及未识别值归世界段。
            _ => LoreSection::World,
        },
        // 酒馆运行时：0 ↑Char、1 ↓Char、2 ↑AN、3 ↓AN、4 @D、5 ↑EM、6 ↓EM。
        Some(Value::Number(value)) => match value.as_i64() {
            Some(0) | Some(1) => LoreSection::Character,
            Some(4) => LoreSection::Scene,
            _ => LoreSection::World,
        },
        _ => LoreSection::World,
    }
}

/// 从 content 中剥离 V3 装饰器（`@@` 行，回退装饰器为 `@@@`）。
struct Decorators {
    lines: Vec<String>,
    content: String,
}

impl Decorators {
    fn split(content: &str) -> Self {
        let mut lines = Vec::new();
        let mut kept = Vec::new();
        for line in content.lines() {
            if line.trim_start().starts_with("@@") {
                lines.push(line.trim().to_owned());
            } else {
                kept.push(line);
            }
        }
        Self {
            lines,
            content: kept.join("\n").trim().to_owned(),
        }
    }

    /// 装饰器对段落的覆盖：深度类 → 场景段，`@@position` → 角色段。
    fn section(&self) -> Option<LoreSection> {
        let has_depth = self.lines.iter().any(|line| {
            let name = decorator_name(line);
            matches!(
                name,
                "@@depth" | "@@instruct_depth" | "@@reverse_depth" | "@@reverse_instruct_depth"
            )
        });
        if has_depth {
            return Some(LoreSection::Scene);
        }
        self.lines
            .iter()
            .any(|line| decorator_name(line) == "@@position")
            .then_some(LoreSection::Character)
    }
}

/// 取装饰器名（不含值），已去掉回退装饰器的第三个 `@`。
fn decorator_name(line: &str) -> &str {
    let trimmed = line.trim();
    let body = trimmed.strip_prefix("@@@").unwrap_or(trimmed);
    body.split_whitespace().next().unwrap_or_default()
}

fn first_non_empty<const N: usize>(candidates: [String; N]) -> Option<String> {
    candidates.into_iter().find(|item| !item.trim().is_empty())
}

fn non_empty(text: String) -> Option<String> {
    (!text.trim().is_empty()).then_some(text)
}

/// 世界书顶层元数据。
pub fn parse_meta(value: &Value) -> super::model::LorebookMeta {
    super::model::LorebookMeta {
        name: text(value, "name"),
        description: text(value, "description"),
        scan_depth: number(value, "scan_depth").or_else(|| number(value, "scanDepth")),
        token_budget: number(value, "token_budget").or_else(|| number(value, "tokenBudget")),
        recursive_scanning: optional_flag(value, "recursive_scanning")
            .or_else(|| optional_flag(value, "recursiveScanning")),
    }
}

/// 条目里出现但 v1 不支持的机制，转成待审阅提示。
pub fn sustainability_warnings(lore: &[ImportedLore]) -> Vec<String> {
    let mut warnings = Vec::new();
    if lore.iter().any(|item| item.vectorized) {
        warnings.push("有条目依赖向量检索激活；v1 不支持向量激活，导入后仅保留内容，需要人工决定是否改为关键词触发。".to_owned());
    }
    if lore.iter().any(|item| item.use_regex == Some(true)) {
        warnings.push("有条目的 keys 声明为正则匹配（use_regex）；v1 的设定条目按字面关键词匹配，正则需人工确认。".to_owned());
    }
    if lore.iter().any(|item| item.character_filter.is_some()) {
        warnings.push("有条目带 characterFilter（仅对指定角色生效）；需要确认对应主体是否存在于本世界。".to_owned());
    }
    if lore
        .iter()
        .any(|item| item.sticky.is_some() || item.cooldown.is_some() || item.delay.is_some())
    {
        warnings.push("有条目带 sticky / cooldown / delay 定时效果；酒馆按消息数计，IF 侧准备按场景数换算。".to_owned());
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_ccv3_entry_with_position_and_regex_flag() {
        let entries = json!([{
            "id": 0,
            "keys": ["长安"],
            "secondary_keys": ["夜禁"],
            "comment": "城规",
            "content": "夜禁",
            "constant": true,
            "selective": true,
            "insertion_order": 340,
            "enabled": true,
            "position": "before_char",
            "extensions": {"x": 1}
        }]);
        let lore = parse_entries(&entries);
        assert_eq!(lore.len(), 1);
        let entry = &lore[0];
        assert_eq!(entry.title, "城规");
        assert_eq!(entry.keys, vec!["长安"]);
        assert_eq!(entry.secondary_keys, vec!["夜禁"]);
        assert!(entry.constant && entry.enabled);
        assert_eq!(entry.order, 340);
        assert_eq!(entry.section, LoreSection::Character);
        assert_eq!(entry.source_dialect, DIALECT_CCV3);
        assert!(entry.extensions.is_some());
        assert_eq!(entry.source_uid.as_deref(), Some("0"));
    }

    #[test]
    fn disabled_ccv3_entry_is_not_enabled() {
        // 这是修复回归：旧实现只看 disable，会把 enabled:false 误判为启用。
        let entries = json!([{"keys": ["a"], "content": "c", "enabled": false, "insertion_order": 1}]);
        assert!(!parse_entries(&entries)[0].enabled);
    }

    #[test]
    fn parses_world_info_entry_with_numeric_position_and_logic() {
        let entries = json!({
            "42": {
                "uid": 42,
                "key": "暴雨,河水",
                "keysecondary": ["夏季"],
                "content": "河水上涨",
                "disable": true,
                "order": 220,
                "selectiveLogic": 2,
                "position": 4,
                "depth": 2,
                "role": 2,
                "probability": 60,
                "group": "天气",
                "groupWeight": 30,
                "sticky": 3
            }
        });
        let lore = parse_entries(&entries);
        assert_eq!(lore.len(), 1);
        let entry = &lore[0];
        assert_eq!(entry.keys, vec!["暴雨", "河水"]);
        assert_eq!(entry.secondary_keys, vec!["夏季"]);
        assert!(!entry.enabled, "disable:true 应表示停用");
        assert_eq!(entry.order, 220);
        assert_eq!(entry.logic, LoreLogic::NotAny);
        assert_eq!(entry.section, LoreSection::Scene);
        assert_eq!(entry.role, Some(LoreRole::Assistant));
        assert_eq!(entry.depth, Some(2.0));
        assert_eq!(entry.probability, Some(60.0));
        assert_eq!(entry.group.as_deref(), Some("天气"));
        assert_eq!(entry.sticky, Some(3.0));
        assert_eq!(entry.source_dialect, DIALECT_WORLD_INFO);
        assert_eq!(entry.source_uid.as_deref(), Some("42"));
    }

    #[test]
    fn strips_decorators_and_uses_depth_for_section() {
        let entries = json!([{
            "keys": ["a"],
            "content": "@@depth 2\n@@role assistant\n真正的设定内容",
            "insertion_order": 1
        }]);
        let entry = &parse_entries(&entries)[0];
        assert_eq!(entry.content, "真正的设定内容");
        assert_eq!(entry.decorators.len(), 2);
        assert_eq!(entry.section, LoreSection::Scene, "@@depth 应覆盖 position");
    }

    #[test]
    fn skips_entries_without_content_and_non_objects() {
        let entries = json!([{"keys": ["a"], "content": "  "}, "junk", {"keys": ["b"], "content": "ok"}]);
        let lore = parse_entries(&entries);
        assert_eq!(lore.len(), 1);
        assert_eq!(lore[0].content, "ok");
    }
}
