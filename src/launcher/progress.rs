// src/launcher/progress.rs

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
