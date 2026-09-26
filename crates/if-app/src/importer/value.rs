//! `serde_json::Value` 的容错取字段工具。
//!
//! 导入面对的是社区来源的 JSON，字段缺失、类型不符都很常见，
//! 这里统一按「取不到就回落默认值」处理，不 panic、不静默改写已有值。

use serde_json::Value;

pub fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

pub fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn optional_flag(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

/// 字符串数组：接受数组，也接受逗号分隔的单个字符串（酒馆两种写法都有）。
pub fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(Value::String(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// 顶层对象的字段名清单，用于提示「哪些字段没有被映射」。
pub fn object_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

/// 把浮点数安全收敛为 i32（社区卡里出现过 1e9 这类越界值）。
pub fn to_i32(value: f64) -> i32 {
    if !value.is_finite() {
        return 100;
    }
    value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// 条目字段的分层读取：先看条目顶层，再看 `extensions`。
///
/// 实测（2026-09-26，两张真实 V3 卡）表明：卡内嵌 `character_book` 的顶层字段常常
/// 是导出工具写的默认值（`insertion_order` 全卡统一、`selective` / `use_regex` 全为
/// true），而酒馆引擎真正使用的条目状态放在 `extensions` 里（28 个键，含 `depth`、
/// `role`、`probability`、`selectiveLogic`、`position` 等）。
///
/// `null` 视为未设置并继续往下找。
pub fn pick<'a>(entry: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        if let Some(found) = entry.get(*key).filter(|value| !value.is_null()) {
            return Some(found);
        }
    }
    let extensions = entry.get("extensions")?;
    for key in keys {
        if let Some(found) = extensions.get(*key).filter(|value| !value.is_null()) {
            return Some(found);
        }
    }
    None
}

pub fn pick_number(entry: &Value, keys: &[&str]) -> Option<f64> {
    pick(entry, keys).and_then(Value::as_f64)
}

/// 只认正数。酒馆把 `sticky` / `cooldown` / `delay` 未启用写成显式的 `0`，
/// 若直接当作「有值」会误报「带定时效果」。
pub fn pick_positive(entry: &Value, keys: &[&str]) -> Option<f64> {
    pick_number(entry, keys).filter(|value| *value > 0.0)
}

pub fn pick_flag(entry: &Value, keys: &[&str]) -> Option<bool> {
    pick(entry, keys).and_then(Value::as_bool)
}

pub fn pick_text(entry: &Value, keys: &[&str]) -> String {
    pick(entry, keys)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub fn pick_strings(entry: &Value, keys: &[&str]) -> Vec<String> {
    strings(pick(entry, keys))
}
