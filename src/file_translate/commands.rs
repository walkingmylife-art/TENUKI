use super::types::{ColumnMode, HeaderMode};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum FileTranslateUiCommand {
    StartFileTranslateScan(PathBuf),
    SelectFileTranslateSource(PathBuf),
    SetFileTranslateColumnMode {
        file: PathBuf,
        column: usize,
        mode: ColumnMode,
    },
    SetFileTranslateHeaderMode {
        file: PathBuf,
        mode: HeaderMode,
    },
    RunFileTranslate,
    StopFileTranslate,
}
