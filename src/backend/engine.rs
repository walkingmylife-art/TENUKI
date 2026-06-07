// src/backend/engine.rs
//
// 推論エンジン（llama-server）のライフサイクル管理。
// ProcessManager から分離された独立した責務モジュール。

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::process::LlamaProcess;
use crate::launcher::app_config::ServerConfig;
use crate::messages::{BackendEvent, LogLevel, LogSource, ProcessType};

// ============================================================
// EngineWaitKind / EngineWaitPolicy
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineWaitKind {
    NormalStartup,
    ModelSwitch,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EngineWaitPolicy {
    pub max_attempts: u32,
    pub log_interval_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl EngineWaitKind {
    pub(crate) fn policy(self) -> EngineWaitPolicy {
        match self {
            Self::NormalStartup => EngineWaitPolicy {
                max_attempts: 24,
                log_interval_attempts: 2,
                initial_backoff: Duration::from_millis(500),
                max_backoff: Duration::from_secs(3),
            },
            Self::ModelSwitch => EngineWaitPolicy {
                max_attempts: 12,
                log_interval_attempts: 1,
                initial_backoff: Duration::from_millis(500),
                max_backoff: Duration::from_secs(3),
            },
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NormalStartup => "normal startup",
            Self::ModelSwitch => "model switch",
        }
    }
}

// ============================================================
// ネットワークヘルパー
// ============================================================

pub(crate) fn is_port_open(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

pub(crate) fn is_local_llama_host(host: &str) -> bool {
    matches!(host.trim(), "127.0.0.1" | "localhost" | "0.0.0.0" | "")
}

pub(crate) fn llama_connect_host(host: &str) -> &str {
    match host.trim() {
        "" | "0.0.0.0" | "localhost" => "127.0.0.1",
        other => other,
    }
}

pub(crate) fn llama_base_url(host: &str, port: u16) -> String {
    format!("http://{}:{}", llama_connect_host(host), port)
}

pub fn get_local_ip() -> String {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0");
    match socket {
        Ok(s) => {
            let _ = s.connect("8.8.8.8:80");
            s.local_addr()
                .map(|addr| addr.ip().to_string())
                .unwrap_or_else(|_| "127.0.0.1".to_string())
        }
        Err(_) => "127.0.0.1".to_string(),
    }
}

pub(crate) fn wait_for_port_closed(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if !is_port_open(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !is_port_open(port)
}

// ============================================================
// llama-server 探索・通知
// ============================================================

fn find_llama_exe(base_dir: &Path) -> Option<PathBuf> {
    let install_root = crate::launcher::resolve_install_root();
    let launcher_config_path = install_root.join("launcher_config.toml");
    let config = crate::launcher::app_config::AppConfig::load(&launcher_config_path).ok()?;
    let backend = config.backend;

    let backend_dir = base_dir.join("runtime").join(&backend);
    if !crate::launcher::runtime_downloader::runtime_is_complete(&backend_dir, &backend) {
        return None;
    }
    crate::launcher::runtime_downloader::find_llama_server_exe(&backend_dir)
}

fn emit_engine_wait_notice(
    event_tx: &mpsc::Sender<BackendEvent>,
    ui_lang: &str,
    kind: EngineWaitKind,
    attempt: u32,
    status_en: &str,
    status_ja: &str,
) {
    let status = if ui_lang == "en" {
        status_en
    } else {
        status_ja
    };
    let msg = format!("llama-server {}: {} ({})", kind.label(), status, attempt);
    let _ = event_tx.send(BackendEvent::Log(
        LogSource::Tenuki,
        msg,
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));
}

#[cfg(target_os = "windows")]
fn terminate_stray_llama_server() -> bool {
    use std::os::windows::process::CommandExt;
    match Command::new("taskkill")
        .args(["/IM", "llama-server.exe", "/F"])
        .creation_flags(0x08000000)
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[cfg(not(target_os = "windows"))]
fn terminate_stray_llama_server() -> bool {
    false
}

// ============================================================
// EngineManager
// ============================================================

pub(crate) struct EngineManager {
    pub server_cfg: ServerConfig,
    llama_process: Option<LlamaProcess>,
    event_tx: mpsc::Sender<BackendEvent>,
    shutdown: Arc<AtomicBool>,
    pub selected_model: Option<PathBuf>,
    base_dir: PathBuf,
    ctx_size: u32,
    llm_slots: usize,
    ui_lang: String,
}

impl EngineManager {
    pub(crate) fn new(
        server_cfg: ServerConfig,
        base_dir: PathBuf,
        event_tx: mpsc::Sender<BackendEvent>,
        selected_model: Option<PathBuf>,
        shutdown: Arc<AtomicBool>,
        ui_lang: String,
    ) -> Self {
        let ctx_size = server_cfg.ctx_size;
        let llm_slots = server_cfg.parallel_slots.max(1) as usize;

        Self {
            server_cfg,
            llama_process: None,
            event_tx,
            shutdown,
            selected_model,
            base_dir,
            ctx_size,
            llm_slots,
            ui_lang,
        }
    }

    pub(crate) fn set_ui_lang(&mut self, ui_lang: String) {
        self.ui_lang = ui_lang;
    }

    pub(crate) fn set_selected_model(&mut self, model: Option<PathBuf>) {
        self.selected_model = model;
    }

    pub(crate) fn is_engine_running(&self) -> bool {
        self.llama_process.is_some()
    }

    pub(crate) fn ctx_size(&self) -> u32 {
        self.ctx_size
    }

    pub(crate) fn llm_slots(&self) -> usize {
        self.llm_slots
    }

    pub(crate) fn llama_base_url(&self) -> String {
        llama_base_url(&self.server_cfg.host, self.server_cfg.port)
    }

    pub(crate) fn has_live_llama_process(&mut self) -> bool {
        if !is_local_llama_host(&self.server_cfg.host) {
            return self.check_remote_llama_endpoint();
        }

        let is_alive = self
            .llama_process
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

    pub(crate) fn resolve_model(&self) -> Option<PathBuf> {
        let m = self.selected_model.as_ref()?;
        if m.exists() {
            Some(m.clone())
        } else {
            None
        }
    }

    pub(crate) fn wait_for_llama_server(&mut self, kind: EngineWaitKind) -> bool {
        let policy = kind.policy();
        let connect_host = llama_connect_host(&self.server_cfg.host);
        let addr: SocketAddr = format!("{}:{}", connect_host, self.server_cfg.port)
            .parse()
            .unwrap();
        let health_url = format!(
            "{}/health",
            llama_base_url(&self.server_cfg.host, self.server_cfg.port)
        );
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(500))
            .timeout_read(Duration::from_secs(5))
            .build();

        let mut backoff = policy.initial_backoff;
        emit_engine_wait_notice(
            &self.event_tx,
            &self.ui_lang,
            kind,
            0,
            "starting",
            "開始中",
        );

        for attempt in 0..policy.max_attempts {
            if self.shutdown.load(Ordering::Relaxed) {
                return false;
            }

            let display_attempt = attempt + 1;
            if !self.has_live_llama_process() {
                emit_engine_wait_notice(
                    &self.event_tx,
                    &self.ui_lang,
                    kind,
                    display_attempt,
                    "process exited",
                    "プロセスが終了しました",
                );
                return false;
            }

            if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err() {
                if attempt % policy.log_interval_attempts == 0 {
                    emit_engine_wait_notice(
                        &self.event_tx,
                        &self.ui_lang,
                        kind,
                        display_attempt,
                        "waiting for port",
                        "ポート待機中",
                    );
                }
                std::thread::sleep(backoff);
                backoff = std::cmp::min(backoff * 2, policy.max_backoff);
                continue;
            }

            match agent.get(&health_url).call() {
                Ok(response) if response.status() == 200 => {
                    let is_ok = response
                        .into_string()
                        .map(|s| s.contains("\"ok\""))
                        .unwrap_or(true);
                    if is_ok {
                        return true;
                    }
                    emit_engine_wait_notice(
                        &self.event_tx,
                        &self.ui_lang,
                        kind,
                        display_attempt,
                        "loading model",
                        "モデルロード中",
                    );
                }
                Ok(_) => {
                    emit_engine_wait_notice(
                        &self.event_tx,
                        &self.ui_lang,
                        kind,
                        display_attempt,
                        "loading model",
                        "モデルロード中",
                    );
                }
                Err(_) => {
                    if attempt % policy.log_interval_attempts == 0 {
                        emit_engine_wait_notice(
                            &self.event_tx,
                            &self.ui_lang,
                            kind,
                            display_attempt,
                            "waiting for health",
                            "health待機中",
                        );
                    }
                }
            }

            std::thread::sleep(backoff);
            backoff = std::cmp::min(backoff * 2, policy.max_backoff);
        }

        emit_engine_wait_notice(
            &self.event_tx,
            &self.ui_lang,
            kind,
            policy.max_attempts,
            "timeout",
            "タイムアウト",
        );
        false
    }

    pub(crate) fn start_llama_server(&mut self, wait_kind: EngineWaitKind) -> bool {
        if self.has_live_llama_process() {
            return true;
        }

        if !is_local_llama_host(&self.server_cfg.host) {
            return self.check_remote_llama_endpoint();
        }

        let model = match self.resolve_model() {
            Some(m) => m,
            None => {
                let msg = if self.ui_lang == "en" {
                    "No startup model could be resolved. Select a model from the list."
                        .to_string()
                } else {
                    "models/ ディレクトリにモデルファイルが見つかりません".to_string()
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                return false;
            }
        };

        let exe = match find_llama_exe(&self.base_dir) {
            Some(e) => e,
            None => {
                let msg = if self.ui_lang == "en" {
                    "llama-server executable not found".to_string()
                } else {
                    "llama-server 実行ファイルが見つかりません".to_string()
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                return false;
            }
        };

        if is_port_open(self.server_cfg.port) {
            if !terminate_stray_llama_server()
                || !wait_for_port_closed(self.server_cfg.port, Duration::from_secs(5))
            {
                return false;
            }
        }

        match LlamaProcess::start(
            &exe,
            &model,
            self.server_cfg.ngl,
            self.ctx_size,
            self.llm_slots.max(1) as u32,
            self.server_cfg.port,
            &self.server_cfg.extra_args,
            self.event_tx.clone(),
        ) {
            Ok(proc) => {
                self.llama_process = Some(proc);
                let _ = self.event_tx.send(BackendEvent::ProcessStatus(
                    ProcessType::InferenceEngine,
                    true,
                ));
                if self.wait_for_llama_server(wait_kind) {
                    self.selected_model = Some(model);
                    return true;
                }
                self.stop_llama_server();
            }
            Err(e) => {
                let msg = if self.ui_lang == "en" {
                    format!("Failed to launch inference engine: {e}")
                } else {
                    format!("推論エンジンの起動に失敗しました: {e}")
                };
                let _ = self.event_tx.send(BackendEvent::Log(
                    LogSource::Tenuki,
                    msg,
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                return false;
            }
        }

        false
    }

    pub(crate) fn stop_llama_server(&mut self) {
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
            let _ = terminate_stray_llama_server();
            let _ = wait_for_port_closed(port, Duration::from_secs(5));
        }
        let _ = self.event_tx.send(BackendEvent::ProcessStatus(
            ProcessType::InferenceEngine,
            false,
        ));
    }

    /// Drop 時のクリーンアップ用。stop_llama_server の別名。
    pub(crate) fn stop(&mut self) {
        self.stop_llama_server();
    }
}
