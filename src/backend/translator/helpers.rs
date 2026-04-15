use crate::backend::normalize::normalize_display;

fn normalize_observed_model_quirks(text: &str) -> String {
    text.replace('\u{2019}', "'")
        .replace("窶冱", "'s")
        .replace("窶ｦ", "...")
}

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

    let src_chunks: Vec<&str> = src_core.split(' ').filter(|chunk| !chunk.is_empty()).collect();
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

fn fix_extra_spaces(src_text: &str, translated_text: &str, preserve_internal_spaces: bool) -> String {
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

fn normalize_plus_minus_spacing(text: &str) -> String {
    let normalized = text
        .replace('\u{FF0B}', "+")
        .replace('\u{FF0D}', "-");
    let chars: Vec<char> = normalized.chars().collect();
    let mut result = String::with_capacity(normalized.len());
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];

        if ch == ' ' {
            let prev = result.chars().next_back();
            let next = chars.get(index + 1).copied();
            if matches!(prev, Some('+') | Some('-')) || matches!(next, Some('+') | Some('-')) {
                index += 1;
                continue;
            }
        }

        result.push(ch);
        index += 1;
    }

    result
}

fn is_wrap_candidate(chars: &[char], index: usize) -> Option<usize> {
    let ch = chars.get(index).copied()?;
    let next = chars.get(index + 1).copied()?;
    if ch == '、' {
        return Some(1);
    }
    if (ch == '.' || ch == ',' || ch == '，' || ch == '。') && next == ' ' {
        Some(2)
    } else {
        None
    }
}

pub fn apply_wrap(
    text: &str,
    enabled: bool,
    min_length: usize,
    min_tail_length: usize,
) -> String {
    if !enabled || text.contains('\n') || text.chars().count() < min_length {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len() + 4);
    let mut segment_start = 0usize;

    while chars.len().saturating_sub(segment_start) > min_length {
        let search_start = segment_start + min_length;
        let mut break_index = None;

        for index in search_start..chars.len().saturating_sub(1) {
            let Some(candidate_width) = is_wrap_candidate(&chars, index) else {
                continue;
            };
            let tail_len = chars.len().saturating_sub(index + candidate_width);
            if tail_len >= min_tail_length {
                break_index = Some(index);
                break;
            }
        }

        let Some(index) = break_index else {
            break;
        };

        let candidate_width = is_wrap_candidate(&chars, index).unwrap_or(1);
        let emit_end = if candidate_width == 2 { index + 1 } else { index + candidate_width };
        result.extend(chars[segment_start..emit_end].iter());
        result.push('\n');
        segment_start = index + candidate_width;
    }

    result.extend(chars[segment_start..].iter());
    result
}

pub fn clean_model_output(
    src: &str,
    translated: &str,
    tgt_lang: &str,
    enable_symbol_cleanup: bool,
) -> String {
    let src = src.trim();
    let mut result = normalize_display(translated);
    result = normalize_observed_model_quirks(&result);
    result = fix_extra_spaces(src, &result, !target_compacts_internal_spaces(tgt_lang));
    if enable_symbol_cleanup {
        result = normalize_plus_minus_spacing(&result);
    }

    for punct in &['。', '，', '．'] {
        if !src.ends_with(*punct) {
            while result.ends_with(*punct) {
                let new_len = result.len() - punct.len_utf8();
                result.truncate(new_len);
                result = result.trim_end().to_string();
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::{apply_wrap, clean_model_output};

    #[test]
    fn trims_only_edges_for_english_target() {
        assert_eq!(clean_model_output("Level 10", "  Level 10  ", "en", true), "Level 10");
    }

    #[test]
    fn removes_extra_internal_spaces_for_japanese_target() {
        assert_eq!(
            clean_model_output("攻撃力10", "攻撃 力 10。", "ja", true),
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
    fn wrap_splits_long_english_at_dot_space() {
        let text = "The hero defeated the ancient dragon in battle. The kingdom was saved.";
        assert_eq!(
            apply_wrap(text, true, 30, 10),
            "The hero defeated the ancient dragon in battle.\nThe kingdom was saved."
        );
    }

    #[test]
    fn wrap_does_not_split_short_text() {
        assert_eq!(apply_wrap("Hello. World.", true, 30, 10), "Hello. World.");
    }

    #[test]
    fn wrap_can_repeat_after_each_threshold_window() {
        let text = "Alpha beta gamma delta epsilon zeta eta, Theta iota kappa lambda mu nu xi omicron. Pi rho sigma tau upsilon phi chi psi omega, Final section stays.";
        assert_eq!(
            apply_wrap(text, true, 30, 10),
            "Alpha beta gamma delta epsilon zeta eta,\nTheta iota kappa lambda mu nu xi omicron.\nPi rho sigma tau upsilon phi chi psi omega,\nFinal section stays."
        );
    }

    #[test]
    fn wrap_requires_ten_chars_after_dot_space() {
        let text = "123456789012345678901234567890. short";
        assert_eq!(apply_wrap(text, true, 30, 10), text);
    }

    #[test]
    fn clean_model_output_normalizes_fullwidth_plus_minus_spacing() {
        assert_eq!(clean_model_output("HP+ATK", "HP ＋ ATK", "en", true), "HP+ATK");
        assert_eq!(clean_model_output("HP-ATK", "HP － ATK", "en", true), "HP-ATK");
    }

    #[test]
    fn clean_model_output_can_skip_symbol_cleanup() {
        assert_eq!(clean_model_output("HP+ATK", "HP ＋ ATK", "en", false), "HP + ATK");
    }

    #[test]
    fn clean_model_output_normalizes_observed_smart_quote() {
        assert_eq!(
            clean_model_output("Others", "Others\u{2019}", "en", true),
            "Others'"
        );
    }

    #[test]
    fn clean_model_output_normalizes_observed_mojibake_sequences() {
        assert_eq!(
            clean_model_output("today battle", "today窶冱 battle", "en", true),
            "today's battle"
        );
        assert_eq!(
            clean_model_output("wait", "wait窶ｦ", "en", true),
            "wait..."
        );
    }
}
