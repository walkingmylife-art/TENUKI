// src/backend/manager.rs

//! バックエンド管理モジュール

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::RwLock;

use crate::backend::analysis::{self, SharedInputReplayState};
use crate::backend::dictionary::Dictionary;
use crate::backend::engine::{self, EngineManager, EngineWaitKind};
use crate::backend::server;
use crate::backend::translator::TranslationSettings;
use crate::backend::translator::{HttpLlmClient, LlmClient, NewEntriesCache, TranslationCache};
use crate::backend_info;
use crate::config::{Config, GameTextOptions};
use crate::launcher::app_config::ServerConfig;
use crate::messages::{BackendEvent, LogLevel, LogSource, ProcessType};

pub use crate::backend::slot::*;

#[cfg(test)]
mod tests {
    use super::{
        create_new_slot, dict_slot_matches_target, find_or_create_slot_under, is_slot_dir,
        is_slot_dir_name_for_lang, resolve_lang_pair_dict_slot, resolve_slot_dir, EngineWaitKind,
    };
    use crate::config::Config;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn normal_startup_wait_policy_is_shorter_than_setup_verify_window() {
        let policy = EngineWaitKind::NormalStartup.policy();
        assert_eq!(policy.max_attempts, 24);
        assert_eq!(policy.log_interval_attempts, 2);
        assert_eq!(policy.initial_backoff, Duration::from_millis(500));
        assert_eq!(policy.max_backoff, Duration::from_secs(3));
    }

    #[test]
    fn model_switch_wait_policy_is_shorter_than_normal_startup() {
        let normal = EngineWaitKind::NormalStartup.policy();
        let model_switch = EngineWaitKind::ModelSwitch.policy();
        assert!(model_switch.max_attempts < normal.max_attempts);
        assert_eq!(model_switch.max_attempts, 12);
        assert_eq!(model_switch.log_interval_attempts, 1);
        assert_eq!(model_switch.initial_backoff, Duration::from_millis(500));
        assert_eq!(model_switch.max_backoff, Duration::from_secs(3));
    }

    #[test]
    fn detects_real_slot_only_under_text_dir() {
        assert!(is_slot_dir(Path::new(r"C:\dicts\ja\text\ja_001")));
        assert!(!is_slot_dir(Path::new(r"C:\dicts\ja\text\ja_001\s_0001")));
    }

    #[test]
    fn recognizes_current_and_legacy_slot_names() {
        assert!(is_slot_dir_name_for_lang("ja_001", "ja"));
        assert!(is_slot_dir_name_for_lang("S_0001", "ja"));
        assert!(!is_slot_dir_name_for_lang("foo_001", "ja"));
    }

    #[test]
    fn slot_match_accepts_current_target_root() {
        assert!(dict_slot_matches_target(
            Path::new(r"C:\dicts\ja\text\ja_001"),
            "ja"
        ));
    }

    #[test]
    fn slot_match_accepts_legacy_slot_under_current_target() {
        assert!(dict_slot_matches_target(
            Path::new(r"C:\dicts\ja\text\S_0001"),
            "ja"
        ));
    }

    #[test]
    fn slot_match_rejects_other_target_slot() {
        assert!(!dict_slot_matches_target(
            Path::new(r"C:\dicts\ja\text\ja_001"),
            "en"
        ));
    }

    #[test]
    fn slot_match_rejects_non_authority_shape() {
        assert!(!dict_slot_matches_target(
            Path::new(r"C:\custom\dict\ja_001"),
            "ja"
        ));
    }

    #[test]
    fn reuses_language_slot_instead_of_creating_under_legacy_subdir() {
        let slot = find_or_create_slot_under(Path::new(r"C:\dicts\ja\text\ja_001\s_0001"), "ja");
        assert_eq!(slot, PathBuf::from(r"C:\dicts\ja\text\ja_001"));
    }

    #[test]
    fn picks_next_number_after_legacy_slots() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));
        let text_dir = base_dir.join("dicts").join("ja").join("text");
        std::fs::create_dir_all(text_dir.join("S_0001")).unwrap();

        let slot = find_or_create_slot_under(&text_dir, "ja");
        assert_eq!(slot, text_dir.join("ja_001"));

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn preserves_explicit_selected_folder_as_dict_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));
        let explicit_dir = base_dir.join("custom_dict_dir");

        let mut config = Config::new();
        config.tgt_lang = "ja".to_string();
        config.dict_slot = Some(explicit_dir.to_string_lossy().to_string());

        let resolved = resolve_slot_dir(&config, &base_dir);

        assert_eq!(resolved, explicit_dir);

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn create_new_slot_picks_next_number_for_language() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));
        let text_dir = base_dir.join("dicts").join("en").join("text");
        std::fs::create_dir_all(text_dir.join("en_001")).unwrap();

        let slot = create_new_slot("en", &base_dir);

        assert_eq!(slot, text_dir.join("en_002"));
        assert!(slot.is_dir());

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn resolve_slot_dir_creates_slot_under_current_target_language() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));

        let mut config = Config::new();
        config.tgt_lang = "ar".to_string();
        config.dict_slot = None;

        let resolved = resolve_slot_dir(&config, &base_dir);

        assert_eq!(
            resolved,
            base_dir
                .join("dicts")
                .join("ar")
                .join("text")
                .join("ar_001")
        );
        assert!(resolved.is_dir());

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn resolve_reuses_committed_slot_when_target_matches() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));

        let text_dir = base_dir.join("dicts").join("ja").join("text");
        std::fs::create_dir_all(text_dir.join("ja_001")).unwrap();

        let resolved = resolve_lang_pair_dict_slot(
            Some(text_dir.join("ja_001").to_string_lossy().as_ref()),
            "ja",
            &base_dir,
        );

        assert_eq!(
            resolved,
            text_dir.join("ja_001").to_string_lossy().to_string()
        );

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn resolve_creates_new_slot_when_target_differs_from_committed() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));

        let resolved =
            resolve_lang_pair_dict_slot(Some(r"C:\dicts\ja\text\ja_001"), "en", &base_dir);

        let expected = base_dir
            .join("dicts")
            .join("en")
            .join("text")
            .join("en_001");
        assert_eq!(resolved, expected.to_string_lossy().to_string());
        assert!(expected.is_dir());

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn resolve_creates_new_slot_when_none_is_committed() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_manager_test_{}", unique));

        let resolved = resolve_lang_pair_dict_slot(None, "zh-CN", &base_dir);

        let expected = base_dir
            .join("dicts")
            .join("zh-CN")
            .join("text")
            .join("zh-CN_001");
        assert_eq!(resolved, expected.to_string_lossy().to_string());
        assert!(expected.is_dir());

        let _ = std::fs::remove_dir_all(&base_dir);
    }
}

// ============================================================
// RestartScope
// ============================================================

/// バックエンド再起動の範囲。
/// コマンドではなく、設定変更の結果として backend 側が決定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartScope {
    /// translator サーバーのみ再起動。engine は生かしたまま。
    /// engine が死んでいる場合は Full に自動昇格する。
    TranslatorOnly,
    /// engine + translator の両方を再起動。
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleOp {
    Stop,
    SleepMs(u64),
    Start(EngineWaitKind),
}

// ============================================================
// ProcessManager
// ============================================================

pub struct ProcessManager {
    engine: EngineManager,
    config: Config,
    base_dir: PathBuf,
    dictionary: Arc<RwLock<Dictionary>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    server_shutdown_tx: Option<Vec<tokio::sync::oneshot::Sender<()>>>,
    server_runtime: Runtime,
    llm_client: Arc<dyn LlmClient>,
    event_tx: mpsc::Sender<BackendEvent>,
    server_event_tx: tokio_mpsc::Sender<BackendEvent>,
    _bridge_thread: Option<thread::JoinHandle<()>>,
    t_cache: Arc<TranslationCache>,
    n_cache: Arc<NewEntriesCache>,
    input_replay: SharedInputReplayState,
    pub server_port: u16,
    /// 起動中フラグ（再起動時の重複防止）
    starting: bool,
    #[cfg(target_os = "windows")]
    pdh: Option<pdh_vram::PdhQuery>,
}

#[cfg(target_os = "windows")]
use crate::backend::pdh_vram;

impl ProcessManager {
    fn emit_rebuilt_input_snapshot(&self, mark_result_stale: bool) {
        if let Some(snapshot) =
            analysis::rebuild_latest_snapshot(&self.input_replay, mark_result_stale)
        {
            let _ = self
                .event_tx
                .send(BackendEvent::InputAnalysisUpdated(snapshot));
        }
    }

    fn rebuild_dictionary(&mut self) {
        let new_slot_dir = resolve_slot_dir(&self.config, &self.base_dir);
        let new_exact_dict_path = get_exact_dict_path(&self.config, &self.base_dir);
        let new_regex_dict_path = get_regex_dict_path(&self.config, &self.base_dir);
        let new_split_dict_path = get_split_dict_path(&self.config, &self.base_dir);
        let new_bin_path = get_bin_path(&self.config, &self.base_dir);

        self.dictionary = Arc::new(RwLock::new(Dictionary::new(
            new_slot_dir.clone(),
            new_exact_dict_path,
            new_regex_dict_path,
            new_split_dict_path,
            new_bin_path,
            self.event_tx.clone(),
        )));
        // パターン辞書も言語スロットに合わせて再初期化
        self.t_cache = Arc::new(TranslationCache::default());
        self.n_cache = Arc::new(NewEntriesCache::default());

        backend_info!(
            self.event_tx,
            "辞書を再読み込みしました (lang={}, slot={})",
            self.config.tgt_lang,
            self.config.dict_slot.as_deref().unwrap_or("auto")
        );
    }

    fn reload_config_internal(&mut self, config_path: &PathBuf, force_dictionary_reload: bool) {
        if let Ok(new_config) = crate::config::load(config_path) {
            let dict_changed = force_dictionary_reload
                || self.config.tgt_lang != new_config.tgt_lang
                || self.config.dict_slot != new_config.dict_slot;
            let mode_changed = self.config.mode != new_config.mode;
            let game_text_changed = self.config.game_text != new_config.game_text;
            let language_changed = self.config.src_lang != new_config.src_lang
                || self.config.tgt_lang != new_config.tgt_lang
                || self.config.custom_lang_name != new_config.custom_lang_name;
            self.server_port = new_config.server_port;
            self.engine.set_ui_lang(new_config.ui_lang.clone());
            self.config = new_config;

            if dict_changed {
                self.rebuild_dictionary();
            }

            if mode_changed || game_text_changed || language_changed {
                self.emit_rebuilt_input_snapshot(true);
            }
        }
    }

    fn llama_base_url(&self) -> String {
        self.engine.llama_base_url()
    }

    pub fn new(
        config: Config,
        server_cfg: ServerConfig,
        base_dir: PathBuf,
        event_tx: mpsc::Sender<BackendEvent>,
        selected_model: Option<PathBuf>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let (async_tx, mut async_rx) = tokio_mpsc::channel(1000);

        let bridge_tx = event_tx.clone();
        let bridge_thread = thread::spawn(move || {
            while let Some(event) = async_rx.blocking_recv() {
                let _ = bridge_tx.send(event);
            }
        });

        let input_replay = Arc::new(std::sync::Mutex::new(analysis::InputReplayState::default()));
        let slot_dir = resolve_slot_dir(&config, &base_dir);
        let exact_dict_path = get_exact_dict_path(&config, &base_dir);
        let regex_dict_path = get_regex_dict_path(&config, &base_dir);
        let split_dict_path = get_split_dict_path(&config, &base_dir);
        let bin_file = get_bin_path(&config, &base_dir);
        let dictionary = Arc::new(RwLock::new(Dictionary::new(
            slot_dir.clone(),
            exact_dict_path,
            regex_dict_path,
            split_dict_path,
            bin_file,
            event_tx.clone(),
        )));

        let engine = EngineManager::new(
            server_cfg.clone(),
            base_dir.clone(),
            event_tx.clone(),
            selected_model,
            shutdown,
            config.ui_lang.clone(),
        );

        let llm_client = Arc::new(HttpLlmClient::new(format!(
            "{}/chat/completions",
            engine::llama_base_url(&server_cfg.host, server_cfg.port)
        )));
        let server_runtime = Runtime::new().expect("Failed to create Tokio runtime");
        let initial_server_port = config.server_port;
        #[cfg(target_os = "windows")]
        let pdh = pdh_vram::PdhQuery::open();

        Self {
            engine,
            config,
            base_dir,
            dictionary,
            server_handle: None,
            server_shutdown_tx: None,
            server_runtime,
            llm_client,
            event_tx,
            server_event_tx: async_tx,
            _bridge_thread: Some(bridge_thread),
            t_cache: Arc::new(TranslationCache::default()),
            n_cache: Arc::new(NewEntriesCache::default()),
            input_replay,
            server_port: initial_server_port,
            starting: false,
            #[cfg(target_os = "windows")]
            pdh,
        }
    }

    pub fn is_engine_running(&self) -> bool {
        self.engine.is_engine_running()
    }

    pub fn is_translation_server_running(&self) -> bool {
        self.server_handle.is_some()
    }

    pub fn set_selected_model(&mut self, model: Option<PathBuf>) {
        self.engine.set_selected_model(model);
    }

    fn dedicated_vram_mb(&self) -> Option<f32> {
        crate::backend::metrics::dedicated_vram_mb(&self.pdh)
    }

    fn shared_memory_mb(&self) -> Option<f32> {
        crate::backend::metrics::shared_memory_mb(&self.pdh)
    }

    pub fn poll_metrics(&self) -> Option<(Option<f32>, Option<f32>, Option<f32>)> {
        crate::backend::metrics::poll_metrics(self.engine.is_engine_running(), &self.llama_base_url(), &self.pdh)
    }

    pub fn set_game_text_options(&mut self, options: GameTextOptions) {
        if self.config.game_text == options {
            return;
        }

        self.config.game_text = options;
        self.emit_rebuilt_input_snapshot(true);
    }

    /// ポートを設定する。実際に値が変化した場合のみ true を返す。
    pub fn set_server_port(&mut self, port: u16) -> bool {
        if self.server_port == port {
            return false;
        }
        self.server_port = port;
        true
    }

    /// ホストを設定する。実際に値が変化した場合のみ true を返す。
    pub fn set_server_host(&mut self, host: &str) -> bool {
        if self.config.server_host == host {
            return false;
        }
        self.config.server_host = host.to_string();
        true
    }

    pub fn reload_config(&mut self, config_path: &PathBuf) {
        self.reload_config_internal(config_path, false);
    }

    fn lifecycle_sequence(&mut self, ops: &[LifecycleOp]) {
        for op in ops {
            match op {
                LifecycleOp::Stop => {
                    self.stop_translation_server();
                    self.engine.stop_llama_server();
                }
                LifecycleOp::SleepMs(ms) => {
                    thread::sleep(Duration::from_millis(*ms));
                }
                LifecycleOp::Start(wait_kind) => {
                    self.start_all_with_wait_kind(*wait_kind);
                }
            }
        }
    }

    pub fn start_all(&mut self) {
        self.start_all_with_wait_kind(EngineWaitKind::NormalStartup);
    }

    fn start_all_with_wait_kind(&mut self, wait_kind: EngineWaitKind) {
        if self.starting {
            return;
        }

        self.starting = true;

        let engine_success = if self.engine.has_live_llama_process() {
            true
        } else {
            self.engine.start_llama_server(wait_kind)
        };

        let translator_success = if engine_success {
            if self.server_handle.is_some() {
                true
            } else {
                self.start_translation_server()
            }
        } else {
            let msg = if self.config.ui_lang == "en" {
                "Inference engine failed to start; translation server will not launch."
            } else {
                "推論エンジンの起動に失敗したため、翻訳サーバーは起動しませんでした"
            };
            let _ = self.event_tx.send(BackendEvent::Log(
                LogSource::Tenuki,
                msg.to_string(),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            false
        };

        let _ = self.event_tx.send(BackendEvent::BackendReady {
            engine_success,
            translator_success,
        });

        self.starting = false;
    }

    pub fn restart_for_model_switch(&mut self) {
        self.lifecycle_sequence(&[
            LifecycleOp::Stop,
            LifecycleOp::Start(EngineWaitKind::ModelSwitch),
        ]);
    }

    pub fn stop_all(&mut self) {
        self.lifecycle_sequence(&[LifecycleOp::Stop, LifecycleOp::SleepMs(500)]);
    }

    pub fn restart_all(&mut self) {
        self.lifecycle_sequence(&[
            LifecycleOp::Stop,
            LifecycleOp::SleepMs(1000),
            LifecycleOp::Start(EngineWaitKind::NormalStartup),
        ]);
    }

    /// 設定変更後の再起動を scope に従って実行する。
    /// `TranslatorOnly` でも engine が死んでいれば `Full` に昇格する。
    /// 戻り値: (engine_success, translator_success)
    pub fn apply_restart(&mut self, scope: RestartScope) -> (bool, bool) {
        match scope {
            RestartScope::Full => {
                self.stop_all();
                self.start_all();
                (
                    self.is_engine_running(),
                    self.is_translation_server_running(),
                )
            }
            RestartScope::TranslatorOnly => {
                if !self.is_engine_running() {
                    // engine 不在 → Full に昇格
                    self.start_all();
                    return (
                        self.is_engine_running(),
                        self.is_translation_server_running(),
                    );
                }
                let translator_success = self.restart_translator();
                (true, translator_success)
            }
        }
    }

    /// translator サーバーを辞書保存・設定再読込込みで再起動する。
    /// engine は触らない。
    pub fn restart_translator(&mut self) -> bool {
        self.stop_translation_server();
        let _ = self.save_dictionary();
        self.start_translation_server()
    }

    pub fn save_dictionary(&mut self) -> usize {
        // stop_translation_server 済みなら n_cache はすでに dict.buffer へ登録済み。
        // 未停止の場合に備えて念のため呼ぶ（n_cache が空なら即リターン）。
        self.flush_n_cache_to_dict();
        let dict = self.dictionary.clone();
        log::info!("[FLUSH] dictionary_flush_buffer start");
        let written = self.server_runtime.block_on(async {
            let mut d = dict.write().await;
            d.flush_buffer()
        });
        log::info!("[FLUSH] dictionary_flush_buffer done count={}", written);
        written
    }

    /// n_cache の全エントリを dict.register して n_cache をクリアする
    fn flush_n_cache_to_dict(&mut self) {
        if self.n_cache.is_empty() {
            return;
        }
        let entries = self.n_cache.drain();
        log::info!("[FLUSH] n_cache_drain count={}", entries.len());
        let dict = self.dictionary.clone();
        self.server_runtime.block_on(async {
            let mut d = dict.write().await;
            for (k, v) in &entries {
                log::info!("[FLUSH] dictionary_register key='{}'", k);
                d.register(k, v);
            }
        });
    }

    pub fn check_alive(&mut self) {
        let engine_alive = self.engine.has_live_llama_process();

        let server_finished = self
            .server_handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(false);

        if server_finished {
            if let Some(handle) = self.server_handle.take() {
                self.server_runtime.block_on(async {
                    let _ = handle.await;
                });
            }
            self.server_shutdown_tx = None;
            let _ = self
                .event_tx
                .send(BackendEvent::ProcessStatus(ProcessType::Tenuki, false));
        }

        if !engine_alive && self.server_handle.is_some() {
            self.stop_translation_server();
        }
    }

    // ----------------------------------------------------------

    fn start_translation_server(&mut self) -> bool {
        if self.server_handle.is_some() {
            return true;
        }

        let dictionary = self.dictionary.clone();
        let src_lang = self.config.src_lang.clone();
        let tgt_lang = self.config.tgt_lang.clone();
        let custom_lang_name = self.config.custom_lang_name.clone();
        let prompt_template = self.config.prompt_template.clone();
        let background_text = self.config.background_text.clone();
        let enable_model_wrap = self.config.effective_model_wrap();
        let model_wrap_min_chars = self.config.model_wrap_min_chars;
        let model_wrap_space_fallback_min_chars = self.config.model_wrap_space_fallback_min_chars;
        let enable_model_symbol_cleanup = self.config.enable_model_symbol_cleanup;
        let llm_client = self.llm_client.clone();
        let server_event_tx = self.server_event_tx.clone();
        let host = self.config.server_host.clone();
        let port = self.server_port;
        let t_cache = self.t_cache.clone();
        let n_cache = self.n_cache.clone();
        let input_replay = self.input_replay.clone();
        let llm_slots = self.engine.llm_slots().max(1);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (tcp_shutdown_tx, tcp_shutdown_rx) = tokio::sync::oneshot::channel();
        let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();

        let translation_settings = TranslationSettings {
            enable_model_wrap,
            model_wrap_min_chars: model_wrap_min_chars as usize,
            model_wrap_space_fallback_min_chars: model_wrap_space_fallback_min_chars as usize,
            enable_model_symbol_cleanup,
        };

        let handle = self.server_runtime.spawn(server::run_translation_server(
            host.clone(),
            port,
            port + 1,
            dictionary,
            src_lang,
            tgt_lang,
            custom_lang_name,
            prompt_template,
            background_text,
            translation_settings,
            llm_client,
            server_event_tx,
            startup_tx,
            shutdown_rx,
            tcp_shutdown_rx,
            t_cache,
            n_cache,
            input_replay,
            llm_slots,
        ));

        match self
            .server_runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(15), startup_rx).await })
        {
            Ok(Ok(Ok(()))) => {
                self.server_shutdown_tx = Some(vec![shutdown_tx, tcp_shutdown_tx]);
                self.server_handle = Some(handle);
                let _ = self
                    .event_tx
                    .send(BackendEvent::ProcessStatus(ProcessType::Tenuki, true));

                let bind_msg = if self.config.ui_lang == "en" {
                    if host == "0.0.0.0" {
                        format!("Translation server started (http://0.0.0.0:{}) - accessible from network", port)
                    } else {
                        format!("Translation server started (http://{}:{})", host, port)
                    }
                } else {
                    if host == "0.0.0.0" {
                        format!("翻訳サーバーを起動しました (http://0.0.0.0:{}) - ネットワークからアクセス可能", port)
                    } else {
                        format!("翻訳サーバーを起動しました (http://{}:{})", host, port)
                    }
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    bind_msg,
                    LogLevel::Info,
                    crate::messages::current_timestamp(),
                ));

                true
            }
            Ok(Ok(Err(e))) => {
                self.server_runtime.block_on(async {
                    let _ = handle.await;
                });
                let msg = if self.config.ui_lang == "en" {
                    format!("Translation server failed to start: {e}")
                } else {
                    format!("翻訳サーバーの起動に失敗しました: {e}")
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                false
            }
            Ok(Err(_)) => {
                self.server_runtime.block_on(async {
                    let _ = handle.await;
                });
                let msg = if self.config.ui_lang == "en" {
                    "Translation server startup channel closed unexpectedly".to_string()
                } else {
                    "翻訳サーバー起動チャンネルが予期せず閉じられました".to_string()
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                false
            }
            Err(_) => {
                handle.abort();
                self.server_runtime.block_on(async {
                    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
                });
                let msg = if self.config.ui_lang == "en" {
                    "Translation server startup timed out (15s)".to_string()
                } else {
                    "翻訳サーバーの起動がタイムアウトしました（15秒）".to_string()
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                false
            }
        }
    }

    fn stop_translation_server(&mut self) {
        // 1. シャットダウンシグナル送信（サーバーが新規リクエストを受け付けなくなる）
        if let Some(txs) = self.server_shutdown_tx.take() {
            for tx in txs {
                let _ = tx.send(());
            }
        }

        if let Some(mut handle) = self.server_handle.take() {
            self.server_runtime.block_on(async {
                match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
                    Ok(_) => backend_info!(self.event_tx, "Translation server stopped gracefully"),
                    Err(_) => {
                        handle.abort();
                        let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
                    }
                }
            });
        }

        // 3. サーバー停止後に n_cache → dict.buffer へ登録
        //    サーバーが完全停止しているため dict の write lock 競合なし
        self.flush_n_cache_to_dict();
        let _ = self
            .event_tx
            .send(BackendEvent::ProcessStatus(ProcessType::Tenuki, false));

        // 4. キャッシュをリセット（次回起動に備えて）
        self.t_cache = Arc::new(TranslationCache::default());
        self.n_cache = Arc::new(NewEntriesCache::default());

        // ポートが解放されるまで待つ（最大5秒）
        let port = self.server_port;
        if engine::is_port_open(port) {
            let closed = engine::wait_for_port_closed(port, Duration::from_secs(5));
            if !closed {
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    format!(
                        "警告: 翻訳サーバー停止後もポート {} が解放されていません",
                        port
                    ),
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
            }
        }

        backend_info!(self.event_tx, "Translation server stopped");
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        self.stop_all();

        // Close the async->sync bridge channel before joining the bridge thread.
        // If we keep the sender alive here, blocking_recv() never returns None and
        // application shutdown can hang indefinitely.
        let (dummy_tx, _dummy_rx) = tokio_mpsc::channel(1);
        let old_tx = std::mem::replace(&mut self.server_event_tx, dummy_tx);
        drop(old_tx);

        if let Some(handle) = self._bridge_thread.take() {
            let _ = handle.join();
        }
    }
}
