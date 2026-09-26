//! 事件日志与世界线存储。
//!
//! `events` 是唯一真相，只追加；世界线只是指向某个 `seq` 的头指针。
//! 读世界 = 沿祖先链把事件折叠成 [`Projection`]，中间可以借助快照少折一部分。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use if_domain::event::{Event, EventDraft, Patch};
use if_domain::id::{BeatId, EventId, SceneId, TurnId, WorldLineId};
use if_domain::projection::{Projection, ProjectionAnchor};
use if_domain::rule::WorldSettings;
use if_domain::turn::TurnRecord;
use if_domain::value::WorldTime;
use if_domain::worldline::{covers, lineage, ParentRef, WorldLine, WorldLineError, WorldLineKind};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Result, StoreError};
use crate::schema::{meta_key, SCHEMA};

/// 每隔多少事件存一次投影快照（docs/03 §5）【初始值 200】。
pub const SNAPSHOT_INTERVAL: u64 = 200;

/// 一个世界的事件存储。
pub struct Store {
    conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// 打开（或创建）一个世界文件。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// 内存库。测试与试算用。
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // WAL 让读写不互相阻塞；内存库会忽略它，但不会报错。
        conn.execute_batch("pragma journal_mode=wal; pragma synchronous=normal;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    // ---------------------------------------------------------------- 世界创建

    /// 建立主世界线并写入 `world_created`。重复调用是幂等的（线已存在就不重建）。
    pub fn create_world(&mut self, label: &str, settings: WorldSettings) -> Result<Event> {
        let main = WorldLineId::new("wl_main");
        if self.world_line(&main)?.is_none() {
            self.upsert_world_line(&WorldLine::main(main.clone(), label))?;
        }
        let seed = settings.seed;
        let event = self.append(EventDraft::new(
            main.clone(),
            TurnId::numbered(0),
            WorldTime::EPOCH,
            Patch::WorldCreated {
                settings,
                label: label.to_owned(),
            },
        ))?;
        self.set_meta(meta_key::WORLD_LABEL, label)?;
        self.set_meta(meta_key::WORLD_SEED, &seed.to_string())?;
        self.set_meta(meta_key::MAIN_LINE, main.as_str())?;
        self.set_meta(meta_key::ACTIVE_LINE, main.as_str())?;
        Ok(event)
    }

    // ---------------------------------------------------------------- 追加

    /// 追加一个事件，自动分配 `seq` 与 `id`，并把所在世界线的头指针推上去。
    ///
    /// 到 [`SNAPSHOT_INTERVAL`] 的整数倍时自动存一份投影快照。
    pub fn append(&mut self, draft: EventDraft) -> Result<Event> {
        let mut written = self.append_batch(vec![draft])?;
        Ok(written.pop().expect("一条草稿必然写出一条事件"))
    }

    /// 一次追加一批。**全部成功或全部不写**——存在半截回合是不可能接受的。
    ///
    /// 写入前把每个事件折叠进所在世界线的当前投影；任何一条折叠失败（例如引用了不存在的命题），
    /// 整批都不写。跨过 [`SNAPSHOT_INTERVAL`] 整数倍时顺带存一份投影快照。
    pub fn append_batch(&mut self, drafts: Vec<EventDraft>) -> Result<Vec<Event>> {
        let lines = self.world_lines()?;
        let mut projections: BTreeMap<WorldLineId, Projection> = BTreeMap::new();
        for draft in &drafts {
            let line = lines.get(&draft.line).ok_or_else(|| {
                StoreError::WorldLine(WorldLineError::NotFound(draft.line.clone()))
            })?;
            if !line.kind.accepts_new_events() {
                return Err(StoreError::LineNotWritable(draft.line.to_string()));
            }
            if !projections.contains_key(&draft.line) {
                let p = self.load_projection(&draft.line)?;
                projections.insert(draft.line.clone(), p);
            }
        }

        // 先在事务之外取一次最大 seq，避免与 `transaction()` 的可变借用打架。
        let mut seq = self.max_seq()?;
        let tx = self.conn.transaction()?;
        let mut written = Vec::with_capacity(drafts.len());
        let mut heads: BTreeMap<WorldLineId, u64> = BTreeMap::new();
        let mut snapshot_due: Vec<WorldLineId> = Vec::new();

        for draft in drafts {
            seq += 1;
            let event = Event {
                id: EventId::numbered(seq),
                line: draft.line.clone(),
                seq,
                world_time: draft.world_time,
                narrative_order: draft.narrative_order,
                payload: draft.payload,
                caused_by: draft.caused_by,
                depends_on: draft.depends_on,
                turn: draft.turn,
                scene: draft.scene,
                beat: draft.beat,
            };
            projections
                .get_mut(&event.line)
                .expect("上面已为每条世界线加载投影")
                .apply(&event)?;
            insert_event_via(&tx, &event)?;
            heads.insert(event.line.clone(), seq);
            if seq % SNAPSHOT_INTERVAL == 0 && !snapshot_due.contains(&event.line) {
                snapshot_due.push(event.line.clone());
            }
            written.push(event);
        }

        for (line, head) in heads {
            tx.execute(
                "update world_lines set head_seq = ?2 where id = ?1",
                params![line.as_str(), head as i64],
            )?;
        }
        tx.commit()?;

        for line in snapshot_due {
            let projection = self.load_projection(&line)?;
            self.save_snapshot(&projection)?;
        }
        Ok(written)
    }

    fn max_seq(&self) -> Result<u64> {
        let max: Option<i64> = self
            .conn
            .query_row("select max(seq) from events", [], |r| r.get(0))?;
        Ok(max.unwrap_or(0) as u64)
    }

    // ---------------------------------------------------------------- 读取

    pub fn event_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("select count(*) from events", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// 下一条被追加的事件将拿到的 `seq`。
    ///
    /// 有了它，调用方就能在**写入之前**算出这一批事件的 ID：`append_batch` 按顺序
    /// 从 `next_seq()` 起发号，第 `i` 条拿到 `seq = next_seq() + i`，ID 是
    /// `EventId::numbered(该 seq)`。世界播种需要这一点——`Subject::created_by` 与
    /// `LoreEntry::source` 存的是**引入它的事件 ID**，而播种是一次成批写入的，
    /// 写完之后再补就晚了。这条规则由 `tests::event_ids_follow_next_seq` 钉住。
    pub fn next_seq(&self) -> Result<u64> {
        Ok(self.max_seq()? + 1)
    }

    /// 某个世界线祖先链上的全部事件，按 `seq` 排序。
    pub fn events_for_line(&self, line: &WorldLineId) -> Result<Vec<Event>> {
        let lines = self.world_lines()?;
        let segments = lineage(&lines, line)?;
        let head = lines.get(line).map_or(0, |l| l.head_seq);
        self.collect_events(head, 0, &segments)
    }

    /// 从投影里取「已展示的节拍」，按叙述顺序（docs/03 §5）。
    pub fn displayed_beats(&self, line: &WorldLineId) -> Result<Vec<if_domain::narrative::Beat>> {
        Ok(self.load_projection(line)?.beats)
    }

    fn collect_events(
        &self,
        upto_seq: u64,
        after_seq: u64,
        segments: &[if_domain::worldline::Segment],
    ) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "select seq, id, line, world_time, narrative_order, payload, caused_by, depends_on,
                    turn, scene, beat
             from events where seq > ?1 and seq <= ?2 order by seq",
        )?;
        let rows = stmt.query_map(params![after_seq as i64, upto_seq as i64], RawEvent::from_row)?;
        let mut events = Vec::new();
        for row in rows {
            let raw = row?;
            let event = raw.into_event()?;
            // 祖先链之外的事件（别的分支写的）不参与折叠。
            if covers(segments, &event.line, event.seq) {
                events.push(event);
            }
        }
        Ok(events)
    }

    // ---------------------------------------------------------------- 投影

    /// 折叠出某条世界线的当前世界。有快照就从快照续折。
    pub fn load_projection(&self, line: &WorldLineId) -> Result<Projection> {
        let lines = self.world_lines()?;
        let segments = lineage(&lines, line)?;
        let head = lines.get(line).map_or(0, |l| l.head_seq);

        let (from_seq, mut projection) = match self.latest_snapshot(line)? {
            // 快照只在自己的头指针之内才有效（回滚之后旧快照不能再用）。
            Some((at, snapshot)) if at <= head => (at, snapshot),
            _ => (0, Projection::genesis(line.clone())),
        };

        for event in self.collect_events(head, from_seq, &segments)? {
            projection.apply(&event)?;
        }
        projection.anchor = ProjectionAnchor {
            line: line.clone(),
            seq: head,
        };
        Ok(projection)
    }

    /// 不带快照的全量折叠。用来验证「快照重建 == 全量折叠」。
    pub fn load_projection_from_scratch(&self, line: &WorldLineId) -> Result<Projection> {
        let lines = self.world_lines()?;
        let segments = lineage(&lines, line)?;
        let head = lines.get(line).map_or(0, |l| l.head_seq);
        let mut projection = Projection::genesis(line.clone());
        for event in self.collect_events(head, 0, &segments)? {
            projection.apply(&event)?;
        }
        projection.anchor = ProjectionAnchor {
            line: line.clone(),
            seq: head,
        };
        Ok(projection)
    }

    pub fn save_snapshot(&self, projection: &Projection) -> Result<()> {
        let payload = serde_json::to_string(projection)?;
        self.conn.execute(
            "insert or replace into snapshots (line, at_seq, payload, created_at)
             values (?1, ?2, ?3, ?4)",
            params![
                projection.anchor.line.as_str(),
                projection.anchor.seq as i64,
                payload,
                now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn latest_snapshot(&self, line: &WorldLineId) -> Result<Option<(u64, Projection)>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "select payload from snapshots where line = ?1 order by at_seq desc limit 1",
                params![line.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match payload {
            Some(text) => {
                let projection: Projection = serde_json::from_str(&text)?;
                Ok(Some((projection.anchor.seq, projection)))
            }
            None => Ok(None),
        }
    }

    // ---------------------------------------------------------------- 世界线

    pub fn world_lines(&self) -> Result<BTreeMap<WorldLineId, WorldLine>> {
        world_lines_via(&self.conn)
    }

    pub fn world_line(&self, id: &WorldLineId) -> Result<Option<WorldLine>> {
        Ok(self.world_lines()?.get(id).cloned())
    }

    pub fn upsert_world_line(&self, line: &WorldLine) -> Result<()> {
        self.conn.execute(
            "insert into world_lines (id, parent_line, parent_seq, head_seq, label, kind)
             values (?1, ?2, ?3, ?4, ?5, ?6)
             on conflict(id) do update set
                parent_line = excluded.parent_line,
                parent_seq  = excluded.parent_seq,
                head_seq    = excluded.head_seq,
                label       = excluded.label,
                kind        = excluded.kind",
            params![
                line.id.as_str(),
                line.parent.as_ref().map(|p| p.line.as_str()),
                line.parent.as_ref().map(|p| p.at_seq as i64),
                line.head_seq as i64,
                line.label.as_str(),
                kind_str(line.kind),
            ],
        )?;
        Ok(())
    }

    /// 移动头指针。指向不存在的事件序号会被拒绝——头指针不能指向未来。
    pub fn set_head(&self, line: &WorldLineId, seq: u64) -> Result<()> {
        let max = self.max_seq()?;
        if seq > max {
            return Err(StoreError::LineNotWritable(format!(
                "头指针不能指向还不存在的 seq={seq}（当前最大 {max}）"
            )));
        }
        self.conn.execute(
            "update world_lines set head_seq = ?2 where id = ?1",
            params![line.as_str(), seq as i64],
        )?;
        Ok(())
    }

    /// 「回到此处」/「分支」：从某个节点另起一条线并把它设为活跃。
    pub fn fork(
        &mut self,
        from: &WorldLineId,
        at_seq: u64,
        kind: WorldLineKind,
        label: &str,
    ) -> Result<WorldLine> {
        let lines = self.world_lines()?;
        let parent = lines
            .get(from)
            .ok_or_else(|| StoreError::WorldLine(WorldLineError::NotFound(from.clone())))?;
        let child = WorldLine::fork(self.next_line_id(&lines), parent, at_seq, kind, label);
        self.upsert_world_line(&child)?;
        self.set_meta(meta_key::ACTIVE_LINE, child.id.as_str())?;
        Ok(child)
    }

    /// 「回滚」：把当前线降级为 abandoned（它保留被退回的那些事件），
    /// 另起一条新线从 `target_seq` 继续。
    ///
    /// 这样做事后仍能完整回看被退回的分支，且**不需要修改任何已写入的事件**。
    pub fn rollback_to(&mut self, line: &WorldLineId, target_seq: u64) -> Result<WorldLine> {
        let lines = self.world_lines()?;
        let old = lines
            .get(line)
            .ok_or_else(|| StoreError::WorldLine(WorldLineError::NotFound(line.clone())))?;
        if target_seq > old.head_seq {
            return Err(StoreError::LineNotWritable(format!(
                "回滚目标 seq={target_seq} 晚于当前头指针 {}",
                old.head_seq
            )));
        }

        let mut abandoned = old.clone();
        abandoned.kind = WorldLineKind::Abandoned;
        abandoned.label = format!("{}（已回滚）", old.label);
        self.upsert_world_line(&abandoned)?;

        let fresh = WorldLine::fork(
            self.next_line_id(&lines),
            old,
            target_seq,
            WorldLineKind::Branch,
            format!("{}（回滚自 seq {target_seq}）", old.label),
        );
        self.upsert_world_line(&fresh)?;
        self.set_meta(meta_key::ACTIVE_LINE, fresh.id.as_str())?;
        Ok(fresh)
    }

    /// 回溯型 IF 执行前的自动备份分支（已确认默认开启，docs/01 §12）。
    pub fn backup_before_retcon(&mut self, line: &WorldLineId) -> Result<WorldLine> {
        let head = self
            .world_lines()?
            .get(line)
            .map(|l| l.head_seq)
            .ok_or_else(|| StoreError::WorldLine(WorldLineError::NotFound(line.clone())))?;
        self.fork(line, head, WorldLineKind::RetconBackup, "回溯前备份")
    }

    fn next_line_id(&self, lines: &BTreeMap<WorldLineId, WorldLine>) -> WorldLineId {
        let mut n = lines.len();
        loop {
            let candidate = WorldLineId::new(format!("wl_{n:04}"));
            if !lines.contains_key(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    // ---------------------------------------------------------------- 回合记录

    pub fn save_turn(&self, turn: &TurnRecord) -> Result<()> {
        let payload = serde_json::to_string(turn)?;
        self.conn.execute(
            "insert or replace into turn_records (id, line, kind, started_at, payload)
             values (?1, ?2, ?3, ?4, ?5)",
            params![
                turn.id.as_str(),
                turn.line.as_str(),
                serde_json::to_value(turn.kind)?
                    .as_str()
                    .unwrap_or("unknown"),
                turn.started_at.minutes(),
                payload
            ],
        )?;
        Ok(())
    }

    pub fn load_turn(&self, id: &TurnId) -> Result<Option<TurnRecord>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "select payload from turn_records where id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match payload {
            Some(text) => Ok(Some(serde_json::from_str(&text)?)),
            None => Ok(None),
        }
    }

    pub fn turns_of_line(&self, line: &WorldLineId) -> Result<Vec<TurnRecord>> {
        let mut stmt = self.conn.prepare(
            "select payload from turn_records where line = ?1 order by started_at, id",
        )?;
        let rows = stmt.query_map(params![line.as_str()], |row| row.get::<_, String>(0))?;
        let mut turns = Vec::new();
        for row in rows {
            turns.push(serde_json::from_str(&row?)?);
        }
        Ok(turns)
    }

    // ---------------------------------------------------------------- 元信息

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "insert into meta (key, value) values (?1, ?2)
             on conflict(key) do update set value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("select value from meta where key = ?1", params![key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// 世界种子。命运骰子的根（docs/03 §8）。
    pub fn world_seed(&self) -> Result<Option<u64>> {
        Ok(self
            .get_meta(meta_key::WORLD_SEED)?
            .and_then(|s| s.parse().ok()))
    }

    pub fn active_line(&self) -> Result<Option<WorldLineId>> {
        Ok(self.get_meta(meta_key::ACTIVE_LINE)?.map(WorldLineId::new))
    }
}

// ---------------------------------------------------------------- 内部辅助

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn kind_str(kind: WorldLineKind) -> &'static str {
    match kind {
        WorldLineKind::Main => "main",
        WorldLineKind::Branch => "branch",
        WorldLineKind::Swipe => "swipe",
        WorldLineKind::RetconBackup => "retcon_backup",
        WorldLineKind::RoadNotTaken => "road_not_taken",
        WorldLineKind::Abandoned => "abandoned",
    }
}

fn kind_from_str(raw: &str) -> WorldLineKind {
    match raw {
        "branch" => WorldLineKind::Branch,
        "swipe" => WorldLineKind::Swipe,
        "retcon_backup" => WorldLineKind::RetconBackup,
        "road_not_taken" => WorldLineKind::RoadNotTaken,
        "abandoned" => WorldLineKind::Abandoned,
        _ => WorldLineKind::Main,
    }
}

fn insert_event_via(conn: &Connection, event: &Event) -> Result<()> {
    conn.execute(
        "insert into events
            (seq, id, line, world_time, narrative_order, event_type, payload,
             caused_by, depends_on, turn, scene, beat)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            event.seq as i64,
            event.id.as_str(),
            event.line.as_str(),
            event.world_time.minutes(),
            event.narrative_order.map(|n| n as i64),
            event.event_type().as_str(),
            serde_json::to_string(&event.payload)?,
            serde_json::to_string(&event.caused_by)?,
            serde_json::to_string(&event.depends_on)?,
            event.turn.as_str(),
            event.scene.as_ref().map(|s| s.as_str()),
            event.beat.as_ref().map(|b| b.as_str()),
        ],
    )?;
    Ok(())
}

fn world_lines_via(conn: &Connection) -> Result<BTreeMap<WorldLineId, WorldLine>> {
    let mut stmt = conn.prepare(
        "select id, parent_line, parent_seq, head_seq, label, kind from world_lines order by id",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let parent_line: Option<String> = row.get(1)?;
        let parent_seq: Option<i64> = row.get(2)?;
        let head_seq: i64 = row.get(3)?;
        let label: String = row.get(4)?;
        let kind: String = row.get(5)?;
        Ok(WorldLine {
            id: WorldLineId::new(id),
            parent: match (parent_line, parent_seq) {
                (Some(line), Some(seq)) => Some(ParentRef {
                    line: WorldLineId::new(line),
                    at_seq: seq as u64,
                }),
                _ => None,
            },
            head_seq: head_seq as u64,
            label,
            kind: kind_from_str(&kind),
        })
    })?;

    let mut lines = BTreeMap::new();
    for row in rows {
        let line = row?;
        lines.insert(line.id.clone(), line);
    }
    Ok(lines)
}

/// 从行里读出的原始字符串，反序列化放在之后做——
/// 这样 serde 的错误能原样冒出来，而不是被塞进 `rusqlite::Error`。
struct RawEvent {
    seq: i64,
    id: String,
    line: String,
    world_time: i64,
    narrative_order: Option<i64>,
    payload: String,
    caused_by: String,
    depends_on: String,
    turn: String,
    scene: Option<String>,
    beat: Option<String>,
}

impl RawEvent {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            seq: row.get(0)?,
            id: row.get(1)?,
            line: row.get(2)?,
            world_time: row.get(3)?,
            narrative_order: row.get(4)?,
            payload: row.get(5)?,
            caused_by: row.get(6)?,
            depends_on: row.get(7)?,
            turn: row.get(8)?,
            scene: row.get(9)?,
            beat: row.get(10)?,
        })
    }

    fn into_event(self) -> Result<Event> {
        Ok(Event {
            id: EventId::new(self.id),
            line: WorldLineId::new(self.line),
            seq: self.seq as u64,
            world_time: WorldTime::from_minutes(self.world_time),
            narrative_order: self.narrative_order.map(|n| n as u64),
            payload: serde_json::from_str(&self.payload)?,
            caused_by: serde_json::from_str(&self.caused_by)?,
            depends_on: serde_json::from_str(&self.depends_on)?,
            turn: TurnId::new(self.turn),
            scene: self.scene.map(SceneId::new),
            beat: self.beat.map(BeatId::new),
        })
    }
}
