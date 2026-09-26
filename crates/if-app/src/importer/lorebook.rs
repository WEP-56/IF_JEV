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
use super::value::{
    flag, number, optional_flag, pick_flag, pick_number, pick_positive, pick_strings, pick_text,
    text, to_i32,
};

/// 方言标记：CCv3 规范字段。
const DIALECT_CCV3: &str = "ccv3";
/// 方言标记：酒馆运行时 World Info 导出。
const DIALECT_WORLD_INFO: &str = "world_info";

/// 解析一组条目，同时返回被跳过的条目数。
///
/// 入口接受数组（CCv3）或 `uid → entry` 对象（酒馆运行时导出）。
///
/// 真实卡里出现过「清空了正文但没删条目」的残留（实测：某 148 条卡有 1 条 `content` 为空、
/// `comment` 为"总结"）。跳过是对的，但要让用户知道，不能静默少一条。
pub fn parse_entries_counting(value: &Value) -> (Vec<ImportedLore>, usize) {
    let pairs: Vec<(String, &Value)> = match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (index.to_string(), item))
            .collect(),
        Value::Object(map) => map.iter().map(|(key, item)| (key.clone(), item)).collect(),
        _ => Vec::new(),
    };
    let total = pairs.len();
    let lore: Vec<ImportedLore> = pairs
        .into_iter()
        .filter_map(|(uid, entry)| parse_entry(&uid, entry))
        .collect();
    let skipped = total - lore.len();
    (lore, skipped)
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
    let keys = pick_strings(entry, &["key", "keys"]);
    let secondary_keys = pick_strings(entry, &["keysecondary", "secondary_keys"]);

    // `enabled`（CCv3）与 `disable`（酒馆）语义相反；缺少两者时按启用处理。
    let enabled = optional_flag(entry, "enabled").unwrap_or_else(|| !flag(entry, "disable"));

    let order = number(entry, "insertion_order")
        .or_else(|| number(entry, "order"))
        .unwrap_or(100.0);

    let logic = pick_number(entry, &["selectiveLogic"])
        .map(|value| LoreLogic::from_world_info(value as i64))
        .unwrap_or_default();

    let mut section = section_for(entry);
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
        constant: pick_flag(entry, &["constant"]).unwrap_or(false),
        enabled,
        order: to_i32(order),
        priority: pick_number(entry, &["priority"]),
        selective: flag(entry, "selective"),
        logic,
        section,
        depth: pick_number(entry, &["depth"]),
        role: pick_number(entry, &["role"]).map(|value| LoreRole::from_world_info(value as i64)),
        probability: pick_number(entry, &["probability"]),
        use_probability: pick_flag(entry, &["useProbability"]),
        group: non_empty(pick_text(entry, &["group"])),
        group_weight: pick_number(entry, &["groupWeight", "group_weight"]),
        group_override: pick_flag(entry, &["groupOverride", "group_override"]),
        // 定时效果：酒馆把「未启用」写成显式 0，按正数才认定。
        sticky: pick_positive(entry, &["sticky"]),
        cooldown: pick_positive(entry, &["cooldown"]),
        delay: pick_positive(entry, &["delay"]),
        scan_depth: pick_number(entry, &["scanDepth", "scan_depth"]),
        case_sensitive: pick_flag(entry, &["case_sensitive", "caseSensitive"]),
        match_whole_words: pick_flag(entry, &["matchWholeWords", "match_whole_words"]),
        use_regex: pick_flag(entry, &["use_regex"]),
        exclude_recursion: pick_flag(entry, &["excludeRecursion", "exclude_recursion"]),
        prevent_recursion: pick_flag(entry, &["preventRecursion", "prevent_recursion"]),
        delay_until_recursion: pick_flag(entry, &["delayUntilRecursion", "delay_until_recursion"]),
        character_filter: entry.get("characterFilter").cloned(),
        vectorized: pick_flag(entry, &["vectorized"]).unwrap_or(false),
        decorators: decorators.lines,
        extensions: entry.get("extensions").cloned(),
        source_uid: Some(source_uid),
        source_dialect: dialect.to_owned(),
    })
}

/// 条目的位置信号来源。
///
/// CCv3 的顶层 `position` 只有 `before_char` / `after_char` 两值，无法区分世界 / 场景；
/// 卡内嵌 `character_book` 的 `extensions.position` 保留了酒馆的 0–6 数字枚举，信号更细。
/// 实测两张真实卡：`before_char ↔ 0` 完全对应，而 3 条 `after_char` 的真实值是 4（@深度）。
/// 因此数字枚举优先，字符串作为回退。
fn section_for(entry: &Value) -> LoreSection {
    if let Some(numeric) = entry.get("extensions").and_then(|ext| ext.get("position")) {
        if numeric.is_number() || numeric.is_string() {
            return section_from_position(Some(numeric));
        }
    }
    section_from_position(entry.get("position"))
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
    // 实测两张真实卡都把 `use_regex` 标成了全 true（导出工具的默认值），而 keys 本身是
    // 字面词。只有 keys 里真的含正则元字符时，字面匹配才会给出不同结果，才值得提醒。
    let regex_shaped = lore
        .iter()
        .filter(|item| item.use_regex == Some(true) && item.keys.iter().any(|key| has_regex_meta(key)))
        .count();
    if regex_shaped > 0 {
        warnings.push(format!(
            "{regex_shaped} 条条目的 keys 含正则元字符且声明为正则匹配；v1 的设定条目按字面关键词匹配，这些条目需要人工确认匹配规则。"
        ));
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

/// 关键词里是否用了正则语法。只看元字符，不尝试编译（社区卡里存在 `(` 这类非法表达式）。
fn has_regex_meta(key: &str) -> bool {
    key.chars()
        .any(|c| matches!(c, '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 测试里只关心条目本身，跳过数另有断言。
    fn parse_entries(value: &Value) -> Vec<ImportedLore> {
        super::parse_entries_counting(value).0
    }

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

        let (lore, skipped) = parse_entries_counting(&entries);
        assert_eq!(lore.len(), 1);
        assert_eq!(skipped, 2, "被跳过的条目数要能报给用户");
    }

    /// 实测形态：卡内嵌 character_book 的顶层 `position` 是 "after_char"，
    /// 而 `extensions.position` 保留酒馆数字枚举（4 = @深度）。
    #[test]
    fn extensions_position_beats_ccv3_string_position() {
        let entries = json!([{
            "keys": [],
            "comment": "文风",
            "content": "style_guide: ...",
            "constant": true,
            "position": "after_char",
            "extensions": {"position": 4}
        }]);
        let lore = parse_entries(&entries);
        assert_eq!(lore[0].section, LoreSection::Scene, "数字枚举更细，应优先");

        // 数字与字符串一致时（before_char ↔ 0）保持角色段。
        let entries = json!([{
            "keys": [],
            "content": "x",
            "position": "before_char",
            "extensions": {"position": 0}
        }]);
        assert_eq!(parse_entries(&entries)[0].section, LoreSection::Character);
    }

    /// 真实卡把 depth / role / probability 放在 extensions 里，顶层没有。
    #[test]
    fn reads_entry_state_from_extensions_when_top_level_is_absent() {
        let entries = json!([{
            "keys": ["长安"],
            "comment": "城规",
            "content": "夜禁",
            "extensions": {
                "depth": 4,
                "role": 1,
                "probability": 100,
                "useProbability": true,
                "selectiveLogic": 2,
                "group_weight": 80,
                "group_override": true,
                "scan_depth": 3,
                "match_whole_words": true,
                "exclude_recursion": true
            }
        }]);
        let entry = &parse_entries(&entries)[0];
        assert_eq!(entry.depth, Some(4.0));
        assert_eq!(entry.role, Some(LoreRole::User), "1 = user");
        assert_eq!(entry.probability, Some(100.0));
        assert_eq!(entry.use_probability, Some(true));
        assert_eq!(entry.logic, LoreLogic::NotAny, "selectiveLogic 2 = NOT ANY");
        assert_eq!(entry.group_weight, Some(80.0));
        assert_eq!(entry.group_override, Some(true));
        assert_eq!(entry.scan_depth, Some(3.0));
        assert_eq!(entry.match_whole_words, Some(true));
        assert_eq!(entry.exclude_recursion, Some(true));
    }

    /// 酒馆把「未启用定时」写成显式的 0；若当作有值会误报「带定时效果」。
    #[test]
    fn explicit_zero_duration_is_not_a_timer() {
        let entries = json!([{
            "keys": ["a"],
            "content": "x",
            "extensions": {"sticky": 0, "cooldown": 0, "delay": 0}
        }]);
        let lore = parse_entries(&entries);
        assert!(lore[0].sticky.is_none() && lore[0].cooldown.is_none() && lore[0].delay.is_none());
        assert!(sustainability_warnings(&lore).is_empty());
    }

    /// 真实卡把 use_regex 全标 true，但 keys 是字面词——此时不该产生噪音警告。
    #[test]
    fn use_regex_warning_needs_regex_shaped_keys() {
        let literal = parse_entries(&json!([{
            "keys": ["朝廷", "官府"],
            "content": "x",
            "use_regex": true
        }]));
        assert!(sustainability_warnings(&literal).is_empty());

        let shaped = parse_entries(&json!([{
            "keys": ["(", "。", "？"],
            "content": "x",
            "use_regex": true
        }]));
        let warnings = sustainability_warnings(&shaped);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("正则元字符"), "{}", warnings[0]);
    }
}
