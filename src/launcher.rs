// src/launcher.rs

pub mod app_config;
mod app_launcher;
mod backend_detector;
mod config_preflight;
mod launcher_ui;
mod migration;
mod progress;
pub(crate) mod runtime_downloader;
mod translation_profile;

pub use app_launcher::{check_ready_detail, AppLauncher, CheckReadyReason};
pub use config_preflight::preflight_runtime_config_for_startup;
pub use launcher_ui::{show_launcher_screen, LauncherEntryIntent, LauncherStep, LauncherUiState};
pub use progress::LaunchProgress;

use std::path::PathBuf;

/// install_root を解決する。launcher_config.toml の権威位置。
/// - 配布 exe: TENUKI.exe のあるディレクトリ
/// - 開発 exe: target/debug または target/release の2つ上
pub fn resolve_install_root() -> PathBuf {
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    let exe_dir = exe_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    // target/debug/TENUKI.exe → system
    // target/release/TENUKI.exe → system
    if exe_dir
        .file_name()
        .map(|s| s == "debug" || s == "release")
        .unwrap_or(false)
    {
        if let Some(system) = exe_dir.parent().and_then(|t| t.parent()) {
            return system.to_path_buf();
        }
    }

    exe_dir
}
