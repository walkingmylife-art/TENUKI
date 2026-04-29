//! File Translate current shape for Phase 2.3.
//!
//! Active path:
//! - multi-format asset intake by content sniffing
//! - source tree / kind-aware preview inside the existing UI
//! - shared table-capable source flow for DelimitedText and normalized JSON tables
//! - column mode = Translate / Original / None
//! - preserved header unless absent mode, with explicit confirmation for DelimitedText
//! - execute uses one shared readiness gate
//! - committed output directory use / List output directory creation is surfaced in UI and in the live List log
//!
//! List boundary:
//! - List mode is not normal translation and has different side-effect rules.
//! - /list does not update dictionary, cache, or input analysis.
//! - output is `{source_stem}.txt` in dict.txt format, written incrementally via `.partial.txt`
//!
//! Parked path:
//! - dict strategy / confirm flow
//! - non-table execute rules

pub mod asset_intake;
pub mod commands;
pub mod preview;
pub mod runner;
pub mod state;
pub mod types;
