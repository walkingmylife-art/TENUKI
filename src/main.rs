// src/main.rs

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod backend;
mod config;
mod file_translate;
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

use file_translate::commands::FileTranslateUiCommand;
use file_translate::controller::FileTranslateController;
use launcher::{show_launcher_screen, LaunchProgress, LauncherUiState};
use messages::{BackendEvent, FrontendCommand, LogLevel};
use ui::container::{LogSource, ProcessType, StatusIcon, StatusKey, UiContainer};
use ui::list_text::{self, ListText};
use ui::work_result_text;

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppMode {
    Launcher,
    Normal,
}

struct TenukiApp {
    mode: AppMode,
    base_dir: PathBuf,
    config_path: PathBuf,
    ui: UiContainer,
    command_tx: mpsc::Sender<FrontendCommand>,
    event_rx: mpsc::Receiver<BackendEvent>,
    event_tx: mpsc::Sender<BackendEvent>,
    command_rx: Option<mpsc::Receiver<FrontendCommand>>,
    backend_thread: Option<thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,

    // Launcher runtime state.
    launcher_state: LauncherUiState,
    launcher_rx: mpsc::Receiver<LaunchProgress>,
    launcher_tx: mpsc::Sender<LaunchProgress>,
    launcher_thread: Option<thread::JoinHandle<()>>,
    launcher_cancel: Arc<AtomicBool>,
    file_translate_ctrl: FileTranslateController,
}

fn sanitize_ui_lang(lang: &str) -> String {
    match lang {
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        "zh-CN" => "zh-CN".to_string(),
        _ => "ja".to_string(),
    }
}

fn create_new_dict_slot_command_for_target(
    tgt_lang: &str,
    base_dir: &PathBuf,
) -> (String, FrontendCommand) {
    let slot = backend::manager::create_new_slot(tgt_lang, base_dir)
        .to_string_lossy()
        .to_string();
    (slot.clone(), FrontendCommand::SetDictSlot(slot))
}

fn build_initial_launcher_state(
    launcher_config_path: &Path,
    config_ready_for_normal: bool,
    readiness: &Result<(), launcher::CheckReadyReason>,
) -> LauncherUiState {
    if !config_ready_for_normal {
        return LauncherUiState::with_startup_reason("config.toml preflight failed".to_string());
    }

    match readiness {
        Ok(()) => LauncherUiState::initial_setup(),
        Err(launcher::CheckReadyReason::ConfigLoadFail(reason)) => {
            if launcher_config_path.exists() {
                LauncherUiState::with_startup_reason(format!(
                    "launcher_config.toml load failed: {}",
                    reason
                ))
            } else {
                LauncherUiState::initial_setup()
            }
        }
        Err(
            launcher::CheckReadyReason::RuntimeIncomplete { .. }
            | launcher::CheckReadyReason::NoModelsAvailable
            | launcher::CheckReadyReason::ModelMissing { .. }
            | launcher::CheckReadyReason::ModelSizeMismatch { .. },
        ) => LauncherUiState::initial_setup(),
        Err(launcher::CheckReadyReason::LocalModelMissing { filename }) => {
            LauncherUiState::with_startup_reason(format!(
                "Local model '{}' not found.\n\
                 Restore it to the models/ folder or select another model.",
                filename
            ))
        }
        Err(launcher::CheckReadyReason::LocalModelChanged {
            filename,
            expected,
            actual,
        }) => LauncherUiState::with_startup_reason(format!(
            "Local model '{}' changed on disk (committed: {} bytes, found: {} bytes).\n\
             Re-select the model to update the authority, or restore the original file.",
            filename, expected, actual
        )),
        Err(launcher::CheckReadyReason::StartupModelUnresolved) => {
            LauncherUiState::with_startup_reason(
                "Unable to resolve a startup model. Re-select the model.".to_string(),
            )
        }
    }
}

fn complete_backend_handoff(
    command_tx: &mpsc::Sender<FrontendCommand>,
    startup_result: Result<(), String>,
) -> Result<(), String> {
    startup_result?;
    command_tx
        .send(FrontendCommand::Start)
        .map_err(|e| format!("Failed to queue backend start command: {}", e))
}

fn reset_backend_runtime(
    command_tx: &mut mpsc::Sender<FrontendCommand>,
    command_rx: &mut Option<mpsc::Receiver<FrontendCommand>>,
    backend_thread: &mut Option<thread::JoinHandle<()>>,
    shutdown: &mut Arc<AtomicBool>,
) {
    let _ = command_tx.send(FrontendCommand::Stop);
    shutdown.store(true, Ordering::Relaxed);

    if let Some(handle) = backend_thread.take() {
        let _ = handle.join();
    }

    *shutdown = Arc::new(AtomicBool::new(false));
    let (new_tx, new_rx) = mpsc::channel();
    *command_tx = new_tx;
    *command_rx = Some(new_rx);
}

fn provision_launcher_config_from_misplaced(misplaced_path: &Path, launcher_config_path: &Path) {
    if misplaced_path == launcher_config_path {
        return;
    }

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

impl TenukiApp {
    fn load_input_records_or_log(&mut self) {
        if let Err(err) = self.ui.load_input_records() {
            self.ui.add_log(
                LogSource::Tenuki,
                format!("Failed to load input history: {}", err),
                LogLevel::Error,
                messages::current_timestamp(),
            );
        }
    }

    fn save_input_records_or_log(&mut self) {
        if let Err(err) = self.ui.save_input_records() {
            self.ui.add_log(
                LogSource::Tenuki,
                format!("Failed to save input history: {}", err),
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

        let (title, preview) =
            work_result_text::pickup_preview(&self.ui.display.ui_lang, &pickup_records);
        self.ui.set_work_result(title, preview, false);
    }

    // File Translate / List mode is a separate entry from normal translation.
    // /list does not update dictionary, cache, or input analysis.
    // Output is {source_stem}.txt in dict.txt format, written incrementally via .partial.txt.
    fn poll_file_translate_scan(&mut self) {
        self.file_translate_ctrl.poll_file_translate_scan(&mut self.ui);
    }

    fn poll_file_translate_preview(&mut self) {
        self.file_translate_ctrl.poll_file_translate_preview(&mut self.ui);
    }

    fn handle_file_translate_command(&mut self, command: FileTranslateUiCommand) {
        self.file_translate_ctrl.handle_file_translate_command(&mut self.ui, command);
    }
}

struct InitialConfigValues {
    src_lang: String,
    tgt_lang: String,
    dict_slot: Option<String>,
    profile: String,
    profile_runtime: config::TranslationProfile,
    ui_lang: String,
    custom_lang_name: String,
}

fn read_initial_config_values(config_path: &Path) -> InitialConfigValues {
    let config_result = config::load(config_path);
    let src_lang = config_result.as_ref().map(|c| c.src_lang.clone()).unwrap_or_else(|_| "en".to_string());
    let tgt_lang = config_result.as_ref().map(|c| c.tgt_lang.clone()).unwrap_or_else(|_| "ja".to_string());
    let dict_slot = config_result.as_ref().ok().and_then(|c| c.dict_slot.clone());
    let profile = config_result.as_ref().map(|c| c.profile.clone()).unwrap_or_else(|_| "game".to_string());
    let profile_runtime = config::load_profile(config_path, &profile).unwrap_or_default();
    let ui_lang = config_result.as_ref().map(|c| c.ui_lang.clone()).unwrap_or_else(|_| "en".to_string());
    let ui_lang = sanitize_ui_lang(&ui_lang);
    let custom_lang_name = config_result.as_ref().map(|c| c.custom_lang_name.clone()).unwrap_or_default();
    InitialConfigValues { src_lang, tgt_lang, dict_slot, profile, profile_runtime, ui_lang, custom_lang_name }
}

fn apply_initial_config_to_ui(ui: &mut UiContainer, config: &InitialConfigValues) {
    ui.update_src_lang(&config.src_lang);
    ui.update_tgt_lang(&config.tgt_lang, Some(&config.custom_lang_name));
    ui.update_dict_slot(config.dict_slot.clone());
    ui.update_profile(&config.profile);
    ui.update_mode(&config.profile_runtime.mode);
    ui.update_game_text_options(config.profile_runtime.game_text.into());
    ui.update_ui_lang(&config.ui_lang);
}

impl TenukiApp {
    fn new(
        cc: &eframe::CreationContext,
        command_tx: mpsc::Sender<FrontendCommand>,
        command_rx: mpsc::Receiver<FrontendCommand>,
        event_tx: mpsc::Sender<BackendEvent>,
        event_rx: mpsc::Receiver<BackendEvent>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        ui::fonts::setup_fonts(cc);

        // launcher_config.toml authority lives under install_root.
        let install_root = launcher::resolve_install_root();
        let launcher_config_path = install_root.join("launcher_config.toml");

        // Retire misplaced debug/release launcher_config.toml copies.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        if let Some(ref dir) = exe_dir {
            let wrong_debug = dir.join("launcher_config.toml");
            provision_launcher_config_from_misplaced(&wrong_debug, &launcher_config_path);
        }

        // Keep resolved authority paths visible in launcher logs.
        eprintln!("[INFO] InstallRoot: {}", install_root.display());
        eprintln!("[INFO] LauncherConfig: {}", launcher_config_path.display());

        let base_dir = install_root.clone();
        let config_path = base_dir.join("config.toml");

        let (launcher_tx, launcher_rx) = mpsc::channel();
        let launcher_cancel = Arc::new(AtomicBool::new(false));
        let launcher_thread = None;

        // Preflight can provision missing config, so always run it before normal mode.
        let config_ready_for_normal = provision_runtime_config_before_normal(&config_path);

        let readiness = launcher::check_ready_detail(&base_dir);
        let mode = if config_ready_for_normal && readiness.is_ok() {
            AppMode::Normal
        } else {
            AppMode::Launcher
        };

        let cfg = read_initial_config_values(&config_path);

        let backend_thread = None;
        let command_rx_opt = Some(command_rx);

        let initial_launcher_state = build_initial_launcher_state(
            &launcher_config_path,
            config_ready_for_normal,
            &readiness,
        );

        let ui = UiContainer::with_base_dir(base_dir.clone());
        let ctrl_base_dir = base_dir.clone();
        let ctrl_config_path = config_path.clone();
        let ctrl_event_tx = event_tx.clone();
        let mut app = Self {
            mode,
            base_dir,
            config_path,
            ui,
            command_tx,
            event_rx,
            event_tx,
            command_rx: command_rx_opt,
            backend_thread,
            shutdown,
            launcher_state: initial_launcher_state,
            launcher_rx,
            launcher_tx,
            launcher_thread,
            launcher_cancel,
            file_translate_ctrl: FileTranslateController {
                base_dir: ctrl_base_dir,
                config_path: ctrl_config_path,
                event_tx: ctrl_event_tx,
                file_translate_cancel: None,
            },
        };

        apply_initial_config_to_ui(&mut app.ui, &cfg);
        app.ui.refresh_available_profiles(&app.base_dir.join("profiles"));
        app.load_input_records_or_log();

        app.try_start_backend_if_normal();

        app
    }

    fn start_backend_after_setup(&mut self) -> Result<(), String> {
        let Some(command_rx) = self.command_rx.take() else {
            return Err("backend command receiver unavailable".to_string());
        };

        if !provision_runtime_config_before_normal(&self.config_path) {
            self.ui.add_log(
                ui::container::LogSource::Tenuki,
                "config.toml could not be prepared for startup".to_string(),
                messages::LogLevel::Error,
                messages::current_timestamp(),
            );
            self.command_rx = Some(command_rx);
            return Err("config.toml could not be prepared for startup".to_string());
        }

        let config = match config::load(&self.config_path) {
            Ok(c) => c,
            Err(e) => {
                self.ui.add_log(
                    ui::container::LogSource::Tenuki,
                    format!("config.toml load failed: {}", e),
                    messages::LogLevel::Error,
                    messages::current_timestamp(),
                );
                self.ui
                    .set_status(StatusKey::ConfigError, StatusIcon::Warning, true);
                self.command_rx = Some(command_rx);
                return Err(format!("config.toml load failed: {e}"));
            }
        };

        // launcher_config.toml authority lives under install_root.
        let launcher_config_path = launcher::resolve_install_root().join("launcher_config.toml");
        let app_config = match launcher::app_config::AppConfig::load(&launcher_config_path) {
            Ok(c) => c,
            Err(e) => {
                self.ui.add_log(
                    ui::container::LogSource::Tenuki,
                    format!("launcher_config.toml load failed: {}", e),
                    messages::LogLevel::Error,
                    messages::current_timestamp(),
                );
                self.command_rx = Some(command_rx);
                return Err(format!("launcher_config.toml load failed: {e}"));
            }
        };

        self.shutdown.store(false, Ordering::Relaxed);
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
        Ok(())
    }

    fn try_start_backend_if_normal(&mut self) {
        if self.mode != AppMode::Normal {
            return;
        }
        for dir in ["profiles", "logs", "tmp"] {
            let _ = fs::create_dir_all(self.base_dir.join(dir));
        }
        let start_result = self.start_backend_after_setup();
        let handoff = complete_backend_handoff(&self.command_tx, start_result);
        if let Err(err) = handoff {
            self.return_to_launcher_with_cleanup(LauncherUiState::error(err));
        }
    }

    fn return_to_launcher_with_cleanup(&mut self, launcher_state: LauncherUiState) {
        reset_backend_runtime(
            &mut self.command_tx,
            &mut self.command_rx,
            &mut self.backend_thread,
            &mut self.shutdown,
        );
        self.mode = AppMode::Launcher;
        self.launcher_state = launcher_state;
        self.launcher_cancel.store(false, Ordering::Relaxed);
        self.launcher_thread = None;
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
        let has_server = logic::check_llama_server(&self.base_dir);
        let mut needs_repaint = self.handle_backend_events();

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
                if switch_to_normal {
                    self.mode = AppMode::Normal;
                    self.ui
                        .refresh_available_profiles(&self.base_dir.join("profiles"));
                    self.ui
                        .set_status(StatusKey::Starting, StatusIcon::Spinner, true);
                    let start_result = self.start_backend_after_setup();
                    let handoff = complete_backend_handoff(&self.command_tx, start_result);
                    if let Err(err) = handoff {
                        self.return_to_launcher_with_cleanup(LauncherUiState::error(err));
                    }
                }
            }
            AppMode::Normal => {
                self.poll_file_translate_scan();
                self.poll_file_translate_preview();
                self.handle_normal_commands(ctx, has_server);
            }
        }
        if needs_repaint {
            ctx.request_repaint();
        }
    }
}

impl TenukiApp {
    fn handle_backend_events(&mut self) -> bool {
        let mut needs_repaint = false;
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
                BackendEvent::FileTranslateProgress { done, total } => {
                    self.ui.update_file_translate_progress(done, total);
                    self.ui
                        .set_file_translate_status_text(format!("{}/{}", done, total));
                }
                BackendEvent::FileTranslateLog { line, level } => {
                    self.ui.append_file_translate_log(line, level);
                }
                BackendEvent::StatisticsUpdate(dict_hits, model_calls) => {
                    self.ui.update_statistics(dict_hits, model_calls);
                }
                BackendEvent::InputAnalysisUpdated(snapshot) => {
                    if self.ui.update_input_analysis(snapshot) {
                        self.save_input_records_or_log();
                    }
                    if self.ui.state.immediate_apply {
                        self.refresh_pickup_preview();
                    }
                }
                BackendEvent::WorkResult {
                    title,
                    text,
                    is_error,
                } => {
                    self.file_translate_ctrl.file_translate_cancel = None;
                    self.ui.set_work_running(false);
                    self.ui.finish_file_translate_progress(is_error);
                    let done_count = self.ui.display.file_translate_done;
                    let status_text = if is_error {
                        list_text::text(&self.ui.display.ui_lang, ListText::Failed).to_string()
                    } else if text.trim() == "Stopped" {
                        list_text::stopped(&sanitize_ui_lang(&self.ui.display.ui_lang), done_count)
                    } else {
                        list_text::completed(
                            &sanitize_ui_lang(&self.ui.display.ui_lang),
                            done_count,
                        )
                    };
                    self.ui.set_file_translate_status_text(status_text);
                    if is_error {
                        self.ui.append_file_translate_log(
                            format!("[error] {}", text.lines().next().unwrap_or("failed")),
                            LogLevel::Error,
                        );
                    } else if text.trim() == "Stopped" {
                        self.ui.append_file_translate_log(
                            format!(
                                "[stopped] {}/{}",
                                self.ui.display.file_translate_done,
                                self.ui.display.file_translate_total
                            ),
                            LogLevel::Info,
                        );
                    }
                    self.ui.set_work_result(title, text, is_error);
                }
                BackendEvent::StatusNotice { title, message } => {
                    self.ui.set_work_result(title, message, false);
                }
                BackendEvent::ProcessStatus(proc_type, running) => {
                    let pt = match proc_type {
                        messages::ProcessType::InferenceEngine => ProcessType::InferenceEngine,
                        messages::ProcessType::Tenuki => ProcessType::Tenuki,
                    };
                    self.ui.update_process_status(pt, running);
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
                    self.ui.update_dict_slot(Some(slot.clone()));
                    self.load_input_records_or_log();
                }
                BackendEvent::LanguageChanged(lang) => {
                    self.ui.update_tgt_lang(&lang, None);
                    self.ui.add_log(
                        LogSource::Tenuki,
                        format!("Language changed: {}", lang),
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
        needs_repaint
    }

    fn handle_normal_commands(
        &mut self,
        ctx: &eframe::egui::Context,
        has_server: bool,
    ) {
        let mut commands = self.ui.show(ctx);

        if commands.exit_app {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            return;
        }

        if !has_server && (commands.start_backend || commands.restart_backend) {
            self.return_to_launcher_with_cleanup(LauncherUiState::error(
                "llama-server was not found. Run setup again.".to_string(),
            ));
            commands.start_backend = false;
            commands.restart_backend = false;
        }

        if commands.start_backend {
            if has_server {
                self.ui
                    .set_status(StatusKey::Starting, StatusIcon::Spinner, true);
                self.command_tx.send(FrontendCommand::Start).ok();
            } else {
                self.mode = AppMode::Launcher;
                self.launcher_state = LauncherUiState::error(
                    "llama-server was not found. Run setup again.".to_string(),
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
                    "llama-server was not found. Run setup again.".to_string(),
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
            let resolved_slot = match dict_slot.as_deref() {
                Some(slot) if !slot.trim().is_empty() => slot.trim().to_string(),
                _ => backend::manager::resolve_lang_pair_dict_slot(
                    None,
                    &tgt,
                    &self.base_dir,
                ),
            };
            self.command_tx
                .send(FrontendCommand::SetLanguagePair {
                    src,
                    tgt,
                    tgt_name,
                    dict_slot: resolved_slot,
                })
                .ok();
        }
        if commands.create_new_dict_slot {
            let tgt = self.ui.display.tgt_lang.clone();
            let (slot, command) =
                create_new_dict_slot_command_for_target(&tgt, &self.base_dir);
            self.ui.update_dict_slot(Some(slot));
            self.load_input_records_or_log();
            self.command_tx.send(command).ok();
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
                self.ui.update_mode(&profile.mode);
                self.ui.update_game_text_options(profile.game_text.into());
            }
            self.command_tx
                .send(FrontendCommand::SetProfile(profile_name))
                .ok();
        }

        if let Some(model_config) = commands.select_model.take() {
            self.command_tx
                .send(FrontendCommand::CommitModelSelection(model_config))
                .ok();
        }

        let file_translate_commands = std::mem::take(&mut commands.file_translate_commands);
        for command in file_translate_commands {
            self.handle_file_translate_command(command);
        }

        if let Some((id, pickup)) = commands.set_input_pickup.take() {
            if self.ui.set_input_pickup(id, pickup) {
                self.save_input_records_or_log();
                if self.ui.state.immediate_apply {
                    self.refresh_pickup_preview();
                }
            }
        }

        if let Some((id, note)) = commands.set_input_pickup_note.take() {
            if self.ui.update_input_pickup_note(id, note) {
                self.save_input_records_or_log();
                if self.ui.state.immediate_apply {
                    self.refresh_pickup_preview();
                }
            }
        }

        let game_text_options = commands.set_game_text_options.take();
        if let Some(options) = game_text_options.as_ref() {
            self.ui.update_game_text_options(*options);
        }

        if commands.refresh_pickup_preview {
            self.refresh_pickup_preview();
        }

        let update_cmd = FrontendCommand::UpdateSettings {
            game_text: game_text_options,
            server_port: commands.set_server_port.take(),
            server_host: commands.set_server_host.take(),
        };

        if !update_cmd.is_empty_update() {
            let _ = self.command_tx.send(update_cmd);
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

#[cfg(test)]
mod tests {
    use super::{
        build_initial_launcher_state, complete_backend_handoff,
        create_new_dict_slot_command_for_target, provision_launcher_config_from_misplaced,
        reset_backend_runtime,
    };
    use crate::launcher::{CheckReadyReason, LauncherEntryIntent, LauncherStep};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tenuki-main-test-{}-{}",
            std::process::id(),
            unique
        ))
    }

    #[test]
    fn provision_launcher_config_from_misplaced_same_path_is_noop() {
        let test_dir = unique_test_dir();
        fs::create_dir_all(&test_dir).expect("create test dir");

        let config_path = test_dir.join("launcher_config.toml");
        fs::write(&config_path, "ui_lang = \"ja\"\n").expect("write launcher config");

        provision_launcher_config_from_misplaced(&config_path, &config_path);

        assert!(
            config_path.exists(),
            "canonical config should remain in place"
        );
        assert!(
            !test_dir.join("launcher_config.toml.bak").exists(),
            "same-path guard must not create a backup"
        );

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn missing_launcher_authority_enters_initial_setup() {
        let test_dir = unique_test_dir();
        fs::create_dir_all(&test_dir).expect("create test dir");

        let config_path = test_dir.join("launcher_config.toml");
        let state = build_initial_launcher_state(
            &config_path,
            true,
            &Err(CheckReadyReason::ConfigLoadFail(
                "launcher_config.toml not found".to_string(),
            )),
        );

        assert!(matches!(
            state.entry_intent,
            LauncherEntryIntent::InitialSetup
        ));
        assert!(matches!(state.step, LauncherStep::WaitingForStart));
        assert!(state.startup_reason.is_none());

        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn existing_launcher_authority_with_runtime_incomplete_enters_initial_setup() {
        let test_dir = unique_test_dir();
        fs::create_dir_all(&test_dir).expect("create test dir");

        let config_path = test_dir.join("launcher_config.toml");
        fs::write(&config_path, "ui_lang = \"ja\"\n").expect("write launcher config");

        let state = build_initial_launcher_state(
            &config_path,
            true,
            &Err(CheckReadyReason::RuntimeIncomplete {
                backend: "vulkan".to_string(),
            }),
        );

        assert!(matches!(
            state.entry_intent,
            LauncherEntryIntent::InitialSetup
        ));
        assert!(matches!(state.step, LauncherStep::WaitingForStart));
        assert!(state.startup_reason.is_none());

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn no_models_available_enters_initial_setup() {
        let test_dir = unique_test_dir();
        fs::create_dir_all(&test_dir).expect("create test dir");

        let config_path = test_dir.join("launcher_config.toml");
        fs::write(&config_path, "ui_lang = \"ja\"\n").expect("write launcher config");

        let state = build_initial_launcher_state(
            &config_path,
            true,
            &Err(CheckReadyReason::NoModelsAvailable),
        );

        assert!(matches!(
            state.entry_intent,
            LauncherEntryIntent::InitialSetup
        ));
        assert!(matches!(state.step, LauncherStep::WaitingForStart));
        assert!(state.startup_reason.is_none());

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn existing_launcher_authority_with_config_load_failure_enters_recovery_wait() {
        let test_dir = unique_test_dir();
        fs::create_dir_all(&test_dir).expect("create test dir");

        let config_path = test_dir.join("launcher_config.toml");
        fs::write(&config_path, "ui_lang = \"ja\"\n").expect("write launcher config");

        let state = build_initial_launcher_state(
            &config_path,
            true,
            &Err(CheckReadyReason::ConfigLoadFail("parse error".to_string())),
        );

        assert!(matches!(
            state.entry_intent,
            LauncherEntryIntent::RecoveryWait
        ));
        assert!(matches!(state.step, LauncherStep::WaitingForStart));
        assert_eq!(
            state.startup_reason.as_deref(),
            Some("launcher_config.toml load failed: parse error")
        );

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn complete_backend_handoff_failure_does_not_queue_start() {
        let (tx, rx) = mpsc::channel();

        let result = complete_backend_handoff(&tx, Err("backend failed".to_string()));

        assert_eq!(result.unwrap_err(), "backend failed");
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn complete_backend_handoff_success_queues_start() {
        let (tx, rx) = mpsc::channel();

        complete_backend_handoff(&tx, Ok(())).expect("handoff should queue start");

        assert!(matches!(
            rx.try_recv(),
            Ok(crate::messages::FrontendCommand::Start)
        ));
    }

    #[test]
    fn create_new_dict_slot_command_uses_current_target_and_set_dict_slot_path() {
        let base_dir = unique_test_dir();
        let text_dir = base_dir.join("dicts").join("ja").join("text");
        fs::create_dir_all(text_dir.join("ja_001")).expect("create existing ja slot");

        let (slot, command) = create_new_dict_slot_command_for_target("ja", &base_dir);
        let expected = text_dir.join("ja_002").to_string_lossy().to_string();

        assert_eq!(slot, expected);
        assert!(text_dir.join("ja_002").is_dir());
        match command {
            crate::messages::FrontendCommand::SetDictSlot(path) => {
                assert_eq!(path, expected);
                assert_ne!(
                    path,
                    "\u{ff0b} \u{65b0}\u{3057}\u{3044}\u{8f9e}\u{66f8}\u{3092}\u{4f5c}\u{6210}"
                );
            }
            crate::messages::FrontendCommand::SetLanguagePair { .. } => {
                panic!("new dict slot action must not call SetLanguagePair");
            }
            other => panic!("unexpected command: {:?}", other),
        }

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn reset_backend_runtime_discards_stale_commands_and_resets_shutdown() {
        let (mut tx, rx) = mpsc::channel();
        tx.send(crate::messages::FrontendCommand::Start)
            .expect("queue stale start");

        let mut command_rx = None;
        let mut shutdown = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let finished_clone = finished.clone();
        let shutdown_clone = shutdown.clone();

        let mut backend_thread = Some(thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                let _ = rx.recv_timeout(Duration::from_millis(20));
            }
            finished_clone.store(true, Ordering::Relaxed);
        }));

        reset_backend_runtime(&mut tx, &mut command_rx, &mut backend_thread, &mut shutdown);

        assert!(backend_thread.is_none());
        assert!(finished.load(Ordering::Relaxed));
        assert!(!shutdown.load(Ordering::Relaxed));

        let new_rx = command_rx.as_ref().expect("new receiver should exist");
        assert!(matches!(new_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        tx.send(crate::messages::FrontendCommand::Restart)
            .expect("new channel should accept commands");
        assert!(matches!(
            new_rx.try_recv(),
            Ok(crate::messages::FrontendCommand::Restart)
        ));
    }
}

