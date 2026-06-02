//! Translation coordinator.

mod cache;
mod client;
mod helpers;
mod lang;
mod persist;
mod plan;
mod render;
mod resolve;
mod structure;
mod types;
mod zm;

pub use cache::{NewEntriesCache, TranslationCache};
pub use client::{HttpLlmClient, LlmClient};
use crate::backend::dictionary::SplitResult;
pub use helpers::clean_model_output;
pub use lang::{build_lang_prefix, fallback_prefix};
pub use types::{
    LogEvent, NewTranslationEntry, PersistEntry, TranslationResult, TranslationSettings,
    TranslationStats,
};

#[cfg(test)]
use persist::build_zm_persist_entry;
#[cfg(test)]
use render::{render_atoms, wrap_render_atoms, RenderAtom};
#[cfg(test)]
use zm::{build_zm_number_mapping, restore_zm_number_tokens, ZmNumberMapping, ZmReplacement};

pub fn translate_chunk<F, S>(
    chunk: &str,
    lookup: F,
    lookup_split: S,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult
where
    F: Fn(&str) -> Option<String> + Clone,
    S: Fn(&str) -> Option<SplitResult> + Clone,
{
    if chunk.trim().is_empty() {
        return TranslationResult::empty(chunk.to_string());
    }

    let plan = plan::plan_document(chunk);
    let (resolved, mut result) =
        resolve::resolve_document(&plan, &lookup, &lookup_split, prefix, tgt_lang, llm_client, settings);

    result.text = render::render_document(&resolved, settings);
    result.new_entries = resolve::dedupe_entries(std::mem::take(&mut result.new_entries));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use std::sync::Mutex;

    fn no_split(_key: &str) -> Option<SplitResult> {
        None
    }

    fn translate_chunk<F>(
        chunk: &str,
        lookup: F,
        prefix: &str,
        tgt_lang: &str,
        llm_client: &dyn LlmClient,
        settings: TranslationSettings,
    ) -> TranslationResult
    where
        F: Fn(&str) -> Option<String> + Clone,
    {
        super::translate_chunk(chunk, lookup, no_split, prefix, tgt_lang, llm_client, settings)
    }

    #[derive(Default)]
    struct MockLlmClient {
        calls: Mutex<Vec<String>>,
        responses: Mutex<Vec<String>>,
    }

    impl MockLlmClient {
        fn with_responses(values: &[&str]) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(values.iter().map(|value| value.to_string()).collect()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl LlmClient for MockLlmClient {
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
            model_wrap_min_chars: 80,
            model_wrap_space_fallback_min_chars: 100,
            enable_model_symbol_cleanup: true,
        }
    }

    fn entry(key: &str, value: &str) -> NewTranslationEntry {
        NewTranslationEntry {
            source: key.to_string(),
            translated: value.to_string(),
            persist: PersistEntry::Exact {
                key: key.to_string(),
                value: value.to_string(),
            },
        }
    }

    fn mapping(pairs: &[(&str, &str)]) -> ZmNumberMapping {
        ZmNumberMapping {
            sent_text: String::new(),
            replacements: pairs
                .iter()
                .map(|(number, marker)| ZmReplacement {
                    number: number.to_string(),
                    marker: marker.to_string(),
                    trim_trailing_minus: false,
                    transport_wrapped: false,
                    transport_left_space: false,
                    transport_right_space: false,
                    source_span: marker.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn restore_does_not_match_digit_prefix() {
        let mapping = mapping(&[("1", "ZAZ"), ("10", "ZBZ")]);
        assert_eq!(restore_zm_number_tokens("1 10", &mapping), "ZAZ ZBZ");
    }

    #[test]
    fn restore_preserves_spaces_as_is() {
        let mapping = mapping(&[("1", "ZAZ")]);
        assert_eq!(
            restore_zm_number_tokens("foo 1 bar", &mapping),
            "foo ZAZ bar"
        );
        assert_eq!(
            restore_zm_number_tokens("foo  1  bar", &mapping),
            "foo  ZAZ  bar"
        );
        assert_eq!(restore_zm_number_tokens("foo1bar", &mapping), "fooZAZbar");
    }

    #[test]
    fn bare_zm_with_spaces_uses_wrapped_transport_number() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(mapping.sent_text, "foo 「2」 bar");
        assert!(mapping.replacements[0].transport_wrapped);
        assert!(mapping.replacements[0].transport_left_space);
        assert!(mapping.replacements[0].transport_right_space);
        assert_eq!(mapping.replacements[0].source_span, "ZMCZ");
    }

    #[test]
    fn bare_zm_at_start_uses_wrapped_transport_number() {
        let mapping = build_zm_number_mapping("ZMCZ bar").unwrap();
        assert_eq!(mapping.sent_text, "「2」 bar");
        assert!(mapping.replacements[0].transport_wrapped);
        assert!(!mapping.replacements[0].transport_left_space);
        assert!(mapping.replacements[0].transport_right_space);
        assert_eq!(mapping.replacements[0].source_span, "ZMCZ");
    }

    #[test]
    fn bare_zm_at_end_uses_wrapped_transport_number() {
        let mapping = build_zm_number_mapping("foo ZMCZ").unwrap();
        assert_eq!(mapping.sent_text, "foo 「2」");
        assert!(mapping.replacements[0].transport_wrapped);
        assert!(mapping.replacements[0].transport_left_space);
        assert!(!mapping.replacements[0].transport_right_space);
        assert_eq!(mapping.replacements[0].source_span, "ZMCZ");
    }

    #[test]
    fn adjacent_text_zm_stays_unwrapped() {
        let mapping = build_zm_number_mapping("fooZMCZbar").unwrap();
        assert_eq!(mapping.sent_text, "foo2bar");
        assert!(!mapping.replacements[0].transport_wrapped);
    }

    #[test]
    fn restore_removes_transport_wrapper_for_middle_bare_zm() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo 2 bar", &mapping),
            "foo ZMCZ bar"
        );
    }

    #[test]
    fn restore_removes_exact_transport_wrapper_for_middle_bare_zm() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo 「2」 bar", &mapping),
            "foo ZMCZ bar"
        );
    }

    #[test]
    fn restore_replaces_variant_wrapper_for_middle_bare_zm() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo （2） bar", &mapping),
            "foo ZMCZ bar"
        );
        assert_eq!(
            restore_zm_number_tokens("foo ”2” bar", &mapping),
            "foo ZMCZ bar"
        );
    }

    #[test]
    fn restore_keeps_inner_space_wrapper_and_only_replaces_number() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo 「 2 」 bar", &mapping),
            "foo 「 ZMCZ 」 bar"
        );
        assert_eq!(
            restore_zm_number_tokens("foo 「　2　」 bar", &mapping),
            "foo 「　ZMCZ　」 bar"
        );
    }

    #[test]
    fn restore_removes_transport_wrapper_for_start_bare_zm() {
        let mapping = build_zm_number_mapping("ZMCZ bar").unwrap();
        assert_eq!(restore_zm_number_tokens("2 bar", &mapping), "ZMCZ bar");
    }

    #[test]
    fn restore_removes_variant_wrapper_for_start_bare_zm() {
        let mapping = build_zm_number_mapping("ZMCZ bar").unwrap();
        assert_eq!(restore_zm_number_tokens("（2）bar", &mapping), "ZMCZ bar");
    }

    #[test]
    fn restore_removes_transport_wrapper_for_end_bare_zm() {
        let mapping = build_zm_number_mapping("foo ZMCZ").unwrap();
        assert_eq!(restore_zm_number_tokens("foo 2", &mapping), "foo ZMCZ");
    }

    #[test]
    fn restore_removes_variant_wrapper_for_end_bare_zm() {
        let mapping = build_zm_number_mapping("foo ZMCZ").unwrap();
        assert_eq!(restore_zm_number_tokens("foo（2）", &mapping), "foo ZMCZ");
    }

    #[test]
    fn restore_does_not_touch_number_not_in_mapping() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo 99 bar", &mapping),
            "foo 99 bar"
        );
    }

    #[test]
    fn restore_only_consumes_first_matching_number() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo 2 and 2 bar", &mapping),
            "foo ZMCZ and 2 bar"
        );
    }

    #[test]
    fn restore_does_not_eat_text_adjacent_to_transport_number() {
        let mapping = build_zm_number_mapping("foo ZMCZ bar").unwrap();
        assert_eq!(
            restore_zm_number_tokens("foo は2のダメージ bar", &mapping),
            "foo はZMCZのダメージ bar"
        );
    }

    #[test]
    fn restore_does_not_treat_signed_percent_adjacent_text_as_wrapper() {
        let mapping = build_zm_number_mapping("ZMCZ +ZMDZ%").unwrap();
        assert_eq!(restore_zm_number_tokens("2+3%", &mapping), "ZMCZ +ZMDZ%");
    }

    #[test]
    fn signed_percent_zm_stays_unwrapped() {
        let mapping = build_zm_number_mapping("ATK+ZMCZ%").unwrap();
        assert_eq!(mapping.sent_text, "ATK+2%");
        assert!(!mapping.replacements[0].transport_wrapped);
    }

    #[test]
    fn hp_signed_percent_zm_stays_unwrapped() {
        let mapping = build_zm_number_mapping("HP-ZMDZ%").unwrap();
        assert_eq!(mapping.sent_text, "HP-2%");
        assert!(!mapping.replacements[0].transport_wrapped);
    }

    #[test]
    fn space_separated_sign_and_percent_zm_stays_unwrapped() {
        let mapping = build_zm_number_mapping("ATK + ZMCZ %").unwrap();
        assert_eq!(mapping.sent_text, "ATK + 2 %");
        assert!(!mapping.replacements[0].transport_wrapped);
    }

    #[test]
    fn split_fragments_are_sent_as_is() {
        let llm = MockLlmClient::with_responses(&["Later", "Cycle"]);
        let result = translate_chunk("Next;Turn", |_| None, "prefix", "en", &llm, test_settings());

        assert_eq!(llm.calls(), vec!["Next", "Turn"]);
        assert_eq!(result.text, "Later;Cycle");
        assert_eq!(
            result.new_entries,
            vec![entry("Next", "Later"), entry("Turn", "Cycle")]
        );
    }

    #[test]
    fn bracket_only_reprocesses_inner_text() {
        let llm = MockLlmClient::with_responses(&["Quest"]);
        let result = translate_chunk(
            "(Start;Next)",
            |key| (key == "Start").then(|| "Begin".to_string()),
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["Next"]);
        assert_eq!(result.text, "(Begin;Quest)");
        assert_eq!(result.new_entries, vec![entry("Next", "Quest")]);
    }

    #[test]
    fn one_sided_text_outside_bracket_is_fragment_not_whole() {
        let result = translate_chunk(
            "中立(ZMCZ)",
            |key| match key {
                "中立" => Some("平常".to_string()),
                "ZMCZ" => Some("ZMCZ".to_string()),
                _ => None,
            },
            "prefix",
            "ja",
            &MockLlmClient::with_responses(&[]),
            test_settings(),
        );

        assert_eq!(result.text, "平常(ZMCZ)");
        assert!(result.new_entries.is_empty());
    }

    #[test]
    fn multi_bracket_mixed_sentence_with_outer_text_is_not_whole_fragment() {
        let llm = MockLlmClient::with_responses(&["Foo", "InnerA", "Sentence", "InnerB", "Bar"]);
        let result = translate_chunk(
            "foo[内伤ZMDZ]造成了伤害ZMCZ点。[真伤ZMEZ]bar",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(
            llm.calls(),
            vec!["foo", "内伤2", "造成了伤害2点。", "真伤2", "bar"]
        );
        assert!(result
            .new_entries
            .iter()
            .all(|entry| entry.source != "foo[内伤ZMDZ]造成了伤害ZMCZ点。[真伤ZMEZ]bar"));
    }

    #[test]
    fn protected_angles_are_not_sent_to_model() {
        let llm = MockLlmClient::with_responses(&["Attack"]);
        let result = translate_chunk(
            "<color=red>攻撃</color>",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["攻撃"]);
        assert_eq!(result.text, "<color=red>Attack</color>");
        assert!(result
            .new_entries
            .iter()
            .all(|entry| !entry.source.contains("<color=red>")));
    }

    #[test]
    fn dict_hit_is_terminal_no_model_nor_registration() {
        let llm = MockLlmClient::with_responses(&[]);
        let result = translate_chunk(
            "吸内ZMDZ",
            |key| (key == "吸内ZMDZ").then(|| "吸い込むZMDZ".to_string()),
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert!(llm.calls().is_empty());
        assert_eq!(result.text, "吸い込むZMDZ");
        assert!(result.new_entries.is_empty());
    }

    #[test]
    fn translate_fragment_register_key_is_fragment_itself() {
        let llm = MockLlmClient::with_responses(&["吸い込む2"]);
        let result = translate_chunk("吸内ZMDZ", |_| None, "prefix", "en", &llm, test_settings());

        assert_eq!(result.new_entries.len(), 1);
        let entry = &result.new_entries[0];
        assert_eq!(entry.source, "吸内ZMDZ");
        assert_eq!(entry.translated, "吸い込むZMDZ");
        assert!(matches!(entry.persist, PersistEntry::Regex { .. }));
    }

    #[test]
    fn test_register_key_is_fragment_whole_with_signed_zm() {
        let llm = MockLlmClient::with_responses(&["Current HP -2"]);
        let result = translate_chunk("体力-ZMDZ", |_| None, "prefix", "en", &llm, test_settings());

        assert_eq!(result.new_entries.len(), 1);
        let entry = &result.new_entries[0];
        assert_eq!(entry.source, "体力-ZMDZ");
        assert_eq!(entry.translated, "Current HP -ZMDZ");
        match &entry.persist {
            PersistEntry::Regex {
                pattern,
                replacement,
            } => {
                let regex = Regex::new(pattern).unwrap();
                assert_eq!(
                    regex.replace("体力-ZMEZ", replacement.as_str()),
                    "Current HP -ZMEZ"
                );
            }
            other => panic!("expected Regex persist, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_zm_source_span_uses_occurrence_order_for_regex_persist() {
        let llm = MockLlmClient::with_responses(&["2ラウンド内、内力を3%回復"]);
        let result = translate_chunk(
            "ZMCZ回合内恢复ZMCZ%内力",
            |_| None,
            "prefix",
            "ja",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["2回合内恢复3%内力"]);

        let entry = result
            .new_entries
            .iter()
            .find(|entry| matches!(entry.persist, PersistEntry::Regex { .. }))
            .expect("regex persist should be created");

        match &entry.persist {
            PersistEntry::Regex {
                pattern,
                replacement,
            } => {
                assert!(pattern.matches("([+＋\\-－−]?Z[A-Z]+Z[%％]?)").count() >= 2);
                assert!(replacement.contains("$1"));
                assert!(replacement.contains("$2"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn duplicate_existing_numbers_are_counted_as_source_slots_for_regex_persist() {
        let llm = MockLlmClient::with_responses(&["stage１ stage1 recovers ２％ power"]);
        let result = translate_chunk(
            "stage１ stage1 recover ZMCZ% power",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["stage１ stage1 recover 2% power"]);

        let entry = result
            .new_entries
            .iter()
            .find(|entry| matches!(entry.persist, PersistEntry::Regex { .. }))
            .expect("regex persist should be created");

        match &entry.persist {
            PersistEntry::Regex {
                pattern,
                replacement,
            } => {
                assert!(pattern.contains("stage１ stage1"));
                assert!(pattern.contains("([+＋\\-－−]?Z[A-Z]+Z[%％]?)"));
                assert_eq!(replacement, "stage１ stage1 recovers $1 power");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn fullwidth_transport_numbers_use_numeric_slot_order_for_regex_persist() {
        let llm = MockLlmClient::with_responses(&["２ turns recover ３％ power"]);
        let result = translate_chunk(
            "ZMCZ turns recover ZMCZ% power",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["「2」 turns recover 3% power"]);

        let entry = result
            .new_entries
            .iter()
            .find(|entry| matches!(entry.persist, PersistEntry::Regex { .. }))
            .expect("regex persist should be created");

        match &entry.persist {
            PersistEntry::Regex {
                pattern,
                replacement,
            } => {
                let regex = Regex::new(pattern).unwrap();
                assert_eq!(
                    regex.replace("ZMDZ turns recover ZMEZ% power", replacement.as_str()),
                    "ZMDZ turns recover ZMEZ% power"
                );
                assert_eq!(replacement, "$1 turns recover $2 power");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn numeric_slot_count_mismatch_skips_zm_regex_without_exact_fallback() {
        let llm = MockLlmClient::with_responses(&["some turns recover ３％ power"]);
        let result = translate_chunk(
            "ZMCZ turns recover ZMCZ% power",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["「2」 turns recover 3% power"]);
        assert!(result.new_entries.is_empty());
    }

    #[test]
    fn space_separated_zm_uses_model_transport_without_exact_fallback() {
        let llm = MockLlmClient::with_responses(&["Translated"]);
        let result = translate_chunk(
            "ATK + ZMCZ %",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["ATK + 2 %"]);
        assert_eq!(result.text, "Translated");
        assert!(result.new_entries.is_empty());
    }

    #[test]
    fn adjacent_sign_zm_with_right_text_uses_model_transport_without_exact_fallback() {
        let llm = MockLlmClient::with_responses(&["Translated"]);
        let result = translate_chunk(
            "ATK+ZMCZ% up",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["ATK+2% up"]);
        assert_eq!(result.text, "Translated");
        assert!(result.new_entries.is_empty());
    }

    #[test]
    fn zm_lookup_miss_keeps_regex_entry_on_restored_surface_match() {
        let llm = MockLlmClient::with_responses(&["*2  #3"]);
        let result = translate_chunk(
            "*ZMCZ  #ZMDZ",
            |_| None,
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["*2  #3"]);
        assert_eq!(result.text, "*ZMCZ  #ZMDZ");
        assert_eq!(result.new_entries.len(), 1);
        assert_eq!(result.new_entries[0].source, "*ZMCZ  #ZMDZ");
        assert!(matches!(
            result.new_entries[0].persist,
            PersistEntry::Regex { .. }
        ));
        assert!(result.logs.iter().any(|event| matches!(
            event,
            LogEvent::ModelResult {
                source,
                original,
                translated,
                ..
            } if source == "*ZMCZ  #ZMDZ"
                && original == "*2  #3"
                && translated == "*ZMCZ  #ZMDZ"
        )));
    }

    #[test]
    fn zm_persist_number_disappears_skips_registration() {
        let llm = MockLlmClient::with_responses(&["A"]);
        let result = translate_chunk("A+ZMCZ", |_| None, "prefix", "en", &llm, test_settings());

        assert_eq!(result.text, "A");
        assert!(result.new_entries.is_empty());
    }

    #[test]
    fn render_atom_wrap_preserves_thai_nai_candidate() {
        let input = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ใน bbb";
        let atoms = vec![RenderAtom::Text(input.to_string())];

        let wrapped = wrap_render_atoms(
            atoms,
            TranslationSettings {
                enable_model_wrap: true,
                model_wrap_min_chars: 1,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        );
        let rendered = render_atoms(&wrapped);

        assert_eq!(
            rendered,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nใน bbb"
        );
    }

    #[test]
    fn render_atom_wrap_does_not_split_on_fullwidth_period() {
        let input = format!("{} ver．1 {}", "a".repeat(40), "b".repeat(46));
        let atoms = vec![RenderAtom::Text(input.clone())];

        let wrapped = wrap_render_atoms(atoms, test_settings());
        assert_eq!(render_atoms(&wrapped), input);
    }

    #[test]
    fn render_atom_wrap_preserves_japanese_comma_candidate() {
        let input =
            "だから今最優先すべきことは、まず太祖長拳の練習に専念しそれを完璧にマスターすることだ";
        let atoms = vec![RenderAtom::Text(input.to_string())];

        let wrapped = wrap_render_atoms(
            atoms,
            TranslationSettings {
                enable_model_wrap: true,
                model_wrap_min_chars: 1,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        );

        assert_eq!(
            render_atoms(&wrapped),
            "だから今最優先すべきことは、\nまず太祖長拳の練習に専念しそれを完璧にマスターすることだ"
        );
    }

    #[test]
    fn render_atom_wrap_preserves_simplified_chinese_fullwidth_comma_candidate() {
        let input = "这是一个很长的简体字句子，后续内容继续保持足够长度";
        let atoms = vec![RenderAtom::Text(input.to_string())];

        let wrapped = wrap_render_atoms(
            atoms,
            TranslationSettings {
                enable_model_wrap: true,
                model_wrap_min_chars: 1,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        );

        assert_eq!(
            render_atoms(&wrapped),
            "这是一个很长的简体字句子，\n后续内容继续保持足够长度"
        );
    }

    #[test]
    fn render_atom_wrap_splits_long_english_at_dot_space() {
        let atoms = vec![RenderAtom::Text(
            "The hero defeated the ancient dragon in a fierce battle. The kingdom was saved at last.".to_string(),
        )];

        let wrapped = wrap_render_atoms(atoms, test_settings());
        assert_eq!(
            render_atoms(&wrapped),
            "The hero defeated the ancient dragon in a fierce battle.\nThe kingdom was saved at last."
        );
    }

    #[test]
    fn render_atom_wrap_splits_long_english_at_comma_space() {
        let atoms = vec![RenderAtom::Text(
            "Alpha beta gamma, next section continues for long enough.".to_string(),
        )];

        let wrapped = wrap_render_atoms(
            atoms,
            TranslationSettings {
                enable_model_wrap: true,
                model_wrap_min_chars: 1,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        );

        assert_eq!(
            render_atoms(&wrapped),
            "Alpha beta gamma,\nnext section continues for long enough."
        );
    }

    #[test]
    fn render_atom_wrap_space_fallback_does_not_run_below_100_chars() {
        let input =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(input.chars().count(), 99);

        let wrapped = wrap_render_atoms(vec![RenderAtom::Text(input.to_string())], test_settings());
        assert_eq!(render_atoms(&wrapped), input);
    }

    #[test]
    fn render_atom_wrap_space_fallback_runs_at_100_chars() {
        let input =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(input.chars().count(), 100);

        let wrapped = wrap_render_atoms(vec![RenderAtom::Text(input.to_string())], test_settings());
        assert_eq!(
            render_atoms(&wrapped),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn transport_wrapped_persist_span_absorbs_model_wrappers() {
        let source = "foo ZMCZ bar";
        let mapping = build_zm_number_mapping(source).unwrap();
        let restored = restore_zm_number_tokens(&mapping.sent_text, &mapping);

        let persist = build_zm_persist_entry(source, &mapping, &mapping.sent_text, &restored)
            .expect("regex persist should be created");

        match persist {
            PersistEntry::Regex {
                pattern,
                replacement,
            } => {
                assert_eq!(replacement, "foo $1 bar");
                assert!(!replacement.contains('「'));
                assert!(!replacement.contains('」'));

                let regex = Regex::new(&pattern).unwrap();
                assert_eq!(
                    regex.replace("foo ZMDZ bar", replacement.as_str()),
                    "foo ZMDZ bar"
                );
            }
            other => panic!("expected Regex persist, got {other:?}"),
        }
    }

    #[test]
    fn transport_wrapped_persist_span_absorbs_variant_wrappers() {
        let source = "foo ZMCZ bar";
        let mapping = build_zm_number_mapping(source).unwrap();

        for model_output in ["foo （2） bar", "foo ”2” bar"] {
            let restored = restore_zm_number_tokens(model_output, &mapping);
            let persist = build_zm_persist_entry(source, &mapping, model_output, &restored)
                .expect("regex persist should be created");

            match persist {
                PersistEntry::Regex { replacement, .. } => {
                    assert_eq!(replacement, "foo $1 bar");
                }
                other => panic!("expected Regex persist, got {other:?}"),
            }
        }
    }

    #[test]
    fn transport_wrapped_persist_span_does_not_absorb_inner_space_wrapper() {
        let source = "foo ZMCZ bar";
        let mapping = build_zm_number_mapping(source).unwrap();
        let model_output = "foo 「 2 」 bar";
        let restored = restore_zm_number_tokens(model_output, &mapping);

        let persist = build_zm_persist_entry(source, &mapping, model_output, &restored)
            .expect("regex persist should be created");

        match persist {
            PersistEntry::Regex { replacement, .. } => {
                assert_eq!(replacement, "foo 「 $1 」 bar");
            }
            other => panic!("expected Regex persist, got {other:?}"),
        }
    }

    #[test]
    fn non_transport_wrapper_is_not_absorbed_by_persist_span() {
        let mapping = mapping(&[("2", "ZMCZ")]);
        let persist = build_zm_persist_entry(
            "foo ZMCZ bar",
            &mapping,
            "foo （2） bar",
            "foo （ZMCZ） bar",
        )
        .expect("regex persist should be created");

        match persist {
            PersistEntry::Regex { replacement, .. } => {
                assert_eq!(replacement, "foo （$1） bar");
            }
            other => panic!("expected Regex persist, got {other:?}"),
        }
    }

    #[test]
    fn newline_is_boundary_not_width() {
        let atoms = vec![
            RenderAtom::Text("aa".to_string()),
            RenderAtom::Newline("\n".to_string()),
            RenderAtom::Text("bb".to_string()),
        ];

        let wrapped = wrap_render_atoms(
            atoms,
            TranslationSettings {
                enable_model_wrap: true,
                model_wrap_min_chars: 1,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        );

        assert_eq!(render_atoms(&wrapped), "aa\nbb");
    }

    #[test]
    fn split_end_anchor_matches_separator_leaves_left() {
        let lookup_split = |key: &str| -> Option<SplitResult> {
            let re = Regex::new("对你的$").unwrap();
            re.captures(key).map(|caps| {
                let m = caps.get(0).unwrap();
                SplitResult {
                    full_match_start: m.start(),
                    full_match_end: m.end(),
                    inner_groups: Vec::new(),
                    replacement: "$Lがあなたに".to_string(),
                }
            })
        };

        let llm = MockLlmClient::with_responses(&["顧游年"]);
        let result = super::translate_chunk(
            "顧游年对你的",
            |_| None,
            lookup_split,
            "prefix",
            "ja",
            &llm,
            test_settings(),
        );

        assert_eq!(result.text, "顧游年があなたに");
        assert_eq!(llm.calls(), vec!["顧游年"]);
    }

    #[test]
    fn split_left_dict_hit_preserves_translation() {
        let lookup_split = |key: &str| -> Option<SplitResult> {
            let re = Regex::new("对你的$").unwrap();
            re.captures(key).map(|caps| {
                let m = caps.get(0).unwrap();
                SplitResult {
                    full_match_start: m.start(),
                    full_match_end: m.end(),
                    inner_groups: Vec::new(),
                    replacement: "$Lがあなたに".to_string(),
                }
            })
        };

        // left "顧游年" hits dictionary, no model calls needed
        let llm = MockLlmClient::with_responses(&[]);
        let result = super::translate_chunk(
            "顧游年对你的",
            |key| (key == "顧游年").then(|| "顧游年".to_string()),
            lookup_split,
            "prefix",
            "ja",
            &llm,
            test_settings(),
        );

        assert_eq!(result.text, "顧游年があなたに");
        assert!(llm.calls().is_empty());
    }
}
