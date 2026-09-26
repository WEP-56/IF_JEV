//! # if-store
//!
//! IF 的事件存储（[docs/12 工程架构](https://github.com/WEP-56/IF_JEV/blob/main/docs/12-工程架构.md) §5）。
//!
//! 每个世界一个 SQLite 文件（`.ifworld`），便于导出、备份和同步。
//!
//! - `events` 只追加，是唯一真相；
//! - `world_lines` 只是指向某个 `seq` 的头指针；
//! - `snapshots` 是投影的缓存，用来少折一部分事件；
//! - `turn_records` 保存判定与裁决的审计信息。
//!
//! 折叠逻辑本身在 `if-domain` 的 [`if_domain::projection`] 里，这里是纯计算，
//! 因此「同一事件序列得到同一投影」可以用单元测试直接验证。
//!
//! ## 两个库
//!
//! - 会话：每个世界一个 `.ifworld`（[`Store`]），只放某个世界被推进的历史；
//! - 世界库：应用级一个 `library.db`（[`library::Library`]），放用户准备好的
//!   **世界资产**。二者是两个对象（docs/10 §1），一个资产可以有多个会话。
//!
//! 分开是刻意的：删会话不该动到世界稿，删世界也不该静默删掉引用它的会话。

#![forbid(unsafe_code)]

pub mod error;
pub mod library;
pub mod schema;
pub mod store;

pub use error::{Result, StoreError};
pub use library::{
    AssetDraft, AssetOrigin, AssetSummary, Library, LoreRecord, SessionRef, SourceRecord,
    WorldAsset, MANUAL_SOURCE,
};
pub use schema::meta_key;
pub use store::{Store, SNAPSHOT_INTERVAL};

/// 待写入的事件草稿住在 `if-domain`（回合编排也产出它），这里原样转出，
/// 让 `if_store::EventDraft` 这个路径继续成立。
pub use if_domain::event::EventDraft;
