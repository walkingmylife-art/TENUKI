//! Regex dictionary rule types and compilation.

use std::path::Path;

use regex::Regex;
use rustc_hash::FxHashMap;

use super::{DictBuildReport, EntryOrigin};

#[derive(Clone)]
pub(crate) struct RegexRule {
    pub(crate) pattern: Regex,
    pub(crate) replacement: String,
    #[allow(dead_code)]
    pub(crate) source_pattern: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RawRegexEntry {
    pub(crate) pattern: String,
    pub(crate) replacement: String,
    pub(crate) origin: EntryOrigin,
    pub(crate) order: u64,
}

pub(crate) fn is_tenuki_regex_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("Tenuki.regex.txt"))
        .unwrap_or(false)
}

pub(crate) fn try_parse_regex_rule(line: &str) -> Option<(String, String)> {
    if !line.starts_with("r:\"") {
        return None;
    }
    let after_prefix = &line[3..];

    let close_quote = after_prefix.find('"')?;
    let pattern = after_prefix[..close_quote].to_string();
    let after_quote = &after_prefix[close_quote + 1..];

    if !after_quote.starts_with('=') {
        return None;
    }
    let replacement = after_quote[1..].to_string();
    Some((pattern, replacement))
}

pub(crate) fn compile_regex_entries(
    raw_regex: &[RawRegexEntry],
    report: &mut DictBuildReport,
) -> Vec<RegexRule> {
    let mut by_pattern: FxHashMap<String, RawRegexEntry> = FxHashMap::default();
    for entry in raw_regex {
        if by_pattern
            .insert(entry.pattern.clone(), entry.clone())
            .is_some()
        {
            report.duplicate_regex_dropped += 1;
        }
    }

    let mut accepted = by_pattern.into_values().collect::<Vec<_>>();
    accepted.sort_by_key(|entry| entry.order);

    let mut rules = Vec::new();
    for entry in accepted {
        match Regex::new(&entry.pattern) {
            Ok(pattern) => rules.push(RegexRule {
                pattern,
                replacement: entry.replacement,
                source_pattern: entry.pattern,
            }),
            Err(e) => {
                report.warnings.push(format!(
                    "× [REGEX] {}:{} regex error — {}",
                    entry.origin.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    entry.origin.line_number,
                    entry.pattern
                ));
                log::warn!(
                    "[REGEX_RULE] compile failed at {}:{} — {} ({})",
                    entry.origin.path.display(),
                    entry.origin.line_number,
                    entry.pattern,
                    e
                );
            }
        }
    }
    report.accepted_regex = rules.len();
    rules
}
