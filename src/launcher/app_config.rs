// src/launcher/app_config.rs

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Launcher 専用設定（launcher_config.toml）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub(crate) backend: String,
    pub(crate) server: ServerConfig,
    pub(crate) model: ModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) ctx_size: u32,
    pub(crate) ngl: u32,
    pub(crate) batch_size: u32,
    pub(crate) ubatch_size: u32,
    pub(crate) parallel_slots: u32,
    pub(crate) cont_batching: bool,
    pub(crate) extra_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub(crate) url: String,
    pub(crate) filename: String,
    pub(crate) expected_size: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backend: "cuda".to_string(),
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                ctx_size: 1024,
                ngl: 999,
                batch_size: 128,
                ubatch_size: 64,
                parallel_slots: 2,
                cont_batching: true,
                extra_args: vec!["--flash-attn".to_string(), "auto".to_string()],
            },
            model: ModelConfig {
                url: "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q6_K.gguf?download=true".to_string(),
                filename: "HY-MT1.5-1.8B-Q6_K.gguf".to_string(),
                expected_size: 0,
            },
        }
    }
}

impl AppConfig {
    /// ゲーム翻訳向けデフォルト（structural モード）
    pub fn game_default() -> Self {
        Self::default()
    }

    /// パススルーモード向けデフォルト（高スループット設定）
    pub fn passthrough_default() -> Self {
        Self {
            server: ServerConfig {
                ctx_size: 2048,
                batch_size: 256,
                ubatch_size: 128,
                parallel_slots: 4,
                ..Self::default().server
            },
            ..Self::default()
        }
    }

    /// translation_mode に応じたデフォルトを返す
    pub fn default_for_mode(translation_mode: &str) -> Self {
        if translation_mode == "passthrough" {
            Self::passthrough_default()
        } else {
            Self::game_default()
        }
    }

    /// launcher_config.toml を読み込む。存在しない場合は translation_mode に応じたデフォルトを書き出す。
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_with_mode(path, "structural")
    }

    pub fn load_with_mode(path: &Path, translation_mode: &str) -> Result<Self> {
        if !path.exists() {
            let config = Self::default_for_mode(translation_mode);
            config.save(path).with_context(|| format!("Failed to write {}", path.display()))?;
            return Ok(config);
        }
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }
}