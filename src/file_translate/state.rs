use super::types::{ColumnMode, HeaderMode, PreviewState, SourcePreview, TableSourceData};
use crate::backend::manager::dict_slot_matches_target;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Debug, Clone)]
pub enum FileTranslateScanMessage {
    Scanned {
        index: usize,
        path: PathBuf,
    },
    Done {
        root: PathBuf,
        sources: Vec<super::types::AssetSourceCandidate>,
    },
}

#[derive(Debug, Clone)]
pub enum FileTranslatePreviewMessage {
    Done {
        file: PathBuf,
        header_mode: HeaderMode,
        result: Result<SourcePreview, String>,
    },
}

pub struct FileTranslateState {
    /// List mode state. This is not panel visibility.
    pub active: bool,
    pub initialized: bool,
    pub root: Option<PathBuf>,
    pub sources: Vec<super::types::AssetSourceCandidate>,
    pub selected_source: Option<PathBuf>,
    pub preview: PreviewState,
    pub preview_loading: bool,
    pub preview_target: Option<PathBuf>,
    pub preview_header_mode: HeaderMode,
    pub column_modes: BTreeMap<usize, ColumnMode>,
    pub table_preview_row_limit: usize,
    pub text_preview_line_limit: usize,
    pub folder_pick_rx: Option<mpsc::Receiver<Option<PathBuf>>>,
    pub scan_rx: Option<mpsc::Receiver<FileTranslateScanMessage>>,
    pub preview_rx: Option<mpsc::Receiver<FileTranslatePreviewMessage>>,
    pub scan_in_progress: bool,
}

impl Default for FileTranslateState {
    fn default() -> Self {
        Self {
            active: false,
            initialized: false,
            root: None,
            sources: Vec::new(),
            selected_source: None,
            preview: PreviewState::Empty,
            preview_loading: false,
            preview_target: None,
            preview_header_mode: HeaderMode::Unknown,
            column_modes: BTreeMap::new(),
            table_preview_row_limit: 100,
            text_preview_line_limit:
                crate::ui::file_translate_panel::TEXT_PREVIEW_INITIAL_LINE_LIMIT,
            folder_pick_rx: None,
            scan_rx: None,
            preview_rx: None,
            scan_in_progress: false,
        }
    }
}

impl FileTranslateState {
    pub fn with_root(root: Option<PathBuf>) -> Self {
        Self {
            root,
            ..Default::default()
        }
    }

    pub fn reset_for_root(
        &mut self,
        root: Option<PathBuf>,
        sources: Vec<super::types::AssetSourceCandidate>,
    ) {
        self.root = root;
        self.initialized = true;
        self.sources = sources;
        self.selected_source = None;
        self.preview = PreviewState::Empty;
        self.preview_loading = false;
        self.preview_target = None;
        self.preview_header_mode = HeaderMode::Unknown;
        self.preview_rx = None;
        self.column_modes.clear();
        self.table_preview_row_limit = 100;
        self.text_preview_line_limit =
            crate::ui::file_translate_panel::TEXT_PREVIEW_INITIAL_LINE_LIMIT;
        self.scan_in_progress = false;
    }

    pub fn enter_list_mode(&mut self) {
        self.active = true;
    }

    pub fn leave_list_mode(&mut self) {
        self.active = false;
        self.folder_pick_rx = None;
    }

    pub fn is_list_mode(&self) -> bool {
        self.active
    }
}

/// Run output directory action resolved by readiness.
///
/// `UseCommitted` uses the committed `dict_slot` authority as the output
/// directory when it already matches the current target language.
///
/// `CreateForRun` does not create or commit a numbered dictionary slot.
/// It resolves a stable output-only directory for the target:
///
///   dicts/{target}/text/list_output
///
/// `CreateForRun` does NOT:
/// - change the committed dict_slot authority
/// - save to config.toml
/// - use dictionary lookup or register on the output directory
///
/// The directory holds `{source_stem}.txt` List results in dict.txt format, not dictionary authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DictSlotAction {
    UseCommitted(PathBuf),
    CreateForRun {
        parent: PathBuf,
        target_lang: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunBlocker {
    NoSourceSelected,
    SourceMissing(PathBuf),
    PreviewLoading,
    PreviewUnavailable(String),
    TableSourceRequired,
    HeaderConfirmationRequired,
    NoColumnsSelected,
    DictSlotUnavailable(String),
}

#[derive(Debug, Clone)]
pub struct FileTranslateRunReadiness {
    pub selected_file: Option<PathBuf>,
    pub table_source: Option<TableSourceData>,
    pub dict_slot_action: Option<DictSlotAction>,
    pub blockers: Vec<RunBlocker>,
}

impl FileTranslateRunReadiness {
    pub fn is_ready(&self) -> bool {
        self.blockers.is_empty() && self.table_source.is_some() && self.dict_slot_action.is_some()
    }
}

pub fn evaluate_run_readiness(
    state: &FileTranslateState,
    committed_dict_slot: Option<&str>,
    target_lang: &str,
    base_dir: &Path,
) -> FileTranslateRunReadiness {
    let target_lang = target_lang.trim();
    let mut blockers = Vec::new();
    let selected_file = state.selected_source.clone();

    if let Some(path) = selected_file.as_ref() {
        if !path.exists() {
            blockers.push(RunBlocker::SourceMissing(path.clone()));
        }
    } else {
        blockers.push(RunBlocker::NoSourceSelected);
    }

    if state.preview_loading {
        blockers.push(RunBlocker::PreviewLoading);
    }

    let table_source = match &state.preview {
        PreviewState::Ready(preview) => preview.as_table().cloned(),
        PreviewState::Error(reason) => {
            blockers.push(RunBlocker::PreviewUnavailable(reason.clone()));
            None
        }
        PreviewState::Empty => None,
    };

    match &table_source {
        Some(table) => {
            if table.requires_header_confirmation() {
                blockers.push(RunBlocker::HeaderConfirmationRequired);
            }
            let valid_column_count = table.column_labels.len();
            let any_valid_selected = state
                .column_modes
                .iter()
                .any(|(&index, mode)| *mode != ColumnMode::None && index < valid_column_count);
            if !any_valid_selected {
                blockers.push(RunBlocker::NoColumnsSelected);
            }
        }
        None if matches!(state.preview, PreviewState::Ready(_)) => {
            blockers.push(RunBlocker::TableSourceRequired);
        }
        None => {}
    }

    let dict_slot_action = match committed_dict_slot
        .map(str::trim)
        .filter(|slot| !slot.is_empty())
    {
        Some(slot) if dict_slot_matches_target(Path::new(slot), target_lang) => {
            Some(DictSlotAction::UseCommitted(PathBuf::from(slot)))
        }
        Some(_) | None => {
            if target_lang.is_empty() {
                blockers.push(RunBlocker::DictSlotUnavailable(
                    "Target language is required before creating a List output directory"
                        .to_string(),
                ));
                None
            } else {
                Some(DictSlotAction::CreateForRun {
                    parent: base_dir.join("dicts").join(target_lang).join("text"),
                    target_lang: target_lang.to_string(),
                })
            }
        }
    };

    FileTranslateRunReadiness {
        selected_file,
        table_source,
        dict_slot_action,
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate_run_readiness, DictSlotAction, FileTranslateState, RunBlocker};
    use crate::file_translate::types::{
        AssetSourceCandidate, ColumnMode, HeaderMode, JsonTableShape, PreviewState, SourceEncoding,
        SourceKind, SourcePreview, TableSourceData,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tenuki_state_test_{}_{}", name, stamp))
    }

    fn sample_table(file: PathBuf) -> TableSourceData {
        TableSourceData {
            file,
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
        }
    }

    #[test]
    fn list_mode_can_enter_before_scan_initialization() {
        let mut state = FileTranslateState::default();

        state.enter_list_mode();

        assert!(state.is_list_mode());
        assert!(!state.initialized);
        assert!(state.root.is_none());
        assert!(state.sources.is_empty());
    }

    #[test]
    fn list_mode_active_is_mode_not_scanned_context() {
        let root = PathBuf::from(r"C:\assets");
        let source_path = root.join("table.csv");
        let mut state = FileTranslateState::default();
        state.reset_for_root(
            Some(root.clone()),
            vec![AssetSourceCandidate {
                path: source_path.clone(),
                kind: SourceKind::DelimitedText,
                encoding: SourceEncoding::Utf8,
                file_size: 16,
                diagnostic: "ok".to_string(),
            }],
        );
        state.selected_source = Some(source_path.clone());

        state.enter_list_mode();
        assert!(state.is_list_mode());

        state.leave_list_mode();
        assert!(!state.is_list_mode());
        assert_eq!(state.root, Some(root));
        assert_eq!(state.sources.len(), 1);
        assert_eq!(state.selected_source, Some(source_path));
    }

    #[test]
    fn readiness_allows_existing_slot_when_table_and_columns_are_ready() {
        let path = unique_path("ready.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        state.column_modes.insert(1, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(readiness.is_ready());
        assert_eq!(
            readiness.dict_slot_action,
            Some(DictSlotAction::UseCommitted(PathBuf::from(
                r"C:\dicts\ja\text\ja_001"
            )))
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_plans_create_for_run_slot_when_none_is_committed() {
        let path = unique_path("new_slot.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        state.column_modes.insert(1, ColumnMode::Translate);

        let base_dir = PathBuf::from(r"C:\base");
        let readiness = evaluate_run_readiness(&state, None, "ja", &base_dir);

        assert!(readiness.is_ready());
        assert_eq!(
            readiness.dict_slot_action,
            Some(DictSlotAction::CreateForRun {
                parent: base_dir.join("dicts").join("ja").join("text"),
                target_lang: "ja".to_string(),
            })
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_uses_list_output_directory_when_committed_slot_target_mismatches() {
        let path = unique_path("mismatch_slot.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        state.column_modes.insert(1, ColumnMode::Translate);

        let base_dir = PathBuf::from(r"C:\base");
        let readiness =
            evaluate_run_readiness(&state, Some(r"C:\dicts\ja\text\ja_001"), "en", &base_dir);

        assert!(readiness.is_ready());
        assert_eq!(
            readiness.dict_slot_action,
            Some(DictSlotAction::CreateForRun {
                parent: base_dir.join("dicts").join("en").join("text"),
                target_lang: "en".to_string(),
            })
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_keeps_legacy_slot_when_it_belongs_to_current_target() {
        let path = unique_path("legacy_slot.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        state.column_modes.insert(1, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\S_0001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert_eq!(
            readiness.dict_slot_action,
            Some(DictSlotAction::UseCommitted(PathBuf::from(
                r"C:\dicts\ja\text\S_0001"
            )))
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_does_not_create_run_slot_without_target_language() {
        let path = unique_path("missing_target.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        state.column_modes.insert(1, ColumnMode::Translate);

        let readiness =
            evaluate_run_readiness(&state, None, " ", PathBuf::from(r"C:\base").as_path());

        assert!(!readiness.is_ready());
        assert_eq!(readiness.dict_slot_action, None);
        assert!(matches!(
            readiness.blockers.as_slice(),
            [RunBlocker::DictSlotUnavailable(_)]
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_blocks_while_preview_is_loading() {
        let path = unique_path("loading.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview_loading = true;
        state.preview_target = Some(path.clone());
        state.preview_header_mode = HeaderMode::Present;

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(!readiness.is_ready());
        assert!(readiness.blockers.contains(&RunBlocker::PreviewLoading));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_blocks_when_header_confirmation_is_missing() {
        let path = unique_path("header.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut table = sample_table(path.clone());
        table.header_mode = HeaderMode::Unknown;

        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(table));
        state.column_modes.insert(1, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(!readiness.is_ready());
        assert!(readiness
            .blockers
            .contains(&RunBlocker::HeaderConfirmationRequired));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn json_table_never_requires_header_confirmation() {
        let path = unique_path("json_shape.csv");
        std::fs::write(&path, "dummy\n").unwrap();

        let mut table = sample_table(path.clone());
        table.source_kind = SourceKind::JsonText;
        table.header_mode = HeaderMode::Present;
        table.json_shape = Some(JsonTableShape::ArrayOfObjects);

        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(table));
        state.column_modes.insert(0, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(!readiness
            .blockers
            .contains(&RunBlocker::HeaderConfirmationRequired));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn delimited_unknown_requires_header_confirmation() {
        let path = unique_path("header_unknown.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut table = sample_table(path.clone());
        table.source_kind = SourceKind::DelimitedText;
        table.header_mode = HeaderMode::Unknown;

        assert!(table.requires_header_confirmation());

        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(table));
        state.column_modes.insert(0, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(!readiness.is_ready());
        assert!(readiness
            .blockers
            .contains(&RunBlocker::HeaderConfirmationRequired));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn json_table_does_not_support_header_toggle() {
        let path = unique_path("json_notoggle.csv");
        std::fs::write(&path, "dummy\n").unwrap();
        let mut table = sample_table(path.clone());
        table.source_kind = SourceKind::JsonText;
        table.json_shape = Some(JsonTableShape::ArrayOfObjects);

        assert!(!table.supports_header_toggle());
        assert!(!table.requires_header_confirmation());

        let mut table2 = sample_table(path.clone());
        table2.source_kind = SourceKind::JsonText;
        table2.json_shape = Some(JsonTableShape::ArrayOfArrays);

        assert!(!table2.supports_header_toggle());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_blocks_when_all_columns_are_none() {
        let path = unique_path("all_none.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(!readiness.is_ready());
        assert!(readiness.blockers.contains(&RunBlocker::NoColumnsSelected));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_ignores_stale_out_of_range_column_modes() {
        let path = unique_path("stale_columns.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        state.column_modes.insert(1, ColumnMode::Translate);
        // stale column index >= column_labels.len()
        state.column_modes.insert(99, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(readiness.is_ready());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_blocks_when_only_out_of_range_columns_are_selected() {
        let path = unique_path("only_stale_columns.csv");
        std::fs::write(&path, "id,text\n1,hello\n").unwrap();
        let mut state = FileTranslateState::default();
        state.selected_source = Some(path.clone());
        state.preview = PreviewState::Ready(SourcePreview::Table(sample_table(path.clone())));
        // all columns are out of range
        state.column_modes.insert(99, ColumnMode::Translate);

        let readiness = evaluate_run_readiness(
            &state,
            Some(r"C:\dicts\ja\text\ja_001"),
            "ja",
            PathBuf::from(r"C:\base").as_path(),
        );

        assert!(!readiness.is_ready());
        assert!(readiness.blockers.contains(&RunBlocker::NoColumnsSelected));
        let _ = std::fs::remove_file(path);
    }
}
