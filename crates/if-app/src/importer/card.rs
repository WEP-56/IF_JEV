//! 角色卡（V1 / V2 / V3）与世界书文档 → `ImportedWorld`。
//!
//! 字段依据 CCv3 规范与 SillyTavern 运行时导出的核对结果（docs/13 §3）。

use serde_json::Value;

use super::lorebook;
use super::model::{ImportedAsset, ImportedCharacter, ImportedWorld, LorebookMeta};
use super::value::{number, object_keys, strings, text};

pub const SPEC_V2: &str = "chara_card_v2";
pub const SPEC_V3: &str = "chara_card_v3";
pub const SPEC_LOREBOOK_V3: &str = "lorebook_v3";
/// 无 `spec` 字段的历史卡。
pub const FORMAT_V1: &str = "chara_card_v1";
/// 酒馆运行时导出的世界书。
pub const FORMAT_WORLD_INFO: &str = "world_info_json";

pub fn spec_of(root: &Value) -> Option<&str> {
    root.get("spec").and_then(Value::as_str)
}

/// 结构上像世界书：有 `entries`、`data.entries` 或 `world_info`。
pub fn is_lorebook(root: &Value) -> bool {
    root.get("entries").is_some()
        || root.get("data").and_then(|value| value.get("entries")).is_some()
        || root.get("world_info").is_some()
}

/// 角色卡 → 世界。V1 没有 `data` 包装，直接读根对象。
pub fn parse_character_card(root: &Value) -> Result<ImportedWorld, String> {
    let spec = spec_of(root).unwrap_or_default().to_owned();
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(root);
    let name = text(data, "name");
    if name.trim().is_empty() {
        return Err("角色卡缺少 name 字段".into());
    }

    let character = ImportedCharacter {
        name: name.clone(),
        nickname: optional(text(data, "nickname")),
        description: text(data, "description"),
        personality: text(data, "personality"),
        scenario: text(data, "scenario"),
        first_message: text(data, "first_mes"),
        alternate_greetings: strings(data.get("alternate_greetings")),
        example_messages: text(data, "mes_example"),
        aliases: strings(data.get("aliases")),
        system_prompt: text(data, "system_prompt"),
        post_history_instructions: text(data, "post_history_instructions"),
        creator_notes: text(data, "creator_notes"),
        group_only_greetings: strings(data.get("group_only_greetings")),
        assets: parse_assets(data.get("assets")),
        source: strings(data.get("source")),
        tags: strings(data.get("tags")),
        creator: optional(text(data, "creator")),
        character_version: optional(text(data, "character_version")),
        creation_date: number(data, "creation_date").map(|value| value as i64),
        modification_date: number(data, "modification_date").map(|value| value as i64),
        extensions: data.get("extensions").cloned(),
    };

    let mut world = ImportedWorld {
        name: name.clone(),
        genre: "待整理".into(),
        summary: character.scenario.clone(),
        source_format: if spec.is_empty() { FORMAT_V1.to_owned() } else { spec.clone() },
        source_kind: "json".into(),
        source_file: None,
        spec_version: optional(text(root, "spec_version")),
        characters: vec![character.clone()],
        lore: Vec::new(),
        lore_meta: LorebookMeta::default(),
        macros: Vec::new(),
        source_fields: object_keys(data),
        warnings: Vec::new(),
    };

    match spec.as_str() {
        SPEC_V2 | SPEC_V3 => {}
        "" => world.push_warning("角色卡没有 spec 字段（可能是 V1 卡），已按兼容字段导入。"),
        other => world.push_warning(format!("未识别的角色卡 spec：{other}，已按兼容字段导入。")),
    }
    if !character.system_prompt.trim().is_empty()
        || !character.post_history_instructions.trim().is_empty()
    {
        world.push_warning(
            "system_prompt / post_history_instructions 未自动当作世界规则；只有其中与文风相关的部分才考虑导入，需要用户确认。",
        );
    }
    if !character.group_only_greetings.is_empty() {
        world.push_warning(format!(
            "卡含 {} 条 group_only_greetings（群聊专用开场）；v1 群聊卡推迟，暂不导入。",
            character.group_only_greetings.len()
        ));
    }
    if !character.assets.is_empty() {
        world.push_warning(format!(
            "卡含 {} 个 assets（立绘 / 背景等）；v1 不生成立绘，仅保留来源信息。",
            character.assets.len()
        ));
    }
    if character.nickname.is_some() {
        world.push_warning("卡定义了 nickname：{{char}} 应替换为 nickname 而不是 name。");
    }

    if let Some(book) = data.get("character_book").filter(|value| value.is_object()) {
        let entries = book.get("entries").unwrap_or(book);
        world.lore = lorebook::parse_entries(entries);
        world.lore_meta = lorebook::parse_meta(book);
        if world.lore.is_empty() {
            world.push_warning("character_book 存在，但没有解析出可用条目（可能全部缺少 content）。");
        }
    }
    let sustainability = lorebook::sustainability_warnings(&world.lore);
    for warning in sustainability {
        world.push_warning(warning);
    }
    Ok(world)
}

/// 独立世界书文档（`lorebook_v3` 或酒馆运行时导出）→ 世界。
pub fn parse_lorebook_document(root: &Value, source_format: &str) -> Result<ImportedWorld, String> {
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(root);
    let entries = data
        .get("entries")
        .or_else(|| data.get("world_info"))
        .unwrap_or(data);
    let lore = lorebook::parse_entries(entries);
    if lore.is_empty() {
        return Err("世界书没有可识别的 entries".into());
    }
    let name = text(data, "name");
    let mut world = ImportedWorld {
        name: if name.trim().is_empty() { "导入的世界书".into() } else { name },
        genre: "待整理".into(),
        summary: format!("导入 {} 条世界书设定", lore.len()),
        source_format: source_format.to_owned(),
        source_kind: "json".into(),
        source_file: None,
        spec_version: optional(text(root, "spec_version")),
        characters: Vec::new(),
        lore_meta: lorebook::parse_meta(data),
        lore,
        macros: Vec::new(),
        source_fields: object_keys(data),
        warnings: Vec::new(),
    };
    let sustainability = lorebook::sustainability_warnings(&world.lore);
    for warning in sustainability {
        world.push_warning(warning);
    }
    Ok(world)
}

fn parse_assets(value: Option<&Value>) -> Vec<ImportedAsset> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.is_object())
                .map(|item| ImportedAsset {
                    asset_type: text(item, "type"),
                    uri: text(item, "uri"),
                    name: text(item, "name"),
                    ext: text(item, "ext"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn optional(text: String) -> Option<String> {
    (!text.trim().is_empty()).then_some(text)
}
