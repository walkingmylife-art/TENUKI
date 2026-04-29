use super::types::{
    AssetSourceCandidate, BinaryPreview, HeaderMode, IntakeError, JsonTableShape, SourceEncoding,
    SourceKind, SourcePreview, TableSourceData, TextPreview,
};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
const SNIFF_BYTES: usize = 8 * 1024;
const UTF8_BOM: &[u8; 3] = b"\xEF\xBB\xBF";
const DELIMITERS: [u8; 4] = [b',', b'\t', b';', b'|'];

pub fn scan_asset_sources(root: &Path) -> Vec<AssetSourceCandidate> {
    scan_asset_sources_with_progress(root, |_index, _candidate| {})
}

pub fn scan_asset_sources_with_progress<F>(
    root: &Path,
    mut on_candidate: F,
) -> Vec<AssetSourceCandidate>
where
    F: FnMut(usize, &AssetSourceCandidate),
{
    let mut sources = Vec::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.into_path();
        let candidate =
            inspect_source_candidate(&path).unwrap_or_else(|err| AssetSourceCandidate {
                path,
                kind: SourceKind::UnknownText,
                encoding: SourceEncoding::Unknown,
                file_size: 0,
                diagnostic: format!("scan failed: {}", err),
            });
        let index = sources.len() + 1;
        on_candidate(index, &candidate);
        sources.push(candidate);
    }

    sources.sort_by(|a, b| a.path.cmp(&b.path));
    sources
}

pub fn load_source_preview(
    path: &Path,
    header_mode: HeaderMode,
) -> Result<SourcePreview, IntakeError> {
    let file_size = std::fs::metadata(path)
        .map_err(|e| IntakeError::Io(e.to_string()))?
        .len();
    let bytes = read_all_bytes(path, file_size)?;

    match decode_utf8_text(&bytes) {
        DecodedSource::Binary(reason) => Ok(SourcePreview::Binary(BinaryPreview {
            file: path.to_path_buf(),
            file_size,
            diagnostic: reason,
        })),
        DecodedSource::Text { encoding, text } => {
            let kind = sniff_source_kind(&text);
            match kind {
                SourceKind::DelimitedText => {
                    Ok(SourcePreview::Table(load_delimited_table_from_text(
                        path.to_path_buf(),
                        file_size,
                        encoding,
                        &text,
                        header_mode,
                    )?))
                }
                SourceKind::JsonText => {
                    match normalize_json_table(path.to_path_buf(), file_size, encoding, &text) {
                        Ok(table) => Ok(SourcePreview::Table(table)),
                        Err(reason) => Ok(SourcePreview::Text(build_text_preview(
                            path,
                            file_size,
                            SourceKind::JsonText,
                            encoding,
                            &text,
                            &format!("JSON preview-only: {}", reason),
                        ))),
                    }
                }
                SourceKind::PlainLines | SourceKind::MarkupText | SourceKind::UnknownText => {
                    Ok(SourcePreview::Text(build_text_preview(
                        path,
                        file_size,
                        kind,
                        encoding,
                        &text,
                        kind_diagnostic(kind, &text),
                    )))
                }
                SourceKind::UnsupportedBinary => Ok(SourcePreview::Binary(BinaryPreview {
                    file: path.to_path_buf(),
                    file_size,
                    diagnostic: "binary-like source".to_string(),
                })),
            }
        }
    }
}

/// Apply a resolved header mode to a `TableSourceData` that was loaded with
/// `HeaderMode::Unknown`. Only DelimitedText tables are transformed; JSON and
/// non-table sources pass through unchanged.
///
/// `Present` removes the first row from `rows` into `header_row` and updates
/// `column_labels`. `Absent` keeps all rows as data with `col N` labels.
/// No file I/O is performed.
pub fn apply_delimited_header_mode_from_unknown(
    mut table: TableSourceData,
    mode: HeaderMode,
) -> TableSourceData {
    if table.source_kind != SourceKind::DelimitedText || table.header_mode != HeaderMode::Unknown {
        return table;
    }
    if mode == HeaderMode::Unknown {
        return table;
    }

    let max_columns = table
        .rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(table.column_labels.len());

    match mode {
        HeaderMode::Present => {
            if table.rows.is_empty() {
                return table;
            }
            let header_row = table.rows.remove(0);
            table.column_labels = build_column_labels(Some(&header_row), max_columns);
            table.header_row = Some(header_row);
            table.header_mode = HeaderMode::Present;
            table.total_rows = table.rows.len();
        }
        HeaderMode::Absent => {
            table.column_labels = build_column_labels(None, max_columns);
            table.header_row = None;
            table.header_mode = HeaderMode::Absent;
            table.total_rows = table.rows.len();
        }
        HeaderMode::Unknown => {}
    }

    table
}

pub fn max_source_bytes() -> u64 {
    MAX_SOURCE_BYTES
}

fn inspect_source_candidate(path: &Path) -> Result<AssetSourceCandidate, IntakeError> {
    let file_size = std::fs::metadata(path)
        .map_err(|e| IntakeError::Io(e.to_string()))?
        .len();
    let bytes = read_sniff_bytes(path)?;
    let (kind, encoding, diagnostic) = match decode_utf8_text(&bytes) {
        DecodedSource::Binary(reason) => (
            SourceKind::UnsupportedBinary,
            SourceEncoding::Binary,
            reason,
        ),
        DecodedSource::Text { encoding, text } => {
            let kind = sniff_source_kind(&text);
            let diagnostic = match kind {
                SourceKind::DelimitedText => match sniff_delimited_shape(&text) {
                    Some(sniff) => format!(
                        "delimiter: '{}' / columns: {}",
                        sniff.delimiter, sniff.column_count
                    ),
                    None => "delimiter-stable table text".to_string(),
                },
                SourceKind::JsonText => "JSON-like text".to_string(),
                SourceKind::PlainLines => format!("{} lines", text.lines().count().max(1)),
                SourceKind::MarkupText => "markup-like text".to_string(),
                SourceKind::UnsupportedBinary => "binary-like source".to_string(),
                SourceKind::UnknownText => "text decoded but kind is unknown".to_string(),
            };
            (kind, encoding, diagnostic)
        }
    };

    Ok(AssetSourceCandidate {
        path: path.to_path_buf(),
        kind,
        encoding,
        file_size,
        diagnostic,
    })
}

fn read_sniff_bytes(path: &Path) -> Result<Vec<u8>, IntakeError> {
    let file = File::open(path).map_err(|e| IntakeError::Io(e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(SNIFF_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| IntakeError::Io(e.to_string()))?;
    Ok(bytes)
}

fn read_all_bytes(path: &Path, file_size: u64) -> Result<Vec<u8>, IntakeError> {
    if file_size > MAX_SOURCE_BYTES {
        return Err(IntakeError::TooLarge {
            bytes: file_size,
            limit: MAX_SOURCE_BYTES,
        });
    }
    std::fs::read(path).map_err(|e| IntakeError::Io(e.to_string()))
}

fn build_text_preview(
    path: &Path,
    file_size: u64,
    source_kind: SourceKind,
    encoding: SourceEncoding,
    text: &str,
    diagnostic: &str,
) -> TextPreview {
    let lines = text
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let line_count = text.lines().count().max(usize::from(!text.is_empty()));

    TextPreview {
        file: path.to_path_buf(),
        file_size,
        source_kind,
        encoding,
        lines,
        line_count,
        diagnostic: diagnostic.to_string(),
    }
}

fn kind_diagnostic(kind: SourceKind, text: &str) -> &'static str {
    match kind {
        SourceKind::PlainLines => {
            let _ = text;
            "plain text lines"
        }
        SourceKind::MarkupText => "markup-like text",
        SourceKind::UnknownText => "text decoded but kind is unknown",
        SourceKind::DelimitedText => "delimiter-stable table text",
        SourceKind::JsonText => "JSON-like text",
        SourceKind::UnsupportedBinary => "binary-like source",
    }
}

fn load_delimited_table_from_text(
    path: PathBuf,
    file_size: u64,
    encoding: SourceEncoding,
    text: &str,
    header_mode: HeaderMode,
) -> Result<TableSourceData, IntakeError> {
    let sniff = sniff_delimited_shape(text)
        .ok_or_else(|| IntakeError::Delimited("delimiter stability not detected".to_string()))?;
    let mut records = parse_delimited_records(text, sniff.delimiter as u8)?;
    if records.is_empty() {
        return Err(IntakeError::EmptyFile);
    }

    let max_columns = records.iter().map(Vec::len).max().unwrap_or(0);
    if max_columns < 1 {
        return Err(IntakeError::Delimited(
            "expected at least 1 column".to_string(),
        ));
    }

    let suggested_header = infer_header_row(&records);
    let header_row = match header_mode {
        HeaderMode::Present => Some(records.remove(0)),
        HeaderMode::Unknown | HeaderMode::Absent => None,
    };
    let column_labels = match header_mode {
        HeaderMode::Present => build_column_labels(header_row.as_deref(), max_columns),
        HeaderMode::Unknown | HeaderMode::Absent => build_column_labels(None, max_columns),
    };

    Ok(TableSourceData {
        file: path,
        file_size,
        source_kind: SourceKind::DelimitedText,
        encoding,
        header_mode,
        suggested_header,
        header_row,
        column_labels,
        total_rows: records.len(),
        rows: records,
        delimiter: Some(sniff.delimiter),
        json_shape: None,
        json_diagnostic: None,
    })
}

fn parse_delimited_records(text: &str, delimiter: u8) -> Result<Vec<Vec<String>>, IntakeError> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut records = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| IntakeError::Delimited(e.to_string()))?;
        if record.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        records.push(
            record
                .iter()
                .map(|cell| cell.to_string())
                .collect::<Vec<_>>(),
        );
    }
    Ok(records)
}

fn build_column_labels(header_row: Option<&[String]>, max_columns: usize) -> Vec<String> {
    match header_row {
        Some(header_row) => (0..max_columns)
            .map(|index| {
                let label = header_row
                    .get(index)
                    .map(|cell| cell.trim())
                    .filter(|cell| !cell.is_empty())
                    .map(str::to_string);
                label.unwrap_or_else(|| format!("col {}", index + 1))
            })
            .collect(),
        None => (0..max_columns)
            .map(|index| format!("col {}", index + 1))
            .collect(),
    }
}

fn infer_header_row(records: &[Vec<String>]) -> bool {
    if records.len() < 2 || records[0].len() < 2 {
        return false;
    }

    let first = &records[0];
    let second = &records[1];
    let first_text = first.iter().filter(|cell| is_textish(cell)).count();
    let second_text = second.iter().filter(|cell| is_textish(cell)).count();
    let first_numeric = first.iter().filter(|cell| is_numericish(cell)).count();
    let second_numeric = second.iter().filter(|cell| is_numericish(cell)).count();
    let header_tokens = first
        .iter()
        .filter(|cell| looks_like_header_token(cell))
        .count();
    let unique_non_empty = {
        let mut seen = std::collections::BTreeSet::new();
        first
            .iter()
            .filter(|cell| !cell.trim().is_empty())
            .all(|cell| seen.insert(normalize_token(cell)))
    };

    let mut score = 0usize;
    if header_tokens > 0 {
        score += 2;
    }
    if unique_non_empty {
        score += 1;
    }
    if first_text >= second_text && first_numeric < second_numeric {
        score += 1;
    }

    score >= 2
}

fn looks_like_header_token(cell: &str) -> bool {
    const HEADER_TOKENS: &[&str] = &[
        "id",
        "key",
        "name",
        "desc",
        "describe",
        "type",
        "num",
        "tips",
        "english",
        "francais",
        "tc",
        "郛門捷",
        "蠎丞捷",
        "蜷榊ｭ・",
        "蜷咲ｧｰ",
        "謠剰ｿｰ",
        "蜀・ｮｹ",
        "邀ｻ蛻ｫ",
        "鬚懆牡",
        "迚ｹ謨・",
    ];

    let normalized = normalize_token(cell);
    HEADER_TOKENS.iter().any(|token| normalized == *token)
}

fn normalize_token(cell: &str) -> String {
    cell.trim().to_lowercase().replace([' ', '_', '-'], "")
}

fn is_numericish(cell: &str) -> bool {
    let trimmed = cell.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.parse::<f64>().is_ok()
}

fn is_textish(cell: &str) -> bool {
    cell.chars()
        .any(|ch| ch.is_ascii_alphabetic() || is_cjk(ch))
}

fn is_cjk(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
        || ('\u{3400}'..='\u{4DBF}').contains(&ch)
        || ('\u{3040}'..='\u{30FF}').contains(&ch)
        || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
}

fn normalize_json_table(
    path: PathBuf,
    file_size: u64,
    encoding: SourceEncoding,
    text: &str,
) -> Result<TableSourceData, String> {
    let value = serde_json::from_str::<serde_json::Value>(text)
        .map_err(|e| format!("parse failed: {}", e))?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        _ => {
            return Err("root must be array<object> or array<array>".to_string());
        }
    };

    if items.is_empty() {
        return Err("root array is empty".to_string());
    }

    if items.iter().all(|value| value.is_object()) {
        return normalize_json_object_rows(path, file_size, encoding, items);
    }
    if items.iter().all(|value| value.is_array()) {
        return normalize_json_array_rows(path, file_size, encoding, items);
    }

    Err("root array is heterogeneous and stays preview-only".to_string())
}

fn normalize_json_object_rows(
    path: PathBuf,
    file_size: u64,
    encoding: SourceEncoding,
    items: Vec<serde_json::Value>,
) -> Result<TableSourceData, String> {
    let mut column_labels = Vec::new();

    for item in &items {
        let object = item
            .as_object()
            .ok_or_else(|| "object table expected".to_string())?;
        for key in object.keys() {
            if !column_labels.iter().any(|existing| existing == key) {
                column_labels.push(key.clone());
            }
        }
    }

    if column_labels.is_empty() {
        return Err("array<object> has no keys".to_string());
    }

    let rows = items
        .into_iter()
        .map(|item| {
            let object = item.as_object().expect("object rows were checked above");
            column_labels
                .iter()
                .map(|label| {
                    object
                        .get(label)
                        .map(json_value_to_cell)
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    Ok(TableSourceData {
        file: path,
        file_size,
        source_kind: SourceKind::JsonText,
        encoding,
        header_mode: HeaderMode::Present,
        suggested_header: true,
        header_row: Some(column_labels.clone()),
        total_rows: rows.len(),
        column_labels,
        rows,
        delimiter: None,
        json_shape: Some(JsonTableShape::ArrayOfObjects),
        json_diagnostic: Some(
            "object keys are used as columns; nested values are stringified".to_string(),
        ),
    })
}

fn normalize_json_array_rows(
    path: PathBuf,
    file_size: u64,
    encoding: SourceEncoding,
    items: Vec<serde_json::Value>,
) -> Result<TableSourceData, String> {
    let rows = items
        .into_iter()
        .map(|item| {
            item.as_array()
                .ok_or_else(|| "array rows expected".to_string())
                .map(|row| row.iter().map(json_value_to_cell).collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let column_count = rows.first().map(Vec::len).unwrap_or(0);
    if column_count == 0 {
        return Err("array<array> rows are empty".to_string());
    }
    if rows.iter().any(|row| row.len() != column_count) {
        return Err("array<array> column counts are irregular".to_string());
    }

    Ok(TableSourceData {
        file: path,
        file_size,
        source_kind: SourceKind::JsonText,
        encoding,
        header_mode: HeaderMode::Absent,
        suggested_header: false,
        header_row: None,
        column_labels: (0..column_count)
            .map(|index| format!("col {}", index + 1))
            .collect(),
        total_rows: rows.len(),
        rows,
        delimiter: None,
        json_shape: Some(JsonTableShape::ArrayOfArrays),
        json_diagnostic: Some(
            "array indices are used as columns; nested values are stringified".to_string(),
        ),
    })
}

fn json_value_to_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "<json>".to_string())
        }
    }
}

fn sniff_source_kind(text: &str) -> SourceKind {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return SourceKind::UnknownText;
    }
    if looks_like_json(trimmed) {
        return SourceKind::JsonText;
    }
    if looks_like_markup(trimmed) {
        return SourceKind::MarkupText;
    }
    if sniff_delimited_shape(text).is_some() {
        return SourceKind::DelimitedText;
    }
    if text.lines().count() >= 2 {
        return SourceKind::PlainLines;
    }
    SourceKind::UnknownText
}

fn looks_like_json(trimmed: &str) -> bool {
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn looks_like_markup(trimmed: &str) -> bool {
    trimmed.starts_with('<') || trimmed.starts_with("---") || trimmed.starts_with("%YAML")
}

#[derive(Clone, Copy)]
struct DelimitedSniff {
    delimiter: char,
    column_count: usize,
    row_count: usize,
}

fn sniff_delimited_shape(text: &str) -> Option<DelimitedSniff> {
    let mut best: Option<DelimitedSniff> = None;

    for delimiter in DELIMITERS {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());

        let mut rows = Vec::new();
        for record in reader.records().take(8) {
            let record = match record {
                Ok(record) => record,
                Err(_) => {
                    rows.clear();
                    break;
                }
            };
            if record.iter().all(|cell| cell.trim().is_empty()) {
                continue;
            }
            rows.push(record.len());
        }

        if rows.is_empty() {
            continue;
        }

        let column_count = rows[0];
        let stable =
            rows.len() >= 2 && column_count >= 1 && rows.iter().all(|len| *len == column_count);

        if !stable {
            continue;
        }

        let sniff = DelimitedSniff {
            delimiter: delimiter as char,
            column_count,
            row_count: rows.len(),
        };
        let current_score = column_count * rows.len();
        let replace = best
            .map(|best| current_score > best.column_count * best.row_count)
            .unwrap_or(true);
        if replace {
            best = Some(sniff);
        }
    }

    best
}

enum DecodedSource {
    Text {
        encoding: SourceEncoding,
        text: String,
    },
    Binary(String),
}

fn decode_utf8_text(bytes: &[u8]) -> DecodedSource {
    if bytes.iter().any(|byte| *byte == 0) {
        return DecodedSource::Binary("contains NUL bytes".to_string());
    }

    let (encoding, body) = if bytes.starts_with(UTF8_BOM) {
        (SourceEncoding::Utf8Bom, &bytes[UTF8_BOM.len()..])
    } else {
        (SourceEncoding::Utf8, bytes)
    };

    match String::from_utf8(body.to_vec()) {
        Ok(text) => DecodedSource::Text { encoding, text },
        Err(_) => DecodedSource::Binary("contains non UTF-8 bytes".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_delimited_header_mode_from_unknown, load_source_preview, max_source_bytes,
        scan_asset_sources,
    };
    use std::path::PathBuf;

    use crate::file_translate::types::{
        HeaderMode, JsonTableShape, SourceEncoding, SourceKind, SourcePreview, TableSourceData,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tenuki_asset_intake_{}_{}", name, stamp))
    }

    #[test]
    fn detects_extensionless_delimited_text() {
        let root = unique_path("dir");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("AchievementData");
        std::fs::write(&path, b"\xEF\xBB\xBFID,Name\n1,one\n").unwrap();

        let sources = scan_asset_sources(&root);

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind.badge(), "DELIM");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(root);
    }

    #[test]
    fn detects_headerless_delimited_preview() {
        let path = unique_path("city");
        std::fs::write(&path, b"-3,-3,-1,-1,-1\n-2,-2,-1,-1,-1\n").unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Table(preview) => {
                assert!(preview.header_row.is_none());
                assert_eq!(preview.header_mode, HeaderMode::Unknown);
                assert_eq!(preview.column_labels[0], "col 1");
                assert_eq!(preview.total_rows, 2);
            }
            other => panic!("expected table preview, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn keeps_delimited_preview_shape_unmodified_until_header_is_confirmed() {
        let path = unique_path("header_unknown.csv");
        std::fs::write(&path, b"ID,Name\n1,one\n2,two\n").unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Table(preview) => {
                assert_eq!(preview.header_mode, HeaderMode::Unknown);
                assert!(preview.suggested_header);
                assert!(preview.header_row.is_none());
                assert_eq!(preview.column_labels, vec!["col 1", "col 2"]);
                assert_eq!(preview.total_rows, 3);
                assert_eq!(preview.rows[0], vec!["ID", "Name"]);
            }
            other => panic!("expected table preview, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn normalizes_json_array_of_objects_as_table() {
        let path = unique_path("json_objects");
        std::fs::write(
            &path,
            br#"[{"text":"hello","id":1},{"text":"world","meta":{"x":1}}]"#,
        )
        .unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Table(preview) => {
                assert_eq!(
                    preview.source_kind,
                    crate::file_translate::types::SourceKind::JsonText
                );
                assert_eq!(preview.json_shape, Some(JsonTableShape::ArrayOfObjects));
                assert_eq!(preview.column_labels, vec!["id", "text", "meta"]);
                assert_eq!(preview.rows[0][1], "hello");
                assert_eq!(preview.rows[1][2], r#"{"x":1}"#);
            }
            other => panic!("expected json table, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn normalizes_json_array_of_arrays_as_table() {
        let path = unique_path("json_arrays");
        std::fs::write(&path, br#"[["hello",1],["world",{"x":1}]]"#).unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Table(preview) => {
                assert_eq!(preview.json_shape, Some(JsonTableShape::ArrayOfArrays));
                assert_eq!(preview.column_labels, vec!["col 1", "col 2"]);
                assert_eq!(preview.rows[1][1], r#"{"x":1}"#);
            }
            other => panic!("expected json table, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn keeps_non_table_json_as_text_preview() {
        let path = unique_path("json_preview_only");
        std::fs::write(&path, br#"{"text":"hello"}"#).unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Text(preview) => {
                assert!(preview.diagnostic.contains("preview-only"));
            }
            other => panic!("expected text preview, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_irregular_json_arrays_for_run_preview() {
        let path = unique_path("json_irregular");
        std::fs::write(&path, br#"[["one"],["two",2]]"#).unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Text(preview) => {
                assert!(preview.diagnostic.contains("preview-only"));
            }
            other => panic!("expected text preview, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detects_binary_source() {
        let path = unique_path("binary");
        std::fs::write(&path, [0_u8, 159, 146, 150]).unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        assert!(matches!(preview, SourcePreview::Binary(_)));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn header_mode_absent_keeps_all_rows_as_data() {
        let path = unique_path("header_absent.csv");
        std::fs::write(&path, b"ID,Name\n1,hello\n2,world\n").unwrap();

        let preview = load_source_preview(&path, HeaderMode::Absent).unwrap();

        match preview {
            SourcePreview::Table(preview) => {
                assert_eq!(preview.header_mode, HeaderMode::Absent);
                assert!(preview.header_row.is_none());
                assert_eq!(preview.column_labels, vec!["col 1", "col 2"]);
                assert_eq!(preview.total_rows, 3);
                assert_eq!(preview.rows[0], vec!["ID", "Name"]);
            }
            other => panic!("expected table preview, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn json_table_always_has_header_mode_present_or_absent_never_unknown() {
        let path = unique_path("json_header_contract");
        std::fs::write(&path, br#"[{"key":"hello"}]"#).unwrap();

        let preview = load_source_preview(&path, HeaderMode::Unknown).unwrap();

        match preview {
            SourcePreview::Table(preview) => {
                assert_ne!(preview.header_mode, HeaderMode::Unknown);
                assert!(preview.suggested_header);
            }
            other => panic!("expected json table, got {:?}", other.kind()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_oversized_source() {
        let path = unique_path("large.csv");
        let oversized = vec![b'a'; (max_source_bytes() + 1) as usize];
        std::fs::write(&path, oversized).unwrap();

        let err = load_source_preview(&path, HeaderMode::Unknown).unwrap_err();

        assert!(err.to_string().contains("file too large"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn apply_unknown_header_suggestion_present_removes_first_row() {
        let table = TableSourceData {
            file: PathBuf::from("test.csv"),
            file_size: 16,
            source_kind: SourceKind::DelimitedText,
            encoding: SourceEncoding::Utf8,
            header_mode: HeaderMode::Unknown,
            suggested_header: true,
            header_row: None,
            column_labels: vec!["col 1".to_string(), "col 2".to_string()],
            rows: vec![
                vec!["ID".to_string(), "Name".to_string()],
                vec!["1".to_string(), "hello".to_string()],
            ],
            total_rows: 2,
            delimiter: Some(','),
            json_shape: None,
            json_diagnostic: None,
        };

        let resolved = apply_delimited_header_mode_from_unknown(table, HeaderMode::Present);

        assert_eq!(resolved.header_mode, HeaderMode::Present);
        assert_eq!(
            resolved.header_row,
            Some(vec!["ID".to_string(), "Name".to_string()])
        );
        assert_eq!(resolved.column_labels, vec!["ID", "Name"]);
        assert_eq!(resolved.total_rows, 1);
        assert_eq!(
            resolved.rows,
            vec![vec!["1".to_string(), "hello".to_string()]]
        );
    }

    #[test]
    fn apply_unknown_header_suggestion_absent_keeps_first_row() {
        let table = TableSourceData {
            file: PathBuf::from("test.csv"),
            file_size: 16,
            source_kind: SourceKind::DelimitedText,
            encoding: SourceEncoding::Utf8,
            header_mode: HeaderMode::Unknown,
            suggested_header: false,
            header_row: None,
            column_labels: vec!["col 1".to_string(), "col 2".to_string()],
            rows: vec![
                vec!["ID".to_string(), "Name".to_string()],
                vec!["1".to_string(), "hello".to_string()],
            ],
            total_rows: 2,
            delimiter: Some(','),
            json_shape: None,
            json_diagnostic: None,
        };

        let resolved = apply_delimited_header_mode_from_unknown(table, HeaderMode::Absent);

        assert_eq!(resolved.header_mode, HeaderMode::Absent);
        assert!(resolved.header_row.is_none());
        assert_eq!(resolved.column_labels, vec!["col 1", "col 2"]);
        assert_eq!(resolved.total_rows, 2);
        assert_eq!(resolved.rows[0], vec!["ID".to_string(), "Name".to_string()]);
    }

    #[test]
    fn apply_unknown_header_ignores_json_table() {
        let table = TableSourceData {
            file: PathBuf::from("test.json"),
            file_size: 16,
            source_kind: SourceKind::JsonText,
            encoding: SourceEncoding::Utf8,
            header_mode: HeaderMode::Unknown,
            suggested_header: true,
            header_row: None,
            column_labels: vec!["key".to_string()],
            rows: vec![vec!["value".to_string()]],
            total_rows: 1,
            delimiter: None,
            json_shape: Some(JsonTableShape::ArrayOfObjects),
            json_diagnostic: None,
        };

        let resolved = apply_delimited_header_mode_from_unknown(table, HeaderMode::Present);

        // JSON tables pass through unchanged.
        assert_eq!(resolved.header_mode, HeaderMode::Unknown);
        assert!(resolved.header_row.is_none());
    }
}
