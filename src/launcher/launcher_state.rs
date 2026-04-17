// src/launcher/launcher_state.rs
//
// 役割：AppLauncher が実際に使用した URL やパスをキャッシュする。
// - 権威設定（AppConfig）を上書きしない。
// - 起動時に前回成功した構成を復元し、ダウンロードや検証をスキップする。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 起動構成のキャッシュ
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LauncherState {
    /// 前回成功したバックエンド名（cuda, vulkan, rocm）
    pub backend: Option<String>,
    /// 前回成功したモデルファイル名
    pub model_filename: Option<String>,
    /// 前回成功した llama-server 実行ファイルのパス
    pub runtime_exe_path: Option<std::path::PathBuf>,
    /// 前回成功したモデルダウンロード URL
    pub model_url: Option<String>,
    /// 前回成功したバックエンドランタイムダウンロード URL
    pub backend_url: Option<String>,
}

impl LauncherState {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let state: LauncherState = serde_json::from_str(&content)?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}