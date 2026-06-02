const FULLWIDTH_PLUS: char = '\u{FF0B}';
const FULLWIDTH_MINUS: char = '\u{FF0D}';
const MINUS_SIGN: char = '\u{2212}';
const FULLWIDTH_PERCENT: char = '\u{FF05}';
const LEFT_DOUBLE_QUOTE: char = '\u{201C}';
const RIGHT_DOUBLE_QUOTE: char = '\u{201D}';

fn is_plus_sign(ch: char) -> bool {
    matches!(ch, '+' | FULLWIDTH_PLUS)
}

fn is_minus_sign(ch: char) -> bool {
    matches!(ch, '-' | FULLWIDTH_MINUS | MINUS_SIGN)
}

fn is_sign(ch: char) -> bool {
    is_plus_sign(ch) || is_minus_sign(ch)
}

fn is_percent_sign(ch: char) -> bool {
    matches!(ch, '%' | FULLWIDTH_PERCENT)
}

fn is_horizontal_space(ch: char) -> bool {
    ch == ' ' || ch == '\u{3000}'
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NumericRun {
    pub raw: String,
    pub normalized: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ZmReplacement {
    pub number: String,
    pub marker: String,
    pub trim_trailing_minus: bool,
    pub transport_wrapped: bool,
    pub transport_left_space: bool,
    pub transport_right_space: bool,
    pub source_span: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ZmNumberMapping {
    pub sent_text: String,
    pub replacements: Vec<ZmReplacement>,
}

fn normalize_digit_char(ch: char) -> Option<char> {
    match ch {
        '0'..='9' => Some(ch),
        '\u{FF10}'..='\u{FF19}' => char::from_u32(ch as u32 - 0xFF10 + '0' as u32),
        _ => None,
    }
}

// --- 新関数①: ASCII → CJK ---
fn ascii_digit_to_cjk(ch: char) -> Option<char> {
    match ch {
        '0' => Some('〇'),
        '1' => Some('一'),
        '2' => Some('二'),
        '3' => Some('三'),
        '4' => Some('四'),
        '5' => Some('五'),
        '6' => Some('六'),
        '7' => Some('七'),
        '8' => Some('八'),
        '9' => Some('九'),
        _ => None,
    }
}

// --- 新関数①逆: CJK → ASCII（予防策用） ---
fn cjk_digit_to_ascii(ch: char) -> Option<char> {
    match ch {
        '〇' => Some('0'),
        '一' => Some('1'),
        '二' => Some('2'),
        '三' => Some('3'),
        '四' => Some('4'),
        '五' => Some('5'),
        '六' => Some('6'),
        '七' => Some('7'),
        '八' => Some('8'),
        '九' => Some('9'),
        _ => None,
    }
}

// --- 新関数②: CJK数字フォールバック（ブラケット制限なし） ---
pub(super) fn apply_cjk_digit_fallback(text: &str, missing: &[&str]) -> String {
    let mut result = text.to_string();
    for &num_str in missing {
        // 単桁ASCII数字でなければスキップ
        if num_str.chars().count() != 1 {
            continue;
        }
        let ascii_ch = num_str.chars().next().unwrap();
        if !ascii_ch.is_ascii_digit() {
            continue;
        }
        let Some(cjk) = ascii_digit_to_cjk(ascii_ch) else {
            continue;
        };

        // テキスト中のCJK数字をすべてASCII数字に置換
        result = result.replace(cjk, &ascii_ch.to_string());
    }
    result
}

pub(super) fn collect_numeric_runs(text: &str) -> Vec<NumericRun> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut runs = Vec::new();
    let mut index = 0usize;

    while index < chars.len() {
        let (start, ch) = chars[index];
        if normalize_digit_char(ch).is_none() {
            index += 1;
            continue;
        }

        let mut normalized = String::new();
        normalized.push(normalize_digit_char(ch).unwrap());

        let mut probe = index + 1;
        while probe < chars.len() {
            if let Some(normalized_digit) = normalize_digit_char(chars[probe].1) {
                normalized.push(normalized_digit);
                probe += 1;
            } else {
                break;
            }
        }

        let end = chars
            .get(probe)
            .map_or(text.len(), |&(byte_idx, _)| byte_idx);
        runs.push(NumericRun {
            raw: text[start..end].to_string(),
            normalized,
            start,
            end,
        });
        index = probe;
    }

    runs
}

pub(super) fn collect_existing_number_tokens(text: &str) -> rustc_hash::FxHashSet<String> {
    collect_numeric_runs(text)
        .into_iter()
        .map(|run| run.normalized)
        .collect()
}

fn prev_char_with_start(text: &str, byte_idx: usize) -> Option<(usize, char)> {
    text[..byte_idx].char_indices().last()
}

fn next_char_with_start(text: &str, byte_idx: usize) -> Option<(usize, char)> {
    text[byte_idx..]
        .char_indices()
        .next()
        .map(|(offset, ch)| (byte_idx + offset, ch))
}

fn is_direct_wrapper_pair(open: char, close: char) -> bool {
    matches!(
        (open, close),
        ('「', '」')
            | ('(', ')')
            | ('（', '）')
            | ('"', '"')
            | (LEFT_DOUBLE_QUOTE, RIGHT_DOUBLE_QUOTE)
            | (RIGHT_DOUBLE_QUOTE, RIGHT_DOUBLE_QUOTE)
    )
}

pub(super) fn direct_wrapper_span_for_numeric_run(
    text: &str,
    run: &NumericRun,
) -> Option<(usize, usize)> {
    let (open_start, open) = prev_char_with_start(text, run.start)?;
    let (_, close) = next_char_with_start(text, run.end)?;
    if is_direct_wrapper_pair(open, close) {
        Some((open_start, run.end + close.len_utf8()))
    } else {
        None
    }
}

fn build_transport_restored_surface(
    text: &str,
    span_start: usize,
    span_end: usize,
    repl: &ZmReplacement,
    absorbed_direct_wrapper: bool,
) -> String {
    if !repl.transport_wrapped {
        return repl.marker.clone();
    }

    let prev_outside = prev_char_with_start(text, span_start).map(|(_, ch)| ch);
    let next_outside = next_char_with_start(text, span_end).map(|(_, ch)| ch);
    let mut restored = String::new();

    let can_add_left_space = repl.transport_left_space
        && !prev_outside.is_some_and(is_horizontal_space)
        && (absorbed_direct_wrapper
            || prev_outside.is_none()
            || prev_outside.is_some_and(|ch| is_sign(ch) || is_percent_sign(ch)));
    if can_add_left_space {
        restored.push(' ');
    }

    restored.push_str(&repl.marker);

    let can_add_right_space = repl.transport_right_space
        && !next_outside.is_some_and(is_horizontal_space)
        && (absorbed_direct_wrapper
            || next_outside.is_none()
            || next_outside.is_some_and(|ch| is_sign(ch) || is_percent_sign(ch)));
    if can_add_right_space {
        restored.push(' ');
    }

    restored
}

pub(super) fn build_zm_number_mapping(text: &str) -> Option<ZmNumberMapping> {
    fn is_zm_inner_char(ch: char) -> bool {
        ch.is_ascii_uppercase() && ch != 'Z'
    }

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut replacements: Vec<ZmReplacement> = Vec::new();
    let mut index = 0usize;

    // --- 予防策: 漢数字との衝突を回避 ---
    let mut existing = collect_existing_number_tokens(text);
    for ch in text.chars() {
        if let Some(ascii) = cjk_digit_to_ascii(ch) {
            existing.insert(ascii.to_string());
        }
    }
    // --- ここまで ---

    let mut counter = 2usize;

    while index < chars.len() {
        let (start, ch) = chars[index];
        if ch != 'Z' {
            index += 1;
            continue;
        }

        let mut probe = index + 1;
        while probe < chars.len() && is_zm_inner_char(chars[probe].1) {
            probe += 1;
        }

        if probe > index + 1 && probe < chars.len() && chars[probe].1 == 'Z' {
            let end = chars.get(probe + 1).map_or(text.len(), |&(i, _)| i);
            let prev_char = (start > 0).then(|| text[..start].chars().last()).flatten();
            let next_char_after_marker = text[end..].chars().next();

            let marker = text[start..end].to_string();
            let number = loop {
                let candidate = counter.to_string();
                counter += 1;
                if !existing.contains(&candidate)
                    && !replacements.iter().any(|r| r.number == candidate)
                {
                    break candidate;
                }
            };

            let trim_trailing_minus = prev_char.is_some_and(is_minus_sign)
                && !matches!(next_char_after_marker, Some(c) if is_percent_sign(c));

            let span_start = if start > 0
                && (is_minus_sign(prev_char.unwrap_or(' '))
                    || is_plus_sign(prev_char.unwrap_or(' ')))
            {
                text[..start]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(start)
            } else {
                start
            };
            let span_end = if matches!(next_char_after_marker, Some(c) if is_percent_sign(c)) {
                let next_idx = chars.get(probe + 1).map_or(end, |&(i, _)| i);
                let percent_byte_len = next_char_after_marker.unwrap().len_utf8();
                next_idx + percent_byte_len
            } else {
                end
            };
            let source_span = text[span_start..span_end].to_string();
            let prev_non_space = text[..start]
                .chars()
                .rev()
                .find(|ch| !is_horizontal_space(*ch));
            let next_non_space = text[end..].chars().find(|ch| !is_horizontal_space(*ch));
            let prev_is_space = prev_char.is_some_and(is_horizontal_space);
            let next_is_space = next_char_after_marker.is_some_and(is_horizontal_space);
            let prev_is_start = prev_char.is_none();
            let next_is_end = next_char_after_marker.is_none();
            let isolated_bare_marker = (prev_is_space && next_is_space)
                || (prev_is_start && next_is_space)
                || (prev_is_space && next_is_end);
            let transport_wrapped = source_span == marker
                && isolated_bare_marker
                && !prev_non_space.is_some_and(is_sign)
                && !next_non_space.is_some_and(is_percent_sign);

            output.push_str(&text[cursor..start]);
            if transport_wrapped {
                output.push('「');
                output.push_str(&number);
                output.push('」');
            } else {
                output.push_str(&number);
            }
            cursor = end;
            replacements.push(ZmReplacement {
                number,
                marker,
                trim_trailing_minus,
                transport_wrapped,
                transport_left_space: transport_wrapped && prev_is_space,
                transport_right_space: transport_wrapped && next_is_space,
                source_span,
            });
            index = probe + 1;
            continue;
        }

        index += 1;
    }

    if replacements.is_empty() {
        return None;
    }

    output.push_str(&text[cursor..]);
    Some(ZmNumberMapping {
        sent_text: output,
        replacements,
    })
}

pub(super) fn restore_zm_number_tokens(text: &str, mapping: &ZmNumberMapping) -> String {
    use rustc_hash::FxHashMap;

    // --- 回復策: ブラケット内漢数字フォールバック ---
    let existing = collect_existing_number_tokens(text);
    let missing_numbers: Vec<&str> = mapping
        .replacements
        .iter()
        .filter(|repl| !existing.contains(&repl.number))
        .map(|repl| repl.number.as_str())
        .collect();

    let text = if !missing_numbers.is_empty() {
        apply_cjk_digit_fallback(text, &missing_numbers)
    } else {
        text.to_string()
    };
    let text = text.as_str();
    // --- ここまで ---

    let mut lookup: FxHashMap<&str, (usize, &ZmReplacement)> = FxHashMap::default();
    for (i, repl) in mapping.replacements.iter().enumerate() {
        lookup.insert(repl.number.as_str(), (i, repl));
    }
    let mut consumed = vec![false; mapping.replacements.len()];

    let mut result = String::with_capacity(text.len() * 2);
    let runs = collect_numeric_runs(text);
    let mut cursor = 0usize;

    for run in runs {
        if let Some(&(idx, repl)) = lookup.get(run.normalized.as_str()) {
            if consumed[idx] {
                result.push_str(&text[cursor..run.start]);
                result.push_str(&run.raw);
                cursor = run.end;
            } else {
                consumed[idx] = true;
                let (span_start, span_end, absorbed_direct_wrapper) = if repl.transport_wrapped {
                    if let Some((start, end)) = direct_wrapper_span_for_numeric_run(text, &run) {
                        (start, end, true)
                    } else {
                        (run.start, run.end, false)
                    }
                } else {
                    (run.start, run.end, false)
                };

                result.push_str(&text[cursor..span_start]);
                result.push_str(&build_transport_restored_surface(
                    text,
                    span_start,
                    span_end,
                    repl,
                    absorbed_direct_wrapper,
                ));
                if repl.trim_trailing_minus {
                    if let Some(next_ch) = text[run.end..].chars().next() {
                        if is_minus_sign(next_ch) {
                            cursor = run.end + next_ch.len_utf8();
                        } else {
                            cursor = run.end;
                        }
                    } else {
                        cursor = run.end;
                    }
                } else {
                    cursor = span_end;
                }
            }
        } else {
            result.push_str(&text[cursor..run.start]);
            result.push_str(&run.raw);
            cursor = run.end;
        }
    }

    result.push_str(&text[cursor..]);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_digit_to_cjk() {
        assert_eq!(ascii_digit_to_cjk('0'), Some('〇'));
        assert_eq!(ascii_digit_to_cjk('1'), Some('一'));
        assert_eq!(ascii_digit_to_cjk('2'), Some('二'));
        assert_eq!(ascii_digit_to_cjk('9'), Some('九'));
        assert_eq!(ascii_digit_to_cjk('a'), None);
        assert_eq!(ascii_digit_to_cjk('２'), None);
    }

    #[test]
    fn test_cjk_digit_to_ascii() {
        assert_eq!(cjk_digit_to_ascii('〇'), Some('0'));
        assert_eq!(cjk_digit_to_ascii('一'), Some('1'));
        assert_eq!(cjk_digit_to_ascii('二'), Some('2'));
        assert_eq!(cjk_digit_to_ascii('九'), Some('9'));
        assert_eq!(cjk_digit_to_ascii('a'), None);
        assert_eq!(cjk_digit_to_ascii('2'), None);
    }

    #[test]
    fn test_apply_cjk_digit_fallback_fullwidth() {
        assert_eq!(
            apply_cjk_digit_fallback("「二」です", &["2"]),
            "「2」です"
        );
    }

    #[test]
    fn test_apply_cjk_digit_fallback_no_bracket() {
        // ブラケットがなくても置換されるようになった
        assert_eq!(
            apply_cjk_digit_fallback("二です", &["2"]),
            "2です"
        );
    }

    #[test]
    fn test_apply_cjk_digit_fallback_halfwidth() {
        assert_eq!(
            apply_cjk_digit_fallback("｢二｣です", &["2"]),
            "｢2｣です"
        );
    }

    #[test]
    fn test_apply_cjk_digit_fallback_skip_non_ascii_digit() {
        assert_eq!(
            apply_cjk_digit_fallback("「十」です", &["10"]),
            "「十」です"
        );
    }

    #[test]
    fn test_apply_cjk_digit_fallback_all_occurrences() {
        // ブラケットの内外問わず、すべての該当文字が置換される
        assert_eq!(
            apply_cjk_digit_fallback("「二」と二です", &["2"]),
            "「2」と2です"
        );
    }

    #[test]
    fn test_apply_cjk_digit_fallback_empty_missing() {
        assert_eq!(
            apply_cjk_digit_fallback("「二」です", &[]),
            "「二」です"
        );
    }
}
