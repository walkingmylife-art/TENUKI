//! スレッド間通信メッセージの定義
//!
//! GUI ↔ バックエンド 間の通信で使用するコマンドとイベントを定義する。

use crate::config::GameTextOptions;
use crate::launcher::app_config::ModelConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// GUI/backend contract for the latest normal translation input analysis.
///
/// Fresh snapshots are produced only from the authority payload recorded when a
/// normal `/translate` request completes. Stale snapshots are clones of the
/// last saved snapshot with `result_stale` set; they are not recomputed from
/// mode, game-text options, or a processor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputAnalysisSnapshot {
    /// Original request text after newline normalization.
    pub raw_text: String,
    /// Text selected by the translation pipeline as analysis source.
    pub extracted_text: String,
    /// Human-readable source view recorded in the authority payload.
    pub visible_text: String,
    /// Model call inputs observed during the completed translation.
    pub model_inputs: Vec<String>,
    /// Final translated output for fresh snapshots; retained for stale replay.
    pub final_output: Option<String>,
    /// True when this is a replay after mode/language/game-text changes.
    pub result_stale: bool,
    /// Dictionary hits recorded by the completed translation.
    pub dict_hits: usize,
    /// Model calls recorded by the completed translation.
    pub model_calls: usize,
}

// ============================================================
// モデル候補（UI 表示 + commit 生成に使う metadata 付きリスト項目）
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum ModelCandidateKind {
    Known,
    Local,
}

#[derive(Debug, Clone)]
pub struct ModelCandidate {
    pub filename: String,
    pub path: PathBuf,
    pub size: u64,
    pub kind: ModelCandidateKind,
}

// ============================================================
// GUI → バックエンド コマンド
// ============================================================

#[derive(Debug, Clone)]
pub enum FrontendCommand {
    Start,
    Stop,
    Restart,
    /// `dict_slot` is already resolved and committed by the UI/preflight path.
    ///
    /// The backend adopts it into `config.toml` and reloads; it must not
    /// provision or infer a different authority slot from this command.
    /// dict_slot は上流で確定済みの commit 済み authority。backend は adopt して save するだけ。
    SetLanguagePair {
        src: String,
        tgt: String,
        tgt_name: Option<String>,
        dict_slot: String,
    },
    SetDictSlot(String),
    SetProfile(String),
    /// UI で確定した完全な ModelConfig authority object を backend へ渡す。
    /// backend は adopt して save するだけ。filename 単独渡し禁止。
    CommitModelSelection(ModelConfig),
    UpdateSettings {
        game_text: Option<GameTextOptions>,
        server_port: Option<u16>,
        server_host: Option<String>,
    },
}

impl FrontendCommand {
    pub fn is_empty_update(&self) -> bool {
        match self {
            FrontendCommand::UpdateSettings {
                game_text,
                server_port,
                server_host,
            } => game_text.is_none() && server_port.is_none() && server_host.is_none(),
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
    FileTranslateProgress {
        done: usize,
        total: usize,
    },
    FileTranslateLog {
        line: String,
        level: LogLevel,
    },
    StatisticsUpdate(usize, usize),
    /// Normal-translation input analysis update.
    ///
    /// `/list` must not emit this event. Mode, game-text, and language changes
    /// may emit only stale replay of the last saved snapshot.
    InputAnalysisUpdated(InputAnalysisSnapshot),
    WorkResult {
        title: String,
        text: String,
        is_error: bool,
    },
    StatusNotice {
        title: String,
        message: String,
    },
    ProcessStatus(ProcessType, bool),
    BackendReady {
        engine_success: bool,
        translator_success: bool,
    },
    /// models/ の .gguf 一覧。Known/Local 種別と metadata 付き。
    AvailableModels(Vec<ModelCandidate>),
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
            LogSource::Tenuki => write!(f, "TENUKI"),
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
