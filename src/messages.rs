//! スレッド間通信メッセージの定義
//!
//! GUI ↔ バックエンド 間の通信で使用するコマンドとイベントを定義する。

use crate::config::StructuralOptions;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputAnalysisSnapshot {
    pub raw_text: String,
    pub extracted_text: String,
    pub visible_text: String,
    pub model_inputs: Vec<String>,
    pub final_output: Option<String>,
    pub result_stale: bool,
    pub dict_hits: usize,
    pub model_calls: usize,
}

// ============================================================
// GUI → バックエンド コマンド
// ============================================================

#[derive(Debug, Clone)]
pub enum FrontendCommand {
    Start,
    Stop,
    Restart,
    /// dict_slot=Some(path) のとき指定スロットを使用、None のとき新規スロットを作成
    SetLanguagePair { src: String, tgt: String, tgt_name: Option<String>, dict_slot: Option<String> },
    SetDictSlot(Option<String>),
    SetProfile(String),
    SetModel(String),
    UpdateSettings {
        structural: Option<StructuralOptions>,
        server_port: Option<u16>,
        server_host: Option<String>,
    },
}

impl FrontendCommand {
    pub fn is_empty_update(&self) -> bool {
        match self {
            FrontendCommand::UpdateSettings {
                structural,
                server_port,
                server_host,
            } => {
                structural.is_none()
                    && server_port.is_none()
                    && server_host.is_none()
            }
            _ => false,
        }
    }
}

// ============================================================
// バックエンド → GUI イベント
// ============================================================

#[derive(Debug, Clone)]
pub enum BackendEvent {
    Log(LogSource, String, LogLevel, String),
    DictionaryLoaded(usize),
    DictionaryNewEntry(String, String, String),
    DictionaryLogEntry(String, String, String),
    StatisticsUpdate(usize, usize),
    InputAnalysisUpdated(InputAnalysisSnapshot),
    WorkResult {
        title: String,
        text: String,
        is_error: bool,
    },
    ProcessStatus(ProcessType, bool),
    BackendReady {
        engine_success: bool,
        translator_success: bool,
    },
    AvailableModels(Vec<PathBuf>),
    /// authority resolved model を UI に通知する。AvailableModels とは分離して送信する。
    SelectedModelResolved(Option<PathBuf>),
    LanguageChanged(String),
    DictSlotChanged(String),
    ServerMetrics {
        vram_mb: Option<f32>,
        shared_mb: Option<f32>,
        tokens_per_second: Option<f32>,
    },
}

// ============================================================
// 補助列挙型
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogSource {
    Tenuki,
    LlamaCpp,
}

impl std::fmt::Display for LogSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogSource::Tenuki   => write!(f, "TENUKI"),
            LogSource::LlamaCpp => write!(f, "llama-cpp-2"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessType {
    InferenceEngine,
    Tenuki,
}

// ============================================================
// タイムスタンプ生成
// ============================================================

pub fn current_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = now.as_secs() % 86400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}
