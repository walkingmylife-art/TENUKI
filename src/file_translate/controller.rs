//! FileTranslateController — File Translate / List mode の UI ロジックを TenukiApp から分離。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use crate::backend;
use crate::config;
use crate::file_translate::asset_intake::{load_source_preview, scan_asset_sources_with_progress};
use crate::file_translate::commands::FileTranslateUiCommand;
use crate::file_translate::preview::{build_preview_summary, build_run_log_seed};
use crate::file_translate::runner::run_file_translate;
use crate::file_translate::state::{
    evaluate_run_readiness, DictSlotAction, FileTranslatePreviewMessage, FileTranslateScanMessage,
};
use crate::file_translate::types::{
    ColumnMode, FileTranslateRunConfig, HeaderMode, PreviewState, SourceKind,
};
use crate::launcher;
use crate::messages::{BackendEvent, LogLevel};
use crate::ui::container::{LeftPanelTab, UiContainer};
use crate::ui::list_text::{self, ListText};

pub struct FileTranslateController {
    pub base_dir: PathBuf,
    pub config_path: PathBuf,
    pub event_tx: mpsc::Sender<BackendEvent>,
    pub file_translate_cancel: Option<Arc<AtomicBool>>,
}

impl FileTranslateController {
    pub fn file_translate_readiness(
        ui: &UiContainer,
        base_dir: &PathBuf,
    ) -> crate::file_translate::state::FileTranslateRunReadiness {
        evaluate_run_readiness(
            &ui.state.file_translate,
            ui.display.dict_slot.as_deref(),
            &ui.display.tgt_lang,
            base_dir,
        )
    }

    pub fn refresh_file_translate_summary(&self, ui: &mut UiContainer) {
        let readiness = Self::file_translate_readiness(ui, &self.base_dir);
        let preview = ui.state.file_translate.preview.clone();
        let column_modes = ui.state.file_translate.column_modes.clone();
        let selected_source = ui.state.file_translate.selected_source.clone();
        let root = ui.state.file_translate.root.clone();
        let sources = ui.state.file_translate.sources.clone();

        match &selected_source {
            Some(file) => {
                let (text, is_error) = build_preview_summary(
                    &ui.display.ui_lang,
                    &preview,
                    &column_modes,
                    &readiness,
                );
                ui.set_work_result(file.display().to_string(), text, is_error);
            }
            None => {
                let lang = &ui.display.ui_lang;
                let root = root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string());
                let summary = [
                    list_text::field(lang, ListText::Root, root),
                    list_text::field(lang, ListText::Sources, sources.len()),
                    list_text::field(
                        lang,
                        ListText::Delimited,
                        sources
                            .iter()
                            .filter(|source| source.kind == SourceKind::DelimitedText)
                            .count(),
                    ),
                    list_text::field(
                        lang,
                        ListText::Json,
                        sources
                            .iter()
                            .filter(|source| source.kind == SourceKind::JsonText)
                            .count(),
                    ),
                    list_text::field(
                        lang,
                        ListText::Markup,
                        sources
                            .iter()
                            .filter(|source| source.kind == SourceKind::MarkupText)
                            .count(),
                    ),
                    list_text::field(
                        lang,
                        ListText::PlainLines,
                        sources
                            .iter()
                            .filter(|source| source.kind == SourceKind::PlainLines)
                            .count(),
                    ),
                    list_text::field(
                        lang,
                        ListText::UnsupportedBinary,
                        sources
                            .iter()
                            .filter(|source| source.kind == SourceKind::UnsupportedBinary)
                            .count(),
                    ),
                    list_text::text(lang, ListText::SelectSourceToPreview).to_string(),
                ]
                .join("\n");
                ui.set_work_result(
                    list_text::text(lang, ListText::FileTranslateTitle).to_string(),
                    summary,
                    false,
                );
            }
        }
    }

    pub fn start_file_translate_scan(&mut self, ui: &mut UiContainer, selection: PathBuf) {
        let root = if selection.is_dir() {
            selection.clone()
        } else {
            selection
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.base_dir.clone())
        };
        let (scan_tx, scan_rx) = mpsc::channel();
        ui.state
            .file_translate
            .reset_for_root(Some(root.clone()), Vec::new());
        ui.state.file_translate.enter_list_mode();
        ui.state.file_translate.scan_in_progress = true;
        ui.state.file_translate.scan_rx = Some(scan_rx);
        ui.state.log_panel_tab = LeftPanelTab::List;
        ui.set_work_running(false);
        ui.reset_file_translate_progress();
        ui.reset_file_translate_logs(vec![format!("[scan] root: {}", root.display())]);
        ui.set_file_translate_status_text(
            list_text::text(&ui.display.ui_lang, ListText::Scanning).to_string(),
        );
        self.refresh_file_translate_summary(ui);

        thread::spawn(move || {
            let sources = scan_asset_sources_with_progress(&root, |index, candidate| {
                let _ = scan_tx.send(FileTranslateScanMessage::Scanned {
                    index,
                    path: candidate.path.clone(),
                });
            });
            let _ = scan_tx.send(FileTranslateScanMessage::Done { root, sources });
        });
    }

    pub fn poll_file_translate_scan(&mut self, ui: &mut UiContainer) {
        let mut messages = Vec::new();
        let mut done = None;

        if let Some(rx) = &ui.state.file_translate.scan_rx {
            loop {
                match rx.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        done = Some((
                            ui.state
                                .file_translate
                                .root
                                .clone()
                                .unwrap_or_else(|| self.base_dir.clone()),
                            Vec::new(),
                        ));
                        break;
                    }
                }
            }
        }

        for message in messages {
            match message {
                FileTranslateScanMessage::Scanned { index, path } => {
                    ui.append_file_translate_log(
                        format!("[scan][{}] {}", index, path.display()),
                        LogLevel::Info,
                    );
                }
                FileTranslateScanMessage::Done { root, sources } => {
                    done = Some((root, sources));
                }
            }
        }

        if let Some((root, sources)) = done {
            let first_source = sources.first().map(|source| source.path.clone());
            let file_count = sources.len();
            ui.state
                .file_translate
                .reset_for_root(Some(root), sources);
            ui.state.file_translate.scan_rx = None;
            ui.append_file_translate_log(
                format!(
                    "[scan] {}",
                    list_text::scan_done(&ui.display.ui_lang, file_count)
                ),
                LogLevel::Info,
            );
            ui.set_file_translate_status_text(list_text::scan_done(
                &ui.display.ui_lang,
                file_count,
            ));

            if let Some(file) = first_source {
                self.select_file_translate_source(ui, file);
            } else {
                self.refresh_file_translate_summary(ui);
            }
        }
    }

    fn start_file_translate_preview_load(
        &self,
        ui: &mut UiContainer,
        file: PathBuf,
        header_mode: HeaderMode,
        reset_columns: bool,
    ) {
        let (preview_tx, preview_rx) = mpsc::channel();
        {
            let state = &mut ui.state.file_translate;
            state.selected_source = Some(file.clone());
            state.preview = PreviewState::Empty;
            state.preview_loading = true;
            state.preview_target = Some(file.clone());
            state.preview_header_mode = header_mode;
            state.table_preview_row_limit = 100;
            state.text_preview_line_limit =
                crate::ui::file_translate_panel::TEXT_PREVIEW_INITIAL_LINE_LIMIT;
            state.preview_rx = Some(preview_rx);
            if reset_columns {
                state.column_modes.clear();
            }
        }
        ui.set_file_translate_status_text(
            list_text::text(&ui.display.ui_lang, ListText::LoadingPreview).to_string(),
        );
        self.refresh_file_translate_summary(ui);

        thread::spawn(move || {
            let result = load_source_preview(&file, header_mode).map_err(|err| err.to_string());
            let _ = preview_tx.send(FileTranslatePreviewMessage::Done {
                file,
                header_mode,
                result,
            });
        });
    }

    pub fn poll_file_translate_preview(&self, ui: &mut UiContainer) {
        let mut messages = Vec::new();
        let mut disconnected = false;

        if let Some(rx) = &ui.state.file_translate.preview_rx {
            loop {
                match rx.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        let mut refresh = false;
        for message in messages {
            match message {
                FileTranslatePreviewMessage::Done {
                    file,
                    header_mode,
                    result,
                } => {
                    let state = &mut ui.state.file_translate;
                    let matches_current = state.selected_source.as_ref() == Some(&file)
                        && state.preview_target.as_ref() == Some(&file)
                        && state.preview_header_mode == header_mode;
                    if !matches_current {
                        continue;
                    }

                    state.preview_loading = false;
                    state.preview_target = None;
                    state.preview_rx = None;
                    state.preview = match result {
                        Ok(preview) => {
                            let preview = if header_mode == HeaderMode::Unknown {
                                match preview {
                                    crate::file_translate::types::SourcePreview::Table(table)
                                        if table.supports_header_toggle() =>
                                    {
                                        let resolved_mode = if table.suggested_header {
                                            HeaderMode::Present
                                        } else {
                                            HeaderMode::Absent
                                        };
                                        state.preview_header_mode = resolved_mode;
                                        crate::file_translate::types::SourcePreview::Table(
                                            crate::file_translate::asset_intake::apply_delimited_header_mode_from_unknown(
                                                table,
                                                resolved_mode,
                                            ),
                                        )
                                    }
                                    other => other,
                                }
                            } else {
                                preview
                            };

                            PreviewState::Ready(preview)
                        }
                        Err(err) => PreviewState::Error(err),
                    };
                    refresh = true;
                }
            }
        }

        if disconnected && ui.state.file_translate.preview_loading {
            let state = &mut ui.state.file_translate;
            state.preview_loading = false;
            state.preview_target = None;
            state.preview_rx = None;
            state.preview = PreviewState::Error("preview worker disconnected".to_string());
            refresh = true;
        }

        if refresh {
            self.refresh_file_translate_summary(ui);
        }
    }

    pub fn select_file_translate_source(&self, ui: &mut UiContainer, file: PathBuf) {
        self.start_file_translate_preview_load(ui, file, HeaderMode::Unknown, true);
    }

    pub fn set_file_translate_column_mode(
        &self,
        ui: &mut UiContainer,
        file: PathBuf,
        column: usize,
        mode: ColumnMode,
    ) {
        if ui.state.file_translate.selected_source.as_ref() != Some(&file) {
            return;
        }

        if mode == ColumnMode::None {
            ui.state.file_translate.column_modes.remove(&column);
        } else {
            ui.state
                .file_translate
                .column_modes
                .insert(column, mode);
        }
        self.refresh_file_translate_summary(ui);
    }

    pub fn set_file_translate_header_mode(
        &self,
        ui: &mut UiContainer,
        file: PathBuf,
        mode: HeaderMode,
    ) {
        if ui.state.file_translate.selected_source.as_ref() != Some(&file) {
            return;
        }

        self.start_file_translate_preview_load(ui, file, mode, false);
    }

    pub fn resolve_file_translate_slot(
        &self,
        ui: &mut UiContainer,
        action: DictSlotAction,
    ) -> Result<PathBuf, String> {
        match action {
            DictSlotAction::UseCommitted(path) => {
                fs::create_dir_all(&path)
                    .map_err(|e| format!("slot directory create failed: {}", e))?;
                ui.append_file_translate_log(
                    format!(
                        "[slot] using committed output directory: {}",
                        path.display()
                    ),
                    LogLevel::Info,
                );
                Ok(path)
            }
            DictSlotAction::CreateForRun {
                parent,
                target_lang,
            } => {
                let slot = parent.join("list_output");
                let committed_mismatch = ui
                    .display
                    .dict_slot
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .is_some_and(|path| {
                        !backend::manager::dict_slot_matches_target(
                            Path::new(path),
                            &target_lang,
                        )
                    });
                let message = if committed_mismatch {
                    format!(
                        "[slot] committed slot does not match target {}; using List output directory: {}",
                        target_lang,
                        slot.display()
                    )
                } else {
                    format!("[slot] List output directory: {}", slot.display())
                };
                ui.append_file_translate_log(message, LogLevel::Info);
                Ok(slot)
            }
        }
    }

    pub fn start_file_translate_run(&mut self, ui: &mut UiContainer) {
        let readiness = Self::file_translate_readiness(ui, &self.base_dir);
        if !readiness.is_ready() {
            let message = list_text::readiness(&ui.display.ui_lang, &readiness);
            ui.set_work_result(
                list_text::text(&ui.display.ui_lang, ListText::FileTranslateTitle).to_string(),
                message.clone(),
                true,
            );
            ui.append_file_translate_log(format!("[need] {}", message), LogLevel::Error);
            return;
        }

        let table_source = readiness
            .table_source
            .clone()
            .expect("ready state must include table source");
        let selected_file = table_source.file.clone();

        let config = match config::load(&self.config_path) {
            Ok(config) => config,
            Err(err) => {
                ui.set_work_result(
                    list_text::text(&ui.display.ui_lang, ListText::FileTranslateTitle)
                        .to_string(),
                    format!("config load failed: {}", err),
                    true,
                );
                return;
            }
        };

        let dict_slot = match self.resolve_file_translate_slot(
            ui,
            readiness
                .dict_slot_action
                .clone()
                .expect("ready state must include dict slot action"),
        ) {
            Ok(slot) => slot,
            Err(err) => {
                ui.set_work_result(
                    list_text::text(&ui.display.ui_lang, ListText::FileTranslateTitle)
                        .to_string(),
                    err,
                    true,
                );
                return;
            }
        };

        let launcher_config_path =
            launcher::resolve_install_root().join("launcher_config.toml");
        let parallel_slots = launcher::app_config::AppConfig::load(&launcher_config_path)
            .map(|c| c.server.parallel_slots.max(1) as usize)
            .unwrap_or(1);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let run_config = FileTranslateRunConfig {
            source: table_source,
            dict_slot,
            column_modes: ui.state.file_translate.column_modes.clone(),
            ui_lang: ui.display.ui_lang.clone(),
            server_host: config.server_host.clone(),
            server_port: config.server_port,
            chunk_size: config.list.effective_chunk_size(parallel_slots),
            request_timeout_secs: config.list.effective_timeout_secs(),
            cancel_flag: cancel_flag.clone(),
            event_tx: self.event_tx.clone(),
        };

        self.file_translate_cancel = Some(cancel_flag);
        ui.state.log_panel_tab = LeftPanelTab::List;
        ui.set_work_running(true);
        ui.reset_file_translate_progress();
        ui.set_file_translate_status_text(
            list_text::text(&ui.display.ui_lang, ListText::Started).to_string(),
        );
        ui.append_file_translate_log("[run] started".to_string(), LogLevel::Info);
        for line in build_run_log_seed(
            &ui.display.ui_lang,
            &ui.state.file_translate.preview,
            &ui.state.file_translate.column_modes,
            &readiness,
        ) {
            ui.append_file_translate_log(line, LogLevel::Info);
        }
        ui.set_work_result(selected_file.display().to_string(), "".to_string(), false);

        let event_tx = self.event_tx.clone();
        thread::spawn(move || {
            let outcome = run_file_translate(run_config);
            let _ = event_tx.send(BackendEvent::WorkResult {
                title: outcome.title,
                text: outcome.text,
                is_error: outcome.is_error,
            });
        });
    }

    pub fn stop_file_translate_run(&mut self, ui: &mut UiContainer) {
        if let Some(cancel_flag) = &self.file_translate_cancel {
            cancel_flag.store(true, Ordering::Relaxed);
            ui.append_file_translate_log("[stop] requested".to_string(), LogLevel::Info);
            ui.set_file_translate_status_text(
                list_text::text(&ui.display.ui_lang, ListText::Stopping).to_string(),
            );
        }
    }

    pub fn handle_file_translate_command(
        &mut self,
        ui: &mut UiContainer,
        command: FileTranslateUiCommand,
    ) {
        match command {
            FileTranslateUiCommand::StartFileTranslateScan(path) => {
                self.start_file_translate_scan(ui, path);
            }
            FileTranslateUiCommand::SelectFileTranslateSource(path) => {
                self.select_file_translate_source(ui, path);
            }
            FileTranslateUiCommand::SetFileTranslateColumnMode { file, column, mode } => {
                self.set_file_translate_column_mode(ui, file, column, mode);
            }
            FileTranslateUiCommand::SetFileTranslateHeaderMode { file, mode } => {
                self.set_file_translate_header_mode(ui, file, mode);
            }
            FileTranslateUiCommand::RunFileTranslate => {
                self.start_file_translate_run(ui);
            }
            FileTranslateUiCommand::StopFileTranslate => {
                self.stop_file_translate_run(ui);
            }
        }
    }
}
