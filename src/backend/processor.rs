use once_cell::sync::Lazy;
use regex::Regex;

use crate::config::StructuralOptions;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TranslationMode {
    #[default]
    Structural,
    Passthrough,
}

impl TranslationMode {
    pub fn from_str(value: &str) -> Self {
        match value {
            "passthrough" => TranslationMode::Passthrough,
            _ => TranslationMode::Structural,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProcessorData {
    Passthrough,
    Structural {
        tokens: Vec<String>,
        flags: Vec<bool>,
        text_tokens: Vec<String>,
        visible_text: String,
    },
}

#[derive(Debug, Clone)]
pub struct TranslationContext {
    pub parts_to_translate: Vec<String>,
    pub processor_data: ProcessorData,
}

impl TranslationContext {
    pub fn new_passthrough(text: String) -> Self {
        Self {
            parts_to_translate: if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text]
            },
            processor_data: ProcessorData::Passthrough,
        }
    }

    pub fn new_structural(
        visible_text: String,
        tokens: Vec<String>,
        flags: Vec<bool>,
        text_tokens: Vec<String>,
    ) -> Self {
        Self {
            parts_to_translate: if visible_text.trim().is_empty() {
                Vec::new()
            } else {
                vec![visible_text.clone()]
            },
            processor_data: ProcessorData::Structural {
                tokens,
                flags,
                text_tokens,
                visible_text,
            },
        }
    }

    pub fn structural_text_tokens(&self) -> Option<&[String]> {
        match &self.processor_data {
            ProcessorData::Structural { text_tokens, .. } => Some(text_tokens.as_slice()),
            ProcessorData::Passthrough => None,
        }
    }

    pub fn preview_text(&self, source: &str) -> String {
        match &self.processor_data {
            ProcessorData::Passthrough => source.to_string(),
            ProcessorData::Structural { visible_text, .. } => visible_text.clone(),
        }
    }
}

pub trait TextProcessor: Send + Sync {
    fn preprocess(&self, text: &str) -> TranslationContext;
    fn postprocess(&self, translated_parts: &[String], ctx: &TranslationContext) -> String;
}

pub struct ProcessorFactory;

impl ProcessorFactory {
    pub fn create(mode: TranslationMode, options: StructuralOptions) -> Box<dyn TextProcessor> {
        match mode {
            TranslationMode::Passthrough => Box::new(PassthroughProcessor::new()),
            TranslationMode::Structural => Box::new(StructuralProcessor::with_options(options)),
        }
    }
}

pub struct PassthroughProcessor;

impl PassthroughProcessor {
    pub fn new() -> Self {
        Self
    }
}

impl TextProcessor for PassthroughProcessor {
    fn preprocess(&self, text: &str) -> TranslationContext {
        TranslationContext::new_passthrough(text.to_string())
    }

    fn postprocess(&self, translated_parts: &[String], _ctx: &TranslationContext) -> String {
        translated_parts.join("")
    }
}

impl Default for PassthroughProcessor {
    fn default() -> Self {
        Self::new()
    }
}

static PROTECTED_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?P<escaped>\\r\\n|\\n|\\t)|(?P<tag><[^>\r\n]+>)|(?P<bracket>\[[^\]\r\n]+\])|(?P<placeholder>\{\{[^\}\r\n]+\}\})|(?P<brace>\{[^\}\r\n]+\})|(?P<marker>Z[A-Z]+Z)|(?P<boundary>[+\-:/%()|\[\];（）【】《》：；])",
    )
    .unwrap()
});

#[allow(dead_code)]
static NUM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[0-9０-９]+(?:\.[0-9０-９]+)*").unwrap()
});

pub struct StructuralProcessor {
    options: StructuralOptions,
}

impl StructuralProcessor {
    pub fn new() -> Self {
        Self::with_options(StructuralOptions::default())
    }

    pub fn with_options(options: StructuralOptions) -> Self {
        Self { options }
    }

    fn is_ascii_alpha_char(ch: char) -> bool {
        ch.is_ascii_alphabetic()
    }

    fn alignment_char(ch: char) -> char {
        if ch.is_ascii() {
            ch.to_ascii_lowercase()
        } else {
            ch
        }
    }

    fn is_segment_boundary(token: &str) -> bool {
        matches!(
            token,
            "+" | "-" | ":" | "/" | "%" | "(" | ")" | "|" | "[" | "]" | ";"
                | "（" | "）" | "【" | "】" | "《" | "》" | "：" | "；"
        )
    }

    fn should_protect_boundary_token(text: &str, start: usize, end: usize, token: &str) -> bool {
        if token != "-" {
            return true;
        }

        let left_is_ascii_alpha = text[..start]
            .chars()
            .next_back()
            .map(Self::is_ascii_alpha_char)
            .unwrap_or(false);
        let right_is_ascii_alpha = text[end..]
            .chars()
            .next()
            .map(Self::is_ascii_alpha_char)
            .unwrap_or(false);

        !(left_is_ascii_alpha && right_is_ascii_alpha)
    }

    fn split_text_and_structure(&self, text: &str) -> (Vec<String>, Vec<bool>) {
        let mut tokens = Vec::new();
        let mut flags = Vec::new();
        let mut last = 0usize;

        for captures in PROTECTED_RE.captures_iter(text) {
            let m = captures.get(0).expect("match missing");

            if captures.name("escaped").is_some() && !self.options.protect_escaped_sequences {
                continue;
            }
            if captures.name("tag").is_some() && !self.options.protect_tags {
                continue;
            }
            if captures.name("bracket").is_some() && !self.options.protect_brackets {
                continue;
            }
            if (captures.name("placeholder").is_some()
                || captures.name("brace").is_some()
                || captures.name("marker").is_some())
                && !self.options.protect_placeholders
            {
                continue;
            }
            if captures.name("boundary").is_some()
                && (!self.options.split_symbolic_segments
                    || !Self::should_protect_boundary_token(text, m.start(), m.end(), m.as_str()))
            {
                continue;
            }

            if m.start() > last {
                tokens.push(text[last..m.start()].to_string());
                flags.push(false);
            }
            tokens.push(m.as_str().to_string());
            flags.push(true);
            last = m.end();
        }

        if last < text.len() {
            tokens.push(text[last..].to_string());
            flags.push(false);
        }

        (tokens, flags)
    }

    fn collect_visible_text(&self, tokens: &[String], flags: &[bool]) -> String {
        tokens
            .iter()
            .zip(flags.iter())
            .filter_map(|(token, flag)| if !*flag { Some(token.as_str()) } else { None })
            .collect()
    }

    fn collect_text_tokens(&self, tokens: &[String], flags: &[bool]) -> Vec<String> {
        let mut text_tokens = Vec::new();
        let mut current = String::new();

        for (token, flag) in tokens.iter().zip(flags.iter()) {
            if !*flag {
                if !token.trim().is_empty() {
                    current.push_str(token);
                }
                continue;
            }

            if Self::is_segment_boundary(token) && !current.trim().is_empty() {
                text_tokens.push(std::mem::take(&mut current));
            }
        }

        if !current.trim().is_empty() {
            text_tokens.push(current);
        }

        text_tokens
    }

    fn alignment_anchors(source_text: &str, translated_text: &str) -> Vec<(usize, usize)> {
        let source_chars: Vec<char> = source_text.chars().collect();
        let translated_chars: Vec<char> = translated_text.chars().collect();
        let source_len = source_chars.len();
        let translated_len = translated_chars.len();

        if source_len == 0 || translated_len == 0 {
            return vec![(0, 0), (source_len, translated_len)];
        }

        let mut dp = vec![vec![0usize; translated_len + 1]; source_len + 1];

        for source_index in (0..source_len).rev() {
            for translated_index in (0..translated_len).rev() {
                if Self::alignment_char(source_chars[source_index])
                    == Self::alignment_char(translated_chars[translated_index])
                {
                    dp[source_index][translated_index] =
                        dp[source_index + 1][translated_index + 1] + 1;
                } else {
                    dp[source_index][translated_index] = dp[source_index + 1][translated_index]
                        .max(dp[source_index][translated_index + 1]);
                }
            }
        }

        let mut anchors = vec![(0usize, 0usize)];
        let mut source_index = 0usize;
        let mut translated_index = 0usize;

        while source_index < source_len && translated_index < translated_len {
            if Self::alignment_char(source_chars[source_index])
                == Self::alignment_char(translated_chars[translated_index])
            {
                anchors.push((source_index + 1, translated_index + 1));
                source_index += 1;
                translated_index += 1;
            } else if dp[source_index + 1][translated_index]
                >= dp[source_index][translated_index + 1]
            {
                source_index += 1;
            } else {
                translated_index += 1;
            }
        }

        if anchors.last().copied() != Some((source_len, translated_len)) {
            anchors.push((source_len, translated_len));
        }

        anchors
    }

    fn translated_boundary(source_offset: usize, anchors: &[(usize, usize)]) -> usize {
        if anchors.is_empty() {
            return 0;
        }

        for window in anchors.windows(2) {
            let (source_start, translated_start) = window[0];
            let (source_end, translated_end) = window[1];

            if source_offset < source_start || source_offset > source_end {
                continue;
            }

            if source_end == source_start {
                return translated_start;
            }

            let source_delta = source_offset - source_start;
            let source_span = source_end - source_start;
            let translated_span = translated_end.saturating_sub(translated_start);

            return translated_start
                + ((source_delta * translated_span + source_span / 2) / source_span);
        }

        anchors.last().map(|(_, translated)| *translated).unwrap_or(0)
    }

    fn reassemble_single_translation(
        &self,
        tokens: &[String],
        flags: &[bool],
        visible_text: &str,
        translated_text: &str,
    ) -> String {
        let anchors = Self::alignment_anchors(visible_text, translated_text);
        let translated_chars: Vec<char> = translated_text.chars().collect();
        let translated_len = translated_chars.len();
        let visible_len = visible_text.chars().count();
        let mut insertions = vec![Vec::<&str>::new(); translated_len + 1];
        let mut source_offset = 0usize;

        for (token, flag) in tokens.iter().zip(flags.iter()) {
            if !*flag {
                source_offset += token.chars().count();
                continue;
            }

            let boundary = if source_offset >= visible_len {
                translated_len
            } else {
                Self::translated_boundary(source_offset, &anchors).min(translated_len)
            };
            insertions[boundary].push(token.as_str());
        }

        let mut result = String::new();
        for token in &insertions[0] {
            result.push_str(token);
        }
        for (index, ch) in translated_chars.iter().enumerate() {
            result.push(*ch);
            for token in &insertions[index + 1] {
                result.push_str(token);
            }
        }

        result
    }

    fn reassemble_multiple_translations(
        &self,
        tokens: &[String],
        flags: &[bool],
        translated_parts: &[String],
    ) -> String {
        let mut result = String::new();
        let mut translated_index = 0usize;

        for (token, flag) in tokens.iter().zip(flags.iter()) {
            if !*flag {
                if token.trim().is_empty() {
                    result.push_str(token);
                    continue;
                }

                let translated = translated_parts
                    .get(translated_index)
                    .map(String::as_str)
                    .unwrap_or(token.as_str());
                result.push_str(translated);
                translated_index += 1;
            } else {
                result.push_str(token);
            }
        }

        result
    }

}

impl TextProcessor for StructuralProcessor {
    fn preprocess(&self, text: &str) -> TranslationContext {
        let (tokens, flags) = self.split_text_and_structure(text);
        let visible_text = self.collect_visible_text(&tokens, &flags);
        let text_tokens = self.collect_text_tokens(&tokens, &flags);

        TranslationContext::new_structural(visible_text, tokens, flags, text_tokens)
    }

    fn postprocess(&self, translated_parts: &[String], ctx: &TranslationContext) -> String {
        match &ctx.processor_data {
            ProcessorData::Structural {
                tokens,
                flags,
                text_tokens,
                visible_text,
            } => {
                if translated_parts.len() == text_tokens.len() && translated_parts.len() > 1 {
                    return self.reassemble_multiple_translations(tokens, flags, translated_parts);
                }

                let translated = translated_parts
                    .first()
                    .cloned()
                    .unwrap_or_else(|| visible_text.clone());

                if flags.iter().all(|flag| !*flag) {
                    return translated;
                }

                self.reassemble_single_translation(tokens, flags, visible_text, &translated)
            }
            ProcessorData::Passthrough => translated_parts.join(""),
        }
    }

}

impl Default for StructuralProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{StructuralProcessor, TextProcessor};

    #[test]
    fn numbers_stay_in_visible_text() {
        let processor = StructuralProcessor::new();
        let ctx = processor.preprocess("Attack+15%");
        assert_eq!(ctx.parts_to_translate, vec!["Attack15".to_string()]);
    }

    #[test]
    fn reassembles_boundary_symbols() {
        let processor = StructuralProcessor::new();
        let ctx = processor.preprocess("<b>Attack+150%</b>");
        let rebuilt = processor.postprocess(&["Damage150".to_string()], &ctx);
        assert_eq!(rebuilt, "<b>Damage+150%</b>");
    }

    #[test]
    fn keeps_hyphenated_ascii_words_whole() {
        let processor = StructuralProcessor::new();
        let ctx = processor.preprocess("Half-Elf");
        assert_eq!(ctx.parts_to_translate, vec!["Half-Elf".to_string()]);
    }
}
