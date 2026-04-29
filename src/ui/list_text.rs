use crate::file_translate::state::{FileTranslateRunReadiness, RunBlocker};
use crate::file_translate::types::{
    ColumnMode, HeaderMode, JsonTableShape, SourceEncoding, SourceKind,
};

#[derive(Debug, Clone, Copy)]
pub enum ListText {
    Run,
    Stop,
    FileTranslateTitle,
    Scanning,
    Started,
    Stopping,
    Failed,
    LoadingPreview,
    NoListActivity,
    SelectAssetSource,
    ScanningSources,
    NoAssetSources,
    Lines,
    Rows,
    Cols,
    Delimiter,
    Showing,
    NoHeaderRow,
    HasHeaderRow,
    OutputDirectory,
    RunUsesListOutputFolder,
    Running,
    RunReady,
    NoSourceSelected,
    PreviewUnavailable,
    ReadyToRun,
    NeedSelectSource,
    NeedSourceMissing,
    NeedPreviewLoading,
    NeedTableSource,
    NeedHeaderConfirmation,
    NeedColumns,
    NeedTargetLang,
    File,
    Kind,
    Encoding,
    Size,
    Diagnostic,
    Header,
    HeaderSuggestion,
    OutputDirectoryAction,
    SelectedColumns,
    Sample,
    Preview,
    Source,
    Sources,
    JsonTable,
    JsonDiagnostic,
    Unavailable,
    UseListOutputDirectory,
    ListOutputDirectoryWillBeUsed,
    UseCommittedOutputDirectory,
    Need,
    Root,
    Delimited,
    Json,
    Markup,
    PlainLines,
    UnsupportedBinary,
    UnknownText,
    BinaryEncoding,
    UnknownEncoding,
    JsonArrayOfObjects,
    JsonArrayOfArrays,
    SelectSourceToPreview,
}

pub fn text(lang: &str, key: ListText) -> &'static str {
    match lang {
        "ja" => ja(key),
        "zh-CN" => zh_cn(key),
        _ => en(key),
    }
}

pub fn completed(lang: &str, count: usize) -> String {
    match lang {
        "ja" => format!("{} 件完了しました", count),
        "zh-CN" => format!("已完成 {} 项", count),
        _ => format!("{} completed", count),
    }
}

pub fn stopped(lang: &str, count: usize) -> String {
    match lang {
        "ja" => format!("{} 件で停止しました", count),
        "zh-CN" => format!("已停止于 {} 项", count),
        _ => format!("Stopped at {}", count),
    }
}

pub fn scan_done(lang: &str, count: usize) -> String {
    match lang {
        "ja" => format!("スキャン完了: {} 件", count),
        "zh-CN" => format!("扫描完成: {} 个文件", count),
        _ => format!("scan done: {} files", count),
    }
}

pub fn showing_text(lang: &str, shown: usize, total: usize) -> String {
    format!("{} {} / {}", text(lang, ListText::Showing), shown, total)
}

pub fn showing_first_rows(lang: &str, rows: usize) -> String {
    match lang {
        "ja" => format!("先頭 {} 行を表示", rows),
        "zh-CN" => format!("显示前 {} 行", rows),
        _ => format!("showing first {} rows", rows),
    }
}

pub fn blocker(lang: &str, blocker: &RunBlocker) -> String {
    match blocker {
        RunBlocker::NoSourceSelected => text(lang, ListText::NeedSelectSource).to_string(),
        RunBlocker::SourceMissing(path) => {
            format!(
                "{}: {}",
                text(lang, ListText::NeedSourceMissing),
                path.display()
            )
        }
        RunBlocker::PreviewLoading => text(lang, ListText::NeedPreviewLoading).to_string(),
        RunBlocker::PreviewUnavailable(reason) => {
            format!("{}: {}", text(lang, ListText::PreviewUnavailable), reason)
        }
        RunBlocker::TableSourceRequired => text(lang, ListText::NeedTableSource).to_string(),
        RunBlocker::HeaderConfirmationRequired => {
            text(lang, ListText::NeedHeaderConfirmation).to_string()
        }
        RunBlocker::NoColumnsSelected => text(lang, ListText::NeedColumns).to_string(),
        RunBlocker::DictSlotUnavailable(reason) => {
            if reason == "Target language is required before creating a List output directory" {
                text(lang, ListText::NeedTargetLang).to_string()
            } else {
                reason.clone()
            }
        }
    }
}

pub fn readiness(lang: &str, readiness: &FileTranslateRunReadiness) -> String {
    readiness
        .blockers
        .first()
        .map(|item| blocker(lang, item))
        .unwrap_or_else(|| text(lang, ListText::ReadyToRun).to_string())
}

pub fn field(lang: &str, key: ListText, value: impl std::fmt::Display) -> String {
    format!("{}: {}", text(lang, key), value)
}

pub fn size_bytes(lang: &str, bytes: u64) -> String {
    match lang {
        "ja" => format!("{} バイト", bytes),
        "zh-CN" => format!("{} 字节", bytes),
        _ => format!("{} bytes", bytes),
    }
}

pub fn source_kind_label(
    lang: &str,
    kind: SourceKind,
    json_shape: Option<JsonTableShape>,
) -> String {
    let label = match kind {
        SourceKind::DelimitedText => text(lang, ListText::Delimited).to_string(),
        SourceKind::JsonText => text(lang, ListText::Json).to_string(),
        SourceKind::PlainLines => text(lang, ListText::PlainLines).to_string(),
        SourceKind::MarkupText => text(lang, ListText::Markup).to_string(),
        SourceKind::UnsupportedBinary => text(lang, ListText::UnsupportedBinary).to_string(),
        SourceKind::UnknownText => text(lang, ListText::UnknownText).to_string(),
    };

    if kind == SourceKind::JsonText {
        json_shape
            .map(|shape| format!("{} ({})", label, json_table_shape_label(lang, shape)))
            .unwrap_or(label)
    } else {
        label
    }
}

pub fn encoding_label(lang: &str, encoding: SourceEncoding) -> &'static str {
    match encoding {
        SourceEncoding::Utf8 => "UTF-8",
        SourceEncoding::Utf8Bom => "UTF-8 BOM",
        SourceEncoding::Binary => text(lang, ListText::BinaryEncoding),
        SourceEncoding::Unknown => text(lang, ListText::UnknownEncoding),
    }
}

pub fn json_table_shape_label(lang: &str, shape: JsonTableShape) -> &'static str {
    match shape {
        JsonTableShape::ArrayOfObjects => text(lang, ListText::JsonArrayOfObjects),
        JsonTableShape::ArrayOfArrays => text(lang, ListText::JsonArrayOfArrays),
    }
}

pub fn source_hover(
    lang: &str,
    encoding: SourceEncoding,
    file_size: u64,
    diagnostic: &str,
) -> String {
    format!(
        "{} / {}\n{}",
        encoding_label(lang, encoding),
        size_bytes(lang, file_size),
        diagnostic
    )
}

pub fn encoding_size_line(lang: &str, encoding: SourceEncoding, file_size: u64) -> String {
    format!(
        "{} / {}",
        encoding_label(lang, encoding),
        size_bytes(lang, file_size)
    )
}

pub fn text_preview_stats(
    lang: &str,
    encoding: SourceEncoding,
    line_count: usize,
    file_size: u64,
    showing: usize,
    total: usize,
) -> String {
    format!(
        "{} / {} {} / {} / {}",
        encoding_label(lang, encoding),
        line_count,
        text(lang, ListText::Lines),
        size_bytes(lang, file_size),
        showing_text(lang, showing, total)
    )
}

pub fn table_preview_stats(
    lang: &str,
    total_rows: usize,
    column_count: usize,
    delimiter: Option<char>,
    showing_rows: usize,
) -> String {
    let delimiter_label = delimiter
        .map(|delimiter| delimiter.to_string())
        .unwrap_or_else(|| text(lang, ListText::Unavailable).to_string());
    format!(
        "{} {} | {} {} | {} {} | {}",
        text(lang, ListText::Rows),
        total_rows,
        text(lang, ListText::Cols),
        column_count,
        text(lang, ListText::Delimiter),
        delimiter_label,
        showing_first_rows(lang, showing_rows)
    )
}

pub fn header_mode(lang: &str, mode: HeaderMode) -> &'static str {
    match mode {
        HeaderMode::Unknown => match lang {
            "ja" => "未確認",
            "zh-CN" => "未确认",
            _ => "unconfirmed",
        },
        HeaderMode::Present => text(lang, ListText::HasHeaderRow),
        HeaderMode::Absent => text(lang, ListText::NoHeaderRow),
    }
}

pub fn header_suggestion(lang: &str, suggested_header: bool) -> String {
    let suggestion = if suggested_header {
        text(lang, ListText::HasHeaderRow)
    } else {
        text(lang, ListText::NoHeaderRow)
    };
    match lang {
        "ja" => format!("推定: {}", suggestion),
        "zh-CN" => format!("推测: {}", suggestion),
        _ => format!("suggested: {}", suggestion),
    }
}

pub fn column_mode(lang: &str, mode: ColumnMode) -> &'static str {
    match mode {
        ColumnMode::Translate => "Translate",
        ColumnMode::Original => "Original",
        ColumnMode::None => match lang {
            "ja" => "なし",
            "zh-CN" => "无",
            _ => "none",
        },
    }
}

fn en(key: ListText) -> &'static str {
    match key {
        ListText::Run => "Run",
        ListText::Stop => "Stop",
        ListText::FileTranslateTitle => "File Translate",
        ListText::Scanning => "Scanning",
        ListText::Started => "Started",
        ListText::Stopping => "Stopping",
        ListText::Failed => "Failed",
        ListText::LoadingPreview => "Loading preview...",
        ListText::NoListActivity => "No List activity yet",
        ListText::SelectAssetSource => "Select an asset source",
        ListText::ScanningSources => "Scanning sources...",
        ListText::NoAssetSources => "No asset sources",
        ListText::Lines => "lines",
        ListText::Rows => "Rows",
        ListText::Cols => "Cols",
        ListText::Delimiter => "Delimiter",
        ListText::Showing => "showing",
        ListText::NoHeaderRow => "First row is data",
        ListText::HasHeaderRow => "First row is header",
        ListText::OutputDirectory => "Output directory",
        ListText::RunUsesListOutputFolder => "Run will use List output directory under",
        ListText::Running => "Running...",
        ListText::RunReady => "Run ready",
        ListText::NoSourceSelected => "No source selected",
        ListText::PreviewUnavailable => "Preview unavailable",
        ListText::ReadyToRun => "Ready to run",
        ListText::NeedSelectSource => "Select a source file",
        ListText::NeedSourceMissing => "Source file is missing",
        ListText::NeedPreviewLoading => "Loading preview",
        ListText::NeedTableSource => "Run is available only for table-capable sources",
        ListText::NeedHeaderConfirmation => "Choose whether the first row is a header",
        ListText::NeedColumns => "Select at least one Translate or Original column",
        ListText::NeedTargetLang => {
            "Target language is required before creating a List output directory"
        }
        ListText::File => "File",
        ListText::Kind => "Kind",
        ListText::Encoding => "Encoding",
        ListText::Size => "Size",
        ListText::Diagnostic => "Diagnostic",
        ListText::Header => "Header",
        ListText::HeaderSuggestion => "Header suggestion",
        ListText::OutputDirectoryAction => "Output directory action",
        ListText::SelectedColumns => "Selected columns",
        ListText::Sample => "Sample",
        ListText::Preview => "Preview",
        ListText::Source => "Source",
        ListText::Sources => "Sources",
        ListText::JsonTable => "JSON table",
        ListText::JsonDiagnostic => "JSON diagnostic",
        ListText::Unavailable => "Unavailable",
        ListText::UseListOutputDirectory => "Use List output directory",
        ListText::ListOutputDirectoryWillBeUsed => "List output directory will be used",
        ListText::UseCommittedOutputDirectory => "Use committed output directory",
        ListText::Need => "Need",
        ListText::Root => "Root",
        ListText::Delimited => "Delimited",
        ListText::Json => "JSON",
        ListText::Markup => "Markup",
        ListText::PlainLines => "Plain lines",
        ListText::UnsupportedBinary => "Unsupported binary",
        ListText::UnknownText => "Unknown text",
        ListText::BinaryEncoding => "binary",
        ListText::UnknownEncoding => "unknown",
        ListText::JsonArrayOfObjects => "array<object>",
        ListText::JsonArrayOfArrays => "array<array>",
        ListText::SelectSourceToPreview => "Select a source to preview",
    }
}

fn ja(key: ListText) -> &'static str {
    match key {
        ListText::Run => "実行",
        ListText::Stop => "停止",
        ListText::FileTranslateTitle => "ファイル翻訳",
        ListText::Scanning => "スキャン中",
        ListText::Started => "開始しました",
        ListText::Stopping => "停止中",
        ListText::Failed => "失敗しました",
        ListText::LoadingPreview => "プレビュー読み込み中...",
        ListText::NoListActivity => "List のログはまだありません",
        ListText::SelectAssetSource => "ソースを選択してください",
        ListText::ScanningSources => "ソースをスキャン中...",
        ListText::NoAssetSources => "ソースがありません",
        ListText::Lines => "行",
        ListText::Rows => "行",
        ListText::Cols => "列",
        ListText::Delimiter => "区切り",
        ListText::Showing => "表示中",
        ListText::NoHeaderRow => "先頭行もデータ",
        ListText::HasHeaderRow => "先頭行はヘッダー",
        ListText::OutputDirectory => "出力ディレクトリ",
        ListText::RunUsesListOutputFolder => "実行時に List 出力ディレクトリを使用します",
        ListText::Running => "実行中...",
        ListText::RunReady => "実行できます",
        ListText::NoSourceSelected => "ソース未選択",
        ListText::PreviewUnavailable => "プレビューできません",
        ListText::ReadyToRun => "実行できます",
        ListText::NeedSelectSource => "ソースファイルを選択してください",
        ListText::NeedSourceMissing => "ソースファイルが見つかりません",
        ListText::NeedPreviewLoading => "プレビュー読み込み中",
        ListText::NeedTableSource => "実行できるのは表形式ソースだけです",
        ListText::NeedHeaderConfirmation => "先頭行をヘッダーとして扱うか選択してください",
        ListText::NeedColumns => "Translate または Original の列を1つ以上選択してください",
        ListText::NeedTargetLang => "実行用スロットを作成するには翻訳先言語が必要です",
        ListText::File => "ファイル",
        ListText::Kind => "種類",
        ListText::Encoding => "文字コード",
        ListText::Size => "サイズ",
        ListText::Diagnostic => "診断",
        ListText::Header => "ヘッダー",
        ListText::HeaderSuggestion => "ヘッダー推定",
        ListText::OutputDirectoryAction => "出力ディレクトリ処理",
        ListText::SelectedColumns => "選択列",
        ListText::Sample => "サンプル",
        ListText::Preview => "プレビュー",
        ListText::Source => "ソース",
        ListText::Sources => "ソース数",
        ListText::JsonTable => "JSON 表",
        ListText::JsonDiagnostic => "JSON 診断",
        ListText::Unavailable => "利用不可",
        ListText::UseListOutputDirectory => "List 出力ディレクトリを使用",
        ListText::ListOutputDirectoryWillBeUsed => "List 出力ディレクトリを使用予定",
        ListText::UseCommittedOutputDirectory => "確定済み出力ディレクトリを使用",
        ListText::Need => "要確認",
        ListText::Root => "ルート",
        ListText::Delimited => "区切りテキスト",
        ListText::Json => "JSON",
        ListText::Markup => "マークアップ",
        ListText::PlainLines => "行テキスト",
        ListText::UnsupportedBinary => "非対応バイナリ",
        ListText::UnknownText => "不明なテキスト",
        ListText::BinaryEncoding => "バイナリ",
        ListText::UnknownEncoding => "不明",
        ListText::JsonArrayOfObjects => "object 配列",
        ListText::JsonArrayOfArrays => "array 配列",
        ListText::SelectSourceToPreview => "プレビューするソースを選択してください",
    }
}

fn zh_cn(key: ListText) -> &'static str {
    match key {
        ListText::Run => "执行",
        ListText::Stop => "停止",
        ListText::FileTranslateTitle => "文件翻译",
        ListText::Scanning => "扫描中",
        ListText::Started => "已开始",
        ListText::Stopping => "停止中",
        ListText::Failed => "失败",
        ListText::LoadingPreview => "正在加载预览...",
        ListText::NoListActivity => "还没有 List 日志",
        ListText::SelectAssetSource => "请选择源文件",
        ListText::ScanningSources => "正在扫描源文件...",
        ListText::NoAssetSources => "没有源文件",
        ListText::Lines => "行",
        ListText::Rows => "行",
        ListText::Cols => "列",
        ListText::Delimiter => "分隔符",
        ListText::Showing => "显示",
        ListText::NoHeaderRow => "首行也是数据",
        ListText::HasHeaderRow => "首行是表头",
        ListText::OutputDirectory => "输出目录",
        ListText::RunUsesListOutputFolder => "执行时会使用 List 输出目录",
        ListText::Running => "执行中...",
        ListText::RunReady => "可以执行",
        ListText::NoSourceSelected => "未选择源文件",
        ListText::PreviewUnavailable => "无法预览",
        ListText::ReadyToRun => "可以执行",
        ListText::NeedSelectSource => "请选择源文件",
        ListText::NeedSourceMissing => "源文件不存在",
        ListText::NeedPreviewLoading => "正在加载预览",
        ListText::NeedTableSource => "只能执行表格式源文件",
        ListText::NeedHeaderConfirmation => "请选择首行是否为表头",
        ListText::NeedColumns => "请至少选择一个 Translate 或 Original 列",
        ListText::NeedTargetLang => "创建本次执行用槽位前需要目标语言",
        ListText::File => "文件",
        ListText::Kind => "类型",
        ListText::Encoding => "编码",
        ListText::Size => "大小",
        ListText::Diagnostic => "诊断",
        ListText::Header => "表头",
        ListText::HeaderSuggestion => "表头推测",
        ListText::OutputDirectoryAction => "输出目录处理",
        ListText::SelectedColumns => "已选列",
        ListText::Sample => "示例",
        ListText::Preview => "预览",
        ListText::Source => "源文件",
        ListText::Sources => "源文件数",
        ListText::JsonTable => "JSON 表",
        ListText::JsonDiagnostic => "JSON 诊断",
        ListText::Unavailable => "不可用",
        ListText::UseListOutputDirectory => "使用 List 输出目录",
        ListText::ListOutputDirectoryWillBeUsed => "将使用 List 输出目录",
        ListText::UseCommittedOutputDirectory => "使用已确定的输出目录",
        ListText::Need => "需要处理",
        ListText::Root => "根目录",
        ListText::Delimited => "分隔文本",
        ListText::Json => "JSON",
        ListText::Markup => "标记文本",
        ListText::PlainLines => "纯文本行",
        ListText::UnsupportedBinary => "不支持的二进制",
        ListText::UnknownText => "未知文本",
        ListText::BinaryEncoding => "二进制",
        ListText::UnknownEncoding => "未知",
        ListText::JsonArrayOfObjects => "object 数组",
        ListText::JsonArrayOfArrays => "array 数组",
        ListText::SelectSourceToPreview => "请选择要预览的源文件",
    }
}
