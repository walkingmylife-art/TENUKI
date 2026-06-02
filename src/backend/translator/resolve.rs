use super::clean_model_output;
use super::lang::fallback_prefix;
use super::persist::build_zm_persist_entry;
use super::types::{
    NewTranslationEntry, PersistEntry, PlannedDocument, PlannedNode, ResolvedDocument,
    ResolvedFragmentNode, ResolvedLine, ResolvedNode, ResolvedSegment, TranslationResult,
    TranslationSettings,
};
use super::zm::{build_zm_number_mapping, restore_zm_number_tokens};
use super::LlmClient;
use crate::backend::dictionary::SplitResult;

fn has_echo_back(text: &str) -> bool {
    ECHO_BACK_MARKERS.iter().any(|m| text.contains(m))
}

const ECHO_BACK_MARKERS: &[&str] = &[
    // ── EN ──
    "[Background Information]", "[Source Text]", "[Original Text]", "[Translation]", "[Translated]",
    "［Background Information］", "［Source Text］", "［Original Text］", "［Translation］",
    "【Background Information】", "【Source Text】", "【Original Text】",
    // ── ZH ──
    "[背景信息]", "[待翻译文本]", "[原文]", "[翻译]", "[译文]",
    "［背景信息］", "［待翻译文本］", "［原文］", "［翻译］", "［译文］",
    "【背景信息】", "【待翻译文本】", "【原文】", "【翻译】", "【译文】", "【翻译结果】",
    // ── ZH-HANT ──
    "[背景資訊]", "[待翻譯文本]", "[譯文]",
    "［背景資訊］", "［待翻譯文本］",
    "【背景資訊】", "【待翻譯文本】", "【譯文】",
    // ── JA ──
    "[背景情報]", "[原文]", "[翻訳]", "[翻訳結果]",
    "［背景情報］", "［原文］", "［翻訳］", "［翻訳結果］",
    "【背景情報】", "【原文】", "【翻訳】",
    // ── KO ──
    "[배경 정보]", "[원문]", "[번역]", "[번역 결과]",
    "［배경 정보］", "［원문］", "［번역］",
    "【배경 정보】", "【원문】", "【번역】",
    // ── FR ──
    "[Informations contextuelles]", "[Texte source]", "[Texte original]", "[Traduction]",
    "［Informations contextuelles］", "［Texte source］", "［Texte original］", "［Traduction］",
    // ── DE ──
    "[Hintergrundinformationen]", "[Quelltext]", "[Originaltext]", "[Übersetzung]",
    "［Hintergrundinformationen］", "［Quelltext］", "［Originaltext］", "［Übersetzung］",
    // ── ES ──
    "[Información de contexto]", "[Texto fuente]", "[Texto original]", "[Traducción]",
    "［Información de contexto］", "［Texto fuente］", "［Texto original］", "［Traducción］",
    // ── PT ──
    "[Informações de contexto]", "[Texto fonte]", "[Texto original]", "[Tradução]",
    "［Informações de contexto］", "［Texto fonte］", "［Texto original］", "［Tradução］",
    // ── RU ──
    "[Исходная информация]", "[Исходный текст]", "[Перевод]",
    "［Исходная информация］", "［Исходный текст］", "［Перевод］",
    // ── AR ──
    "[معلومات أساسية]", "[النص الأصلي]", "[النص المصدر]", "[الترجمة]",
    "［معلومات أساسية］", "［النص الأصلي］", "［الترجمة］",
    // ── TH ──
    "[ข้อมูลพื้นหลัง]", "[ข้อความต้นฉบับ]", "[คำแปล]",
    "［ข้อมูลพื้นหลัง］", "［ข้อความต้นฉบับ］", "［คำแปล］",
    // ── VI ──
    "[Thông tin cơ bản]", "[Văn bản gốc]", "[Văn bản nguồn]", "[Bản dịch]",
    "［Thông tin cơ bản］", "［Văn bản gốc］", "［Bản dịch］",
    // ── TR ──
    "[Arka Plan Bilgisi]", "[Kaynak Metin]", "[Orijinal Metin]", "[Çeviri]",
    "［Arka Plan Bilgisi］", "［Kaynak Metin］", "［Orijinal Metin］", "［Çeviri］",
    // ── IT ──
    "[Informazioni di base]", "[Testo originale]", "[Testo sorgente]", "[Traduzione]",
    "［Informazioni di base］", "［Testo originale］", "［Traduzione］",
    // ── MS ──
    "[Maklumat Latar Belakang]", "[Teks Sumber]", "[Teks Asal]", "[Terjemahan]",
    "［Maklumat Latar Belakang］", "［Teks Sumber］", "［Terjemahan］",
    // ── ID ──
    "[Informasi Latar Belakang]", "[Teks Sumber]", "[Teks Asli]", "[Terjemahan]",
    "［Informasi Latar Belakang］", "［Teks Sumber］", "［Terjemahan］",
    // ── TL ──
    "[Impormasyon sa Background]", "[Orihinal na Teksto]", "[Pagsasalin]",
    "［Impormasyon sa Background］", "［Orihinal na Teksto］", "［Pagsasalin］",
    // ── HI ──
    "[पृष्ठभूमि जानकारी]", "[मूल पाठ]", "[अनुवाद]",
    "［पृष्ठभूमि जानकारी］", "［मूल पाठ］", "［अनुवाद］",
    // ── PL ──
    "[Informacje ogólne]", "[Tekst źródłowy]", "[Tekst oryginalny]", "[Tłumaczenie]",
    "［Informacje ogólne］", "［Tekst źródłowy］", "［Tłumaczenie］",
    // ── CS ──
    "[Základní informace]", "[Zdrojový text]", "[Původní text]", "[Překlad]",
    "［Základní informace］", "［Zdrojový text］", "［Překlad］",
    // ── NL ──
    "[Achtergrondinformatie]", "[Brontekst]", "[Originele tekst]", "[Vertaling]",
    "［Achtergrondinformatie］", "［Brontekst］", "［Vertaling］",
    // ── KM ──
    "[ព័ត៌មានផ្ទៃខាងក្រោយ]", "[អត្ថបទដើម]", "[ការបកប្រែ]",
    "［ព័ត៌មានផ្ទៃខាងក្រោយ］", "［អត្ថបទដើម］", "［ការបកប្រែ］",
    // ── MY ──
    "[နောက်ခံအချက်အလက်]", "[မူရင်းစာသား]", "[ဘာသာပြန်]",
    "［နောက်ခံအချက်အလက်］", "［မူရင်းစာသား］", "［ဘာသာပြန်］",
    // ── FA ──
    "[اطلاعات پس‌زمینه]", "[متن اصلی]", "[ترجمه]",
    "［اطلاعات پس‌زمینه］", "［متن اصلی］", "［ترجمه］",
    // ── GU ──
    "[પૃષ્ઠભૂમિ માહિતી]", "[મૂળ લખાણ]", "[અનુવાદ]",
    "［પૃષ્ઠભૂમિ માહિતી］", "［મૂળ લખાણ］", "［અનુવાદ］",
    // ── UR ──
    "[پس منظر کی معلومات]", "[اصل متن]", "[ترجمہ]",
    "［پس منظر کی معلومات］", "［اصل متن］", "［ترجمہ］",
    // ── TE ──
    "[నేపథ్య సమాచారం]", "[మూల పాఠం]", "[అనువాదం]",
    "［నేపథ్య సమాచారం］", "［మూల పాఠం］", "［అనువాదం］",
    // ── MR ──
    "[पार्श्वभूमी माहिती]", "[मूळ मजकूर]", "[अनुवाद]",
    "［पार्श्वभूमी माहिती］", "［मूळ मजकूर］", "［अनुवाद］",
    // ── HE ──
    "[מידע רקע]", "[טקסט מקורי]", "[תרגום]",
    "［מידע רקע］", "［טקסט מקורי］", "［תרגום］",
    // ── BN ──
    "[পটভূমির তথ্য]", "[মূল পাঠ্য]", "[অনুবাদ]",
    "［পটভূমির তথ্য］", "［মূল পাঠ্য］", "［অনুবাদ］",
    // ── TA ──
    "[பின்னணி தகவல்]", "[மூல உரை]", "[மொழிபெயர்ப்பு]",
    "［பின்னணி தகவல்］", "［மூல உரை］", "［மொழிபெயர்ப்பு］",
    // ── UK ──
    "[Основна інформація]", "[Вихідний текст]", "[Переклад]",
    "［Основна інформація］", "［Вихідний текст］", "［Переклад］",
    // ── BO ──
    "[རྒྱབ་ལྗོངས་གནས་ཚུལ]", "[མ་ཡིག]", "[འགྱུར་ཡིག]",
    "［རྒྱབ་ལྗོངས་གནས་ཚུལ］", "［མ་ཡིག］", "［འགྱུར་ཡིག］",
    // ── KK ──
    "[Фондық ақпарат]", "[Бастапқы мәтін]", "[Аударма]",
    "［Фондық ақпарат］", "［Бастапқы мәтін］", "［Аударма］",
    // ── MN ──
    "[Суурь мэдээлэл]", "[Эх текст]", "[Орчуулга]",
    "［Суурь мэдээлэл］", "［Эх текст］", "［Орчуулга］",
    // ── UG ──
    "[ئارقا كۆرۈنۈش ئۇچۇرى]", "[ئەسلى تېكىست]", "[تەرجىمە]",
    "［ئارقا كۆرۈنۈش ئۇچۇرى］", "［ئەسلى تېكىست］", "［تەرجىمە］",
    // ── YUE ──
    "[背景資訊]", "[原文]", "[翻譯]",
    "［背景資訊］", "［原文］",
    "【背景資訊】", "【原文】", "【翻譯】",
];

pub(super) fn dedupe_entries(entries: Vec<NewTranslationEntry>) -> Vec<NewTranslationEntry> {
    let mut seen = rustc_hash::FxHashSet::default();
    entries
        .into_iter()
        .filter(|entry| seen.insert(entry.source.clone()))
        .collect()
}

pub(super) fn resolve_document<F, S>(
    document: &PlannedDocument,
    lookup: &F,
    lookup_split: &S,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> (ResolvedDocument, TranslationResult)
where
    F: Fn(&str) -> Option<String>,
    S: Fn(&str) -> Option<SplitResult>,
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
                            lookup_split,
                            prefix,
                            tgt_lang,
                            llm_client,
                            settings,
                        );

                        log::info!(
                            "[RESOLVE] fragment_source=\"{}\" child_text=\"{}\"",
                            fragment.authority.source,
                            child.text
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

    let translated_raw = llm_client.translate_sync(model_input, prefix);
    let translated_raw = match translated_raw {
        Some(ref raw) if has_echo_back(raw) => {
            log::info!(
                "[RETRY] echo_back_detected source=\"{}\"",
                text
            );
            llm_client.translate_sync(model_input, &fallback_prefix(tgt_lang))
        }
        other => other,
    };

    if let Some(translated_raw) = translated_raw {
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

pub(super) fn translate_fragment<F, S>(
    fragment: &str,
    lookup: &F,
    lookup_split: &S,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult
where
    F: Fn(&str) -> Option<String>,
    S: Fn(&str) -> Option<SplitResult>,
{
    // Contract:
    // `fragment` is the dictionary key authority for this call.
    // Do not derive lookup/register keys from model output, restored text, or render surface.
    // ZM numeric replacement is model transport only and stays inside translate_model_only().
    if fragment.trim().is_empty() {
        return TranslationResult::empty(fragment.to_string());
    }

    let start = std::time::Instant::now();

    // Dict lookup first.
    if let Some(hit) = lookup(fragment) {
        log::warn!("[FRAGMENT] fragment=\"{}\" result=dict_hit value=\"{}\"", fragment, hit);
        return TranslationResult::from_dict_hit(hit, fragment, start.elapsed());
    }

    // Split: separator acts as dictionary-hit boundary.
    if let Some(sr) = lookup_split(fragment) {
        log::warn!("[FRAGMENT] fragment=\"{}\" result=split_hit", fragment);
        let full_start = sr.full_match_start;
        let full_end = sr.full_match_end;

        let left = &fragment[..full_start];
        let right = &fragment[full_end..];

        // Build separator text: everything in the full match that is not an inner group.
        let mut separator_parts: Vec<&str> = Vec::new();
        let mut prev_end = full_start;
        for g in &sr.inner_groups {
            if let Some((g_start, g_end)) = g {
                if *g_start > prev_end {
                    separator_parts.push(&fragment[prev_end..*g_start]);
                }
                prev_end = *g_end;
            }
        }
        if prev_end < full_end {
            separator_parts.push(&fragment[prev_end..full_end]);
        }
        let separator_raw: String = if separator_parts.is_empty() {
            String::new()
        } else {
            separator_parts.concat()
        };

        // Translate inner groups (recursive).
        let mut inner_results: Vec<TranslationResult> = Vec::new();
        for g in &sr.inner_groups {
            if let Some((g_start, g_end)) = g {
                let inner_text = &fragment[*g_start..*g_end];
                if inner_text.is_empty() {
                    inner_results.push(TranslationResult::empty(String::new()));
                } else {
                    inner_results.push(translate_fragment(
                        inner_text,
                        lookup,
                        lookup_split,
                        prefix,
                        tgt_lang,
                        llm_client,
                        settings,
                    ));
                }
            }
        }

        // If no inner groups and no left/right, fall through to model.
        if sr.inner_groups.iter().all(|g| g.is_none()) && left.is_empty() && right.is_empty() {
            log::warn!("[FRAGMENT] fragment=\"{}\" result=model_only_after_split_empty", fragment);
            return translate_model_only(fragment, prefix, tgt_lang, llm_client, settings);
        }

        let left_result = if left.is_empty() {
            TranslationResult::empty(String::new())
        } else {
            translate_fragment(left, lookup, lookup_split, prefix, tgt_lang, llm_client, settings)
        };

        let separator_text = if separator_raw.is_empty() {
            String::new()
        } else {
            match lookup(&separator_raw) {
                Some(hit) => hit,
                None => separator_raw.to_string(),
            }
        };

        let right_result = if right.is_empty() {
            TranslationResult::empty(String::new())
        } else {
            translate_fragment(right, lookup, lookup_split, prefix, tgt_lang, llm_client, settings)
        };

        log::warn!(
            "[SPLIT_DEBUG] fragment=\"{}\" full_match=\"{}\" left=\"{}\" separator=\"{}\" right=\"{}\" replacement_template=\"{}\"",
            fragment,
            &fragment[full_start..full_end],
            left,
            separator_raw,
            right,
            sr.replacement
        );

        let mut combined_text = sr.replacement.clone();
        combined_text = combined_text.replace("$L", &left_result.text);
        for (i, inner) in inner_results.iter().enumerate() {
            let placeholder = format!("$S{}", i + 1);
            combined_text = combined_text.replace(&placeholder, &inner.text);
        }
        combined_text = combined_text.replace("$S", &separator_text);
        combined_text = combined_text.replace("$R", &right_result.text);

        log::warn!(
            "[SPLIT_DEBUG] combined_before=\"{}\" left_result=\"{}\" sep_result=\"{}\" right_result=\"{}\" combined_after=\"{}\"",
            sr.replacement,
            left_result.text,
            separator_text,
            right_result.text,
            combined_text
        );

        let mut result = TranslationResult::empty(combined_text);
        result.absorb(left_result);
        for inner in inner_results {
            result.absorb(inner);
        }
        result.absorb(right_result);
        return result;
    }

    log::warn!("[FRAGMENT] fragment=\"{}\" result=model_only", fragment);
    translate_model_only(fragment, prefix, tgt_lang, llm_client, settings)
}
