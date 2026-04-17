// src/main.rs

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod backend;
mod config;
mod launcher;
mod logic;
mod messages;
mod ui;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use anyhow::anyhow;
use serde::Serialize;

use backend::processor::{ProcessorData, ProcessorFactory, TranslationMode};
use launcher::{show_launcher_screen, LaunchProgress, LauncherStep, LauncherUiState};
use messages::{BackendEvent, FrontendCommand, LogLevel};
use ui::container::{LogSource, ProcessType, StatusIcon, StatusKey, UiContainer};

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppMode {
    Launcher,
    Normal,
}

struct TenukiApp {
    mode: AppMode,
    base_dir: PathBuf,
    config_path: PathBuf,
    cached_model_check: Option<(bool, Instant)>,
    ui: UiContainer,
    command_tx: mpsc::Sender<FrontendCommand>,
    event_rx: mpsc::Receiver<BackendEvent>,
    event_tx: mpsc::Sender<BackendEvent>,
    command_rx: Option<mpsc::Receiver<FrontendCommand>>,
    backend_thread: Option<thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,

    // ランチャー用
    launcher_state: LauncherUiState,
    launcher_rx: mpsc::Receiver<LaunchProgress>,
    launcher_tx: mpsc::Sender<LaunchProgress>,
    launcher_thread: Option<thread::JoinHandle<()>>,
    launcher_cancel: Arc<AtomicBool>,
}

fn sanitize_ui_lang(lang: &str) -> String {
    match lang {
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        "zh-CN" => "zh-CN".to_string(),
        _ => "ja".to_string(),
    }
}

#[derive(Serialize)]
struct ListPayload {
    texts: Vec<String>,
}

fn wait_for_translation_server(port: u16, client: &reqwest::blocking::Client) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);
    for _ in 0..40 {
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

fn provision_launcher_config_from_misplaced(misplaced_path: &Path, launcher_config_path: &Path) {
    if !misplaced_path.exists() {
        return;
    }

    if launcher_config_path.exists() {
        let backup = misplaced_path.with_extension("toml.bak");
        let _ = std::fs::rename(misplaced_path, &backup);
        eprintln!(
            "[WARN] Misplaced launcher_config.toml at {} - canonical exists, retired to {}",
            misplaced_path.display(),
            backup.display()
        );
        return;
    }

    if let Err(e) = std::fs::copy(misplaced_path, launcher_config_path) {
        eprintln!(
            "[WARN] Failed to migrate misplaced launcher_config.toml: {}",
            e
        );
    } else {
        let backup = misplaced_path.with_extension("toml.bak");
        let _ = std::fs::rename(misplaced_path, &backup);
        eprintln!(
            "[INFO] Migrated misplaced launcher_config.toml to {}",
            launcher_config_path.display()
        );
    }
}

fn provision_runtime_config_before_normal(config_path: &Path) -> bool {
    match launcher::preflight_runtime_config_for_startup(config_path) {
        Ok(outcome) => {
            eprintln!(
                "[INFO] Runtime config preflight at {} => {:?}",
                config_path.display(),
                outcome
            );
            true
        }
        Err(e) => {
            eprintln!(
                "[WARN] Runtime config preflight failed at {}: {}",
                config_path.display(),
                e
            );
            false
        }
    }
}

fn model_inputs_from_context(ctx: &crate::backend::processor::TranslationContext) -> Vec<String> {
    match &ctx.processor_data {
        ProcessorData::Structural { text_tokens, .. } if text_tokens.len() > 1 => {
            text_tokens.clone()
        }
        _ => ctx.parts_to_translate.clone(),
    }
}

impl TenukiApp {
    fn load_input_records_or_log(&mut self) {
        if let Err(err) = self.ui.load_input_records() {
            self.ui.add_log(
                LogSource::Tenuki,
                format!("入力履歴の読み込みに失敗しました: {}", err),
                LogLevel::Error,
                messages::current_timestamp(),
            );
        }
    }

    fn save_input_records_or_log(&mut self) {
        if let Err(err) = self.ui.save_input_records() {
            self.ui.add_log(
                LogSource::Tenuki,
                format!("入力履歴の保存に失敗しました: {}", err),
                LogLevel::Error,
                messages::current_timestamp(),
            );
        }
    }

    fn refresh_pickup_preview(&mut self) {
        let pickup_records: Vec<_> = self
            .ui
            .display
            .input_records
            .iter()
            .filter(|record| record.pickup)
            .cloned()
            .collect();

        if pickup_records.is_empty() {
            self.ui.set_work_result(
                "pickup".to_string(),
                "pickup はまだありません".to_string(),
                false,
            );
            return;
        }

        let mode = TranslationMode::from_str(&self.ui.display.translation_mode);
        let processor = ProcessorFactory::create(mode, self.ui.state.structural_edit);
        let mut sections = Vec::new();

        for record in &pickup_records {
            let mut visible_lines = Vec::new();
            let mut model_inputs = Vec::new();

            for line in record.snapshot.extracted_text.split('\n') {
                let ctx = processor.preprocess(line);
                let visible = match &ctx.processor_data {
                    ProcessorData::Structural { visible_text, .. } => visible_text.clone(),
                    ProcessorData::Passthrough => line.to_string(),
                };
                visible_lines.push(visible);
                model_inputs.extend(model_inputs_from_context(&ctx));
            }

            let preview = format!(
                "[{}] {}\n原文: {}\n抽出: {}\n可視: {}\nモデル入力: {}\nメモ: {}",
                record.timestamp,
                if record.occurrences > 1 {
                    format!("x{}", record.occurrences)
                } else {
                    "x1".to_string()
                },
                record.snapshot.raw_text,
                record.snapshot.extracted_text,
                visible_lines.join("\n"),
                if model_inputs.is_empty() {
                    "-".to_string()
                } else {
                    model_inputs.join(" | ")
                },
                if record.note.trim().is_empty() {
                    "-".to_string()
                } else {
                    record.note.clone()
                }
            );
            sections.push(preview);
        }

        self.ui.set_work_result(
            format!("pickup {}件", pickup_records.len()),
            sections.join("\n\n--------------------------------\n\n"),
            false,
        );
    }

    fn start_file_work(&mut self, file_path: PathBuf) {
        let port = config::load(&self.config_path)
            .map(|cfg| cfg.server_port)
            .unwrap_or(14371);
        let event_tx = self.event_tx.clone();
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to build HTTP client");
        self.ui.set_work_running(true);
        self.ui
            .set_work_result(file_path.display().to_string(), "".to_string(), false);

        thread::spawn(move || {
            let title = file_path.display().to_string();
            let result = (|| -> anyhow::Result<String> {
                let content = fs::read_to_string(&file_path)?;
                let texts = content
                    .split('\n')
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>();
                if texts.is_empty() {
                    return Ok(String::new());
                }

                if !wait_for_translation_server(port, &client) {
                    anyhow::bail!("translation server is not ready");
                }

                let url = format!("http://127.0.0.1:{}/list", port);
                let response = client.post(&url).json(&ListPayload { texts }).send()?;
                Ok(response.text()?)
            })();

            let (text, is_error) = match result {
                Ok(text) => (text, false),
                Err(err) => (format!("ファイル翻訳に失敗しました: {}", err), true),
            };

            let _ = event_tx.send(BackendEvent::WorkResult {
                title,
                text,
                is_error,
            });
        });
    }

    fn new(
        cc: &eframe::CreationContext,
        command_tx: mpsc::Sender<FrontendCommand>,
        command_rx: mpsc::Receiver<FrontendCommand>,
        event_tx: mpsc::Sender<BackendEvent>,
        event_rx: mpsc::Receiver<BackendEvent>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        ui::fonts::setup_fonts(cc);

        // install_root 解決 / launcher_config.toml の権威位置
        let install_root = launcher::resolve_install_root();
        let launcher_config_path = install_root.join("launcher_config.toml");

        // 誤配置対策: target/debug または target/release の config を移設
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        if let Some(ref dir) = exe_dir {
            let wrong_debug = dir.join("launcher_config.toml");
            provision_launcher_config_from_misplaced(&wrong_debug, &launcher_config_path);
        }

        // 起動ログ
        eprintln!("[INFO] InstallRoot: {}", install_root.display());
        eprintln!("[INFO] LauncherConfig: {}", launcher_config_path.display());

        let base_dir = install_root.clone();
        let config_path = base_dir.join("config.toml");

        // ランチャー用チャンネル
        let (launcher_tx, launcher_rx) = mpsc::channel();
        let launcher_cancel = Arc::new(AtomicBool::new(false));
        let launcher_thread = None;

        let config_ready_for_normal = if config_path.exists() {
            provision_runtime_config_before_normal(&config_path)
        } else {
            false
        };

        let config_result = if config_ready_for_normal {
            config::load(&config_path)
        } else {
            Err(anyhow!("config.toml is not ready for normal startup"))
        };

        let mode = if config_ready_for_normal && launcher::check_ready(&base_dir) {
            AppMode::Normal
        } else {
            AppMode::Launcher
        };

        let initial_src_lang = config_result
            .as_ref()
            .map(|c| c.src_lang.clone())
            .unwrap_or_else(|_| "en".to_string());

        let initial_tgt_lang = config_result
            .as_ref()
            .map(|c| c.tgt_lang.clone())
            .unwrap_or_else(|_| "ja".to_string());

        let initial_dict_slot = config_result
            .as_ref()
            .ok()
            .and_then(|c| c.dict_slot.clone());

        let initial_profile = config_result
            .as_ref()
            .map(|c| c.profile.clone())
            .unwrap_or_else(|_| "game".to_string());
        let initial_profile_runtime =
            config::load_profile(&config_path, &initial_profile).unwrap_or_default();

        let initial_ui_lang = config_result
            .as_ref()
            .map(|c| c.ui_lang.clone())
            .unwrap_or_else(|_| "en".to_string());
        let initial_ui_lang = sanitize_ui_lang(&initial_ui_lang);

        let initial_custom_lang_name = config_result
            .as_ref()
            .map(|c| c.custom_lang_name.clone())
            .unwrap_or_default();

        let backend_thread = None;
        let command_rx_opt = Some(command_rx);

        let ui = UiContainer::with_base_dir(base_dir.clone());
        let mut app = Self {
            mode,
            base_dir,
            config_path,
            cached_model_check: None,
            ui,
            command_tx,
            event_rx,
            event_tx,
            command_rx: command_rx_opt,
            backend_thread,
            shutdown,
            launcher_state: LauncherUiState::default(),
            launcher_rx,
            launcher_tx,
            launcher_thread,
            launcher_cancel,
        };

        app.ui.update_src_lang(&initial_src_lang);
        app.ui
            .update_tgt_lang(&initial_tgt_lang, Some(&initial_custom_lang_name));
        app.ui.update_dict_slot(initial_dict_slot);
        app.ui.update_profile(&initial_profile);
        app.ui
            .update_translation_mode(&initial_profile_runtime.translation_mode);
        app.ui
            .update_structural_options(initial_profile_runtime.structural.into());
        app.ui.update_ui_lang(&initial_ui_lang);
        app.ui
            .refresh_available_profiles(&app.base_dir.join("profiles"));
        app.load_input_records_or_log();

        // 通常起動時は backend を自動起動する
        if app.mode == AppMode::Normal {
            // Launcher を経由しない場合でも基本ディレクトリは確保する
            for dir in ["profiles", "logs", "tmp"] {
                let _ = fs::create_dir_all(app.base_dir.join(dir));
            }
            app.start_backend_after_setup();
            let _ = app.command_tx.send(FrontendCommand::Start);
        }

        app
    }

    fn check_models(&mut self) -> bool {
        logic::check_models(&self.base_dir, &mut self.cached_model_check)
    }

    fn start_backend_after_setup(&mut self) {
        if let Some(command_rx) = self.command_rx.take() {
            if !provision_runtime_config_before_normal(&self.config_path) {
                self.ui.add_log(
                    ui::container::LogSource::Tenuki,
                    "config.toml を current shape に再構成できなかったため launcher へ戻します"
                        .to_string(),
                    messages::LogLevel::Error,
                    messages::current_timestamp(),
                );
                self.mode = AppMode::Launcher;
                self.launcher_state = LauncherUiState::error(
                    "config.toml を再構成できませんでした。Retry してください。".to_string(),
                );
                self.launcher_cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                self.launcher_thread = None;
                self.command_rx = Some(command_rx);
                return;
            }

            let config = match config::load(&self.config_path) {
                Ok(c) => c,
                Err(e) => {
                    self.ui.add_log(
                        ui::container::LogSource::Tenuki,
                        format!(
                            "config.toml の読み込みに失敗したため backend を起動できません: {}",
                            e
                        ),
                        messages::LogLevel::Error,
                        messages::current_timestamp(),
                    );
                    self.ui
                        .set_status(StatusKey::ConfigError, StatusIcon::Warning, true);
                    self.mode = AppMode::Launcher;
                    self.launcher_state = LauncherUiState::error(format!(
                        "config.toml 読み込み失敗: {e}"
                    ));
                    self.launcher_cancel
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                    self.launcher_thread = None;
                    self.command_rx = Some(command_rx);
                    return;
                }
            };

            let launcher_config_path =
                launcher::resolve_install_root().join("launcher_config.toml");
            let app_config = match launcher::app_config::AppConfig::load(&launcher_config_path) {
                Ok(c) => c,
                Err(e) => {
                    self.ui.add_log(
                        ui::container::LogSource::Tenuki,
                        format!(
                            "launcher_config.toml の読み込みに失敗したため launcher へ戻ります: {}",
                            e
                        ),
                        messages::LogLevel::Error,
                        messages::current_timestamp(),
                    );
                    self.mode = AppMode::Launcher;
                    self.launcher_state = LauncherUiState::error(format!(
                        "launcher_config.toml 読み込み失敗: {e}"
                    ));
                    self.launcher_cancel
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                    self.launcher_thread = None;
                    self.command_rx = Some(command_rx);
                    return;
                }
            };

            let shutdown_clone = self.shutdown.clone();
            let event_tx_clone = self.event_tx.clone();
            let base_dir_clone = self.base_dir.clone();
            self.backend_thread = Some(backend::start_backend(
                config,
                app_config,
                base_dir_clone,
                shutdown_clone,
                event_tx_clone,
                command_rx,
            ));
        }
    }
}

impl eframe::App for TenukiApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.command_tx.send(FrontendCommand::Stop);
        self.shutdown.store(true, Ordering::Relaxed);
        self.launcher_cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.backend_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.launcher_thread.take() {
            let _ = handle.join();
        }
    }

    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        let base_dir = self.base_dir.clone();
        let _config_path = self.config_path.clone();
        let _has_models = self.check_models();
        let has_server = logic::check_llama_server(&base_dir);

        let mut needs_repaint = false;

        // backend イベント処理。Normal モードで適用する
        while let Ok(event) = self.event_rx.try_recv() {
            needs_repaint = true;
            match event {
                BackendEvent::Log(source, msg, level, timestamp) => {
                    let src = match source {
                        messages::LogSource::Tenuki => LogSource::Tenuki,
                        messages::LogSource::LlamaCpp => LogSource::LlamaCpp,
                    };
                    self.ui.add_log(src, msg, level, timestamp);
                }
                BackendEvent::DictionaryLoaded(count) => {
                    self.ui.set_dictionary_loaded(count);
                }
                BackendEvent::DictionaryNewEntry(timestamp, original, translated) => {
                    self.ui
                        .add_dictionary_entry(timestamp, original, translated);
                }
                BackendEvent::DictionaryLogEntry(timestamp, original, translated) => {
                    self.ui
                        .add_dictionary_log_entry(timestamp, original, translated);
                }
                BackendEvent::StatisticsUpdate(dict_hits, model_calls) => {
                    self.ui.update_statistics(dict_hits, model_calls);
                }
                BackendEvent::InputAnalysisUpdated(snapshot) => {
                    if self.ui.update_input_analysis(snapshot) {
                        self.save_input_records_or_log();
                    }
                    if self.ui.state.work_source == ui::container::WorkSource::PickupList
                        && self.ui.state.immediate_apply
                    {
                        self.refresh_pickup_preview();
                    }
                }
                BackendEvent::WorkResult {
                    title,
                    text,
                    is_error,
                } => {
                    self.ui.set_work_running(false);
                    self.ui.set_work_result(title, text, is_error);
                }
                BackendEvent::ProcessStatus(proc_type, running) => {
                    let pt = match proc_type {
                        messages::ProcessType::InferenceEngine => ProcessType::InferenceEngine,
                        messages::ProcessType::Tenuki => ProcessType::Tenuki,
                    };
                    self.ui.update_process_status(pt, running);
                    // 停止中に Tenuki が落ちたら Stopped へ遷移する
                    if !running && pt == ProcessType::Tenuki {
                        if self.ui.display.status_key == StatusKey::Stopping {
                            self.ui
                                .set_status(StatusKey::Stopped, StatusIcon::None, true);
                        }
                    }
                }
                BackendEvent::AvailableModels(models) => {
                    self.ui.update_available_models(models);
                }
                BackendEvent::SelectedModelResolved(model) => {
                    self.ui.update_selected_model(model);
                }
                BackendEvent::DictSlotChanged(slot) => {
                    self.ui.update_dict_slot(Some(slot));
                    self.load_input_records_or_log();
                }
                BackendEvent::LanguageChanged(lang) => {
                    self.ui.update_tgt_lang(&lang, None);
                    self.ui.add_log(
                        LogSource::Tenuki,
                        format!("翻訳先を {} に切り替えました", lang),
                        LogLevel::Info,
                        messages::current_timestamp(),
                    );
                }
                BackendEvent::BackendReady {
                    engine_success,
                    translator_success,
                } => {
                    if engine_success && translator_success {
                        self.ui
                            .set_status(StatusKey::Ready, StatusIcon::Check, true);
                    } else {
                        self.ui
                            .set_status(StatusKey::Failed, StatusIcon::Warning, true);
                    }
                }
                BackendEvent::ServerMetrics {
                    vram_mb,
                    shared_mb,
                    tokens_per_second,
                } => {
                    self.ui
                        .update_server_metrics(vram_mb, shared_mb, tokens_per_second);
                }
            }
        }

        match self.mode {
            AppMode::Launcher => {
                let (repaint, switch_to_normal) = show_launcher_screen(
                    ctx,
                    &mut self.launcher_state,
                    &self.launcher_rx,
                    &self.launcher_tx,
                    &mut self.launcher_thread,
                    &self.launcher_cancel,
                    &self.base_dir,
                    "en",
                );
                if repaint {
                    needs_repaint = true;
                }

                // Launcher から Normal へ切り替わったら backend を起動する
                if switch_to_normal {
                    self.mode = AppMode::Normal;
                    // profiles/ は launcher 側でも作られるが、UI 用に再読込しておく
                    self.ui
                        .refresh_available_profiles(&self.base_dir.join("profiles"));
                    self.ui
                        .set_status(StatusKey::Starting, StatusIcon::Spinner, true);
                    self.start_backend_after_setup();
                    let _ = self.command_tx.send(FrontendCommand::Start);
                }
            }

            AppMode::Normal => {
                let mut commands = self.ui.show(ctx);

                if commands.exit_app {
                    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                    return;
                }

                if commands.start_backend {
                    if has_server {
                        self.ui
                            .set_status(StatusKey::Starting, StatusIcon::Spinner, true);
                        self.command_tx.send(FrontendCommand::Start).ok();
                    } else {
                        self.mode = AppMode::Launcher;
                        self.launcher_state = LauncherUiState::error(
                            "llama-server が見つかりません。セットアップを再実行してください。"
                                .to_string(),
                        );
                        self.launcher_cancel.store(false, Ordering::Relaxed);
                        self.launcher_thread = None;
                    }
                }
                if commands.stop_backend {
                    self.ui
                        .set_status(StatusKey::Stopping, StatusIcon::Spinner, true);
                    self.command_tx.send(FrontendCommand::Stop).ok();
                }
                if commands.restart_backend {
                    if has_server {
                        self.ui
                            .set_status(StatusKey::Restarting, StatusIcon::Spinner, true);
                        self.command_tx.send(FrontendCommand::Restart).ok();
                    } else {
                        self.mode = AppMode::Launcher;
                        self.launcher_state = LauncherUiState::error(
                            "llama-server が見つかりません。セットアップを再実行してください。"
                                .to_string(),
                        );
                        self.launcher_cancel.store(false, Ordering::Relaxed);
                        self.launcher_thread = None;
                    }
                }
                if let Some(lang) = commands.set_ui_lang.take() {
                    let san = sanitize_ui_lang(&lang);
                    self.ui.update_ui_lang(&san);
                    if let Ok(mut cfg) = config::load(&self.config_path) {
                        cfg.ui_lang = san.clone();
                        let _ = config::save(&self.config_path, &cfg);
                    }
                }

                let pending_lang_pair = commands.set_lang_pair.take();
                if let Some((src, tgt, tgt_name, dict_slot)) = pending_lang_pair {
                    self.ui.update_src_lang(&src);
                    self.ui.update_tgt_lang(&tgt, tgt_name.as_deref());
                    // dict_slot は送信前にここで確定する。None は「新規スロットが必要」を意味する。
                    let resolved_slot = dict_slot.unwrap_or_else(|| {
                        backend::manager::create_new_slot(&tgt, &self.base_dir)
                            .to_string_lossy()
                            .to_string()
                    });
                    self.command_tx
                        .send(FrontendCommand::SetLanguagePair {
                            src,
                            tgt,
                            tgt_name,
                            dict_slot: resolved_slot,
                        })
                        .ok();
                }
                if let Some(slot) = commands.set_dict_slot.take() {
                    self.ui.update_dict_slot(Some(slot.clone()));
                    self.load_input_records_or_log();
                    self.command_tx
                        .send(FrontendCommand::SetDictSlot(slot))
                        .ok();
                }

                if let Some(profile_name) = commands.set_profile.take() {
                    self.ui.update_profile(&profile_name);
                    if let Ok(profile) = config::load_profile(&self.config_path, &profile_name) {
                        self.ui.update_translation_mode(&profile.translation_mode);
                        self.ui.update_structural_options(profile.structural.into());
                    }
                    self.command_tx
                        .send(FrontendCommand::SetProfile(profile_name))
                        .ok();
                }

                if let Some(filename) = commands.select_model.take() {
                    self.command_tx
                        .send(FrontendCommand::SetModel(filename))
                        .ok();
                }

                if let Some(folder) = commands.set_work_folder.take() {
                    self.ui.state.work_folder = Some(folder);
                    self.ui.state.selected_work_file = None;
                }

                if let Some((id, pickup)) = commands.set_input_pickup.take() {
                    if self.ui.set_input_pickup(id, pickup) {
                        self.save_input_records_or_log();
                        if self.ui.state.work_source == ui::container::WorkSource::PickupList
                            && self.ui.state.immediate_apply
                        {
                            self.refresh_pickup_preview();
                        }
                    }
                }

                if let Some((id, note)) = commands.set_input_pickup_note.take() {
                    if self.ui.update_input_pickup_note(id, note) {
                        self.save_input_records_or_log();
                        if self.ui.state.work_source == ui::container::WorkSource::PickupList
                            && self.ui.state.immediate_apply
                        {
                            self.refresh_pickup_preview();
                        }
                    }
                }

                let structural_options = commands.set_structural_options.take();
                if let Some(options) = structural_options.as_ref() {
                    self.ui.update_structural_options(*options);
                }

                if commands.refresh_pickup_preview {
                    self.refresh_pickup_preview();
                }

                let update_cmd = FrontendCommand::UpdateSettings {
                    structural: structural_options,
                    server_port: commands.set_server_port.take(),
                    server_host: commands.set_server_host.take(),
                };

                if !update_cmd.is_empty_update() {
                    let _ = self.command_tx.send(update_cmd);
                }

                if commands.run_work {
                    match self.ui.state.work_source {
                        ui::container::WorkSource::PickupList => {
                            self.refresh_pickup_preview();
                        }
                        ui::container::WorkSource::File => {
                            if let Some(file_path) = self.ui.state.selected_work_file.clone() {
                                self.start_file_work(file_path);
                            } else {
                                self.ui.set_work_result(
                                    "ファイル翻訳".to_string(),
                                    "ファイルが選択されていません。".to_string(),
                                    true,
                                );
                            }
                        }
                    }
                }
            }
        }

        if needs_repaint {
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result<()> {
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();

    let shutdown = Arc::new(AtomicBool::new(false));

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([480.0, 320.0])
            .with_min_inner_size([400.0, 240.0])
            .with_resizable(true)
            .with_title("TENUKI"),
        ..Default::default()
    };

    eframe::run_native(
        "TENUKI",
        options,
        Box::new(|cc| {
            Ok(Box::new(TenukiApp::new(
                cc, command_tx, command_rx, event_tx, event_rx, shutdown,
            )))
        }),
    )
}
