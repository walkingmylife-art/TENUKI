use crate::file_translate::commands::FileTranslateUiCommand;
use crate::file_translate::state::{evaluate_run_readiness, DictSlotAction};
use crate::file_translate::types::{
    AssetSourceCandidate, ColumnMode, HeaderMode, PreviewState, SourceEncoding, SourceKind,
    SourcePreview, TableSourceData,
};
use crate::ui::container::{UiCommands, UiDisplayData, UiState};
use crate::ui::list_text::{self, ListText};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) const TEXT_PREVIEW_INITIAL_LINE_LIMIT: usize = 200;
pub(crate) const TEXT_PREVIEW_EXTEND_LINES: usize = 200;

pub fn request_folder_pick(state: &mut UiState, base_dir: &std::path::Path) {
    if state.file_translate.folder_pick_rx.is_some() {
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    state.file_translate.folder_pick_rx = Some(rx);
    let initial_dir = desktop_dir().unwrap_or_else(|| base_dir.to_path_buf());
    std::thread::spawn(move || {
        let _ = tx.send(
            rfd::FileDialog::new()
                .set_directory(initial_dir)
                .pick_folder(),
        );
    });
}

pub fn poll_folder_picker(state: &mut UiState, commands: &mut UiCommands) {
    if let Some(rx) = &state.file_translate.folder_pick_rx {
        if let Ok(result) = rx.try_recv() {
            if let Some(path) = result {
                state.file_translate.enter_list_mode();
                commands
                    .file_translate_commands
                    .push(FileTranslateUiCommand::StartFileTranslateScan(path));
            } else if !state.file_translate.initialized {
                state.file_translate.leave_list_mode();
            }
            state.file_translate.folder_pick_rx = None;
        }
    }
}

pub fn toggle_list_mode(state: &mut UiState, base_dir: &std::path::Path) {
    if state.file_translate.is_list_mode() {
        state.file_translate.leave_list_mode();
        return;
    }

    state.file_translate.enter_list_mode();
    if state.file_translate.initialized {
        return;
    }

    request_folder_pick(state, base_dir);
}

fn desktop_dir() -> Option<std::path::PathBuf> {
    let user_profile = std::env::var_os("USERPROFILE")?;
    let desktop = std::path::PathBuf::from(user_profile).join("Desktop");
    desktop.is_dir().then_some(desktop)
}

pub fn can_run_from_toolbar(data: &UiDisplayData, state: &UiState) -> bool {
    evaluate_run_readiness(
        &state.file_translate,
        data.dict_slot.as_deref(),
        &data.tgt_lang,
        &data.base_dir,
    )
    .is_ready()
}

pub fn render_tree_panel(
    ui: &mut egui::Ui,
    data: &UiDisplayData,
    state: &mut UiState,
    commands: &mut UiCommands,
) {
    let l = &data.ui_lang;
    if let Some(root) = &state.file_translate.root {
        ui.label(
            egui::RichText::new(root.display().to_string())
                .monospace()
                .size(11.0),
        );
        ui.separator();
    }

    if state.file_translate.scan_in_progress {
        ui.colored_label(
            egui::Color32::from_rgb(140, 170, 255),
            list_text::text(l, ListText::ScanningSources),
        );
        ui.separator();
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        if state.file_translate.sources.is_empty() {
            let message = if state.file_translate.scan_in_progress {
                list_text::text(l, ListText::Scanning)
            } else {
                list_text::text(l, ListText::NoAssetSources)
            };
            ui.colored_label(egui::Color32::GRAY, message);
            return;
        }

        let tree = build_source_tree(
            state.file_translate.root.as_deref(),
            &state.file_translate.sources,
        );
        render_source_tree(
            ui,
            &tree,
            0,
            l,
            &state.file_translate.selected_source,
            commands,
        );
    });
}

#[derive(Default)]
struct SourceTreeNode {
    directories: BTreeMap<String, SourceTreeNode>,
    files: Vec<SourceTreeFile>,
}

struct SourceTreeFile {
    name: String,
    path: std::path::PathBuf,
    kind: SourceKind,
    encoding: SourceEncoding,
    file_size: u64,
    diagnostic: String,
}

fn build_source_tree(root: Option<&Path>, sources: &[AssetSourceCandidate]) -> SourceTreeNode {
    let mut tree = SourceTreeNode::default();

    for source in sources {
        insert_source_into_tree(&mut tree, root, source);
    }

    sort_source_tree(&mut tree);
    tree
}

fn insert_source_into_tree(
    tree: &mut SourceTreeNode,
    root: Option<&Path>,
    source: &AssetSourceCandidate,
) {
    let relative = root
        .and_then(|root| source.path.strip_prefix(root).ok())
        .unwrap_or_else(|| source.path.as_path());

    let mut node = tree;
    let mut parts = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let file_name = parts
        .pop()
        .filter(|part| !part.is_empty())
        .unwrap_or_else(|| {
            source
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
                .unwrap_or_else(|| source.path.display().to_string())
        });

    for directory in parts {
        node = node.directories.entry(directory).or_default();
    }

    node.files.push(SourceTreeFile {
        name: file_name,
        path: source.path.clone(),
        kind: source.kind,
        encoding: source.encoding,
        file_size: source.file_size,
        diagnostic: source.diagnostic.clone(),
    });
}

fn sort_source_tree(tree: &mut SourceTreeNode) {
    tree.files.sort_by(|a, b| a.name.cmp(&b.name));
    for child in tree.directories.values_mut() {
        sort_source_tree(child);
    }
}

fn render_source_tree(
    ui: &mut egui::Ui,
    tree: &SourceTreeNode,
    depth: usize,
    lang: &str,
    selected_source: &Option<std::path::PathBuf>,
    commands: &mut UiCommands,
) {
    for (directory, child) in &tree.directories {
        let header = egui::CollapsingHeader::new(
            egui::RichText::new(directory)
                .strong()
                .color(egui::Color32::from_rgb(190, 190, 190)),
        )
        .default_open(depth < 2);
        header.show(ui, |ui| {
            render_source_tree(ui, child, depth + 1, lang, selected_source, commands);
        });
    }

    for file in &tree.files {
        render_source_leaf(ui, file, lang, selected_source, commands);
    }
}

fn render_source_leaf(
    ui: &mut egui::Ui,
    file: &SourceTreeFile,
    lang: &str,
    selected_source: &Option<std::path::PathBuf>,
    commands: &mut UiCommands,
) {
    let selected = selected_source.as_ref() == Some(&file.path);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(file.kind.badge())
                .size(10.0)
                .color(kind_color(file.kind))
                .strong(),
        );
        let response = ui.selectable_label(selected, &file.name);
        if response.clicked() {
            commands.file_translate_commands.push(
                FileTranslateUiCommand::SelectFileTranslateSource(file.path.clone()),
            );
        }
        response.on_hover_text(list_text::source_hover(
            lang,
            file.encoding,
            file.file_size,
            &file.diagnostic,
        ));
    });
}

pub fn render_preview_panel(
    ui: &mut egui::Ui,
    data: &UiDisplayData,
    state: &mut UiState,
    commands: &mut UiCommands,
) {
    let readiness = evaluate_run_readiness(
        &state.file_translate,
        data.dict_slot.as_deref(),
        &data.tgt_lang,
        &data.base_dir,
    );
    let preview_state = state.file_translate.preview.clone();
    let l = &data.ui_lang;
    if state.file_translate.preview_loading {
        ui.colored_label(
            egui::Color32::from_rgb(140, 170, 255),
            list_text::text(l, ListText::LoadingPreview),
        );
        if let Some(file) = &state.file_translate.preview_target {
            ui.label(
                egui::RichText::new(list_text::field(l, ListText::File, file.display()))
                    .monospace()
                    .size(11.0),
            );
        }
        return;
    }

    match preview_state {
        PreviewState::Empty => {
            ui.label(list_text::text(l, ListText::SelectAssetSource));
        }
        PreviewState::Error(reason) => {
            ui.colored_label(egui::Color32::LIGHT_RED, reason);
        }
        PreviewState::Ready(preview) => {
            render_preview_header(ui, data, state, &preview, &readiness);
            ui.separator();
            match preview {
                SourcePreview::Table(preview) => {
                    render_table_preview(ui, data, state, &preview, commands)
                }
                SourcePreview::Text(preview) => {
                    let showing_lines = preview
                        .lines
                        .len()
                        .min(state.file_translate.text_preview_line_limit);
                    ui.label(list_text::text_preview_stats(
                        l,
                        preview.encoding,
                        preview.line_count,
                        preview.file_size,
                        showing_lines,
                        preview.line_count,
                    ));
                    ui.label(
                        egui::RichText::new(preview.diagnostic.clone()).color(egui::Color32::GRAY),
                    );
                    ui.separator();
                    let mut extend_text_preview = false;
                    egui::ScrollArea::both().show_viewport(ui, |ui, viewport| {
                        for (index, line) in preview.lines.iter().take(showing_lines).enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:>4}", index + 1))
                                        .monospace()
                                        .color(egui::Color32::GRAY),
                                );
                                ui.monospace(line);
                            });
                        }
                        let content_bottom = ui.min_rect().bottom();
                        let viewport_bottom = viewport.bottom();
                        if showing_lines < preview.lines.len()
                            && viewport_bottom + TEXT_PREVIEW_EXTEND_LINES as f32 >= content_bottom
                        {
                            extend_text_preview = true;
                        }
                    });
                    if extend_text_preview {
                        let current = state.file_translate.text_preview_line_limit;
                        let next = (current + TEXT_PREVIEW_EXTEND_LINES).min(preview.lines.len());
                        state.file_translate.text_preview_line_limit = next;
                        ui.ctx().request_repaint();
                    }
                }
                SourcePreview::Binary(preview) => {
                    ui.label(list_text::size_bytes(l, preview.file_size));
                    ui.colored_label(egui::Color32::GRAY, &preview.diagnostic);
                }
            }
        }
    }
}

fn render_table_preview(
    ui: &mut egui::Ui,
    data: &UiDisplayData,
    state: &mut UiState,
    preview: &TableSourceData,
    commands: &mut UiCommands,
) {
    let l = &data.ui_lang;
    let total_rows = preview.rows.len();
    let limit = state.file_translate.table_preview_row_limit;
    let showing_rows = total_rows.min(limit);

    ui.horizontal(|ui| {
        ui.label(list_text::table_preview_stats(
            l,
            preview.total_rows,
            preview.column_labels.len(),
            preview.delimiter,
            showing_rows,
        ));

        if preview.supports_header_toggle() {
            ui.separator();
            if ui
                .selectable_label(
                    preview.header_mode == HeaderMode::Absent,
                    list_text::text(l, ListText::NoHeaderRow),
                )
                .clicked()
            {
                commands.file_translate_commands.push(
                    FileTranslateUiCommand::SetFileTranslateHeaderMode {
                        file: preview.file.clone(),
                        mode: HeaderMode::Absent,
                    },
                );
            }
            if ui
                .selectable_label(
                    preview.header_mode == HeaderMode::Present,
                    list_text::text(l, ListText::HasHeaderRow),
                )
                .clicked()
            {
                commands.file_translate_commands.push(
                    FileTranslateUiCommand::SetFileTranslateHeaderMode {
                        file: preview.file.clone(),
                        mode: HeaderMode::Present,
                    },
                );
            }
        }
    });

    if let Some(diagnostic) = &preview.json_diagnostic {
        ui.label(egui::RichText::new(diagnostic).color(egui::Color32::GRAY));
    }
    ui.separator();

    let total_rows_for_closure = total_rows;
    egui::ScrollArea::both().show(ui, |ui| {
        egui::Grid::new("file_translate_preview_grid")
            .striped(true)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("#").strong().monospace());
                for (index, label) in preview.column_labels.iter().enumerate() {
                    let mode = state
                        .file_translate
                        .column_modes
                        .get(&index)
                        .copied()
                        .unwrap_or(ColumnMode::None);
                    let color = match mode {
                        ColumnMode::Translate => egui::Color32::from_rgb(80, 150, 110),
                        ColumnMode::Original => egui::Color32::from_rgb(80, 120, 180),
                        ColumnMode::None => egui::Color32::from_rgb(70, 70, 70),
                    };
                    let button = egui::Button::new(format!("{} [{}]", label, mode.short_label()))
                        .fill(color);
                    if ui.add(button).clicked() {
                        commands.file_translate_commands.push(
                            FileTranslateUiCommand::SetFileTranslateColumnMode {
                                file: preview.file.clone(),
                                column: index,
                                mode: mode.next(),
                            },
                        );
                    }
                }
                ui.end_row();

                for (row_index, row) in preview
                    .rows
                    .iter()
                    .take(state.file_translate.table_preview_row_limit)
                    .enumerate()
                {
                    ui.label(
                        egui::RichText::new(format!("{}", row_index + 1))
                            .monospace()
                            .color(egui::Color32::GRAY),
                    );
                    for column_index in 0..preview.column_labels.len() {
                        let cell = row.get(column_index).map(String::as_str).unwrap_or("");
                        ui.label(egui::RichText::new(cell).monospace().size(12.0));
                    }
                    ui.end_row();
                }
            });

        let current_limit = state.file_translate.table_preview_row_limit;
        if current_limit < total_rows_for_closure {
            let content_bottom = ui.min_rect().bottom();
            let viewport_bottom = ui.clip_rect().bottom();
            if viewport_bottom + 200.0 >= content_bottom {
                let next = (current_limit + 100).min(total_rows_for_closure);
                state.file_translate.table_preview_row_limit = next;
                ui.ctx().request_repaint();
            }
        }
    });
}

fn render_preview_header(
    ui: &mut egui::Ui,
    data: &UiDisplayData,
    state: &UiState,
    preview: &SourcePreview,
    readiness: &crate::file_translate::state::FileTranslateRunReadiness,
) {
    let l = &data.ui_lang;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(preview.kind().badge())
                .color(kind_color(preview.kind()))
                .strong(),
        );
        ui.label(list_text::source_kind_label(
            l,
            preview.kind(),
            preview.as_table().and_then(|table| table.json_shape),
        ));
        ui.label(
            egui::RichText::new(preview.path().display().to_string())
                .monospace()
                .size(11.0),
        );
    });

    ui.label(list_text::encoding_size_line(
        l,
        preview.encoding(),
        preview.file_size(),
    ));

    match &readiness.dict_slot_action {
        Some(DictSlotAction::UseCommitted(path)) => {
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    list_text::text(l, ListText::OutputDirectory),
                    path.display()
                ))
                .monospace()
                .size(11.0),
            );
        }
        Some(DictSlotAction::CreateForRun {
            parent,
            target_lang,
        }) => {
            ui.colored_label(
                egui::Color32::from_rgb(210, 190, 120),
                format!(
                    "{}: {} {}",
                    list_text::text(l, ListText::RunUsesListOutputFolder),
                    target_lang,
                    parent.display()
                ),
            );
        }
        None => {}
    }

    if data.work_running {
        ui.colored_label(egui::Color32::GRAY, list_text::text(l, ListText::Running));
    } else if readiness.is_ready() {
        ui.colored_label(
            egui::Color32::from_rgb(100, 200, 120),
            list_text::text(l, ListText::RunReady),
        );
    } else {
        for blocker in &readiness.blockers {
            ui.colored_label(egui::Color32::GRAY, list_text::blocker(l, blocker));
        }
    }

    if state.file_translate.selected_source.is_none() {
        ui.colored_label(
            egui::Color32::GRAY,
            list_text::text(l, ListText::NoSourceSelected),
        );
    }
}

fn kind_color(kind: SourceKind) -> egui::Color32 {
    match kind {
        SourceKind::DelimitedText => egui::Color32::from_rgb(94, 201, 122),
        SourceKind::JsonText => egui::Color32::from_rgb(91, 156, 255),
        SourceKind::PlainLines => egui::Color32::from_rgb(212, 188, 92),
        SourceKind::MarkupText => egui::Color32::from_rgb(193, 126, 233),
        SourceKind::UnsupportedBinary => egui::Color32::from_rgb(205, 97, 85),
        SourceKind::UnknownText => egui::Color32::from_rgb(160, 160, 160),
    }
}
