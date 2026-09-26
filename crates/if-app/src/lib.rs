//! IF 桌面应用入口：Tauri 命令与事件（docs/12 §4）。
//!
//! 目前接入的是设置、密钥、连通性诊断、**世界库**（世界资产的持久化，docs/10 §2），
//! 以及**会话的创建与恢复**（把世界资产播种成一个新世界，docs/10 §3）。
//! 回合流程的其余部分（继续回合、观测、节拍渲染……）随 `if-pipeline` 一起接入。

mod diagnostics;
mod if_parser;
// 对 `examples/inspect_card` 暴露，便于对真实卡文件做导入体检。
pub mod importer;
pub mod library;
/// 导入结果 → `if-domain` 的确定性映射：会话创建时的播种（docs/10 §3）。
pub mod seed;
mod secrets;
mod session;
mod settings;
/// 名字 → 文件名 / ID 片段的共用规则。
pub mod slug;
mod world_worker;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use if_domain::id::AssetId;
use if_store::library::{AssetSummary, Library, SessionRef};
use serde::Serialize;
use tauri::{Emitter, Manager, State};

use diagnostics::{JudgeTestInput, JudgeTestResult, LlmTestResult};
use if_parser::IfDraft;
use library::{AssetChange, AssetDetail};
use secrets::{KeySlot, KeySource};
use session::SessionView;
use settings::{AppSettings, Slot};
use world_worker::{WorldSnapshot, WorldWorker};

struct AppState {
    settings: Mutex<AppSettings>,
    settings_path: PathBuf,
    /// 世界库（`library.db`）。世界资产与会话事件日志是两个对象，因此两个库（docs/10 §1）。
    library: Mutex<Library>,
    runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    world: Mutex<Option<WorldWorker>>,
}

impl AppState {
    fn snapshot(&self) -> AppSettings {
        self.settings.lock().expect("settings poisoned").clone()
    }

    fn start_run(&self, run_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.runs.lock().expect("runs poisoned").insert(run_id.to_owned(), flag.clone());
        flag
    }

    fn end_run(&self, run_id: &str) {
        self.runs.lock().expect("runs poisoned").remove(run_id);
    }
}

#[derive(Serialize)]
struct KeyStatus {
    structure: KeySource,
    narrative: KeySource,
    jev: KeySource,
}

#[derive(Serialize)]
struct SettingsView {
    settings: AppSettings,
    keys: KeyStatus,
    settings_path: String,
}

fn view(state: &AppState) -> SettingsView {
    SettingsView {
        settings: state.snapshot(),
        keys: KeyStatus {
            structure: secrets::get(KeySlot::Structure).1,
            narrative: secrets::get(KeySlot::Narrative).1,
            jev: secrets::get(KeySlot::Jev).1,
        },
        settings_path: state.settings_path.display().to_string(),
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> SettingsView {
    view(&state)
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<SettingsView, String> {
    settings.save(&state.settings_path).map_err(|e| format!("保存设置失败：{e}"))?;
    *state.settings.lock().expect("settings poisoned") = settings;
    Ok(view(&state))
}

/// 空字符串表示清除。
#[tauri::command]
fn set_api_key(state: State<'_, AppState>, slot: KeySlot, key: String) -> Result<SettingsView, String> {
    secrets::set(slot, &key)?;
    Ok(view(&state))
}

fn slot_with_key(settings: &AppSettings, slot: Slot) -> if_agent::ProviderSettings {
    let mut provider = settings.slot(slot).clone();
    let key_slot = match slot {
        Slot::Structure => KeySlot::Structure,
        Slot::Narrative => KeySlot::Narrative,
    };
    provider.api_key = secrets::get(key_slot).0.unwrap_or_default();
    provider
}

fn jev_key() -> Result<String, String> {
    secrets::get(KeySlot::Jev).0.ok_or_else(|| "还没有配置 Jev 密钥".to_owned())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
async fn test_llm(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    slot: Slot,
    prompt: String,
    with_tool: bool,
) -> Result<LlmTestResult, String> {
    let provider = slot_with_key(&state.snapshot(), slot);
    let cancel = state.start_run(&run_id);
    let id = run_id.clone();
    let result = blocking(move || {
        diagnostics::test_llm(provider, id, prompt, with_tool, cancel, |event| {
            let _ = app.emit("diag://llm", event);
        })
    })
    .await;
    state.end_run(&run_id);
    result
}

#[tauri::command]
async fn probe_jev(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.snapshot();
    let key = jev_key()?;
    blocking(move || diagnostics::jev(&settings.jev, key).probe().map_err(|e| e.to_string())).await
}

/// `backend`：`jev` 或 `llm`（用结构模型充当裁判，D11）。
#[tauri::command]
async fn test_judge(
    state: State<'_, AppState>,
    run_id: String,
    backend: String,
    input: JudgeTestInput,
) -> Result<JudgeTestResult, String> {
    let settings = state.snapshot();
    let cancel = state.start_run(&run_id);
    let result = match backend.as_str() {
        "jev" => match jev_key() {
            Ok(key) => {
                let judge = diagnostics::jev(&settings.jev, key);
                blocking(move || diagnostics::run_judge(&judge, "jev", input, &cancel)).await
            }
            Err(e) => Err(e),
        },
        "llm" => {
            let judge = diagnostics::llm_judge(slot_with_key(&settings, Slot::Structure));
            blocking(move || diagnostics::run_judge(&judge, "llm", input, &cancel)).await
        }
        other => Err(format!("未知的判定后端 {other:?}")),
    };
    state.end_run(&run_id);
    result
}

#[tauri::command]
async fn parse_if_model(
    state: State<'_, AppState>,
    run_id: String,
    input: String,
) -> Result<IfDraft, String> {
    let provider = slot_with_key(&state.snapshot(), Slot::Structure);
    let cancel = state.start_run(&run_id);
    let result = blocking(move || diagnostics::parse_if_with_model(provider, input, cancel)).await;
    state.end_run(&run_id);
    result
}

#[tauri::command]
async fn submit_if_model(
    state: State<'_, AppState>,
    run_id: String,
    input: String,
) -> Result<WorldSnapshot, String> {
    let provider = slot_with_key(&state.snapshot(), Slot::Structure);
    let cancel = state.start_run(&run_id);
    let draft_result = blocking(move || diagnostics::parse_if_with_model(provider, input, cancel)).await;
    let draft = match draft_result {
        Ok(draft) => draft,
        Err(error) => { state.end_run(&run_id); return Err(error); }
    };
    let world = state.world.lock().expect("world state poisoned");
    let worker = world.as_ref().ok_or_else(|| "当前没有打开的世界".to_owned())?;
    let result = worker.submit_if_draft(draft);
    state.end_run(&run_id);
    result
}

#[tauri::command]
fn cancel_run(state: State<'_, AppState>, run_id: String) {
    if let Some(flag) = state.runs.lock().expect("runs poisoned").get(&run_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/* ---------- 会话：把世界资产播种成一个新世界（docs/10 §3） ---------- */

/// 世界文件放在应用数据目录下——**和 `library.db` 分开放**：
/// 世界库是「材料」，`worlds/` 是「历史」，两者的生命周期不同。
fn worlds_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("worlds");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建世界目录失败：{e}"))?;
    Ok(dir)
}

/// 选一个世界资产，建一个新会话。
///
/// 顺序是刻意的：**先把世界写出来，再落会话引用**。反过来的话，建世界失败就会在库里
/// 留下一条指向不存在文件的会话——那种脏数据没法从界面上看出来。
///
/// docs/10 §3：新建会话必须先选世界，所以这里没有「建一个空世界」的入口。
/// 想要空世界要走「手动撰写」——那也是一个资产，只是 payload 里没有角色与设定。
#[tauri::command]
async fn create_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    world_id: String,
    label: Option<String>,
) -> Result<SessionView, String> {
    let material = {
        let library = state.library.lock().expect("library poisoned");
        session::load(&library, &AssetId::new(world_id))?
    };
    let dir = worlds_dir(&app)?;
    let (handle, view, reference) = session::create(material, &dir, label)?;
    {
        let library = state.library.lock().expect("library poisoned");
        library.attach_session(&reference).map_err(|e| e.to_string())?;
    }
    *state.world.lock().expect("world state poisoned") = Some(handle.world);
    let _ = app.emit("world://opened", &view.snapshot);
    Ok(view)
}

/// 某个世界已有的会话。新建会话前用它提示「这个世界已经有会话了」。
#[tauri::command]
fn list_sessions(state: State<'_, AppState>, world_id: String) -> Result<Vec<SessionRef>, String> {
    let library = state.library.lock().expect("library poisoned");
    session::sessions_of(&library, &AssetId::new(world_id))
}

/// 恢复一个已有会话：重启之后回到上一次的进度。
#[tauri::command]
async fn open_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionView, String> {
    let (handle, view) = {
        let library = state.library.lock().expect("library poisoned");
        session::resume(&library, &session_id)?
    };
    *state.world.lock().expect("world state poisoned") = Some(handle.world);
    let _ = app.emit("world://opened", &view.snapshot);
    Ok(view)
}

#[tauri::command]
async fn open_world(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<WorldSnapshot, String> {
    let handle = WorldWorker::open(PathBuf::from(path))?;
    let snapshot = handle.snapshot.clone();
    *state.world.lock().expect("world state poisoned") = Some(handle.world);
    let _ = app.emit("world://opened", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn submit_if(state: State<'_, AppState>, input: String) -> Result<WorldSnapshot, String> {
    let world = state.world.lock().expect("world state poisoned");
    world
        .as_ref()
        .ok_or_else(|| "当前没有打开的世界".to_owned())?
        .submit_if(input)
}

#[tauri::command]
fn confirm_if(state: State<'_, AppState>, input: Option<String>) -> Result<WorldSnapshot, String> {
    let world = state.world.lock().expect("world state poisoned");
    world
        .as_ref()
        .ok_or_else(|| "当前没有打开的世界".to_owned())?
        .confirm_if(input)
}

#[tauri::command]
fn reinterpret_if(state: State<'_, AppState>) -> Result<WorldSnapshot, String> {
    let world = state.world.lock().expect("world state poisoned");
    world
        .as_ref()
        .ok_or_else(|| "当前没有打开的世界".to_owned())?
        .reinterpret_if()
}

#[tauri::command]
fn cancel_if(state: State<'_, AppState>) -> Result<WorldSnapshot, String> {
    let world = state.world.lock().expect("world state poisoned");
    world
        .as_ref()
        .ok_or_else(|| "当前没有打开的世界".to_owned())?
        .cancel_if()
}

#[tauri::command]
fn parse_if(input: String) -> Result<IfDraft, String> {
    if_parser::parse(&input)
}

#[tauri::command]
fn import_world_json(input: String) -> Result<importer::ImportedWorld, String> {
    importer::parse_json(&input)
}

/// 从文件导入：前端把文件读成 base64 传进来，这里自动识别 PNG 或 JSON。
#[tauri::command]
fn import_world_file(file_name: String, data_base64: String) -> Result<importer::ImportedWorld, String> {
    let bytes = importer::decode_base64(&data_base64)?;
    importer::parse_bytes(&bytes, Some(&file_name))
}

/* ---------- 世界库：世界资产的持久化（docs/10 §2） ---------- */

/// 导入结果写入世界库。
///
/// 来源身份键相同（同一张卡，哪怕版本/正文/文件名变了）时落在**同一个资产**上并
/// 按来源整组替换条目；否则新建一个。判据见 `library::source_key_of`。
fn save_import(
    world: &importer::ImportedWorld,
    raw: &str,
    guard: &mut Library,
) -> Result<AssetChange, String> {
    let existing = guard
        .asset_of_source(&library::source_key_of(world))
        .map_err(|e| e.to_string())?;
    let draft = library::draft_from_import(world, raw, existing)?;
    let asset = guard.save_asset(draft).map_err(|e| e.to_string())?;
    Ok(AssetChange {
        detail: AssetDetail::load(guard, &asset.id)?,
        assets: library::list(guard)?,
    })
}

#[tauri::command]
fn list_worlds(state: State<'_, AppState>) -> Result<Vec<AssetSummary>, String> {
    library::list(&state.library.lock().expect("library poisoned"))
}

#[tauri::command]
fn get_world_asset(state: State<'_, AppState>, id: String) -> Result<AssetDetail, String> {
    AssetDetail::load(&state.library.lock().expect("library poisoned"), &AssetId::new(id))
}

/// 删除资产。被会话引用时会被拒绝，并说明有几个会话在引用它（docs/10 §2）。
#[tauri::command]
fn delete_world_asset(state: State<'_, AppState>, id: String) -> Result<Vec<AssetSummary>, String> {
    let mut guard = state.library.lock().expect("library poisoned");
    guard.delete_asset(&AssetId::new(id)).map_err(|e| e.to_string())?;
    library::list(&guard)
}

#[tauri::command]
fn import_world_to_library(state: State<'_, AppState>, input: String) -> Result<AssetChange, String> {
    let world = importer::parse_json(&input)?;
    let mut guard = state.library.lock().expect("library poisoned");
    save_import(&world, &input, &mut guard)
}

#[tauri::command]
fn import_world_file_to_library(
    state: State<'_, AppState>,
    file_name: String,
    data_base64: String,
) -> Result<AssetChange, String> {
    let bytes = importer::decode_base64(&data_base64)?;
    let world = importer::parse_bytes(&bytes, Some(&file_name))?;
    // 来源附件存的是**原始 JSON**（PNG 卡取内嵌那段），不是 PNG 二进制：
    // 日后重导要的是可读材料，二进制对审阅没有价值。
    let raw = importer::raw_json(&bytes)?;
    let mut guard = state.library.lock().expect("library poisoned");
    save_import(&world, &raw, &mut guard)
}

/// 手动撰写一个新世界。主体、设定、规则在世界库界面里补（docs/10 §2）。
#[tauri::command]
fn create_written_world(
    state: State<'_, AppState>,
    name: String,
    genre: Option<String>,
    summary: Option<String>,
) -> Result<AssetChange, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("世界名称不能为空".into());
    }
    let mut guard = state.library.lock().expect("library poisoned");
    let draft = library::draft_written(
        name,
        genre.as_deref().unwrap_or(""),
        summary.as_deref().unwrap_or(""),
        None,
    );
    let asset = guard.save_asset(draft).map_err(|e| e.to_string())?;
    Ok(AssetChange {
        detail: AssetDetail::load(&guard, &asset.id)?,
        assets: library::list(&guard)?,
    })
}

#[tauri::command]
fn close_world(app: tauri::AppHandle, state: State<'_, AppState>) {
    state.world.lock().expect("world state poisoned").take();
    let _ = app.emit("world://closed", ());
}

#[tauri::command]
fn get_world_snapshot(state: State<'_, AppState>) -> Result<WorldSnapshot, String> {
    let world = state.world.lock().expect("world state poisoned");
    world
        .as_ref()
        .ok_or_else(|| "当前没有打开的世界".to_owned())?
        .snapshot()
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let settings_path = dir.join("settings.json");
            let settings = AppSettings::load(&settings_path);
            // 世界库与设置放在同一处。打不开就是硬错误：一个「装作已经存好了」
            // 的世界库比一个起不来的应用更糟。
            let library = Library::open(dir.join("library.db"))?;
            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_path,
                library: Mutex::new(library),
                runs: Mutex::new(HashMap::new()),
                world: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_api_key,
            test_llm,
            probe_jev,
            test_judge,
            parse_if_model,
            submit_if_model,
            cancel_run,
            create_session,
            list_sessions,
            open_session,
            open_world,
            submit_if,
            confirm_if,
            reinterpret_if,
            cancel_if,
            parse_if,
            import_world_json,
            import_world_file,
            list_worlds,
            get_world_asset,
            delete_world_asset,
            import_world_to_library,
            import_world_file_to_library,
            create_written_world,
            close_world,
            get_world_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("IF 启动失败");
}
