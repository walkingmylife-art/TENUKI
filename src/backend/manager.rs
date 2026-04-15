// src/backend/manager.rs

//! バックエンド管理モジュール

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::runtime::Runtime;

use crate::config::{Config, StructuralOptions};
use crate::launcher::app_config::ServerConfig;
use crate::backend::analysis::{self, SharedInputReplayState};
use crate::messages::{BackendEvent, LogLevel, LogSource, ProcessType};
use crate::backend::dictionary::Dictionary;
use crate::backend::process::LlamaProcess;
use crate::backend::processor::{
    ProcessorFactory, TextProcessor, TranslationMode,
};
use crate::backend::translator::{LlmClient, HttpLlmClient, TranslationCache, NewEntriesCache};
use crate::backend::translator::TranslationSettings;
use crate::backend::server;
use crate::backend_info;

pub const DEFAULT_SERVER_PORT: u16 = 14371;

// ============================================================
// スロット管理ユーティリティ
// ============================================================

fn max_existing_slot_num(text_dir: &Path, lang: &str) -> Option<u32> {
    std::fs::read_dir(text_dir).ok()?.filter_map(|e| {
        let e = e.ok()?;
        if !e.file_type().ok()?.is_dir() {
            return None;
        }

        let name = e.file_name().to_string_lossy().into_owned();
        slot_num_from_name(&name, lang)
    }).max()
}

fn is_slot_dir_name(name: &str) -> bool {
    slot_num_from_name(name, "").is_some()
}

fn is_slot_dir_name_for_lang(name: &str, lang: &str) -> bool {
    slot_num_from_name(name, lang).is_some()
}

fn slot_num_from_name(name: &str, lang: &str) -> Option<u32> {
    let (prefix, suffix) = name.rsplit_once('_')?;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let is_current_lang_slot = !lang.is_empty() && prefix == lang;
    let is_legacy_slot = prefix == "S" && suffix.len() == 4;

    if is_current_lang_slot || is_legacy_slot {
        suffix.parse::<u32>().ok()
    } else if lang.is_empty()
        && !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        suffix.parse::<u32>().ok()
    } else {
        None
    }
}

fn find_slot_ancestor(path: &Path, lang: &str) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| is_slot_dir_name_for_lang(name, lang))
            .map(|_| ancestor.to_path_buf())
    })
}

pub fn is_slot_dir(p: &Path) -> bool {
    let parent_is_text = p
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("text"))
        .unwrap_or(false);

    parent_is_text
        && p.file_name()
            .and_then(|n| n.to_str())
            .map(is_slot_dir_name)
            .unwrap_or(false)
}

pub fn find_or_create_slot_under(container: &Path, lang: &str) -> PathBuf {
    if let Some(existing_slot) = find_slot_ancestor(container, lang) {
        let _ = std::fs::create_dir_all(&existing_slot);
        return existing_slot;
    }

    let _ = std::fs::create_dir_all(container);
    if let Some(max) = max_existing_slot_num(container, lang) {
        let next_num = max + 1;
        let slot = container.join(format!("{}_{:03}", lang, next_num));
        let _ = std::fs::create_dir_all(&slot);
        slot
    } else {
        let slot = container.join(format!("{}_001", lang));
        let _ = std::fs::create_dir_all(&slot);
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::{find_or_create_slot_under, find_slot_ancestor, is_slot_dir, is_slot_dir_name_for_lang, resolve_slot_dir};
    use crate::config::Config;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn finds_language_slot_ancestor_from_legacy_nested_slot() {
        let slot = find_slot_ancestor(Path::new(r"C:\dicts\ja\text\ja_001\s_0001"), "ja");
        assert_eq!(slot, Some(PathBuf::from(r"C:\dicts\ja\text\ja_001")));
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
        assert_eq!(slot, text_dir.join("ja_002"));

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
}

pub fn create_new_slot(tgt_lang: &str, base_dir: &PathBuf) -> PathBuf {
    let text_dir = base_dir.join("dicts").join(tgt_lang).join("text");
    let _ = std::fs::create_dir_all(&text_dir);
    let next_num = max_existing_slot_num(&text_dir, tgt_lang).map(|n| n + 1).unwrap_or(1);
    let slot = text_dir.join(format!("{}_{:03}", tgt_lang, next_num));
    let _ = std::fs::create_dir_all(&slot);
    slot
}

pub fn resolve_slot_dir(config: &Config, base_dir: &PathBuf) -> PathBuf {
    if let Some(slot) = &config.dict_slot {
        if !slot.is_empty() {
            let p = PathBuf::from(slot);
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }

    let text_dir = base_dir.join("dicts").join(&config.tgt_lang).join("text");
    find_or_create_slot_under(&text_dir, &config.tgt_lang)
}

pub fn get_dict_path(config: &Config, base_dir: &PathBuf) -> PathBuf {
    resolve_slot_dir(config, base_dir).join("dict.txt")
}

/// dict.bin は常に起動時に再生成されるため dicts/ 直下に1つだけ置く
pub fn get_bin_path(_config: &Config, base_dir: &PathBuf) -> PathBuf {
    let dir = base_dir.join("dicts");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("dict.bin")
}

fn is_port_open(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn is_local_llama_host(host: &str) -> bool {
    matches!(host.trim(), "127.0.0.1" | "localhost" | "0.0.0.0" | "")
}

fn llama_connect_host(host: &str) -> &str {
    match host.trim() {
        "" | "0.0.0.0" | "localhost" => "127.0.0.1",
        other => other,
    }
}

fn llama_base_url(host: &str, port: u16) -> String {
    format!("http://{}:{}", llama_connect_host(host), port)
}

pub fn get_local_ip() -> String {
    // シンプルな方法: UDPソケットで外部に接続尝试してローカルIPを取得
    let socket = std::net::UdpSocket::bind("0.0.0.0:0");
    match socket {
        Ok(s) => {
            // ループバック以外のIPを取得するため、外部アドレスに接続尝试
            let _ = s.connect("8.8.8.8:80");
            s.local_addr()
                .map(|addr| addr.ip().to_string())
                .unwrap_or_else(|_| "127.0.0.1".to_string())
        }
        Err(_) => "127.0.0.1".to_string(),
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

// ============================================================
// ProcessManager
// ============================================================

pub struct ProcessManager {
    config: Config,
    /// llama-server 起動条件（launcher_config.toml 由来・唯一の権威）
    server_cfg: ServerConfig,
    base_dir: PathBuf,
    current_processor: Arc<dyn TextProcessor>,
    translation_mode: TranslationMode,
    dictionary: Arc<RwLock<Dictionary>>,
    llama_process: Option<LlamaProcess>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    server_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_runtime: Runtime,
    llm_client: Arc<dyn LlmClient>,
    event_tx: mpsc::Sender<BackendEvent>,
    server_event_tx: tokio_mpsc::Sender<BackendEvent>,
    _bridge_thread: Option<thread::JoinHandle<()>>,
    t_cache: Arc<TranslationCache>,
    n_cache: Arc<NewEntriesCache>,
    input_replay: SharedInputReplayState,
    pub ctx_size: u32,
    pub selected_model: Option<PathBuf>,
    pub server_port: u16,
    llm_slots: usize,
    /// 起動中フラグ（再起動時の重複防止）
    starting: bool,
    shutdown: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    pdh: Option<pdh_vram::PdhQuery>,
}

// ============================================================
// ヘルパー関数
// ============================================================

/// base_dir から llama-server 実行ファイルを探す
///
/// 優先順:
///   1. launcher_config.toml の backend 名から runtime/<backend>/ を直接探索
///   2. runtime/ 配下を max_depth(3) で広く探索
///   3. base_dir 直下 / llama-server サブフォルダ（旧来フォールバック）
///
/// 起動判断の権威は launcher_config.toml。state.json は参照しない。
fn find_llama_exe(base_dir: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let name = "llama-server.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "llama-server";

    // 1. launcher_config.toml の backend から runtime/<backend>/ を直接探索
    let launcher_config_path = base_dir.join("launcher_config.toml");
    let backend = std::fs::read_to_string(&launcher_config_path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .and_then(|v| v.get("backend")?.as_str().map(|s| s.to_string()));

    if let Some(backend) = backend {
        let backend_dir = base_dir.join("runtime").join(&backend);
        if backend_dir.exists() {
            let found = walkdir::WalkDir::new(&backend_dir)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
                .find(|e| e.file_name().to_string_lossy() == name)
                .map(|e| e.path().to_path_buf());
            if found.is_some() {
                return found;
            }
        }
    }

    // 2. runtime/ 配下を広く探索
    let runtime_dir = base_dir.join("runtime");
    if runtime_dir.exists() {
        let found = walkdir::WalkDir::new(&runtime_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy() == name)
            .map(|e| e.path().to_path_buf());
        if found.is_some() {
            return found;
        }
    }

    // 3. 旧来フォールバック
    let candidate = base_dir.join(name);
    if candidate.exists() { return Some(candidate); }

    let in_subdir = base_dir.join("llama-server").join(name);
    if in_subdir.exists() { return Some(in_subdir); }

    None
}

/// llama-server の /health エンドポイントが {"status":"ok"} を返すまで待機
///
/// llama-server.exe はモデルロード前にポートを開くため、TCP 接続成功だけでは
/// モデルが準備完了かどうか判定できない。/health が 200 OK を返すまで待つ。
/// 最大待機: 約 120 秒（大型モデルの VRAM ロードを考慮）
fn wait_for_llama_server(
    host: &str,
    port: u16,
    event_tx: &mpsc::Sender<BackendEvent>,
    shutdown: &Arc<AtomicBool>,
    ui_lang: &str,
) -> bool {
    let connect_host = llama_connect_host(host);
    let addr: SocketAddr = format!("{}:{}", connect_host, port).parse().unwrap();
    let health_url = format!("{}/health", llama_base_url(host, port));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(500))
        .timeout_read(Duration::from_secs(5))
        .build();

    let mut backoff = Duration::from_millis(500);
    for attempt in 0..60 {
        if shutdown.load(Ordering::Relaxed) { return false; }
        // まず TCP レベルで到達できるか確認（ポートが開いていない段階をスキップ）
        if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err() {
            thread::sleep(backoff);
            backoff = std::cmp::min(backoff * 2, Duration::from_secs(3));
            continue;
        }

        // TCP は開いている → /health を確認
        match agent.get(&health_url).call() {
            Ok(response) if response.status() == 200 => {
                // body を読んで "ok" を確認（読めなくても 200 なら OK とみなす）
                let is_ok = response.into_string()
                    .map(|s| s.contains("\"ok\""))
                    .unwrap_or(true);
                if is_ok {
                    return true;
                }
                // {"status":"loading model"} → まだ待つ
                let msg = if ui_lang == "en" {
                    format!("llama-server loading model... (attempt {})", attempt + 1)
                } else {
                    format!("llama-server モデルロード中... ({}回目)", attempt + 1)
                };
                let _ = event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Info,
                    crate::messages::current_timestamp(),
                ));
            }
            Ok(_) => {
                // 503 など → まだロード中
                let msg = if ui_lang == "en" {
                    format!("llama-server loading model... (attempt {})", attempt + 1)
                } else {
                    format!("llama-server モデルロード中... ({}回目)", attempt + 1)
                };
                let _ = event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Info,
                    crate::messages::current_timestamp(),
                ));
            }
            Err(_) => {
                // 接続エラー → 待つ
            }
        }

        thread::sleep(backoff);
        backoff = std::cmp::min(backoff * 2, Duration::from_secs(3));
    }
    false
}

fn parse_metric_value(body: &str, metric_name: &str) -> Option<f32> {
    body.lines()
        .find_map(|line| {
            if line.starts_with('#') {
                return None;
            }

            let (name, value) = line.split_once(' ')?;
            if name == metric_name {
                return value.trim().parse::<f32>().ok();
            }

            None
        })
}

#[cfg(target_os = "windows")]
mod pdh_vram {
    use windows::core::PCWSTR;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW,
        PdhCloseQuery,
        PdhCollectQueryData,
        PdhGetFormattedCounterArrayW,
        PdhOpenQueryW,
        PDH_FMT_COUNTERVALUE_ITEM_W,
        PDH_FMT_LARGE,
    };

    pub struct PdhQuery {
        query: isize,
        dedicated_counter: isize,
        shared_counter: Option<isize>,
    }

    impl PdhQuery {
        pub fn open() -> Option<Self> {
            unsafe {
                let mut query: isize = 0;
                if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }

                let dedicated_path: Vec<u16> =
                    "\\GPU Adapter Memory(*)\\Dedicated Usage\0".encode_utf16().collect();
                let shared_path: Vec<u16> =
                    "\\GPU Adapter Memory(*)\\Shared Usage\0".encode_utf16().collect();

                let mut dedicated_counter: isize = 0;
                if PdhAddEnglishCounterW(
                    query,
                    PCWSTR::from_raw(dedicated_path.as_ptr()),
                    0,
                    &mut dedicated_counter,
                ) != 0 {
                    PdhCloseQuery(query);
                    return None;
                }

                let mut shared_counter: isize = 0;
                let shared_counter = if PdhAddEnglishCounterW(
                    query,
                    PCWSTR::from_raw(shared_path.as_ptr()),
                    0,
                    &mut shared_counter,
                ) == 0 {
                    Some(shared_counter)
                } else {
                    None
                };

                let _ = PdhCollectQueryData(query);

                Some(Self {
                    query,
                    dedicated_counter,
                    shared_counter,
                })
            }
        }

        fn collect_counter_mb(&self, counter: isize) -> Option<f32> {
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }

                let mut buffer_size: u32 = 0;
                let mut item_count: u32 = 0;

                let _ = PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_LARGE,
                    &mut buffer_size,
                    &mut item_count,
                    None,
                );

                if buffer_size == 0 || item_count == 0 {
                    return None;
                }

                let mut buffer = vec![0u8; buffer_size as usize];
                if PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_LARGE,
                    &mut buffer_size,
                    &mut item_count,
                    Some(buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
                ) != 0 {
                    return None;
                }

                let items = std::slice::from_raw_parts(
                    buffer.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W,
                    item_count as usize,
                );

                let bytes = items
                    .iter()
                    .map(|item| item.FmtValue.Anonymous.largeValue)
                    .max()
                    .unwrap_or(0)
                    .max(0) as f32;

                Some(bytes / (1024.0 * 1024.0))
            }
        }

        pub fn collect_dedicated_mb(&self) -> Option<f32> {
            self.collect_counter_mb(self.dedicated_counter)
        }

        pub fn collect_shared_mb(&self) -> Option<f32> {
            self.shared_counter
                .and_then(|counter| self.collect_counter_mb(counter))
        }
    }

    impl Drop for PdhQuery {
        fn drop(&mut self) {
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}

impl ProcessManager {
    fn emit_rebuilt_input_snapshot(&self, mark_result_stale: bool) {
        if let Some(snapshot) = analysis::rebuild_latest_snapshot(
            &self.input_replay,
            self.current_processor.as_ref(),
            mark_result_stale,
        ) {
            let _ = self.event_tx.send(BackendEvent::InputAnalysisUpdated(snapshot));
        }
    }

    fn rebuild_processor(&mut self) {
        self.current_processor =
            Arc::from(ProcessorFactory::create(self.translation_mode, self.config.structural));
        self.emit_rebuilt_input_snapshot(true);
    }

    fn rebuild_dictionary(&mut self) {
        let new_slot_dir = resolve_slot_dir(&self.config, &self.base_dir);
        let new_dict_path = get_dict_path(&self.config, &self.base_dir);
        let new_bin_path = get_bin_path(&self.config, &self.base_dir);

        self.dictionary = Arc::new(RwLock::new(
            Dictionary::new(new_slot_dir.clone(), new_dict_path, new_bin_path, self.event_tx.clone())
        ));
        // パターン辞書も言語スロットに合わせて再初期化
        self.t_cache = Arc::new(TranslationCache::default());
        self.n_cache = Arc::new(NewEntriesCache::default());

        backend_info!(self.event_tx,
            "辞書を再読み込みしました (lang={}, slot={})",
            self.config.tgt_lang,
            self.config.dict_slot.as_deref().unwrap_or("auto"));
    }

    fn reload_config_internal(&mut self, config_path: &PathBuf, force_dictionary_reload: bool) {
        if let Ok(new_config) = crate::config::load(config_path) {
            let dict_changed = force_dictionary_reload
                || self.config.tgt_lang != new_config.tgt_lang
                || self.config.dict_slot != new_config.dict_slot;
            let mode_changed = self.config.translation_mode != new_config.translation_mode;
            let structural_changed = self.config.structural != new_config.structural;
            let language_changed = self.config.src_lang != new_config.src_lang
                || self.config.tgt_lang != new_config.tgt_lang
                || self.config.custom_lang_code != new_config.custom_lang_code
                || self.config.custom_lang_name != new_config.custom_lang_name;
            self.server_port = new_config.server_port;
            self.config = new_config;

            if dict_changed {
                self.rebuild_dictionary();
            }

            if mode_changed {
                let new_mode = TranslationMode::from_str(&self.config.translation_mode);
                self.set_translation_mode(new_mode);
            } else if structural_changed {
                self.rebuild_processor();
            } else if language_changed {
                self.emit_rebuilt_input_snapshot(true);
            }
        }
    }

    fn wait_for_port_closed(&self, port: u16, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if !is_port_open(port) {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        !is_port_open(port)
    }

    #[cfg(target_os = "windows")]
    fn terminate_stray_llama_server(&self) -> bool {
        match Command::new("taskkill")
            .args(["/IM", "llama-server.exe", "/F"])
            .status()
        {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn terminate_stray_llama_server(&self) -> bool {
        false
    }

    fn has_live_llama_process(&mut self) -> bool {
        if !is_local_llama_host(&self.server_cfg.host) {
            return self.check_remote_llama_endpoint();
        }

        let is_alive = self.llama_process
            .as_mut()
            .map(|proc| proc.is_alive())
            .unwrap_or(false);

        if !is_alive && self.llama_process.is_some() {
            self.llama_process = None;
            let _ = self.event_tx.send(BackendEvent::ProcessStatus(
                ProcessType::InferenceEngine,
                false,
            ));
        }

        is_alive
    }

    fn llama_base_url(&self) -> String {
        llama_base_url(&self.server_cfg.host, self.server_cfg.port)
    }

    fn check_remote_llama_endpoint(&self) -> bool {
        let health_url = format!("{}/health", self.llama_base_url());
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(500))
            .timeout_read(Duration::from_secs(2))
            .build();

        let alive = agent
            .get(&health_url)
            .call()
            .ok()
            .is_some_and(|response| response.status() == 200);

        let _ = self.event_tx.send(BackendEvent::ProcessStatus(
            ProcessType::InferenceEngine,
            alive,
        ));
        alive
    }

    pub fn new(
        config: Config,
        server_cfg: ServerConfig,
        base_dir: PathBuf,
        event_tx: mpsc::Sender<BackendEvent>,
        selected_model: Option<PathBuf>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let mode = TranslationMode::from_str(&config.translation_mode);

        let (async_tx, mut async_rx) = tokio_mpsc::channel(1000);

        let bridge_tx = event_tx.clone();
        let bridge_thread = thread::spawn(move || {
            while let Some(event) = async_rx.blocking_recv() {
                let _ = bridge_tx.send(event);
            }
        });

        let processor: Arc<dyn TextProcessor> =
            Arc::from(ProcessorFactory::create(mode, config.structural));
        let input_replay = Arc::new(std::sync::Mutex::new(analysis::InputReplayState::default()));
        let slot_dir = resolve_slot_dir(&config, &base_dir);
        let dict_path = get_dict_path(&config, &base_dir);
        let bin_file  = get_bin_path(&config, &base_dir);
        let dictionary = Arc::new(RwLock::new(
            Dictionary::new(slot_dir.clone(), dict_path, bin_file, event_tx.clone())
        ));
        let ctx_size = server_cfg.ctx_size;
        let llm_slots = server_cfg.parallel_slots.max(1) as usize;
        let llm_client = Arc::new(HttpLlmClient::new(format!(
            "{}/chat/completions",
            llama_base_url(&server_cfg.host, server_cfg.port)
        )));
        let server_runtime = Runtime::new().expect("Failed to create Tokio runtime");
        #[cfg(target_os = "windows")]
        let pdh = pdh_vram::PdhQuery::open();

        Self {
            config,
            server_cfg,
            base_dir,
            current_processor: processor,
            translation_mode: mode,
            dictionary,
            llama_process: None,
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
            ctx_size,
            selected_model,
            server_port: DEFAULT_SERVER_PORT,
            llm_slots,
            starting: false,
            shutdown,
            #[cfg(target_os = "windows")]
            pdh,
        }
    }

    pub fn is_running(&self) -> bool {
        self.server_handle.is_some() || self.llama_process.is_some()
    }

    pub fn is_engine_running(&self) -> bool {
        self.llama_process.is_some()
    }

    pub fn is_translation_server_running(&self) -> bool {
        self.server_handle.is_some()
    }

    fn dedicated_vram_mb(&self) -> Option<f32> {
        #[cfg(target_os = "windows")]
        {
            return self
                .pdh
                .as_ref()
                .and_then(|query| query.collect_dedicated_mb());
        }

        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    fn shared_memory_mb(&self) -> Option<f32> {
        #[cfg(target_os = "windows")]
        {
            return self
                .pdh
                .as_ref()
                .and_then(|query| query.collect_shared_mb());
        }

        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    /// llama-server の /metrics から tokens/s を取得する。
    pub fn poll_metrics(&self) -> Option<(Option<f32>, Option<f32>, Option<f32>)> {
        if self.llama_process.is_none() {
            return None;
        }

        let vram_mb = self.dedicated_vram_mb();
        let shared_mb = self.shared_memory_mb();
        let metrics_url = format!("{}/metrics", self.llama_base_url());
        let tokens_per_second = ureq::get(&metrics_url)
            .timeout(Duration::from_millis(500))
            .call()
            .ok()
            .and_then(|response| response.into_string().ok())
            .and_then(|body| parse_metric_value(&body, "llamacpp:predicted_tokens_seconds"));

        if tokens_per_second.is_none() && vram_mb.is_none() && shared_mb.is_none() {
            return None;
        }

        Some((tokens_per_second, vram_mb, shared_mb))
    }

    pub fn set_translation_mode(&mut self, mode: TranslationMode) {
        if self.translation_mode == mode { return; }
        self.translation_mode = mode;
        // モードに応じた server_cfg をメモリ上で差し替える（launcher_config.toml は書かない）
        let mode_str = match mode {
            TranslationMode::Passthrough => "passthrough",
            _ => "structural",
        };
        let default_cfg = crate::launcher::app_config::AppConfig::default_for_mode(mode_str);
        // host/port はユーザー設定を維持し、スループット関連のみ差し替える
        self.server_cfg.ctx_size = default_cfg.server.ctx_size;
        self.server_cfg.batch_size = default_cfg.server.batch_size;
        self.server_cfg.ubatch_size = default_cfg.server.ubatch_size;
        self.server_cfg.parallel_slots = default_cfg.server.parallel_slots;
        self.ctx_size = default_cfg.server.ctx_size;
        self.llm_slots = default_cfg.server.parallel_slots.max(1) as usize;
        self.rebuild_processor();
    }

    pub fn set_structural_options(&mut self, options: StructuralOptions) {
        if self.config.structural == options {
            return;
        }

        self.config.structural = options;
        self.rebuild_processor();
    }

    pub fn set_server_port(&mut self, port: u16) {
        self.server_port = port;
    }

    pub fn set_server_host(&mut self, host: &str) {
        self.config.server_host = host.to_string();
    }

    pub fn set_enable_model_wrap(&mut self, enabled: bool) {
        self.config.enable_model_wrap = enabled;
    }

    pub fn set_model_wrap_min_chars(&mut self, value: u32) {
        self.config.model_wrap_min_chars = value;
    }

    pub fn set_model_wrap_min_tail_chars(&mut self, value: u32) {
        self.config.model_wrap_min_tail_chars = value;
    }

    pub fn set_enable_model_symbol_cleanup(&mut self, enabled: bool) {
        self.config.enable_model_symbol_cleanup = enabled;
    }

    pub fn update_settings(
        &mut self,
        ctx_size: Option<u32>,
        model: Option<PathBuf>,
    ) {
        if let Some(v) = ctx_size { self.ctx_size = v; }
        if let Some(v) = model {
            self.selected_model = Some(v);
        }
    }

    pub fn reload_config(&mut self, config_path: &PathBuf) {
        self.reload_config_internal(config_path, false);
    }

    pub fn start_all(&mut self) {
        if self.starting { return; }

        self.starting = true;

        let _ = self.event_tx.send(BackendEvent::Log(
            LogSource::Tenuki,
            "start_all: launching inference engine...".to_string(),
            LogLevel::Info,
            crate::messages::current_timestamp(),
        ));

        let engine_success = if self.has_live_llama_process() {
            true
        } else {
            self.start_llama_server()
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

    pub fn stop_all(&mut self) {
        self.stop_translation_server();
        self.stop_llama_server();
        thread::sleep(Duration::from_millis(500));
    }

    pub fn restart_all(&mut self) {
        self.stop_all();
        // 完全に停止してから起動
        thread::sleep(Duration::from_millis(1000));
        self.start_all();
    }

    pub fn restart_translation_server(&mut self) -> bool {
        self.stop_translation_server();
        self.start_translation_server()
    }

    /// 設定変更後の再起動を scope に従って実行する。
    /// `TranslatorOnly` でも engine が死んでいれば `Full` に昇格する。
    /// 戻り値: (engine_success, translator_success)
    pub fn apply_restart(&mut self, scope: RestartScope) -> (bool, bool) {
        match scope {
            RestartScope::Full => {
                self.stop_all();
                self.start_all();
                (self.is_engine_running(), self.is_translation_server_running())
            }
            RestartScope::TranslatorOnly => {
                if !self.is_engine_running() {
                    // engine 不在 → Full に昇格
                    self.start_all();
                    return (self.is_engine_running(), self.is_translation_server_running());
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
        self.server_runtime.block_on(async {
            let mut d = dict.write().await;
            d.flush_buffer()
        })
    }

    /// n_cache の全エントリを dict.register して n_cache をクリアする
    fn flush_n_cache_to_dict(&mut self) {
        if self.n_cache.is_empty() { return; }
        let entries = self.n_cache.drain();
        let dict = self.dictionary.clone();
        self.server_runtime.block_on(async {
            let mut d = dict.write().await;
            for (k, v) in &entries {
                d.register(k, v);
            }
        });
    }

    pub fn check_alive(&mut self) {
        let engine_alive = self.has_live_llama_process();

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
            let _ = self.event_tx.send(BackendEvent::ProcessStatus(
                ProcessType::Tenuki,
                false,
            ));
        }

        if !engine_alive && self.server_handle.is_some() {
            self.stop_translation_server();
        }
    }

    // ----------------------------------------------------------

    fn start_llama_server(&mut self) -> bool {
        if self.has_live_llama_process() { return true; }

        if !is_local_llama_host(&self.server_cfg.host) {
            return self.check_remote_llama_endpoint();
        }

        let model = match self.resolve_model() {
            Some(m) => m,
            None => return false,
        };

        let exe = match find_llama_exe(&self.base_dir) {
            Some(e) => e,
            None => return false,
        };

        if is_port_open(self.server_cfg.port) {
            if !self.terminate_stray_llama_server()
                || !self.wait_for_port_closed(self.server_cfg.port, Duration::from_secs(5))
            {
                return false;
            }
        }

        match LlamaProcess::start(
            &exe,
            &model,
            self.server_cfg.ngl,
            self.ctx_size,
            self.server_cfg.batch_size,
            self.server_cfg.ubatch_size,
            self.server_cfg.cont_batching,
            self.llm_slots.max(1) as u32,
            self.server_cfg.port,
            self.event_tx.clone(),
        ) {
            Ok(proc) => {
                self.llama_process = Some(proc);
                let _ = self.event_tx.send(BackendEvent::ProcessStatus(
                    ProcessType::InferenceEngine, true,
                ));
                if wait_for_llama_server(
                    &self.server_cfg.host,
                    self.server_cfg.port,
                    &self.event_tx,
                    &self.shutdown,
                    &self.config.ui_lang,
                ) {
                    self.selected_model = Some(model);
                    return true;
                }
                self.stop_llama_server();
            }
            Err(_) => return false,
        }

        false
    }

    fn stop_llama_server(&mut self) {
        if !is_local_llama_host(&self.server_cfg.host) {
            let _ = self.event_tx.send(BackendEvent::ProcessStatus(
                ProcessType::InferenceEngine,
                false,
            ));
            return;
        }

        let port = self.server_cfg.port;
        if let Some(mut proc) = self.llama_process.take() {
            proc.stop();
        }
        if is_port_open(port) {
            let _ = self.terminate_stray_llama_server();
            let _ = self.wait_for_port_closed(port, Duration::from_secs(5));
        }
        let _ = self.event_tx.send(BackendEvent::ProcessStatus(
            ProcessType::InferenceEngine, false,
        ));
    }

    fn start_translation_server(&mut self) -> bool {
        if self.server_handle.is_some() { return true; }

        let dictionary = self.dictionary.clone();
        let processor = self.current_processor.clone();
        let src_lang = self.config.src_lang.clone();
        let tgt_lang = self.config.tgt_lang.clone();
        let custom_lang_code = self.config.custom_lang_code.clone();
        let custom_lang_name = self.config.custom_lang_name.clone();
        let prompt_template = self.config.prompt_template.clone();
        let enable_model_wrap = self.config.enable_model_wrap;
        let model_wrap_min_chars = self.config.model_wrap_min_chars;
        let model_wrap_min_tail_chars = self.config.model_wrap_min_tail_chars;
        let enable_model_symbol_cleanup = self.config.enable_model_symbol_cleanup;
        let llm_client = self.llm_client.clone();
        let server_event_tx = self.server_event_tx.clone();
        let host = self.config.server_host.clone();
        let port = self.server_port;
        let t_cache = self.t_cache.clone();
        let n_cache = self.n_cache.clone();
        let input_replay = self.input_replay.clone();
        let llm_slots = self.llm_slots.max(1);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();

        let translation_settings = TranslationSettings {
            enable_model_wrap,
            model_wrap_min_chars: model_wrap_min_chars as usize,
            model_wrap_min_tail_chars: model_wrap_min_tail_chars as usize,
            enable_model_symbol_cleanup,
        };

        let handle = self.server_runtime.spawn(
            server::run_translation_server(
                host.clone(), port, dictionary, processor,
                src_lang, tgt_lang, custom_lang_code, custom_lang_name,
                prompt_template,
                translation_settings,
                llm_client, server_event_tx, startup_tx, shutdown_rx,
                t_cache, n_cache, input_replay, llm_slots,
            )
        );

        match self.server_runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(15), startup_rx).await
        }) {
            Ok(Ok(Ok(()))) => {
                self.server_shutdown_tx = Some(shutdown_tx);
                self.server_handle = Some(handle);
                let _ = self.event_tx.send(BackendEvent::ProcessStatus(
                    ProcessType::Tenuki, true,
                ));

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
            Ok(Ok(Err(_))) | Ok(Err(_)) => {
                self.server_runtime.block_on(async { let _ = handle.await; });
                false
            }
            Err(_) => {
                handle.abort();
                self.server_runtime.block_on(async {
                    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
                });
                false
            }
        }
    }

    fn stop_translation_server(&mut self) {
        // 1. シャットダウンシグナル送信（サーバーが新規リクエストを受け付けなくなる）
        if let Some(tx) = self.server_shutdown_tx.take() {
            let _ = tx.send(());
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
        let _ = self.event_tx.send(BackendEvent::ProcessStatus(
            ProcessType::Tenuki, false,
        ));
        
        // 4. キャッシュをリセット（次回起動に備えて）
        self.t_cache = Arc::new(TranslationCache::default());
        self.n_cache = Arc::new(NewEntriesCache::default());

        // ポート解放を少し待つ
        thread::sleep(Duration::from_millis(200));
        
        backend_info!(self.event_tx, "Translation server stopped");
    }

    fn resolve_model(&self) -> Option<PathBuf> {
        if let Some(ref m) = self.selected_model {
            if m.exists() { return Some(m.clone()); }
        }
        self.find_gguf_model()
    }

    fn find_gguf_model(&self) -> Option<PathBuf> {
        self.available_gguf_models().into_iter().next()
    }

    fn available_gguf_models(&self) -> Vec<PathBuf> {
        let models_dir = self.base_dir.join("models");
        let mut models = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "gguf").unwrap_or(false) {
                    models.push(path);
                }
            }
        }

        models.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        models
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
