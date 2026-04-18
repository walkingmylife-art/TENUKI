use crate::launcher::app_config::{known_model_tuple, ModelConfig, UrlPair};
use crate::messages::{LogLevel, ModelCandidateKind};
use crate::ui::container::{
    DictConfirmPending, LangPanelTab, LeftPanelTab, LogEntry, UiCommands, UiDisplayData, UiState,
};
use eframe::egui;

const LANGS: &[&str] = crate::config::TARGET_LANGUAGE_PRESETS;
const UI_LANGS: &[&str] = &["ja", "en", "zh-CN"];

fn lang_label(ui_lang: &str, code: &str) -> &'static str {
    match (ui_lang, code) {
        ("en", "ja") => "Japanese",
        ("en", "en") => "English",
        ("en", "zh-CN") => "Chinese (Simplified)",
        ("en", "zh-TW") => "Chinese (Traditional)",
        ("en", "ko") => "Korean",
        ("en", "ar") => "Arabic",
        ("zh-CN", "ja") => "日语",
        ("zh-CN", "en") => "英语",
        ("zh-CN", "zh-CN") => "中文（简体）",
        ("zh-CN", "zh-TW") => "中文（繁体）",
        ("zh-CN", "ko") => "韩语",
        ("zh-CN", "ar") => "阿拉伯语",
        (_, "ja") => "日本語",
        (_, "en") => "English",
        (_, "zh-CN") => "簡体字中国語",
        (_, "zh-TW") => "繁体字中国語",
        (_, "ko") => "韓国語",
        (_, "ar") => "アラビア語",
        _ => "Unknown",
    }
}

fn t(ui_lang: &str, en: &'static str, ja: &'static str, zh_cn: &'static str) -> &'static str {
    match ui_lang {
        "ja" => ja,
        "zh-CN" => zh_cn,
        _ => en,
    }
}

fn is_preset_lang(code: &str) -> bool {
    crate::config::is_target_language_preset(code)
}

fn status_icon_label(icon: crate::ui::container::StatusIcon) -> &'static str {
    match icon {
        crate::ui::container::StatusIcon::None => "",
        crate::ui::container::StatusIcon::Spinner => "...",
        crate::ui::container::StatusIcon::Check => "OK",
        crate::ui::container::StatusIcon::Warning => "WARN",
    }
}

fn profile_label(profile: &str) -> String {
    match profile {
        "default" => "Default".to_string(),
        "game" => "Game".to_string(),
        other => other.to_string(),
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
    let L = &data.ui_lang;
    let no_logs = t(L, "No logs yet", "ログなし", "暂无日志");
    let no_history = t(L, "No entries yet", "履歴なし", "暂无记录");

    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("●")
                    .color(if data.tenuki_running {
                        egui::Color32::GREEN
                    } else if data.status_icon == crate::ui::container::StatusIcon::Spinner {
                        egui::Color32::from_rgb(255, 200, 50)
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

            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("profile_combo")
                .width(120.0)
                .selected_text(profile_label(&data.profile))
                .show_ui(ui, |ui| {
                    if data.available_profiles.is_empty() {
                        ui.label(t(L, "No profiles", "プロファイルなし", "无配置"));
                    } else {
                        for profile in &data.available_profiles {
                            if ui
                                .selectable_label(data.profile == *profile, profile_label(profile))
                                .clicked()
                            {
                                commands.set_profile = Some(profile.clone());
                            }
                        }
                    }
                });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t(L, "Exit", "終了", "退出")).clicked() {
                    commands.exit_app = true;
                }
                ui.add_space(4.0);
                if ui.button(t(L, "Log", "ログ", "日志")).clicked() {
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
                            commands.set_dict_slot = Some(path.to_string_lossy().to_string());
                        }
                        state.dict_slot_rx = None;
                    }
                }
                let btn_text = match &data.dict_slot {
                    Some(p) => std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Dict".into()),
                    None => t(L, "Dict[none]", "辞書[なし]", "词典[无]").to_string(),
                };
                if ui
                    .button(egui::RichText::new(btn_text).size(14.0))
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
            });
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("●")
                    .color(if data.llama_running {
                        egui::Color32::GREEN
                    } else if data.status_icon == crate::ui::container::StatusIcon::Spinner {
                        egui::Color32::from_rgb(255, 200, 50)
                    } else {
                        egui::Color32::RED
                    })
                    .size(14.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    t(L, "Dict", "辞書", "词典"),
                    data.dictionary_loaded + data.dictionary_new
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
                            t(L, "No model", "モデルがありません", "无模型").to_string()
                        } else {
                            t(L, "Select model", "モデルを選択", "选择模型").to_string()
                        }
                    });
                egui::ComboBox::from_id_salt("model_combo")
                    .width(220.0)
                    .selected_text(&selected_name)
                    .show_ui(ui, |ui| {
                        if data.available_models.is_empty() {
                            ui.label(t(L, "No models", "モデルがありません", "无模型"));
                        } else {
                            for candidate in &data.available_models {
                                let tag = match candidate.kind {
                                    ModelCandidateKind::Known => "[known]",
                                    ModelCandidateKind::Local => "[local]",
                                };
                                let label = format!("{} {}", tag, candidate.filename);
                                let is_selected = candidate.filename == selected_name;
                                if ui.selectable_label(is_selected, &label).clicked() {
                                    let model_config = match candidate.kind {
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
            .collapsible(false)
            .fixed_pos(state.lang_panel_anchor)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Tgt,
                            t(L, "Target", "翻訳先", "翻译目标"),
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Tgt;
                    }
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Ui,
                            t(L, "Display", "表示", "显示"),
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Ui;
                    }
                    if ui
                        .selectable_label(
                            state.lang_panel_tab == LangPanelTab::Network,
                            t(L, "Network", "ネットワーク", "网络"),
                        )
                        .clicked()
                    {
                        state.lang_panel_tab = LangPanelTab::Network;
                    }
                    ui.add_space(8.0);
                    if ui.button(egui::RichText::new("OK").size(14.0)).clicked() {
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
                        let mut btn = egui::Button::new(lang_label(&data.ui_lang, code))
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
                    ui.label(format!("{}:", t(L, "Host", "ホスト", "主机")));
                    let host_resp = ui.add(
                        egui::TextEdit::singleline(&mut state.network_host_buf)
                            .hint_text("127.0.0.1")
                            .desired_width(ui.available_width() - 4.0)
                            .font(egui::TextStyle::Small),
                    );
                    let host_commit = host_resp.lost_focus()
                        && host_resp.ctx.input(|i| i.key_pressed(egui::Key::Enter));
                    if host_commit {
                        commands.set_server_host = Some(state.network_host_buf.trim().to_string());
                    }

                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", t(L, "Port", "ポート", "端口")));
                        let mut port_str = state.network_port_buf.clone();
                        let port_resp = ui.add(
                            egui::TextEdit::singleline(&mut port_str)
                                .hint_text("14371")
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
                                    egui::RichText::new(t(
                                        L,
                                        "Reset to local",
                                        "ローカルへ戻す",
                                        "重置为本地",
                                    ))
                                    .size(12.0),
                                )
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
                        t(
                            L,
                            "Network accessible (0.0.0.0)",
                            "ネットワーク公開 (0.0.0.0)",
                            "网络可访问 (0.0.0.0)",
                        )
                    } else {
                        t(
                            L,
                            "Local only (127.0.0.1)",
                            "ローカルのみ (127.0.0.1)",
                            "仅本地 (127.0.0.1)",
                        )
                    };
                    ui.label(
                        egui::RichText::new(note)
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                    );
                }

                if state.lang_panel_tab == LangPanelTab::Tgt {
                    let mut custom_btn =
                        egui::Button::new(t(L, "Custom language", "カスタム言語", "自定义语言"))
                            .min_size(egui::vec2(ui.available_width(), 0.0));
                    if is_custom {
                        custom_btn = custom_btn.fill(egui::Color32::from_rgb(70, 90, 120));
                    }
                    ui.add(custom_btn);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", t(L, "Code", "コード", "代码")));
                        let code_resp = ui.add(
                            egui::TextEdit::singleline(&mut state.custom_tgt_val_buf)
                                .id(egui::Id::new("custom_lang_val_edit"))
                                .hint_text("pt-BR")
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
                        ui.label(format!("{}:", t(L, "Name", "言語名", "名称")));
                        let name_resp = ui.add(
                            egui::TextEdit::singleline(&mut state.custom_tgt_name_buf)
                                .id(egui::Id::new("custom_lang_name_edit"))
                                .hint_text("Brazilian Portuguese")
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
            let current_slot = data.dict_slot.clone().filter(|s| !s.is_empty());
            if current_slot.is_some() {
                state.dict_confirm = Some(DictConfirmPending {
                    tgt,
                    tgt_name,
                    current_slot,
                });
            } else {
                commands.set_lang_pair = Some((data.src_lang.clone(), tgt, tgt_name, None));
            }
            state.pending_tgt_lang = None;
        }
        if ok_clicked {
            state.show_lang_panel = false;
        }
    }

    if state.dict_confirm.is_some() {
        let mut action: Option<bool> = None;
        egui::Window::new(t(L, "Dictionary", "辞書の確認", "词典确认"))
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
                        t(L, "Current", "現在", "当前"),
                        slot_name
                    ));
                    ui.label(t(
                        L,
                        "Keep current dictionary?",
                        "現在の辞書をそのまま使いますか？",
                        "保留当前词典？",
                    ));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(
                            egui::RichText::new(t(L, "Keep", "そのまま使う", "保留")).size(13.0),
                        )
                        .clicked()
                    {
                        action = Some(true);
                    }
                    if ui
                        .button(
                            egui::RichText::new(t(L, "Create New", "新しく作る", "新建"))
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
                let dict_slot = if keep { p.current_slot } else { None };
                commands.set_lang_pair =
                    Some((data.src_lang.clone(), p.tgt, p.tgt_name, dict_slot));
            }
        }
    }

    egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("VRAM: {:.0}MB", data.vram_mb)).size(12.0));
            ui.label(egui::RichText::new(format!("Shared: {:.0}MB", data.shared_mb)).size(12.0));
            ui.label(
                egui::RichText::new(format!("Tokens: {:.1} t/s", data.tokens_per_second))
                    .size(12.0),
            );
            ui.label(egui::RichText::new(format!("Dict hits: {}", data.dict_hits)).size(12.0));
        });
        ui.horizontal(|ui| {
            if data.server_host == "0.0.0.0" && !data.local_ip.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "URL: http://{}:{}",
                        data.local_ip, data.server_port
                    ))
                    .size(12.0)
                    .color(egui::Color32::from_rgb(100, 200, 100)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if data.status_visible {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            data.status_key.label(&data.ui_lang),
                            status_icon_label(data.status_icon)
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
