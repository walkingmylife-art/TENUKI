use super::state::{DictSlotAction, FileTranslateRunReadiness};
use super::types::{
    ColumnMode, HeaderMode, PreviewState, SourceKind, SourcePreview, TableSourceData,
};
use crate::ui::list_text::{self, ListText};
use std::collections::BTreeMap;

pub fn build_preview_summary(
    lang: &str,
    preview: &PreviewState,
    column_modes: &BTreeMap<usize, ColumnMode>,
    readiness: &FileTranslateRunReadiness,
) -> (String, bool) {
    match preview {
        PreviewState::Empty => (
            [
                list_text::text(lang, ListText::FileTranslateTitle).to_string(),
                list_text::text(lang, ListText::SelectSourceToPreview).to_string(),
            ]
            .join("\n"),
            false,
        ),
        PreviewState::Error(reason) => (
            [
                list_text::text(lang, ListText::PreviewUnavailable).to_string(),
                reason.clone(),
            ]
            .join("\n"),
            true,
        ),
        PreviewState::Ready(preview) => match preview {
            SourcePreview::Table(preview) => {
                build_table_summary(lang, preview, column_modes, readiness)
            }
            SourcePreview::Text(preview) => (
                build_text_summary(
                    lang,
                    list_text::source_kind_label(lang, preview.source_kind, None).as_str(),
                    preview.lines.as_slice(),
                    &[
                        list_text::field(lang, ListText::File, preview.file.display()),
                        list_text::field(
                            lang,
                            ListText::Kind,
                            list_text::source_kind_label(lang, preview.source_kind, None),
                        ),
                        list_text::field(
                            lang,
                            ListText::Encoding,
                            list_text::encoding_label(lang, preview.encoding),
                        ),
                        list_text::field(lang, ListText::Lines, preview.line_count),
                        list_text::field(
                            lang,
                            ListText::Size,
                            list_text::size_bytes(lang, preview.file_size),
                        ),
                        list_text::field(lang, ListText::Diagnostic, &preview.diagnostic),
                        list_text::field(
                            lang,
                            ListText::Run,
                            list_text::readiness(lang, readiness),
                        ),
                    ],
                ),
                false,
            ),
            SourcePreview::Binary(preview) => (
                [
                    list_text::field(lang, ListText::File, preview.file.display()),
                    list_text::field(
                        lang,
                        ListText::Kind,
                        list_text::text(lang, ListText::UnsupportedBinary),
                    ),
                    list_text::field(
                        lang,
                        ListText::Encoding,
                        list_text::encoding_label(
                            lang,
                            crate::file_translate::types::SourceEncoding::Binary,
                        ),
                    ),
                    list_text::field(
                        lang,
                        ListText::Size,
                        list_text::size_bytes(lang, preview.file_size),
                    ),
                    list_text::field(lang, ListText::Diagnostic, &preview.diagnostic),
                    list_text::field(lang, ListText::Run, list_text::readiness(lang, readiness)),
                ]
                .join("\n"),
                false,
            ),
        },
    }
}

pub fn build_run_log_seed(
    lang: &str,
    preview: &PreviewState,
    column_modes: &BTreeMap<usize, ColumnMode>,
    readiness: &FileTranslateRunReadiness,
) -> Vec<String> {
    match preview {
        PreviewState::Empty => vec![format!(
            "[list] {}",
            list_text::text(lang, ListText::SelectSourceToPreview)
        )],
        PreviewState::Error(reason) => vec![format!(
            "[error] {}: {}",
            list_text::text(lang, ListText::PreviewUnavailable),
            reason
        )],
        PreviewState::Ready(SourcePreview::Table(table)) => {
            let mut lines = vec![
                format!(
                    "[{}] {}",
                    list_text::text(lang, ListText::Source),
                    table.file.display()
                ),
                format!(
                    "[{}] {} / {} {} / {} {}",
                    list_text::text(lang, ListText::Kind),
                    list_text::source_kind_label(lang, table.source_kind, table.json_shape),
                    table.total_rows,
                    list_text::text(lang, ListText::Rows),
                    table.column_labels.len(),
                    list_text::text(lang, ListText::Cols)
                ),
                format!(
                    "[{}] {}",
                    list_text::text(lang, ListText::Header),
                    header_line(lang, table)
                ),
                format!(
                    "[{}] {}",
                    list_text::text(lang, ListText::SelectedColumns),
                    selected_columns_line(lang, table, column_modes)
                ),
                format!(
                    "[{}] {}",
                    list_text::text(lang, ListText::OutputDirectory),
                    slot_line(lang, readiness)
                ),
            ];
            if let Some(diagnostic) = &table.json_diagnostic {
                lines.push(format!(
                    "[{}] {}",
                    list_text::text(lang, ListText::JsonDiagnostic),
                    diagnostic
                ));
            }
            if readiness.is_ready() {
                lines.push(format!(
                    "[{}] {}",
                    list_text::text(lang, ListText::Run),
                    list_text::text(lang, ListText::ReadyToRun)
                ));
            } else {
                for blocker in &readiness.blockers {
                    lines.push(format!(
                        "[{}] {}",
                        list_text::text(lang, ListText::Need),
                        list_text::blocker(lang, blocker)
                    ));
                }
            }
            lines
        }
        PreviewState::Ready(SourcePreview::Text(preview)) => vec![
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Source),
                preview.file.display()
            ),
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Kind),
                list_text::source_kind_label(lang, preview.source_kind, None)
            ),
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Preview),
                preview.diagnostic
            ),
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Run),
                list_text::readiness(lang, readiness)
            ),
        ],
        PreviewState::Ready(SourcePreview::Binary(preview)) => vec![
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Source),
                preview.file.display()
            ),
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Kind),
                list_text::text(lang, ListText::UnsupportedBinary)
            ),
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Preview),
                preview.diagnostic
            ),
            format!(
                "[{}] {}",
                list_text::text(lang, ListText::Run),
                list_text::readiness(lang, readiness)
            ),
        ],
    }
}

fn build_table_summary(
    lang: &str,
    preview: &TableSourceData,
    column_modes: &BTreeMap<usize, ColumnMode>,
    readiness: &FileTranslateRunReadiness,
) -> (String, bool) {
    let mut lines = vec![
        list_text::field(lang, ListText::File, preview.file.display()),
        list_text::field(
            lang,
            ListText::Kind,
            list_text::source_kind_label(lang, preview.source_kind, preview.json_shape),
        ),
        list_text::field(
            lang,
            ListText::Encoding,
            list_text::encoding_label(lang, preview.encoding),
        ),
        list_text::field(lang, ListText::Rows, preview.total_rows),
        list_text::field(lang, ListText::Cols, preview.column_labels.len()),
        list_text::field(
            lang,
            ListText::Header,
            list_text::header_mode(lang, preview.header_mode),
        ),
        list_text::field(
            lang,
            ListText::HeaderSuggestion,
            list_text::header_suggestion(lang, preview.suggested_header),
        ),
        list_text::field(
            lang,
            ListText::OutputDirectoryAction,
            slot_action_line(lang, readiness),
        ),
        list_text::field(lang, ListText::Run, list_text::readiness(lang, readiness)),
    ];

    if let Some(delimiter) = preview.delimiter {
        lines.push(list_text::field(
            lang,
            ListText::Delimiter,
            format!("'{}'", delimiter),
        ));
    }
    if let Some(shape) = preview.json_shape {
        lines.push(list_text::field(
            lang,
            ListText::JsonTable,
            list_text::json_table_shape_label(lang, shape),
        ));
    }
    if let Some(diagnostic) = &preview.json_diagnostic {
        lines.push(list_text::field(lang, ListText::JsonDiagnostic, diagnostic));
    }
    lines.push(format!(
        "{}: {}",
        list_text::text(lang, ListText::SelectedColumns),
        selected_columns_line(lang, preview, column_modes)
    ));

    if let Some(first_row) = preview.rows.first() {
        let sample = first_row
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                column_modes.get(index).copied().unwrap_or(ColumnMode::None) != ColumnMode::None
            })
            .map(|(index, value)| {
                let mode = column_modes
                    .get(&index)
                    .copied()
                    .unwrap_or(ColumnMode::None);
                format!(
                    "{} [{}]={}",
                    preview.column_labels[index],
                    list_text::column_mode(lang, mode),
                    value
                )
            })
            .collect::<Vec<_>>();
        if !sample.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "{}: {}",
                list_text::text(lang, ListText::Sample),
                sample.join(" | ")
            ));
        }
    }

    (lines.join("\n"), false)
}

fn build_text_summary(
    lang: &str,
    kind: &str,
    preview_lines: &[String],
    prefix_lines: &[String],
) -> String {
    let mut lines = prefix_lines.to_vec();
    if !preview_lines.is_empty() {
        lines.push(String::new());
        lines.push(list_text::field(lang, ListText::Preview, kind));
        lines.extend(preview_lines.iter().cloned());
    }
    lines.join("\n")
}

fn header_line(lang: &str, preview: &TableSourceData) -> String {
    match preview.source_kind {
        SourceKind::DelimitedText => {
            if preview.header_mode == HeaderMode::Unknown {
                format!(
                    "{} ({})",
                    list_text::header_mode(lang, preview.header_mode),
                    list_text::header_suggestion(lang, preview.suggested_header)
                )
            } else {
                list_text::header_mode(lang, preview.header_mode).to_string()
            }
        }
        SourceKind::JsonText => list_text::header_mode(lang, preview.header_mode).to_string(),
        _ => list_text::header_mode(lang, preview.header_mode).to_string(),
    }
}

fn selected_columns_line(
    lang: &str,
    preview: &TableSourceData,
    column_modes: &BTreeMap<usize, ColumnMode>,
) -> String {
    let selected = preview
        .column_labels
        .iter()
        .enumerate()
        .filter_map(|(index, label)| {
            let mode = column_modes
                .get(&index)
                .copied()
                .unwrap_or(ColumnMode::None);
            (mode != ColumnMode::None).then(|| {
                format!(
                    "{}: {} [{}]",
                    index + 1,
                    label,
                    list_text::column_mode(lang, mode)
                )
            })
        })
        .collect::<Vec<_>>();

    if selected.is_empty() {
        "-".to_string()
    } else {
        selected.join(", ")
    }
}

fn slot_action_line(lang: &str, readiness: &FileTranslateRunReadiness) -> String {
    match &readiness.dict_slot_action {
        Some(DictSlotAction::UseCommitted(path)) => format!(
            "{}: {}",
            list_text::text(lang, ListText::UseCommittedOutputDirectory),
            path.display()
        ),
        Some(DictSlotAction::CreateForRun {
            parent,
            target_lang,
        }) => format!(
            "{}: {} {} {}",
            list_text::text(lang, ListText::UseListOutputDirectory),
            target_lang,
            list_text::text(lang, ListText::Root),
            parent.display()
        ),
        None => list_text::text(lang, ListText::Unavailable).to_string(),
    }
}

fn slot_line(lang: &str, readiness: &FileTranslateRunReadiness) -> String {
    match &readiness.dict_slot_action {
        Some(DictSlotAction::UseCommitted(path)) => format!(
            "{}: {}",
            list_text::text(lang, ListText::UseCommittedOutputDirectory),
            path.display()
        ),
        Some(DictSlotAction::CreateForRun {
            parent,
            target_lang,
        }) => format!(
            "{}: {} {} {}",
            list_text::text(lang, ListText::ListOutputDirectoryWillBeUsed),
            target_lang,
            list_text::text(lang, ListText::Root),
            parent.display()
        ),
        None => list_text::readiness(lang, readiness),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_preview_summary, build_run_log_seed};
    use crate::file_translate::state::{DictSlotAction, FileTranslateRunReadiness};
    use crate::file_translate::types::{
        ColumnMode, HeaderMode, PreviewState, SourceEncoding, SourceKind, SourcePreview,
        TableSourceData,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn ready_list_output_directory_fixture() -> (
        PreviewState,
        BTreeMap<usize, ColumnMode>,
        FileTranslateRunReadiness,
    ) {
        let table = TableSourceData {
            file: PathBuf::from(r"C:\assets\table.csv"),
            file_size: 16,
            source_kind: SourceKind::DelimitedText,
            encoding: SourceEncoding::Utf8,
            header_mode: HeaderMode::Present,
            suggested_header: true,
            header_row: Some(vec!["id".to_string(), "text".to_string()]),
            column_labels: vec!["id".to_string(), "text".to_string()],
            rows: vec![vec!["1".to_string(), "hello".to_string()]],
            total_rows: 1,
            delimiter: Some(','),
            json_shape: None,
            json_diagnostic: None,
        };
        let mut column_modes = BTreeMap::new();
        column_modes.insert(1, ColumnMode::Translate);
        let readiness = FileTranslateRunReadiness {
            selected_file: Some(table.file.clone()),
            table_source: Some(table.clone()),
            dict_slot_action: Some(DictSlotAction::CreateForRun {
                parent: PathBuf::from(r"C:\base\dicts\ja\text"),
                target_lang: "ja".to_string(),
            }),
            blockers: Vec::new(),
        };
        (
            PreviewState::Ready(SourcePreview::Table(table)),
            column_modes,
            readiness,
        )
    }

    #[test]
    fn create_for_run_preview_and_log_use_output_directory_wording() {
        let (preview, column_modes, readiness) = ready_list_output_directory_fixture();
        let (summary, is_error) = build_preview_summary("en", &preview, &column_modes, &readiness);
        let log = build_run_log_seed("en", &preview, &column_modes, &readiness).join("\n");

        assert!(!is_error);
        assert!(summary.contains("Use List output directory"));
        assert!(log.contains("List output directory will be used"));
    }
}
