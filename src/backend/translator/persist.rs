use regex;

use super::types::PersistEntry;
use super::zm::{
    collect_numeric_runs, direct_wrapper_span_for_numeric_run, NumericRun, ZmNumberMapping,
    ZmReplacement,
};

const FULLWIDTH_PLUS: char = '\u{FF0B}';
const FULLWIDTH_MINUS: char = '\u{FF0D}';
const MINUS_SIGN: char = '\u{2212}';
const FULLWIDTH_PERCENT: char = '\u{FF05}';
const ZM_CAPTURE_PATTERN: &str = r"([+＋\-－−]?Z[A-Z]+Z[%％]?)";

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

fn is_space_like(ch: char) -> bool {
    ch == ' ' || ch == '\u{3000}'
}

enum SourceNumberSlot<'a> {
    ExistingNumber,
    Zm {
        capture_idx: usize,
        repl: &'a ZmReplacement,
    },
}

struct TransportSpan {
    capture_idx: usize,
    start: usize,
    end: usize,
}

fn expand_transport_span(
    model_output: &str,
    run: &NumericRun,
    repl: &ZmReplacement,
) -> (usize, usize) {
    let source_span = &repl.source_span;
    let src_has_sign = source_span.chars().next().is_some_and(is_sign);
    let src_has_percent = source_span.chars().last().is_some_and(is_percent_sign);

    let wrapper_span = repl
        .transport_wrapped
        .then(|| direct_wrapper_span_for_numeric_run(model_output, run))
        .flatten();

    let span_start = if let Some((wrapper_start, _)) = wrapper_span {
        wrapper_start
    } else if src_has_sign && run.start > 0 {
        let mut scan = run.start;
        let before = model_output[..run.start].chars().rev();
        let mut found = false;
        for c in before {
            if is_sign(c) {
                scan -= c.len_utf8();
                found = true;
                break;
            } else if is_space_like(c) {
                scan -= c.len_utf8();
            } else {
                break;
            }
        }
        if found {
            scan
        } else {
            run.start
        }
    } else {
        run.start
    };

    let span_end = if let Some((_, wrapper_end)) = wrapper_span {
        wrapper_end
    } else if src_has_percent && run.end < model_output.len() {
        let mut scan = run.end;
        let after = model_output[run.end..].chars();
        let mut found = false;
        for c in after {
            if is_percent_sign(c) {
                scan += c.len_utf8();
                found = true;
                break;
            } else if is_space_like(c) {
                scan += c.len_utf8();
            } else {
                break;
            }
        }
        if found {
            scan
        } else {
            run.end
        }
    } else {
        run.end
    };

    (span_start, span_end)
}

pub(super) fn build_zm_persist_entry(
    source: &str,
    mapping: &ZmNumberMapping,
    model_output: &str,
    restored_output: &str,
) -> Option<PersistEntry> {
    let value = restored_output.trim();
    if value.is_empty() {
        log::info!(
            "[PERSIST] skip source=\"{}\" reason=restored_output_empty",
            source
        );
        return None;
    }

    let source_numeric_runs = collect_numeric_runs(source);
    let output_numeric_runs = collect_numeric_runs(model_output);

    let mut source_spans: Vec<(usize, usize, usize, &ZmReplacement)> = Vec::new();
    let mut search_from = 0usize;
    for (capture_idx, repl) in mapping.replacements.iter().enumerate() {
        let Some(relative_pos) = source[search_from..].find(&repl.source_span) else {
            log::info!(
                "[PERSIST] skip source=\"{}\" reason=source_span_not_found source_span=\"{}\"",
                source,
                repl.source_span
            );
            return None;
        };

        let pos = search_from + relative_pos;
        let end = pos + repl.source_span.len();
        source_spans.push((pos, end, capture_idx, repl));
        search_from = end;
    }

    let mut source_slots = Vec::new();
    let mut number_i = 0usize;
    let mut zm_i = 0usize;
    while number_i < source_numeric_runs.len() || zm_i < source_spans.len() {
        let next_number_start = source_numeric_runs.get(number_i).map(|run| run.start);
        let next_zm_start = source_spans.get(zm_i).map(|(start, _, _, _)| *start);

        match (next_number_start, next_zm_start) {
            (Some(n), Some(z)) if n < z => {
                source_slots.push(SourceNumberSlot::ExistingNumber);
                number_i += 1;
            }
            (Some(_), Some(_)) => {
                let (_, _, capture_idx, repl) = source_spans[zm_i];
                source_slots.push(SourceNumberSlot::Zm { capture_idx, repl });
                zm_i += 1;
            }
            (Some(_), None) => {
                source_slots.push(SourceNumberSlot::ExistingNumber);
                number_i += 1;
            }
            (None, Some(_)) => {
                let (_, _, capture_idx, repl) = source_spans[zm_i];
                source_slots.push(SourceNumberSlot::Zm { capture_idx, repl });
                zm_i += 1;
            }
            (None, None) => break,
        }
    }

    if output_numeric_runs.len() != source_slots.len() {
        log::info!(
            "[PERSIST] skip source=\"{}\" reason=number_slot_count_mismatch output_numbers={} source_slots={} model_output=\"{}\"",
            source,
            output_numeric_runs.len(),
            source_slots.len(),
            model_output
        );
        return None;
    }

    let mut spans: Vec<TransportSpan> = Vec::new();
    for (slot, run) in source_slots.iter().zip(output_numeric_runs.iter()) {
        if let SourceNumberSlot::Zm { capture_idx, repl } = slot {
            let (span_start, span_end) = expand_transport_span(model_output, run, repl);
            spans.push(TransportSpan {
                capture_idx: *capture_idx,
                start: span_start,
                end: span_end,
            });
        }
    }

    let mut replacement = String::new();
    let mut last_end = 0;
    for span in &spans {
        replacement.push_str(&model_output[last_end..span.start].replace('$', "$$"));
        replacement.push('$');
        replacement.push_str(&(span.capture_idx + 1).to_string());
        last_end = span.end;
    }
    replacement.push_str(&model_output[last_end..].replace('$', "$$"));

    let mut pattern_parts = String::new();
    let mut last_end = 0;
    for (pos, end, _, _) in &source_spans {
        pattern_parts.push_str(&regex::escape(&source[last_end..*pos]));
        pattern_parts.push_str(ZM_CAPTURE_PATTERN);
        last_end = *end;
    }
    pattern_parts.push_str(&regex::escape(&source[last_end..]));

    let pattern = format!("^{}$", pattern_parts);

    if let Err(error) = regex::Regex::new(&pattern) {
        log::info!(
            "[PERSIST] skip source=\"{}\" reason=regex_compile_failed pattern=\"{}\" error=\"{}\"",
            source,
            pattern,
            error
        );
        return None;
    }

    log::info!(
        "[PERSIST] regex_built source=\"{}\" pattern=\"{}\" replacement=\"{}\"",
        source,
        pattern,
        replacement
    );

    Some(PersistEntry::Regex {
        pattern,
        replacement,
    })
}
