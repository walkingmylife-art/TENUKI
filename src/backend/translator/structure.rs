const SEPARATOR_CHARS: [char; 4] = [':', '\u{FF1A}', ';', '\u{FF1B}'];
const BRACKET_PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('\u{FF08}', '\u{FF09}'),
    ('\u{FF3B}', '\u{FF3D}'),
    ('\u{FF5B}', '\u{FF5D}'),
    ('\u{3008}', '\u{3009}'),
    ('\u{300A}', '\u{300B}'),
    ('\u{3010}', '\u{3011}'),
];

#[derive(Debug, Clone, PartialEq)]
pub(super) enum StructureToken {
    Text(String),
    Delimiter(DelimiterToken),
    Bracket(BracketSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DelimiterKind {
    Newline,
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DelimiterToken {
    pub text: String,
    pub kind: DelimiterKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct BracketSlot {
    pub open: char,
    pub close: char,
    pub inner_left_spaces: String,
    pub inner_core: String,
    pub inner_right_spaces: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LinePlan {
    pub segments: Vec<Vec<StructureToken>>,
    pub separators: Vec<DelimiterToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RawPart {
    Text(String),
    ProtectedAngle(String),
    Newline(String),
}

pub(super) fn split_protected_angles_and_newlines(input: &str) -> Vec<RawPart> {
    let chars: Vec<char> = input.chars().collect();
    let mut parts = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];

        if ch == '\r' || ch == '\n' {
            if !buf.is_empty() {
                parts.push(RawPart::Text(std::mem::take(&mut buf)));
            }
            if ch == '\r' && chars.get(i + 1) == Some(&'\n') {
                parts.push(RawPart::Newline("\r\n".to_string()));
                i += 2;
            } else {
                parts.push(RawPart::Newline(ch.to_string()));
                i += 1;
            }
            continue;
        }

        if ch == '<' {
            if let Some(end) = find_protected_angle_end(&chars, i) {
                if !buf.is_empty() {
                    parts.push(RawPart::Text(std::mem::take(&mut buf)));
                }
                let span: String = chars[i..=end].iter().collect();
                parts.push(RawPart::ProtectedAngle(span));
                i = end + 1;
                continue;
            }
        }

        buf.push(ch);
        i += 1;
    }

    if !buf.is_empty() {
        parts.push(RawPart::Text(buf));
    }

    parts
}

fn find_protected_angle_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;

    while i < chars.len() {
        match chars[i] {
            '>' => return Some(i),
            '\r' | '\n' => return None,
            '<' => return None,
            _ => i += 1,
        }
    }

    None
}

pub(super) fn matching_close_bracket(open: char) -> Option<char> {
    BRACKET_PAIRS
        .iter()
        .find_map(|&(l, r)| (l == open).then_some(r))
}

pub(super) fn matching_open_bracket(close: char) -> Option<char> {
    BRACKET_PAIRS
        .iter()
        .find_map(|&(l, r)| (r == close).then_some(l))
}

fn split_inner_spaces(text: &str) -> (String, String, String) {
    let trimmed_start = text.trim_start_matches(' ');
    let left_len = text.len() - trimmed_start.len();
    let trimmed = trimmed_start.trim_end_matches(' ');
    let right_len = trimmed_start.len() - trimmed.len();
    (
        text[..left_len].to_string(),
        trimmed.to_string(),
        trimmed_start[trimmed.len()..trimmed.len() + right_len].to_string(),
    )
}

fn extract_bracket_slots(text: &str) -> Vec<StructureToken> {
    let mut tokens = Vec::new();
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut cursor = 0usize;

    for (byte_index, ch) in text.char_indices() {
        if matching_close_bracket(ch).is_some() {
            stack.push((ch, byte_index));
            continue;
        }
        let Some(expected_open) = matching_open_bracket(ch) else {
            continue;
        };
        let Some((open, start_byte)) = stack.pop() else {
            continue;
        };
        if open != expected_open {
            stack.clear();
            continue;
        }
        if !stack.is_empty()
            && matching_close_bracket(open) == matching_close_bracket(stack.last().unwrap().0)
        {
            continue;
        }

        let unmatched_from = stack.last().map(|(_, pos)| *pos).unwrap_or(cursor);
        stack.clear();

        if start_byte > unmatched_from {
            tokens.push(StructureToken::Text(
                text[unmatched_from..start_byte].to_string(),
            ));
        }

        let end_byte = byte_index + ch.len_utf8();
        let content = &text[start_byte + open.len_utf8()..byte_index];
        let (inner_left_spaces, inner_core, inner_right_spaces) = split_inner_spaces(content);
        tokens.push(StructureToken::Bracket(BracketSlot {
            open,
            close: ch,
            inner_left_spaces,
            inner_core,
            inner_right_spaces,
        }));
        cursor = end_byte;
        stack.clear();
    }

    if cursor < text.len() {
        tokens.push(StructureToken::Text(text[cursor..].to_string()));
    }
    if tokens.is_empty() {
        vec![StructureToken::Text(String::new())]
    } else {
        tokens
    }
}

fn pop_trailing_spaces(text: &mut String) -> String {
    let trimmed_len = text.trim_end_matches(' ').len();
    let spaces = text[trimmed_len..].to_string();
    text.truncate(trimmed_len);
    spaces
}

fn take_leading_spaces(text: &str, start: usize) -> (String, usize) {
    let mut spaces = String::new();
    let mut consumed = 0usize;
    for ch in text[start..].chars() {
        if ch != ' ' {
            break;
        }
        spaces.push(ch);
        consumed += ch.len_utf8();
    }
    (spaces, consumed)
}

fn split_text_token(text: &str) -> Vec<StructureToken> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let (_, ch) = chars[index];

        let (delimiter, kind, advance_chars) =
            if ch == '\r' && chars.get(index + 1).map(|(_, c)| *c) == Some('\n') {
                ("\r\n".to_string(), DelimiterKind::Newline, 2usize)
            } else if ch == '\n' {
                ("\n".to_string(), DelimiterKind::Newline, 1usize)
            } else if SEPARATOR_CHARS.contains(&ch) {
                (ch.to_string(), DelimiterKind::Separator, 1usize)
            } else {
                current.push(ch);
                index += 1;
                continue;
            };

        let left_spaces = pop_trailing_spaces(&mut current);
        if !current.is_empty() {
            tokens.push(StructureToken::Text(std::mem::take(&mut current)));
        }

        let advance_bytes: usize = (0..advance_chars)
            .map(|o| chars[index + o].1.len_utf8())
            .sum();
        let delimiter_start = chars[index].0;
        let delimiter_end = delimiter_start + advance_bytes;
        let (right_spaces, consumed) = take_leading_spaces(text, delimiter_end);

        tokens.push(StructureToken::Delimiter(DelimiterToken {
            text: format!("{}{}{}", left_spaces, delimiter, right_spaces),
            kind,
        }));

        index += advance_chars;
        while index < chars.len() && chars[index].0 < delimiter_end + consumed {
            index += 1;
        }
    }

    if !current.is_empty() {
        tokens.push(StructureToken::Text(current));
    }
    tokens
}

pub(super) fn tokenize_structure(text: &str) -> Vec<StructureToken> {
    let mut tokens = Vec::new();
    for token in extract_bracket_slots(text) {
        match token {
            StructureToken::Text(t) => tokens.extend(split_text_token(&t)),
            other => tokens.push(other),
        }
    }
    tokens
}

pub(super) fn split_lines(tokens: Vec<StructureToken>) -> (Vec<LinePlan>, Vec<DelimiterToken>) {
    let mut lines = Vec::new();
    let mut newline_tokens = Vec::new();
    let mut current_line = Vec::new();

    for token in tokens {
        match token {
            StructureToken::Delimiter(d) if d.kind == DelimiterKind::Newline => {
                lines.push(build_line_plan(std::mem::take(&mut current_line)));
                newline_tokens.push(d);
            }
            other => current_line.push(other),
        }
    }
    lines.push(build_line_plan(current_line));
    (lines, newline_tokens)
}

fn build_line_plan(tokens: Vec<StructureToken>) -> LinePlan {
    let mut segments = Vec::new();
    let mut separators = Vec::new();
    let mut current = Vec::new();

    for token in tokens {
        match token {
            StructureToken::Delimiter(d) if d.kind == DelimiterKind::Separator => {
                segments.push(std::mem::take(&mut current));
                separators.push(d);
            }
            other => current.push(other),
        }
    }
    segments.push(current);
    LinePlan {
        segments,
        separators,
    }
}

pub(super) fn reconstruct_bracket(slot: &BracketSlot, inner_text: &str) -> String {
    format!(
        "{}{}{}{}{}",
        slot.open, slot.inner_left_spaces, inner_text, slot.inner_right_spaces, slot.close,
    )
}

pub(super) fn reconstruct_segment(tokens: &[StructureToken]) -> String {
    let mut text = String::new();
    for token in tokens {
        match token {
            StructureToken::Text(t) => text.push_str(t),
            StructureToken::Bracket(s) => text.push_str(&reconstruct_bracket(s, &s.inner_core)),
            StructureToken::Delimiter(_) => {}
        }
    }
    text
}
