//! Split dictionary rule types and loading.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use regex::Regex;

#[derive(Clone)]
pub(crate) struct SplitRule {
    pub(crate) pattern: Regex,
    pub(crate) replacement: String,
    #[allow(dead_code)]
    pub(crate) source_pattern: String,
}

#[derive(Clone, Debug)]
pub struct SplitResult {
    pub full_match_start: usize,
    pub full_match_end: usize,
    pub inner_groups: Vec<Option<(usize, usize)>>,
    pub replacement: String,
}

use super::sorted_dir_entries;

pub(crate) fn is_tenuki_split_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.to_lowercase().starts_with("tenuki.split")
                && name.to_lowercase().ends_with(".txt")
        })
        .unwrap_or(false)
}

pub(crate) fn try_parse_split_rule(line: &str) -> Option<(String, String)> {
    if !line.starts_with("s:\"") {
        return None;
    }
    let after_prefix = &line[3..];
    let close_quote = after_prefix.find('"')?;
    let pattern = after_prefix[..close_quote].to_string();
    let after_quote = &after_prefix[close_quote + 1..];
    let replacement = if after_quote.starts_with('=') {
        after_quote[1..].to_string()
    } else {
        String::new()
    };
    Some((pattern, replacement))
}

pub(crate) fn load_split_rules(root_dir: &Path) -> (Vec<SplitRule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let root_entries = sorted_dir_entries(root_dir);

    for path in root_entries.iter().filter(|path| path.is_dir()) {
        for sub_path in sorted_dir_entries(path) {
            if is_tenuki_split_file(&sub_path) {
                read_split_file(&sub_path, &mut rules, &mut warnings);
            }
        }
    }

    for path in root_entries.iter().filter(|path| is_tenuki_split_file(path)) {
        read_split_file(path, &mut rules, &mut warnings);
    }

    rules.sort_by(|a, b| b.source_pattern.len().cmp(&a.source_pattern.len()));
    (rules, warnings)
}

pub(crate) fn read_split_file(path: &Path, rules: &mut Vec<SplitRule>, warnings: &mut Vec<String>) {
    if let Ok(file) = File::open(path) {
        let reader = BufReader::new(file);
        for (line_index, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(line) => line,
                Err(_) => continue,
            };
            let line = if line_index == 0 {
                line.strip_prefix('\u{feff}').unwrap_or(&line)
            } else {
                &line
            };
            if line.trim().is_empty() {
                continue;
            }
            if let Some((pattern, replacement)) = try_parse_split_rule(line) {
                if replacement.contains('\u{ff04}') {
                    let msg = format!(
                        "× [SPLIT] {}:{} fullwidth $ in replacement",
                        path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                        line_index + 1,
                    );
                    warnings.push(msg.clone());
                    log::warn!("{} — {}", msg, line);
                }
                match Regex::new(&pattern) {
                    Ok(re) => rules.push(SplitRule {
                        pattern: re,
                        replacement,
                        source_pattern: pattern,
                    }),
                    Err(e) => {
                        let msg = format!(
                            "× [SPLIT] {}:{} regex error — {}",
                            path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                            line_index + 1,
                            pattern,
                        );
                        warnings.push(msg.clone());
                        log::warn!("{} ({})", msg, e);
                    }
                }
            } else if line.starts_with('s') {
                let msg = format!(
                    "× [SPLIT] {}:{} parse error — {}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    line_index + 1,
                    line
                );
                warnings.push(msg.clone());
                log::warn!("{}", msg);
            }
        }
    }
}
