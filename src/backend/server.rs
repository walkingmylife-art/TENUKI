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
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::sync::RwLock;

use crate::backend::analysis::{self, SharedInputReplayState};
use crate::backend::dictionary::Dictionary;
use crate::backend::logger::{LogEvent as PersistentLogEvent, LOG_TX};
use crate::backend::processor::TextProcessor;
use crate::backend::translator::{
    self, LogEvent, NewEntriesCache, TranslationCache, TranslationResult, TranslationSettings,
    TranslationStats,
};
use crate::messages::{BackendEvent, LogLevel, LogSource};

static OBSERVED_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<[^>\r\n]+>|＜[^＞\r\n]+＞").expect("tag regex"));
static OBSERVED_MARKER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Z[A-Z]+Z").expect("marker regex"));

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

// ============================================================
// Application state
// ============================================================

#[derive(Clone)]
pub struct AppState {
    pub dictionary: Arc<RwLock<Dictionary>>,
    pub processor: Arc<dyn TextProcessor>,
    pub src_lang: Arc<RwLock<String>>,
    pub tgt_lang: Arc<RwLock<String>>,
    pub custom_lang_name: Arc<RwLock<String>>,
    pub prompt_template: String,
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
        let src = self.src_lang.read().await;
        let tgt = self.tgt_lang.read().await;
        let custom_name = self.custom_lang_name.read().await;
        translator::build_lang_prefix(&src, &tgt, &custom_name, &self.prompt_template)
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
        let (msg, level) = match event {
            LogEvent::DictHit {
                elapsed_secs,
                original,
                translated,
            } => (
                format!(
                    "[TENUKI] ({:.2}s) {} -> {}",
                    elapsed_secs, original, translated
                ),
                LogLevel::Success,
            ),
            LogEvent::PreModelCall { original } => {
                (format!("[XUnity] {}", original), LogLevel::Info)
            }
            LogEvent::ModelResult {
                elapsed_secs,
                translated,
                ..
            } => (
                format!("[Model] ({:.2}s) {}", elapsed_secs, translated),
                LogLevel::Info,
            ),
            _ => return,
        };
        self.emit_persistent_log(msg, level);
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
        crate::backend::logger::write_observation(message.clone());
        if crate::backend::logger::debug_logs_enabled() {
            self.emit_persistent_log(format!("[OBSERVE] {}", message), LogLevel::Info);
        }
    }

    fn emit_request_log(&self, message: String) {
        crate::backend::logger::write_request(message.clone());
        if crate::backend::logger::debug_logs_enabled() {
            self.emit_persistent_log(format!("[REQUEST] {}", message), LogLevel::Info);
        }
    }
}

fn dictionary_log_display_pair(key: &str, value: &str, logs: &[LogEvent]) -> (String, String) {
    let trimmed_key = key.trim();

    if let Some((translated, elapsed_secs)) = logs.iter().find_map(|event| match event {
        LogEvent::ModelResult {
            original,
            translated,
            elapsed_secs,
        } if original.trim() == trimmed_key => Some((translated.clone(), *elapsed_secs)),
        _ => None,
    }) {
        return (
            format!("[XUnity] {}", key),
            format!("[Model] ({:.2}s) {}", elapsed_secs, translated),
        );
    }

    if let Some((translated, elapsed_secs)) = logs.iter().find_map(|event| match event {
        LogEvent::DictHit {
            original,
            translated,
            elapsed_secs,
        } if original.trim() == trimmed_key => Some((translated.clone(), *elapsed_secs)),
        _ => None,
    }) {
        return (
            format!("[XUnity] {}", key),
            format!("[TENUKI] ({:.2}s) {}", elapsed_secs, translated),
        );
    }

    (key.to_string(), value.to_string())
}

#[derive(Debug, Clone)]
struct ItemDiagnostics {
    raw_text: String,
    input_preview: String,
    dict_hits: usize,
    model_calls: usize,
    model_inputs: Vec<String>,
}

#[derive(Default)]
struct BatchTranslationOutput {
    texts: Vec<String>,
    new_entries: Vec<(String, String)>,
    stats: TranslationStats,
    logs: Vec<LogEvent>,
    item_diagnostics: Vec<ItemDiagnostics>,
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

fn contains_observed_markup(text: &str) -> bool {
    OBSERVED_TAG_RE.is_match(text) || OBSERVED_MARKER_RE.is_match(text)
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
    final_output: &str,
    dict_hits: usize,
    model_calls: usize,
    model_inputs: &[String],
) -> String {
    serde_json::json!({
        "route": route,
        "raw_line": raw_line,
        "extracted_text": raw_line,
        "visible_text": raw_line,
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
            translated,
            diagnostic.dict_hits,
            diagnostic.model_calls,
            &diagnostic.model_inputs,
        );
        state.emit_observation(record);
    }
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

    let body_text = std::str::from_utf8(body).ok()?.trim().to_string();
    if body_text.is_empty() {
        return None;
    }

    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();

    if content_type.contains("application/json") || body_text.starts_with('{') {
        if let Ok(request) = serde_json::from_str::<TranslateRequest>(&body_text) {
            if let Some(text) = request.text.or(request.content) {
                return Some(text);
            }
        }
    }

    if content_type.contains("application/x-www-form-urlencoded")
        || body_text.starts_with("text=")
        || body_text.starts_with("content=")
        || body_text.contains("&text=")
        || body_text.contains("&content=")
    {
        if let Some(text) = parse_form_text(&body_text) {
            return Some(text);
        }
    }

    Some(body_text)
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
    texts: Vec<String>,
) -> BatchTranslationOutput {
    fn translate_one_text(
        dictionary: &Arc<RwLock<Dictionary>>,
        llm_client: &Arc<dyn translator::LlmClient>,
        t_cache: &Arc<TranslationCache>,
        prefix: &str,
        tgt_lang: &str,
        settings: TranslationSettings,
        text: &str,
    ) -> (TranslationResult, ItemDiagnostics) {
        let lookup = |key: &str| -> Option<String> {
            t_cache
                .get(key)
                .map(|value| value.clone())
                .or_else(|| dictionary.blocking_read().lookup(key))
        };

        let result = translator::translate_chunk(
            text,
            lookup,
            prefix,
            tgt_lang,
            llm_client.as_ref(),
            settings,
        );

        let diagnostics = ItemDiagnostics {
            raw_text: text.to_string(),
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
        for (key, value) in &result.new_entries {
            let (display_key, display_value) =
                dictionary_log_display_pair(key, value, &result.logs);
            let _ = event_tx.try_send(BackendEvent::DictionaryLogEntry(
                crate::messages::current_timestamp(),
                display_key,
                display_value,
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
                &text,
            );

            emit_new_entries(&result, &event_tx);
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
                    &text,
                );

                let mut results = results.lock().expect("results mutex poisoned");
                results[index] = Some(result);
            });
        }
    });

    let results = results.lock().expect("results mutex poisoned");
    for (result, diagnostics) in results.iter().flatten() {
        emit_new_entries(result, &event_tx);
        output.logs.extend(result.logs.clone());
        output.new_entries.extend(result.new_entries.clone());
        output.stats.merge(&result.stats);
        output.texts.push(result.text.clone());
        output.item_diagnostics.push(diagnostics.clone());
    }

    output
}

fn commit_new_entries(
    t_cache: &TranslationCache,
    n_cache: &NewEntriesCache,
    entries: &[(String, String)],
) {
    for (key, value) in entries {
        t_cache.insert(key.clone(), value.clone());
        n_cache.insert(key.clone(), value.clone());
    }
}

// ============================================================
// Translation entry point for a single text

async fn perform_translation(state: Arc<AppState>, text: String) -> Response {
    if text.is_empty() {
        return "no text".into_response();
    }

    let text = text.replace("\r\n", "\n");
    let prefix = state.current_prefix().await;
    let llm_client = state.llm_client.clone();
    let dictionary = state.dictionary.clone();
    let t_cache = state.t_cache.clone();
    let event_tx = state.event_tx.clone();
    let llm_slots = state.llm_slots;
    let tgt_lang = state.tgt_lang.read().await.clone();
    let settings = state.translation_settings;

    let texts = vec![text.clone()];
    let batch = tokio::task::spawn_blocking(move || {
        translate_texts_batch(
            dictionary, llm_client, t_cache, event_tx, prefix, llm_slots, tgt_lang, settings, texts,
        )
    })
    .await
    .unwrap();

    commit_new_entries(
        state.t_cache.as_ref(),
        state.n_cache.as_ref(),
        &batch.new_entries,
    );
    for log in &batch.logs {
        state.emit_log(log);
    }
    state.emit_stats(&batch.stats);
    emit_batch_diagnostics(state.as_ref(), "translate", &batch);
    emit_observation_logs(
        state.as_ref(),
        "translate",
        &batch.texts,
        &batch.item_diagnostics,
    );
    let translated_text = batch.texts.join("\n");
    state.emit_request_log(build_translate_response_record(&translated_text));
    let snapshot = analysis::record_completed_translation(
        &state.input_replay,
        &text,
        &translated_text,
        state.processor.as_ref(),
        batch.stats.dict_hits,
        batch.stats.model_calls,
    );
    let _ = state
        .event_tx
        .try_send(BackendEvent::InputAnalysisUpdated(snapshot));
    translated_text.into_response()
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
    perform_translation(state, text).await
}

pub async fn translate_post_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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

    state.emit_request_log(build_list_request_record(&request));

    let prefix = state.current_prefix().await;
    let llm_client = state.llm_client.clone();
    let dictionary = state.dictionary.clone();
    let t_cache = state.t_cache.clone();
    let event_tx = state.event_tx.clone();
    let llm_slots = state.llm_slots;
    let tgt_lang = state.tgt_lang.read().await.clone();
    let settings = state.translation_settings;
    let texts = request.texts;
    let raw_text = texts.join("\n");
    let batch = tokio::task::spawn_blocking(move || {
        translate_texts_batch(
            dictionary, llm_client, t_cache, event_tx, prefix, llm_slots, tgt_lang, settings, texts,
        )
    })
    .await
    .unwrap();

    commit_new_entries(
        state.t_cache.as_ref(),
        state.n_cache.as_ref(),
        &batch.new_entries,
    );
    for log in &batch.logs {
        state.emit_log(log);
    }
    state.emit_stats(&batch.stats);
    emit_batch_diagnostics(state.as_ref(), "list", &batch);
    emit_observation_logs(
        state.as_ref(),
        "list",
        &batch.texts,
        &batch.item_diagnostics,
    );
    let translated_text = batch.texts.join("\n");
    state.emit_request_log(build_list_response_record(
        &translated_text,
        batch.texts.len(),
    ));
    let snapshot = analysis::record_completed_translation(
        &state.input_replay,
        &raw_text,
        &translated_text,
        state.processor.as_ref(),
        batch.stats.dict_hits,
        batch.stats.model_calls,
    );
    let _ = state
        .event_tx
        .try_send(BackendEvent::InputAnalysisUpdated(snapshot));
    translated_text.into_response()
}

async fn shutdown_handler(State(state): State<Arc<AppState>>) -> &'static str {
    // n_cache -> dict.register -> flush_buffer
    if !state.n_cache.is_empty() {
        let entries = state.n_cache.drain();
        let mut dict = state.dictionary.write().await;
        for (k, v) in &entries {
            dict.register(&k, &v);
        }
        let _ = dict.flush_buffer();
        let _ = state.event_tx.try_send(BackendEvent::Log(
            LogSource::Tenuki,
            format!(
                "Shutdown: {} new entries flushed to dictionary",
                entries.len()
            ),
            LogLevel::Info,
            crate::messages::current_timestamp(),
        ));
    }
    let _ = state.event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        "Shutdown request received".to_string(),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));
    "ok"
}

// ============================================================
// Server startup
// Caller owns task spawning and shutdown coordination.
// ============================================================

pub async fn run_translation_server(
    host: String,
    port: u16,
    dictionary: Arc<RwLock<Dictionary>>,
    processor: Arc<dyn TextProcessor>,
    src_lang: String,
    tgt_lang: String,
    custom_lang_name: String,
    prompt_template: String,
    translation_settings: TranslationSettings,
    llm_client: Arc<dyn translator::LlmClient>,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
    startup_tx: oneshot::Sender<Result<(), String>>,
    shutdown_rx: oneshot::Receiver<()>,
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
        processor,
        src_lang: Arc::new(RwLock::new(src_lang)),
        tgt_lang: Arc::new(RwLock::new(tgt_lang)),
        custom_lang_name: Arc::new(RwLock::new(custom_lang_name)),
        prompt_template,
        translation_settings,
        llm_client,
        event_tx: event_tx.clone(),
        t_cache,
        n_cache,
        input_replay,
        llm_slots: llm_slots.max(1),
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
        build_list_request_record, build_list_response_record, build_observation_record,
        build_translate_request_record, build_translate_response_record, commit_new_entries,
        contains_observed_markup, extract_translate_post_text, translate_texts_batch, ListRequest,
    };
    use crate::backend::analysis::find_unescaped_assignment_separator;
    use crate::backend::dictionary::Dictionary;
    use crate::backend::translator::{
        self, NewEntriesCache, TranslationCache, TranslationSettings,
    };
    use crate::messages::BackendEvent;
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

    fn test_settings() -> TranslationSettings {
        TranslationSettings {
            enable_model_wrap: true,
            model_wrap_min_chars: 60,
            model_wrap_min_tail_chars: 10,
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
    fn finds_first_unescaped_assignment_separator() {
        let text = r#"[World6.12.24]Rumor says the guild attacked <color\=#2779FA>target</color>.=[Other6.12.24]broken target"#;
        let index = find_unescaped_assignment_separator(text).unwrap();

        assert_eq!(
            &text[..index],
            r#"[World6.12.24]Rumor says the guild attacked <color\=#2779FA>target</color>."#
        );
        assert_eq!(&text[index + 1..], "[Other6.12.24]broken target");
    }

    #[test]
    fn returns_none_without_assignment_separator() {
        let text = r#"[World6.12.24]Rumor says the guild attacked <color\=#2779FA>target</color>."#;
        assert_eq!(find_unescaped_assignment_separator(text), None);
    }

    #[test]
    fn returns_none_for_operator_style_ui_text() {
        let text = ">>=Move Distance";
        assert_eq!(find_unescaped_assignment_separator(text), None);
    }

    #[test]
    fn returns_none_for_equals_inside_sprite_tag() {
        let text = r#"<sprite name="Half-Elf">"#;
        assert_eq!(find_unescaped_assignment_separator(text), None);
    }

    #[test]
    fn detects_tag_or_marker_lines_for_observation() {
        assert!(contains_observed_markup("<b>Attack+150%</b>"));
        assert!(contains_observed_markup("marker ZMDZ"));
        assert!(!contains_observed_markup("plain text only"));
    }

    #[test]
    fn observation_record_includes_snapshot_fields_for_tag_lines() {
        let record = build_observation_record(
            "translate",
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
    fn batch_keeps_translator_new_entries_until_commit_ascii() {
        let dict_dir = test_dictionary_dir("batch_commit_ascii");
        let (dict_tx, _dict_rx) = std::sync::mpsc::channel::<BackendEvent>();
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            dict_dir.join("txt_root"),
            dict_dir.join("dict.txt"),
            dict_dir.join("dict.bin"),
            dict_tx,
        )));

        let llm_client: Arc<dyn translator::LlmClient> =
            Arc::new(MockLlmClient::with_responses(&["*1  #2"]));
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
            vec!["*ZMCZ  #ZMDZ".to_string()],
        );

        assert_eq!(
            batch.new_entries,
            vec![("*ZMCZ  #ZMDZ".to_string(), "*ZMCZ  #ZMDZ".to_string())]
        );
        assert!(t_cache.get("*ZMCZ  #ZMDZ").is_none());

        let _ = std::fs::remove_dir_all(dict_dir);
    }

    #[test]
    fn commit_keeps_translator_key_in_both_caches_ascii() {
        let t_cache = TranslationCache::default();
        let n_cache = NewEntriesCache::default();
        let entries = vec![("*ZMCZ  #ZMDZ".to_string(), "*ZMCZ  #ZMDZ".to_string())];

        commit_new_entries(&t_cache, &n_cache, &entries);

        assert_eq!(
            t_cache.get("*ZMCZ  #ZMDZ").map(|value| value.clone()),
            Some("*ZMCZ  #ZMDZ".to_string())
        );
        assert_eq!(n_cache.drain(), entries);
    }
}
