//! 酒馆角色卡 / 世界书导入。
//!
//! 入口：
//! - [`parse_json`]：从 JSON 文本导入；
//! - [`parse_bytes`]：从文件字节导入，自动识别 PNG（读 `chara` / `ccv3` 文本块）或 UTF-8 JSON。
//!
//! 全流程确定性、无网络、不执行卡中脚本。字段映射见 `docs/13-酒馆兼容.md`。
//! 语义抽取（主体 / 事实 / 故事线）不在这里做，留给后续 T-parse。

mod card;
mod lorebook;
mod model;
mod png;
mod value;

pub use model::{
    ImportedAsset, ImportedCharacter, ImportedLore, ImportedWorld, LoreLogic, LoreRole, LoreSection,
    LorebookMeta,
};
pub use png::decode_base64;

use serde_json::Value;

/// 从 JSON 文本导入角色卡或世界书。
pub fn parse_json(input: &str) -> Result<ImportedWorld, String> {
    let root: Value = serde_json::from_str(input).map_err(|e| format!("JSON 解析失败：{e}"))?;
    parse_value(&root)
}

/// 从已解析的 JSON 值导入。按 `spec` 分派；没有 `spec` 时按结构判断。
pub fn parse_value(root: &Value) -> Result<ImportedWorld, String> {
    let mut world = match card::spec_of(root) {
        Some("chara_card_v2") | Some("chara_card_v3") => card::parse_character_card(root)?,
        Some("lorebook_v3") => card::parse_lorebook_document(root, card::SPEC_LOREBOOK_V3)?,
        _ => {
            if card::is_lorebook(root) {
                card::parse_lorebook_document(root, card::FORMAT_WORLD_INFO)?
            } else {
                card::parse_character_card(root)?
            }
        }
    };
    collect_macros(&mut world);
    Ok(world)
}

/// 从文件字节导入。PNG 按 `ccv3` → `chara` 优先级取内嵌 JSON。
pub fn parse_bytes(bytes: &[u8], file_name: Option<&str>) -> Result<ImportedWorld, String> {
    let mut world = if png::looks_like_png(bytes) {
        let (json, chunk) = png::card_json_from_png(bytes)?;
        let mut world = parse_json(&json)?;
        world.source_kind = format!("png:{chunk}");
        world
    } else {
        let raw = std::str::from_utf8(bytes).map_err(|e| format!("文件不是 UTF-8 文本：{e}"))?;
        parse_json(raw)?
    };
    // 文本 JSON 走完 parse_json 后仍要补宏收集（PNG 分支已在内层做过一次，重复无害）。
    collect_macros(&mut world);
    if let Some(name) = file_name.map(str::trim).filter(|name| !name.is_empty()) {
        world.source_file = Some(name.to_owned());
    }
    Ok(world)
}

/// 取出文件里的**原始 JSON 文本**：PNG 卡是内嵌的那段，文本文件就是文本本身。
///
/// 世界库把它当**来源附件**留在库里（docs/10 §7 第 5 步），这样日后重导不必再找原文件。
/// 它是一次独立的扫描，与 [`parse_bytes`] 各扫一遍——PNG 块是线性扫的，
/// 相对于「把 `parse_bytes` 的返回值改成元组、所有调用点跟着改」这点开销不算什么。
pub fn raw_json(bytes: &[u8]) -> Result<String, String> {
    if png::looks_like_png(bytes) {
        png::card_json_from_png(bytes).map(|(json, _chunk)| json)
    } else {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|e| format!("文件不是 UTF-8 文本：{e}"))
    }
}

/// 卡/世界书中出现的宏标记。按 D14，`{{user}}` 会被转成由世界推演的主角。
fn collect_macros(world: &mut ImportedWorld) {
    let mut haystack = String::new();
    for character in &world.characters {
        for field in [
            &character.description,
            &character.personality,
            &character.scenario,
            &character.first_message,
            &character.example_messages,
        ] {
            haystack.push_str(field);
            haystack.push('\n');
        }
    }
    for entry in &world.lore {
        haystack.push_str(&entry.content);
        haystack.push('\n');
    }
    const MARKERS: [&str; 9] = [
        "{{user}}",
        "{{char}}",
        "<char>",
        "<bot>",
        "{{random:",
        "{{pick:",
        "{{roll:",
        "{{//",
        "{{outlet::",
    ];
    let found: Vec<String> = MARKERS
        .iter()
        .filter(|marker| haystack.contains(**marker))
        .map(|marker| (*marker).to_owned())
        .collect();
    if found.iter().any(|marker| marker == "{{user}}") {
        world.push_warning(
            "卡中使用了 {{user}}：按 D14，它会被转成由世界推演的主角——用户不再扮演主角，而是改写主角的命运。",
        );
    }
    world.macros = found;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_card_and_character_book() {
        let world = parse_json(r#"{"spec":"chara_card_v2","data":{"name":"裴聿","description":"县令","personality":"克制","scenario":"长安","first_mes":"雨夜","character_book":{"entries":{"1":{"comment":"城规","content":"夜禁","key":["长安"],"constant":true}}}}}"#).unwrap();
        assert_eq!(world.source_format, "chara_card_v2");
        assert_eq!(world.characters[0].name, "裴聿");
        assert_eq!(world.lore[0].keys, vec!["长安"]);
        assert!(world.lore[0].constant);
    }

    #[test]
    fn parses_world_info_object_entries_and_defaults_disabled() {
        let world = parse_json(r#"{"name":"雨城","entries":{"42":{"content":"河水上涨","key":"暴雨,河水","keysecondary":["夏季"],"disable":true,"order":220}}}"#).unwrap();
        assert_eq!(world.name, "雨城");
        assert_eq!(world.lore[0].keys, vec!["暴雨", "河水"]);
        assert!(!world.lore[0].enabled);
        assert_eq!(world.lore[0].order, 220);
        assert_eq!(world.source_format, "world_info_json");
    }

    #[test]
    fn parses_standalone_lorebook_v3_document() {
        let world = parse_json(
            r#"{"spec":"lorebook_v3","data":{"name":"雨城","scan_depth":3,"entries":[{"keys":["河"],"content":"上涨","enabled":true,"insertion_order":10}]}}"#,
        )
        .unwrap();
        assert_eq!(world.source_format, "lorebook_v3");
        assert_eq!(world.lore_meta.scan_depth, Some(3.0));
        assert_eq!(world.lore.len(), 1);
    }

    #[test]
    fn parses_v3_card_new_fields_and_flags_deferred_parts() {
        let world = parse_json(
            r#"{"spec":"chara_card_v3","spec_version":"3.0","data":{"name":"裴聿","nickname":"阎罗笔","first_mes":"雨夜","group_only_greetings":["群聊开场"],"assets":[{"type":"icon","uri":"ccdefault:","name":"main","ext":"png"}],"creation_date":1700000000,"creator_notes":"备注"}}"#,
        )
        .unwrap();
        assert_eq!(world.source_format, "chara_card_v3");
        assert_eq!(world.spec_version.as_deref(), Some("3.0"));
        assert_eq!(world.characters[0].nickname.as_deref(), Some("阎罗笔"));
        assert_eq!(world.characters[0].creation_date, Some(1700000000));
        let warnings = world.warnings.join("\n");
        assert!(warnings.contains("group_only_greetings"), "{warnings}");
        assert!(warnings.contains("assets"), "{warnings}");
        assert!(warnings.contains("nickname"), "{warnings}");
    }

    #[test]
    fn reports_v1_card_without_spec() {
        let world = parse_json(r#"{"name":"旧卡","description":"描述","first_mes":"开场"}"#).unwrap();
        assert_eq!(world.source_format, "chara_card_v1");
        assert!(world.warnings.iter().any(|w| w.contains("V1")));
    }

    #[test]
    fn rejects_card_without_name() {
        assert!(parse_json(r#"{"spec":"chara_card_v2","data":{"description":"没有名字"}}"#).is_err());
        assert!(parse_json("{ not json }").is_err());
        assert!(parse_json("{}").is_err());
    }

    #[test]
    fn imports_png_card_end_to_end() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;
        let json = r#"{"spec":"chara_card_v3","data":{"name":"猫娘","first_mes":"喵"}}"#;
        let png = crate::importer::png::tests::png_with_text_chunks(&[(
            "tEXt",
            "ccv3",
            &STANDARD.encode(json.as_bytes()),
        )]);
        let world = parse_bytes(&png, Some("cat.png")).unwrap();
        assert_eq!(world.source_kind, "png:ccv3");
        assert_eq!(world.source_file.as_deref(), Some("cat.png"));
        assert_eq!(world.characters[0].name, "猫娘");
    }

    #[test]
    fn records_user_macro_for_d14() {
        let world = parse_json(
            r#"{"spec":"chara_card_v2","data":{"name":"A","description":"你是{{user}}，{{char}}在等你"}}"#,
        )
        .unwrap();
        assert!(world.macros.contains(&"{{user}}".to_owned()));
        assert!(world.warnings.iter().any(|w| w.contains("D14")));
    }

    fn fixture(name: &str) -> Option<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        std::fs::read_to_string(path).ok()
    }

    /// 真实公开样本回归（CC BY 4.0，来源见 tests/fixtures/README.md）。
    /// 这两张卡是「V2 卡外壳 + CCv3 格式的 character_book」，正是要兼容的真实形态。
    #[test]
    fn imports_public_sample_cards() {
        let cases = [
            ("peiyu.v2.json", "裴聿", 5usize),
            ("nie-xiaoqian.v2.json", "聂小倩", 12usize),
        ];
        for (file, expected_name, min_lore) in cases {
            let Some(raw) = fixture(file) else {
                eprintln!("跳过 {file}：样本不存在（见 tests/fixtures/README.md）");
                continue;
            };
            let world = parse_json(&raw).unwrap_or_else(|e| panic!("{file} 解析失败：{e}"));
            assert_eq!(world.source_format, "chara_card_v2", "{file}");
            assert_eq!(world.name, expected_name, "{file}");
            assert_eq!(world.characters[0].name, expected_name, "{file}");
            assert!(world.lore.len() >= min_lore, "{file} 只解析出 {} 条", world.lore.len());
            assert!(
                world.lore.iter().all(|entry| entry.source_dialect == "ccv3"),
                "{file} 的 character_book 应识别为 ccv3 方言"
            );
            assert!(
                world.lore.iter().any(|entry| entry.constant),
                "{file} 应有常驻条目"
            );
            assert!(
                world.lore.iter().all(|entry| entry.section == super::model::LoreSection::Character),
                "{file} 的 position 全部为 before_char，应归角色段"
            );
            assert!(
                world.warnings.iter().any(|w| w.contains("system_prompt")),
                "{file} 应提示 system_prompt 待人工确认"
            );
            assert!(
                world.source_fields.iter().any(|field| field == "character_book"),
                "{file} 应保留 character_book 来源字段"
            );
            assert!(
                world.macros.contains(&"{{user}}".to_owned()),
                "{file} 含 {{user}}，应记录并按 D14 提示"
            );
        }
    }
}
