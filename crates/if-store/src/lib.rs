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

#![forbid(unsafe_code)]

pub mod error;
pub mod schema;
pub mod store;

pub use error::{Result, StoreError};
pub use schema::meta_key;
pub use store::{EventDraft, Store, SNAPSHOT_INTERVAL};
