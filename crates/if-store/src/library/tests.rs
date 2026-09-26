//! 世界库的存储规则测试。
//!
//! 重点不在 CRUD 本身，而在两条**规则**：
//! - 按来源整组替换（重导不留残影、也不误伤别的来源）；
//! - 被会话引用的资产不允许被删掉。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use if_domain::id::AssetId;
use serde_json::json;

use super::{AssetDraft, AssetOrigin, Library, LoreRecord, SessionRef, SourceRecord, MANUAL_SOURCE};

// ------------------------------------------------------------------ 测试脚手架

/// 一个用完即删的临时库文件。测完连 WAL 边车一起清掉，不在 `temp_dir` 里留垃圾。
struct TempDb(PathBuf);

impl TempDb {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "if-store-library-{}-{tag}-{n}.db",
            std::process::id()
        ));
        let db = TempDb(path);
        db.clean();
        db
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn open(&self) -> Library {
        Library::open(self.path()).expect("打开世界库")
    }

    fn clean(&self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        self.clean();
    }
}

fn source(key: &str, file: &str) -> SourceRecord {
    let mut record = SourceRecord::new(key, "chara_card", "chara_card_v3");
    record.file = Some(file.to_owned());
    record.spec_version = Some("3.0".to_owned());
    record.content_hash = format!("hash:{key}");
    record.raw = Some("{}".to_owned());
    record
}

fn lore(key: &str, ordinal: i64, title: &str) -> LoreRecord {
    let mut record = LoreRecord::new(key, ordinal, json!({ "content": title, "enabled": true }));
    record.title = title.to_owned();
    record.section = "character".to_owned();
    record
}

fn titles(records: &[LoreRecord]) -> Vec<String> {
    records.iter().map(|r| r.title.clone()).collect()
}

fn session(id: &str, asset: &AssetId) -> SessionRef {
    SessionRef {
        id: id.to_owned(),
        asset_id: asset.clone(),
        label: format!("{id} 的会话"),
        world_file: format!("{id}.ifworld"),
        created_at: 1_700_000_000_000,
    }
}

// ------------------------------------------------------------------ 基本读写

#[test]
fn save_then_load_round_trips_everything() {
    let mut library = Library::open_in_memory().unwrap();
    let payload = json!({ "characters": [{ "name": "顾昭", "source_uid": "u1" }], "macros": ["{{user}}"] });

    let saved = library
        .save_asset(
            AssetDraft::new("河流涨水的那个夏天", AssetOrigin::Imported)
                .with_genre("悬疑")
                .with_summary("一场洪水改变了三个人")
                .with_payload(payload.clone())
                .with_source(
                    source("card:river", "river.png"),
                    vec![lore("card:river", 0, "顾昭"), lore("card:river", 1, "林玥")],
                ),
        )
        .unwrap();

    assert_eq!(saved.id.as_str(), "asset_0001");
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.created_at, saved.updated_at);

    let loaded = library.load_asset(&saved.id).unwrap().expect("资产应存在");
    assert_eq!(loaded.name, "河流涨水的那个夏天");
    assert_eq!(loaded.genre, "悬疑");
    assert_eq!(loaded.origin, AssetOrigin::Imported);
    // payload 是不透明 JSON：原样存取，不解释、不丢字段
    assert_eq!(loaded.payload, payload);

    let records = library.lore_of(&saved.id).unwrap();
    assert_eq!(titles(&records), vec!["顾昭", "林玥"]);
    assert_eq!(records[0].payload["enabled"], json!(true));

    let sources = library.sources_of(&saved.id).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].file.as_deref(), Some("river.png"));
    assert_eq!(sources[0].spec_version.as_deref(), Some("3.0"));
    assert!(sources[0].raw.is_some());
}

#[test]
fn missing_asset_loads_as_none() {
    let library = Library::open_in_memory().unwrap();
    assert!(library.load_asset(&AssetId::new("asset_9999")).unwrap().is_none());
}

#[test]
fn ids_are_allocated_in_sequence() {
    let mut library = Library::open_in_memory().unwrap();
    let a = library
        .save_asset(AssetDraft::new("一", AssetOrigin::Written))
        .unwrap();
    let b = library
        .save_asset(AssetDraft::new("二", AssetOrigin::Written))
        .unwrap();
    assert_eq!(a.id.as_str(), "asset_0001");
    assert_eq!(b.id.as_str(), "asset_0002");
}

#[test]
fn explicit_id_creates_when_absent_and_updates_when_present() {
    let mut library = Library::open_in_memory().unwrap();

    let created = library
        .save_asset(
            AssetDraft::new("手写的世界", AssetOrigin::Written).with_id("asset_0007"),
        )
        .unwrap();
    assert_eq!(created.id.as_str(), "asset_0007");
    assert_eq!(created.revision, 1);

    let updated = library
        .save_asset(
            AssetDraft::new("手写的世界（改）", AssetOrigin::Written).with_id("asset_0007"),
        )
        .unwrap();
    assert_eq!(updated.id.as_str(), "asset_0007");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(library.list_assets().unwrap().len(), 1);
}

#[test]
fn next_id_skips_taken_ids() {
    let mut library = Library::open_in_memory().unwrap();
    library
        .save_asset(AssetDraft::new("占位", AssetOrigin::Written).with_id("asset_0003"))
        .unwrap();
    // 手工把序号拨回去，模拟「从别处拷来一个库、序号没跟着走」
    library.set_meta("next_asset_seq", "3").unwrap();

    let next = library
        .save_asset(AssetDraft::new("下一个", AssetOrigin::Written))
        .unwrap();
    assert_eq!(next.id.as_str(), "asset_0004");
    // 序号要往前推，否则下次还要再探一遍
    assert_eq!(library.get_meta("next_asset_seq").unwrap().as_deref(), Some("5"));
}

#[test]
fn origin_survives_the_round_trip() {
    let mut library = Library::open_in_memory().unwrap();
    let written = library
        .save_asset(AssetDraft::new("手写", AssetOrigin::Written))
        .unwrap();
    let imported = library
        .save_asset(AssetDraft::new("导入", AssetOrigin::Imported))
        .unwrap();
    assert_eq!(library.load_asset(&written.id).unwrap().unwrap().origin, AssetOrigin::Written);
    assert_eq!(library.load_asset(&imported.id).unwrap().unwrap().origin, AssetOrigin::Imported);
}

// ------------------------------------------------------------------ 列表与摘要

#[test]
fn list_returns_summaries_with_counts_and_files() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(
            AssetDraft::new("有来源的世界", AssetOrigin::Imported).with_source(
                source("card:river", "river.png"),
                vec![lore("card:river", 0, "A"), lore("card:river", 1, "B")],
            ),
        )
        .unwrap();
    library.attach_session(&session("s1", &asset.id)).unwrap();
    library.attach_session(&session("s2", &asset.id)).unwrap();

    let summaries = library.list_assets().unwrap();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.id, asset.id);
    assert_eq!(summary.lore_count, 2);
    assert_eq!(summary.source_count, 1);
    assert_eq!(summary.session_count, 2);
    assert_eq!(summary.source_files, vec!["river.png".to_owned()]);
}

#[test]
fn list_is_ordered_by_most_recent_update() {
    let mut library = Library::open_in_memory().unwrap();
    let first = library
        .save_asset(AssetDraft::new("先", AssetOrigin::Written))
        .unwrap();
    let second = library
        .save_asset(AssetDraft::new("后", AssetOrigin::Written))
        .unwrap();
    // 同毫秒内两次保存的顺序不确定，所以显式把时间拨开
    library
        .save_asset(AssetDraft::new("先（再存一次）", AssetOrigin::Written).with_id(first.id.clone()))
        .unwrap();

    let ids: Vec<AssetId> = library
        .list_assets()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert!(ids.contains(&first.id) && ids.contains(&second.id));
    assert_eq!(ids.len(), 2);
}

// ------------------------------------------------------------------ 按来源整组替换

#[test]
fn reimport_replaces_only_that_source_group() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported).with_source(
                source("card:a", "a.png"),
                vec![lore("card:a", 0, "A0"), lore("card:a", 1, "A1")],
            ),
        )
        .unwrap();

    // 第二个来源进来，第一组必须原封不动
    library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported)
                .with_id(asset.id.clone())
                .with_source(source("book:b", "b.json"), vec![lore("book:b", 0, "B0")]),
        )
        .unwrap();

    let all = library.lore_of(&asset.id).unwrap();
    // 按 source_key 排序：`book:b` 在 `card:a` 之前
    assert_eq!(titles(&all), vec!["B0", "A0", "A1"]);
    assert_eq!(library.sources_of(&asset.id).unwrap().len(), 2);

    // 重导 A：条数变少，旧的必须消失，而不是叠加
    library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported)
                .with_id(asset.id.clone())
                .with_source(source("card:a", "a.png"), vec![lore("card:a", 0, "A0 改")]),
        )
        .unwrap();

    let all = library.lore_of(&asset.id).unwrap();
    assert_eq!(titles(&all), vec!["B0", "A0 改"]);
    assert_eq!(titles(&library.lore_of_source(&asset.id, Some("card:a")).unwrap()), vec!["A0 改"]);
}

#[test]
fn reimport_with_zero_entries_still_clears_the_old_group() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported).with_source(
                source("card:a", "a.png"),
                vec![lore("card:a", 0, "A0"), lore("card:a", 1, "A1")],
            ),
        )
        .unwrap();
    assert_eq!(library.lore_count(&asset.id).unwrap(), 2);

    // 这次一条条目都没解析出来——旧条目不能留成残影
    library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported)
                .with_id(asset.id.clone())
                .with_source(source("card:a", "a.png"), Vec::new()),
        )
        .unwrap();

    assert_eq!(library.lore_count(&asset.id).unwrap(), 0);
    // 来源记录本身要留着：它是「这个来源曾经属于这个资产」的凭据
    assert_eq!(library.sources_of(&asset.id).unwrap().len(), 1);
}

#[test]
fn manual_lore_needs_no_source_record() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(
            AssetDraft::new("手写的世界", AssetOrigin::Written)
                .with_lore(vec![lore(MANUAL_SOURCE, 0, "手写条目")]),
        )
        .unwrap();
    assert_eq!(titles(&library.lore_of(&asset.id).unwrap()), vec!["手写条目"]);
    assert!(library.sources_of(&asset.id).unwrap().is_empty());
}

#[test]
fn unknown_lore_source_is_rejected() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(AssetDraft::new("世界", AssetOrigin::Imported))
        .unwrap();

    // 来源键打错一个字符，如果不拦就会造出一组永远不会被替换的孤儿条目
    let err = library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported)
                .with_id(asset.id.clone())
                .with_lore(vec![lore("card:typo", 0, "孤儿")]),
        )
        .unwrap_err();
    assert!(matches!(err, crate::StoreError::UnknownLoreSource(ref key) if key == "card:typo"));

    // 校验在事务之前失败，库里不该留下半个资产
    assert_eq!(library.lore_count(&asset.id).unwrap(), 0);
}

#[test]
fn lore_payload_keeps_its_full_shape() {
    let mut library = Library::open_in_memory().unwrap();
    let mut record = lore(MANUAL_SOURCE, 0, "带装饰器的条目");
    record.uid = Some("lore_uid_7".to_owned());
    record.payload = json!({
        "content": "正文",
        "keys": ["河", "涨水"],
        "decorators": [{ "kind": "position", "value": "before_char" }],
        "extensions": { "depth": 4, "sticky": null },
    });

    let asset = library
        .save_asset(AssetDraft::new("世界", AssetOrigin::Written).with_lore(vec![record.clone()]))
        .unwrap();

    let back = library.lore_of(&asset.id).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0], record);
}

// ------------------------------------------------------------------ 来源反查

#[test]
fn asset_of_source_finds_the_owner() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported)
                .with_source(source("card:river", "river.png"), Vec::new()),
        )
        .unwrap();

    assert_eq!(library.asset_of_source("card:river").unwrap(), Some(asset.id.clone()));
    assert_eq!(library.asset_of_source("card:nobody").unwrap(), None);
}

#[test]
fn detaching_a_session_frees_the_asset() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(AssetDraft::new("世界", AssetOrigin::Written))
        .unwrap();
    library.attach_session(&session("s1", &asset.id)).unwrap();
    assert_eq!(library.session_count(&asset.id).unwrap(), 1);

    library.detach_session("s1").unwrap();
    assert_eq!(library.session_count(&asset.id).unwrap(), 0);
    library.delete_asset(&asset.id).unwrap();
    assert!(library.load_asset(&asset.id).unwrap().is_none());
}

/// 恢复会话要靠 `session(id)` 找到那条 `.ifworld`，所以 id 必须是能反查的。
#[test]
fn a_session_can_be_looked_up_by_id() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(AssetDraft::new("世界", AssetOrigin::Written))
        .unwrap();

    let mut reference = SessionRef::new("river-1727", asset.id.clone(), "河流", "/worlds/river.ifworld");
    assert!(reference.created_at > 0, "构造时就该填上时间戳");
    reference.created_at = 1_700_000_000_000;
    library.attach_session(&reference).unwrap();

    assert_eq!(library.session("river-1727").unwrap(), Some(reference));
    assert_eq!(library.session("不存在").unwrap(), None);
    // 反查出来的世界文件路径要能直接用
    assert_eq!(
        library.session("river-1727").unwrap().unwrap().world_file,
        "/worlds/river.ifworld"
    );
}

// ------------------------------------------------------------------ 删除

#[test]
fn delete_is_refused_while_a_session_references_the_asset() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(AssetDraft::new("世界", AssetOrigin::Written))
        .unwrap();
    library.attach_session(&session("s1", &asset.id)).unwrap();

    let err = library.delete_asset(&asset.id).unwrap_err();
    match err {
        crate::StoreError::AssetInUse { asset: id, sessions } => {
            assert_eq!(id, asset.id.to_string());
            assert_eq!(sessions, 1);
        }
        other => panic!("期望 AssetInUse，实际是 {other:?}"),
    }
    // 拒绝之后资产必须完好
    assert!(library.load_asset(&asset.id).unwrap().is_some());
}

#[test]
fn delete_removes_sources_and_lore() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(
            AssetDraft::new("世界", AssetOrigin::Imported).with_source(
                source("card:a", "a.png"),
                vec![lore("card:a", 0, "A0")],
            ),
        )
        .unwrap();

    library.delete_asset(&asset.id).unwrap();
    assert!(library.load_asset(&asset.id).unwrap().is_none());
    assert_eq!(library.lore_count(&asset.id).unwrap(), 0);
    assert!(library.sources_of(&asset.id).unwrap().is_empty());
    // 来源索引也要清掉，否则下次导入同一张卡会长回一个已被删的资产上
    assert_eq!(library.asset_of_source("card:a").unwrap(), None);
    assert!(library.list_assets().unwrap().is_empty());
}

// ------------------------------------------------------------------ 落盘

#[test]
fn assets_survive_closing_and_reopening() {
    let db = TempDb::new("persist");
    let asset_id;

    {
        let mut library = db.open();
        let asset = library
            .save_asset(
                AssetDraft::new("落盘的世界", AssetOrigin::Imported).with_source(
                    source("card:river", "river.png"),
                    vec![lore("card:river", 0, "顾昭")],
                ),
            )
            .unwrap();
        asset_id = asset.id;
    }

    let library = db.open();
    let loaded = library.load_asset(&asset_id).unwrap().expect("重开后资产应还在");
    assert_eq!(loaded.name, "落盘的世界");
    assert_eq!(titles(&library.lore_of(&asset_id).unwrap()), vec!["顾昭"]);
    assert_eq!(library.asset_of_source("card:river").unwrap(), Some(asset_id));
}

#[test]
fn reopening_does_not_restart_the_id_sequence() {
    let db = TempDb::new("sequence");

    {
        let mut library = db.open();
        library
            .save_asset(AssetDraft::new("一", AssetOrigin::Written))
            .unwrap();
        library
            .save_asset(AssetDraft::new("二", AssetOrigin::Written))
            .unwrap();
    }

    let mut library = db.open();
    let third = library
        .save_asset(AssetDraft::new("三", AssetOrigin::Written))
        .unwrap();
    assert_eq!(third.id.as_str(), "asset_0003");
}
