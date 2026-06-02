//! Translation HTTP server module (axum + tokio)

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::sync::oneshot;
use tokio::sync::RwLock;

use crate::backend::analysis::{
    self, CompletedAnalysisPayload, CompletedTranslationRecord, SharedInputReplayState,
};
use crate::backend::dictionary::{Dictionary, SplitResult};
use crate::backend::logger::{LogEvent as PersistentLogEvent, LOG_TX};
use crate::backend::translator::{
    self, LogEvent, NewEntriesCache, NewTranslationEntry, PersistEntry, TranslationCache,
    TranslationResult, TranslationSettings, TranslationStats,
};
use crate::messages::{BackendEvent, LogLevel, LogSource};

const MAX_LIST_ITEMS: usize = 1024;
const MAX_LIST_TOTAL_BYTES: usize = 512 * 1024;
// `/translate` is a single normal translation request, not batch intake.
// Keep this below /list's total limit so oversized POST bodies are rejected
// before raw request logging or parsing.
const MAX_TRANSLATE_BODY_BYTES: usize = 96 * 1024;
const MAX_TRANSLATE_TEXT_BYTES: usize = 64 * 1024;
const TCP_MAX_PAYLOAD_BYTES: usize = 64 * 1024;

// ============================================================
// Request types
// ============================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct TranslateRequest {
    pub text: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListRequest {
    pub texts: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListResponse {
    pub texts: Vec<String>,
}

// ============================================================
// Application state
// ============================================================

#[derive(Clone)]
pub struct AppState {
    pub dictionary: Arc<RwLock<Dictionary>>,
    pub src_lang: Arc<RwLock<String>>,
    pub tgt_lang: Arc<RwLock<String>>,
    pub custom_lang_name: Arc<RwLock<String>>,
    pub prompt_template: String,
    pub background_text: String,
    pub translation_settings: TranslationSettings,
    pub llm_client: Arc<dyn translator::LlmClient>,
    pub event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
    // Session cache of translated results.
    // New dictionary entries waiting for shutdown flush.
    // Replay state used by input analysis.
    // Number of LLM slots available to batch translation.
    pub t_cache: Arc<TranslationCache>,
    pub n_cache: Arc<NewEntriesCache>,
    pub input_replay: SharedInputReplayState,
    pub llm_slots: usize,
}

impl AppState {
    pub async fn current_prefix(&self) -> String {
        let tgt = self.tgt_lang.read().await;
        if self.background_text.trim().is_empty() {
            return translator::fallback_prefix(&tgt);
        }
        let src = self.src_lang.read().await;
        let custom_name = self.custom_lang_name.read().await;
        translator::build_lang_prefix(
            &src,
            &tgt,
            &custom_name,
            &self.prompt_template,
            &self.background_text,
        )
    }

    fn emit_persistent_log(&self, message: String, level: LogLevel) {
        let timestamp = crate::messages::current_timestamp();

        let _ = self.event_tx.try_send(BackendEvent::Log(
            LogSource::Tenuki,
            message.clone(),
            level,
            timestamp.clone(),
        ));

        let _ = LOG_TX.try_send(PersistentLogEvent {
            timestamp,
            level: format!("{:?}", level),
            msg: message,
        });
    }

    fn emit_log(&self, event: &LogEvent) {
        let entry = match event {
            LogEvent::DictHit {
                elapsed_secs,
                original,
                translated,
            } => Some((
                format!(
                    "[TENUKI] ({:.2}s) {} -> {}",
                    elapsed_secs, original, translated
                ),
                LogLevel::Success,
            )),
            LogEvent::Error { message } => Some((format!("Error {}", message), LogLevel::Error)),
            LogEvent::Trace { message } if crate::backend::logger::debug_logs_enabled() => {
                Some((format!("[TRACE] {}", message), LogLevel::Info))
            }
            LogEvent::PreModelCall { .. }
            | LogEvent::ModelResult { .. }
            | LogEvent::Trace { .. } => None,
        };

        if let Some((msg, level)) = entry {
            self.emit_persistent_log(msg, level);
        }
    }

    fn emit_stats(&self, stats: &TranslationStats) {
        if stats.dict_hits > 0 || stats.model_calls > 0 {
            let _ = self.event_tx.try_send(BackendEvent::StatisticsUpdate(
                stats.dict_hits,
                stats.model_calls,
            ));
        }
    }

    fn emit_diagnostic(&self, message: String) {
        if crate::backend::logger::debug_logs_enabled() {
            self.emit_persistent_log(format!("[DIAG] {}", message), LogLevel::Info);
        }
    }

    fn emit_observation(&self, message: String) {
        if crate::backend::logger::debug_logs_enabled() {
            crate::backend::logger::write_observation(message.clone());
            self.emit_persistent_log(format!("[OBSERVE] {}", message), LogLevel::Info);
        }
    }

    fn emit_request_log(&self, message: String) {
        if crate::backend::logger::debug_logs_enabled() {
            crate::backend::logger::write_request(message.clone());
            self.emit_persistent_log(format!("[REQUEST] {}", message), LogLevel::Info);
        }
    }
}

fn emit_word_log_pairs(event_tx: &tokio::sync::mpsc::Sender<BackendEvent>, logs: &[LogEvent]) {
    for log in logs {
        match log {
            LogEvent::ModelResult {
                source,
                translated,
                elapsed_secs,
                ..
            } => {
                let _ = event_tx.try_send(BackendEvent::DictionaryLogEntry(
                    crate::messages::current_timestamp(),
                    format!("[XUnity] {}", source),
                    format!("[Model] ({:.2}s) {}", elapsed_secs, translated),
                ));
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
struct ItemDiagnostics {
    raw_text: String,
    extracted_text: String,
    visible_text: String,
    input_preview: String,
    dict_hits: usize,
    model_calls: usize,
    model_inputs: Vec<String>,
}

#[derive(Default)]
struct BatchTranslationOutput {
    texts: Vec<String>,
    new_entries: Vec<NewTranslationEntry>,
    stats: TranslationStats,
    logs: Vec<LogEvent>,
    item_diagnostics: Vec<ItemDiagnostics>,
}

#[derive(Clone, Copy)]
struct PipelineBehavior {
    use_dictionary_lookup: bool,
    emit_dictionary_events: bool,
    commit_new_entries: bool,
    emit_word_logs: bool,
    emit_stats: bool,
    emit_observations: bool,
}

impl PipelineBehavior {
    fn normal_translate() -> Self {
        Self {
            use_dictionary_lookup: true,
            emit_dictionary_events: true,
            commit_new_entries: true,
            emit_word_logs: true,
            emit_stats: true,
            emit_observations: true,
        }
    }

    fn list_mode() -> Self {
        Self {
            use_dictionary_lookup: false,
            emit_dictionary_events: false,
            commit_new_entries: false,
            emit_word_logs: true,
            emit_stats: false,
            emit_observations: false,
        }
    }
}

struct PipelineResult {
    texts: Vec<String>,
    translated_text: String,
    item_count: usize,
    analysis_payload: Option<CompletedAnalysisPayload>,
}

#[derive(Default)]
struct CommitSummary {
    regex_registered: usize,
    regex_skipped: usize,
    exact_committed: usize,
}

fn preview_text(text: &str) -> String {
    const LIMIT: usize = 80;

    let normalized = text.replace("\r", "").replace("\n", " [nl] ");
    let mut preview = String::new();

    for (index, ch) in normalized.chars().enumerate() {
        if index >= LIMIT {
            preview.push_str("...");
            break;
        }
        preview.push(ch);
    }

    preview
}

fn emit_batch_diagnostics(state: &AppState, route: &str, batch: &BatchTranslationOutput) {
    if batch.item_diagnostics.is_empty() {
        return;
    }

    let segmented_items = batch
        .item_diagnostics
        .iter()
        .filter(|item| item.model_calls > 1)
        .count();

    state.emit_diagnostic(format!(
        "{} request: items={}, model_calls={}, dict_hits={}, segmented_items={}",
        route,
        batch.item_diagnostics.len(),
        batch.stats.model_calls,
        batch.stats.dict_hits,
        segmented_items,
    ));

    for (index, item) in batch.item_diagnostics.iter().enumerate() {
        if item.model_calls > 1 {
            state.emit_diagnostic(format!(
                "{} item#{} split into {} model calls (dict_hits={}): {}",
                route,
                index + 1,
                item.model_calls,
                item.dict_hits,
                item.input_preview,
            ));
        }
    }
}

fn preview_body(text: &str) -> String {
    preview_text(text)
}

fn build_translate_request_record(
    source: &str,
    content_type: Option<&str>,
    raw_request: &str,
    parsed_text: &str,
) -> String {
    serde_json::json!({
        "kind": "request",
        "route": "translate",
        "source": source,
        "content_type": content_type.unwrap_or_default(),
        "raw_request": raw_request,
        "parsed_text": parsed_text,
        "line_count": parsed_text.split('\n').count(),
    })
    .to_string()
}

fn build_list_request_record(request: &ListRequest) -> String {
    serde_json::json!({
        "kind": "request",
        "route": "list",
        "source": "json",
        "raw_request": request,
        "joined_text": request.texts.join("\n"),
        "item_count": request.texts.len(),
        "total_bytes": total_list_request_bytes(&request.texts),
    })
    .to_string()
}

fn build_translate_response_record(translated_text: &str) -> String {
    serde_json::json!({
        "kind": "response",
        "route": "translate",
        "response_text": translated_text,
        "line_count": translated_text.split('\n').count(),
    })
    .to_string()
}

fn build_list_response_record(translated_text: &str, item_count: usize) -> String {
    serde_json::json!({
        "kind": "response",
        "route": "list",
        "response_text": translated_text,
        "item_count": item_count,
        "line_count": translated_text.split('\n').count(),
    })
    .to_string()
}

fn total_list_request_bytes(texts: &[String]) -> usize {
    texts.iter().map(|text| text.len()).sum()
}

fn model_inputs_from_logs(logs: &[LogEvent]) -> Vec<String> {
    logs.iter()
        .filter_map(|event| match event {
            LogEvent::PreModelCall { original } => Some(original.clone()),
            _ => None,
        })
        .collect()
}

fn build_observation_record(
    route: &str,
    raw_line: &str,
    extracted_text: &str,
    visible_text: &str,
    final_output: &str,
    dict_hits: usize,
    model_calls: usize,
    model_inputs: &[String],
) -> String {
    serde_json::json!({
        "route": route,
        "raw_line": raw_line,
        "extracted_text": extracted_text,
        "visible_text": visible_text,
        "model_inputs": model_inputs,
        "final_output": final_output,
        "dict_hits": dict_hits,
        "model_calls": model_calls,
    })
    .to_string()
}

fn emit_observation_logs(
    state: &AppState,
    route: &str,
    translated_lines: &[String],
    diagnostics: &[ItemDiagnostics],
) {
    for (translated, diagnostic) in translated_lines.iter().zip(diagnostics.iter()) {
        let record = build_observation_record(
            route,
            &diagnostic.raw_text,
            &diagnostic.extracted_text,
            &diagnostic.visible_text,
            translated,
            diagnostic.dict_hits,
            diagnostic.model_calls,
            &diagnostic.model_inputs,
        );
        state.emit_observation(record);
    }
}

fn build_completed_analysis_payload(
    batch: &BatchTranslationOutput,
    final_output: &str,
) -> Option<CompletedAnalysisPayload> {
    if batch.item_diagnostics.is_empty() {
        return None;
    }

    Some(CompletedAnalysisPayload {
        raw_text: batch
            .item_diagnostics
            .iter()
            .map(|item| item.raw_text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        extracted_text: batch
            .item_diagnostics
            .iter()
            .map(|item| item.extracted_text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        visible_text: batch
            .item_diagnostics
            .iter()
            .map(|item| item.visible_text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        model_inputs: batch
            .item_diagnostics
            .iter()
            .flat_map(|item| item.model_inputs.iter().cloned())
            .collect(),
        final_output: final_output.to_string(),
        dict_hits: batch.stats.dict_hits,
        model_calls: batch.stats.model_calls,
    })
}

fn parse_form_text(body: &str) -> Option<String> {
    for pair in body.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next().unwrap_or_default();

        if key != "text" && key != "content" {
            continue;
        }

        if let Ok(decoded) = urlencoding::decode(value) {
            return Some(decoded.into_owned());
        }
    }

    None
}

fn extract_translate_post_text(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }

    let body_text = std::str::from_utf8(body).ok()?;
    let body_view = body_text.trim();
    if body_view.is_empty() {
        return None;
    }

    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();

    if content_type.contains("application/json") || body_view.starts_with('{') {
        if let Ok(request) = serde_json::from_str::<TranslateRequest>(body_view) {
            if let Some(text) = request.text.or(request.content) {
                return Some(text);
            }
        }
    }

    if content_type.contains("application/x-www-form-urlencoded")
        || body_view.starts_with("text=")
        || body_view.starts_with("content=")
        || body_view.contains("&text=")
        || body_view.contains("&content=")
    {
        if let Some(text) = parse_form_text(body_view) {
            return Some(text);
        }
    }

    Some(body_text.to_string())
}

fn translate_texts_batch(
    dictionary: Arc<RwLock<Dictionary>>,
    llm_client: Arc<dyn translator::LlmClient>,
    t_cache: Arc<TranslationCache>,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
    prefix: String,
    llm_slots: usize,
    tgt_lang: String,
    settings: TranslationSettings,
    behavior: PipelineBehavior,
    texts: Vec<String>,
) -> BatchTranslationOutput {
    fn translate_one_text(
        dictionary: &Arc<RwLock<Dictionary>>,
        llm_client: &Arc<dyn translator::LlmClient>,
        t_cache: &Arc<TranslationCache>,
        prefix: &str,
        tgt_lang: &str,
        settings: TranslationSettings,
        use_dictionary_lookup: bool,
        text: &str,
    ) -> (TranslationResult, ItemDiagnostics) {
        let lookup_trace = std::sync::Arc::new(std::sync::Mutex::new(Vec::<LogEvent>::new()));
        let lookup_trace_clone = std::sync::Arc::clone(&lookup_trace);

        let lookup = move |key: &str| -> Option<String> {
            if !use_dictionary_lookup {
                lookup_trace_clone.lock().unwrap().push(LogEvent::Trace {
                    message: serde_json::json!({
                        "stage": "after_lookup",
                        "result": "lookup_disabled",
                        "key": preview_text(key),
                        "key_len": key.len(),
                    })
                    .to_string(),
                });
                return None;
            }

            if let Some(value) = t_cache.lookup_source(key) {
                lookup_trace_clone.lock().unwrap().push(LogEvent::Trace {
                    message: serde_json::json!({
                        "stage": "after_lookup",
                        "result": "hit_cache_source",
                        "key": preview_text(key),
                        "key_len": key.len(),
                        "hit_value": crate::backend::server::preview_text(&value),
                        "hit_value_len": value.len(),
                    })
                    .to_string(),
                });
                return Some(value);
            }

            if let Some(value) = dictionary.blocking_read().lookup_source(key) {
                t_cache.insert(key.to_string(), value.clone());
                lookup_trace_clone.lock().unwrap().push(LogEvent::Trace {
                    message: serde_json::json!({
                        "stage": "after_lookup",
                        "result": "hit_dictionary_source",
                        "key": preview_text(key),
                        "key_len": key.len(),
                        "hit_value": crate::backend::server::preview_text(&value),
                        "hit_value_len": value.len(),
                    })
                    .to_string(),
                });
                return Some(value);
            }

            if let Some(value) = t_cache.lookup_value(key) {
                lookup_trace_clone.lock().unwrap().push(LogEvent::Trace {
                    message: serde_json::json!({
                        "stage": "after_lookup",
                        "result": "hit_cache_value",
                        "hit_kind": "value_observation",
                        "key": preview_text(key),
                        "key_len": key.len(),
                        "hit_value": crate::backend::server::preview_text(&value),
                        "hit_value_len": value.len(),
                    })
                    .to_string(),
                });
                return Some(value);
            }

            if let Some(value) = dictionary.blocking_read().lookup_value(key) {
                lookup_trace_clone.lock().unwrap().push(LogEvent::Trace {
                    message: serde_json::json!({
                        "stage": "after_lookup",
                        "result": "hit_dictionary_value",
                        "hit_kind": "value_observation",
                        "key": preview_text(key),
                        "key_len": key.len(),
                        "hit_value": crate::backend::server::preview_text(&value),
                        "hit_value_len": value.len(),
                    })
                    .to_string(),
                });
                return Some(value);
            }

            lookup_trace_clone.lock().unwrap().push(LogEvent::Trace {
                message: serde_json::json!({
                    "stage": "after_lookup",
                    "result": "miss",
                    "key": preview_text(key),
                    "key_len": key.len(),
                })
                .to_string(),
            });
            None
        };

        let lookup_split = move |key: &str| -> Option<SplitResult> {
            dictionary.blocking_read().lookup_split(key)
        };

        let mut result = translator::translate_chunk(
            text,
            lookup,
            lookup_split,
            prefix,
            tgt_lang,
            llm_client.as_ref(),
            settings,
        );

        let lookup_logs = lookup_trace.lock().unwrap().drain(..).collect::<Vec<_>>();
        result.logs.extend(lookup_logs);

        let diagnostics = ItemDiagnostics {
            raw_text: text.to_string(),
            extracted_text: text.to_string(),
            visible_text: text.to_string(),
            input_preview: preview_text(text),
            dict_hits: result.stats.dict_hits,
            model_calls: result.stats.model_calls,
            model_inputs: model_inputs_from_logs(&result.logs),
        };

        (result, diagnostics)
    }

    fn emit_new_entries(
        result: &TranslationResult,
        event_tx: &tokio::sync::mpsc::Sender<BackendEvent>,
    ) {
        for entry in &result.new_entries {
            let _ = event_tx.try_send(BackendEvent::DictionaryNewEntry(
                crate::messages::current_timestamp(),
                entry.source.clone(),
                entry.translated.clone(),
            ));
        }
    }

    let mut output = BatchTranslationOutput::default();

    if texts.is_empty() {
        return output;
    }

    let worker_count = llm_slots.max(1).min(texts.len());

    if worker_count == 1 {
        for text in texts {
            let (result, diagnostics) = translate_one_text(
                &dictionary,
                &llm_client,
                &t_cache,
                &prefix,
                &tgt_lang,
                settings,
                behavior.use_dictionary_lookup,
                &text,
            );

            if behavior.emit_dictionary_events {
                emit_new_entries(&result, &event_tx);
            }
            output.logs.extend(result.logs.clone());
            output.new_entries.extend(result.new_entries.clone());
            output.stats.merge(&result.stats);
            output.texts.push(result.text);
            output.item_diagnostics.push(diagnostics);
        }

        return output;
    }

    let total = texts.len();
    let jobs = Arc::new(Mutex::new(
        texts.into_iter().enumerate().collect::<VecDeque<_>>(),
    ));
    let results = Arc::new(Mutex::new(vec![
        None::<(TranslationResult, ItemDiagnostics)>;
        total
    ]));

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let jobs = Arc::clone(&jobs);
            let results = Arc::clone(&results);
            let dictionary = Arc::clone(&dictionary);
            let llm_client = Arc::clone(&llm_client);
            let t_cache = Arc::clone(&t_cache);
            let prefix = prefix.clone();
            let tgt_lang = tgt_lang.clone();
            let settings = settings;

            scope.spawn(move || loop {
                let next_job = {
                    let mut jobs = jobs.lock().expect("jobs mutex poisoned");
                    jobs.pop_front()
                };

                let (index, text) = match next_job {
                    Some(job) => job,
                    None => break,
                };

                let result = translate_one_text(
                    &dictionary,
                    &llm_client,
                    &t_cache,
                    &prefix,
                    &tgt_lang,
                    settings,
                    behavior.use_dictionary_lookup,
                    &text,
                );

                let mut results = results.lock().expect("results mutex poisoned");
                results[index] = Some(result);
            });
        }
    });

    let results = results.lock().expect("results mutex poisoned");
    for (result, diagnostics) in results.iter().flatten() {
        if behavior.emit_dictionary_events {
            emit_new_entries(result, &event_tx);
        }
        output.logs.extend(result.logs.clone());
        output.new_entries.extend(result.new_entries.clone());
        output.stats.merge(&result.stats);
        output.texts.push(result.text.clone());
        output.item_diagnostics.push(diagnostics.clone());
    }

    output
}

async fn commit_new_entries(
    dictionary: &Arc<RwLock<Dictionary>>,
    t_cache: &TranslationCache,
    n_cache: &NewEntriesCache,
    entries: &[NewTranslationEntry],
) -> CommitSummary {
    let mut summary = CommitSummary::default();
    for entry in entries {
        match &entry.persist {
            PersistEntry::Exact { key, value } => {
                log::info!(
                    "[COMMIT] exact_received source=\"{}\" value=\"{}\"",
                    key,
                    value
                );
                t_cache.insert(key.clone(), value.clone());
                n_cache.insert(key.clone(), value.clone());
                log::info!(
                    "[COMMIT] exact_queued_save key=\"{}\" value=\"{}\"",
                    key,
                    value
                );
                summary.exact_committed += 1;
            }
            PersistEntry::Regex {
                pattern,
                replacement,
            } => {
                log::info!(
                    "[COMMIT] regex_received source=\"{}\" pattern=\"{}\" replacement=\"{}\"",
                    entry.source,
                    pattern,
                    replacement
                );
                // Register immediately in the live dictionary for same‑session hits
                let registered = {
                    let mut dict = dictionary.write().await;
                    dict.register_regex_rule(pattern.clone(), replacement.clone())
                };

                log::info!(
                    "[COMMIT] regex_live_registered ok={} pattern=\"{}\"",
                    registered,
                    pattern
                );

                if registered {
                    // Save r: line to n_cache for shutdown flush
                    let key = format!("r:\"{}\"", pattern);
                    n_cache.insert(key.clone(), replacement.clone());
                    log::info!(
                        "[COMMIT] regex_queued_save key='{}' value=\"{}\"",
                        key,
                        replacement
                    );
                    summary.regex_registered += 1;
                } else {
                    // Regex registration failed. Do not queue an exact fallback save.
                    log::info!(
                        "[COMMIT] regex_save_skipped source=\"{}\" pattern=\"{}\" reason=live_register_failed",
                        entry.source,
                        pattern
                    );
                    summary.regex_skipped += 1;
                }
            }
        }
    }

    summary
}

// ============================================================
// Common translation pipeline: translate → commit → emit
//
// Handlers own HTTP shape and request/response records.
// run_pipeline owns everything between: batch execution and side-effects.

async fn run_pipeline(
    state: &AppState,
    route: &str,
    behavior: PipelineBehavior,
    texts: Vec<String>,
) -> Result<PipelineResult, String> {
    let prefix = state.current_prefix().await;
    let llm_client = state.llm_client.clone();
    let dictionary = state.dictionary.clone();
    let dict_for_commit = dictionary.clone();
    let t_cache = state.t_cache.clone();
    let event_tx = state.event_tx.clone();
    let llm_slots = state.llm_slots;
    let tgt_lang = state.tgt_lang.read().await.clone();
    let settings = state.translation_settings;

    let batch = tokio::task::spawn_blocking(move || {
        translate_texts_batch(
            dictionary, llm_client, t_cache, event_tx, prefix, llm_slots, tgt_lang, settings,
            behavior, texts,
        )
    })
    .await
    .map_err(|e| format!("translation worker panicked: {}", e))?;

    let item_count = batch.texts.len();
    let translated_text = batch.texts.join("\n");
    let analysis_payload = behavior
        .emit_observations
        .then(|| build_completed_analysis_payload(&batch, &translated_text))
        .flatten();

    if behavior.commit_new_entries {
        let exact_entries = batch
            .new_entries
            .iter()
            .filter(|entry| matches!(entry.persist, PersistEntry::Exact { .. }))
            .count();
        let regex_entries = batch.new_entries.len().saturating_sub(exact_entries);
        state.emit_log(&LogEvent::Trace {
            message: serde_json::json!({
                "stage": "before_commit_new_entries",
                "new_entries_len": batch.new_entries.len(),
                "regex_entries": regex_entries,
                "exact_entries": exact_entries,
            })
            .to_string(),
        });

        let summary = commit_new_entries(
            &dict_for_commit,
            state.t_cache.as_ref(),
            state.n_cache.as_ref(),
            &batch.new_entries,
        )
        .await;

        state.emit_log(&LogEvent::Trace {
            message: serde_json::json!({
                "stage": "after_commit_new_entries",
                "regex_registered": summary.regex_registered,
                "regex_skipped": summary.regex_skipped,
                "exact_committed": summary.exact_committed,
            })
            .to_string(),
        });
    }
    if behavior.emit_word_logs {
        emit_word_log_pairs(&state.event_tx, &batch.logs);
    }
    for log in &batch.logs {
        state.emit_log(log);
    }
    if behavior.emit_stats {
        state.emit_stats(&batch.stats);
    }
    emit_batch_diagnostics(state, route, &batch);
    if behavior.emit_observations {
        emit_observation_logs(state, route, &batch.texts, &batch.item_diagnostics);
    }

    Ok(PipelineResult {
        texts: batch.texts,
        translated_text,
        item_count,
        analysis_payload,
    })
}

async fn perform_translation(state: Arc<AppState>, text: String) -> Response {
    let text = text.replace("\r\n", "\n");
    let result = match run_pipeline(
        &state,
        "translate",
        PipelineBehavior::normal_translate(),
        vec![text.clone()],
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            state.emit_persistent_log(e, LogLevel::Error);
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    state.emit_request_log(build_translate_response_record(&result.translated_text));

    state.emit_log(&LogEvent::Trace {
        message: serde_json::json!({
            "stage": "before_response",
            "response_text": preview_text(&result.translated_text),
            "response_text_len": result.translated_text.len(),
        })
        .to_string(),
    });

    let Some(authority_payload) = result.analysis_payload else {
        state.emit_persistent_log(
            "translate completed without input analysis authority payload".to_string(),
            LogLevel::Error,
        );
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let snapshot = analysis::record_completed_translation(
        &state.input_replay,
        CompletedTranslationRecord { authority_payload },
    );
    let _ = state
        .event_tx
        .try_send(BackendEvent::InputAnalysisUpdated(snapshot));
    result.translated_text.into_response()
}
// ============================================================
// Handlers
// ============================================================
// ============================================================

async fn health_handler() -> &'static str {
    "OK"
}

#[axum::debug_handler]
async fn translate_get_handler(
    State(state): State<Arc<AppState>>,
    query: Query<TranslateRequest>,
) -> Response {
    let text = query
        .text
        .clone()
        .or(query.content.clone())
        .unwrap_or_default();
    let raw_request = serde_json::to_string(&query.0).unwrap_or_else(|_| "{}".to_string());
    state.emit_request_log(build_translate_request_record(
        "query",
        None,
        &raw_request,
        &text,
    ));
    if text.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "no text").into_response();
    }

    if text.len() > MAX_TRANSLATE_TEXT_BYTES {
        state.emit_diagnostic(format!(
            "translate GET rejected: text too large bytes={} limit={}",
            text.len(),
            MAX_TRANSLATE_TEXT_BYTES
        ));
        return (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "translate text too large",
        )
            .into_response();
    }

    state.emit_log(&LogEvent::Trace {
        message: serde_json::json!({
            "stage": "request_received",
            "route": "translate",
            "source": "query",
            "content_type": "",
            "raw_request_preview": preview_text(&raw_request),
            "raw_request_len": raw_request.len(),
            "parsed_text_preview": preview_text(&text),
            "parsed_text_len": text.len(),
            "normalized_text_preview": preview_text(&text.replace("\r\n", "\n")),
            "normalized_text_len": text.replace("\r\n", "\n").len(),
        })
        .to_string(),
    });

    perform_translation(state, text).await
}

pub async fn translate_post_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if body.len() > MAX_TRANSLATE_BODY_BYTES {
        state.emit_diagnostic(format!(
            "translate rejected: body too large bytes={} limit={}",
            body.len(),
            MAX_TRANSLATE_BODY_BYTES
        ));
        return (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "translate request body too large",
        )
            .into_response();
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let raw_request = std::str::from_utf8(&body)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| format!("{} bytes (non-utf8)", body.len()));

    let text = extract_translate_post_text(content_type, &body).unwrap_or_default();
    state.emit_request_log(build_translate_request_record(
        "post_body",
        content_type,
        &raw_request,
        &text,
    ));

    if text.is_empty() && !body.is_empty() {
        let raw_preview = std::str::from_utf8(&body)
            .map(preview_body)
            .unwrap_or_else(|_| format!("{} bytes (non-utf8)", body.len()));
        state.emit_diagnostic(format!(
            "translate POST body could not be parsed: content_type='{}', body={}",
            content_type.unwrap_or("<none>"),
            raw_preview,
        ));
    }

    if text.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "no text").into_response();
    }

    if text.len() > MAX_TRANSLATE_TEXT_BYTES {
        state.emit_diagnostic(format!(
            "translate rejected: text too large bytes={} limit={}",
            text.len(),
            MAX_TRANSLATE_TEXT_BYTES
        ));
        return (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "translate text too large",
        )
            .into_response();
    }

    state.emit_log(&LogEvent::Trace {
        message: serde_json::json!({
            "stage": "request_received",
            "route": "translate",
            "source": "post_body",
            "content_type": content_type.unwrap_or_default(),
            "raw_request_preview": preview_text(&raw_request),
            "raw_request_len": raw_request.len(),
            "parsed_text_preview": preview_text(&text),
            "parsed_text_len": text.len(),
            "normalized_text_preview": preview_text(&text.replace("\r\n", "\n")),
            "normalized_text_len": text.replace("\r\n", "\n").len(),
        })
        .to_string(),
    });

    perform_translation(state, text).await
}

#[axum::debug_handler]
async fn list_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ListRequest>,
) -> Response {
    if request.texts.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "no texts").into_response();
    }

    let item_count = request.texts.len();
    if item_count > MAX_LIST_ITEMS {
        state.emit_diagnostic(format!(
            "list request rejected: item_count={} exceeds limit={}",
            item_count, MAX_LIST_ITEMS
        ));
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "too many texts for /list",
        )
            .into_response();
    }

    let total_bytes = total_list_request_bytes(&request.texts);
    if total_bytes > MAX_LIST_TOTAL_BYTES {
        state.emit_diagnostic(format!(
            "list request rejected: total_bytes={} exceeds limit={}",
            total_bytes, MAX_LIST_TOTAL_BYTES
        ));
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "list payload too large",
        )
            .into_response();
    }

    state.emit_request_log(build_list_request_record(&request));

    let result =
        match run_pipeline(&state, "list", PipelineBehavior::list_mode(), request.texts).await {
            Ok(r) => r,
            Err(e) => {
                state.emit_persistent_log(e, LogLevel::Error);
                return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    state.emit_request_log(build_list_response_record(
        &result.translated_text,
        result.item_count,
    ));
    Json(ListResponse {
        texts: result.texts,
    })
    .into_response()
}

async fn shutdown_handler(State(state): State<Arc<AppState>>) -> &'static str {
    state.t_cache.clear();
    let drained = state.n_cache.drain();
    if !drained.is_empty() {
        log::info!(
            "[FLUSH] shutdown_route_drain_discard count={}",
            drained.len()
        );
    }

    let _ = state.event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        "Shutdown request received: translation cache cleared".to_string(),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));

    "ok"
}

async fn serve_tcp_connection(
    stream: tokio::net::TcpStream,
    state: Arc<AppState>,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = TokioBufReader::new(reader);
    let mut len_buf = [0u8; 4];

    loop {
        if reader.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let payload_len = u32::from_le_bytes(len_buf) as usize;

        if payload_len > TCP_MAX_PAYLOAD_BYTES {
            let _ = event_tx.try_send(BackendEvent::Log(
                LogSource::Tenuki,
                format!("tcp payload too large: {} (max {})", payload_len, TCP_MAX_PAYLOAD_BYTES),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            break;
        }

        let mut payload = vec![0u8; payload_len];
        if reader.read_exact(&mut payload).await.is_err() {
            break;
        }

        let text = String::from_utf8_lossy(&payload).into_owned();

        let result = run_pipeline(
            &state,
            "tcp",
            PipelineBehavior::normal_translate(),
            vec![text],
        )
        .await;

        let response = match result {
            Ok(r) => r.translated_text.into_bytes(),
            Err(e) => {
                let _ = event_tx.try_send(BackendEvent::Log(
                    LogSource::Tenuki,
                    format!("tcp pipeline error: {}", e),
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                continue;
            }
        };

        let resp_len = response.len() as u32;
        if writer.write_all(&resp_len.to_le_bytes()).await.is_err() {
            break;
        }
        if writer.write_all(&response).await.is_err() {
            break;
        }
    }
}

async fn run_tcp_listener(
    state: Arc<AppState>,
    host: &str,
    port: u16,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let addr: std::net::SocketAddr = match format!("{}:{}", host, port).parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = event_tx.try_send(BackendEvent::Log(
                LogSource::Tenuki,
                format!("tcp invalid address {}:{} ({})", host, port, e),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            return;
        }
    };

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            let _ = event_tx.try_send(BackendEvent::Log(
                LogSource::Tenuki,
                format!("tcp bind failed {}:{} ({})", host, port, e),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            return;
        }
    };

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        format!("tcp translation listening on {}:{}", host, port),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));

    let accept_handle = {
        let state = state.clone();
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        serve_tcp_connection(stream, state.clone(), event_tx.clone()).await;
                    }
                    Err(_) => break,
                }
            }
        })
    };

    let _ = shutdown_rx.await;
    accept_handle.abort();

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        "tcp server stopped".to_string(),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));
}

// ============================================================
// Server startup
// Caller owns task spawning and shutdown coordination.
// ============================================================

pub async fn run_translation_server(
    host: String,
    port: u16,
    tcp_port: u16,
    dictionary: Arc<RwLock<Dictionary>>,
    src_lang: String,
    tgt_lang: String,
    custom_lang_name: String,
    prompt_template: String,
    background_text: String,
    translation_settings: TranslationSettings,
    llm_client: Arc<dyn translator::LlmClient>,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
    startup_tx: oneshot::Sender<Result<(), String>>,
    shutdown_rx: oneshot::Receiver<()>,
    tcp_shutdown_rx: oneshot::Receiver<()>,
    t_cache: Arc<TranslationCache>,
    n_cache: Arc<NewEntriesCache>,
    input_replay: SharedInputReplayState,
    llm_slots: usize,
) {
    let addr: std::net::SocketAddr = match format!("{}:{}", host, port).parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = startup_tx.send(Err(format!("Invalid host:port '{}:{}': {}", host, port, e)));
            return;
        }
    };

    let state = Arc::new(AppState {
        dictionary,
        src_lang: Arc::new(RwLock::new(src_lang)),
        tgt_lang: Arc::new(RwLock::new(tgt_lang)),
        custom_lang_name: Arc::new(RwLock::new(custom_lang_name)),
        prompt_template,
        background_text,
        translation_settings,
        llm_client,
        event_tx: event_tx.clone(),
        t_cache,
        n_cache,
        input_replay,
        llm_slots: llm_slots.max(1),
    });

    let tcp_state = state.clone();
    let tcp_event_tx = event_tx.clone();
    let tcp_host = host.clone();
    tokio::spawn(async move {
        run_tcp_listener(tcp_state, &tcp_host, tcp_port, tcp_event_tx, tcp_shutdown_rx).await;
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/", get(translate_get_handler))
        .route("/", post(translate_post_handler))
        .route("/translate", get(translate_get_handler))
        .route("/translate", post(translate_post_handler))
        .route("/list", post(list_handler))
        .route("/shutdown", post(shutdown_handler))
        .with_state(state);

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        format!("Translation server binding: {}:{}", host, port),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));

    // Retry bind for AddrInUse / os error 10048.
    let max_retries: u32 = 10;
    let mut retries = max_retries;
    let listener = loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => break l,
            Err(e)
                if retries > 0
                    && (e.kind() == std::io::ErrorKind::AddrInUse
                        || e.raw_os_error() == Some(10048)) =>
            {
                retries -= 1;
                let _ = event_tx.try_send(BackendEvent::Log(
                    LogSource::Tenuki,
                    format!(
                        "Translation server waiting to bind ({}:{}, retries left {}): {}",
                        host, port, retries, e
                    ),
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                continue;
            }
            Err(e) => {
                let _ = startup_tx.send(Err(format!(
                    "Failed to bind translation server: {}:{} (attempt {}/{}): {}",
                    host,
                    port,
                    max_retries - retries,
                    max_retries,
                    e
                )));
                return;
            }
        }
    };

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        format!(
            "TENUKI translation server listening on http://127.0.0.1:{}",
            port
        ),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));
    // Serve with graceful shutdown.
    let _ = startup_tx.send(Ok(()));

    // Serve with graceful shutdown.
    let _ = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            shutdown_rx.await.ok();
        })
        .await;

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        "Translation server stopped gracefully".to_string(),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));
}

#[cfg(test)]
mod tests {
    use super::{
        build_completed_analysis_payload, build_list_request_record, build_list_response_record,
        build_observation_record, build_translate_request_record, build_translate_response_record,
        commit_new_entries, emit_word_log_pairs, extract_translate_post_text, list_handler,
        perform_translation, run_pipeline, total_list_request_bytes, translate_get_handler,
        translate_post_handler, translate_texts_batch, AppState, BatchTranslationOutput,
        ItemDiagnostics, ListRequest, ListResponse, PipelineBehavior, TranslateRequest,
        MAX_LIST_ITEMS, MAX_LIST_TOTAL_BYTES, MAX_TRANSLATE_BODY_BYTES, MAX_TRANSLATE_TEXT_BYTES,
    };
    use crate::backend::analysis::InputReplayState;
    use crate::backend::dictionary::{Dictionary, SplitResult};
    use crate::backend::translator::{
        self, LogEvent, NewEntriesCache, NewTranslationEntry, PersistEntry, TranslationCache,
        TranslationSettings, TranslationStats,
    };
    use crate::messages::BackendEvent;
    use axum::{body::to_bytes, http::StatusCode};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::sync::RwLock;

    #[derive(Default)]
    struct MockLlmClient {
        calls: Mutex<Vec<String>>,
        responses: Mutex<Vec<String>>,
    }

    impl MockLlmClient {
        fn with_responses(values: &[&str]) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(values.iter().map(|v| v.to_string()).collect()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl translator::LlmClient for MockLlmClient {
        fn translate_sync(&self, text: &str, _prefix: &str) -> Option<String> {
            self.calls.lock().unwrap().push(text.to_string());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                None
            } else {
                Some(responses.remove(0))
            }
        }
    }

    struct PanickingLlmClient;

    impl translator::LlmClient for PanickingLlmClient {
        fn translate_sync(&self, _text: &str, _prefix: &str) -> Option<String> {
            panic!("test panic")
        }
    }

    struct InjectingLlmClient {
        inner: MockLlmClient,
        dictionary: Arc<RwLock<Dictionary>>,
        pattern: String,
        replacement: String,
    }

    impl InjectingLlmClient {
        fn with_responses(
            dictionary: Arc<RwLock<Dictionary>>,
            pattern: &str,
            replacement: &str,
            values: &[&str],
        ) -> Self {
            Self {
                inner: MockLlmClient::with_responses(values),
                dictionary,
                pattern: pattern.to_string(),
                replacement: replacement.to_string(),
            }
        }
    }

    impl translator::LlmClient for InjectingLlmClient {
        fn translate_sync(&self, text: &str, prefix: &str) -> Option<String> {
            let translated = self.inner.translate_sync(text, prefix);
            if translated.is_some() {
                let _ = self
                    .dictionary
                    .blocking_write()
                    .register_regex_rule(self.pattern.clone(), self.replacement.clone());
            }
            translated
        }
    }

    fn test_app_state_with_event_rx(
        llm_client: Arc<dyn translator::LlmClient>,
    ) -> (Arc<AppState>, tokio::sync::mpsc::Receiver<BackendEvent>) {
        let dict_dir = test_dictionary_dir("handler");
        let (dict_tx, _) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(128);
        let state = Arc::new(AppState {
            dictionary,
            src_lang: Arc::new(RwLock::new("ja".to_string())),
            tgt_lang: Arc::new(RwLock::new("en".to_string())),
            custom_lang_name: Arc::new(RwLock::new(String::new())),
            prompt_template: String::new(),
            background_text: String::new(),
            translation_settings: test_settings(),
            llm_client,
            event_tx,
            t_cache: Arc::new(TranslationCache::default()),
            n_cache: Arc::new(NewEntriesCache::default()),
            input_replay: Arc::new(Mutex::new(InputReplayState::default())),
            llm_slots: 1,
        });
        (state, event_rx)
    }

    fn test_app_state_with_llm(llm_client: Arc<dyn translator::LlmClient>) -> Arc<AppState> {
        test_app_state_with_event_rx(llm_client).0
    }

    fn test_app_state() -> Arc<AppState> {
        test_app_state_with_llm(Arc::new(MockLlmClient::default()))
    }

    fn test_app_state_with_dictionary_lines(
        tag: &str,
        llm_client: Arc<dyn translator::LlmClient>,
        lines: &[&str],
    ) -> (Arc<AppState>, PathBuf) {
        let dict_dir = test_dictionary_dir(tag);
        let root = dict_dir.join("txt_root");
        std::fs::write(root.join("root.txt"), lines.join("\n")).unwrap();

        let (dict_tx, _) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            root,
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(128);
        let state = Arc::new(AppState {
            dictionary,
            src_lang: Arc::new(RwLock::new("ja".to_string())),
            tgt_lang: Arc::new(RwLock::new("en".to_string())),
            custom_lang_name: Arc::new(RwLock::new(String::new())),
            prompt_template: String::new(),
            background_text: String::new(),
            translation_settings: test_settings(),
            llm_client,
            event_tx,
            t_cache: Arc::new(TranslationCache::default()),
            n_cache: Arc::new(NewEntriesCache::default()),
            input_replay: Arc::new(Mutex::new(InputReplayState::default())),
            llm_slots: 1,
        });
        (state, dict_dir)
    }

    fn test_settings() -> TranslationSettings {
        TranslationSettings {
            enable_model_wrap: true,
            model_wrap_min_chars: 60,
            model_wrap_space_fallback_min_chars: 100,
            enable_model_symbol_cleanup: true,
        }
    }

    fn test_dictionary_dir(tag: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tenuki_server_test_{}_{}", tag, unique));
        std::fs::create_dir_all(dir.join("txt_root")).unwrap();
        dir
    }

    #[tokio::test]
    async fn handler_returns_400_for_empty_input_translate_and_list() {
        let state = test_app_state();

        let res = translate_get_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Query(TranslateRequest {
                text: None,
                content: None,
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let res = list_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Json(ListRequest { texts: vec![] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_handler_rejects_too_many_items() {
        let state = test_app_state();
        let texts = (0..(MAX_LIST_ITEMS + 1))
            .map(|i| format!("item-{i}"))
            .collect::<Vec<_>>();

        let res = list_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Json(ListRequest { texts }),
        )
        .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "too many texts for /list"
        );
    }

    #[tokio::test]
    async fn list_handler_rejects_payloads_over_total_byte_limit() {
        let state = test_app_state();
        let oversized = "a".repeat(MAX_LIST_TOTAL_BYTES + 1);
        let texts = vec![oversized];

        assert!(total_list_request_bytes(&texts) > MAX_LIST_TOTAL_BYTES);

        let res = list_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Json(ListRequest { texts }),
        )
        .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "list payload too large"
        );
    }

    #[tokio::test]
    async fn list_handler_bypasses_dictionary_cache_commit_and_input_analysis_but_emits_word_logs()
    {
        let llm_client: Arc<dyn translator::LlmClient> =
            Arc::new(MockLlmClient::with_responses(&["model result"]));
        let (state, mut event_rx) = test_app_state_with_event_rx(llm_client);

        state
            .dictionary
            .write()
            .await
            .register("hello", "dictionary result");
        state
            .t_cache
            .insert("hello".to_string(), "cache result".to_string());

        let res = list_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Json(ListRequest {
                texts: vec!["hello".to_string()],
            }),
        )
        .await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let response: ListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(response.texts, vec!["model result".to_string()]);
        assert_eq!(
            state.t_cache.get("hello").map(|value| value.clone()),
            Some("cache result".to_string())
        );
        assert!(state.n_cache.drain().is_empty());

        let mut word_log = None;
        while let Ok(event) = event_rx.try_recv() {
            match event {
                BackendEvent::DictionaryLogEntry(_, source, translated) => {
                    word_log = Some((source, translated));
                }
                BackendEvent::InputAnalysisUpdated(_) | BackendEvent::StatisticsUpdate(_, _) => {
                    panic!("List mode emitted normal-mode side-effect event: {event:?}");
                }
                _ => {}
            }
        }

        assert_eq!(
            word_log,
            Some((
                "[XUnity] hello".to_string(),
                "[Model] (0.00s) model result".to_string(),
            ))
        );
    }

    #[tokio::test]
    async fn emit_log_keeps_tenuki_entries_but_skips_word_log_entries() {
        let (state, mut event_rx) =
            test_app_state_with_event_rx(Arc::new(MockLlmClient::default()));

        state.emit_log(&LogEvent::PreModelCall {
            original: "hello".to_string(),
        });
        state.emit_log(&LogEvent::ModelResult {
            source: "hello".to_string(),
            original: "hello".to_string(),
            translated: "bonjour".to_string(),
            elapsed_secs: 0.42,
        });
        state.emit_log(&LogEvent::DictHit {
            original: "hello".to_string(),
            translated: "dictionary".to_string(),
            elapsed_secs: 0.01,
        });

        let mut tenuki_messages = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            if let BackendEvent::Log(_, message, _, _) = event {
                tenuki_messages.push(message);
            }
        }

        assert_eq!(
            tenuki_messages,
            vec!["[TENUKI] (0.01s) hello -> dictionary".to_string()]
        );
    }

    #[tokio::test]
    async fn emit_word_log_pairs_emits_dictionary_log_entries_from_model_pairs() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
        let logs = vec![
            LogEvent::PreModelCall {
                original: "ATK+2%".to_string(),
            },
            LogEvent::ModelResult {
                source: "ATK+ZMCZ%".to_string(),
                original: "ATK+2%".to_string(),
                translated: "Attack+ZMCZ%".to_string(),
                elapsed_secs: 0.42,
            },
            LogEvent::Trace {
                message: "ignored".to_string(),
            },
        ];

        emit_word_log_pairs(&event_tx, &logs);
        drop(event_tx);

        assert!(matches!(
            event_rx.try_recv(),
            Ok(BackendEvent::DictionaryLogEntry(_, source, translated))
                if source == "[XUnity] ATK+ZMCZ%"
                    && translated == "[Model] (0.42s) Attack+ZMCZ%"
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn normal_translate_emits_dictionary_new_entry_and_single_word_log() {
        let llm_client: Arc<dyn translator::LlmClient> =
            Arc::new(MockLlmClient::with_responses(&["bonjour"]));
        let (state, mut event_rx) = test_app_state_with_event_rx(llm_client);

        let result = run_pipeline(
            &state,
            "translate",
            PipelineBehavior {
                use_dictionary_lookup: false,
                emit_dictionary_events: true,
                commit_new_entries: false,
                emit_word_logs: true,
                emit_stats: false,
                emit_observations: false,
            },
            vec!["hello".to_string()],
        )
        .await
        .expect("pipeline should succeed");

        assert_eq!(result.translated_text, "bonjour");

        let mut new_entry_events = Vec::new();
        let mut word_log_events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            match event {
                BackendEvent::DictionaryNewEntry(_, source, translated) => {
                    new_entry_events.push((source, translated));
                }
                BackendEvent::DictionaryLogEntry(_, source, translated) => {
                    word_log_events.push((source, translated));
                }
                _ => {}
            }
        }

        assert_eq!(
            new_entry_events,
            vec![("hello".to_string(), "bonjour".to_string())]
        );
        assert_eq!(
            word_log_events,
            vec![(
                "[XUnity] hello".to_string(),
                "[Model] (0.00s) bonjour".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn translate_post_rejects_oversized_raw_body() {
        let state = test_app_state();
        let oversized = "a".repeat(MAX_TRANSLATE_BODY_BYTES + 1);

        let res = translate_post_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::http::HeaderMap::new(),
            axum::body::Bytes::from(oversized),
        )
        .await;

        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "translate request body too large"
        );
    }

    #[tokio::test]
    async fn translate_post_rejects_oversized_parsed_text() {
        let state = test_app_state();
        // body within limit, but parsed text exceeds text limit
        let padding = "a".repeat(MAX_TRANSLATE_TEXT_BYTES + 1);
        let json_body = format!(r#"{{"text":"{}"}}"#, padding);

        let res = translate_post_handler(
            axum::extract::State(Arc::clone(&state)),
            {
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    "application/json".parse().unwrap(),
                );
                headers
            },
            axum::body::Bytes::from(json_body),
        )
        .await;

        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "translate text too large"
        );
    }

    #[tokio::test]
    async fn translate_post_accepts_normal_json_body() {
        let state = test_app_state();

        let res = translate_post_handler(
            axum::extract::State(Arc::clone(&state)),
            {
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    "application/json".parse().unwrap(),
                );
                headers
            },
            axum::body::Bytes::from(r#"{"text":"Hello"}"#),
        )
        .await;

        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn translate_post_returns_400_for_empty_body() {
        let state = test_app_state();
        let res = translate_post_handler(
            axum::extract::State(Arc::clone(&state)),
            axum::http::HeaderMap::new(),
            axum::body::Bytes::new(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn pipeline_join_error_returns_500() {
        let state = test_app_state_with_llm(Arc::new(PanickingLlmClient));
        let res = perform_translation(Arc::clone(&state), "hello".to_string()).await;
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn translate_emits_input_analysis_from_authority_payload() {
        let (state, mut event_rx) =
            test_app_state_with_event_rx(Arc::new(MockLlmClient::with_responses(&["translated"])));
        let res = perform_translation(Arc::clone(&state), "hello".to_string()).await;
        assert_eq!(res.status(), StatusCode::OK);

        let mut snapshot = None;
        while let Ok(event) = event_rx.try_recv() {
            if let BackendEvent::InputAnalysisUpdated(value) = event {
                snapshot = Some(value);
                break;
            }
        }
        let snapshot = snapshot.expect("translation should emit input analysis");

        assert_eq!(snapshot.raw_text, "hello");
        assert_eq!(snapshot.extracted_text, "hello");
        assert_eq!(snapshot.visible_text, "hello");
        assert_eq!(snapshot.model_inputs, vec!["hello".to_string()]);
        assert_eq!(snapshot.final_output.as_deref(), Some("translated"));
        assert_eq!(snapshot.dict_hits, 0);
        assert_eq!(snapshot.model_calls, 1);
    }

    #[test]
    fn cache_value_hit_is_terminal_observation_not_new_entry() {
        let llm = Arc::new(MockLlmClient::with_responses(&["model result"]));
        let llm_client: Arc<dyn translator::LlmClient> = llm.clone();
        let dict_dir = test_dictionary_dir("cache_value_terminal");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));
        let t_cache = Arc::new(TranslationCache::default());
        t_cache.insert("source".to_string(), " Value ".to_string());
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);

        let batch = translate_texts_batch(
            dictionary,
            llm_client,
            Arc::clone(&t_cache),
            event_tx,
            "prefix".to_string(),
            1,
            "en".to_string(),
            test_settings(),
            PipelineBehavior::normal_translate(),
            vec![" Value ".to_string()],
        );

        assert_eq!(batch.texts, vec![" Value ".to_string()]);
        assert_eq!(batch.stats.dict_hits, 1);
        assert_eq!(batch.stats.model_calls, 0);
        assert!(batch.new_entries.is_empty());
        assert!(llm.calls().is_empty());
        assert!(t_cache.get(" Value ").is_none());
        assert!(batch.logs.iter().any(|log| matches!(
            log,
            LogEvent::Trace { message }
                if message.contains(r#""result":"hit_cache_value""#)
                    && message.contains(r#""hit_kind":"value_observation""#)
        )));

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[tokio::test]
    async fn dictionary_value_hit_does_not_commit_or_register() {
        let llm = Arc::new(MockLlmClient::with_responses(&["model result"]));
        let llm_client: Arc<dyn translator::LlmClient> = llm.clone();
        let (state, dict_dir) = test_app_state_with_dictionary_lines(
            "dictionary_value_terminal",
            llm_client,
            &["source= Value "],
        );

        let result = run_pipeline(
            &state,
            "translate",
            PipelineBehavior::normal_translate(),
            vec![" Value ".to_string()],
        )
        .await
        .expect("pipeline should succeed");

        assert_eq!(result.translated_text, " Value ");
        assert!(llm.calls().is_empty());
        assert!(state.n_cache.drain().is_empty());
        assert!(state.t_cache.get(" Value ").is_none());
        assert_eq!(state.dictionary.read().await.lookup_source(" Value "), None);

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[test]
    fn source_hit_precedes_value_observation_hit_for_same_text() {
        let llm = Arc::new(MockLlmClient::with_responses(&["model result"]));
        let llm_client: Arc<dyn translator::LlmClient> = llm.clone();
        let dict_dir = test_dictionary_dir("source_before_value");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));
        let t_cache = Arc::new(TranslationCache::default());
        t_cache.insert("same".to_string(), "source hit".to_string());
        t_cache.insert("other".to_string(), "same".to_string());
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);

        let batch = translate_texts_batch(
            dictionary,
            llm_client,
            Arc::clone(&t_cache),
            event_tx,
            "prefix".to_string(),
            1,
            "en".to_string(),
            test_settings(),
            PipelineBehavior::normal_translate(),
            vec!["same".to_string()],
        );

        assert_eq!(batch.texts, vec!["source hit".to_string()]);
        assert_eq!(batch.stats.dict_hits, 1);
        assert_eq!(batch.stats.model_calls, 0);
        assert!(batch.new_entries.is_empty());
        assert!(llm.calls().is_empty());
        assert!(batch.logs.iter().any(|log| matches!(
            log,
            LogEvent::Trace { message }
                if message.contains(r#""result":"hit_cache_source""#)
        )));
        assert!(!batch.logs.iter().any(|log| matches!(
            log,
            LogEvent::Trace { message }
                if message.contains(r#""result":"hit_cache_value""#)
        )));

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[tokio::test]
    async fn zm_regex_request_returns_200_and_commits_live_regex() {
        let llm_client: Arc<dyn translator::LlmClient> =
            Arc::new(MockLlmClient::with_responses(&["Attack+2%"]));
        let state = test_app_state_with_llm(llm_client);

        let res = perform_translation(Arc::clone(&state), "ATK+ZMCZ%".to_string()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(std::str::from_utf8(&body).unwrap(), "Attack+ZMCZ%");
        assert!(state.t_cache.get("ATK+ZMCZ%").is_none());
        assert_eq!(
            state.dictionary.read().await.lookup_source("ATK+ZMDZ%"),
            Some("Attack+ZMDZ%".to_string())
        );
        assert_eq!(
            state.n_cache.drain(),
            vec![(
                "r:\"^ATK([+＋\\-－−]?Z[A-Z]+Z[%％]?)$\"".to_string(),
                "Attack$1".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn duplicate_regex_live_register_does_not_break_pipeline_response() {
        let dict_dir = test_dictionary_dir("duplicate_regex_http_200");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));
        let llm_client: Arc<dyn translator::LlmClient> =
            Arc::new(InjectingLlmClient::with_responses(
                Arc::clone(&dictionary),
                "^ATK([+＋\\-－−]?Z[A-Z]+Z[%％]?)$",
                "Attack$1",
                &["Attack+2%"],
            ));
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(128);
        let state = Arc::new(AppState {
            dictionary,
            src_lang: Arc::new(RwLock::new("ja".to_string())),
            tgt_lang: Arc::new(RwLock::new("en".to_string())),
            custom_lang_name: Arc::new(RwLock::new(String::new())),
            prompt_template: String::new(),
            background_text: String::new(),
            translation_settings: test_settings(),
            llm_client,
            event_tx,
            t_cache: Arc::new(TranslationCache::default()),
            n_cache: Arc::new(NewEntriesCache::default()),
            input_replay: Arc::new(Mutex::new(InputReplayState::default())),
            llm_slots: 1,
        });

        let result = run_pipeline(
            &state,
            "translate",
            PipelineBehavior {
                use_dictionary_lookup: false,
                emit_dictionary_events: true,
                commit_new_entries: true,
                emit_word_logs: true,
                emit_stats: true,
                emit_observations: true,
            },
            vec!["ATK+ZMCZ%".to_string()],
        )
        .await
        .expect("pipeline should still return response on duplicate live regex");

        assert_eq!(result.translated_text, "Attack+ZMCZ%");
        assert!(state.t_cache.get("ATK+ZMCZ%").is_none());
        assert!(state.n_cache.drain().is_empty());

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[test]
    fn extracts_text_from_form_body() {
        let body = b"text=%E3%83%86%E3%82%B9%E3%83%88";
        assert_eq!(
            extract_translate_post_text(Some("application/x-www-form-urlencoded"), body),
            Some("テスト".to_string())
        );
    }

    #[test]
    fn extracts_content_from_json_body() {
        let body = br#"{"content":"hello"}"#;
        assert_eq!(
            extract_translate_post_text(Some("application/json"), body),
            Some("hello".to_string())
        );
    }

    #[test]
    fn falls_back_to_plain_text_body() {
        let body = b"plain text body";
        assert_eq!(
            extract_translate_post_text(Some("text/plain"), body),
            Some("plain text body".to_string())
        );
    }

    #[test]
    fn plain_text_fallback_preserves_raw_body_edges() {
        let body = b"  plain text body\r\n";
        assert_eq!(
            extract_translate_post_text(Some("text/plain"), body),
            Some("  plain text body\r\n".to_string())
        );
    }

    #[test]
    fn observation_record_includes_snapshot_fields_for_tag_lines() {
        let record = build_observation_record(
            "translate",
            r#"<sprite name="Half-Elf">=Target"#,
            r#"<sprite name="Half-Elf">=Target"#,
            r#"<sprite name="Half-Elf">=Target"#,
            r#"<sprite name="Half-Elf">"#,
            1,
            0,
            &[],
        );

        let json: serde_json::Value =
            serde_json::from_str(&record).expect("record should be valid json");

        assert_eq!(json["route"], "translate");
        assert_eq!(json["raw_line"], r#"<sprite name="Half-Elf">=Target"#);
        assert_eq!(json["extracted_text"], r#"<sprite name="Half-Elf">=Target"#);
        assert_eq!(json["visible_text"], r#"<sprite name="Half-Elf">=Target"#);
        assert_eq!(json["final_output"], r#"<sprite name="Half-Elf">"#);
        assert_eq!(json["dict_hits"], 1);
        assert_eq!(json["model_calls"], 0);
    }

    #[test]
    fn observation_record_includes_plain_lines_too() {
        let record = build_observation_record(
            "list",
            "plain text",
            "plain text",
            "plain text",
            "plain text",
            0,
            1,
            &["plain text".to_string()],
        );

        let json: serde_json::Value =
            serde_json::from_str(&record).expect("record should be valid json");

        assert_eq!(json["route"], "list");
        assert_eq!(json["raw_line"], "plain text");
        assert_eq!(json["extracted_text"], "plain text");
        assert_eq!(json["visible_text"], "plain text");
        assert_eq!(json["final_output"], "plain text");
        assert_eq!(json["dict_hits"], 0);
        assert_eq!(json["model_calls"], 1);
    }

    #[test]
    fn completed_analysis_payload_uses_item_diagnostics() {
        let batch = BatchTranslationOutput {
            stats: TranslationStats {
                dict_hits: 1,
                model_calls: 2,
            },
            item_diagnostics: vec![ItemDiagnostics {
                raw_text: "raw".to_string(),
                extracted_text: "extracted".to_string(),
                visible_text: "visible".to_string(),
                input_preview: "raw".to_string(),
                dict_hits: 1,
                model_calls: 2,
                model_inputs: vec!["model A".to_string(), "model B".to_string()],
            }],
            ..Default::default()
        };

        let payload = build_completed_analysis_payload(&batch, "final").unwrap();

        assert_eq!(payload.raw_text, "raw");
        assert_eq!(payload.extracted_text, "extracted");
        assert_eq!(payload.visible_text, "visible");
        assert_eq!(
            payload.model_inputs,
            vec!["model A".to_string(), "model B".to_string()]
        );
        assert_eq!(payload.final_output, "final");
        assert_eq!(payload.dict_hits, 1);
        assert_eq!(payload.model_calls, 2);
    }

    #[test]
    fn translate_request_record_keeps_raw_and_parsed_text() {
        let record = build_translate_request_record(
            "post_body",
            Some("application/json"),
            r#"{"text":"a\nb"}"#,
            "a\nb",
        );

        let json: serde_json::Value =
            serde_json::from_str(&record).expect("record should be valid json");

        assert_eq!(json["route"], "translate");
        assert_eq!(json["source"], "post_body");
        assert_eq!(json["content_type"], "application/json");
        assert_eq!(json["raw_request"], r#"{"text":"a\nb"}"#);
        assert_eq!(json["parsed_text"], "a\nb");
        assert_eq!(json["line_count"], 2);
    }

    #[test]
    fn list_request_record_keeps_full_request_payload() {
        let request = ListRequest {
            texts: vec!["one".to_string(), "two".to_string()],
        };
        let record = build_list_request_record(&request);

        let json: serde_json::Value =
            serde_json::from_str(&record).expect("record should be valid json");

        assert_eq!(json["route"], "list");
        assert_eq!(json["source"], "json");
        assert_eq!(json["raw_request"]["texts"][0], "one");
        assert_eq!(json["raw_request"]["texts"][1], "two");
        assert_eq!(json["joined_text"], "one\ntwo");
        assert_eq!(json["item_count"], 2);
        assert_eq!(json["total_bytes"], 6);
    }

    #[test]
    fn translate_response_record_keeps_full_response_text() {
        let record = build_translate_response_record("a\nb");

        let json: serde_json::Value =
            serde_json::from_str(&record).expect("record should be valid json");

        assert_eq!(json["kind"], "response");
        assert_eq!(json["route"], "translate");
        assert_eq!(json["response_text"], "a\nb");
        assert_eq!(json["line_count"], 2);
    }

    #[test]
    fn list_response_record_keeps_full_response_text() {
        let record = build_list_response_record("one\ntwo", 2);

        let json: serde_json::Value =
            serde_json::from_str(&record).expect("record should be valid json");

        assert_eq!(json["kind"], "response");
        assert_eq!(json["route"], "list");
        assert_eq!(json["response_text"], "one\ntwo");
        assert_eq!(json["item_count"], 2);
        assert_eq!(json["line_count"], 2);
    }

    #[test]
    fn batch_keeps_zm_entry_when_regex_is_available() {
        let dict_dir = test_dictionary_dir("batch_commit_ascii");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));

        let llm_client: Arc<dyn translator::LlmClient> =
            Arc::new(MockLlmClient::with_responses(&["*2  #3"]));
        let t_cache = Arc::new(TranslationCache::default());
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);

        let batch = translate_texts_batch(
            dictionary,
            llm_client,
            Arc::clone(&t_cache),
            event_tx,
            "prefix".to_string(),
            1,
            "en".to_string(),
            test_settings(),
            PipelineBehavior::normal_translate(),
            vec!["*ZMCZ  #ZMDZ".to_string()],
        );

        assert_eq!(batch.new_entries.len(), 1);
        assert_eq!(batch.new_entries[0].source, "*ZMCZ  #ZMDZ");
        assert!(t_cache.get("*ZMCZ  #ZMDZ").is_none());

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[tokio::test]
    async fn commit_keeps_exact_entry_in_both_caches() {
        let dict_dir = test_dictionary_dir("commit_cache_ascii");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));

        let t_cache = TranslationCache::default();
        let n_cache = NewEntriesCache::default();
        let entries = vec![NewTranslationEntry {
            source: "hello".to_string(),
            translated: "bonjour".to_string(),
            persist: PersistEntry::Exact {
                key: "hello".to_string(),
                value: "bonjour".to_string(),
            },
        }];

        commit_new_entries(&dictionary, &t_cache, &n_cache, &entries).await;

        assert_eq!(
            t_cache.get("hello").map(|value| value.clone()),
            Some("bonjour".to_string())
        );
        assert_eq!(
            n_cache.drain(),
            vec![("hello".to_string(), "bonjour".to_string())]
        );

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[tokio::test]
    async fn commit_regex_registers_live_rule_without_source_t_cache() {
        let dict_dir = test_dictionary_dir("commit_regex_cache");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));

        let t_cache = TranslationCache::default();
        let n_cache = NewEntriesCache::default();
        let pattern = "^ATK([+]Z[A-Z]+Z[%])$".to_string();
        let entries = vec![NewTranslationEntry {
            source: "ATK+ZMCZ%".to_string(),
            translated: "Attack+ZMCZ%".to_string(),
            persist: PersistEntry::Regex {
                pattern: pattern.clone(),
                replacement: "Attack$1".to_string(),
            },
        }];

        commit_new_entries(&dictionary, &t_cache, &n_cache, &entries).await;

        assert!(t_cache.get("ATK+ZMCZ%").is_none());
        assert_eq!(
            dictionary.read().await.lookup_source("ATK+ZMDZ%"),
            Some("Attack+ZMDZ%".to_string())
        );
        assert_eq!(
            n_cache.drain(),
            vec![(format!("r:\"{}\"", pattern), "Attack$1".to_string())]
        );

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[tokio::test]
    async fn commit_regex_duplicate_live_register_does_not_fallback_to_exact_save() {
        let dict_dir = test_dictionary_dir("commit_regex_duplicate");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("Tenuki.dict.txt"),
            dict_dir.join("Tenuki.regex.txt"),
            dict_dir.join("Tenuki.split.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));

        let t_cache = TranslationCache::default();
        let n_cache = NewEntriesCache::default();
        let entry = NewTranslationEntry {
            source: "ATK+ZMCZ%".to_string(),
            translated: "Attack+ZMCZ%".to_string(),
            persist: PersistEntry::Regex {
                pattern: "^ATK([+]Z[A-Z]+Z[%])$".to_string(),
                replacement: "Attack$1".to_string(),
            },
        };

        commit_new_entries(&dictionary, &t_cache, &n_cache, &[entry.clone()]).await;
        let _ = n_cache.drain();

        commit_new_entries(&dictionary, &t_cache, &n_cache, &[entry]).await;

        assert!(t_cache.get("ATK+ZMCZ%").is_none());
        assert!(n_cache.drain().is_empty());

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[tokio::test]
    async fn second_same_session_zm_request_uses_live_regex_not_source_t_cache() {
        let llm = Arc::new(MockLlmClient::with_responses(&["Attack+2%"]));
        let llm_client: Arc<dyn translator::LlmClient> = llm.clone();
        let state = test_app_state_with_llm(llm_client);

        let first = perform_translation(Arc::clone(&state), "ATK+ZMCZ%".to_string()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
        assert_eq!(std::str::from_utf8(&first_body).unwrap(), "Attack+ZMCZ%");
        assert!(state.t_cache.get("ATK+ZMCZ%").is_none());

        let second = perform_translation(Arc::clone(&state), "ATK+ZMCZ%".to_string()).await;
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
        assert_eq!(std::str::from_utf8(&second_body).unwrap(), "Attack+ZMCZ%");
        assert_eq!(llm.calls(), vec!["ATK+2%".to_string()]);
    }
}
