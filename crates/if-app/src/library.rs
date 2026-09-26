//! 世界资产 ↔ 世界库（`library.db`）的映射（docs/10 §2）。
//!
//! `if-store::library` 只认不透明的 `payload` 和拆表后的设定条目；把
//! [`ImportedWorld`] 拆成这个形状是**这一层**的事。反过来让存储层去认识导入结果，
//! 依赖方向就指反了（`if-store` 不依赖 `if-app`）。
//!
//! ## 来源身份键（`source_key`）
//!
//! 重导同一张卡时要**替换**而不是**追加**，靠的就是这个键。它是来源身份的指纹，
//! 不是内容哈希——否则卡片一更新键就变了，重导会叠出一堆残影。
//!
//! 因此键只取**稳定的身份字段**：来源类别 + 名字。**不包括**卡版本
//! （`character_version`）、文件哈希、文件名——这三样都会随着「用户改了卡、
//! 重新下载了一遍」而变，而那种情况恰恰要替换。
//!
//! 代价是：用户把卡改名之后再导入，会被当成新来源、与旧的并存。这是**可见且可恢复**的
//! （列表里能看到两条、可以删掉不要的那条），比静默覆盖一张同名的别的卡安全得多。
//! 内容是否变过是另一个问题，由 `content_hash` 回答。

use if_domain::id::AssetId;
use if_store::library::{
    AssetDraft, AssetOrigin, AssetSummary, Library, LoreRecord, SourceRecord, WorldAsset,
};

use crate::importer::{ImportedLore, ImportedWorld, LoreSection};

/// 卡内嵌世界书 / 独立世界书的来源类别前缀。
const BOOK_KIND: &str = "book";
/// 角色卡的来源类别前缀。
const CARD_KIND: &str = "card";

/// 从导入结果推出来源身份键。见模块文档的取舍说明。
pub fn source_key_of(world: &ImportedWorld) -> String {
    let kind = match source_kind_of(world) {
        "lorebook" => BOOK_KIND,
        _ => CARD_KIND,
    };
    format!("{kind}:{}", identity_of(world))
}

/// `chara_card` 或 `lorebook`。
pub fn source_kind_of(world: &ImportedWorld) -> &'static str {
    match world.source_format.as_str() {
        "lorebook_v3" | "world_info_json" => "lorebook",
        _ => "chara_card",
    }
}

/// 名字；名字为空（理论上导入层会拦掉）时退到文件名主干，最后退到占位。
fn identity_of(world: &ImportedWorld) -> String {
    let name = world.name.trim();
    if !name.is_empty() {
        return name.to_owned();
    }
    world
        .source_file
        .as_deref()
        .and_then(|file| {
            std::path::Path::new(file)
                .file_stem()
                .map(|stem| stem.to_string_lossy().trim().to_owned())
        })
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "未命名".to_owned())
}

/// 原文哈希。回答的是「这次导入的内容与上次是不是同一份」，与来源身份无关。
pub fn content_hash(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(raw.as_bytes());
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 把一次导入结果转成世界库的写入请求。
///
/// `asset` 给定时是**替换**该资产（重导），否则新建。
/// `raw` 是原始文本（PNG 卡则是内嵌的那段 JSON），作为来源附件留存。
pub fn draft_from_import(
    world: &ImportedWorld,
    raw: &str,
    asset: Option<AssetId>,
) -> Result<AssetDraft, String> {
    let key = source_key_of(world);

    let mut source = SourceRecord::new(key.clone(), source_kind_of(world), world.source_format.clone());
    source.file = world.source_file.clone();
    source.spec_version = world.spec_version.clone();
    // 卡自身的版本：记录下来给人看，但不参与来源身份（见模块文档）。
    source.version = world
        .characters
        .iter()
        .find_map(|character| character.character_version.clone())
        .filter(|version| !version.trim().is_empty());
    source.content_hash = content_hash(raw);
    source.raw = Some(raw.to_owned());

    let mut lore = Vec::with_capacity(world.lore.len());
    for (ordinal, entry) in world.lore.iter().enumerate() {
        lore.push(lore_record(&key, ordinal, entry)?);
    }

    let mut draft = AssetDraft::new(identity_of(world), AssetOrigin::Imported)
        .with_genre(world.genre.clone())
        .with_summary(summary_of(world))
        .with_payload(serde_json::to_value(world).map_err(|e| format!("世界内容序列化失败：{e}"))?)
        .with_source(source, lore);
    if let Some(id) = asset {
        draft = draft.with_id(id);
    }
    Ok(draft)
}

/// 手写资产：没有来源文件，因此没有来源记录；条目走 `manual` 来源键（见 [`MANUAL_SOURCE`]）。
///
/// [`MANUAL_SOURCE`]: if_store::library::MANUAL_SOURCE
pub fn draft_written(name: &str, genre: &str, summary: &str, asset: Option<AssetId>) -> AssetDraft {
    let mut draft = AssetDraft::new(name.trim(), AssetOrigin::Written)
        .with_genre(genre.trim())
        .with_summary(summary.trim())
        // 手写资产的世界层内容为空骨架：主体、设定、规则由用户在世界库里补。
        .with_payload(serde_json::json!({
            "name": name.trim(),
            "genre": genre.trim(),
            "summary": summary.trim(),
            "source_format": "manual",
            "source_kind": "manual",
            "characters": [],
            "lore": [],
        }));
    if let Some(id) = asset {
        draft = draft.with_id(id);
    }
    draft
}

fn lore_record(key: &str, ordinal: usize, entry: &ImportedLore) -> Result<LoreRecord, String> {
    let mut record = LoreRecord::new(
        key,
        ordinal as i64,
        serde_json::to_value(entry).map_err(|e| format!("设定条目序列化失败：{e}"))?,
    );
    record.uid = entry.source_uid.clone();
    record.title = entry.title.clone();
    record.section = section_key(entry.section).to_owned();
    Ok(record)
}

/// 条目的归段。与 `importer::LoreSection` 的 serde 名字保持一致
/// （`if-store` 不依赖上层类型，所以这里显式写出字符串）。
fn section_key(section: LoreSection) -> &'static str {
    match section {
        LoreSection::World => "world",
        LoreSection::Character => "character",
        LoreSection::Scene => "scene",
        LoreSection::Style => "style",
    }
}

/// 列表页的一句话说明：优先卡自带摘要，其次世界书描述，再次创作者备注，最后角色描述开头。
/// 都没有就留空，不编造。
fn summary_of(world: &ImportedWorld) -> String {
    let candidates = [
        world.summary.clone(),
        world.lore_meta.description.clone(),
        world
            .characters
            .first()
            .map(|character| character.creator_notes.clone())
            .unwrap_or_default(),
        world
            .characters
            .first()
            .map(|character| character.description.clone())
            .unwrap_or_default(),
    ];
    for candidate in candidates {
        let text = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            return clip(&text, SUMMARY_LIMIT);
        }
    }
    String::new()
}

const SUMMARY_LIMIT: usize = 140;

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

/// 一次世界库读取的完整内容：资产 + 来源 + 条目 + 引用它的会话。
///
/// 前端要重建世界视图就得同时拿到这几样（来源用来交代出处，会话用来提示「删不掉」的原因），
/// 所以一次给全，避免来回三趟 IPC。
#[derive(Debug, serde::Serialize)]
pub struct AssetDetail {
    pub asset: WorldAsset,
    pub sources: Vec<SourceRecord>,
    pub lore: Vec<LoreRecord>,
    pub sessions: Vec<if_store::library::SessionRef>,
}

impl AssetDetail {
    pub fn load(library: &Library, id: &AssetId) -> Result<Self, String> {
        let asset = library
            .load_asset(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("世界库里没有资产 {id}"))?;
        Ok(Self {
            sources: library.sources_of(id).map_err(|e| e.to_string())?,
            lore: library.lore_of(id).map_err(|e| e.to_string())?,
            sessions: library.sessions_of_asset(id).map_err(|e| e.to_string())?,
            asset,
        })
    }
}

/// 资产写入后返回「详情 + 刷新过的列表」。
///
/// 前端确认导入后既要把卡片插进列表，又要能立刻展开预览；一次调用给全，列表就不用手拼。
#[derive(Debug, serde::Serialize)]
pub struct AssetChange {
    pub detail: AssetDetail,
    pub assets: Vec<AssetSummary>,
}

pub fn list(library: &Library) -> Result<Vec<AssetSummary>, String> {
    library.list_assets().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
