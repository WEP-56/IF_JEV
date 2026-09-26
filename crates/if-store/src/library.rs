//! 世界库：应用级的**世界资产**存储（docs/10 §2、docs/12 §5）。
//!
//! 世界资产与会话事件日志是**两个对象**（docs/10 §1）：资产是用户准备好的材料，
//! 会话是某个世界被推进的历史。一个资产可以有多个会话；新建会话只创建会话记录，
//! 不创建或复制资产。
//!
//! 所以它不在 `.ifworld` 里，而是单独一个 `library.db`——这正是 docs/12 §5 把
//! `assets` 划到「另一个库」的原因：删会话不该动到世界稿，删世界也不该静默删掉
//! 引用它的会话（后者由 [`Library::delete_asset`] 直接拒绝）。
//!
//! ## 为什么 `payload` 是不透明的 JSON
//!
//! 世界层的角色、世界书元数据、宏与警告，`if-store` 不解释它们，只负责存取；
//! 形状由上层决定（现在是 `if-app::importer` 的 `ImportedWorld`）。
//!
//! `ImportedWorld → if-domain` 的确定性映射已经落地（`if-app::seed`），但它**不在这里**，
//! 而且这里也不该认识它：映射的产物是**事件**，写进的是会话的 `.ifworld`，不是 `library.db`。
//! 世界库始终只存「用户准备好的那份材料」，保持不透明反而让两层的职责干净——
//! 上层改 `ImportedWorld` 的形状时，世界库一行都不用动。
//!
//! **但设定条目是例外，单独一张表**：因为「重新导入同一张卡时按来源整组替换」
//! 是这一层必须自己保证的规则（见 [`Library::save_asset`]），
//! 把条目埋在不透明的 payload 里就没法做——除非让 `if-store` 去解析上层的 JSON，
//! 那才是真正的分层错误。

use std::collections::BTreeSet;
use std::path::Path;

use if_domain::id::AssetId;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, StoreError};
use crate::store::now_ms;

/// 手写条目的来源键。它们没有来源文件，用一个固定值让「整组替换」也能作用于它们，
/// 而不是留一堆永远替换不掉的孤儿行。
pub const MANUAL_SOURCE: &str = "manual";

mod meta_key {
    /// 下一个资产序号。
    pub const NEXT_ASSET_SEQ: &str = "next_asset_seq";
}

const SCHEMA: &str = r#"
create table if not exists library_meta (
    key   text primary key,
    value text not null
);

create table if not exists world_assets (
    id         text primary key,
    name       text    not null,
    genre      text    not null default '',
    summary    text    not null default '',
    origin     text    not null default 'imported',
    revision   integer not null default 1,
    created_at integer not null,
    updated_at integer not null,
    payload    text    not null
);

create table if not exists world_sources (
    asset_id     text    not null,
    source_key   text    not null,
    kind         text    not null default '',
    file         text,
    format       text    not null default '',
    spec_version text,
    version      text,
    content_hash text    not null default '',
    imported_at  integer not null,
    raw          text,
    primary key (asset_id, source_key)
);

create index if not exists idx_sources_key on world_sources(source_key);

create table if not exists world_lore (
    asset_id   text    not null,
    source_key text    not null,
    ordinal    integer not null,
    uid        text,
    title      text    not null default '',
    section    text    not null default '',
    payload    text    not null,
    primary key (asset_id, source_key, ordinal)
);

create index if not exists idx_lore_asset on world_lore(asset_id);

create table if not exists world_sessions (
    id         text primary key,
    asset_id   text    not null,
    label      text    not null default '',
    world_file text    not null default '',
    created_at integer not null
);

create index if not exists idx_sessions_asset on world_sessions(asset_id);
"#;

/// 资产从哪来。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetOrigin {
    /// 从酒馆角色卡 / 世界书导入。
    #[default]
    Imported,
    /// 用户在世界库里手写。
    Written,
}

impl AssetOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            AssetOrigin::Imported => "imported",
            AssetOrigin::Written => "written",
        }
    }

    fn from_str(raw: &str) -> Self {
        match raw {
            "written" => AssetOrigin::Written,
            _ => AssetOrigin::Imported,
        }
    }
}

/// 一条来源附件的身份（docs/13 §0.2）。
///
/// `source_key` 是**来源身份**的指纹，不是内容哈希：同一张卡重新导入时它不变，
/// 即使文件名改了、卡版本升了。这是「按来源整组替换」能成立的前提——
/// 拿内容哈希当键的话，卡片一更新键就变了，重导会变成**追加**，
/// 旧条目留在那里形成残影（ST 的世界书是绑定时的快照，官方流程就是解绑→重绑→重导）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceRecord {
    pub source_key: String,
    /// 来源类型：`chara_card` / `lorebook` / `manual`。
    pub kind: String,
    pub file: Option<String>,
    /// 来源格式：`chara_card_v3` / `lorebook_v3` / …（docs/13 §0.1）。
    pub format: String,
    pub spec_version: Option<String>,
    /// 卡自身的版本（V3 `character_version`）。
    pub version: Option<String>,
    /// 原文哈希。用来判断「这次导入的内容和上次是不是同一份」。
    pub content_hash: String,
    /// 原始文件文本（PNG 卡则是内嵌的那段 JSON），作为来源附件保留，
    /// 便于日后重新导入而不必再找原文件。
    pub raw: Option<String>,
    pub imported_at: i64,
}

impl SourceRecord {
    pub fn new(source_key: impl Into<String>, kind: impl Into<String>, format: impl Into<String>) -> Self {
        Self {
            source_key: source_key.into(),
            kind: kind.into(),
            file: None,
            format: format.into(),
            spec_version: None,
            version: None,
            content_hash: String::new(),
            raw: None,
            imported_at: 0,
        }
    }
}

/// 一条设定条目。`payload` 是上层对该条目的完整记录（现在是 `ImportedLore` 的 JSON）；
/// `title` / `section` 单独提列，只为了让列表与检索不必解析 payload。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoreRecord {
    pub source_key: String,
    pub uid: Option<String>,
    pub ordinal: i64,
    pub title: String,
    pub section: String,
    pub payload: Value,
}

impl LoreRecord {
    pub fn new(source_key: impl Into<String>, ordinal: i64, payload: Value) -> Self {
        Self {
            source_key: source_key.into(),
            uid: None,
            ordinal,
            title: String::new(),
            section: String::new(),
            payload,
        }
    }
}

/// 保存一个世界资产时提交的内容。
#[derive(Clone, Debug, Default)]
pub struct AssetDraft {
    /// 空表示新建，由存储分配 `asset_NNNN`。
    pub id: Option<AssetId>,
    pub name: String,
    pub genre: String,
    pub summary: String,
    pub origin: AssetOrigin,
    /// 世界层内容（角色、世界书元数据、宏、警告……）。形状由上层决定。
    pub payload: Value,
    /// 本次导入的来源身份。给了它，就会连同 `lore` 一起把该来源的旧条目整组替换。
    pub source: Option<SourceRecord>,
    pub lore: Vec<LoreRecord>,
}

impl AssetDraft {
    pub fn new(name: impl Into<String>, origin: AssetOrigin) -> Self {
        Self {
            name: name.into(),
            origin,
            payload: Value::Object(serde_json::Map::new()),
            ..Default::default()
        }
    }

    pub fn with_id(mut self, id: impl Into<AssetId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_genre(mut self, genre: impl Into<String>) -> Self {
        self.genre = genre.into();
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    /// 附上一个来源与它的条目。同一来源再次提交时，旧条目会被整组替换。
    pub fn with_source(mut self, source: SourceRecord, lore: Vec<LoreRecord>) -> Self {
        self.source = Some(source);
        self.lore = lore;
        self
    }

    pub fn with_lore(mut self, lore: Vec<LoreRecord>) -> Self {
        self.lore = lore;
        self
    }
}

/// 一个世界资产。`payload` 原样返回，不做解释。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldAsset {
    pub id: AssetId,
    pub name: String,
    pub genre: String,
    pub summary: String,
    pub origin: AssetOrigin,
    /// 每次保存 +1。前端可以据此判断资产是否变过。
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub payload: Value,
}

/// 列表用的摘要。不带 payload 与条目正文——列表页不需要几千条设定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetSummary {
    pub id: AssetId,
    pub name: String,
    pub genre: String,
    pub summary: String,
    pub origin: AssetOrigin,
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub lore_count: u64,
    pub source_count: u64,
    /// 引用这个资产的会话数。大于 0 时不允许删除（docs/10 §2）。
    pub session_count: u64,
    /// 来源文件的显示名，列表页用来交代出处。
    pub source_files: Vec<String>,
}

/// 会话对资产的引用。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionRef {
    pub id: String,
    pub asset_id: AssetId,
    pub label: String,
    /// 会话的 `.ifworld` 路径。
    pub world_file: String,
    pub created_at: i64,
}

impl SessionRef {
    /// 造一条引用。`created_at` 用存储层的时钟，免得调用方各算一份。
    pub fn new(
        id: impl Into<String>,
        asset_id: impl Into<AssetId>,
        label: impl Into<String>,
        world_file: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            asset_id: asset_id.into(),
            label: label.into(),
            world_file: world_file.into(),
            created_at: now_ms(),
        }
    }
}

/// `world_assets` 的一行。单独开一个结构，是为了不让九元组出现在签名里。
struct AssetRow {
    id: String,
    name: String,
    genre: String,
    summary: String,
    origin: String,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    payload: String,
}

/// 世界库。一个应用一个文件。
pub struct Library {
    conn: Connection,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library").finish_non_exhaustive()
    }
}

impl Library {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // 与事件库一致：WAL 让读写不互相阻塞，内存库会忽略它但不报错。
        conn.execute_batch("pragma journal_mode=wal; pragma synchronous=normal;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    // ---------------------------------------------------------------- 资产

    /// 保存一个资产（有 `id` 就替换，没有就新建）。
    ///
    /// **按来源整组替换**：本次提交涉及到的每个 `source_key`，其旧条目先全部删除再写入新的。
    /// 「本次提交涉及」包括 `draft.source` 本身——所以「重新导入一张卡、这次一条条目都没解析出来」
    /// 也会清掉上次的条目，而不是留下残影。
    pub fn save_asset(&mut self, draft: AssetDraft) -> Result<WorldAsset> {
        let now = now_ms();
        let id = match draft.id.clone() {
            Some(id) => id,
            None => self.next_asset_id()?,
        };
        // 校验放在事务之前：提交一半再发现来源键不认识，回滚也没意义。
        self.check_lore_sources(&id, &draft)?;

        let existing = self.load_asset(&id)?;
        let (created_at, revision) = match &existing {
            Some(asset) => (asset.created_at, asset.revision + 1),
            None => (now, 1),
        };

        let payload = serde_json::to_string(&draft.payload)?;
        let replaced: BTreeSet<String> = draft
            .lore
            .iter()
            .map(|record| record.source_key.clone())
            .chain(draft.source.iter().map(|source| source.source_key.clone()))
            .collect();

        let tx = self.conn.transaction()?;
        tx.execute(
            "insert into world_assets
                (id, name, genre, summary, origin, revision, created_at, updated_at, payload)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             on conflict(id) do update set
                name = excluded.name,
                genre = excluded.genre,
                summary = excluded.summary,
                origin = excluded.origin,
                revision = excluded.revision,
                updated_at = excluded.updated_at,
                payload = excluded.payload",
            params![
                id.as_str(),
                draft.name,
                draft.genre,
                draft.summary,
                draft.origin.as_str(),
                revision as i64,
                created_at,
                now,
                payload
            ],
        )?;

        if let Some(source) = &draft.source {
            tx.execute(
                "insert into world_sources
                    (asset_id, source_key, kind, file, format, spec_version, version,
                     content_hash, imported_at, raw)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 on conflict(asset_id, source_key) do update set
                    kind = excluded.kind,
                    file = excluded.file,
                    format = excluded.format,
                    spec_version = excluded.spec_version,
                    version = excluded.version,
                    content_hash = excluded.content_hash,
                    imported_at = excluded.imported_at,
                    raw = excluded.raw",
                params![
                    id.as_str(),
                    source.source_key,
                    source.kind,
                    source.file,
                    source.format,
                    source.spec_version,
                    source.version,
                    source.content_hash,
                    if source.imported_at == 0 { now } else { source.imported_at },
                    source.raw
                ],
            )?;
        }

        for key in &replaced {
            tx.execute(
                "delete from world_lore where asset_id = ?1 and source_key = ?2",
                params![id.as_str(), key],
            )?;
        }
        for record in &draft.lore {
            tx.execute(
                "insert into world_lore (asset_id, source_key, ordinal, uid, title, section, payload)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.as_str(),
                    record.source_key,
                    record.ordinal,
                    record.uid,
                    record.title,
                    record.section,
                    serde_json::to_string(&record.payload)?
                ],
            )?;
        }
        tx.commit()?;

        self.load_asset(&id)?
            .ok_or_else(|| StoreError::AssetMissing(id.to_string()))
    }

    pub fn load_asset(&self, id: &AssetId) -> Result<Option<WorldAsset>> {
        let row: Option<AssetRow> = self
            .conn
            .query_row(
                "select id, name, genre, summary, origin, revision, created_at, updated_at, payload
                 from world_assets where id = ?1",
                params![id.as_str()],
                // 按列名取而不是按下标：以后加列不必回头数位置。
                |row| {
                    Ok(AssetRow {
                        id: row.get("id")?,
                        name: row.get("name")?,
                        genre: row.get("genre")?,
                        summary: row.get("summary")?,
                        origin: row.get("origin")?,
                        revision: row.get("revision")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                        payload: row.get("payload")?,
                    })
                },
            )
            .optional()?;
        match row {
            Some(row) => Ok(Some(WorldAsset {
                id: AssetId::new(row.id),
                name: row.name,
                genre: row.genre,
                summary: row.summary,
                origin: AssetOrigin::from_str(&row.origin),
                revision: row.revision as u64,
                created_at: row.created_at,
                updated_at: row.updated_at,
                payload: serde_json::from_str(&row.payload)?,
            })),
            None => Ok(None),
        }
    }

    /// 列表，最近更新的在前。列表页用，因此不含 payload 与条目正文。
    pub fn list_assets(&self) -> Result<Vec<AssetSummary>> {
        let mut stmt = self.conn.prepare(
            "select a.id, a.name, a.genre, a.summary, a.origin, a.revision,
                    a.created_at, a.updated_at,
                    (select count(*) from world_lore l where l.asset_id = a.id),
                    (select count(*) from world_sources s where s.asset_id = a.id),
                    (select count(*) from world_sessions w where w.asset_id = a.id)
             from world_assets a
             order by a.updated_at desc, a.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AssetSummary {
                id: AssetId::new(row.get::<_, String>(0)?),
                name: row.get(1)?,
                genre: row.get(2)?,
                summary: row.get(3)?,
                origin: AssetOrigin::from_str(&row.get::<_, String>(4)?),
                revision: row.get::<_, i64>(5)? as u64,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                lore_count: row.get::<_, i64>(8)? as u64,
                source_count: row.get::<_, i64>(9)? as u64,
                session_count: row.get::<_, i64>(10)? as u64,
                source_files: Vec::new(),
            })
        })?;

        let mut summaries = Vec::new();
        for row in rows {
            let mut summary = row?;
            summary.source_files = self
                .sources_of(&summary.id)?
                .into_iter()
                .map(|source| source.file.unwrap_or_else(|| source.kind.clone()))
                .collect();
            summaries.push(summary);
        }
        Ok(summaries)
    }

    /// 删除一个资产。**被会话引用时拒绝**（docs/10 §2：不得静默删除会话数据）。
    pub fn delete_asset(&mut self, id: &AssetId) -> Result<()> {
        let sessions = self.session_count(id)?;
        if sessions > 0 {
            return Err(StoreError::AssetInUse {
                asset: id.to_string(),
                sessions,
            });
        }
        let tx = self.conn.transaction()?;
        tx.execute("delete from world_lore where asset_id = ?1", params![id.as_str()])?;
        tx.execute("delete from world_sources where asset_id = ?1", params![id.as_str()])?;
        tx.execute("delete from world_assets where id = ?1", params![id.as_str()])?;
        tx.commit()?;
        Ok(())
    }

    fn next_asset_id(&mut self) -> Result<AssetId> {
        let mut n: u64 = self
            .get_meta(meta_key::NEXT_ASSET_SEQ)?
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(1);
        // 序号可能落后于实际表（手工改过库、或从别处拷来的文件），所以逐个探到没被占用的。
        loop {
            let candidate = AssetId::numbered(n);
            let taken: Option<String> = self
                .conn
                .query_row(
                    "select id from world_assets where id = ?1",
                    params![candidate.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if taken.is_none() {
                self.set_meta(meta_key::NEXT_ASSET_SEQ, &(n + 1).to_string())?;
                return Ok(candidate);
            }
            n += 1;
        }
    }

    /// 条目的来源键必须是已登记的来源，或是手写来源。
    ///
    /// 不加这道校验的话，来源键打错一个字符就会造出一组**永远不会被替换**的孤儿条目
    /// ——重导时按的是正确键，孤儿行留在库里，而且看不出哪里不对。
    fn check_lore_sources(&self, id: &AssetId, draft: &AssetDraft) -> Result<()> {
        let draft_key = draft.source.as_ref().map(|source| source.source_key.as_str());
        for record in &draft.lore {
            let key = record.source_key.as_str();
            if key == MANUAL_SOURCE || Some(key) == draft_key {
                continue;
            }
            let registered: Option<String> = self
                .conn
                .query_row(
                    "select source_key from world_sources where asset_id = ?1 and source_key = ?2",
                    params![id.as_str(), key],
                    |row| row.get(0),
                )
                .optional()?;
            if registered.is_none() {
                return Err(StoreError::UnknownLoreSource(key.to_owned()));
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- 来源

    pub fn sources_of(&self, id: &AssetId) -> Result<Vec<SourceRecord>> {
        let mut stmt = self.conn.prepare(
            "select source_key, kind, file, format, spec_version, version, content_hash,
                    imported_at, raw
             from world_sources where asset_id = ?1 order by imported_at, source_key",
        )?;
        let rows = stmt.query_map(params![id.as_str()], |row| {
            Ok(SourceRecord {
                source_key: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                format: row.get(3)?,
                spec_version: row.get(4)?,
                version: row.get(5)?,
                content_hash: row.get(6)?,
                imported_at: row.get(7)?,
                raw: row.get(8)?,
            })
        })?;
        let mut sources = Vec::new();
        for row in rows {
            sources.push(row?);
        }
        Ok(sources)
    }

    /// 某个来源键归属哪个资产。重新导入时先问它，就知道这次是「新增来源」还是「替换来源」。
    pub fn asset_of_source(&self, source_key: &str) -> Result<Option<AssetId>> {
        Ok(self
            .conn
            .query_row(
                "select asset_id from world_sources where source_key = ?1
                 order by imported_at limit 1",
                params![source_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(AssetId::new))
    }

    // ---------------------------------------------------------------- 条目

    pub fn lore_of(&self, id: &AssetId) -> Result<Vec<LoreRecord>> {
        self.lore_of_source(id, None)
    }

    /// `source` 给定时只取该来源的条目。
    pub fn lore_of_source(&self, id: &AssetId, source: Option<&str>) -> Result<Vec<LoreRecord>> {
        let mut stmt = self.conn.prepare(
            "select source_key, uid, ordinal, title, section, payload
             from world_lore
             where asset_id = ?1 and (?2 is null or source_key = ?2)
             order by source_key, ordinal",
        )?;
        let rows = stmt.query_map(params![id.as_str(), source], |row| {
            let payload: String = row.get(5)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                payload,
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (source_key, uid, ordinal, title, section, payload) = row?;
            records.push(LoreRecord {
                source_key,
                uid,
                ordinal,
                title,
                section,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(records)
    }

    pub fn lore_count(&self, id: &AssetId) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "select count(*) from world_lore where asset_id = ?1",
            params![id.as_str()],
            |row| row.get(0),
        )?;
        Ok(n as u64)
    }

    // ---------------------------------------------------------------- 会话引用

    pub fn attach_session(&self, session: &SessionRef) -> Result<()> {
        self.conn.execute(
            "insert into world_sessions (id, asset_id, label, world_file, created_at)
             values (?1, ?2, ?3, ?4, ?5)
             on conflict(id) do update set
                asset_id = excluded.asset_id,
                label = excluded.label,
                world_file = excluded.world_file",
            params![
                session.id,
                session.asset_id.as_str(),
                session.label,
                session.world_file,
                session.created_at
            ],
        )?;
        Ok(())
    }

    pub fn detach_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "delete from world_sessions where id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// 按 id 取一条会话引用。恢复会话时用它找到那条 `.ifworld`。
    pub fn session(&self, id: &str) -> Result<Option<SessionRef>> {
        Ok(self
            .conn
            .query_row(
                "select id, asset_id, label, world_file, created_at
                 from world_sessions where id = ?1",
                params![id],
                |row| {
                    Ok(SessionRef {
                        id: row.get(0)?,
                        asset_id: AssetId::new(row.get::<_, String>(1)?),
                        label: row.get(2)?,
                        world_file: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn sessions_of_asset(&self, id: &AssetId) -> Result<Vec<SessionRef>> {
        let mut stmt = self.conn.prepare(
            "select id, asset_id, label, world_file, created_at
             from world_sessions where asset_id = ?1 order by created_at, id",
        )?;
        let rows = stmt.query_map(params![id.as_str()], |row| {
            Ok(SessionRef {
                id: row.get(0)?,
                asset_id: AssetId::new(row.get::<_, String>(1)?),
                label: row.get(2)?,
                world_file: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// 世界库里**全部**会话，最近的在前。
    ///
    /// 与 [`Library::sessions_of_asset`] 分开，不是为了少写一个 `where`：
    /// 「这台机器上有哪些会话」和「这个资产下有哪些会话」是两个不同的问题。
    /// 前者是启动时赖以恢复侧栏的问题，按资产问的话 N 个资产就是 N 次往返；
    /// 而且它天然容得下「会话引用的资产已经不在库里」这种情况——那种会话照样该列出来
    /// （点开时会报「没有资产」，总比它凭空消失、用户以为数据丢了要好）。
    pub fn all_sessions(&self) -> Result<Vec<SessionRef>> {
        let mut stmt = self.conn.prepare(
            "select id, asset_id, label, world_file, created_at
             from world_sessions order by created_at desc, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRef {
                id: row.get(0)?,
                asset_id: AssetId::new(row.get::<_, String>(1)?),
                label: row.get(2)?,
                world_file: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn session_count(&self, id: &AssetId) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "select count(*) from world_sessions where asset_id = ?1",
            params![id.as_str()],
            |row| row.get(0),
        )?;
        Ok(n as u64)
    }

    // ---------------------------------------------------------------- 元信息

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "insert into library_meta (key, value) values (?1, ?2)
             on conflict(key) do update set value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "select value from library_meta where key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests;
