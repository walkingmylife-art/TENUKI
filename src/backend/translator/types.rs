use std::time::Duration;

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
        source: String,
        original: String,
        translated: String,
        elapsed_secs: f64,
    },
    Error {
        message: String,
    },
    Trace {
        message: String,
    },
}

impl LogEvent {
    pub(crate) fn dict_hit(original: &str, translated: &str, elapsed: Duration) -> Self {
        Self::DictHit {
            original: original.to_string(),
            translated: translated.to_string(),
            elapsed_secs: elapsed.as_secs_f64(),
        }
    }

    pub(crate) fn pre_model_call(original: &str) -> Self {
        Self::PreModelCall {
            original: original.to_string(),
        }
    }

    pub(crate) fn model_result(
        source: &str,
        original: &str,
        translated: &str,
        elapsed: Duration,
    ) -> Self {
        Self::ModelResult {
            source: source.to_string(),
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
    pub(crate) fn dict_hit() -> Self {
        Self {
            dict_hits: 1,
            model_calls: 0,
        }
    }

    pub(crate) fn model_call() -> Self {
        Self {
            dict_hits: 0,
            model_calls: 1,
        }
    }

    pub(crate) fn merge(&mut self, other: &Self) {
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
pub enum PersistEntry {
    Exact {
        key: String,
        value: String,
    },
    Regex {
        pattern: String,
        replacement: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTranslationEntry {
    pub source: String,
    pub translated: String,
    pub persist: PersistEntry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranslationResult {
    pub text: String,
    pub new_entries: Vec<NewTranslationEntry>,
    pub stats: TranslationStats,
    pub logs: Vec<LogEvent>,
}

impl TranslationResult {
    pub(crate) fn empty(text: String) -> Self {
        Self {
            text,
            new_entries: Vec::new(),
            stats: TranslationStats::default(),
            logs: Vec::new(),
        }
    }

    pub(crate) fn from_dict_hit(text: String, original: &str, elapsed: Duration) -> Self {
        let log = LogEvent::dict_hit(original, &text, elapsed);
        Self {
            text,
            new_entries: Vec::new(),
            stats: TranslationStats::dict_hit(),
            logs: vec![log],
        }
    }

    pub(crate) fn from_model_call_success(
        text: String,
        source: &str,
        original: &str,
        elapsed: Duration,
    ) -> Self {
        let model_log = LogEvent::model_result(source, original, &text, elapsed);
        Self {
            text,
            new_entries: Vec::new(),
            stats: TranslationStats::model_call(),
            logs: vec![LogEvent::pre_model_call(original), model_log],
        }
    }

    pub(crate) fn from_model_call_failure(original: &str) -> Self {
        Self {
            text: original.to_string(),
            new_entries: Vec::new(),
            stats: TranslationStats::model_call(),
            logs: vec![
                LogEvent::pre_model_call(original),
                LogEvent::Error {
                    message: format!("LLM call failed for: {}", original),
                },
            ],
        }
    }

    pub(crate) fn absorb(&mut self, other: Self) {
        self.new_entries.extend(other.new_entries);
        self.stats.merge(&other.stats);
        self.logs.extend(other.logs);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FragmentAuthority {
    pub source: String,
}

impl FragmentAuthority {
    pub(super) fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SurfaceKind {
    Visible,
    ProtectedAngle,
    Newline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SurfaceNode {
    pub text: String,
    pub kind: SurfaceKind,
}

impl SurfaceNode {
    pub(super) fn visible(text: String) -> Self {
        Self {
            text,
            kind: SurfaceKind::Visible,
        }
    }

    pub(super) fn protected_angle(text: String) -> Self {
        Self {
            text,
            kind: SurfaceKind::ProtectedAngle,
        }
    }

    pub(super) fn newline(text: String) -> Self {
        Self {
            text,
            kind: SurfaceKind::Newline,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FragmentNode {
    pub authority: FragmentAuthority,
}

impl FragmentNode {
    pub(super) fn new(source: &str) -> Self {
        Self {
            authority: FragmentAuthority::new(source),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PlannedNode {
    Surface(SurfaceNode),
    Fragment(FragmentNode),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlannedSegment {
    pub nodes: Vec<PlannedNode>,
    pub trailing_separator: Option<SurfaceNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlannedLine {
    pub segments: Vec<PlannedSegment>,
    pub trailing_newline: Option<SurfaceNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlannedDocument {
    pub lines: Vec<PlannedLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedFragmentNode {
    pub authority: FragmentAuthority,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ResolvedNode {
    Surface(SurfaceNode),
    Fragment(ResolvedFragmentNode),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedSegment {
    pub nodes: Vec<ResolvedNode>,
    pub trailing_separator: Option<SurfaceNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedLine {
    pub segments: Vec<ResolvedSegment>,
    pub trailing_newline: Option<SurfaceNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedDocument {
    pub lines: Vec<ResolvedLine>,
}
