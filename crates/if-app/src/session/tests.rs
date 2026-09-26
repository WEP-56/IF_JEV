//! 会话创建 / 恢复的测试。
//!
//! 这里跑的是**真实文件与真实 SQLite**：会话的价值就在于「关掉再打开还在」，
//! 用内存库测等于把要验的那件事绕过去了。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use if_domain::id::AssetId;
use if_store::library::{AssetDraft, AssetOrigin, Library};

use super::{create, load, resume, sessions_of};

const CARD: &str = r#"{"spec":"chara_card_v2","data":{
    "name":"裴聿","description":"长安县令","scenario":"雨夜的长安","first_mes":"雨落在青石板上。",
    "character_book":{"entries":{
        "1":{"comment":"宵禁","content":"入夜后坊门落锁。","key":["长安"],"constant":true}
    }}}}"#;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("if-session-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn library_with_card() -> (Library, AssetId) {
    let mut library = Library::open_in_memory().unwrap();
    let world = crate::importer::parse_json(CARD).unwrap();
    let draft = crate::library::draft_from_import(&world, CARD, None).unwrap();
    let asset = library.save_asset(draft).unwrap();
    (library, asset.id)
}

#[test]
fn create_writes_the_world_and_describes_it() {
    let (library, id) = library_with_card();
    let dir = TempDir::new();

    let (_handle, view, reference) = create(load(&library, &id).unwrap(), dir.path(), None).unwrap();

    // 世界真的写出来了
    assert!(PathBuf::from(&reference.world_file).is_file());
    assert_eq!(reference.asset_id, id);
    // 会话 ID 就是世界文件名主干，从 ID 能反查出文件
    assert_eq!(
        reference.id,
        PathBuf::from(&reference.world_file)
            .file_stem()
            .unwrap()
            .to_string_lossy()
    );

    // world_created + 主体 + 情境 + 宵禁
    assert_eq!(view.snapshot.event_count, 4);
    assert_eq!(view.snapshot.projection.subjects.len(), 1);
    assert_eq!(view.snapshot.projection.lore.len(), 2);
    assert_eq!(view.asset_id, id.to_string());
    assert_eq!(view.asset_name, "裴聿");
    assert_eq!(view.opening.as_deref(), Some("雨落在青石板上。"));
    // 没有给名字时用资产名
    assert_eq!(view.snapshot.label, "裴聿");

    let report = view.seed.expect("新建会话要给出播种简报");
    assert_eq!(report.subjects, 1);
    assert_eq!(report.lore, 2);
}

#[test]
fn create_uses_a_given_label_for_the_file_and_the_world_line() {
    let (library, id) = library_with_card();
    let dir = TempDir::new();

    let (handle, view, reference) =
        create(load(&library, &id).unwrap(), dir.path(), Some("  第一次游玩  ".into())).unwrap();

    assert_eq!(view.snapshot.label, "第一次游玩", "两端空白要去掉");
    assert!(PathBuf::from(&reference.world_file)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("第一次游玩"));
    drop(handle);
}

#[test]
fn create_refuses_a_world_whose_name_is_blank() {
    let mut library = Library::open_in_memory().unwrap();
    // 手写资产允许名字为空，播种时就必须拦住——否则会建出一个没有名字的世界文件
    let asset = library
        .save_asset(
            AssetDraft::new("   ", AssetOrigin::Written)
                .with_payload(serde_json::json!({ "name": "   " })),
        )
        .unwrap();
    let dir = TempDir::new();

    let error = create(load(&library, &asset.id).unwrap(), dir.path(), None).unwrap_err();
    assert!(error.contains("不能为空"), "{error}");
}

#[test]
fn a_manual_world_sows_into_an_empty_world_and_says_so() {
    let mut library = Library::open_in_memory().unwrap();
    let asset = library
        .save_asset(crate::library::draft_written("手写的世界", "待撰写", "什么都没有。", None))
        .unwrap();
    let dir = TempDir::new();

    let (handle, view, _) = create(load(&library, &asset.id).unwrap(), dir.path(), None).unwrap();

    // 只有 world_created
    assert_eq!(view.snapshot.event_count, 1);
    assert!(view.snapshot.projection.subjects.is_empty());
    let report = view.seed.expect("手写世界也走过播种");
    assert_eq!(report.subjects, 0);
    assert!(
        report.notes.iter().any(|note| note.contains("没有角色")),
        "空世界要把「没有可模拟的主体」说出来：{:?}",
        report.notes
    );
    drop(handle);
}

#[test]
fn resume_reopens_the_same_world_from_the_library() {
    let (library, id) = library_with_card();
    let dir = TempDir::new();

    let (handle, created, reference) = create(load(&library, &id).unwrap(), dir.path(), None).unwrap();
    let session_id = reference.id.clone();
    library.attach_session(&reference).unwrap();
    drop(handle);

    let (reopened, view) = resume(&library, &session_id).unwrap();
    assert_eq!(view.snapshot.event_count, created.snapshot.event_count);
    assert_eq!(view.snapshot.label, created.snapshot.label);
    assert_eq!(view.snapshot.projection.subjects, created.snapshot.projection.subjects);
    // 恢复不是播种
    assert!(view.seed.is_none());
    // 但开场白仍然要给得出来——第一条消息还得靠它
    assert_eq!(view.opening.as_deref(), Some("雨落在青石板上。"));
    drop(reopened);
}

#[test]
fn resume_reports_an_unknown_session() {
    let (library, _) = library_with_card();
    let error = resume(&library, "没有这条").unwrap_err();
    assert!(error.contains("没有会话"), "{error}");
}

#[test]
fn load_reports_a_missing_asset() {
    let (library, _) = library_with_card();
    let error = load(&library, &AssetId::new("asset_9999")).unwrap_err();
    assert!(error.contains("没有资产"), "{error}");
}

#[test]
fn load_refuses_a_payload_that_could_not_be_a_world() {
    let mut library = Library::open_in_memory().unwrap();

    // `ImportedWorld` 的字段全是 `default`，所以「一个没有 name 的对象」会被静默读成空世界。
    // 这正是要在这一步拦掉的东西：坏数据必须报错，不能长得像「这个世界什么都没有」。
    let broken = library
        .save_asset(
            AssetDraft::new("坏资产", AssetOrigin::Written)
                .with_payload(serde_json::json!({ "characters": [] })),
        )
        .unwrap();
    let error = load(&library, &broken.id).unwrap_err();
    assert!(error.contains("缺少 name"), "{error}");

    let not_an_object = library
        .save_asset(
            AssetDraft::new("更坏的资产", AssetOrigin::Written)
                .with_payload(serde_json::json!([1, 2, 3])),
        )
        .unwrap();
    assert!(load(&library, &not_an_object.id).is_err());
}

#[test]
fn sessions_of_lists_what_was_created_for_a_world() {
    let (library, id) = library_with_card();
    let dir = TempDir::new();

    assert!(sessions_of(&library, &id).unwrap().is_empty());

    let (first, _, reference) = create(load(&library, &id).unwrap(), dir.path(), Some("一".into())).unwrap();
    library.attach_session(&reference).unwrap();
    drop(first);
    let (second, _, reference) =
        create(load(&library, &id).unwrap(), dir.path(), Some("二".into())).unwrap();
    library.attach_session(&reference).unwrap();
    drop(second);

    let sessions = sessions_of(&library, &id).unwrap();
    assert_eq!(sessions.len(), 2);
    // 一个资产可以有多个会话，各自的 `.ifworld` 是分开的
    assert_ne!(sessions[0].world_file, sessions[1].world_file);
    assert!(sessions.iter().all(|session| session.asset_id == id));
}
