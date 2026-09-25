//! 密钥存系统钥匙串（docs/12 §6）。前端只能提交或清除，永远读不到明文。
//!
//! 开发便利：Jev 密钥在钥匙串里没有时，依次回退到环境变量 `JEV_KEY`
//! 和仓库根的 `jevkey` 文件（已 gitignore，只用于开发，docs/14 §9）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const SERVICE: &str = "io.github.wep56.if";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySlot {
    Structure,
    Narrative,
    Jev,
}

impl KeySlot {
    fn account(self) -> &'static str {
        match self {
            KeySlot::Structure => "llm.structure",
            KeySlot::Narrative => "llm.narrative",
            KeySlot::Jev => "jev",
        }
    }
}

/// 密钥来自哪里。前端据此显示"已保存"之类的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    Keyring,
    Env,
    DevFile,
    None,
}

fn entry(slot: KeySlot) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, slot.account()).map_err(|e| format!("无法访问系统钥匙串：{e}"))
}

pub fn set(slot: KeySlot, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return clear(slot);
    }
    entry(slot)?.set_password(key).map_err(|e| format!("写入钥匙串失败：{e}"))
}

pub fn clear(slot: KeySlot) -> Result<(), String> {
    match entry(slot)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("删除钥匙串条目失败：{e}")),
    }
}

pub fn get(slot: KeySlot) -> (Option<String>, KeySource) {
    if let Ok(e) = entry(slot) {
        if let Ok(key) = e.get_password() {
            if !key.trim().is_empty() {
                return (Some(key), KeySource::Keyring);
            }
        }
    }
    if slot == KeySlot::Jev {
        if let Ok(key) = std::env::var("JEV_KEY") {
            if !key.trim().is_empty() {
                return (Some(key.trim().to_owned()), KeySource::Env);
            }
        }
        if let Some(key) = dev_file() {
            return (Some(key), KeySource::DevFile);
        }
    }
    (None, KeySource::None)
}

/// 从当前目录向上找 `jevkey`。`tauri dev` 的工作目录在 crates/if-app，所以要往上找。
fn dev_file() -> Option<String> {
    let mut dir: Option<PathBuf> = std::env::current_dir().ok();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(d.join("jevkey")) {
            let key = text.trim().to_owned();
            if !key.is_empty() {
                return Some(key);
            }
        }
        dir = d.parent().map(PathBuf::from);
    }
    None
}
