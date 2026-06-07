//! 観測・診断・レコード構築

use crate::backend::analysis::CompletedAnalysisPayload;
use crate::messages::BackendEvent;

use super::{AppState, BatchTranslationOutput, ItemDiagnostics};

pub(super) fn emit_word_log_pairs(event_tx: &tokio::sync::mpsc::Sender<BackendEvent>, logs: &[crate::backend::translator::LogEvent]) {
    for log in logs {
        match log {
            crate::backend::translator::LogEvent::ModelResult {
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

pub(super) fn preview_text(text: &str) -> String {
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

pub(super) fn preview_body(text: &str) -> String {
    preview_text(text)
}

pub(super) fn emit_batch_diagnostics(state: &AppState, route: &str, batch: &BatchTranslationOutput) {
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

pub(super) fn build_translate_request_record(
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

pub(super) fn build_list_request_record(request: &super::ListRequest) -> String {
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

pub(super) fn build_translate_response_record(translated_text: &str) -> String {
    serde_json::json!({
        "kind": "response",
        "route": "translate",
        "response_text": translated_text,
        "line_count": translated_text.split('\n').count(),
    })
    .to_string()
}

pub(super) fn build_list_response_record(translated_text: &str, item_count: usize) -> String {
    serde_json::json!({
        "kind": "response",
        "route": "list",
        "response_text": translated_text,
        "item_count": item_count,
        "line_count": translated_text.split('\n').count(),
    })
    .to_string()
}

pub(super) fn total_list_request_bytes(texts: &[String]) -> usize {
    texts.iter().map(|text| text.len()).sum()
}

pub(super) fn model_inputs_from_logs(logs: &[crate::backend::translator::LogEvent]) -> Vec<String> {
    logs.iter()
        .filter_map(|event| match event {
            crate::backend::translator::LogEvent::PreModelCall { original } => Some(original.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn build_observation_record(
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

pub(super) fn emit_observation_logs(
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

pub(super) fn build_completed_analysis_payload(
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
