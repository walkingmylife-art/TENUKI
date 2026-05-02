use super::types::{ResolvedDocument, ResolvedNode, SurfaceKind, SurfaceNode, TranslationSettings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RenderAtom {
    Text(String),
    ProtectedAngle(String),
    Newline(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WrapPlan {
    emit_end: usize,
    next_start: usize,
}

const WRAP_SPACE_FALLBACK_MIN_CHARS: usize = 100;

fn is_wrap_candidate(chars: &[char], index: usize) -> Option<usize> {
    let ch = chars.get(index).copied()?;
    let next = chars.get(index + 1).copied();

    if ch == '\u{3002}' {
        return Some(1);
    }
    if ch == '\u{3001}' || ch == '\u{FF0C}' {
        return Some(1);
    }
    if (ch == '.' || ch == ',') && next == Some(' ') {
        return Some(2);
    }

    if ch == ' ' && chars.get(index + 1) == Some(&'ใ') && chars.get(index + 2) == Some(&'น') {
        return Some(3);
    }

    None
}

fn wrap_candidate_score(chars: &[char], index: usize, candidate_width: usize) -> i32 {
    match (chars[index], candidate_width) {
        ('\u{3002}', 1) => 20,
        ('\u{3001}', 1) | ('\u{FF0C}', 1) => 8,
        ('.', 2) => 20,
        (',', 2) => 8,
        (' ', 3) => 20,
        _ => 0,
    }
}

fn find_center_ascii_space(chars: &[char]) -> Option<usize> {
    let center = chars.len() / 2;
    let mut best: Option<(usize, usize)> = None;

    for (i, &ch) in chars.iter().enumerate() {
        if ch == ' ' {
            let dist = center.abs_diff(i);
            if best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, i));
            }
        }
    }

    best.map(|(_, i)| i)
}

fn surface_to_render_atom(surface: &SurfaceNode) -> RenderAtom {
    match surface.kind {
        SurfaceKind::Visible => RenderAtom::Text(surface.text.clone()),
        SurfaceKind::ProtectedAngle => RenderAtom::ProtectedAngle(surface.text.clone()),
        SurfaceKind::Newline => RenderAtom::Newline(surface.text.clone()),
    }
}

fn build_document_render_atoms(document: &ResolvedDocument) -> Vec<RenderAtom> {
    let mut atoms = Vec::new();

    for line in &document.lines {
        for segment in &line.segments {
            atoms.extend(build_render_atoms(&segment.nodes));
            if let Some(separator) = &segment.trailing_separator {
                atoms.push(surface_to_render_atom(separator));
            }
        }

        if let Some(newline) = &line.trailing_newline {
            atoms.push(surface_to_render_atom(newline));
        }
    }

    atoms
}

pub(super) fn build_render_atoms(nodes: &[ResolvedNode]) -> Vec<RenderAtom> {
    let mut atoms = Vec::new();

    for node in nodes {
        match node {
            ResolvedNode::Surface(surface) => atoms.push(surface_to_render_atom(surface)),
            ResolvedNode::Fragment(fragment) => {
                if !fragment.text.is_empty() {
                    atoms.push(RenderAtom::Text(fragment.text.clone()));
                }
            }
        }
    }

    atoms
}

fn split_display_segments(atoms: Vec<RenderAtom>) -> Vec<Vec<RenderAtom>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();

    for atom in atoms {
        match atom {
            RenderAtom::Newline(nl) => {
                if !current.is_empty() {
                    segments.push(current);
                    current = Vec::new();
                }
                segments.push(vec![RenderAtom::Newline(nl)]);
            }
            other => current.push(other),
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    segments
}

fn visible_text_for_wrap(atoms: &[RenderAtom]) -> String {
    atoms
        .iter()
        .filter_map(|atom| match atom {
            RenderAtom::Text(text) => Some(text.as_str()),
            RenderAtom::ProtectedAngle(_) | RenderAtom::Newline(_) => None,
        })
        .collect()
}

fn insert_wrap_plan_into_atoms(atoms: Vec<RenderAtom>, plan: Option<WrapPlan>) -> Vec<RenderAtom> {
    let Some(plan) = plan else {
        return atoms;
    };

    let mut out = Vec::new();
    let mut visible_index = 0usize;
    let mut inserted_break = false;

    for atom in atoms {
        match atom {
            RenderAtom::Text(text) => {
                let mut current_buf = String::new();

                for ch in text.chars() {
                    if !inserted_break && visible_index >= plan.emit_end {
                        if !current_buf.is_empty() {
                            out.push(RenderAtom::Text(std::mem::take(&mut current_buf)));
                        }

                        out.push(RenderAtom::Text("\n".to_string()));
                        inserted_break = true;
                    }

                    let keep = visible_index < plan.emit_end || visible_index >= plan.next_start;
                    if keep {
                        current_buf.push(ch);
                    }

                    visible_index += 1;
                }

                if !current_buf.is_empty() {
                    out.push(RenderAtom::Text(current_buf));
                }
            }
            RenderAtom::ProtectedAngle(text) => out.push(RenderAtom::ProtectedAngle(text)),
            RenderAtom::Newline(text) => out.push(RenderAtom::Newline(text)),
        }
    }

    if !inserted_break && visible_index >= plan.emit_end {
        out.push(RenderAtom::Text("\n".to_string()));
    }

    out
}

fn find_wrap_plan(
    visible_text: &str,
    enabled: bool,
    min_length: usize,
    min_tail_length: usize,
) -> Option<WrapPlan> {
    if !enabled || visible_text.contains('\n') || visible_text.chars().count() < min_length {
        return None;
    }

    let chars: Vec<char> = visible_text.chars().collect();
    let len = chars.len();
    let center = len / 2;

    let mut best: Option<(i32, usize, usize, usize)> = None;

    for index in 0..len {
        let Some(candidate_width) = is_wrap_candidate(&chars, index) else {
            continue;
        };

        let next_start = match candidate_width {
            1 => index + 1,
            2 => index + 2,
            3 => index + 1,
            _ => continue,
        };

        if candidate_width != 3 && len.saturating_sub(next_start) < min_tail_length {
            continue;
        }

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

    if let Some((_, _, index, candidate_width)) = best {
        let (emit_end, next_start) = match candidate_width {
            1 => (index + 1, index + 1),
            2 => (index + 1, index + 2),
            3 => (index, index + 1),
            _ => return None,
        };

        return Some(WrapPlan {
            emit_end,
            next_start,
        });
    }

    if len >= WRAP_SPACE_FALLBACK_MIN_CHARS {
        if let Some(index) = find_center_ascii_space(&chars) {
            if len.saturating_sub(index + 1) < min_tail_length {
                return None;
            }

            return Some(WrapPlan {
                emit_end: index,
                next_start: index + 1,
            });
        }
    }

    None
}

pub(super) fn wrap_render_atoms(
    atoms: Vec<RenderAtom>,
    settings: TranslationSettings,
) -> Vec<RenderAtom> {
    if !settings.enable_model_wrap {
        return atoms;
    }

    let segments = split_display_segments(atoms);
    let mut result = Vec::new();

    for segment in segments {
        if matches!(segment.first(), Some(RenderAtom::Newline(_))) {
            result.extend(segment);
            continue;
        }

        let visible = visible_text_for_wrap(&segment);
        let plan = find_wrap_plan(
            &visible,
            settings.enable_model_wrap,
            settings.model_wrap_min_chars,
            settings.model_wrap_min_tail_chars,
        );

        result.extend(insert_wrap_plan_into_atoms(segment, plan));
    }

    result
}

pub(super) fn render_atoms(atoms: &[RenderAtom]) -> String {
    let mut out = String::new();
    for atom in atoms {
        match atom {
            RenderAtom::Text(text)
            | RenderAtom::ProtectedAngle(text)
            | RenderAtom::Newline(text) => out.push_str(text),
        }
    }
    out
}

pub(super) fn render_document(
    document: &ResolvedDocument,
    settings: TranslationSettings,
) -> String {
    let atoms = build_document_render_atoms(document);
    let atoms = wrap_render_atoms(atoms, settings);
    render_atoms(&atoms)
}

#[cfg(test)]
mod tests {
    use super::{is_wrap_candidate, wrap_candidate_score};

    #[test]
    fn fullwidth_comma_is_weak_wrap_candidate() {
        let chars: Vec<char> = "很长的简体字文，后续文".chars().collect();
        let index = chars
            .iter()
            .position(|&ch| ch == '\u{FF0C}')
            .expect("test text should contain fullwidth comma");

        assert_eq!(is_wrap_candidate(&chars, index), Some(1));
        assert_eq!(wrap_candidate_score(&chars, index, 1), 8);
    }
}
