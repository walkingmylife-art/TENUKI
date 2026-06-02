use super::structure::{
    reconstruct_segment, split_lines, split_protected_angles_and_newlines, tokenize_structure,
    RawPart, StructureToken,
};
use super::types::{
    FragmentNode, PlannedDocument, PlannedLine, PlannedNode, PlannedSegment, SurfaceNode,
};

const EDGE_PREFIX_CLOSING_BRACKETS: &[char] =
    &[')', '\u{FF09}', ']', '\u{FF3D}', '\u{300B}', '\u{3011}'];

const EDGE_SUFFIX_BRACKETS: &[char] = &[
    '(', '\u{FF08}', '[', '\u{FF3B}', '\u{300A}', '\u{3010}', ')', '\u{FF09}', ']', '\u{FF3D}',
    '\u{300B}', '\u{3011}',
];

const EDGE_PREFIX_ADJACENT_PUNCTUATION: &[char] = &['\u{3001}', '\u{3002}', '\u{FF0C}', '\u{FF0E}'];

fn is_horizontal_space(ch: char) -> bool {
    ch == ' ' || ch == '\u{3000}'
}

struct EdgeStructuralParts {
    prefix: String,
    core: String,
    suffix: String,
}

fn split_text_edge_structural_affixes(text: &str) -> EdgeStructuralParts {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    let mut prefix_end = 0usize;
    let mut has_prefix_bracket = false;
    while prefix_end < len {
        let ch = chars[prefix_end];
        if is_edge_prefix_closing_bracket(ch) {
            has_prefix_bracket = true;
            prefix_end += 1;
        } else if has_prefix_bracket && is_edge_prefix_adjacent_punctuation(ch) {
            prefix_end += 1;
        } else if has_prefix_bracket && is_horizontal_space(ch) {
            prefix_end += 1;
        } else {
            break;
        }
    }

    let mut suffix_start = len;
    let mut has_suffix_bracket = false;
    while suffix_start > prefix_end {
        let ch = chars[suffix_start - 1];
        if is_edge_suffix_bracket(ch) {
            has_suffix_bracket = true;
            suffix_start -= 1;
        } else if has_suffix_bracket && is_horizontal_space(ch) {
            suffix_start -= 1;
        } else {
            break;
        }
    }

    let prefix: String = chars[..prefix_end].iter().collect();
    let core: String = chars[prefix_end..suffix_start].iter().collect();
    let suffix: String = chars[suffix_start..].iter().collect();

    EdgeStructuralParts {
        prefix,
        core,
        suffix,
    }
}

fn is_edge_prefix_closing_bracket(ch: char) -> bool {
    EDGE_PREFIX_CLOSING_BRACKETS.contains(&ch)
}

fn is_edge_suffix_bracket(ch: char) -> bool {
    EDGE_SUFFIX_BRACKETS.contains(&ch)
}

fn is_edge_prefix_adjacent_punctuation(ch: char) -> bool {
    EDGE_PREFIX_ADJACENT_PUNCTUATION.contains(&ch)
}

fn is_edge_structural_bracket_char(ch: char) -> bool {
    is_edge_prefix_closing_bracket(ch) || is_edge_suffix_bracket(ch)
}

fn is_render_surface_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ','
            | '!'
            | '?'
            | ';'
            | ':'
            | '\u{3002}'
            | '\u{3001}'
            | '\u{FF01}'
            | '\u{FF1F}'
            | '\u{FF1B}'
            | '\u{FF1A}'
            | '\u{FF0E}'
            | '\u{FF0C}'
            | '\u{2026}'
    )
}

fn is_translation_core_char(ch: char) -> bool {
    !ch.is_whitespace()
        && !is_render_surface_punctuation(ch)
        && !is_edge_structural_bracket_char(ch)
}

fn push_text_token_nodes(nodes: &mut Vec<PlannedNode>, text: &str) {
    let parts = split_text_edge_structural_affixes(text);
    if !parts.prefix.is_empty() {
        nodes.push(PlannedNode::Surface(SurfaceNode::visible(parts.prefix)));
    }
    if !parts.core.trim().is_empty() {
        if parts.core.chars().any(is_translation_core_char) {
            nodes.push(PlannedNode::Fragment(FragmentNode::new(&parts.core)));
        } else {
            nodes.push(PlannedNode::Surface(SurfaceNode::visible(parts.core)));
        }
    }
    if !parts.suffix.is_empty() {
        nodes.push(PlannedNode::Surface(SurfaceNode::visible(parts.suffix)));
    }
}

fn plan_mixed_structure_nodes(tokens: &[StructureToken]) -> Vec<PlannedNode> {
    let mut nodes = Vec::new();
    for token in tokens {
        match token {
            StructureToken::Text(text) => push_text_token_nodes(&mut nodes, text),
            StructureToken::Bracket(slot) => {
                nodes.push(PlannedNode::Surface(SurfaceNode::visible(
                    slot.open.to_string(),
                )));
                nodes.push(PlannedNode::Surface(SurfaceNode::visible(
                    slot.inner_left_spaces.clone(),
                )));
                nodes.extend(plan_inner_text_to_segment_nodes(&slot.inner_core));
                nodes.push(PlannedNode::Surface(SurfaceNode::visible(
                    slot.inner_right_spaces.clone(),
                )));
                nodes.push(PlannedNode::Surface(SurfaceNode::visible(
                    slot.close.to_string(),
                )));
            }
            StructureToken::Delimiter(_) => {}
        }
    }
    nodes
}

fn plan_segment(tokens: &[StructureToken]) -> Vec<PlannedNode> {
    let bracket_count = tokens
        .iter()
        .filter(|token| matches!(token, StructureToken::Bracket(_)))
        .count();

    if bracket_count == 0 {
        let fragment = reconstruct_segment(tokens);
        if fragment.is_empty() {
            return Vec::new();
        }
        let mut nodes = Vec::new();
        push_text_token_nodes(&mut nodes, &fragment);
        return nodes;
    }

    plan_mixed_structure_nodes(tokens)
}

pub(super) fn plan_inner_text_to_segment_nodes(text: &str) -> Vec<PlannedNode> {
    if text.is_empty() {
        return vec![PlannedNode::Surface(SurfaceNode::visible(String::new()))];
    }

    let (lines, newline_tokens) = split_lines(tokenize_structure(text));
    let mut all_nodes = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        for (segment_index, segment) in line.segments.iter().enumerate() {
            all_nodes.extend(plan_segment(segment));
            if let Some(separator) = line.separators.get(segment_index) {
                all_nodes.push(PlannedNode::Surface(SurfaceNode::visible(
                    separator.text.clone(),
                )));
            }
        }

        if let Some(newline) = newline_tokens.get(line_index) {
            all_nodes.push(PlannedNode::Surface(SurfaceNode::visible(
                newline.text.clone(),
            )));
        }
    }

    all_nodes
}

#[derive(Default)]
struct PlannedLineBuilder {
    segments: Vec<PlannedSegment>,
    current_nodes: Vec<PlannedNode>,
    last_was_surface: bool,
}

impl PlannedLineBuilder {
    fn append_text(&mut self, text: &str) {
        let text = if self.last_was_surface {
            self.last_was_surface = false;
            let trimmed = text.trim_start_matches(is_horizontal_space);
            if trimmed.len() < text.len() {
                let spaces = text[..text.len() - trimmed.len()].to_string();
                self.current_nodes
                    .push(PlannedNode::Surface(SurfaceNode::visible(spaces)));
            }
            if trimmed.is_empty() {
                return;
            }
            trimmed
        } else {
            text
        };

        let (lines, newline_tokens) = split_lines(tokenize_structure(text));

        for (line_index, line) in lines.iter().enumerate() {
            for (segment_index, segment) in line.segments.iter().enumerate() {
                self.current_nodes.extend(plan_segment(segment));
                if let Some(separator) = line.separators.get(segment_index) {
                    self.finish_segment(Some(SurfaceNode::visible(separator.text.clone())));
                }
            }

            if let Some(newline) = newline_tokens.get(line_index) {
                self.finish_line(Some(SurfaceNode::newline(newline.text.clone())));
            }
        }
    }

    fn append_surface(&mut self, surface: SurfaceNode) {
        self.absorb_trailing_spaces();
        self.current_nodes.push(PlannedNode::Surface(surface));
        self.last_was_surface = true;
    }

    fn absorb_trailing_spaces(&mut self) {
        let Some(last) = self.current_nodes.last_mut() else {
            return;
        };
        let text: &mut String = match last {
            PlannedNode::Fragment(frag) => &mut frag.authority.source,
            PlannedNode::Surface(surf) => &mut surf.text,
        };
        let trimmed_end = text.trim_end_matches(is_horizontal_space).len();
        if trimmed_end == text.len() {
            return;
        }
        let spaces = text[trimmed_end..].to_string();
        text.truncate(trimmed_end);
        self.current_nodes
            .push(PlannedNode::Surface(SurfaceNode::visible(spaces)));
    }

    fn finish_segment(&mut self, trailing_separator: Option<SurfaceNode>) {
        if self.current_nodes.is_empty() && trailing_separator.is_none() {
            return;
        }

        self.segments.push(PlannedSegment {
            nodes: std::mem::take(&mut self.current_nodes),
            trailing_separator,
        });
    }

    fn finish_line(&mut self, trailing_newline: Option<SurfaceNode>) -> PlannedLine {
        if !self.current_nodes.is_empty() {
            self.finish_segment(None);
        }

        PlannedLine {
            segments: std::mem::take(&mut self.segments),
            trailing_newline,
        }
    }
}

pub(super) fn plan_document(input: &str) -> PlannedDocument {
    let mut lines = Vec::new();
    let mut builder = PlannedLineBuilder::default();

    for part in split_protected_angles_and_newlines(input) {
        match part {
            RawPart::Text(text) => {
                if !text.is_empty() {
                    builder.append_text(&text);
                }
            }
            RawPart::ProtectedAngle(span) => {
                builder.append_surface(SurfaceNode::protected_angle(span));
            }
            RawPart::Newline(newline) => {
                lines.push(builder.finish_line(Some(SurfaceNode::newline(newline))));
            }
        }
    }

    lines.push(builder.finish_line(None));

    PlannedDocument { lines }
}
