use crate::backend::normalize::normalize_display;
use once_cell::sync::Lazy;
use regex::Regex;

static APOSTROPHE_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\p{L})'\s+(\p{L})").unwrap());

fn target_compacts_internal_spaces(tgt_lang: &str) -> bool {
    matches!(tgt_lang, "ja" | "zh" | "zh-CN" | "zh-Hant" | "zh-TW")
}

fn count_leading_spaces(text: &str) -> usize {
    text.chars().take_while(|ch| *ch == ' ').count()
}

fn count_trailing_spaces(text: &str) -> usize {
    text.chars().rev().take_while(|ch| *ch == ' ').count()
}

fn collect_space_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current = 0usize;

    for ch in text.chars() {
        if ch == ' ' {
            current += 1;
            continue;
        }

        if current > 0 {
            runs.push(" ".repeat(current));
            current = 0;
        }
    }

    if current > 0 {
        runs.push(" ".repeat(current));
    }

    runs
}

fn reapply_source_space_template(src_text: &str, translated_text: &str) -> Option<String> {
    let src_core = src_text.trim();
    if !src_core.contains("  ") {
        return None;
    }

    let src_chunks: Vec<&str> = src_core
        .split(' ')
        .filter(|chunk| !chunk.is_empty())
        .collect();
    if src_chunks.len() < 2 {
        return None;
    }

    let src_runs = collect_space_runs(src_core);
    if src_runs.len() + 1 != src_chunks.len() {
        return None;
    }

    let translated_chunks: Vec<&str> = translated_text.split_whitespace().collect();
    if translated_chunks.len() != src_chunks.len() {
        return None;
    }

    let mut rebuilt = translated_chunks[0].to_string();
    for (run, chunk) in src_runs.iter().zip(translated_chunks.iter().skip(1)) {
        rebuilt.push_str(run);
        rebuilt.push_str(chunk);
    }

    Some(rebuilt)
}

fn normalize_apostrophes(text: &str) -> String {
    let s = text
        .replace('\u{2019}', "'")
        .replace('\u{02BC}', "'")
        .replace('`', "'");
    APOSTROPHE_SPACE.replace_all(&s, "$1'$2").into_owned()
}

fn fix_extra_spaces(
    src_text: &str,
    translated_text: &str,
    preserve_internal_spaces: bool,
) -> String {
    let leading_spaces = count_leading_spaces(src_text);
    let trailing_spaces = count_trailing_spaces(src_text);

    if preserve_internal_spaces {
        return format!(
            "{}{}{}",
            " ".repeat(leading_spaces),
            translated_text.trim(),
            " ".repeat(trailing_spaces),
        );
    }

    if let Some(rebuilt) = reapply_source_space_template(src_text, translated_text.trim()) {
        return format!(
            "{}{}{}",
            " ".repeat(leading_spaces),
            rebuilt,
            " ".repeat(trailing_spaces),
        );
    }

    let src_spaces = src_text.trim().matches(' ').count();
    let mut result = translated_text.trim().to_string();
    let mut extra_spaces = result.matches(' ').count().saturating_sub(src_spaces);

    while extra_spaces > 0 {
        if let Some(index) = result.rfind(' ') {
            result.remove(index);
            extra_spaces -= 1;
        } else {
            break;
        }
    }

    format!(
        "{}{}{}",
        " ".repeat(leading_spaces),
        result,
        " ".repeat(trailing_spaces),
    )
}

fn is_zm_marker_start(chars: &[char], start: usize) -> bool {
    if chars.get(start) != Some(&'Z') {
        return false;
    }

    let mut probe = start + 1;
    while probe < chars.len() && chars[probe].is_ascii_uppercase() && chars[probe] != 'Z' {
        probe += 1;
    }

    probe > start + 1 && chars.get(probe) == Some(&'Z')
}

fn normalize_plus_minus_spacing(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        let normalized_sign = match ch {
            '+' | '\u{FF0B}' => Some('+'),
            '-' | '\u{FF0D}' => Some('-'),
            _ => None,
        };

        if let Some(sign) = normalized_sign {
            let mut probe = index + 1;
            let mut saw_space = false;

            while probe < chars.len() && chars[probe] == ' ' {
                saw_space = true;
                probe += 1;
            }

            if saw_space && is_zm_marker_start(&chars, probe) {
                result.push(sign);
                index = probe;
                continue;
            }
        }

        result.push(ch);
        index += 1;
    }

    result
}

fn collapse_duplicate_terminal_punctuation(mut text: String) -> String {
    for punct in ['。', '、'] {
        let terminal_count = text.chars().rev().take_while(|&ch| ch == punct).count();
        if terminal_count == 2 {
            let new_len = text.len() - punct.len_utf8();
            text.truncate(new_len);
            text = text.trim_end().to_string();
        }
    }
    text
}

pub fn clean_model_output(
    src: &str,
    translated: &str,
    tgt_lang: &str,
    enable_symbol_cleanup: bool,
) -> String {
    let src = src.trim();
    let mut result = normalize_display(translated);
    result = normalize_apostrophes(&result);
    result = fix_extra_spaces(src, &result, !target_compacts_internal_spaces(tgt_lang));
    if enable_symbol_cleanup {
        result = normalize_plus_minus_spacing(&result);
    }

    collapse_duplicate_terminal_punctuation(result)
}

#[cfg(test)]
mod tests {
    use super::clean_model_output;

    #[test]
    fn trims_only_edges_for_english_target() {
        assert_eq!(
            clean_model_output("Level 10", "  Level 10  ", "en", true),
            "Level 10"
        );
    }

    #[test]
    fn removes_extra_internal_spaces_for_japanese_target() {
        assert_eq!(
            clean_model_output("攻撃力10", "攻撃 力 10", "ja", true),
            "攻撃力10"
        );
    }

    #[test]
    fn preserves_internal_spaces_for_vietnamese_target() {
        assert_eq!(
            clean_model_output("Kinh nghiem", "  Kinh  nghiem  ", "vi", true),
            "Kinh  nghiem"
        );
    }

    #[test]
    fn clean_model_output_normalizes_fullwidth_plus_minus_spacing() {
        assert_eq!(
            clean_model_output("bonus", "\u{FF0B} ZAZ", "en", true),
            "+ZAZ"
        );
        assert_eq!(
            clean_model_output("bonus", "\u{FF0D} ZAZ", "en", true),
            "-ZAZ"
        );
    }

    #[test]
    fn clean_model_output_can_skip_symbol_cleanup() {
        assert_eq!(
            clean_model_output("bonus", "\u{FF0B} ZAZ", "en", false),
            "+ ZAZ"
        );
    }

    #[test]
    fn clean_model_output_leaves_regular_plus_minus_spacing_unchanged() {
        assert_eq!(clean_model_output("+ 10", "+ 10", "en", true), "+ 10");
        assert_eq!(
            clean_model_output("HP + 10", "HP + 10", "en", true),
            "HP + 10"
        );
        assert_eq!(clean_model_output("A - B", "A - B", "en", true), "A - B");
        assert_eq!(clean_model_output("- foo", "- foo", "en", true), "- foo");
    }

    #[test]
    fn clean_model_output_fixes_apostrophe_spacing_for_english_target() {
        assert_eq!(
            clean_model_output("one's", "one\u{2019} s", "en", true),
            "one's"
        );
    }

    #[test]
    fn clean_model_output_fixes_apostrophe_spacing_for_vietnamese_target() {
        assert_eq!(
            clean_model_output("d'accord", "d\u{2019} accord", "vi", true),
            "d'accord"
        );
    }

    #[test]
    fn clean_model_output_fixes_apostrophe_spacing_for_non_ascii_letters() {
        assert_eq!(
            clean_model_output("l'ami", "l\u{2019} ami", "fr", true),
            "l'ami"
        );
    }

    #[test]
    fn clean_model_output_keeps_regular_internal_spaces_for_vietnamese() {
        assert_eq!(
            clean_model_output("Kinh nghiem", "  Kinh  nghiem  ", "vi", true),
            "Kinh  nghiem"
        );
    }

    #[test]
    fn clean_model_output_removes_cjk_internal_spaces_without_punctuation() {
        assert_eq!(
            clean_model_output("攻撃力10", "攻撃 力 10", "ja", true),
            "攻撃力10"
        );
    }

    #[test]
    fn clean_model_output_keeps_single_terminal_period_even_if_source_has_none() {
        assert_eq!(
            clean_model_output("Hello", "こんにちは。", "ja", true),
            "こんにちは。"
        );
    }

    #[test]
    fn clean_model_output_keeps_single_terminal_period_when_source_has_ascii_period() {
        assert_eq!(
            clean_model_output("Hello.", "こんにちは。", "ja", true),
            "こんにちは。"
        );
    }

    #[test]
    fn clean_model_output_collapses_duplicate_terminal_period() {
        assert_eq!(
            clean_model_output("Hello", "こんにちは。。", "ja", true),
            "こんにちは。"
        );
    }

    #[test]
    fn clean_model_output_keeps_three_terminal_periods() {
        assert_eq!(
            clean_model_output("Hello", "こんにちは。。。", "ja", true),
            "こんにちは。。。"
        );
    }

    #[test]
    fn clean_model_output_collapses_duplicate_terminal_japanese_comma() {
        assert_eq!(
            clean_model_output("Hello", "こんにちは、、", "ja", true),
            "こんにちは、"
        );
    }

    #[test]
    fn clean_model_output_keeps_single_terminal_japanese_comma() {
        assert_eq!(
            clean_model_output("Hello", "こんにちは、", "ja", true),
            "こんにちは、"
        );
    }

    #[test]
    fn clean_model_output_keeps_three_terminal_japanese_commas() {
        assert_eq!(
            clean_model_output("Hello", "こんにちは、、、", "ja", true),
            "こんにちは、、、"
        );
    }

    #[test]
    fn clean_model_output_leaves_fullwidth_comma_pair_unchanged() {
        assert_eq!(
            clean_model_output("Hello", "你好，，", "zh-CN", true),
            "你好，，"
        );
    }

}
