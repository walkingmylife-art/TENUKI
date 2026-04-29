use super::types::{ColumnMode, FileTranslateRunConfig};
use crate::messages::{BackendEvent, LogLevel};
use serde::{Deserialize, Serialize};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

const FILE_TRANSLATE_STOPPED: &str = "__TENUKI_FILE_TRANSLATE_STOPPED__";

pub struct FileTranslateRunOutcome {
    pub title: String,
    pub text: String,
    pub is_error: bool,
}

#[derive(Serialize)]
struct ListPayload {
    texts: Vec<String>,
}

#[derive(Deserialize)]
struct ListResponse {
    texts: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectedMode {
    Translate,
    Original,
}

impl SelectedMode {
    fn label(self) -> &'static str {
        match self {
            Self::Translate => "translate",
            Self::Original => "original",
        }
    }
}

struct SelectedEntry {
    column_index: usize,
    row_index: usize,
    source: String,
    mode: SelectedMode,
}

/// Incremental TXT writer for List-mode translation output.
///
/// Writes entries to a `.partial.txt` file during execution
/// in `source=target` dict.txt format.
/// On `finish()`, renames `.partial.txt` → `.txt`.
/// On drop before `finish()`, the partial file remains as recovery evidence.
struct ListTextOutputWriter {
    final_path: PathBuf,
    partial_path: PathBuf,
    writer: BufWriter<std::fs::File>,
    written_count: usize,
}

impl ListTextOutputWriter {
    fn new(output_dir: &Path, source_file: &Path) -> Result<Self, String> {
        let stem = source_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");

        let final_path = resolve_output_txt_path(output_dir, stem)?;
        let partial_path = final_path.with_extension("partial.txt");

        let file = std::fs::File::create(&partial_path)
            .map_err(|e| format!("partial file create failed: {}", e))?;
        let mut writer = BufWriter::new(file);

        // BOM for dict.txt format compatibility
        writer
            .write_all(&[0xEF, 0xBB, 0xBF])
            .map_err(|e| format!("BOM write failed: {}", e))?;

        Ok(Self {
            final_path,
            partial_path,
            writer,
            written_count: 0,
        })
    }

    fn append_entry(&mut self, source: &str, target: &str) -> Result<(), String> {
        writeln!(self.writer, "{}={}", source, target)
            .map_err(|e| format!("write failed: {}", e))?;
        self.written_count += 1;
        Ok(())
    }

    fn flush_checkpoint(&mut self) -> Result<(), String> {
        self.writer
            .flush()
            .map_err(|e| format!("flush failed: {}", e))
    }

    fn finish(mut self) -> Result<PathBuf, String> {
        self.writer
            .flush()
            .map_err(|e| format!("final flush failed: {}", e))?;
        drop(self.writer);

        std::fs::rename(&self.partial_path, &self.final_path)
            .map_err(|e| format!("rename to final failed: {}", e))?;
        Ok(self.final_path.clone())
    }
}

/// Resolve a non-colliding `.txt` output path in `output_dir` for the given stem.
fn resolve_output_txt_path(output_dir: &Path, stem: &str) -> Result<PathBuf, String> {
    let base = output_dir.join(format!("{}.txt", stem));
    if !base.exists() {
        return Ok(base);
    }
    for n in 1u32..1000 {
        let candidate = output_dir.join(format!("{}_{:03}.txt", stem, n));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!("too many existing files for stem '{}'", stem))
}

pub fn run_file_translate(cfg: FileTranslateRunConfig) -> FileTranslateRunOutcome {
    let title = cfg.source.file.display().to_string();
    let output_dir = cfg.dict_slot.display().to_string();
    let source_file = cfg.source.file.display().to_string();
    match run_file_translate_inner(cfg) {
        Ok(text) => FileTranslateRunOutcome {
            title,
            text,
            is_error: false,
        },
        Err(err) if err == FILE_TRANSLATE_STOPPED => FileTranslateRunOutcome {
            title,
            text: format!(
                "Stopped\npartial file may remain\noutput directory: {}\nsource file: {}",
                output_dir, source_file
            ),
            is_error: false,
        },
        Err(err) => FileTranslateRunOutcome {
            title,
            text: err,
            is_error: true,
        },
    }
}

/// Execute a List-mode translation run.
///
/// Writes translation results incrementally to `{source_stem}.partial.txt`
/// in the output directory (`cfg.dict_slot`).
/// On successful completion, renames `.partial.txt` → `.txt`.
/// On failure or stop, the partial file remains.
///
/// TXT format is `source=target` (UTF-8 BOM), compatible with dict.txt
/// for downstream BIN creation.
fn run_file_translate_inner(cfg: FileTranslateRunConfig) -> Result<String, String> {
    ensure_not_cancelled(&cfg)?;
    std::fs::create_dir_all(&cfg.dict_slot)
        .map_err(|e| format!("output directory create failed: {}", e))?;

    let column_count = cfg.source.column_labels.len();
    let selected_columns = cfg
        .column_modes
        .iter()
        .filter_map(|(index, mode)| {
            if *index >= column_count {
                return None;
            }
            match mode {
                ColumnMode::Translate => Some((*index, SelectedMode::Translate)),
                ColumnMode::Original => Some((*index, SelectedMode::Original)),
                ColumnMode::None => None,
            }
        })
        .collect::<Vec<_>>();

    if selected_columns.is_empty() {
        return Err("no Translate or Original columns selected within column range".to_string());
    }

    send_log(
        &cfg,
        LogLevel::Info,
        format!(
            "[run] {} rows / {} selected columns",
            cfg.source.total_rows,
            selected_columns.len()
        ),
    );

    let mut selected_entries = Vec::new();
    let mut translated_count = 0usize;
    let mut original_count = 0usize;

    for (column_index, mode) in &selected_columns {
        for (row_index, row) in cfg.source.rows.iter().enumerate() {
            let Some(source) = row.get(*column_index).cloned() else {
                continue;
            };
            if source.trim().is_empty() {
                continue;
            }

            match mode {
                SelectedMode::Translate => translated_count += 1,
                SelectedMode::Original => original_count += 1,
            }
            selected_entries.push(SelectedEntry {
                column_index: *column_index,
                row_index,
                source,
                mode: *mode,
            });
        }
    }

    if selected_entries.is_empty() {
        return Err("selected columns contain no runnable cells".to_string());
    }

    let total_entries = selected_entries.len();
    send_progress(&cfg, 0, total_entries);

    let mut writer = ListTextOutputWriter::new(&cfg.dict_slot, &cfg.source.file)?;

    let translate_indexes = selected_entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (entry.mode == SelectedMode::Translate).then_some(index))
        .collect::<Vec<_>>();

    let mut done_count = 0usize;

    // Write Original entries immediately to partial TXT
    for (_index, entry) in selected_entries.iter().enumerate() {
        if entry.mode != SelectedMode::Original {
            continue;
        }
        ensure_not_cancelled(&cfg)?;
        writer.append_entry(&entry.source, &entry.source)?;
        done_count += 1;
        send_progress(&cfg, done_count, total_entries);
        send_log(
            &cfg,
            LogLevel::Info,
            format!(
                "[original][{}/{}] {} => {}",
                done_count, total_entries, entry.source, entry.source
            ),
        );
    }
    writer.flush_checkpoint()?;

    // Translate chunks
    if !translate_indexes.is_empty() {
        ensure_not_cancelled(&cfg)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(cfg.request_timeout_secs.max(1)))
            .build()
            .map_err(|e| format!("HTTP client build failed: {}", e))?;

        wait_for_translation_server(cfg.server_host.as_str(), cfg.server_port, &client, &cfg)?;

        let server_url = format!(
            "http://{}:{}/list",
            connect_host(cfg.server_host.as_str()),
            cfg.server_port
        );
        let chunk_size = cfg.chunk_size.max(1);

        for chunk in translate_indexes.chunks(chunk_size) {
            ensure_not_cancelled(&cfg)?;
            let body = ListPayload {
                texts: chunk
                    .iter()
                    .map(|index| selected_entries[*index].source.clone())
                    .collect(),
            };
            let response = client
                .post(&server_url)
                .json(&body)
                .send()
                .map_err(|e| format!("POST /list failed: {}", e))?;
            if !response.status().is_success() {
                return Err(format!("POST /list returned {}", response.status()));
            }
            let response_body = response
                .json::<ListResponse>()
                .map_err(|e| format!("response parse failed: {}", e))?;
            if response_body.texts.len() != chunk.len() {
                return Err(format!(
                    "response count mismatch: sent {}, got {}",
                    chunk.len(),
                    response_body.texts.len()
                ));
            }

            for (index, target) in chunk.iter().zip(response_body.texts.into_iter()) {
                let source = &selected_entries[*index].source;
                writer.append_entry(source, &target)?;
                done_count += 1;
                send_progress(&cfg, done_count, total_entries);
                send_log(
                    &cfg,
                    LogLevel::Info,
                    format!("[{}/{}] {}", done_count, total_entries, source),
                );
                send_log(&cfg, LogLevel::Info, format!("=> {}", target));
            }
            // Flush per chunk so partial is recoverable on crash
            writer.flush_checkpoint()?;
        }
    }

    // Success: rename partial → final
    let final_path = writer.finish()?;
    let written_count = total_entries; // All entries were written

    let mut lines = vec![
        format!("file: {}", cfg.source.file.display()),
        format!("output directory: {}", cfg.dict_slot.display()),
        format!("output: {}", final_path.display()),
        format!(
            "selected columns: {}",
            labels_from_selected_columns(&cfg.source.column_labels, &selected_columns)
        ),
        format!("written entries: {}", written_count),
        "duplicate sources: preserved".to_string(),
        format!("translated count: {}", translated_count),
        format!("original count: {}", original_count),
    ];

    send_log(
        &cfg,
        LogLevel::Success,
        format!("[done] {} entries", written_count),
    );
    send_log(
        &cfg,
        LogLevel::Info,
        format!("[output] {}", final_path.display()),
    );

    lines.push(String::new());
    lines.push("result preview:".to_string());
    for (_index, entry) in selected_entries.iter().enumerate().take(12) {
        let target = match entry.mode {
            SelectedMode::Translate => "…",
            SelectedMode::Original => &entry.source,
        };
        lines.push(format!(
            "col {} row {} [{}] {} => {}",
            entry.column_index + 1,
            entry.row_index + 1,
            entry.mode.label(),
            entry.source,
            target
        ));
    }

    Ok(lines.join("\n"))
}

fn ensure_not_cancelled(cfg: &FileTranslateRunConfig) -> Result<(), String> {
    if cfg.cancel_flag.load(Ordering::Relaxed) {
        send_log(cfg, LogLevel::Info, "[stopped] run cancelled".to_string());
        Err(FILE_TRANSLATE_STOPPED.to_string())
    } else {
        Ok(())
    }
}

fn labels_from_selected_columns(headers: &[String], columns: &[(usize, SelectedMode)]) -> String {
    if columns.is_empty() {
        return "-".to_string();
    }
    columns
        .iter()
        .map(|(index, mode)| {
            let label = headers
                .get(*index)
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            let mode_label = match mode {
                SelectedMode::Translate => "translate",
                SelectedMode::Original => "original",
            };
            format!("{}: {} [{}]", index + 1, label, mode_label)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn connect_host(host: &str) -> &str {
    match host.trim() {
        "" | "0.0.0.0" | "localhost" => "127.0.0.1",
        other => other,
    }
}

fn wait_for_translation_server(
    host: &str,
    port: u16,
    client: &reqwest::blocking::Client,
    cfg: &FileTranslateRunConfig,
) -> Result<(), String> {
    let url = format!("http://{}:{}/health", connect_host(host), port);
    for _ in 0..40 {
        ensure_not_cancelled(cfg)?;
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("translation server is not ready".to_string())
}

fn send_progress(cfg: &FileTranslateRunConfig, done: usize, total: usize) {
    let _ = cfg
        .event_tx
        .send(BackendEvent::FileTranslateProgress { done, total });
}

fn send_log(cfg: &FileTranslateRunConfig, level: LogLevel, line: String) {
    let _ = cfg
        .event_tx
        .send(BackendEvent::FileTranslateLog { line, level });
}

#[cfg(test)]
mod tests {
    use super::{run_file_translate, ListTextOutputWriter};
    use crate::file_translate::types::{
        ColumnMode, FileTranslateRunConfig, HeaderMode, SourceEncoding, SourceKind, TableSourceData,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tenuki_runner_test_{}_{}", name, stamp))
    }

    #[test]
    fn text_writer_creates_partial_with_bom_and_writes_source_equals_target() {
        let dir = unique_path("tx_dir");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.csv");
        std::fs::write(&source, b"dummy\n").unwrap();

        let mut w = ListTextOutputWriter::new(&dir, &source).unwrap();
        let partial = w.partial_path.clone();
        assert!(partial.extension().unwrap() == "txt");
        assert!(partial
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("partial"));

        w.append_entry("Attack", "攻撃").unwrap();
        w.append_entry("Defense", "防御").unwrap();
        w.flush_checkpoint().unwrap();

        let content = std::fs::read(&partial).unwrap();
        // BOM present
        assert_eq!(&content[..3], &[0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8_lossy(&content[3..]);
        assert!(text.contains("Attack=攻撃"));
        assert!(text.contains("Defense=防御"));

        let final_txt = w.finish().unwrap();
        assert!(final_txt.exists());
        assert!(!partial.exists(), "partial should be gone after finish");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn text_writer_resolves_collision_when_txt_exists() {
        let dir = unique_path("tx_coll");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.csv");
        std::fs::write(&source, b"dummy\n").unwrap();

        // Pre-create final TXT
        std::fs::write(dir.join("source.txt"), b"existing\n").unwrap();

        let w = ListTextOutputWriter::new(&dir, &source).unwrap();
        assert!(w
            .final_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("source_"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn text_writer_original_mode_writes_source_equals_source() {
        let dir = unique_path("tx_orig");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.csv");
        std::fs::write(&source, b"dummy\n").unwrap();

        let mut w = ListTextOutputWriter::new(&dir, &source).unwrap();
        w.append_entry("keep", "keep").unwrap();
        w.flush_checkpoint().unwrap();

        let partial_path = w.partial_path.clone();
        let content = std::fs::read_to_string(&partial_path).unwrap();
        assert!(content.contains("keep=keep"));

        w.finish().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn run_errors_when_selected_columns_have_no_non_empty_cells() {
        let path = unique_path("empty_cells.csv");
        let slot = unique_path("empty_slot");
        let (tx, _rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let source = TableSourceData {
            file: path.clone(),
            file_size: 16,
            source_kind: SourceKind::DelimitedText,
            encoding: SourceEncoding::Utf8,
            header_mode: HeaderMode::Present,
            suggested_header: true,
            header_row: Some(vec!["id".to_string(), "text".to_string()]),
            column_labels: vec!["id".to_string(), "text".to_string()],
            rows: vec![vec!["".to_string(), "".to_string()]],
            total_rows: 1,
            delimiter: Some(','),
            json_shape: None,
            json_diagnostic: None,
        };
        let mut column_modes = BTreeMap::new();
        column_modes.insert(1, ColumnMode::Translate);
        let cfg = FileTranslateRunConfig {
            source,
            dict_slot: slot.clone(),
            column_modes,
            ui_lang: "en".to_string(),
            server_host: "127.0.0.1".to_string(),
            server_port: 1,
            chunk_size: 8,
            request_timeout_secs: 1,
            cancel_flag: cancel,
            event_tx: tx,
        };

        let outcome = run_file_translate(cfg);

        assert!(outcome.is_error);
        assert!(outcome.text.contains("no runnable cells"));
        let _ = std::fs::remove_dir_all(slot);
    }

    #[test]
    fn run_does_not_write_output_when_cancelled() {
        let path = unique_path("cancel.csv");
        let slot = unique_path("cancel_slot");
        let (tx, _rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(true));
        let source = TableSourceData {
            file: path.clone(),
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
        let cfg = FileTranslateRunConfig {
            source,
            dict_slot: slot.clone(),
            column_modes,
            ui_lang: "en".to_string(),
            server_host: "127.0.0.1".to_string(),
            server_port: 1,
            chunk_size: 8,
            request_timeout_secs: 1,
            cancel_flag: cancel,
            event_tx: tx,
        };

        let outcome = run_file_translate(cfg);

        assert!(!outcome.is_error);
        assert!(outcome.text.contains("Stopped"));
        assert!(outcome.text.contains("partial file may remain"));
        // Cancelled run: partial TXT remains, final TXT must not exist.
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let final_txt = slot.join(format!("{}.txt", stem));
        assert!(
            !final_txt.exists(),
            "cancelled run must not create final TXT"
        );
        let _ = std::fs::remove_dir_all(slot);
    }

    #[test]
    fn runner_errors_when_only_out_of_range_columns_are_selected() {
        let path = unique_path("range_err.csv");
        let slot = unique_path("range_err_slot");
        let (tx, _rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let source = TableSourceData {
            file: path.clone(),
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
        // both indices out of 2-column range
        column_modes.insert(99, ColumnMode::Translate);
        let cfg = FileTranslateRunConfig {
            source,
            dict_slot: slot.clone(),
            column_modes,
            ui_lang: "en".to_string(),
            server_host: "127.0.0.1".to_string(),
            server_port: 1,
            chunk_size: 8,
            request_timeout_secs: 1,
            cancel_flag: cancel,
            event_tx: tx,
        };

        let outcome = run_file_translate(cfg);

        assert!(outcome.is_error);
        assert!(outcome.text.contains("no Translate or Original columns"));
        let _ = std::fs::remove_dir_all(slot);
    }
}
