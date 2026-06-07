use crate::messages::LogLevel;
use crate::ui::container::{
    LeftPanelTab, LogEntry, UiCommands, UiDisplayData, UiState,
};
use crate::ui::normal_text::{self, NormalText, TopModeText};
use eframe::egui;
use regex;
use std::path::{Path, PathBuf};

pub(super) const LANGS: &[&str] = crate::config::TARGET_LANGUAGE_PRESETS;
pub(super) const UI_LANGS: &[&str] = &["ja", "en", "zh-CN"];
const STATUS_LAMP_CELL_WIDTH: f32 = 18.0;
const STATUS_LAMP_AFTER_SPACE: f32 = 0.0;
const CREATE_NEW_DICT_SLOT_LABEL: &str =
    "\u{ff0b} \u{65b0}\u{3057}\u{3044}\u{8f9e}\u{66f8}\u{3092}\u{4f5c}\u{6210}";

pub(super) fn is_preset_lang(code: &str) -> bool {
    crate::config::is_target_language_preset(code)
}

pub(super) fn next_log_panel_tab(current: LeftPanelTab, list_available: bool) -> LeftPanelTab {
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

pub(super) fn dict_slot_for_target_commit(
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

fn current_dict_slot_label(data: &UiDisplayData) -> String {
    match &data.dict_slot {
        Some(p) => Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| normal_text::text(&data.ui_lang, NormalText::Dict).into()),
        None => normal_text::text(&data.ui_lang, NormalText::DictNone).to_string(),
    }
}

fn available_dict_slots_for_target(base_dir: &Path, target_lang: &str) -> Vec<PathBuf> {
    let text_dir = base_dir.join("dicts").join(target_lang).join("text");
    let mut slots = std::fs::read_dir(text_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();

    slots.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    slots
}

fn request_create_new_dict_slot(commands: &mut UiCommands) {
    commands.create_new_dict_slot = true;
}

pub(super) fn render_status_lamp(
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

pub(super) fn render_mode_combo(
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

pub(super) fn render_dict_slot_combo(ui: &mut egui::Ui, data: &UiDisplayData, commands: &mut UiCommands) {
    egui::ComboBox::from_id_salt("dict_slot_combo")
        .selected_text(egui::RichText::new(current_dict_slot_label(data)).size(14.0))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(false, CREATE_NEW_DICT_SLOT_LABEL)
                .clicked()
            {
                request_create_new_dict_slot(commands);
                ui.close_menu();
            }

            ui.separator();

            for slot in available_dict_slots_for_target(&data.base_dir, &data.tgt_lang) {
                let slot_value = slot.to_string_lossy().to_string();
                let label = slot
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| slot_value.clone());
                let selected = data.dict_slot.as_deref() == Some(slot_value.as_str());
                if ui.selectable_label(selected, label).clicked() {
                    commands.set_dict_slot = Some(slot_value);
                    ui.close_menu();
                }
            }
        });
}

pub(super) fn draw_log_entries(ui: &mut egui::Ui, logs: &std::collections::VecDeque<LogEntry>, empty: &str) {
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

pub(super) fn draw_dictionary_history(ui: &mut egui::Ui, data: &UiDisplayData, empty: &str) {
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

pub(super) fn draw_file_translate_log_entries(
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

pub use crate::ui::panels::show_normal_ui;

#[cfg(test)]
mod tests {
    use super::{
        available_dict_slots_for_target, dict_slot_for_target_commit, next_log_panel_tab,
        request_create_new_dict_slot, select_top_mode, selected_top_mode, TopModeChoice,
        CREATE_NEW_DICT_SLOT_LABEL,
    };
    use crate::ui::container::{LeftPanelTab, UiCommands, UiDisplayData, UiState};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tenuki-ui-normal-test-{}-{}",
            std::process::id(),
            unique
        ))
    }

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

    #[test]
    fn dict_slot_create_action_sets_flag_without_slot_value() {
        let mut commands = UiCommands::default();

        request_create_new_dict_slot(&mut commands);

        assert!(commands.create_new_dict_slot);
        assert_eq!(commands.set_dict_slot, None);
        assert_ne!(
            commands.set_dict_slot.as_deref(),
            Some(CREATE_NEW_DICT_SLOT_LABEL)
        );
    }

    #[test]
    fn available_dict_slots_lists_all_directories_under_current_target_text_dir() {
        let base_dir = unique_test_dir();
        let ja_text_dir = base_dir.join("dicts").join("ja").join("text");
        let en_text_dir = base_dir.join("dicts").join("en").join("text");
        fs::create_dir_all(ja_text_dir.join("ja_002")).expect("create ja_002");
        fs::create_dir_all(ja_text_dir.join("ja_001")).expect("create ja_001");
        fs::create_dir_all(ja_text_dir.join("LongYinLiZhiZhuan"))
            .expect("create game-name dictionary folder");
        fs::create_dir_all(ja_text_dir.join("UserGameDictionary"))
            .expect("create arbitrary dictionary folder");
        fs::create_dir_all(ja_text_dir.join("S_0001")).expect("create legacy slot folder");
        fs::create_dir_all(en_text_dir.join("en_001")).expect("create en_001");
        fs::write(ja_text_dir.join("note.txt"), "not a slot").expect("write non-slot file");

        let slots = available_dict_slots_for_target(&base_dir, "ja");
        let names = slots
            .iter()
            .filter_map(|slot| slot.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "LongYinLiZhiZhuan".to_string(),
                "S_0001".to_string(),
                "UserGameDictionary".to_string(),
                "ja_001".to_string(),
                "ja_002".to_string()
            ]
        );

        let _ = fs::remove_dir_all(base_dir);
    }
}
