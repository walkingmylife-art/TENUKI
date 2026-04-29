use crate::messages::BackendEvent;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColumnMode {
    Translate,
    Original,
    #[default]
    None,
}

impl ColumnMode {
    pub fn next(self) -> Self {
        match self {
            Self::Translate => Self::Original,
            Self::Original => Self::None,
            Self::None => Self::Translate,
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Translate => "T",
            Self::Original => "O",
            Self::None => "-",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HeaderMode {
    #[default]
    Unknown,
    Present,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SourceKind {
    DelimitedText,
    JsonText,
    PlainLines,
    MarkupText,
    UnsupportedBinary,
    #[default]
    UnknownText,
}

impl SourceKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::DelimitedText => "DELIM",
            Self::JsonText => "JSON",
            Self::PlainLines => "LINES",
            Self::MarkupText => "MARKUP",
            Self::UnsupportedBinary => "BINARY",
            Self::UnknownText => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SourceEncoding {
    Utf8,
    Utf8Bom,
    Binary,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JsonTableShape {
    ArrayOfObjects,
    ArrayOfArrays,
}

#[derive(Debug, Clone)]
pub struct AssetSourceCandidate {
    pub path: PathBuf,
    pub kind: SourceKind,
    pub encoding: SourceEncoding,
    pub file_size: u64,
    pub diagnostic: String,
}

#[derive(Debug, Clone)]
pub struct TableSourceData {
    pub file: PathBuf,
    pub file_size: u64,
    pub source_kind: SourceKind,
    pub encoding: SourceEncoding,
    pub header_mode: HeaderMode,
    pub suggested_header: bool,
    pub header_row: Option<Vec<String>>,
    pub column_labels: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub delimiter: Option<char>,
    pub json_shape: Option<JsonTableShape>,
    pub json_diagnostic: Option<String>,
}

impl TableSourceData {
    pub fn requires_header_confirmation(&self) -> bool {
        self.source_kind == SourceKind::DelimitedText && self.header_mode == HeaderMode::Unknown
    }

    pub fn supports_header_toggle(&self) -> bool {
        self.source_kind == SourceKind::DelimitedText
    }
}

#[derive(Debug, Clone)]
pub struct TextPreview {
    pub file: PathBuf,
    pub file_size: u64,
    pub source_kind: SourceKind,
    pub encoding: SourceEncoding,
    pub lines: Vec<String>,
    pub line_count: usize,
    pub diagnostic: String,
}

#[derive(Debug, Clone)]
pub struct BinaryPreview {
    pub file: PathBuf,
    pub file_size: u64,
    pub diagnostic: String,
}

#[derive(Debug, Clone)]
pub enum SourcePreview {
    Table(TableSourceData),
    Text(TextPreview),
    Binary(BinaryPreview),
}

impl SourcePreview {
    pub fn path(&self) -> &PathBuf {
        match self {
            Self::Table(preview) => &preview.file,
            Self::Text(preview) => &preview.file,
            Self::Binary(preview) => &preview.file,
        }
    }

    pub fn kind(&self) -> SourceKind {
        match self {
            Self::Table(preview) => preview.source_kind,
            Self::Text(preview) => preview.source_kind,
            Self::Binary(_) => SourceKind::UnsupportedBinary,
        }
    }

    pub fn encoding(&self) -> SourceEncoding {
        match self {
            Self::Table(preview) => preview.encoding,
            Self::Text(preview) => preview.encoding,
            Self::Binary(_) => SourceEncoding::Binary,
        }
    }

    pub fn file_size(&self) -> u64 {
        match self {
            Self::Table(preview) => preview.file_size,
            Self::Text(preview) => preview.file_size,
            Self::Binary(preview) => preview.file_size,
        }
    }

    pub fn as_table(&self) -> Option<&TableSourceData> {
        match self {
            Self::Table(preview) => Some(preview),
            Self::Text(_) | Self::Binary(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PreviewState {
    Empty,
    Error(String),
    Ready(SourcePreview),
}

impl Default for PreviewState {
    fn default() -> Self {
        Self::Empty
    }
}

#[derive(Debug, Clone)]
pub enum IntakeError {
    TooLarge { bytes: u64, limit: u64 },
    Io(String),
    EncodingMismatch,
    EmptyFile,
    Delimited(String),
}

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes, limit } => {
                write!(f, "file too large: {} bytes > {} bytes", bytes, limit)
            }
            Self::Io(err) => write!(f, "file read failed: {}", err),
            Self::EncodingMismatch => write!(f, "encoding mismatch: only UTF-8 / UTF-8 BOM"),
            Self::EmptyFile => write!(f, "source is empty"),
            Self::Delimited(reason) => write!(f, "DelimitedText preview unavailable: {}", reason),
        }
    }
}

impl std::error::Error for IntakeError {}

#[derive(Debug, Clone)]
pub struct FileTranslateRunConfig {
    pub source: TableSourceData,
    pub dict_slot: PathBuf,
    pub column_modes: BTreeMap<usize, ColumnMode>,
    pub ui_lang: String,
    pub server_host: String,
    pub server_port: u16,
    pub chunk_size: usize,
    pub request_timeout_secs: u64,
    pub cancel_flag: Arc<AtomicBool>,
    pub event_tx: Sender<BackendEvent>,
}
