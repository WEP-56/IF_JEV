//! 应用级设置（docs/11 §6）。不含密钥：密钥在系统钥匙串里（见 [`crate::secrets`]）。
//!
//! 世界设置（种子、叙事风格、视角、机制步长、导演风格、模式）属于每个世界，
//! 存在世界文件里，不在这里。

use std::path::{Path, PathBuf};
use std::time::Duration;

use if_agent::{ApiKind, ProviderSettings};
use if_judge::JevConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// 结构模型：解析、候选、计划、回收。
    pub structure: ProviderSettings,
    /// 叙事模型：正文。可以与结构模型指向同一个模型。
    pub narrative: ProviderSettings,
    pub jev: JevSettings,
    pub engine: EngineSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JevSettings {
    pub endpoint: String,
    /// 请求用的别名；判定记录存响应回显的快照（docs/07 §6）。
    pub model: String,
    pub timeout_secs: u64,
}

/// 引擎策略的全局默认值（docs/11 §6）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    /// 一致性严格度，0–100（D10：线性缩放到各阈值）。
    pub strictness: u8,
    /// 因果深度【初始值 3】（docs/06 §4）。
    pub causal_depth: u8,
    /// 同一节拍的重试上限【初始值 2】（docs/04 §4）。
    pub beat_retries: u8,
    /// 裁定卡自动确认，默认关闭（D3）。
    pub auto_confirm_ruling: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            structure: default_slot("结构模型", Some(0.3)),
            narrative: default_slot("叙事模型", Some(0.85)),
            jev: JevSettings::default(),
            engine: EngineSettings::default(),
        }
    }
}

fn default_slot(name: &str, temperature: Option<f64>) -> ProviderSettings {
    ProviderSettings {
        name: name.into(),
        api: ApiKind::ChatCompletions,
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: String::new(),
        model: String::new(),
        max_tokens: Some(4096),
        temperature,
        reasoning_effort: None,
        prompt_cache: false,
        replay_encrypted_reasoning: false,
    }
}

impl Default for JevSettings {
    fn default() -> Self {
        JevSettings {
            endpoint: if_judge::jev::DEFAULT_ENDPOINT.into(),
            model: if_judge::jev::DEFAULT_MODEL.into(),
            timeout_secs: 30,
        }
    }
}

impl Default for EngineSettings {
    fn default() -> Self {
        EngineSettings { strictness: 70, causal_depth: 3, beat_retries: 2, auto_confirm_ruling: false }
    }
}

impl JevSettings {
    pub fn config(&self, api_key: String) -> JevConfig {
        let mut cfg = JevConfig::new(api_key);
        cfg.endpoint = self.endpoint.clone();
        cfg.model = self.model.clone();
        cfg.timeout = Duration::from_secs(self.timeout_secs.max(1));
        cfg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    Structure,
    Narrative,
}

impl AppSettings {
    pub fn slot(&self, slot: Slot) -> &ProviderSettings {
        match slot {
            Slot::Structure => &self.structure,
            Slot::Narrative => &self.narrative,
        }
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// 先写临时文件再改名，避免写到一半崩溃留下损坏的设置。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp: PathBuf = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_never_writes_keys() {
        let dir = std::env::temp_dir().join(format!("if-app-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let mut s = AppSettings::default();
        s.structure.api_key = "sk-secret".into();
        s.structure.model = "m1".into();
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("sk-secret"));
        let loaded = AppSettings::load(&path);
        assert_eq!(loaded.structure.model, "m1");
        assert_eq!(loaded.structure.api_key, "");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_or_partial_file_falls_back_to_defaults() {
        let s = AppSettings::load(Path::new("definitely/missing.json"));
        assert_eq!(s.jev.model, "typesafe/jev-1.13");
        let partial: AppSettings = serde_json::from_str(r#"{"engine":{"strictness":10}}"#).unwrap();
        assert_eq!(partial.engine.strictness, 10);
        assert_eq!(partial.engine.causal_depth, 3);
    }
}
