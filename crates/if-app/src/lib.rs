//! IF 桌面应用入口：Tauri 命令与事件（docs/12 §4）。
//!
//! 目前接入的是设置、密钥和连通性诊断；回合相关的命令（`submit_if`、`continue_turn`……）
//! 随 `if-pipeline` 一起接入。

mod diagnostics;
mod secrets;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use diagnostics::{JudgeTestInput, JudgeTestResult, LlmTestResult};
use secrets::{KeySlot, KeySource};
use settings::{AppSettings, Slot};

struct AppState {
    settings: Mutex<AppSettings>,
    settings_path: PathBuf,
    runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
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
fn cancel_run(state: State<'_, AppState>, run_id: String) {
    if let Some(flag) = state.runs.lock().expect("runs poisoned").get(&run_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let settings_path = dir.join("settings.json");
            let settings = AppSettings::load(&settings_path);
            app.manage(AppState { settings: Mutex::new(settings), settings_path, runs: Mutex::new(HashMap::new()) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_api_key,
            test_llm,
            probe_jev,
            test_judge,
            cancel_run,
        ])
        .run(tauri::generate_context!())
        .expect("IF 启动失败");
}
