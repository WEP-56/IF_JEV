//! 工具参数用的 JSON Schema 子集校验器。
//!
//! 不解析 `$ref` 与外部 URI，preflight 不会获得额外的文件或网络能力。
//! 相对 Onemore 新增：`items`、`minItems`、`maxItems`（IF 的工具参数大量使用数组，
//! docs/05 §3.1），以及 `type` 写成数组（如 `["string","null"]`）。
//! 新增关键字时先补测试再扩展。

use serde_json::Value;

pub fn validate(schema: &Value, value: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    check(schema, value, "", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(s) = schema.as_object() else {
        errors.push(at(path, "schema 必须是 object"));
        return;
    };
    let mut push = |msg: String| errors.push(at(path, &msg));

    if let Some(ty) = s.get("type") {
        let expected: Vec<&str> = match ty {
            Value::String(t) => vec![t.as_str()],
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        for t in &expected {
            if !matches!(*t, "object" | "array" | "string" | "integer" | "number" | "boolean" | "null") {
                push(format!("不支持的 schema type {t:?}"));
                return;
            }
        }
        if !expected.is_empty() && !expected.iter().any(|t| type_matches(t, value)) {
            push(format!("期望类型 {}，实际为 {}", expected.join(" | "), json_type(value)));
            return;
        }
    }
    if let Some(options) = s.get("enum").and_then(Value::as_array) {
        if !options.iter().any(|o| o == value) {
            push(format!("值不在 enum {options:?} 中"));
        }
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = s.get("minimum").and_then(Value::as_f64) {
            if n < min {
                push(format!("数值不得小于 {min}"));
            }
        }
        if let Some(max) = s.get("maximum").and_then(Value::as_f64) {
            if n > max {
                push(format!("数值不得大于 {max}"));
            }
        }
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if let Some(min) = s.get("minLength").and_then(Value::as_u64) {
            if len < min {
                push(format!("字符串长度不得小于 {min}"));
            }
        }
        if let Some(max) = s.get("maxLength").and_then(Value::as_u64) {
            if len > max {
                push(format!("字符串长度不得大于 {max}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        let len = items.len() as u64;
        if let Some(min) = s.get("minItems").and_then(Value::as_u64) {
            if len < min {
                push(format!("数组至少需要 {min} 项，实际 {len} 项"));
            }
        }
        if let Some(max) = s.get("maxItems").and_then(Value::as_u64) {
            if len > max {
                push(format!("数组至多 {max} 项，实际 {len} 项"));
            }
        }
        if let Some(item_schema) = s.get("items") {
            for (i, item) in items.iter().enumerate() {
                check(item_schema, item, &format!("{path}[{i}]"), errors);
            }
        }
    }
    if let Some(object) = value.as_object() {
        let empty = serde_json::Map::new();
        let properties = s.get("properties").and_then(Value::as_object).unwrap_or(&empty);
        if let Some(required) = s.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    errors.push(at(path, &format!("缺少必填字段 {key:?}")));
                }
            }
        }
        if s.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
            for key in object.keys() {
                if !properties.contains_key(key) {
                    errors.push(at(path, &format!("不允许额外字段 {key:?}")));
                }
            }
        }
        for (key, child_schema) in properties {
            if let Some(child) = object.get(key) {
                let child_path = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                check(child_schema, child, &child_path, errors);
            }
        }
    }
}

fn at(path: &str, msg: &str) -> String {
    if path.is_empty() {
        msg.to_owned()
    } else {
        format!("{path}: {msg}")
    }
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn candidate_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "options": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 4,
                    "items": {
                        "type": "object",
                        "properties": { "id": { "type": "string", "minLength": 1 }, "p": { "type": "number", "minimum": 0 } },
                        "required": ["id"],
                        "additionalProperties": false
                    }
                },
                "depends_on": { "type": ["array", "null"], "items": { "type": "string" } }
            },
            "required": ["options"]
        })
    }

    #[test]
    fn accepts_valid_nested_arrays() {
        let v = json!({ "options": [{"id": "a"}, {"id": "b", "p": 0.5}], "depends_on": null });
        assert_eq!(validate(&candidate_schema(), &v), Ok(()));
    }

    #[test]
    fn checks_array_length_and_items_with_paths() {
        let errs = validate(&candidate_schema(), &json!({ "options": [{"id": ""}] })).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("options: 数组至少需要 2 项")), "{errs:?}");
        assert!(errs.iter().any(|e| e.starts_with("options[0].id: 字符串长度")), "{errs:?}");

        let five: Vec<Value> = (0..5).map(|i| json!({"id": i.to_string()})).collect();
        let errs = validate(&candidate_schema(), &json!({ "options": five })).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("至多 4 项")), "{errs:?}");

        let errs = validate(&candidate_schema(), &json!({ "options": [{"id": "a", "x": 1}, {"p": -1}] })).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("options[0]: 不允许额外字段")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("options[1]: 缺少必填字段")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("options[1].p: 数值不得小于")), "{errs:?}");

        let errs = validate(&candidate_schema(), &json!({ "options": [{"id":"a"},{"id":"b"}], "depends_on": [1] })).unwrap_err();
        assert!(errs.iter().any(|e| e.starts_with("depends_on[0]: 期望类型 string")), "{errs:?}");
    }

    #[test]
    fn union_type_rejects_other_types() {
        let errs = validate(&candidate_schema(), &json!({ "options": [{"id":"a"},{"id":"b"}], "depends_on": "x" })).unwrap_err();
        assert!(errs[0].contains("array | null"), "{errs:?}");
    }
}
