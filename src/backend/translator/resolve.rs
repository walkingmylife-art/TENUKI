use super::clean_model_output;
use super::persist::build_zm_persist_entry;
use super::types::{
    NewTranslationEntry, PersistEntry, PlannedDocument, PlannedNode, ResolvedDocument,
    ResolvedFragmentNode, ResolvedLine, ResolvedNode, ResolvedSegment, TranslationResult,
    TranslationSettings,
};
use super::zm::{build_zm_number_mapping, restore_zm_number_tokens};
use super::LlmClient;

pub(super) fn dedupe_entries(entries: Vec<NewTranslationEntry>) -> Vec<NewTranslationEntry> {
    let mut seen = rustc_hash::FxHashSet::default();
    entries
        .into_iter()
        .filter(|entry| seen.insert(entry.source.clone()))
        .collect()
}

pub(super) fn resolve_document<F>(
    document: &PlannedDocument,
    lookup: &F,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> (ResolvedDocument, TranslationResult)
where
    F: Fn(&str) -> Option<String>,
{
    let mut lines = Vec::new();
    let mut accumulated = TranslationResult::empty(String::new());

    for line in &document.lines {
        let mut resolved_segments = Vec::new();

        for segment in &line.segments {
            let mut nodes = Vec::new();

            for node in &segment.nodes {
                match node {
                    PlannedNode::Surface(surface) => {
                        nodes.push(ResolvedNode::Surface(surface.clone()));
                    }
                    PlannedNode::Fragment(fragment) => {
                        let child = translate_fragment(
                            &fragment.authority.source,
                            lookup,
                            prefix,
                            tgt_lang,
                            llm_client,
                            settings,
                        );

                        nodes.push(ResolvedNode::Fragment(ResolvedFragmentNode {
                            authority: fragment.authority.clone(),
                            text: child.text.clone(),
                        }));

                        accumulated.absorb(child);
                    }
                }
            }

            resolved_segments.push(ResolvedSegment {
                nodes,
                trailing_separator: segment.trailing_separator.clone(),
            });
        }

        lines.push(ResolvedLine {
            segments: resolved_segments,
            trailing_newline: line.trailing_newline.clone(),
        });
    }

    (ResolvedDocument { lines }, accumulated)
}

pub(super) fn translate_model_only(
    text: &str,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult {
    if text.trim().is_empty() {
        return TranslationResult::empty(text.to_string());
    }

    let zm_mapping = build_zm_number_mapping(text);
    let model_input = zm_mapping
        .as_ref()
        .map_or(text, |mapping| mapping.sent_text.as_str());
    let start = std::time::Instant::now();

    if let Some(translated_raw) = llm_client.translate_sync(model_input, prefix) {
        let elapsed = start.elapsed();
        let cleaned = clean_model_output(
            model_input,
            &translated_raw,
            tgt_lang,
            settings.enable_model_symbol_cleanup,
        );
        let translated = match &zm_mapping {
            Some(mapping) => restore_zm_number_tokens(&cleaned, mapping),
            None => cleaned.clone(),
        };

        let mut result = TranslationResult::from_model_call_success(
            translated.clone(),
            text,
            model_input,
            elapsed,
        );

        if zm_mapping.is_some() {
            log::info!(
                "[PERSIST] zm_candidate source=\"{}\" model_input=\"{}\" cleaned=\"{}\" restored=\"{}\"",
                text, model_input, cleaned, translated
            );
        }

        let value = translated.trim().to_string();
        if value.is_empty() {
            log::info!(
                "[PERSIST] new_entry_skipped source=\"{}\" reason=value_empty",
                text
            );
            return result;
        }

        let persist = match &zm_mapping {
            Some(mapping) => build_zm_persist_entry(text, mapping, &cleaned, &translated),
            None => Some(PersistEntry::Exact {
                key: text.to_string(),
                value: value.clone(),
            }),
        };

        match persist {
            Some(PersistEntry::Regex {
                pattern,
                replacement,
            }) => {
                log::info!(
                    "[PERSIST] new_entry_created source=\"{}\" translated=\"{}\" persist=Regex",
                    text,
                    value
                );
                result.new_entries.push(NewTranslationEntry {
                    source: text.to_string(),
                    translated: value,
                    persist: PersistEntry::Regex {
                        pattern,
                        replacement,
                    },
                });
            }
            Some(PersistEntry::Exact {
                key,
                value: persist_value,
            }) => {
                log::info!(
                    "[PERSIST] new_entry_created source=\"{}\" translated=\"{}\" persist=Exact",
                    text,
                    persist_value
                );
                result.new_entries.push(NewTranslationEntry {
                    source: text.to_string(),
                    translated: value,
                    persist: PersistEntry::Exact {
                        key,
                        value: persist_value,
                    },
                });
            }
            None => {
                log::info!(
                    "[PERSIST] new_entry_skipped source=\"{}\" reason=zm_regex_unavailable",
                    text
                );
            }
        }

        result
    } else {
        TranslationResult::from_model_call_failure(model_input)
    }
}

pub(super) fn translate_fragment<F>(
    fragment: &str,
    lookup: &F,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult
where
    F: Fn(&str) -> Option<String>,
{
    // Contract:
    // `fragment` is the dictionary key authority for this call.
    // Do not derive lookup/register keys from model output, restored text, or render surface.
    // ZM numeric replacement is model transport only and stays inside translate_model_only().
    if fragment.trim().is_empty() {
        return TranslationResult::empty(fragment.to_string());
    }

    let start = std::time::Instant::now();
    if let Some(hit) = lookup(fragment) {
        return TranslationResult::from_dict_hit(hit, fragment, start.elapsed());
    }

    translate_model_only(fragment, prefix, tgt_lang, llm_client, settings)
}
