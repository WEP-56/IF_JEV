//! 会话的创建与恢复（docs/10 §3）。
//!
//! 「会话」是**某个世界被推进的那一段历史**，落在自己的 `.ifworld` 里；世界资产留在
//! `library.db`（见 `if-store::library` 的模块说明）。所以建会话要做两件事：把资产播种成
//! 一个新世界，再在世界库里记一条**引用**——记引用是为了「删资产时不静默删掉引用它的会话」。
//!
//! docs/10 §3 要求新建会话**必须先选一个已保存的世界**：这里没有「建一个空世界」的入口，
//! 客户端的空世界只能从「手动撰写」的资产来（那种资产的 payload 没有角色与设定，
//! 播完种子就是个空壳，播种简报会直接说「来源里没有角色」）。

use std::path::{Path, PathBuf};

use if_domain::id::AssetId;
use if_store::library::{Library, SessionRef};
use serde::Serialize;

use crate::importer::ImportedWorld;
use crate::seed::{self, SeedReport};
use crate::world_worker::{
    new_world_path, new_world_settings, CreateRequest, WorldHandle, WorldSnapshot, WorldWorker,
};

/// 会话向客户端呈现的样子。
///
/// 名字与题材**不在投影里**，只能从世界库带出来；开场白在卡的 payload 里。所以这个结构是
/// 「投影 + 世界库那一侧的一点元数据」的拼接，而不是单纯的快照。
#[derive(Debug, Serialize)]
pub struct SessionView {
    pub snapshot: WorldSnapshot,
    /// 会话在世界库里的 ID（也是世界文件名主干）。
    ///
    /// 前端拿它去删除或重新打开这条会话。没有它的话，前端只能靠世界文件路径反推，
    /// 而世界文件路径是后端实现的细节，不该成为客户端的寻址方式。
    pub session_id: String,
    pub asset_id: String,
    pub asset_name: String,
    pub genre: String,
    /// 卡里的开场白。它是**素材**，不是世界状态（见 [`crate::seed`]）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    /// 只有新建会话才有：这一次播种都做了什么。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<SeedReport>,
}

/// 从世界库里取出的一份世界材料。
#[derive(Debug)]
pub struct Material {
    pub id: AssetId,
    pub name: String,
    pub genre: String,
    pub world: ImportedWorld,
}

/// 读一个资产，并把不透明的 `payload` 还原成 [`ImportedWorld`]。
///
/// 反序列化之前先确认它是个带 `name` 的对象。`ImportedWorld` 的字段全是 `#[serde(default)]`，
/// 不检查的话，一个被写坏的 payload 会**静默变成一个空世界**——用户看到的是「这个世界怎么
/// 什么都没有」，而不是「世界库里那份材料坏了」。宁可在这里硬报错。
pub fn load(library: &Library, id: &AssetId) -> Result<Material, String> {
    let asset = library
        .load_asset(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("世界库里没有资产 {id}"))?;
    if asset.payload.get("name").and_then(serde_json::Value::as_str).is_none() {
        return Err(format!(
            "世界资产 {} 的内容缺少 name 字段，可能已经损坏",
            asset.id
        ));
    }
    let world: ImportedWorld = serde_json::from_value(asset.payload.clone())
        .map_err(|e| format!("世界资产 {} 的内容不是一份世界材料：{e}", asset.id))?;
    Ok(Material {
        id: asset.id,
        name: asset.name,
        genre: asset.genre,
        world,
    })
}

/// 新建一个会话（docs/10 §3）。
///
/// 返回三样东西：worker 句柄（调用方负责安装并持有）、给客户端的视图、以及**要在世界库里
/// 落下的会话引用**。引用由调用方写，是因为这一步之后还得把 worker 装进应用状态——
/// 让 `session` 同时管这两件事，它就得知道 Tauri 的 `AppState`。
pub fn create(
    material: Material,
    worlds_dir: &Path,
    label: Option<String>,
) -> Result<(WorldHandle, SessionView, SessionRef), String> {
    let label = label
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| material.name.trim().to_owned());
    if label.is_empty() {
        return Err("会话名称不能为空".into());
    }

    let path = new_world_path(worlds_dir, &label);
    let opening = seed::opening_of(&material.world);
    let handle = WorldWorker::create(
        path.clone(),
        CreateRequest::new(&label, new_world_settings(), Some(material.world)),
    )?;

    let reference = SessionRef::new(
        session_id(&path),
        material.id.clone(),
        &label,
        path.display().to_string(),
    );
    let view = SessionView {
        snapshot: handle.snapshot.clone(),
        session_id: reference.id.clone(),
        asset_id: material.id.to_string(),
        asset_name: material.name,
        genre: material.genre,
        opening,
        seed: handle.seed.clone(),
    };
    Ok((handle, view, reference))
}

/// 恢复一个已有会话：按引用找到那条 `.ifworld` 再打开它。
pub fn resume(library: &Library, session_id: &str) -> Result<(WorldHandle, SessionView), String> {
    let reference = library
        .session(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("世界库里没有会话 {session_id}"))?;
    let material = load(library, &reference.asset_id)?;
    let handle = WorldWorker::open(PathBuf::from(&reference.world_file))?;
    let view = SessionView {
        snapshot: handle.snapshot.clone(),
        session_id: reference.id.clone(),
        asset_id: material.id.to_string(),
        asset_name: material.name,
        genre: material.genre,
        opening: seed::opening_of(&material.world),
        // 恢复不是播种：世界早就写好了，这里没有简报可给。
        seed: None,
    };
    Ok((handle, view))
}

/// 某个世界已有的会话，按创建时间排。新建会话前用它提示「这个世界已经有会话了」。
pub fn sessions_of(library: &Library, id: &AssetId) -> Result<Vec<SessionRef>, String> {
    library.sessions_of_asset(id).map_err(|e| e.to_string())
}

/// 世界库里全部会话，最近的在前。**启动时靠它把侧栏恢复出来**——
/// 没有它，前端只能显示本次运行里建过的会话，重启就等于「会话全没了」。
pub fn all_sessions(library: &Library) -> Result<Vec<SessionRef>, String> {
    library.all_sessions().map_err(|e| e.to_string())
}

/// 一条会话在世界磁盘上的全部文件：`.ifworld` 与它的 SQLite 边车 `-wal` / `-shm`。
///
/// 边车文件不能漏：把主文件删掉而留下 `-wal`，下次有人用同名文件建会话时
/// SQLite 会读到一段**属于上一个世界的预写日志**。
pub fn world_files(path: &Path) -> Vec<PathBuf> {
    let mut files = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        files.push(PathBuf::from(name));
    }
    files
}

/// 删掉一条会话引用，返回它原来指向的世界文件。
///
/// **先删引用、再删文件**（文件由调用方删，因为它还要先关掉可能正开着的那个世界：
/// Windows 上打开着的文件删不掉）。顺序反过来的话，文件没了而引用还在，
/// 列表里会留下一条「点开就报错」的会话，而用户看不出哪里不对。
/// 引用先没了的话，最坏情况只是一个**没有人引用**的 `.ifworld` 留在磁盘上。
///
/// 返回 `None` 表示这条会话本来就不存在——调用方据此报错，而不是假装删掉了。
pub fn remove(library: &Library, session_id: &str) -> Result<Option<SessionRef>, String> {
    let Some(reference) = library.session(session_id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    library.detach_session(session_id).map_err(|e| e.to_string())?;
    Ok(Some(reference))
}

/// 会话 ID 用世界文件名主干：`new_world_path` 已经保证了它唯一（带纳秒时间戳），
/// 而且从 ID 就能看出它指向哪个文件。
fn session_id(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".to_owned())
}

#[cfg(test)]
mod tests;
