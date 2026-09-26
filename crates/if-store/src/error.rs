//! 存储错误。手写 `Display` / `Error`，避免为两个 crate 引入 `thiserror`——

use std::fmt;

use if_domain::projection::ProjectionError;
use if_domain::worldline::WorldLineError;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    /// 事件折叠失败：数据不一致，通常意味着写入路径有 bug。
    Projection(ProjectionError),
    WorldLine(WorldLineError),
    /// 世界线不存在，或者不接受新事件（备份分支、未选之路）。
    LineNotWritable(String),
    /// 空数据库：还没有 `world_created`。
    WorldNotCreated,
    /// 世界库里没有这个资产。
    AssetMissing(String),
    /// 资产被会话引用，不允许删除（docs/10 §2：不得静默删掉会话数据）。
    AssetInUse { asset: String, sessions: u64 },
    /// 条目挂在一个没登记过的来源键上。多半是键写错了。
    UnknownLoreSource(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "SQLite 错误: {e}"),
            StoreError::Json(e) => write!(f, "JSON 编解码错误: {e}"),
            StoreError::Projection(e) => write!(f, "投影失败: {e}"),
            StoreError::WorldLine(e) => write!(f, "世界线错误: {e}"),
            StoreError::LineNotWritable(line) => {
                write!(f, "世界线 {line} 不接受新事件（备份分支或未选之路）")
            }
            StoreError::WorldNotCreated => write!(f, "世界尚未创建，缺少 world_created 事件"),
            StoreError::AssetMissing(id) => write!(f, "世界库里没有资产 {id}"),
            StoreError::AssetInUse { asset, sessions } => write!(
                f,
                "资产 {asset} 被 {sessions} 个会话引用，不能删除；请先删除或改绑这些会话"
            ),
            StoreError::UnknownLoreSource(key) => {
                write!(f, "设定条目的来源键 {key} 没有对应的来源记录")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sqlite(e) => Some(e),
            StoreError::Json(e) => Some(e),
            StoreError::Projection(e) => Some(e),
            StoreError::WorldLine(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        StoreError::Sqlite(value)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        StoreError::Json(value)
    }
}

impl From<ProjectionError> for StoreError {
    fn from(value: ProjectionError) -> Self {
        StoreError::Projection(value)
    }
}

impl From<WorldLineError> for StoreError {
    fn from(value: WorldLineError) -> Self {
        StoreError::WorldLine(value)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;
