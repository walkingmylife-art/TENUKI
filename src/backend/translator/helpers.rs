use crate::backend::normalize::normalize_display;

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

fn is_wrap_candidate(chars: &[char], index: usize) -> Option<usize> {
    let ch = chars.get(index).copied()?;
    let next = chars.get(index + 1).copied();

    if ch == '、' {
        return Some(1);
    }
    if ch == '。' {
        return Some(1);
    }
    if ch == '，' {
        return Some(1);
    }
    if (ch == '.' || ch == ',') && next == Some(' ') {
        return Some(2);
    }

    // Thai: detect " ใน" and break at the leading space
    if ch == ' ' && chars.get(index + 1) == Some(&'ใ') && chars.get(index + 2) == Some(&'น') {
        return Some(3);
    }

    None
}

fn wrap_candidate_score(chars: &[char], index: usize, candidate_width: usize) -> i32 {
    match (chars[index], candidate_width) {
        ('。', 1) => 12,
        ('、', 1) => 6,
        ('，', 1) => 6,
        ('.', 2) => 12,
        (',', 2) => 6,
        (' ', 3) => 12,
        _ => 0,
    }
}

pub fn apply_wrap(text: &str, enabled: bool, min_length: usize, _min_tail_length: usize) -> String {
    if !enabled || text.contains('\n') || text.chars().count() < min_length {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let center = len / 2;

    let mut best: Option<(i32, usize, usize, usize)> = None;

    for index in 0..len {
        let Some(candidate_width) = is_wrap_candidate(&chars, index) else {
            continue;
        };

        let base = wrap_candidate_score(&chars, index, candidate_width);
        if base == 0 {
            continue;
        }

        let distance = center.abs_diff(index);
        let score = base - distance as i32;

        match best {
            None => best = Some((score, distance, index, candidate_width)),
            Some((best_score, best_distance, best_index, _)) => {
                if score > best_score
                    || (score == best_score && distance < best_distance)
                    || (score == best_score && distance == best_distance && index > best_index)
                {
                    best = Some((score, distance, index, candidate_width));
                }
            }
        }
    }

    let Some((_, _, index, candidate_width)) = best else {
        return text.to_string();
    };

    let (emit_end, next_start) = match candidate_width {
        1 => (index + 1, index + 1),
        2 => (index + 1, index + 2),
        3 => (index, index + 1),
        _ => return text.to_string(),
    };

    let mut result = String::with_capacity(text.len() + 1);
    result.extend(chars[..emit_end].iter());
    result.push('\n');
    result.extend(chars[next_start..].iter());
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
        assert_eq!(
            clean_model_output("Level 10", "  Level 10  ", "en", true),
            "Level 10"
        );
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
            apply_wrap(text, true, 60, 10),
            "The hero defeated the ancient dragon in battle.\nThe kingdom was saved."
        );
    }

    #[test]
    fn wrap_does_not_split_short_text() {
        assert_eq!(apply_wrap("Hello. World.", true, 60, 10), "Hello. World.");
    }

    #[test]
    fn wrap_picks_one_best_candidate() {
        let text = "Alpha beta gamma delta epsilon zeta eta, Theta iota kappa lambda mu nu xi omicron. Pi rho sigma tau upsilon phi chi psi omega, Final section stays.";
        assert_eq!(
            apply_wrap(text, true, 60, 10),
            "Alpha beta gamma delta epsilon zeta eta, Theta iota kappa lambda mu nu xi omicron.\nPi rho sigma tau upsilon phi chi psi omega, Final section stays."
        );
    }

    #[test]
    fn wrap_ignores_min_tail_length() {
        let text = "123456789012345678901234567890. short";
        assert_eq!(
            apply_wrap(text, true, 31, 10),
            "123456789012345678901234567890.\nshort"
        );
    }

    #[test]
    fn wrap_splits_thai_at_leading_space_before_nai() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ใน bbb";
        assert_eq!(
            apply_wrap(text, true, 60, 10),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nใน bbb"
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
}
