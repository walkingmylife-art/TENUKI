// src/ui/container.rs

use crate::config::GameTextOptions;
use crate::file_translate::commands::FileTranslateUiCommand;
use crate::file_translate::state::FileTranslateState;
use crate::launcher::app_config::ModelConfig;
use crate::messages::{InputAnalysisSnapshot, LogLevel, ModelCandidate};
use anyhow::Result;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;

const MAX_INPUT_HISTORY: usize = 500;

fn default_occurrences() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LeftPanelTab {
    #[default]
    TenukiLog,
    ServerLog,
    Dictionary,
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAnalysisRecord {
    pub id: u64,
    pub timestamp: String,
    pub snapshot: InputAnalysisSnapshot,
    #[serde(default = "default_occurrences")]
    pub occurrences: u32,
    pub pickup: bool,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LangPanelTab {
    #[default]
    Tgt,
    Ui,
    Network,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub message: String,
    pub level: LogLevel,
}

impl LogEntry {
    pub fn format(&self) -> String {
        format!("[{}] {}", self.timestamp, self.message)
    }
}

#[derive(Default)]
pub struct UiDisplayData {
    pub tenuki_logs: VecDeque<LogEntry>,
    pub llama_logs: VecDeque<LogEntry>,
    pub translation_logs: VecDeque<LogEntry>,
    pub file_translate_logs: VecDeque<LogEntry>,
    pub input_snapshot: Option<InputAnalysisSnapshot>,
    pub input_records: VecDeque<InputAnalysisRecord>,
    pub work_result_title: String,
    pub work_result_text: String,
    pub work_result_error: bool,
    pub work_running: bool,
    pub file_translate_done: usize,
    pub file_translate_total: usize,
    pub file_translate_error: bool,
    pub file_translate_status_text: String,
    pub dictionary_loaded: usize,
    pub dictionary_new: usize,
    pub dictionary_history: VecDeque<(String, String, String)>,
    pub dict_hits: usize,
    pub model_calls: usize,
    pub llama_running: bool,
    pub tenuki_running: bool,
    pub available_models: Vec<ModelCandidate>,
    pub selected_model: Option<PathBuf>,
    pub status_key: StatusKey,
    pub status_icon: StatusIcon,
    pub status_visible: bool,
    pub base_dir: PathBuf,
    pub src_lang: String,
    pub tgt_lang: String,
    pub ui_lang: String,
    pub server_host: String,
    pub server_port: u16,
    pub local_ip: String,
    pub custom_lang_name: String,
    pub dict_slot: Option<String>,
    pub mode: String,
    pub game_text: GameTextOptions,
    pub profile: String,
    pub available_profiles: Vec<String>,
    pub vram_mb: f32,
    pub shared_mb: f32,
    pub tokens_per_second: f32,
}

#[derive(Default, Clone, Copy, PartialEq)]
pub enum StatusKey {
    #[default]
    None,
    Ready,
    Failed,
    Stopped,
    Starting,
    Stopping,
    Restarting,
    ConfigError,
}

#[derive(Default, Clone, Copy, PartialEq)]
pub enum StatusIcon {
    #[default]
    None,
    Spinner,
    Check,
    Warning,
}

impl StatusIcon {
    pub fn color(&self) -> egui::Color32 {
        match self {
            StatusIcon::None => egui::Color32::GRAY,
            StatusIcon::Spinner => egui::Color32::from_rgb(100, 150, 255),
            StatusIcon::Check => egui::Color32::from_rgb(0, 200, 100),
            StatusIcon::Warning => egui::Color32::from_rgb(255, 200, 100),
        }
    }
}

#[derive(Default)]
pub struct UiState {
    pub log_panel_tab: LeftPanelTab,
    pub immediate_apply: bool,
    pub show_lang_panel: bool,
    pub show_game_text_panel: bool,
    pub lang_panel_tab: LangPanelTab,
    pub lang_panel_anchor: egui::Pos2,
    pub game_text_edit: GameTextOptions,
    pub custom_tgt_val_buf: String,
    pub custom_tgt_name_buf: String,
    pub pending_tgt_lang: Option<String>,
    /// 言語切替時に現在の dict_slot が次ターゲットと一致しない場合に表示する確認ダイアログの保留状態。
    /// (src, tgt, tgt_name, current_slot)
    pub dict_check_pending: Option<(String, String, Option<String>, String)>,
    pub network_host_buf: String,
    pub network_port_buf: String,
    pub selected_input_record_id: Option<u64>,
    pub pickup_note_edit: String,
    pub file_translate: FileTranslateState,
}

#[derive(Default)]
pub struct UiCommands {
    pub start_backend: bool,
    pub stop_backend: bool,
    pub restart_backend: bool,
    pub set_lang_pair: Option<(String, String, Option<String>, Option<String>)>,
    pub set_ui_lang: Option<String>,
    pub set_dict_slot: Option<String>,
    pub create_new_dict_slot: bool,
    pub exit_app: bool,
    pub set_game_text_options: Option<GameTextOptions>,
    pub set_profile: Option<String>,
    pub select_model: Option<ModelConfig>,
    pub set_input_pickup: Option<(u64, bool)>,
    pub set_input_pickup_note: Option<(u64, String)>,
    pub refresh_pickup_preview: bool,
    pub set_server_port: Option<u16>,
    pub set_server_host: Option<String>,
    pub file_translate_commands: Vec<FileTranslateUiCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogSource {
    Tenuki,
    LlamaCpp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessType {
    InferenceEngine,
    Tenuki,
}

pub struct UiContainer {
    pub display: UiDisplayData,
    pub state: UiState,
}

impl UiContainer {
    pub fn new() -> Self {
        Self::with_base_dir(PathBuf::new())
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        let default_file_translate_root = if base_dir.join("Text").is_dir() {
            Some(base_dir.join("Text"))
        } else {
            Some(base_dir.clone())
        };

        let state = UiState {
            file_translate: FileTranslateState::with_root(default_file_translate_root.clone()),
            ..Default::default()
        };

        Self {
            display: UiDisplayData {
                base_dir,
                src_lang: "en".to_string(),
                tgt_lang: "ja".to_string(),
                ui_lang: "ja".to_string(),
                dictionary_loaded: 0,
                dictionary_new: 0,
                mode: "game".to_string(),
                game_text: GameTextOptions::default(),
                ..Default::default()
            },
            state,
        }
    }

    pub fn add_log(&mut self, source: LogSource, msg: String, level: LogLevel, timestamp: String) {
        let is_word_log = msg.starts_with("[XUnity]") || msg.starts_with("[Model]");
        let is_server_log = msg.starts_with("TENUKI translation server ")
            || msg.starts_with("Translation server ")
            || msg.starts_with("llama-server ");

        let entry = LogEntry {
            timestamp,
            message: msg,
            level,
        };
        match source {
            LogSource::Tenuki => {
                if is_word_log {
                    return;
                }
                if is_server_log {
                    self.display.translation_logs.push_back(entry);
                    while self.display.translation_logs.len() > 1000 {
                        self.display.translation_logs.pop_front();
                    }
                } else {
                    self.display.tenuki_logs.push_back(entry);
                    while self.display.tenuki_logs.len() > 500 {
                        self.display.tenuki_logs.pop_front();
                    }
                }
            }
            LogSource::LlamaCpp => {
                self.display.llama_logs.push_back(entry);
                while self.display.llama_logs.len() > 2000 {
                    self.display.llama_logs.pop_front();
                }
            }
        }
    }

    pub fn add_dictionary_entry(
        &mut self,
        timestamp: String,
        original: String,
        translated: String,
    ) {
        self.display.dictionary_new += 1;
        let _ = (timestamp, original, translated);
    }

    pub fn add_dictionary_log_entry(
        &mut self,
        timestamp: String,
        original: String,
        translated: String,
    ) {
        self.display
            .dictionary_history
            .push_back((timestamp, original, translated));
        while self.display.dictionary_history.len() > 100 {
            self.display.dictionary_history.pop_front();
        }
    }

    pub fn set_dictionary_loaded(&mut self, count: usize) {
        self.display.dictionary_loaded = count;
    }

    pub fn update_statistics(&mut self, dict_hits: usize, model_calls: usize) {
        self.display.dict_hits += dict_hits;
        self.display.model_calls = model_calls;
    }

    pub fn update_process_status(&mut self, proc_type: ProcessType, running: bool) {
        match proc_type {
            ProcessType::InferenceEngine => {
                self.display.llama_running = running;
                if !running {
                    self.display.vram_mb = 0.0;
                    self.display.shared_mb = 0.0;
                    self.display.tokens_per_second = 0.0;
                }
            }
            ProcessType::Tenuki => self.display.tenuki_running = running,
        }
    }

    pub fn update_available_models(&mut self, models: Vec<ModelCandidate>) {
        let selected_still_valid = self
            .display
            .selected_model
            .as_ref()
            .is_some_and(|selected| models.iter().any(|c| &c.path == selected));
        if !selected_still_valid {
            self.display.selected_model = None;
        }
        self.display.available_models = models;
    }

    pub fn update_selected_model(&mut self, model: Option<PathBuf>) {
        self.display.selected_model = model;
    }

    pub fn set_status(&mut self, key: StatusKey, icon: StatusIcon, visible: bool) {
        self.display.status_key = key;
        self.display.status_icon = icon;
        self.display.status_visible = visible;
    }

    pub fn set_work_result(&mut self, title: String, text: String, is_error: bool) {
        self.display.work_result_title = title;
        self.display.work_result_text = text;
        self.display.work_result_error = is_error;
    }

    pub fn set_work_running(&mut self, running: bool) {
        self.display.work_running = running;
    }

    pub fn reset_file_translate_logs(&mut self, lines: impl IntoIterator<Item = String>) {
        self.display.file_translate_logs.clear();
        for line in lines {
            self.push_file_translate_log(LogEntry {
                timestamp: crate::messages::current_timestamp(),
                message: line,
                level: LogLevel::Info,
            });
        }
    }

    pub fn append_file_translate_log(&mut self, line: String, level: LogLevel) {
        self.push_file_translate_log(LogEntry {
            timestamp: crate::messages::current_timestamp(),
            message: line,
            level,
        });
    }

    pub fn reset_file_translate_progress(&mut self) {
        self.display.file_translate_done = 0;
        self.display.file_translate_total = 0;
        self.display.file_translate_error = false;
        self.display.file_translate_status_text.clear();
    }

    pub fn update_file_translate_progress(&mut self, done: usize, total: usize) {
        self.display.file_translate_done = done;
        self.display.file_translate_total = total;
        self.display.file_translate_error = false;
    }

    pub fn set_file_translate_status_text(&mut self, text: String) {
        self.display.file_translate_status_text = text;
    }

    pub fn finish_file_translate_progress(&mut self, is_error: bool) {
        self.display.file_translate_error = is_error;
    }

    fn push_file_translate_log(&mut self, entry: LogEntry) {
        self.display.file_translate_logs.push_back(entry);
        while self.display.file_translate_logs.len() > 4000 {
            self.display.file_translate_logs.pop_front();
        }
    }

    pub fn update_src_lang(&mut self, lang: &str) {
        self.display.src_lang = lang.to_string();
    }

    pub fn update_tgt_lang(&mut self, lang: &str, name: Option<&str>) {
        self.display.tgt_lang = lang.to_string();
        let is_preset = crate::config::is_target_language_preset(lang);
        if is_preset {
            self.display.custom_lang_name.clear();
            self.state.custom_tgt_val_buf.clear();
            self.state.custom_tgt_name_buf.clear();
        } else {
            let n = name.unwrap_or(&self.display.custom_lang_name).to_string();
            self.display.custom_lang_name = n.clone();
            self.state.custom_tgt_val_buf = lang.to_string();
            self.state.custom_tgt_name_buf = n;
        }
    }

    pub fn update_dict_slot(&mut self, slot: Option<String>) {
        self.display.dict_slot = slot;
    }

    pub fn load_input_records(&mut self) -> Result<()> {
        self.display.input_records.clear();
        self.display.input_snapshot = None;

        self.state.selected_input_record_id = None;
        self.state.pickup_note_edit = self
            .display
            .input_records
            .back()
            .map(|record| record.note.clone())
            .unwrap_or_default();

        Ok(())
    }

    pub fn save_input_records(&self) -> Result<()> {
        Ok(())
    }

    pub fn update_input_analysis(&mut self, snapshot: InputAnalysisSnapshot) -> bool {
        let should_persist = !snapshot.result_stale
            && !snapshot.raw_text.trim().is_empty()
            && snapshot.model_calls > 0;
        self.display.input_snapshot = Some(snapshot.clone());
        self.state.selected_input_record_id = None;

        if should_persist {
            let timestamp = crate::messages::current_timestamp();
            if let Some(last_record) = self.display.input_records.back_mut() {
                if last_record.snapshot == snapshot {
                    last_record.timestamp = timestamp;
                    last_record.occurrences = last_record.occurrences.saturating_add(1);
                    self.state.pickup_note_edit = last_record.note.clone();
                    return true;
                }
            }

            let next_id = self
                .display
                .input_records
                .back()
                .map(|record| record.id.saturating_add(1))
                .unwrap_or(1);

            self.display.input_records.push_back(InputAnalysisRecord {
                id: next_id,
                timestamp,
                snapshot,
                occurrences: 1,
                pickup: false,
                note: String::new(),
            });

            while self.display.input_records.len() > MAX_INPUT_HISTORY {
                self.display.input_records.pop_front();
            }
        }

        self.state.pickup_note_edit = self
            .display
            .input_records
            .back()
            .map(|record| record.note.clone())
            .unwrap_or_default();

        should_persist
    }

    pub fn set_input_pickup(&mut self, id: u64, pickup: bool) -> bool {
        let latest_record_id = self.display.input_records.back().map(|record| record.id);
        if let Some(record) = self
            .display
            .input_records
            .iter_mut()
            .find(|record| record.id == id)
        {
            if record.pickup == pickup {
                return false;
            }
            record.pickup = pickup;
            if self.state.selected_input_record_id == Some(id) || latest_record_id == Some(id) {
                self.state.pickup_note_edit = record.note.clone();
            }
            return true;
        }

        false
    }

    pub fn update_input_pickup_note(&mut self, id: u64, note: String) -> bool {
        let latest_record_id = self.display.input_records.back().map(|record| record.id);
        if let Some(record) = self
            .display
            .input_records
            .iter_mut()
            .find(|record| record.id == id)
        {
            if record.note == note {
                return false;
            }

            record.note = note.clone();
            if self.state.selected_input_record_id == Some(id) || latest_record_id == Some(id) {
                self.state.pickup_note_edit = note;
            }
            return true;
        }

        false
    }

    pub fn update_ui_lang(&mut self, lang: &str) {
        self.display.ui_lang = lang.to_string();
    }

    pub fn update_mode(&mut self, mode: &str) {
        self.display.mode = mode.to_string();
    }

    pub fn refresh_available_profiles(&mut self, profiles_dir: &std::path::Path) {
        let mut names: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(profiles_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "toml").unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let normalized = match stem {
                            "default" => "game",
                            other => other,
                        };
                        if !names.iter().any(|name| name == normalized) {
                            names.push(normalized.to_string());
                        }
                    }
                }
            }
        }
        names.sort();
        self.display.available_profiles = names;
    }

    pub fn update_profile(&mut self, profile: &str) {
        self.display.profile = profile.to_string();
    }

    pub fn update_game_text_options(&mut self, options: GameTextOptions) {
        self.display.game_text = options;
        if !self.state.show_game_text_panel {
            self.state.game_text_edit = options;
        }
    }

    pub fn update_server_metrics(
        &mut self,
        vram_mb: Option<f32>,
        shared_mb: Option<f32>,
        tokens_per_second: Option<f32>,
    ) {
        if let Some(v) = vram_mb {
            self.display.vram_mb = v;
        }
        if let Some(v) = shared_mb {
            self.display.shared_mb = v;
        }
        if let Some(v) = tokens_per_second {
            self.display.tokens_per_second = v;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> UiCommands {
        let mut commands = UiCommands::default();
        let display = &self.display;
        let state = &mut self.state;
        crate::ui::normal::show_normal_ui(ctx, display, state, &mut commands);
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::{LogSource, UiContainer};
    use crate::messages::{InputAnalysisSnapshot, LogLevel};

    #[test]
    fn skips_persisting_dictionary_only_snapshots() {
        let mut ui = UiContainer::new();
        let snapshot = InputAnalysisSnapshot {
            raw_text: "known".to_string(),
            extracted_text: "known".to_string(),
            visible_text: "known".to_string(),
            model_inputs: vec!["known".to_string()],
            final_output: Some("既存".to_string()),
            result_stale: false,
            dict_hits: 1,
            model_calls: 0,
        };

        let persisted = ui.update_input_analysis(snapshot);

        assert!(!persisted);
        assert!(ui.display.input_records.is_empty());
        assert!(ui.display.input_snapshot.is_some());
    }

    #[test]
    fn merges_duplicate_saved_snapshots() {
        let mut ui = UiContainer::new();
        let snapshot = InputAnalysisSnapshot {
            raw_text: "new text".to_string(),
            extracted_text: "new text".to_string(),
            visible_text: "new text".to_string(),
            model_inputs: vec!["new text".to_string()],
            final_output: Some("新しい訳".to_string()),
            result_stale: false,
            dict_hits: 0,
            model_calls: 1,
        };

        assert!(ui.update_input_analysis(snapshot.clone()));
        assert!(ui.update_input_analysis(snapshot));

        assert_eq!(ui.display.input_records.len(), 1);
        assert_eq!(ui.display.input_records[0].occurrences, 2);
    }

    #[test]
    fn keeps_dict_hit_logs_in_tenuki_panel() {
        let mut ui = UiContainer::new();

        ui.add_log(
            LogSource::Tenuki,
            "[TENUKI] (0.01s) Attack+10% -> 攻撃+10%".to_string(),
            LogLevel::Success,
            "12:00:00".to_string(),
        );

        assert!(ui.display.translation_logs.is_empty());
        assert_eq!(ui.display.tenuki_logs.len(), 1);
    }

    #[test]
    fn routes_server_status_logs_to_server_panel() {
        let mut ui = UiContainer::new();

        ui.add_log(
            LogSource::Tenuki,
            "TENUKI translation server listening on http://127.0.0.1:14371".to_string(),
            LogLevel::Info,
            "12:00:00".to_string(),
        );

        assert_eq!(ui.display.translation_logs.len(), 1);
        assert!(ui.display.tenuki_logs.is_empty());
    }

    #[test]
    fn save_input_records_does_not_create_history_file() {
        let dir = std::env::temp_dir().join(format!(
            "tenuki_ui_history_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        let mut ui = UiContainer::with_base_dir(dir.clone());
        let snapshot = InputAnalysisSnapshot {
            raw_text: "new text".to_string(),
            extracted_text: "new text".to_string(),
            visible_text: "new text".to_string(),
            model_inputs: vec!["new text".to_string()],
            final_output: Some("新しい訳".to_string()),
            result_stale: false,
            dict_hits: 0,
            model_calls: 1,
        };
        assert!(ui.update_input_analysis(snapshot));

        ui.save_input_records().expect("save should be a no-op");

        let history_path = dir
            .join("dicts")
            .join("ja")
            .join("text")
            .join("input_mode_history.json");
        assert!(!history_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

impl Default for UiContainer {
    fn default() -> Self {
        Self::new()
    }
}
