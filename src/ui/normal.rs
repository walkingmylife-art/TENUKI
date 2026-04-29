use crate::launcher::app_config::{known_model_tuple, ModelConfig, UrlPair};
use crate::messages::{LogLevel, ModelCandidateKind};
use crate::ui::container::{
    LangPanelTab, LeftPanelTab, LogEntry, UiCommands, UiDisplayData, UiState,
};
use crate::ui::list_text::{self, ListText};
use crate::ui::normal_text::{self, NormalText, TopModeText};
use eframe::egui;
use regex;
use std::path::Path;

const LANGS: &[&str] = crate::config::TARGET_LANGUAGE_PRESETS;
const UI_LANGS: &[&str] = &["ja", "en", "zh-CN"];
const STATUS_LAMP_CELL_WIDTH: f32 = 18.0;
const STATUS_LAMP_AFTER_SPACE: f32 = 0.0;

fn is_preset_lang(code: &str) -> bool {
    crate::config::is_target_language_preset(code)
}

fn next_log_panel_tab(current: LeftPanelTab, list_available: bool) -> LeftPanelTab {
    match current {
        LeftPanelTab::TenukiLog => LeftPanelTab::ServerLog,
        LeftPanelTab::ServerLog => LeftPanelTab::Dictionary,
        LeftPanelTab::Dictionary if list_available => LeftPanelTab::List,
        LeftPanelTab::Dictionary | LeftPanelTab::List => LeftPanelTab::TenukiLog,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopModeChoice {
    Game,
    Normal,
    List,
}

impl TopModeChoice {
    fn text_key(self) -> TopModeText {
        match self {
            Self::Game => TopModeText::Game,
            Self::Normal => TopModeText::Normal,
            Self::List => TopModeText::List,
        }
    }
}

fn selected_top_mode(data: &UiDisplayData, state: &UiState) -> TopModeChoice {
    if state.file_translate.is_list_mode() {
        TopModeChoice::List
    } else if data.profile == "game" {
        TopModeChoice::Game
    } else {
        TopModeChoice::Normal
    }
}

fn select_live_profile(
    state: &mut UiState,
    commands: &mut UiCommands,
    current_profile: &str,
    target_profile: &str,
) {
    state.file_translate.leave_list_mode();
    if state.log_panel_tab == LeftPanelTab::List {
        state.log_panel_tab = LeftPanelTab::TenukiLog;
    }
    if current_profile != target_profile {
        commands.set_profile = Some(target_profile.to_string());
    }
}

fn select_top_mode(
    choice: TopModeChoice,
    data: &UiDisplayData,
    state: &mut UiState,
    commands: &mut UiCommands,
) {
    match choice {
        TopModeChoice::Game => select_live_profile(state, commands, &data.profile, "game"),
        TopModeChoice::Normal => select_live_profile(state, commands, &data.profile, "normal"),
        TopModeChoice::List => {
            if !state.file_translate.is_list_mode() {
                crate::ui::file_translate_panel::toggle_list_mode(state, &data.base_dir);
            }
            if state.file_translate.is_list_mode() {
                state.log_panel_tab = LeftPanelTab::List;
            }
        }
    }
}

fn dict_slot_for_target_commit(
    current_tgt_lang: &str,
    next_tgt_lang: &str,
    current_slot: Option<&str>,
) -> Option<String> {
    let slot = current_slot
        .map(str::trim)
        .filter(|slot| !slot.is_empty())?;

    if current_tgt_lang == next_tgt_lang
        || crate::backend::manager::dict_slot_matches_target(Path::new(slot), next_tgt_lang)
    {
        Some(slot.to_string())
    } else {
        None
    }
}

fn render_status_lamp(
    ui: &mut egui::Ui,
    l: &str,
    running: bool,
    status_icon: crate::ui::container::StatusIcon,
) {
    let color = if running {
        egui::Color32::GREEN
    } else if status_icon == crate::ui::container::StatusIcon::Spinner {
        egui::Color32::from_rgb(255, 200, 50)
    } else {
        egui::Color32::RED
    };

    ui.add_sized(
        [STATUS_LAMP_CELL_WIDTH, 0.0],
        egui::Label::new(
            egui::RichText::new(normal_text::text(l, NormalText::StatusLamp))
                .color(color)
                .size(14.0),
        ),
    );
    ui.add_space(STATUS_LAMP_AFTER_SPACE);
}

fn render_mode_combo(
    ui: &mut egui::Ui,
    data: &UiDisplayData,
    state: &mut UiState,
    commands: &mut UiCommands,
) {
    let current = selected_top_mode(data, state);
    egui::ComboBox::from_id_salt("mode_combo")
        .width(92.0)
        .selected_text(normal_text::top_mode_label(
            &data.ui_lang,
            current.text_key(),
        ))
        .show_ui(ui, |ui| {
            for choice in [
                TopModeChoice::Game,
                TopModeChoice::Normal,
                TopModeChoice::List,
            ] {
                if ui
                    .selectable_label(
                        current == choice,
                        normal_text::top_mode_label(&data.ui_lang, choice.text_key()),
                    )
                    .clicked()
                {
                    select_top_mode(choice, data, state, commands);
                }
            }
        });
}

fn render_dict_slot_button(ui: &mut egui::Ui, data: &UiDisplayData, state: &mut UiState) {
    let l = &data.ui_lang;
    let dict_btn_text_row = match &data.dict_slot {
        Some(p) => std::path::Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| normal_text::text(l, NormalText::Dict).into()),
        None => normal_text::text(l, NormalText::DictNone).to_string(),
    };
    if ui
        .button(egui::RichText::new(dict_btn_text_row).size(14.0))
        .clicked()
        && state.dict_slot_rx.is_none()
    {
        let (tx, rx) = std::sync::mpsc::channel();
        state.dict_slot_rx = Some(rx);
        let default_dir = data
            .base_dir
            .join("dicts")
            .join(&data.tgt_lang)
            .join("text");
        std::thread::spawn(move || {
            let _ = tx.send(
                rfd::FileDialog::new()
                    .set_directory(&default_dir)
                    .pick_folder(),
            );
        });
    }
}

fn draw_log_entries(ui: &mut egui::Ui, logs: &std::collections::VecDeque<LogEntry>, empty: &str) {
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if logs.is_empty() {
                ui.colored_label(egui::Color32::GRAY, empty);
                return;
            }
            for entry in logs {
                let color = match entry.level {
                    LogLevel::Info => egui::Color32::from_rgb(200, 200, 200),
                    LogLevel::Success => egui::Color32::from_rgb(100, 200, 100),
                    LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(entry.format())
                            .color(color)
                            .family(egui::FontFamily::Monospace)
                            .size(13.0),
                    )
                    .selectable(true),
                );
            }
        });
}

fn draw_dictionary_history(ui: &mut egui::Ui, data: &UiDisplayData, empty: &str) {
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if data.dictionary_history.is_empty() {
                ui.colored_label(egui::Color32::GRAY, empty);
                return;
            }
            for (timestamp, original, translated) in &data.dictionary_history {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(timestamp).monospace().size(12.0));
                    ui.label(
                        egui::RichText::new(original)
                            .size(13.0)
                            .color(egui::Color32::from_rgb(220, 220, 215)),
                    );
                    ui.label(
                        egui::RichText::new(translated)
                            .size(13.0)
                            .color(egui::Color32::from_rgb(110, 160, 210)),
                    );
                });
            }
        });
}

fn draw_file_translate_log_entries(
    ui: &mut egui::Ui,
    logs: &std::collections::VecDeque<LogEntry>,
    empty: &str,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if logs.is_empty() {
                ui.colored_label(egui::Color32::GRAY, empty);
                return;
            }
            let re_source = regex::Regex::new(r"^\[\d+/\d+\]").unwrap();
            for entry in logs {
                let msg = &entry.message;
                let color = if entry.level == LogLevel::Error {
                    egui::Color32::from_rgb(255, 100, 100)
                } else if msg.starts_with("[done]") {
                    egui::Color32::from_rgb(100, 200, 100)
                } else if msg.starts_with("=> ") {
                    // model/target 色（辞書履歴の訳語と同じ青系）
                    egui::Color32::from_rgb(110, 160, 210)
                } else if re_source.is_match(msg) {
                    // source 色（辞書履歴の原文と同じ白系）
                    egui::Color32::from_rgb(220, 220, 215)
                } else {
                    egui::Color32::from_rgb(200, 200, 200)
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(entry.format())
                            .color(color)
                            .family(egui::FontFamily::Monospace)
                            .size(13.0),
                    )
                    .selectable(true),
                );
            }
        });
}

pub fn show_normal_ui(
    ctx: &egui::Context,
    data: &UiDisplayData,
    state: &mut UiState,
    commands: &mut UiCommands,
) {
    crate::ui::file_translate_panel::poll_folder_picker(state, commands);
    if !state.file_translate.is_list_mode() && state.log_panel_tab == LeftPanelTab::List {
        state.log_panel_tab = LeftPanelTab::TenukiLog;
    }
    let l = &data.ui_lang;
    let no_logs = normal_text::text(l, NormalText::NoLogsYet);
    let no_history = normal_text::text(l, NormalText::NoEntriesYet);

    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            render_status_lamp(ui, l, data.tenuki_running, data.status_icon);

            let tgt_label = if LANGS.contains(&data.tgt_lang.as_str()) {
                normal_text::target_language_label(&data.ui_lang, &data.tgt_lang)
            } else {
                data.tgt_lang.as_str()
            };
            let btn = ui.button(egui::RichText::new(tgt_label).size(14.0));
            if btn.clicked() {
                state.show_lang_panel = !state.show_lang_panel;
                if state.show_lang_panel {
                    state.lang_panel_anchor = btn.rect.left_bottom();
                    state.pending_tgt_lang = Some(data.tgt_lang.clone());
                    state.custom_tgt_val_buf = if is_preset_lang(&data.tgt_lang) {
                        String::new()
                    } else {
                        data.tgt_lang.clone()
                    };
                    state.custom_tgt_name_buf = if !is_preset_lang(&data.tgt_lang) {
                        data.custom_lang_name.clone()
                    } else {
                        String::new()
                    };
                }
            }
            render_dict_slot_button(ui, data, state);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(normal_text::text(l, NormalText::Exit)).clicked() {
                    commands.exit_app = true;
                }
                ui.add_space(4.0);
                if ui.button(normal_text::text(l, NormalText::Log)).clicked() {
                    state.log_panel_tab =
                        next_log_panel_tab(state.log_panel_tab, state.file_translate.is_list_mode());
                }
                ui.add_space(4.0);
                if state.file_translate.is_list_mode() {
                    ui.add_space(4.0);
                    let run_label = if data.work_running {
                        list_text::text(l, ListText::Stop)
                    } else {
                        list_text::text(l, ListText::Run)
                    };
                    let run_enabled = data.work_running
                        || crate::ui::file_translate_panel::can_run_from_toolbar(data, state);
                    if ui
                        .add_enabled(run_enabled, egui::Button::new(run_label))
                        .clicked()
                    {
                        commands.file_translate_commands.push(if data.work_running {
                            crate::file_translate::commands::FileTranslateUiCommand::StopFileTranslate
                        } else {
                            crate::file_translate::commands::FileTranslateUiCommand::RunFileTranslate
                        });
                    }
                }
                ui.add_space(4.0);
                if let Some(rx) = &state.dict_slot_rx {
                    if let Ok(result) = rx.try_recv() {
                        if let Some(path) = result {
                            commands.set_dict_slot = Some(path.to_string_lossy().to_string());
                        }
                        state.dict_slot_rx = None;
                    }
                }
            });
        });

        ui.separator();
        ui.horizontal(|ui| {
            render_status_lamp(ui, l, data.llama_running, data.status_icon);
            render_mode_combo(ui, data, state, commands);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    normal_text::text(l, NormalText::Dict),
                    data.dictionary_loaded
                ))
                .size(13.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let selected_name = data
                    .selected_model
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| {
                        if data.available_models.is_empty() {
                            normal_text::text(l, NormalText::NoModel).to_string()
                        } else {
                            normal_text::text(l, NormalText::SelectModel).to_string()
                        }
                    });
                egui::ComboBox::from_id_salt("model_combo")
                    .width(220.0)
                    .selected_text(&selected_name)
                    .show_ui(ui, |ui| {
                        if data.available_models.is_empty() {
                            ui.label(normal_text::text(l, NormalText::NoModels));
                        } else {
                            for candidate in &data.available_models {
                                let tag = normal_text::model_kind_tag(l, &candidate.kind);
                                let label = format!("{} {}", tag, candidate.filename);
                                let is_selected = candidate.filename == selected_name;
                                if ui.selectable_label(is_selected, &label).clicked() {
                                    let model_config = match &candidate.kind {
                                        ModelCandidateKind::Known => {
                                            known_model_tuple(&candidate.filename).map(|t| {
                                                ModelConfig::Known {
                                                    filename: t.filename.to_string(),
                                                    expected_size: t.expected_size,
                                                    urls: UrlPair::single(t.url),
                                                }
                                            })
                                        }
                                        ModelCandidateKind::Local => Some(ModelConfig::Local {
                                            filename: candidate.filename.clone(),
                                            expected_size: candidate.size,
                                        }),
                                    };
                                    if let Some(mc) = model_config {
                                        commands.select_model = Some(mc);
                                    }
                                }
                            }
                        }
                    });
            });
        });
    });

    if state.show_lang_panel {
        let mut ok_clicked = false;
        egui::Window::new("lang_panel")
            .title_bar(false)
            .resizable(false)
            .fixed_pos(state.lang_panel_anchor)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Tgt,
                            normal_text::text(l, NormalText::Target),
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Tgt;
                    }
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Ui,
                            normal_text::text(l, NormalText::Display),
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Ui;
                    }
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Network,
                            normal_text::text(l, NormalText::Network),
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Network;
                    }
                    ui.add_space(8.0);
                    if ui
                        .button(
                            egui::RichText::new(normal_text::text(l, NormalText::Ok)).size(14.0),
                        )
                        .clicked()
                    {
                        ok_clicked = true;
                    }
                });
                ui.separator();
                ui.set_min_width(220.0);

                let is_custom = state.lang_panel_tab == LangPanelTab::Tgt
                    && state
                        .pending_tgt_lang
                        .as_deref()
                        .is_some_and(|lang| !LANGS.contains(&lang));
                if state.lang_panel_tab != LangPanelTab::Network {
                    let list = match state.lang_panel_tab {
                        LangPanelTab::Ui => UI_LANGS,
                        _ => LANGS,
                    };
                    for code in list {
                        let is_selected = match state.lang_panel_tab {
                            LangPanelTab::Tgt => state.pending_tgt_lang.as_deref() == Some(*code),
                            LangPanelTab::Ui => data.ui_lang.as_str() == *code,
                            _ => false,
                        };
                        let mut btn = egui::Button::new(normal_text::target_language_label(
                            &data.ui_lang,
                            code,
                        ))
                        .min_size(egui::vec2(ui.available_width(), 0.0));
                        if is_selected {
                            btn = btn.fill(egui::Color32::from_rgb(70, 90, 120));
                        }
                        if ui.add(btn).clicked() {
                            match state.lang_panel_tab {
                                LangPanelTab::Tgt => {
                                    state.pending_tgt_lang = Some((*code).to_string());
                                    state.custom_tgt_val_buf.clear();
                                    state.custom_tgt_name_buf.clear();
                                }
                                LangPanelTab::Ui => {
                                    commands.set_ui_lang = Some((*code).to_string());
                                }
                                _ => {}
                            }
                        }
                    }
                }

                if state.lang_panel_tab == LangPanelTab::Network {
                    ui.separator();
                    ui.label(format!("{}:", normal_text::text(l, NormalText::Host)));
                    let host_resp = ui.add(
                        egui::TextEdit::singleline(&mut state.network_host_buf)
                            .hint_text(normal_text::text(l, NormalText::HostPlaceholder))
                            .desired_width(ui.available_width() - 4.0)
                            .font(egui::TextStyle::Small),
                    );
                    let host_commit = host_resp.lost_focus()
                        && host_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter));
                    if host_commit {
                        commands.set_server_host = Some(state.network_host_buf.trim().to_string());
                    }

                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", normal_text::text(l, NormalText::Port)));
                        let mut port_str = state.network_port_buf.clone();
                        let port_resp = ui.add(
                            egui::TextEdit::singleline(&mut port_str)
                                .hint_text(normal_text::text(l, NormalText::PortPlaceholder))
                                .desired_width(80.0)
                                .font(egui::TextStyle::Small),
                        );
                        if port_resp.changed() {
                            state.network_port_buf = port_str.clone();
                        }
                        let port_commit = port_resp.lost_focus()
                            && port_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter));
                        if port_commit {
                            if let Ok(p) = state.network_port_buf.parse::<u16>() {
                                commands.set_server_port = Some(p);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(
                                    egui::RichText::new(normal_text::text(
                                        l,
                                        NormalText::ResetToLocal,
                                    ))
                                    .size(12.0),
                                )
                                .clicked()
                            {
                                let local_host = normal_text::local_host_value();
                                state.network_host_buf = local_host.to_string();
                                commands.set_server_host = Some(local_host.to_string());
                                commands.restart_backend = true;
                            }
                        });
                    });
                    ui.separator();
                    let note = if data.server_host == normal_text::public_host_value() {
                        normal_text::text(l, NormalText::NetworkAccessible)
                    } else {
                        normal_text::text(l, NormalText::LocalOnly)
                    };
                    ui.label(
                        egui::RichText::new(note)
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                    );
                }

                if state.lang_panel_tab == LangPanelTab::Tgt {
                    let mut custom_btn =
                        egui::Button::new(normal_text::text(l, NormalText::CustomLanguage))
                            .min_size(egui::vec2(ui.available_width(), 0.0));
                    if is_custom {
                        custom_btn = custom_btn.fill(egui::Color32::from_rgb(70, 90, 120));
                    }
                    ui.add(custom_btn);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", normal_text::text(l, NormalText::Code)));
                        let code_resp = ui.add(
                            egui::TextEdit::singleline(&mut state.custom_tgt_val_buf)
                                .id(egui::Id::new("custom_lang_val_edit"))
                                .hint_text(normal_text::text(
                                    l,
                                    NormalText::CustomLanguageCodePlaceholder,
                                ))
                                .desired_width(80.0)
                                .font(egui::TextStyle::Small),
                        );
                        if code_resp.changed() {
                            let code = state.custom_tgt_val_buf.trim().to_string();
                            state.pending_tgt_lang = Some(if code.is_empty() {
                                data.tgt_lang.clone()
                            } else {
                                code
                            });
                        }
                        let enter = code_resp.lost_focus()
                            && code_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter));
                        if enter {
                            let tgt = state.custom_tgt_val_buf.trim().to_string();
                            if tgt.is_empty() {
                                code_resp.ctx.memory_mut(|m| {
                                    m.request_focus(egui::Id::new("custom_lang_val_edit"))
                                });
                            } else if state.custom_tgt_name_buf.trim().is_empty() {
                                code_resp.ctx.memory_mut(|m| {
                                    m.request_focus(egui::Id::new("custom_lang_name_edit"))
                                });
                            } else {
                                ok_clicked = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", normal_text::text(l, NormalText::Name)));
                        let name_resp = ui.add(
                            egui::TextEdit::singleline(&mut state.custom_tgt_name_buf)
                                .id(egui::Id::new("custom_lang_name_edit"))
                                .hint_text(normal_text::text(
                                    l,
                                    NormalText::CustomLanguageNamePlaceholder,
                                ))
                                .desired_width(ui.available_width() - 4.0)
                                .font(egui::TextStyle::Small),
                        );
                        let enter = name_resp.lost_focus()
                            && name_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter));
                        if enter {
                            let tgt = state.custom_tgt_val_buf.trim().to_string();
                            let name = state.custom_tgt_name_buf.trim().to_string();
                            if tgt.is_empty() {
                                name_resp.ctx.memory_mut(|m| {
                                    m.request_focus(egui::Id::new("custom_lang_val_edit"))
                                });
                            } else if name.is_empty() {
                                name_resp.ctx.memory_mut(|m| {
                                    m.request_focus(egui::Id::new("custom_lang_name_edit"))
                                });
                            } else {
                                ok_clicked = true;
                            }
                        }
                    });
                }
            });
        if ok_clicked && state.lang_panel_tab == LangPanelTab::Tgt {
            let tgt = state
                .pending_tgt_lang
                .clone()
                .unwrap_or_else(|| data.tgt_lang.clone());
            let tgt_name = if is_preset_lang(&tgt) {
                None
            } else {
                let n = state.custom_tgt_name_buf.trim().to_string();
                if n.is_empty() {
                    None
                } else {
                    Some(n)
                }
            };
            let current_slot =
                dict_slot_for_target_commit(&data.tgt_lang, &tgt, data.dict_slot.as_deref());

            // 言語変更時：現在の dict_slot が次ターゲットと一致しない場合は辞書確認ダイアログへ
            if current_slot.is_none()
                && data.tgt_lang != tgt
                && data
                    .dict_slot
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty())
                && !crate::backend::manager::dict_slot_matches_target(
                    Path::new(data.dict_slot.as_deref().unwrap()),
                    &tgt,
                )
            {
                state.dict_check_pending = Some((
                    data.src_lang.clone(),
                    tgt,
                    tgt_name,
                    data.dict_slot.clone().unwrap(),
                ));
            } else {
                commands.set_lang_pair = Some((data.src_lang.clone(), tgt, tgt_name, current_slot));
            }
            state.pending_tgt_lang = None;
        }
        if ok_clicked {
            state.show_lang_panel = false;
        }
    }

    if state.dict_check_pending.is_some() {
        let (src, tgt, tgt_name, current_slot) = state.dict_check_pending.clone().unwrap();
        let mut show_dialog = true;
        egui::Window::new("dict_check")
            .title_bar(true)
            .resizable(false)
            .collapsible(false)
            .open(&mut show_dialog)
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.label(format!(
                    "{} {}",
                    normal_text::text(l, NormalText::DictCheckCurrent),
                    current_slot
                ));
                ui.add_space(8.0);
                ui.label(normal_text::text(l, NormalText::DictCheckQuestion));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(normal_text::text(l, NormalText::DictCheckUseAsIs))
                        .clicked()
                    {
                        commands.set_lang_pair = Some((
                            src.clone(),
                            tgt.clone(),
                            tgt_name.clone(),
                            Some(current_slot.clone()),
                        ));
                        state.dict_check_pending = None;
                    }
                    if ui
                        .button(normal_text::text(l, NormalText::DictCheckCreateNew))
                        .clicked()
                    {
                        commands.set_lang_pair =
                            Some((src.clone(), tgt.clone(), tgt_name.clone(), None));
                        state.dict_check_pending = None;
                    }
                });
            });
        if !show_dialog {
            // ユーザーが×ボタンで閉じた → 保留を破棄、言語変更を commit しない
            state.dict_check_pending = None;
        }
    }

    egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(normal_text::vram_metric(l, data.vram_mb)).size(12.0));
            ui.label(egui::RichText::new(normal_text::shared_metric(l, data.shared_mb)).size(12.0));
            ui.label(
                egui::RichText::new(normal_text::tokens_metric(l, data.tokens_per_second))
                    .size(12.0),
            );
            ui.label(
                egui::RichText::new(normal_text::dict_hits_metric(l, data.dict_hits)).size(12.0),
            );
        });
        ui.horizontal(|ui| {
            if data.server_host == normal_text::public_host_value() && !data.local_ip.is_empty() {
                ui.label(
                    egui::RichText::new(normal_text::server_url(
                        l,
                        &data.local_ip,
                        data.server_port,
                    ))
                    .size(12.0)
                    .color(egui::Color32::from_rgb(100, 200, 100)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if data.work_running
                    || data.file_translate_total > 0
                    || !data.file_translate_status_text.is_empty()
                {
                    let progress_label = if data.work_running
                        && data.file_translate_total > 0
                        && data.file_translate_status_text.is_empty()
                    {
                        format!(
                            "{}/{} {}",
                            data.file_translate_done,
                            data.file_translate_total,
                            list_text::text(l, ListText::Running)
                        )
                    } else if !data.file_translate_status_text.is_empty() {
                        data.file_translate_status_text.clone()
                    } else {
                        list_text::text(l, ListText::Started).to_string()
                    };
                    let progress_color = if data.work_running {
                        egui::Color32::from_rgb(100, 150, 255)
                    } else if data.file_translate_error {
                        egui::Color32::from_rgb(255, 200, 100)
                    } else {
                        egui::Color32::from_rgb(0, 200, 100)
                    };
                    ui.label(
                        egui::RichText::new(progress_label)
                            .size(12.0)
                            .color(progress_color),
                    );
                } else if data.status_visible {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            normal_text::status_label(&data.ui_lang, data.status_key),
                            normal_text::status_icon_label(data.status_icon)
                        ))
                        .size(12.0)
                        .color(data.status_icon.color()),
                    );
                }
            });
        });
    });

    if state.file_translate.is_list_mode() {
        egui::SidePanel::left("file_translate_tree")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                crate::ui::file_translate_panel::render_tree_panel(ui, data, state, commands);
            });

        egui::SidePanel::right("file_translate_preview")
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                crate::ui::file_translate_panel::render_preview_panel(ui, data, state, commands);
            });
    }

    egui::CentralPanel::default().show(ctx, |ui| match state.log_panel_tab {
        LeftPanelTab::ServerLog => {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    draw_log_entries(ui, &data.translation_logs, no_logs)
                });
        }
        LeftPanelTab::TenukiLog => {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| draw_log_entries(ui, &data.tenuki_logs, no_logs));
        }
        LeftPanelTab::Dictionary => {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| draw_dictionary_history(ui, data, no_history));
        }
        LeftPanelTab::List => {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    draw_file_translate_log_entries(
                        ui,
                        &data.file_translate_logs,
                        list_text::text(l, ListText::NoListActivity),
                    )
                });
        }
    });

    ctx.request_repaint_after(std::time::Duration::from_millis(33));
}

#[cfg(test)]
mod tests {
    use super::{
        dict_slot_for_target_commit, next_log_panel_tab, select_top_mode, selected_top_mode,
        TopModeChoice,
    };
    use crate::ui::container::{LeftPanelTab, UiCommands, UiDisplayData, UiState};

    fn display_with_profile(profile: &str) -> UiDisplayData {
        UiDisplayData {
            profile: profile.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn log_rotation_skips_list_outside_list_mode() {
        assert_eq!(
            next_log_panel_tab(LeftPanelTab::Dictionary, false),
            LeftPanelTab::TenukiLog
        );
    }

    #[test]
    fn log_rotation_includes_list_only_in_list_mode() {
        assert_eq!(
            next_log_panel_tab(LeftPanelTab::Dictionary, true),
            LeftPanelTab::List
        );
        assert_eq!(
            next_log_panel_tab(LeftPanelTab::List, true),
            LeftPanelTab::TenukiLog
        );
    }

    #[test]
    fn top_mode_uses_list_entry_over_profile_axis() {
        let data = display_with_profile("game");
        let mut state = UiState::default();

        assert_eq!(selected_top_mode(&data, &state), TopModeChoice::Game);

        state.file_translate.enter_list_mode();

        assert_eq!(selected_top_mode(&data, &state), TopModeChoice::List);
    }

    #[test]
    fn selecting_list_preserves_current_profile() {
        let data = display_with_profile("game");
        let mut state = UiState::default();
        state.file_translate.initialized = true;
        let mut commands = UiCommands::default();

        select_top_mode(TopModeChoice::List, &data, &mut state, &mut commands);

        assert!(state.file_translate.is_list_mode());
        assert_eq!(state.log_panel_tab, LeftPanelTab::List);
        assert_eq!(commands.set_profile, None);
    }

    #[test]
    fn selecting_live_profile_exits_list_mode_and_updates_profile_axis() {
        let data = display_with_profile("game");
        let mut state = UiState::default();
        state.file_translate.enter_list_mode();
        state.log_panel_tab = LeftPanelTab::List;
        let mut commands = UiCommands::default();

        select_top_mode(TopModeChoice::Normal, &data, &mut state, &mut commands);

        assert!(!state.file_translate.is_list_mode());
        assert_eq!(state.log_panel_tab, LeftPanelTab::TenukiLog);
        assert_eq!(commands.set_profile, Some("normal".to_string()));
    }

    #[test]
    fn dict_slot_is_kept_when_target_language_is_unchanged() {
        assert_eq!(
            dict_slot_for_target_commit("ja", "ja", Some(r"C:\TENUKI\dicts\ja\text\ja_001")),
            Some(r"C:\TENUKI\dicts\ja\text\ja_001".to_string())
        );
    }

    #[test]
    fn dict_slot_requires_confirmation_when_target_language_changes() {
        assert_eq!(
            dict_slot_for_target_commit("ja", "en", Some(r"C:\TENUKI\dicts\ja\text\ja_001")),
            None
        );
    }

    #[test]
    fn dict_slot_is_kept_when_slot_matches_next_target_language() {
        assert_eq!(
            dict_slot_for_target_commit("ja", "en", Some(r"C:\TENUKI\dicts\en\text\en_001")),
            Some(r"C:\TENUKI\dicts\en\text\en_001".to_string())
        );
    }

    #[test]
    fn empty_dict_slot_is_not_forwarded() {
        assert_eq!(dict_slot_for_target_commit("ja", "ja", Some("")), None);
    }
}
