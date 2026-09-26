//! 导入结果 → 世界库的映射测试。
//!
//! 关注三件事：来源身份键是否**稳**（重导会替换而不是追加）、条目映射是否忠实、
//! 以及写进去的东西重启之后还在。

use super::*;
use crate::importer;

const CARD_V1: &str = r#"{
  "spec": "chara_card_v2",
  "data": {
    "name": "裴聿",
    "description": "长安县令，克制。",
    "character_version": "1.0",
    "creator_notes": "写给雨夜长安",
    "character_book": {
      "entries": {
        "0": { "id": 7, "comment": "城规", "content": "夜禁之后不得出坊。", "key": ["长安"], "constant": true },
        "1": { "comment": "河堤", "content": "河堤年久失修。", "key": ["河水"], "position": "after_char" }
      }
    }
  }
}"#;

/// 同一张卡，改了正文与卡版本——这正是「重导要替换」的场景。
const CARD_V2: &str = r#"{
  "spec": "chara_card_v2",
  "data": {
    "name": "裴聿",
    "description": "长安县令，克制。第二版加了一段。",
    "character_version": "2.0",
    "creator_notes": "写给雨夜长安",
    "character_book": {
      "entries": {
        "0": { "id": 7, "comment": "城规（改）", "content": "夜禁之后不得出坊，违者杖二十。", "key": ["长安"], "constant": true }
      }
    }
  }
}"#;

const BOOK: &str = r#"{
  "spec": "lorebook_v3",
  "data": {
    "name": "裴聿",
    "description": "同一个名字的世界书",
    "entries": [{ "keys": ["河"], "content": "河水上涨", "enabled": true, "insertion_order": 10 }]
  }
}"#;

fn card(raw: &str, file: Option<&str>) -> ImportedWorld {
    let mut world = importer::parse_json(raw).unwrap();
    world.source_file = file.map(str::to_owned);
    world
}

// ------------------------------------------------------------------ 来源身份键

#[test]
fn source_key_survives_content_and_version_changes() {
    let first = source_key_of(&card(CARD_V1, Some("peiyu.png")));
    let second = source_key_of(&card(CARD_V2, Some("peiyu-v2.png")));

    assert_eq!(first, "card:裴聿");
    // 卡版本升了、文件名换了、正文改了——仍然是同一个来源
    assert_eq!(first, second);
    // 但内容哈希必须能看出区别，否则「是不是同一份」就没法回答
    assert_ne!(content_hash(CARD_V1), content_hash(CARD_V2));
}

#[test]
fn source_key_separates_cards_from_lorebooks() {
    let card_key = source_key_of(&card(CARD_V1, None));
    let book_key = source_key_of(&importer::parse_json(BOOK).unwrap());
    assert_eq!(book_key, "book:裴聿");
    assert_ne!(card_key, book_key);
}

#[test]
fn source_key_falls_back_to_file_stem() {
    let mut world = card(CARD_V1, Some("peiyu-card.png"));
    world.name = "   ".to_owned();
    assert_eq!(source_key_of(&world), "card:peiyu-card");
}

// ------------------------------------------------------------------ 条目映射

#[test]
fn draft_maps_lore_faithfully() {
    let world = card(CARD_V1, Some("peiyu.png"));
    let draft = draft_from_import(&world, CARD_V1, None).unwrap();

    assert_eq!(draft.name, "裴聿");
    assert_eq!(draft.origin, AssetOrigin::Imported);
    assert_eq!(draft.id, None);
    // 卡里没有 genre 字段：留空，不编造
    assert!(draft.genre.is_empty());
    // 摘要退到创作者备注
    assert_eq!(draft.summary, "写给雨夜长安");

    let source = draft.source.as_ref().expect("导入必须带来源记录");
    assert_eq!(source.source_key, "card:裴聿");
    assert_eq!(source.kind, "chara_card");
    assert_eq!(source.format, "chara_card_v2");
    assert_eq!(source.file.as_deref(), Some("peiyu.png"));
    assert_eq!(source.version.as_deref(), Some("1.0"));
    assert_eq!(source.raw.as_deref(), Some(CARD_V1));
    // 来源记录里不该有内容哈希之外的东西冒充身份
    assert!(source.content_hash.starts_with("sha256:"));

    assert_eq!(draft.lore.len(), 2);
    assert_eq!(draft.lore[0].title, "城规");
    // uid 只断言「原样带过来」：条目 uid 怎么从来源推出来是导入层的事
    // （`lorebook::parse_entry` 依次看 `uid` → `id` → map 键），这里不替它做主。
    assert_eq!(draft.lore[0].uid, world.lore[0].source_uid);
    assert!(draft.lore[0].uid.is_some());
    // 归段用的是导入层的结果：没写 position 归世界段，`after_char` 归角色段
    assert_eq!(draft.lore[0].section, "world");
    assert_eq!(draft.lore[1].section, "character");
    assert_eq!(draft.lore[0].ordinal, 0);
    assert_eq!(draft.lore[1].ordinal, 1);
    assert_eq!(draft.lore[0].payload["constant"], serde_json::json!(true));
    // 正文原样保留在条目 payload 里，一个字符都不改
    assert_eq!(draft.lore[0].payload["content"], serde_json::json!("夜禁之后不得出坊。"));
}

#[test]
fn payload_round_trips_as_imported_world() {
    let world = card(CARD_V1, Some("peiyu.png"));
    let draft = draft_from_import(&world, CARD_V1, None).unwrap();

    let back: ImportedWorld = serde_json::from_value(draft.payload).unwrap();
    assert_eq!(back, world);
}

#[test]
fn summary_falls_back_and_never_invents_text() {
    let mut world = card(CARD_V1, None);
    world.summary = "  卡自带摘要  ".to_owned();
    assert_eq!(summary_of(&world), "卡自带摘要");

    world.summary.clear();
    world.lore_meta.description = "世界书描述".to_owned();
    assert_eq!(summary_of(&world), "世界书描述");

    world.lore_meta.description.clear();
    world.characters[0].creator_notes.clear();
    world.characters[0].description = "长安县令，克制。".to_owned();
    assert_eq!(summary_of(&world), "长安县令，克制。");

    world.characters[0].description.clear();
    assert!(summary_of(&world).is_empty());
}

#[test]
fn summary_clips_on_char_boundaries() {
    let long = "雨".repeat(SUMMARY_LIMIT + 50);
    let clipped = clip(&long, SUMMARY_LIMIT);
    assert_eq!(clipped.chars().count(), SUMMARY_LIMIT + 1);
    assert!(clipped.ends_with('…'));
}

#[test]
fn cards_without_lore_still_import() {
    let raw = r#"{"spec":"chara_card_v2","data":{"name":"独白卡","description":"没有世界书"}}"#;
    let world = card(raw, None);
    let draft = draft_from_import(&world, raw, None).unwrap();
    assert!(draft.lore.is_empty());
    assert_eq!(draft.source.unwrap().source_key, "card:独白卡");
}

// ------------------------------------------------------------------ 手写资产

#[test]
fn written_draft_has_no_source_and_no_lore() {
    let draft = draft_written(" 新世界 ", "悬疑", " 一句话简介 ", None);
    assert_eq!(draft.name, "新世界");
    assert_eq!(draft.genre, "悬疑");
    assert_eq!(draft.summary, "一句话简介");
    assert_eq!(draft.origin, AssetOrigin::Written);
    assert!(draft.source.is_none());
    assert!(draft.lore.is_empty());
    assert_eq!(draft.payload["source_kind"], serde_json::json!("manual"));
}

// ------------------------------------------------------------------ 落库

#[test]
fn import_persists_and_reimport_replaces_by_source() {
    let mut library = Library::open_in_memory().unwrap();

    let first = library
        .save_asset(draft_from_import(&card(CARD_V1, Some("peiyu.png")), CARD_V1, None).unwrap())
        .unwrap();
    assert_eq!(first.revision, 1);

    let detail = AssetDetail::load(&library, &first.id).unwrap();
    assert_eq!(detail.lore.len(), 2);
    assert_eq!(detail.sources.len(), 1);
    assert!(detail.sessions.is_empty());
    // payload 能重建世界视图
    let restored: ImportedWorld = serde_json::from_value(detail.asset.payload).unwrap();
    assert_eq!(restored.characters[0].name, "裴聿");

    // 重导同一张卡的新版本：落在同一个资产上，条目被替换而不是叠加
    let again = library
        .save_asset(draft_from_import(&card(CARD_V2, Some("peiyu.png")), CARD_V2, Some(first.id.clone())).unwrap())
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.revision, 2);

    let detail = AssetDetail::load(&library, &first.id).unwrap();
    assert_eq!(detail.lore.len(), 1);
    assert_eq!(detail.lore[0].title, "城规（改）");
    assert_eq!(detail.asset.summary, "写给雨夜长安");
    // content_hash 要跟着内容走
    assert_ne!(
        detail.sources[0].content_hash,
        content_hash(CARD_V1)
    );
}

#[test]
fn a_card_and_a_lorebook_with_the_same_name_coexist() {
    let mut library = Library::open_in_memory().unwrap();

    let card_asset = library
        .save_asset(draft_from_import(&card(CARD_V1, None), CARD_V1, None).unwrap())
        .unwrap();
    let book_world = importer::parse_json(BOOK).unwrap();
    let book_asset = library
        .save_asset(draft_from_import(&book_world, BOOK, None).unwrap())
        .unwrap();

    assert_ne!(card_asset.id, book_asset.id);
    assert_eq!(library.list_assets().unwrap().len(), 2);
}

#[test]
fn list_reflects_what_was_written() {
    let mut library = Library::open_in_memory().unwrap();

    library
        .save_asset(draft_from_import(&card(CARD_V1, Some("peiyu.png")), CARD_V1, None).unwrap())
        .unwrap();
    library
        .save_asset(draft_written("手写的", "悬疑", "", None))
        .unwrap();

    let assets = list(&library).unwrap();
    assert_eq!(assets.len(), 2);
    let imported = assets.iter().find(|a| a.origin == AssetOrigin::Imported).unwrap();
    assert_eq!(imported.lore_count, 2);
    assert_eq!(imported.source_count, 1);
    assert_eq!(imported.source_files, vec!["peiyu.png".to_owned()]);
    let written = assets.iter().find(|a| a.origin == AssetOrigin::Written).unwrap();
    assert_eq!(written.lore_count, 0);
    assert_eq!(written.session_count, 0);
}

#[test]
fn detail_reports_a_missing_asset_instead_of_panicking() {
    let library = Library::open_in_memory().unwrap();
    let err = AssetDetail::load(&library, &AssetId::new("asset_9999")).unwrap_err();
    assert!(err.contains("asset_9999"), "{err}");
}
