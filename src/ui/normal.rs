use crate::messages::LogLevel;
use crate::ui::container::{
    DictConfirmPending, LangPanelTab, LeftPanelTab, LogEntry, UiCommands, UiDisplayData, UiState,
};
use eframe::egui;

const LANGS: &[&str] = &["ja", "en", "zh-CN", "zh-TW", "ko", "ar"];

fn lang_label(ui_lang: &str, code: &str) -> &'static str {
    match (ui_lang, code) {
        ("en", "ja") => "Japanese",
        ("en", "en") => "English",
        ("en", "zh-CN") => "Chinese (Simplified)",
        ("en", "zh-TW") => "Chinese (Traditional)",
        ("en", "ko") => "Korean",
        ("en", "ar") => "Arabic",
        (_, "ja") => "日本語",
        (_, "en") => "English",
        (_, "zh-CN") => "簡体字中国語",
        (_, "zh-TW") => "繁体字中国語",
        (_, "ko") => "韓国語",
        (_, "ar") => "アラビア語",
        _ => "Unknown",
    }
}

fn is_preset_lang(code: &str) -> bool {
    ["en", "ja", "zh-CN", "zh-TW", "ko", "fr", "ar", "es", "it", "pt", "ru"].contains(&code)
}

fn status_icon_label(icon: crate::ui::container::StatusIcon) -> &'static str {
    match icon {
        crate::ui::container::StatusIcon::None => "",
        crate::ui::container::StatusIcon::Spinner => "...",
        crate::ui::container::StatusIcon::Check => "OK",
        crate::ui::container::StatusIcon::Warning => "WARN",
    }
}

fn profile_label(profile: &str) -> &str {
    match profile {
        "game" => "GAME",
        "default" => "default",
        other => other,
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
                    LogLevel::Warning => egui::Color32::from_rgb(255, 200, 100),
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
                    ui.label(egui::RichText::new(original).size(13.0).color(egui::Color32::from_rgb(220, 220, 215)));
                    ui.label(
                        egui::RichText::new(translated)
                            .size(13.0)
                            .color(egui::Color32::from_rgb(110, 160, 210)),
                    );
                });
                ui.add_space(4.0);
            }
        });
}

pub fn show_normal_ui(
    ctx: &egui::Context,
    data: &UiDisplayData,
    state: &mut UiState,
    commands: &mut UiCommands,
) {
    let is_en = data.ui_lang == "en";
    let no_logs = if is_en { "No logs yet" } else { "ログはまだありません" };
    let no_history = if is_en {
        "No entries yet"
    } else {
        "登録履歴はまだありません"
    };

    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("●")
                    .color(if data.tenuki_running {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::RED
                    })
                    .size(14.0),
            );
            ui.add_space(8.0);
            let tgt_label = if LANGS.contains(&data.tgt_lang.as_str()) {
                lang_label(&data.ui_lang, &data.tgt_lang)
            } else {
                data.tgt_lang.as_str()
            };
            let btn = ui.button(egui::RichText::new(tgt_label).size(14.0));
            if btn.clicked() {
                state.show_lang_panel = !state.show_lang_panel;
                if state.show_lang_panel {
                    state.lang_panel_anchor = btn.rect.left_bottom();
                    state.custom_tgt_code_buf = if is_preset_lang(&data.tgt_lang) {
                        String::new()
                    } else {
                        data.tgt_lang.clone()
                    };
                    state.custom_tgt_name_buf =
                        if !is_preset_lang(&data.tgt_lang) && data.custom_lang_code == data.tgt_lang
                        {
                            data.custom_lang_name.clone()
                        } else {
                            String::new()
                        };
                }
            }
            ui.add_space(8.0);
            // モード切替
            {
                let mode_text = if data.translation_mode == "passthrough" {
                    if is_en { "Normal" } else { "ノーマル" }
                } else {
                    if is_en { "Game" } else { "ゲーム" }
                };
                egui::ComboBox::from_id_salt("translation_mode_combo")
                    .width(90.0)
                    .selected_text(mode_text)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(
                            data.translation_mode == "structural",
                            if is_en { "Game" } else { "ゲーム" },
                        ).clicked() {
                            commands.set_translation_mode = Some("structural".to_string());
                        }
                        if ui.selectable_label(
                            data.translation_mode == "passthrough",
                            if is_en { "Normal" } else { "ノーマル" },
                        ).clicked() {
                            commands.set_translation_mode = Some("passthrough".to_string());
                        }
                    });
            }
            ui.add_space(8.0);
            if !data.available_profiles.is_empty() {
                let current = if data.profile.is_empty() {
                    "default"
                } else {
                    &data.profile
                };
                egui::ComboBox::from_id_salt("profile_combo")
                    .width(110.0)
                    .selected_text(profile_label(current))
                    .show_ui(ui, |ui| {
                        for p in &data.available_profiles {
                            if ui
                                .selectable_label(current == p.as_str(), profile_label(p))
                                .clicked()
                            {
                                commands.set_profile = Some(p.clone());
                            }
                        }
                    });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(if is_en { "Exit" } else { "終了" }).clicked() {
                    commands.exit_app = true;
                }
                ui.add_space(4.0);
                if ui
                    .button(egui::RichText::new(if is_en { "Log" } else { "ログ" }).size(14.0))
                    .clicked()
                {
                    state.log_panel_tab = match state.log_panel_tab {
                        LeftPanelTab::TenukiLog => LeftPanelTab::ServerLog,
                        LeftPanelTab::ServerLog => LeftPanelTab::Dictionary,
                        LeftPanelTab::Dictionary => LeftPanelTab::TenukiLog,
                    };
                }
                ui.add_space(4.0);
                if let Some(rx) = &state.dict_slot_rx {
                    if let Ok(result) = rx.try_recv() {
                        if let Some(path) = result {
                            commands.set_dict_slot =
                                Some(Some(path.to_string_lossy().to_string()));
                        }
                        state.dict_slot_rx = None;
                    }
                }
                let btn_text = match &data.dict_slot {
                    Some(p) => std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| if is_en { "Dict".into() } else { "辞書".into() }),
                    None => {
                        if is_en {
                            "Dict[none]".into()
                        } else {
                            "辞書[未選択]".into()
                        }
                    }
                };
                if ui
                    .button(egui::RichText::new(btn_text).size(14.0))
                    .clicked()
                    && state.dict_slot_rx.is_none()
                {
                    let (tx, rx) = std::sync::mpsc::channel();
                    state.dict_slot_rx = Some(rx);
                    let default_dir = data.base_dir.join("dicts").join(&data.tgt_lang).join("text");
                    std::thread::spawn(move || {
                        let _ = tx.send(rfd::FileDialog::new().set_directory(&default_dir).pick_folder());
                    });
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("●")
                    .color(if data.llama_running {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::RED
                    })
                    .size(14.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    if is_en { "Dict" } else { "辞書" },
                    data.dictionary_loaded + data.dictionary_new
                ))
                .size(13.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let model_names: Vec<String> = data
                    .available_models
                    .iter()
                    .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                    .collect();
                let selected_name = data
                    .selected_model
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| {
                        if model_names.is_empty() {
                            if is_en {
                                "No model".into()
                            } else {
                                "モデルがありません".into()
                            }
                        } else if is_en {
                            "Select model".into()
                        } else {
                            "モデルを選択".into()
                        }
                    });
                egui::ComboBox::from_label("")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        if model_names.is_empty() {
                            ui.label(if is_en { "No model" } else { "モデルがありません" });
                        } else {
                            for (i, model_path) in data.available_models.iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        data.selected_model.as_ref() == Some(model_path),
                                        &model_names[i],
                                    )
                                    .clicked()
                                {
                                    commands.select_model = Some(model_path.clone());
                                }
                            }
                        }
                    });
            });
        });
    });

    if state.show_lang_panel {
        egui::Window::new("lang_panel")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_pos(state.lang_panel_anchor)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Tgt,
                            if is_en { "Target" } else { "翻訳先" },
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Tgt;
                    }
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Ui,
                            if is_en { "Display" } else { "表示" },
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Ui;
                    }
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Network,
                            if is_en { "Network" } else { "ネットワーク" },
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Network;
                    }
                    ui.add_space(8.0);
                    if ui.button(egui::RichText::new("OK").size(14.0)).clicked() {
                        let code = state.custom_tgt_code_buf.trim().to_string();
                        let name = state.custom_tgt_name_buf.trim().to_string();
                        if !code.is_empty() {
                            commands.set_custom_lang = Some((code, name));
                        }
                        let current_slot = data.dict_slot.clone().filter(|s| !s.is_empty());
                        if current_slot.is_some() {
                            state.dict_confirm = Some(DictConfirmPending {
                                tgt: data.tgt_lang.clone(),
                                current_slot,
                            });
                        } else {
                            commands.set_lang_pair =
                                Some((data.src_lang.clone(), data.tgt_lang.clone(), false));
                        }
                        state.show_lang_panel = false;
                    }
                });
                ui.separator();
                ui.set_min_width(220.0);

                let is_custom =
                    state.lang_panel_tab == LangPanelTab::Tgt && !LANGS.contains(&data.tgt_lang.as_str());
                if state.lang_panel_tab != LangPanelTab::Network {
                    for code in LANGS {
                        let is_selected = match state.lang_panel_tab {
                            LangPanelTab::Tgt => data.tgt_lang.as_str() == *code,
                            LangPanelTab::Ui => data.ui_lang.as_str() == *code,
                            _ => false,
                        };
                        let disabled =
                            state.lang_panel_tab == LangPanelTab::Ui && *code != "en" && *code != "ja";
                        let mut btn = egui::Button::new(lang_label(&data.ui_lang, code))
                            .min_size(egui::vec2(ui.available_width(), 0.0));
                        if is_selected {
                            btn = btn.fill(egui::Color32::from_rgb(70, 90, 120));
                        }
                        if ui.add_enabled(!disabled, btn).clicked() {
                            match state.lang_panel_tab {
                                LangPanelTab::Tgt => {
                                    commands.set_tgt_lang = Some((*code).to_string());
                                    state.custom_tgt_code_buf.clear();
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
                    ui.label(format!("{}:", if is_en { "Host" } else { "ホスト" }));
                    let host_resp = ui.add(
                        egui::TextEdit::singleline(&mut state.network_host_buf)
                            .hint_text("127.0.0.1")
                            .desired_width(ui.available_width() - 4.0)
                            .font(egui::TextStyle::Small),
                    );
                    if host_resp.changed() {
                        commands.set_server_host = Some(state.network_host_buf.trim().to_string());
                    }
                    if host_resp.lost_focus()
                        && host_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        commands.restart_backend = true;
                    }

                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", if is_en { "Port" } else { "ポート" }));
                        let mut port_str = state.network_port_buf.clone();
                        let port_resp = ui.add(
                            egui::TextEdit::singleline(&mut port_str)
                                .hint_text("14371")
                                .desired_width(80.0)
                                .font(egui::TextStyle::Small),
                        );
                        if port_resp.changed() {
                            state.network_port_buf = port_str.clone();
                            if let Ok(p) = port_str.parse::<u16>() {
                                commands.set_server_port = Some(p);
                            }
                        }
                        if port_resp.lost_focus()
                            && port_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            commands.restart_backend = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(egui::RichText::new(if is_en {
                                    "Reset to local"
                                } else {
                                    "ローカルへ戻す"
                                })
                                .size(12.0))
                                .clicked()
                            {
                                state.network_host_buf = "127.0.0.1".to_string();
                                commands.set_server_host = Some("127.0.0.1".to_string());
                                commands.restart_backend = true;
                            }
                        });
                    });
                    ui.separator();
                    let note = if data.server_host == "0.0.0.0" {
                        if is_en {
                            "Network accessible (0.0.0.0)"
                        } else {
                            "ネットワーク公開 (0.0.0.0)"
                        }
                    } else if is_en {
                        "Local only (127.0.0.1)"
                    } else {
                        "ローカルのみ (127.0.0.1)"
                    };
                    ui.label(
                        egui::RichText::new(note)
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                    );
                }

                if state.lang_panel_tab == LangPanelTab::Tgt {
                    let mut custom_btn = egui::Button::new(if is_en {
                        "Custom language"
                    } else {
                        "カスタム言語"
                    })
                    .min_size(egui::vec2(ui.available_width(), 0.0));
                    if is_custom {
                        custom_btn = custom_btn.fill(egui::Color32::from_rgb(70, 90, 120));
                    }
                    ui.add(custom_btn);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", if is_en { "Code" } else { "コード" }));
                        let code_edit = egui::TextEdit::singleline(&mut state.custom_tgt_code_buf)
                            .hint_text("vi")
                            .desired_width(50.0)
                            .font(egui::TextStyle::Small);
                        if ui.add(code_edit).changed() {
                            let code = state.custom_tgt_code_buf.trim().to_string();
                            commands.set_tgt_lang =
                                Some(if code.is_empty() { "ja".to_string() } else { code });
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", if is_en { "Name" } else { "言語名" }));
                        let name_edit = egui::TextEdit::singleline(&mut state.custom_tgt_name_buf)
                            .hint_text("Vietnamese")
                            .desired_width(ui.available_width() - 4.0)
                            .font(egui::TextStyle::Small);
                        if ui.add(name_edit).changed() {
                            let code = state.custom_tgt_code_buf.trim().to_string();
                            if !code.is_empty() {
                                commands.set_custom_lang =
                                    Some((code, state.custom_tgt_name_buf.trim().to_string()));
                            }
                        }
                    });
                }
            });
    }

    if state.dict_confirm.is_some() {
        let mut action: Option<bool> = None;
        egui::Window::new(if is_en { "Dictionary" } else { "辞書の確認" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(260.0);
                ui.separator();
                if let Some(ref pending) = state.dict_confirm {
                    let slot_name = pending
                        .current_slot
                        .as_deref()
                        .and_then(|s| std::path::Path::new(s).file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("?");
                    ui.label(format!(
                        "{}: {}",
                        if is_en { "Current" } else { "現在" },
                        slot_name
                    ));
                    ui.label(if is_en {
                        "Keep current dictionary?"
                    } else {
                        "現在の辞書をそのまま使いますか？"
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new(if is_en { "Keep" } else { "そのまま使う" }).size(13.0))
                        .clicked()
                    {
                        action = Some(true);
                    }
                    if ui
                        .button(
                            egui::RichText::new(if is_en { "Create New" } else { "新しく作る" })
                                .size(13.0),
                        )
                        .clicked()
                    {
                        action = Some(false);
                    }
                });
            });
        if let Some(keep) = action {
            if let Some(p) = state.dict_confirm.take() {
                commands.set_lang_pair = Some((data.src_lang.clone(), p.tgt, keep));
            }
        }
    }

    egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("VRAM: {:.0}MB", data.vram_mb)).size(12.0));
            ui.label(
                egui::RichText::new(format!(
                    "{}: {:.0}MB",
                    if is_en { "Shared" } else { "共有" },
                    data.shared_mb
                ))
                .size(12.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{}: {:.1} t/s",
                    if is_en { "Tokens" } else { "トークン" },
                    data.tokens_per_second
                ))
                .size(12.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    if is_en { "Dict hits" } else { "辞書 hit" },
                    data.dict_hits
                ))
                .size(12.0),
            );
        });
        ui.horizontal(|ui| {
            if data.server_host == "0.0.0.0" && !data.local_ip.is_empty() {
                ui.label(
                    egui::RichText::new(format!("URL: http://{}:{}", data.local_ip, data.server_port))
                        .size(12.0)
                        .color(egui::Color32::from_rgb(100, 200, 100)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if data.status_visible {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            status_icon_label(data.status_icon),
                            data.status_message
                        ))
                        .size(12.0)
                        .color(data.status_icon.color()),
                    );
                }
            });
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| match state.log_panel_tab {
                LeftPanelTab::ServerLog => draw_log_entries(ui, &data.translation_logs, no_logs),
                LeftPanelTab::TenukiLog => draw_log_entries(ui, &data.tenuki_logs, no_logs),
                LeftPanelTab::Dictionary => draw_dictionary_history(ui, data, no_history),
            });
    });

    ctx.request_repaint_after(std::time::Duration::from_millis(33));
}
