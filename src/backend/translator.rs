//! Translation coordinator.

mod cache;
mod client;
mod helpers;
mod lang;

pub use cache::{NewEntriesCache, TranslationCache};
pub use client::{HttpLlmClient, LlmClient};
pub use helpers::{apply_wrap, clean_model_output};
pub use lang::build_lang_prefix;

use std::time::Duration;

const SEPARATOR_CHARS: [char; 4] = [':', '：', ';', '；'];
const BRACKET_PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('（', '）'),
    ('［', '］'),
    ('｛', '｝'),
    ('〈', '〉'),
    ('《', '》'),
    ('【', '】'),
];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum LogEvent {
    DictHit {
        original: String,
        translated: String,
        elapsed_secs: f64,
    },
    PreModelCall {
        original: String,
    },
    ModelResult {
        original: String,
        translated: String,
        elapsed_secs: f64,
    },
    Error {
        message: String,
    },
}

impl LogEvent {
    pub fn dict_hit(original: &str, translated: &str, elapsed: Duration) -> Self {
        Self::DictHit {
            original: original.to_string(),
            translated: translated.to_string(),
            elapsed_secs: elapsed.as_secs_f64(),
        }
    }

    pub fn pre_model_call(original: &str) -> Self {
        Self::PreModelCall {
            original: original.to_string(),
        }
    }

    pub fn model_result(original: &str, translated: &str, elapsed: Duration) -> Self {
        Self::ModelResult {
            original: original.to_string(),
            translated: translated.to_string(),
            elapsed_secs: elapsed.as_secs_f64(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TranslationStats {
    pub dict_hits: usize,
    pub model_calls: usize,
}

impl TranslationStats {
    pub fn dict_hit() -> Self {
        Self {
            dict_hits: 1,
            model_calls: 0,
        }
    }
    pub fn model_call() -> Self {
        Self {
            dict_hits: 0,
            model_calls: 1,
        }
    }
    pub fn merge(&mut self, other: &Self) {
        self.dict_hits += other.dict_hits;
        self.model_calls += other.model_calls;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationSettings {
    pub enable_model_wrap: bool,
    pub model_wrap_min_chars: usize,
    pub model_wrap_min_tail_chars: usize,
    pub enable_model_symbol_cleanup: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranslationResult {
    pub text: String,
    pub new_entries: Vec<(String, String)>,
    pub new_pattern_entries: Vec<(String, String)>,
    pub stats: TranslationStats,
    pub logs: Vec<LogEvent>,
}

impl TranslationResult {
    pub fn empty(text: String) -> Self {
        Self {
            text,
            new_entries: Vec::new(),
            new_pattern_entries: Vec::new(),
            stats: TranslationStats::default(),
            logs: Vec::new(),
        }
    }

    pub fn from_dict_hit(text: String, original: &str, elapsed: Duration) -> Self {
        let log = LogEvent::dict_hit(original, &text, elapsed);
        Self {
            text,
            new_entries: Vec::new(),
            new_pattern_entries: Vec::new(),
            stats: TranslationStats::dict_hit(),
            logs: vec![log],
        }
    }

    pub fn from_model_call_success(text: String, original: &str, elapsed: Duration) -> Self {
        let model_log = LogEvent::model_result(original, &text, elapsed);
        Self {
            text,
            new_entries: Vec::new(),
            new_pattern_entries: Vec::new(),
            stats: TranslationStats::model_call(),
            logs: vec![LogEvent::pre_model_call(original), model_log],
        }
    }

    pub fn from_model_call_failure(original: &str) -> Self {
        Self {
            text: original.to_string(),
            new_entries: Vec::new(),
            new_pattern_entries: Vec::new(),
            stats: TranslationStats::model_call(),
            logs: vec![
                LogEvent::pre_model_call(original),
                LogEvent::Error {
                    message: format!("LLM call failed for: {}", original),
                },
            ],
        }
    }

    pub fn absorb(&mut self, other: Self) {
        self.new_entries.extend(other.new_entries);
        self.new_pattern_entries.extend(other.new_pattern_entries);
        self.stats.merge(&other.stats);
        self.logs.extend(other.logs);
    }
}

// ---------------------------------------------------------------------------
// Internal tokenizer types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum StructureToken {
    Text(String),
    Delimiter(DelimiterToken),
    Bracket(BracketSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Newline,
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
struct DelimiterToken {
    text: String,
    kind: DelimiterKind,
}

#[derive(Debug, Clone, PartialEq)]
struct BracketSlot {
    open: char,
    close: char,
    inner_left_spaces: String,
    inner_core: String,
    inner_right_spaces: String,
}

#[derive(Debug, Clone, PartialEq)]
struct LinePlan {
    segments: Vec<Vec<StructureToken>>,
    separators: Vec<DelimiterToken>,
}

// ---------------------------------------------------------------------------
// ZM-number substitution
//
// ZM マーカー（例: ZABZ）を数字に置き換えてモデルに送り、
// モデル出力から数字を元のマーカーに戻す。
// スペースは一切操作しない。sent_text に含まれるスペースは
// そのままモデルに渡り、復元時もそのまま通過する。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZmNumberMapping {
    sent_text: String,
    replacements: Vec<(String, String)>, // (number, original_marker)
}

fn is_zm_inner_char(ch: char) -> bool {
    ch.is_ascii_uppercase() && ch != 'Z'
}

fn collect_existing_number_tokens(text: &str) -> rustc_hash::FxHashSet<String> {
    let mut numbers = rustc_hash::FxHashSet::default();
    let mut chars = text.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if !ch.is_ascii_digit() {
            chars.next();
            continue;
        }
        chars.next();
        while matches!(chars.peek(), Some(&(_, c)) if c.is_ascii_digit()) {
            chars.next();
        }
        let end = chars.peek().map_or(text.len(), |&(i, _)| i);
        numbers.insert(text[start..end].to_string());
    }

    numbers
}

/// ZM マーカーを数字に置き換えた sent_text とマッピングを返す。
/// スペースは操作しない。マーカーの位置だけを数字で置換する。
fn build_zm_number_mapping(text: &str) -> Option<ZmNumberMapping> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut replacements: Vec<(String, String)> = Vec::new();
    let mut index = 0usize;
    let existing = collect_existing_number_tokens(text);
    let mut counter = 1usize;

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

            // ±ZMZ 形式の算術式はスキップ
            if matches!(prev_char, Some('+') | Some('-')) {
                index = probe + 1;
                continue;
            }

            let marker = text[start..end].to_string();
            let number = loop {
                let candidate = counter.to_string();
                counter += 1;
                if !existing.contains(&candidate)
                    && !replacements.iter().any(|(used, _)| used == &candidate)
                {
                    break candidate;
                }
            };

            output.push_str(&text[cursor..start]);
            output.push_str(&number);
            cursor = end;
            replacements.push((number, marker));
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

/// モデル出力中の数字を元の ZM マーカーに戻す。
///
/// - O(n) スキャン、O(1) ルックアップ（FxHashMap）
/// - atomic digit run で "1" が "10" の先頭にマッチするバグを防ぐ
/// - consumed フラグで各スロットを左から右へ 1 回のみ使用
/// - スペースは一切操作しない（モデル出力のスペースをそのまま保持）
fn restore_zm_number_tokens(text: &str, mapping: &ZmNumberMapping) -> String {
    use rustc_hash::FxHashMap;

    let mut lookup: FxHashMap<&str, (usize, &str)> = FxHashMap::default();
    for (i, (num, marker)) in mapping.replacements.iter().enumerate() {
        lookup.insert(num.as_str(), (i, marker.as_str()));
    }
    let mut consumed = vec![false; mapping.replacements.len()];

    let mut result = String::with_capacity(text.len() * 2);
    let mut chars = text.char_indices().peekable();

    while let Some(&(byte_idx, ch)) = chars.peek() {
        if !ch.is_ascii_digit() {
            result.push(ch);
            chars.next();
            continue;
        }

        // 数字列を一括トークンとして消費（"1" が "10" の先頭にマッチするのを防ぐ）
        let start = byte_idx;
        chars.next();
        while matches!(chars.peek(), Some(&(_, c)) if c.is_ascii_digit()) {
            chars.next();
        }
        let end = chars.peek().map_or(text.len(), |&(i, _)| i);
        let token = &text[start..end];

        if let Some(&(idx, marker)) = lookup.get(token) {
            if consumed[idx] {
                result.push_str(token);
            } else {
                consumed[idx] = true;
                result.push_str(marker);
            }
        } else {
            result.push_str(token);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Bracket helpers
// ---------------------------------------------------------------------------

fn matching_close_bracket(open: char) -> Option<char> {
    BRACKET_PAIRS
        .iter()
        .find_map(|&(l, r)| (l == open).then_some(r))
}

fn matching_open_bracket(close: char) -> Option<char> {
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

/// テキストからトップレベルのブラケットブロックを BracketSlot トークンとして抽出する。
/// マッチしないブラケット（閉じなし・開きなし）はプレーンテキストとして素通しする。
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
        if !stack.is_empty() {
            continue;
        }

        if start_byte > cursor {
            tokens.push(StructureToken::Text(text[cursor..start_byte].to_string()));
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

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

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

/// テキストトークンをセパレータ・改行で分割し、隣接スペースを
/// DelimiterToken に吸着させる。これによりセグメントの両端にスペースが
/// 残らず、辞書検索・モデル呼び出しのキーが安定する。
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

fn tokenize_structure(text: &str) -> Vec<StructureToken> {
    let mut tokens = Vec::new();
    for token in extract_bracket_slots(text) {
        match token {
            StructureToken::Text(t) => tokens.extend(split_text_token(&t)),
            other => tokens.push(other),
        }
    }
    tokens
}

fn split_lines(tokens: Vec<StructureToken>) -> (Vec<LinePlan>, Vec<DelimiterToken>) {
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

// ---------------------------------------------------------------------------
// Reconstruction helpers
// ---------------------------------------------------------------------------

fn reconstruct_bracket(slot: &BracketSlot, inner_text: &str) -> String {
    format!(
        "{}{}{}{}{}",
        slot.open, slot.inner_left_spaces, inner_text, slot.inner_right_spaces, slot.close,
    )
}

fn reconstruct_segment(tokens: &[StructureToken]) -> String {
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

fn dedupe_entries(entries: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = rustc_hash::FxHashSet::default();
    entries
        .into_iter()
        .filter(|pair| seen.insert(pair.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Translation logic
// ---------------------------------------------------------------------------

fn translate_model_only(
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
    let model_input = zm_mapping.as_ref().map_or(text, |m| m.sent_text.as_str());
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
            Some(m) => restore_zm_number_tokens(&cleaned, m),
            None => cleaned,
        };
        TranslationResult::from_model_call_success(translated, model_input, elapsed)
    } else {
        TranslationResult::from_model_call_failure(model_input)
    }
}

fn translate_fragment<F>(
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
    if fragment.trim().is_empty() {
        return TranslationResult::empty(fragment.to_string());
    }

    let start = std::time::Instant::now();
    if let Some(hit) = lookup(fragment) {
        return TranslationResult::from_dict_hit(hit, fragment, start.elapsed());
    }

    let mut result = translate_model_only(fragment, prefix, tgt_lang, llm_client, settings);
    if result.stats.model_calls > 0 {
        let value = result.text.trim().to_string();
        if !value.is_empty() {
            result.new_entries.push((fragment.to_string(), value));
        }
    }
    result
}

fn translate_bracket_slot<F>(
    slot: &BracketSlot,
    lookup: &F,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult
where
    F: Fn(&str) -> Option<String> + Clone,
{
    let inner_settings = TranslationSettings {
        enable_model_wrap: settings.enable_model_wrap,
        ..settings
    };
    let inner = translate_text_internal(
        &slot.inner_core,
        lookup,
        prefix,
        tgt_lang,
        llm_client,
        inner_settings,
    );
    let mut result = inner;
    let inner_text = result.text.clone();
    result.text = reconstruct_bracket(slot, &inner_text);
    result
}

fn append_translated_piece(
    result: &mut TranslationResult,
    rendered: &mut String,
    piece: TranslationResult,
) {
    rendered.push_str(&piece.text);
    result.absorb(piece);
}

fn translate_segment_tokens<F>(
    tokens: &[StructureToken],
    lookup: &F,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult
where
    F: Fn(&str) -> Option<String> + Clone,
{
    if let [StructureToken::Bracket(slot)] = tokens {
        return translate_bracket_slot(slot, lookup, prefix, tgt_lang, llm_client, settings);
    }

    if !tokens
        .iter()
        .any(|token| matches!(token, StructureToken::Bracket(_)))
    {
        return translate_fragment(
            &reconstruct_segment(tokens),
            lookup,
            prefix,
            tgt_lang,
            llm_client,
            settings,
        );
    }

    // 混在セグメントは、周囲のテキストと括弧内を分けて順に処理する。
    let mut accumulated = TranslationResult::empty(String::new());
    let mut rendered = String::new();
    let mut buffered_text = String::new();

    for token in tokens {
        match token {
            StructureToken::Text(t) => buffered_text.push_str(t),
            StructureToken::Bracket(slot) => {
                if !buffered_text.is_empty() {
                    let chunk = std::mem::take(&mut buffered_text);
                    let translated =
                        translate_fragment(&chunk, lookup, prefix, tgt_lang, llm_client, settings);
                    append_translated_piece(&mut accumulated, &mut rendered, translated);
                }

                let translated =
                    translate_bracket_slot(slot, lookup, prefix, tgt_lang, llm_client, settings);
                append_translated_piece(&mut accumulated, &mut rendered, translated);
            }
            StructureToken::Delimiter(_) => {}
        }
    }

    if !buffered_text.is_empty() {
        let chunk = std::mem::take(&mut buffered_text);
        let translated = translate_fragment(&chunk, lookup, prefix, tgt_lang, llm_client, settings);
        append_translated_piece(&mut accumulated, &mut rendered, translated);
    }

    accumulated.text = rendered;
    accumulated
}

fn translate_text_internal<F>(
    text: &str,
    lookup: &F,
    prefix: &str,
    tgt_lang: &str,
    llm_client: &dyn LlmClient,
    settings: TranslationSettings,
) -> TranslationResult
where
    F: Fn(&str) -> Option<String> + Clone,
{
    if text.is_empty() {
        return TranslationResult::empty(String::new());
    }

    let (lines, newline_tokens) = split_lines(tokenize_structure(text));
    let mut accumulated = TranslationResult::empty(String::new());
    let mut rendered_lines = Vec::with_capacity(lines.len());

    for line in &lines {
        let mut segment_texts = Vec::with_capacity(line.segments.len());
        for seg in &line.segments {
            let result =
                translate_segment_tokens(seg, lookup, prefix, tgt_lang, llm_client, settings);
            segment_texts.push(result.text.clone());
            accumulated.absorb(result);
        }

        let mut joined = String::new();
        for (i, seg) in segment_texts.iter().enumerate() {
            joined.push_str(seg);
            if let Some(d) = line.separators.get(i) {
                joined.push_str(&d.text);
            }
        }

        let rendered = if settings.enable_model_wrap {
            apply_wrap(
                &joined,
                true,
                settings.model_wrap_min_chars,
                settings.model_wrap_min_tail_chars,
            )
        } else {
            joined
        };
        rendered_lines.push(rendered);
    }

    let mut final_text = String::new();
    for (i, line) in rendered_lines.iter().enumerate() {
        final_text.push_str(line);
        if let Some(d) = newline_tokens.get(i) {
            final_text.push_str(&d.text);
        }
    }

    accumulated.new_entries = dedupe_entries(accumulated.new_entries);
    accumulated.text = final_text;
    accumulated
}

pub fn translate_chunk<F>(
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
    if chunk.trim().is_empty() {
        return TranslationResult::empty(chunk.to_string());
    }
    translate_text_internal(chunk, &lookup, prefix, tgt_lang, llm_client, settings)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockLlmClient {
        calls: Mutex<Vec<String>>,
        responses: Mutex<Vec<String>>,
    }

    impl MockLlmClient {
        fn with_responses(values: &[&str]) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(values.iter().map(|v| v.to_string()).collect()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl LlmClient for MockLlmClient {
        fn translate_sync(&self, text: &str, _prefix: &str) -> Option<String> {
            self.calls.lock().unwrap().push(text.to_string());
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() {
                None
            } else {
                Some(r.remove(0))
            }
        }
    }

    fn test_settings() -> TranslationSettings {
        TranslationSettings {
            enable_model_wrap: true,
            model_wrap_min_chars: 60,
            model_wrap_min_tail_chars: 10,
            enable_model_symbol_cleanup: true,
        }
    }

    fn mapping(pairs: &[(&str, &str)]) -> ZmNumberMapping {
        ZmNumberMapping {
            sent_text: String::new(),
            replacements: pairs
                .iter()
                .map(|(n, m)| (n.to_string(), m.to_string()))
                .collect(),
        }
    }

    // --- restore_zm_number_tokens -------------------------------------------

    // "1" が "10" の先頭にマッチしないこと
    #[test]
    fn restore_does_not_match_digit_prefix() {
        let m = mapping(&[("1", "ZAZ"), ("10", "ZBZ")]);
        assert_eq!(restore_zm_number_tokens("1 10", &m), "ZAZ ZBZ");
    }

    // スペースは操作しない（モデル出力のスペースをそのまま保持）
    #[test]
    fn restore_preserves_spaces_as_is() {
        let m = mapping(&[("1", "ZAZ")]);
        assert_eq!(restore_zm_number_tokens("foo 1 bar", &m), "foo ZAZ bar");
        assert_eq!(restore_zm_number_tokens("foo  1  bar", &m), "foo  ZAZ  bar");
        assert_eq!(restore_zm_number_tokens("foo1bar", &m), "fooZAZbar");
    }

    // マッピングにない数字はそのまま通過
    #[test]
    fn restore_passes_through_unmatched_token() {
        let m = mapping(&[("1", "ZAZ")]);
        assert_eq!(
            restore_zm_number_tokens("no numbers here", &m),
            "no numbers here"
        );
        assert_eq!(restore_zm_number_tokens("value is 99", &m), "value is 99");
    }

    // 同じ数字が 2 回出ても最初の 1 回だけ置換
    #[test]
    fn restore_consumes_slot_at_most_once() {
        let m = mapping(&[("1", "ZAZ")]);
        assert_eq!(restore_zm_number_tokens("chapter 1 1", &m), "chapter ZAZ 1");
    }

    // 隣接トークン・スペースはそのまま
    #[test]
    fn restore_adjacent_tokens_preserve_spaces() {
        let m = mapping(&[("1", "ZAZ"), ("10", "ZBZ")]);
        assert_eq!(restore_zm_number_tokens("1 10", &m), "ZAZ ZBZ");
        assert_eq!(restore_zm_number_tokens("1  10", &m), "ZAZ  ZBZ");
        assert_eq!(restore_zm_number_tokens("1  10  1", &m), "ZAZ  ZBZ  1");
    }

    // CJK 全角スペースもそのまま通過
    #[test]
    fn restore_preserves_cjk_spaces() {
        let m = mapping(&[("1", "ZAZ"), ("10", "ZBZ")]);
        assert_eq!(
            restore_zm_number_tokens("1\u{3000}10\u{3000}1", &m),
            "ZAZ\u{3000}ZBZ\u{3000}1"
        );
    }

    // 連結数字は分割しない
    #[test]
    fn restore_does_not_split_concatenated_digits() {
        let m = mapping(&[("1", "ZAZ"), ("10", "ZBZ")]);
        assert_eq!(restore_zm_number_tokens("110", &m), "110");
    }

    // --- translate_chunk ----------------------------------------------------

    #[test]
    fn split_fragments_are_sent_as_is() {
        let llm = MockLlmClient::with_responses(&["Later", "Cycle"]);
        let result = translate_chunk("Next;Turn", |_| None, "prefix", "en", &llm, test_settings());

        assert_eq!(llm.calls(), vec!["Next", "Turn"]);
        assert_eq!(result.text, "Later;Cycle");
        assert_eq!(
            result.new_entries,
            vec![
                ("Next".to_string(), "Later".to_string()),
                ("Turn".to_string(), "Cycle".to_string()),
            ]
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
        assert_eq!(
            result.new_entries,
            vec![("Next".to_string(), "Quest".to_string())]
        );
    }

    #[test]
    fn mixed_bracket_segment_reprocesses_inner_text() {
        let llm = MockLlmClient::with_responses(&["Quest"]);
        let result = translate_chunk(
            "foo(Start;Next)bar",
            |key| match key {
                "foo" => Some("Foo".to_string()),
                "Start" => Some("Begin".to_string()),
                "bar" => Some("Bar".to_string()),
                _ => None,
            },
            "prefix",
            "en",
            &llm,
            test_settings(),
        );

        assert_eq!(llm.calls(), vec!["Next"]);
        assert_eq!(result.text, "Foo(Begin;Quest)Bar");
        assert_eq!(
            result.new_entries,
            vec![("Next".to_string(), "Quest".to_string())]
        );
    }
}
