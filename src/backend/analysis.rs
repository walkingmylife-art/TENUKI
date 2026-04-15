use std::sync::{Arc, Mutex};

use crate::backend::processor::{TextProcessor, TranslationContext};
use crate::messages::InputAnalysisSnapshot;

#[derive(Debug, Clone, Default)]
pub struct InputReplayState {
    pub raw_text: Option<String>,
    pub final_output: Option<String>,
    pub result_stale: bool,
    pub dict_hits: usize,
    pub model_calls: usize,
}

pub type SharedInputReplayState = Arc<Mutex<InputReplayState>>;

pub fn normalize_input(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn looks_like_assignment_side(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.chars().any(|ch| !ch.is_ascii()) {
        return true;
    }

    let ascii_letters = trimmed.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    ascii_letters >= 3 || trimmed.contains(' ')
}

fn has_unclosed_tag_prefix(text: &str) -> bool {
    fn is_tag_like_opener(rest: &str) -> bool {
        rest.chars()
            .next()
            .map(|ch| ch.is_ascii_alphabetic() || matches!(ch, '/' | '!' | '?'))
            .unwrap_or(false)
    }

    for (open, close) in [('<', '>'), ('＜', '＞')] {
        if let Some(open_index) = text.rfind(open) {
            let after_open = &text[open_index + open.len_utf8()..];
            if is_tag_like_opener(after_open) && !after_open.contains(close) {
                return true;
            }
        }
    }

    false
}

fn is_assignment_separator(text: &str, index: usize) -> bool {
    let left = text[..index].trim();
    let right = text[index + 1..].trim();

    if left.is_empty() || right.is_empty() {
        return false;
    }

    if matches!(left.chars().last(), Some('<' | '!' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | ':' | '=')) {
        return false;
    }

    if has_unclosed_tag_prefix(left) {
        return false;
    }

    looks_like_assignment_side(left) && looks_like_assignment_side(right)
}

pub fn find_unescaped_assignment_separator(text: &str) -> Option<usize> {
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '=' if is_assignment_separator(text, index) => return Some(index),
            _ => {}
        }
    }

    None
}

fn extract_assignment_source(line: &str) -> String {
    find_unescaped_assignment_separator(line)
        .map(|index| line[..index].to_string())
        .unwrap_or_else(|| line.to_string())
}

fn visible_text_for_context(source: &str, ctx: &TranslationContext) -> String {
    ctx.preview_text(source)
}

fn model_inputs_for_context(ctx: &TranslationContext) -> Vec<String> {
    match ctx.structural_text_tokens() {
        Some(text_tokens) if text_tokens.len() > 1 => text_tokens.to_vec(),
        _ => ctx.parts_to_translate.clone(),
    }
}

pub fn build_snapshot(
    raw_text: &str,
    processor: &dyn TextProcessor,
    final_output: Option<String>,
    result_stale: bool,
    dict_hits: usize,
    model_calls: usize,
) -> InputAnalysisSnapshot {
    let normalized = normalize_input(raw_text);
    let extracted_lines: Vec<String> = normalized.split('\n').map(extract_assignment_source).collect();

    let mut visible_lines = Vec::with_capacity(extracted_lines.len());
    let mut model_inputs = Vec::new();

    for line in &extracted_lines {
        let ctx = processor.preprocess(line);
        visible_lines.push(visible_text_for_context(line, &ctx));
        model_inputs.extend(model_inputs_for_context(&ctx));
    }

    InputAnalysisSnapshot {
        raw_text: normalized,
        extracted_text: extracted_lines.join("\n"),
        visible_text: visible_lines.join("\n"),
        model_inputs,
        final_output,
        result_stale,
        dict_hits,
        model_calls,
    }
}

pub fn record_completed_translation(
    replay_state: &SharedInputReplayState,
    raw_text: &str,
    final_output: &str,
    processor: &dyn TextProcessor,
    dict_hits: usize,
    model_calls: usize,
) -> InputAnalysisSnapshot {
    let normalized = normalize_input(raw_text);
    let final_output = final_output.to_string();

    let snapshot = build_snapshot(
        &normalized,
        processor,
        Some(final_output.clone()),
        false,
        dict_hits,
        model_calls,
    );

    if let Ok(mut state) = replay_state.lock() {
        state.raw_text = Some(normalized);
        state.final_output = Some(final_output);
        state.result_stale = false;
        state.dict_hits = dict_hits;
        state.model_calls = model_calls;
    }

    snapshot
}

pub fn rebuild_latest_snapshot(
    replay_state: &SharedInputReplayState,
    processor: &dyn TextProcessor,
    mark_result_stale: bool,
) -> Option<InputAnalysisSnapshot> {
    let mut state = replay_state.lock().ok()?;
    let raw_text = state.raw_text.clone()?;
    if mark_result_stale && state.final_output.is_some() {
        state.result_stale = true;
    }

    Some(build_snapshot(
        &raw_text,
        processor,
        state.final_output.clone(),
        state.result_stale,
        state.dict_hits,
        state.model_calls,
    ))
}

#[cfg(test)]
mod tests {
    use super::{build_snapshot, find_unescaped_assignment_separator};
    use crate::backend::processor::StructuralProcessor;

    #[test]
    fn ignores_equals_inside_sprite_tag_when_extracting_assignment_source() {
        let text = r#"<sprite name="Half-Elf">=Target"#;
        let separator = find_unescaped_assignment_separator(text).unwrap();

        assert_eq!(&text[..separator], r#"<sprite name="Half-Elf">"#);
    }

    #[test]
    fn builds_snapshot_with_visible_text_and_model_inputs() {
        let processor = StructuralProcessor::new();
        let snapshot = build_snapshot(
            "Buff: (Round)",
            &processor,
            Some("バフ: (ラウンド)".to_string()),
            false,
            0,
            2,
        );

        assert_eq!(snapshot.extracted_text, "Buff: (Round)");
        assert_eq!(snapshot.visible_text, "Buff Round");
        assert_eq!(snapshot.model_inputs, vec!["Buff".to_string(), "Round".to_string()]);
        assert_eq!(snapshot.final_output.as_deref(), Some("バフ: (ラウンド)"));
        assert!(!snapshot.result_stale);
        assert_eq!(snapshot.dict_hits, 0);
        assert_eq!(snapshot.model_calls, 2);
    }
}
