// src/launcher/progress.rs

/// Launcher の実行モード。UI から呼び出し口を分けることで
/// 将来「runtime だけ更新」「model だけ差し替え」を個別に起動できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetupMode {
    /// 初回セットアップ / フル修復（runtime + model + verify）
    #[default]
    Full,
    /// runtime だけ取得・展開・verify し直す
    RepairRuntime,
    /// model だけ取得し直す
    RepairModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherStage {
    Directories,
    Gpu,
    Runtime,
    Model,
    Verify,
    Save,
    Complete,
    Error,
}

impl Default for LauncherStage {
    fn default() -> Self {
        LauncherStage::Directories
    }
}

#[derive(Debug, Clone)]
pub enum LaunchProgress {
    Stage(LauncherStage),
    Status(String),
    SubStatus(String),
    Progress(f32),
    Cancelled,
    Complete,
    Error(String),
}