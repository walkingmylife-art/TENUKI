//! Batch translation engine: translates multiple texts with LLM+dictionary lookups.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;

use tokio::sync::RwLock;

use crate::backend::dictionary::{Dictionary, SplitResult};
use crate::backend::translator::{
    self, LlmClient, LogEvent, NewTranslationEntry, TranslationCache, TranslationResult,
    TranslationSettings, TranslationStats,
};
use crate::messages::BackendEvent;

use super::observation::{model_inputs_from_logs, preview_text};
use super::pipeline::PipelineBehavior;

#[derive(Debug, Clone)]
pub(super) struct ItemDiagnostics {
    pub(super) raw_text: String,
    pub(super) extracted_text: String,
    pub(super) visible_text: String,
    pub(super) input_preview: String,
    pub(super) dict_hits: usize,
    pub(super) model_calls: usize,
    pub(super) model_inputs: Vec<String>,
}

#[derive(Default)]
pub(super) struct BatchTranslationOutput {
    pub(super) texts: Vec<String>,
    pub(super) new_entries: Vec<NewTranslationEntry>,
    pub(super) stats: TranslationStats,
    pub(super) logs: Vec<LogEvent>,
    pub(super) item_diagnostics: Vec<ItemDiagnostics>,
}

pub(super) fn translate_texts_batch(
    dictionary: Arc<RwLock<Dictionary>>,
    llm_client: Arc<dyn LlmClient>,
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
        llm_client: &Arc<dyn LlmClient>,
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
                        "hit_value": preview_text(&value),
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
                        "hit_value": preview_text(&value),
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
                        "hit_value": preview_text(&value),
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
                        "hit_value": preview_text(&value),
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
