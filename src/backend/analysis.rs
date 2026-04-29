use std::sync::{Arc, Mutex};

use crate::messages::InputAnalysisSnapshot;

#[derive(Debug, Clone, Default)]
pub struct InputReplayState {
    pub latest_snapshot: Option<InputAnalysisSnapshot>,
}

pub type SharedInputReplayState = Arc<Mutex<InputReplayState>>;

pub struct CompletedTranslationRecord {
    pub authority_payload: CompletedAnalysisPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedAnalysisPayload {
    pub raw_text: String,
    pub extracted_text: String,
    pub visible_text: String,
    pub model_inputs: Vec<String>,
    pub final_output: String,
    pub dict_hits: usize,
    pub model_calls: usize,
}

pub fn normalize_input(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn build_snapshot_from_payload(payload: CompletedAnalysisPayload) -> InputAnalysisSnapshot {
    InputAnalysisSnapshot {
        raw_text: normalize_input(&payload.raw_text),
        extracted_text: normalize_input(&payload.extracted_text),
        visible_text: normalize_input(&payload.visible_text),
        model_inputs: payload.model_inputs,
        final_output: Some(payload.final_output),
        result_stale: false,
        dict_hits: payload.dict_hits,
        model_calls: payload.model_calls,
    }
}

pub fn record_completed_translation(
    replay_state: &SharedInputReplayState,
    record: CompletedTranslationRecord,
) -> InputAnalysisSnapshot {
    let snapshot = build_snapshot_from_payload(record.authority_payload);

    if let Ok(mut state) = replay_state.lock() {
        state.latest_snapshot = Some(snapshot.clone());
    }

    snapshot
}

pub fn rebuild_latest_snapshot(
    replay_state: &SharedInputReplayState,
    mark_result_stale: bool,
) -> Option<InputAnalysisSnapshot> {
    let mut state = replay_state.lock().ok()?;
    let mut snapshot = state.latest_snapshot.clone()?;
    if mark_result_stale && snapshot.final_output.is_some() {
        snapshot.result_stale = true;
        state.latest_snapshot = Some(snapshot.clone());
    }

    Some(snapshot)
}

#[cfg(test)]
mod tests {
    use super::{
        rebuild_latest_snapshot, record_completed_translation, CompletedAnalysisPayload,
        CompletedTranslationRecord, InputReplayState,
    };
    use std::sync::{Arc, Mutex};

    #[test]
    fn completed_translation_builds_snapshot_from_authority_payload() {
        let replay = Arc::new(Mutex::new(InputReplayState::default()));

        let snapshot = record_completed_translation(
            &replay,
            CompletedTranslationRecord {
                authority_payload: CompletedAnalysisPayload {
                    raw_text: "authority raw".to_string(),
                    extracted_text: "authority extracted".to_string(),
                    visible_text: "authority visible".to_string(),
                    model_inputs: vec!["authority model input".to_string()],
                    final_output: "authority output".to_string(),
                    dict_hits: 3,
                    model_calls: 4,
                },
            },
        );

        assert_eq!(snapshot.raw_text, "authority raw");
        assert_eq!(snapshot.extracted_text, "authority extracted");
        assert_eq!(snapshot.visible_text, "authority visible");
        assert_eq!(
            snapshot.model_inputs,
            vec!["authority model input".to_string()]
        );
        assert_eq!(snapshot.final_output.as_deref(), Some("authority output"));
        assert_eq!(snapshot.dict_hits, 3);
        assert_eq!(snapshot.model_calls, 4);

        let state = replay.lock().unwrap();
        let saved = state.latest_snapshot.as_ref().unwrap();
        assert_eq!(saved.raw_text, "authority raw");
        assert_eq!(saved.extracted_text, "authority extracted");
        assert_eq!(saved.visible_text, "authority visible");
        assert_eq!(
            saved.model_inputs,
            vec!["authority model input".to_string()]
        );
        assert_eq!(saved.final_output.as_deref(), Some("authority output"));
        assert_eq!(saved.dict_hits, 3);
        assert_eq!(saved.model_calls, 4);
    }

    #[test]
    fn rebuild_latest_snapshot_replays_saved_snapshot_as_stale() {
        let replay = Arc::new(Mutex::new(InputReplayState::default()));
        let snapshot = record_completed_translation(
            &replay,
            CompletedTranslationRecord {
                authority_payload: CompletedAnalysisPayload {
                    raw_text: "raw=source".to_string(),
                    extracted_text: "payload extracted".to_string(),
                    visible_text: "payload visible".to_string(),
                    model_inputs: vec!["payload model input".to_string()],
                    final_output: "payload output".to_string(),
                    dict_hits: 5,
                    model_calls: 6,
                },
            },
        );
        assert!(!snapshot.result_stale);

        let replayed = rebuild_latest_snapshot(&replay, true).unwrap();
        assert_eq!(replayed.raw_text, "raw=source");
        assert_eq!(replayed.extracted_text, "payload extracted");
        assert_eq!(replayed.visible_text, "payload visible");
        assert_eq!(
            replayed.model_inputs,
            vec!["payload model input".to_string()]
        );
        assert_eq!(replayed.final_output.as_deref(), Some("payload output"));
        assert!(replayed.result_stale);
        assert_eq!(replayed.dict_hits, 5);
        assert_eq!(replayed.model_calls, 6);

        let saved = replay.lock().unwrap().latest_snapshot.clone().unwrap();
        assert!(saved.result_stale);
    }
}
