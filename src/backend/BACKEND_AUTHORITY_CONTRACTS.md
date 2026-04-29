# Backend Authority Contracts

This file records the Phase 6/7 state of backend processor removal, input analysis, and public authority boundaries.

## Processor Decision

- `src/backend/processor.rs` has been removed.
- `backend.rs` no longer exposes `pub mod processor`.
- `ProcessorFactory`, `TextProcessor`, `TranslationContext`, `NormalTextProcessor`, and `GameTextProcessor` are not part of the backend live path.
- No processor abstraction remains for input analysis.

## Mode Boundary

- `Config.mode` remains a public runtime/config contract.
- `normalize_mode_value()` still normalizes legacy `structural` to `game` and `passthrough` to `normal`.
- Removing `processor.rs` does not remove the `game` / `normal` mode concept.
- Mode handling belongs to config/runtime policy and caller behavior, not to a shared backend processor module.

## Authority Boundary

- `FrontendCommand::SetLanguagePair.dict_slot` is already resolved by the UI/preflight path.
- The backend adopts `dict_slot` into `config.toml`, reloads, and restarts as needed.
- The backend must not infer a different slot, repair missing slot authority, or treat discovery as authority.
- Backend startup reads committed `config.toml` / `launcher_config.toml` authority; it does not create a new authority path for input analysis.

## Input Analysis Snapshot Contract

- Fresh snapshot: built from `CompletedAnalysisPayload` recorded at successful normal `/translate` completion.
- Stale snapshot: cloned from `InputReplayState.latest_snapshot` and marked with `result_stale = true`.
- Stale replay is not recomputed from current mode, game-text options, language settings, or a processor.
- `raw_text`: normalized original request text from the completed translation payload.
- `extracted_text`: analysis source selected by the completed translation payload.
- `visible_text`: human-readable source view recorded by the completed translation payload.
- `model_inputs`: model call inputs observed during the completed translation.
- `final_output`: final translated output for fresh snapshots; retained during stale replay.
- `result_stale`: `false` for fresh snapshots, `true` for replay after mode/language/game-text changes.
- `dict_hits`: dictionary hit count from the completed translation.
- `model_calls`: model call count from the completed translation.

## Module Shape

### `backend/analysis.rs`

- Builds fresh `InputAnalysisSnapshot` from `CompletedAnalysisPayload`.
- Stores the latest completed snapshot in `InputReplayState`.
- Replays the saved snapshot for stale display by toggling `result_stale`.
- Does not depend on processor code or `InputAnalysisProjector`.

### `backend/manager.rs`

- Does not hold an input-analysis projector or processor.
- `mode`, `game_text`, and language changes emit a stale replay of the last authority snapshot.
- Dictionary reload is real work; input analysis replay is only saved snapshot replay.

### `backend/server.rs`

- `AppState` does not hold an input-analysis projector or processor.
- `/translate` builds an authority payload from translation diagnostics and records it.
- `/list` uses `PipelineBehavior::list_mode()` and does not update dictionary/cache/input analysis.

### `main.rs`

- Reads `InputAnalysisSnapshot` for pickup/work-result display.
- Does not reconstruct input analysis.

## List Mode Boundary

- `/list` is a separate backend entry from normal `/translate`.
- `/list` does not commit dictionary authority, update translation caches, emit statistics, or update input analysis.
- File Translate/List output directories are execution locations for that run, not committed authority changes.
- Continuation/resume state for List mode is independent from normal input analysis snapshot replay.
