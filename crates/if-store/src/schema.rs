//! 表结构（docs/12 §5）。
//!
//! 每个世界一个 SQLite 文件（`.ifworld`），便于导出、备份和同步。
//! `events` 只追加，是唯一真相；其余表都是它的投影或审计信息。
//!
//! 与 docs/12 §5 的差异：`beats` / `lore_entries` 不单独建表——它们是 `events` 的投影，
//! 由 [`crate::Store::load_projection`] 折叠得到，另建一份表只会引入不一致。
//! `assets` 与 `question_versions` 属于应用级数据，按 docs/12 §5 放在另一个库里。

/// 建表语句。全部 `if not exists`，可以重复执行。
pub const SCHEMA: &str = r#"
create table if not exists events (
    seq             integer primary key,
    id              text    not null unique,
    line            text    not null,
    world_time      integer not null,
    narrative_order integer,
    event_type      text    not null,
    payload         text    not null,
    caused_by       text    not null default '[]',
    depends_on      text    not null default '[]',
    turn            text    not null,
    scene           text,
    beat            text
);

create index if not exists idx_events_line on events(line);
create index if not exists idx_events_turn on events(turn);
create index if not exists idx_events_narrative
    on events(narrative_order) where narrative_order is not null;

create table if not exists world_lines (
    id          text primary key,
    parent_line text,
    parent_seq  integer,
    head_seq    integer not null,
    label       text    not null,
    kind        text    not null
);

create table if not exists snapshots (
    line       text    not null,
    at_seq     integer not null,
    payload    text    not null,
    created_at integer not null,
    primary key (line, at_seq)
);

create table if not exists turn_records (
    id         text primary key,
    line       text    not null,
    kind       text    not null,
    started_at integer not null,
    payload    text    not null
);

create index if not exists idx_turns_line on turn_records(line);

create table if not exists meta (
    key   text primary key,
    value text not null
);
"#;

/// 事件日志的元信息键。
pub mod meta_key {
    /// 世界种子。命运骰子的根（docs/03 §8）。
    pub const WORLD_SEED: &str = "world_seed";
    /// 世界显示名。
    pub const WORLD_LABEL: &str = "world_label";
    /// 主世界线 ID。
    pub const MAIN_LINE: &str = "main_line";
    /// 当前活跃的世界线。
    pub const ACTIVE_LINE: &str = "active_line";
}
