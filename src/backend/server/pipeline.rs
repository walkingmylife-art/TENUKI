//! Translation pipeline: batch execution, commit, and side-effects.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::backend::dictionary::Dictionary;
use crate::backend::translator::{
    LogEvent, NewEntriesCache, NewTranslationEntry, PersistEntry, TranslationCache,
};

use super::observation::{
    build_completed_analysis_payload, emit_batch_diagnostics, emit_observation_logs,
    emit_word_log_pairs,
};
use super::{translate_texts_batch, AppState};

#[derive(Clone, Copy)]
pub(super) struct PipelineBehavior {
    pub(super) use_dictionary_lookup: bool,
    pub(super) emit_dictionary_events: bool,
    pub(super) commit_new_entries: bool,
    pub(super) emit_word_logs: bool,
    pub(super) emit_stats: bool,
    pub(super) emit_observations: bool,
}

impl PipelineBehavior {
    pub(super) fn normal_translate() -> Self {
        Self {
            use_dictionary_lookup: true,
            emit_dictionary_events: true,
            commit_new_entries: true,
            emit_word_logs: true,
            emit_stats: true,
            emit_observations: true,
        }
    }

    pub(super) fn list_mode() -> Self {
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

pub(super) struct PipelineResult {
    pub(super) texts: Vec<String>,
    pub(super) translated_text: String,
    pub(super) item_count: usize,
    pub(super) analysis_payload: Option<
        crate::backend::analysis::CompletedAnalysisPayload,
    >,
}

#[derive(Default)]
pub(super) struct CommitSummary {
    regex_registered: usize,
    regex_skipped: usize,
    exact_committed: usize,
}

pub(super) async fn commit_new_entries(
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
                    let key = format!("r:\"{}\"", pattern);
                    n_cache.insert(key.clone(), replacement.clone());
                    log::info!(
                        "[COMMIT] regex_queued_save key='{}' value=\"{}\"",
                        key,
                        replacement
                    );
                    summary.regex_registered += 1;
                } else {
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

pub(super) async fn run_pipeline(
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
            dictionary, llm_client, t_cache, event_tx, prefix, llm_slots, tgt_lang,
            settings, behavior, texts,
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
