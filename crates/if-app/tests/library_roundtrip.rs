//! 真实卡 → 世界库的端到端回归。
//!
//! `examples/inspect_card` 只体检、**不落盘**，所以「PNG 字节 → 解析 → 映射 → 写进
//! `library.db` → 再读回来」这条链路在真实卡上此前没有任何自动化覆盖。
//!
//! 真实卡在这里的价值是**规模**，不是字段：大乾卡 1.9 MB / 148 条 / 10 万字，
//! 正是 IPC 传输与 SQLite 写入的真实量级。合成 fixture 覆盖不到这个。
//!
//! 样本是第三方社区卡，版权不属于本仓库，已加进 `.gitignore`（docs/13 §6.4）。
//! 文件不在就**跳过**并说明原因，不让别人 clone 之后跑出红。

use std::path::PathBuf;

use if_app_lib::{importer, library};
use if_store::library::{AssetOrigin, Library};

const BIG_CARD: &str = "南疆风云大乾风华录_武侠江湖.png";
const SMALL_CARD: &str = "风之絮言.png";

/// 读本地样本；不存在返回 `None`。
fn sample(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("sk-example")
        .join(name);
    std::fs::read(path).ok()
}

/// 导入、落库、重导一遍；返回 `(资产数, 是否真的跑了)`。
fn round_trip() -> (usize, bool) {
    let samples = [(BIG_CARD, sample(BIG_CARD)), (SMALL_CARD, sample(SMALL_CARD))];
    if samples.iter().all(|(_, bytes)| bytes.is_none()) {
        eprintln!("跳过：sk-example/ 不存在（第三方社区卡不入库，见 docs/13 §6.4）");
        return (0, false);
    }

    let mut db = Library::open_in_memory().expect("内存世界库");
    let mut asset_ids = Vec::new();

    for (name, bytes) in samples {
        let Some(bytes) = bytes else {
            eprintln!("跳过 {name}：文件不在");
            continue;
        };

        let world = importer::parse_bytes(&bytes, Some(name)).unwrap_or_else(|e| panic!("{name} 解析失败：{e}"));
        let raw = importer::raw_json(&bytes).unwrap_or_else(|e| panic!("{name} 取原始 JSON 失败：{e}"));
        // 真实卡在这里的价值是规模：把量级打出来，出问题时一眼看得出是哪一档。
        eprintln!(
            "{name}: 源文件 {} KB → 来源附件 {} KB · {} 条设定",
            bytes.len() / 1024,
            raw.len() / 1024,
            world.lore.len()
        );
        // 来源附件必须是**内嵌的 JSON 文本**，不是 PNG 二进制：PNG 以 \x89PNG 开头。
        // 存错了的话世界库里会躺着几 MB 的图，而且日后无法用来重导。
        assert!(raw.starts_with('{'), "{name} 的来源附件不是 JSON 文本");
        assert!(raw.len() != bytes.len(), "{name} 的来源附件长度与源文件相同，像是存了二进制");

        let asset = db
            .save_asset(library::draft_from_import(&world, &raw, None).unwrap())
            .unwrap_or_else(|e| panic!("{name} 落库失败：{e}"));

        // 条目数与解析结果一致——大乾卡是 148 条，少一条都是静默丢数据
        assert_eq!(
            db.lore_count(&asset.id).unwrap() as usize,
            world.lore.len(),
            "{name} 落库条目数与解析结果不一致"
        );
        assert!(!world.lore.is_empty(), "{name} 应该解析出条目");
        assert_eq!(db.sources_of(&asset.id).unwrap().len(), 1, "{name} 应有且仅有一条来源记录");

        // 重导同一张卡：落在同一个资产上，条目**替换**而不是叠加
        let again = db
            .save_asset(library::draft_from_import(&world, &raw, Some(asset.id.clone())).unwrap())
            .unwrap();
        assert_eq!(again.id, asset.id, "{name} 重导落到了别的资产上");
        assert_eq!(again.revision, 2, "{name} 重导应把 revision 推到 2");
        assert_eq!(
            db.lore_count(&asset.id).unwrap() as usize,
            world.lore.len(),
            "{name} 重导后条目数翻倍了——按来源整组替换没生效"
        );

        // 详情里的 payload 能原样重建世界（这就是 IPC 上要传的量级）
        let detail = library::AssetDetail::load(&db, &asset.id).unwrap();
        let restored: importer::ImportedWorld =
            serde_json::from_value(detail.asset.payload).expect("payload 应能反序列化回 ImportedWorld");
        assert_eq!(restored, world, "{name} 的 payload 往返丢失了内容");
        assert_eq!(detail.lore.len(), world.lore.len(), "{name} 详情条目数不一致");

        // 列表摘要：不带 payload，但计数要对
        let summary = library::list(&db)
            .unwrap()
            .into_iter()
            .find(|s| s.id == asset.id)
            .expect("列表里应有这个资产");
        assert_eq!(summary.lore_count as usize, world.lore.len());
        assert_eq!(summary.source_count, 1);
        assert_eq!(summary.session_count, 0);
        assert_eq!(summary.origin, AssetOrigin::Imported);
        assert_eq!(summary.source_files, vec![name.to_owned()]);

        asset_ids.push(asset.id);
    }

    // 两张卡名字不同 → 两个不同来源 → 两个资产，互不影响
    assert_eq!(db.list_assets().unwrap().len(), asset_ids.len());
    (asset_ids.len(), true)
}

#[test]
fn real_png_cards_round_trip_through_the_library() {
    let (count, ran) = round_trip();
    if ran {
        eprintln!("真实卡回归完成：{count} 个资产");
    }
}

/// 单独盯住「重导不翻倍」这条规则——它是世界库存在的理由，
/// 值得在真实卡上单独失败一次，而不是混在上面那个大测试里。
#[test]
fn real_card_reimport_does_not_double_entries() {
    let Some(bytes) = sample(BIG_CARD) else {
        eprintln!("跳过：{BIG_CARD} 不在 sk-example/");
        return;
    };
    let world = importer::parse_bytes(&bytes, Some(BIG_CARD)).unwrap();
    let raw = importer::raw_json(&bytes).unwrap();

    let mut db = Library::open_in_memory().unwrap();
    let first = db
        .save_asset(library::draft_from_import(&world, &raw, None).unwrap())
        .unwrap();
    let baseline = db.lore_count(&first.id).unwrap();

    for round in 1..=3 {
        db.save_asset(
            library::draft_from_import(&world, &raw, Some(first.id.clone())).unwrap(),
        )
        .unwrap();
        assert_eq!(
            db.lore_count(&first.id).unwrap(),
            baseline,
            "第 {round} 次重导后条目数变了：{baseline} 条"
        );
    }
    assert_eq!(db.list_assets().unwrap().len(), 1, "重导不应造出新资产");
    assert_eq!(db.sources_of(&first.id).unwrap().len(), 1, "重导不应堆出多个来源记录");
}
