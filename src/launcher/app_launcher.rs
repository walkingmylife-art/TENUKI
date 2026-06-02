// src/launcher/app_launcher.rs
//
// 役割：権威設定（AppConfig）に基づきモデルとバックエンドを準備する。
// - setup で通った backend を launcher_config.toml の backend フィールドに反映して保存。
// - 通常起動の判定（check_ready）は launcher_config.toml の backend のみ参照。
// - 検証（test_backend_exe）は必ず 127.0.0.1 + 動的ポートを使用する。
// - 起動引数の組み立ては build_server_command() に一元化し、本番起動とテスト起動で完全に一致させる。
// - LaunchProgress::Progress は「現在のフェーズ（モデルDL/ランタイムDL）単体の進捗（0.0～1.0）」であり、
//   UI 側で全体進捗に換算することを前提とする。

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// 診断メッセージを SubStatus で流し、launcher_debug.log にも追記する。
fn diag(progress_tx: &Sender<LaunchProgress>, base_dir: &Path, msg: &str) {
    progress_tx
        .send(LaunchProgress::SubStatus(msg.to_string()))
        .ok();
    let log_path = base_dir.join("launcher_debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

use super::app_config::AppConfig;
use super::backend_detector::BackendDetector;
use super::progress::LaunchProgress;
use super::runtime_downloader::{find_llama_server_exe, RuntimeDownloader};
use crate::backend::process::build_llama_command;

/// モデルファイルが完成しているか判定する共通関数。
///
/// 完成条件:
/// - ファイルが存在する
/// - sidecar (.sidecar.json) が存在しない
/// - expected_size > 0 のとき、ファイルサイズが一致する
/// - expected_size == 0 のときは未完成扱い（サイズ検証不能）
fn model_is_complete(model_path: &Path, expected_size: u64) -> bool {
    if expected_size == 0 {
        return false;
    }
    let actual = match std::fs::metadata(model_path) {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    if actual != expected_size {
        return false;
    }
    // サイズ一致 = 完成。stale sidecar / .part は掃除する。
    cleanup_model_resume_artifacts(model_path);
    true
}

fn cleanup_model_resume_artifacts(model_path: &Path) {
    let sidecar = PathBuf::from(format!("{}.sidecar.json", model_path.display()));
    if sidecar.exists() {
        let _ = std::fs::remove_file(&sidecar);
    }
    let part = model_path.with_extension("part");
    if part.exists() {
        let _ = std::fs::remove_file(&part);
    }
}

fn push_unique_candidate(out: &mut Vec<BackendCandidate>, candidate: BackendCandidate) {
    if out.iter().all(|existing| existing.name != candidate.name) {
        out.push(candidate);
    }
}

/// `check_ready` が false を返す理由の構造化表現。
#[derive(Debug, Clone)]
pub enum CheckReadyReason {
    ConfigLoadFail(String),
    RuntimeIncomplete {
        backend: String,
    },
    NoModelsAvailable,
    /// Known model が missing → launcher で download 可。
    ModelMissing {
        filename: String,
    },
    /// Known model のサイズ不一致（ダウンロード不完全など）。
    ModelSizeMismatch {
        filename: String,
        expected: u64,
        actual: u64,
    },
    /// Local model が missing → download 不可。ユーザーに再配置/再選択を促す。
    LocalModelMissing {
        filename: String,
    },
    /// Local model のサイズが authority と一致しない。
    /// ファイル自体は存在するが中身が差し替わっている可能性がある。
    LocalModelChanged {
        filename: String,
        expected: u64,
        actual: u64,
    },
    /// Authority モデル不在かつ代替候補が複数あり、起動モデルを一意に決定できない。
    StartupModelUnresolved,
}

impl std::fmt::Display for CheckReadyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigLoadFail(e) => write!(f, "launcher_config.toml load failed: {}", e),
            Self::RuntimeIncomplete { backend } => {
                write!(f, "runtime/{} is incomplete", backend)
            }
            Self::NoModelsAvailable => write!(f, "no usable model files found"),
            Self::ModelMissing { filename } => write!(f, "model '{}' not found", filename),
            Self::ModelSizeMismatch {
                filename,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "model '{}' size mismatch: expected {} got {}",
                    filename, expected, actual
                )
            }
            Self::LocalModelMissing { filename } => {
                write!(
                    f,
                    "local model '{}' not found (download not available)",
                    filename
                )
            }
            Self::LocalModelChanged {
                filename,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "local model '{}' changed on disk: committed size {} but found {}. \
                     Re-select to update authority.",
                    filename, expected, actual
                )
            }
            Self::StartupModelUnresolved => write!(
                f,
                "起動モデルを決定できません。usable なモデルが複数あり一意に選択できません。再選択が必要です。"
            ),
        }
    }
}

/// runtime とモデルが揃っているか確認し、不足の場合は理由を返す。
/// - launcher_config.toml は install_root から読む（権威位置）
/// - models/, runtime/ は base_dir から読む
pub fn check_ready_detail(base_dir: &std::path::Path) -> Result<(), CheckReadyReason> {
    use crate::launcher::runtime_downloader::runtime_is_complete;

    let install_root = super::resolve_install_root();
    let config_path = install_root.join("launcher_config.toml");
    let config = match super::app_config::AppConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            let reason = CheckReadyReason::ConfigLoadFail(e.to_string());
            diag_file(base_dir, &format!("[check_ready] {}", reason));
            return Err(reason);
        }
    };

    let backend = &config.backend;
    let runtime_dir = base_dir.join("runtime").join(backend);
    let rt_ok = runtime_is_complete(&runtime_dir, backend);
    diag_file(
        base_dir,
        &format!(
            "[check_ready] runtime_is_complete backend={} rt_ok={}",
            backend, rt_ok
        ),
    );
    if !rt_ok {
        return Err(CheckReadyReason::RuntimeIncomplete {
            backend: backend.clone(),
        });
    }

    let available_models = crate::backend::find_available_models(&base_dir.to_path_buf())
        .into_iter()
        .filter(|candidate| candidate.size > 0)
        .collect::<Vec<_>>();
    if available_models.is_empty() {
        return Err(CheckReadyReason::NoModelsAvailable);
    }

    let filename = config.model.filename().to_string();
    let expected_size = config.model.expected_size();
    let is_known = config.model.is_known();
    let model_path = base_dir.join("models").join(&filename);

    let actual_size = std::fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0);
    diag_file(
        base_dir,
        &format!(
            "[check_ready] model filename={} kind={} expected={} actual={}",
            filename,
            if is_known { "Known" } else { "Local" },
            expected_size,
            actual_size
        ),
    );

    if actual_size == expected_size && expected_size > 0 {
        cleanup_model_resume_artifacts(&model_path);
        diag_file(base_dir, "[check_ready] true (authority model matched)");
        return Ok(());
    }

    if crate::backend::resolve_startup_model(&config, &available_models).is_some() {
        diag_file(
            base_dir,
            &format!(
                "[check_ready] authority model unavailable, but startup model resolved from {} candidate(s)",
                available_models.len()
            ),
        );
        return Ok(());
    }
    if available_models.len() > 1 {
        diag_file(
            base_dir,
            &format!(
                "[check_ready] {} candidates exist but startup model unresolvable",
                available_models.len()
            ),
        );
        return Err(CheckReadyReason::StartupModelUnresolved);
    }

    if actual_size == 0 {
        return if is_known {
            Err(CheckReadyReason::ModelMissing { filename })
        } else {
            Err(CheckReadyReason::LocalModelMissing { filename })
        };
    }
    if actual_size != expected_size || expected_size == 0 {
        return if is_known {
            Err(CheckReadyReason::ModelSizeMismatch {
                filename,
                expected: expected_size,
                actual: actual_size,
            })
        } else {
            Err(CheckReadyReason::LocalModelChanged {
                filename,
                expected: expected_size,
                actual: actual_size,
            })
        };
    }

    // サイズ一致 = 完成。stale sidecar / .part は掃除する。
    cleanup_model_resume_artifacts(&model_path);
    diag_file(base_dir, "[check_ready] → true");
    Ok(())
}

// re-export ModelConfig for use in app_launcher
use super::app_config::ModelConfig;

/// runtime とモデルが揃っているか確認する（bool 版）。
/// main.rs でモードを決定するために使用される。
pub fn check_ready(base_dir: &std::path::Path) -> bool {
    check_ready_detail(base_dir).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendCandidateReason {
    AuthorityConfig,
    GpuDetection,
    InstalledRuntime,
}

impl BackendCandidateReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityConfig => "authority_config",
            Self::GpuDetection => "gpu_detection",
            Self::InstalledRuntime => "installed_runtime",
        }
    }
}

#[derive(Debug, Clone)]
struct BackendCandidate {
    name: String,
    reason: BackendCandidateReason,
}

impl BackendCandidate {
    fn new(name: String, reason: BackendCandidateReason) -> Self {
        Self { name, reason }
    }
}

#[derive(Debug, Clone)]
struct BackendPlan {
    gpu_detected: Vec<BackendCandidate>,
    installed: Vec<BackendCandidate>,
    authority_download: Option<BackendCandidate>,
    skipped_authority_download: Option<BackendCandidate>,
    fallback_downloads: Vec<BackendCandidate>,
}

#[derive(Debug, Clone)]
struct VerifiedBackendCandidate {
    candidate: BackendCandidate,
    exe_path: PathBuf,
}

pub struct AppLauncher {
    base_dir: PathBuf,
    config: AppConfig,
    http_client: Client,
    ui_lang: String,
}

impl AppLauncher {
    pub fn new(base_dir: PathBuf, ui_lang: String) -> Result<Self> {
        // launcher_config.toml は install_root（権威位置）から読む
        let install_root = super::resolve_install_root();
        let config_path = install_root.join("launcher_config.toml");
        // missing = setup 前の未生成。default をメモリ上の出発点にするだけで保存しない。
        // 保存は run() の Save 段階でのみ行う。
        let config = if config_path.exists() {
            AppConfig::load(&config_path)?
        } else {
            AppConfig::default()
        };

        let http_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            base_dir,
            config,
            http_client,
            ui_lang,
        })
    }

    fn t(&self, en: &str, ja: &str) -> String {
        if self.ui_lang == "en" {
            en.to_string()
        } else {
            ja.to_string()
        }
    }

    pub fn run(
        &mut self,
        progress_tx: Sender<LaunchProgress>,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        use super::progress::LauncherStage;

        macro_rules! check_cancel {
            () => {
                if cancel_flag.load(Ordering::Relaxed) {
                    progress_tx.send(LaunchProgress::Cancelled).ok();
                    anyhow::bail!("Cancelled by user");
                }
            };
        }

        // Stage: Directories
        progress_tx
            .send(LaunchProgress::Stage(LauncherStage::Directories))
            .ok();
        progress_tx
            .send(LaunchProgress::Status(
                self.t("Preparing directories...", "ディレクトリを確認中..."),
            ))
            .ok();
        self.create_directories()?;
        check_cancel!();

        // Stage: Gpu
        progress_tx
            .send(LaunchProgress::Stage(LauncherStage::Gpu))
            .ok();
        progress_tx
            .send(LaunchProgress::Status(
                self.t("Detecting GPU...", "GPUを検出中..."),
            ))
            .ok();
        let backend_plan = self.build_backend_plan();
        diag(
            &progress_tx,
            &self.base_dir,
            &format!(
                "[run] backend plan gpu_detected={:?} installed={:?} authority_download={:?} skipped_authority_download={:?} fallback_downloads={:?}",
                backend_plan.gpu_detected,
                backend_plan.installed,
                backend_plan.authority_download,
                backend_plan.skipped_authority_download,
                backend_plan.fallback_downloads
            ),
        );
        if let Some(candidate) = backend_plan.skipped_authority_download.as_ref() {
            diag(
                &progress_tx,
                &self.base_dir,
                &format!(
                    "[run] skip authority download backend={} reason=authority_backend_not_in_current_gpu_evidence gpu_detected={:?}",
                    candidate.name,
                    backend_plan
                        .gpu_detected
                        .iter()
                        .map(|gpu| gpu.name.as_str())
                        .collect::<Vec<_>>()
                ),
            );
        }
        check_cancel!();

        // Stage: Model
        progress_tx
            .send(LaunchProgress::Stage(LauncherStage::Model))
            .ok();
        progress_tx
            .send(LaunchProgress::Status(
                self.t("Checking model...", "モデルを確認中..."),
            ))
            .ok();
        let model_path = self.ensure_model(&progress_tx, &cancel_flag)?;
        check_cancel!();

        // Stage: Runtime
        // Pass 1: 既インストール runtime を順に試す（ダウンロードなし）
        // Pass 2: 既存で動くものがなければ config.backend のみダウンロード
        progress_tx
            .send(LaunchProgress::Stage(LauncherStage::Runtime))
            .ok();
        progress_tx
            .send(LaunchProgress::Status(
                self.t("Selecting backend...", "バックエンドを選定中..."),
            ))
            .ok();

        let mut working_backend: Option<VerifiedBackendCandidate> = None;
        for candidate in backend_plan.installed.iter() {
            check_cancel!();
            let name = &candidate.name;
            progress_tx
                .send(LaunchProgress::SubStatus(self.t(
                    &format!("Trying {}...", candidate.name),
                    &format!("{} を試行中...", name),
                )))
                .ok();
            match self.try_existing_backend(candidate, &model_path, &progress_tx) {
                Ok(verified) => {
                    working_backend = Some(verified);
                    break;
                }
                Err(e) => {
                    diag(
                        &progress_tx,
                        &self.base_dir,
                        &format!(
                            "[run] existing runtime rejected backend={} reason={} error={}",
                            name,
                            candidate.reason.as_str(),
                            e
                        ),
                    );
                }
            }
        }

        if working_backend.is_none() {
            if let Some(candidate) = backend_plan.authority_download.as_ref() {
                let name = &candidate.name;
                check_cancel!();
                progress_tx
                    .send(LaunchProgress::SubStatus(self.t(
                        &format!("Downloading {} runtime...", name),
                        &format!("{} ランタイムをダウンロード中...", name),
                    )))
                    .ok();
                match self.try_backend(candidate, &model_path, &progress_tx, &cancel_flag) {
                    Ok(verified) => {
                        working_backend = Some(verified);
                    }
                    Err(e) => {
                        diag(
                            &progress_tx,
                            &self.base_dir,
                            &format!(
                                "[run] authority download rejected backend={} reason={} error={}",
                                name,
                                candidate.reason.as_str(),
                                e
                            ),
                        );
                    }
                }
            }
        }

        if working_backend.is_none() {
            for candidate in backend_plan.fallback_downloads.iter() {
                let name = &candidate.name;
                check_cancel!();
                progress_tx
                    .send(LaunchProgress::SubStatus(self.t(
                        &format!("Downloading {} runtime...", name),
                        &format!("{} ランタイムをダウンロード中...", name),
                    )))
                    .ok();
                match self.try_backend(candidate, &model_path, &progress_tx, &cancel_flag) {
                    Ok(verified) => {
                        working_backend = Some(verified);
                        break;
                    }
                    Err(e) => {
                        diag(
                            &progress_tx,
                            &self.base_dir,
                            &format!(
                                "[run] fallback download rejected backend={} reason={} error={}",
                                name,
                                candidate.reason.as_str(),
                                e
                            ),
                        );
                    }
                }
            }
        }

        let verified_backend =
            working_backend.ok_or_else(|| anyhow!("No working backend found"))?;
        let backend_name = verified_backend.candidate.name.clone();
        let exe_path = verified_backend.exe_path;
        check_cancel!();

        // Stage: Verify (test_backend_exe already ran inside try_backend)
        progress_tx
            .send(LaunchProgress::Stage(LauncherStage::Verify))
            .ok();
        progress_tx
            .send(LaunchProgress::Status(self.t("Verified.", "検証完了.")))
            .ok();
        check_cancel!();

        // Stage: Save — setup で通った backend を authority (launcher_config.toml) に反映
        progress_tx
            .send(LaunchProgress::Stage(LauncherStage::Save))
            .ok();
        progress_tx
            .send(LaunchProgress::Status(
                self.t("Saving config...", "設定を保存中..."),
            ))
            .ok();
        self.config.backend = backend_name.clone();
        let install_root = super::resolve_install_root();
        let config_path = install_root.join("launcher_config.toml");
        self.config.save(&config_path)?;
        self.seed_profiles()?;

        // Post-save self-check: 保存した authority で startup 条件が満たされているか検証する。
        // これが false なら、次回起動で check_ready() が落ちる前にここで失敗させる。
        let self_check = check_ready(&self.base_dir);
        diag(
            &progress_tx,
            &self.base_dir,
            &format!(
                "[run] post-save self_check={} backend={} model={} exe={}",
                self_check,
                backend_name,
                self.config.model.filename(),
                exe_path.display(),
            ),
        );
        if !self_check {
            anyhow::bail!(
                "Setup completed but startup readiness check failed \
                 (backend={} model={}). \
                 Runtime or model did not satisfy authority exact check after save.",
                backend_name,
                self.config.model.filename()
            );
        }

        progress_tx.send(LaunchProgress::Complete).ok();
        Ok(())
    }

    fn create_directories(&self) -> Result<()> {
        let dirs = ["runtime", "models", "profiles", "logs", "tmp"];
        for d in dirs {
            std::fs::create_dir_all(self.base_dir.join(d))?;
        }
        Ok(())
    }

    /// Known model: complete なら返す、incomplete なら download。
    /// Local model: complete なら返す、missing/incomplete なら bail。
    fn ensure_model(
        &self,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<PathBuf> {
        let expected = self.config.model.expected_size();
        let filename = self.config.model.filename().to_string();
        if expected == 0 {
            diag(
                progress_tx,
                &self.base_dir,
                "[ensure_model] ERROR: expected_size == 0 in launcher_config.toml",
            );
            anyhow::bail!(
                "model.expected_size is 0 in launcher_config.toml. \
                 Set it to the correct file size in bytes."
            );
        }

        let model_dir = self.base_dir.join("models");
        let model_path = model_dir.join(&filename);

        let complete = model_is_complete(&model_path, expected);
        diag(
            progress_tx,
            &self.base_dir,
            &format!(
                "[ensure_model] filename={} kind={} exists={} expected_size={} complete={}",
                filename,
                if self.config.model.is_known() {
                    "Known"
                } else {
                    "Local"
                },
                model_path.exists(),
                expected,
                complete
            ),
        );
        if complete {
            diag(
                progress_tx,
                &self.base_dir,
                "[ensure_model] → reuse existing model",
            );
            return Ok(model_path);
        }

        // authority model が不完全でも、別の完成済み .gguf があればそちらを使う。
        // ダウンロードを起動する前にチェックする。
        let alternative = std::fs::read_dir(&model_dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let is_gguf = path.extension().map(|e| e == "gguf").unwrap_or(false);
                let is_other = path.file_name().and_then(|n| n.to_str()) != Some(filename.as_str());
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if is_gguf && is_other && size > 0 {
                    Some(path)
                } else {
                    None
                }
            })
            .next();
        if let Some(alt_path) = alternative {
            diag(
                progress_tx,
                &self.base_dir,
                &format!(
                    "[ensure_model] authority incomplete, using alternative: {}",
                    alt_path.display()
                ),
            );
            return Ok(alt_path);
        }

        // Local model: download 不可。ユーザーに再配置/再選択を促す。
        if let ModelConfig::Local { .. } = &self.config.model {
            diag(
                progress_tx,
                &self.base_dir,
                &format!(
                    "[ensure_model] Local model missing, cannot download: {}",
                    filename
                ),
            );
            anyhow::bail!(
                "Local model '{}' not found in models/. \
                 Restore the file or select another model.",
                filename
            );
        }

        // Known model: download パス
        let ModelConfig::Known { urls, .. } = &self.config.model else {
            unreachable!()
        };

        // 不完全なファイルが残っている場合は削除して再取得
        // (sidecar ありの場合は download_model 側が再開する)
        if model_path.exists() {
            let sidecar = PathBuf::from(format!("{}.sidecar.json", model_path.display()));
            if !sidecar.exists() {
                diag(
                    progress_tx,
                    &self.base_dir,
                    "[ensure_model] incomplete (no sidecar, bad size) → remove and re-download",
                );
                std::fs::remove_file(&model_path)?;
            }
        }

        progress_tx
            .send(LaunchProgress::Status(
                self.t("Downloading model...", "モデルをダウンロード中..."),
            ))
            .ok();
        progress_tx
            .send(LaunchProgress::SubStatus(self.t(
                &format!("Fetching: {}", filename),
                &format!("取得中: {}", filename),
            )))
            .ok();

        let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
        let _used_url_opt = downloader.download_model(
            &urls.primary,
            urls.fallback.as_deref(),
            &model_path,
            expected,
            |progress, _| {
                // この progress はモデルダウンロード単体の進捗（0.0～1.0）
                progress_tx.send(LaunchProgress::Progress(progress)).ok();
            },
        )?;

        Ok(model_path)
    }

    fn build_backend_plan(&self) -> BackendPlan {
        let gpu_candidates = self.gpu_backend_candidates();
        let installed_runtime_candidates = self.installed_runtime_candidates();
        self.build_backend_plan_from_observations(gpu_candidates, installed_runtime_candidates)
    }

    fn build_backend_plan_from_observations(
        &self,
        gpu_candidates: Vec<BackendCandidate>,
        installed_runtime_candidates: Vec<BackendCandidate>,
    ) -> BackendPlan {
        let authority_download = BackendCandidate::new(
            self.config.backend.clone(),
            BackendCandidateReason::AuthorityConfig,
        );

        let mut installed = Vec::new();
        if self.has_complete_runtime(&authority_download.name) {
            push_unique_candidate(&mut installed, authority_download.clone());
        }
        for candidate in gpu_candidates.iter().cloned() {
            if self.has_complete_runtime(&candidate.name) {
                push_unique_candidate(&mut installed, candidate);
            }
        }
        for candidate in installed_runtime_candidates {
            push_unique_candidate(&mut installed, candidate);
        }

        let authority_matches_current_gpu = gpu_candidates
            .iter()
            .any(|candidate| candidate.name == authority_download.name);

        let mut fallback_downloads = Vec::new();
        for candidate in gpu_candidates.iter().cloned() {
            if candidate.name != authority_download.name {
                push_unique_candidate(&mut fallback_downloads, candidate);
            }
        }

        BackendPlan {
            gpu_detected: gpu_candidates,
            installed,
            authority_download: authority_matches_current_gpu.then_some(authority_download.clone()),
            skipped_authority_download: (!authority_matches_current_gpu)
                .then_some(authority_download),
            fallback_downloads,
        }
    }

    fn gpu_backend_candidates(&self) -> Vec<BackendCandidate> {
        let gpus = BackendDetector::enumerate_gpus().unwrap_or_default();
        BackendDetector::build_backend_candidate_names(&gpus)
            .into_iter()
            .map(|name| BackendCandidate::new(name, BackendCandidateReason::GpuDetection))
            .collect()
    }

    fn installed_runtime_candidates(&self) -> Vec<BackendCandidate> {
        ["cuda", "rocm", "vulkan"]
            .into_iter()
            .filter(|name| self.has_complete_runtime(name))
            .map(|name| {
                BackendCandidate::new(name.to_string(), BackendCandidateReason::InstalledRuntime)
            })
            .collect()
    }

    fn has_complete_runtime(&self, name: &str) -> bool {
        let runtime_dir = self.base_dir.join("runtime").join(name);
        crate::launcher::runtime_downloader::runtime_is_complete(&runtime_dir, name)
    }

    /// 既インストール runtime を検証して (exe, url) を返す。ダウンロードしない。
    fn try_existing_backend(
        &self,
        candidate: &BackendCandidate,
        model_path: &Path,
        progress_tx: &Sender<LaunchProgress>,
    ) -> Result<VerifiedBackendCandidate> {
        let name = &candidate.name;
        let runtime_dir = self.base_dir.join("runtime").join(name);
        if !self.has_complete_runtime(name) {
            anyhow::bail!("runtime/{} not installed", name);
        }
        let exe_path = find_llama_server_exe(&runtime_dir)
            .ok_or_else(|| anyhow!("llama-server not found in {}", runtime_dir.display()))?;
        progress_tx
            .send(LaunchProgress::SubStatus(
                self.t("Verifying runtime...", "ランタイム検証中..."),
            ))
            .ok();
        self.test_backend_exe(&exe_path, model_path)?;
        diag(
            progress_tx,
            &self.base_dir,
            &format!(
                "[try_existing_backend] using {} → {}",
                name,
                exe_path.display()
            ),
        );
        Ok(VerifiedBackendCandidate {
            candidate: candidate.clone(),
            exe_path,
        })
    }

    /// バックエンドを試行し、(実行ファイルパス, 採用する URL) を返す。
    fn try_backend(
        &self,
        candidate: &BackendCandidate,
        model_path: &Path,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<VerifiedBackendCandidate> {
        let name = &candidate.name;
        let runtime_dir = self.base_dir.join("runtime").join(name);

        self.ensure_runtime(name, &runtime_dir, progress_tx, cancel_flag)?;
        let exe_path = find_llama_server_exe(&runtime_dir)
            .ok_or_else(|| anyhow!("llama-server not found in {}", runtime_dir.display()))?;

        progress_tx
            .send(LaunchProgress::SubStatus(
                self.t("Verifying runtime...", "ランタイム検証中..."),
            ))
            .ok();
        self.test_backend_exe(&exe_path, model_path)?;

        diag(
            progress_tx,
            &self.base_dir,
            &format!(
                "[try_backend] verified backend={} reason={} exe={}",
                name,
                candidate.reason.as_str(),
                exe_path.display()
            ),
        );

        Ok(VerifiedBackendCandidate {
            candidate: candidate.clone(),
            exe_path,
        })
    }

    fn ensure_runtime(
        &self,
        name: &str,
        dest_dir: &Path,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<()> {
        let rt_ok = self.has_complete_runtime(name);
        diag(
            progress_tx,
            &self.base_dir,
            &format!("[ensure_runtime] backend={} rt_ok={}", name, rt_ok),
        );
        if rt_ok {
            return Ok(());
        }

        progress_tx
            .send(LaunchProgress::Status(self.t(
                &format!("Downloading {} runtime...", name),
                &format!("{} ランタイムをダウンロード中...", name),
            )))
            .ok();

        let assets = self
            .config
            .runtime_urls
            .for_backend(name)
            .ok_or_else(|| anyhow!("Unsupported backend: {}", name))?;

        let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
        let _used_url = downloader.download_backend(
            name,
            &assets.primary,
            &assets.extra_assets,
            assets.fallback.as_deref(),
            dest_dir,
            |progress, _| {
                // ランタイムダウンロード単体の進捗（0.0～1.0）
                progress_tx.send(LaunchProgress::Progress(progress)).ok();
            },
        )?;
        Ok(())
    }

    /// バックエンド実行ファイルの検証。
    /// 必ず動的ポートを使用する。build_llama_command を使い本番起動と引数を完全一致させる。
    fn test_backend_exe(&self, exe_path: &Path, model_path: &Path) -> Result<()> {
        let test_port = find_free_port()?;
        let s = &self.config.server;
        let mut cmd = build_llama_command(
            exe_path,
            model_path,
            test_port,
            s.ngl,
            s.ctx_size,
            s.parallel_slots,
            &s.extra_args,
        );

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Failed to spawn test backend: {}", exe_path.display()))?;

        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || for _ in BufReader::new(stderr).lines() {});
        }

        let result = self.wait_for_healthy_process(&mut child, test_port, Duration::from_secs(120));
        let _ = child.kill();
        let _ = child.wait();
        result
    }

    fn wait_for_healthy_process(
        &self,
        child: &mut Child,
        port: u16,
        timeout: Duration,
    ) -> Result<()> {
        let start = Instant::now();
        let interval = Duration::from_millis(500);
        loop {
            match child.try_wait()? {
                Some(status) => anyhow::bail!("llama-server exited early: {}", status),
                None => {
                    if self.check_health(port).is_ok() {
                        return Ok(());
                    }
                    if start.elapsed() > timeout {
                        anyhow::bail!("Health check timeout");
                    }
                }
            }
            thread::sleep(interval);
        }
    }

    fn check_health(&self, port: u16) -> Result<()> {
        let url = format!("http://127.0.0.1:{}/health", port);
        let resp = self.http_client.get(&url).send()?;
        if resp.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("Health endpoint returned {}", resp.status())
        }
    }

    /// profiles/game.toml と profiles/normal.toml を shipped default で生成する。
    /// game.toml が旧 shipped bad prompt のままの場合は公式文面に修復する。
    fn seed_profiles(&self) -> Result<()> {
        use super::translation_profile::TranslationProfile;
        let profiles_dir = self.base_dir.join("profiles");
        std::fs::create_dir_all(&profiles_dir)?;

        const GAME_BAD_PROMPT: &str =
            "Translate the following segment into {target}, preserving all special symbols and tags exactly as they appear. Do not add any explanations.";

        let game_path = profiles_dir.join("game.toml");
        if !game_path.exists() {
            TranslationProfile::game_default().save(&game_path)?;
        } else if let Ok(content) = std::fs::read_to_string(&game_path) {
            if let Ok(existing) = toml::from_str::<TranslationProfile>(&content) {
                if existing.prompt_template == GAME_BAD_PROMPT {
                    TranslationProfile::game_default().save(&game_path)?;
                }
            }
        }

        let normal_path = profiles_dir.join("normal.toml");
        if !normal_path.exists() {
            TranslationProfile::normal_default().save(&normal_path)?;
        }

        Ok(())
    }
}

/// check_ready 用：progress_tx なしでファイルだけに書く。
fn diag_file(base_dir: &Path, msg: &str) {
    let log_path = base_dir.join("launcher_debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

/// 空きポートを動的に取得する（127.0.0.1 固定）
fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::app_config::AppConfig;
    use crate::launcher::runtime_downloader::runtime_is_complete;
    use std::fs;
    use std::time::Duration;

    fn temp_base(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tenuki_launcher_{}", tag));
        fs::create_dir_all(dir.join("models")).unwrap();
        dir
    }

    fn make_launcher(base_dir: PathBuf, filename: &str, expected_size: u64) -> AppLauncher {
        use crate::launcher::app_config::{known_model_tuple, ModelConfig, UrlPair};
        let model = if let Some(known) = known_model_tuple(filename) {
            ModelConfig::Known {
                filename: known.filename.to_string(),
                expected_size: known.expected_size,
                urls: UrlPair::single(known.url),
            }
        } else {
            ModelConfig::Local {
                filename: filename.to_string(),
                expected_size,
            }
        };
        let mut config = AppConfig::default();
        config.model = model;
        AppLauncher {
            base_dir,
            config,
            http_client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            ui_lang: "ja".to_string(),
        }
    }

    // --- check_ready: authority backend のみ参照する ---

    /// check_ready はランタイムを authority backend (launcher_config.toml) で絞る。
    /// TODO: check_ready が resolve_install_root() に依存しているため、
    /// config injection seam を追加後に統合テストとして実装する。
    /// 現状は has_complete_runtime / runtime_is_complete の直呼びで代替する。

    #[test]
    fn check_ready_authority_backend_only_runtime_check() {
        // authority=cuda, runtime/vulkan=complete, runtime/cuda=incomplete → false
        let base = temp_base("cr_authority");
        let vk_dir = base.join("runtime").join("vulkan");
        fs::create_dir_all(&vk_dir).unwrap();
        let exe = if cfg!(target_os = "windows") {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        fs::write(vk_dir.join(exe), b"").unwrap();

        let cuda_dir = base.join("runtime").join("cuda");
        fs::create_dir_all(&cuda_dir).unwrap();
        // cuda incomplete (no exe)

        let vulkan_ok = runtime_is_complete(&vk_dir, "vulkan");
        let cuda_ok = runtime_is_complete(&cuda_dir, "cuda");

        assert!(vulkan_ok, "vulkan runtime should be complete");
        assert!(!cuda_ok, "cuda runtime without exe should be incomplete");

        // Verify: authority cuda + vulkan complete → check_ready would return false
        // (model also required, but runtime gate fails first)
        // Full check_ready integration requires seam injection — see TODO above.

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn backend_plan_skips_authority_download_when_gpu_evidence_disagrees() {
        let base = temp_base("plan_skip_authority_download");
        let mut launcher = make_launcher(base.clone(), "authority.gguf", 1000);
        launcher.config.backend = "cuda".to_string();

        let plan = launcher.build_backend_plan_from_observations(
            vec![BackendCandidate::new(
                "vulkan".to_string(),
                BackendCandidateReason::GpuDetection,
            )],
            Vec::new(),
        );

        assert!(plan.authority_download.is_none());
        assert_eq!(
            plan.skipped_authority_download
                .as_ref()
                .map(|candidate| candidate.name.as_str()),
            Some("cuda")
        );
        assert_eq!(
            plan.fallback_downloads
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["vulkan"]
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn backend_plan_keeps_complete_authority_runtime_in_installed_pass() {
        let base = temp_base("plan_installed_authority");
        let mut launcher = make_launcher(base.clone(), "authority.gguf", 1000);
        launcher.config.backend = "cuda".to_string();

        let runtime_dir = base.join("runtime").join("cuda");
        fs::create_dir_all(&runtime_dir).unwrap();
        let exe = if cfg!(target_os = "windows") {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        fs::write(runtime_dir.join(exe), b"").unwrap();
        fs::write(runtime_dir.join("nvcuda.dll"), b"").unwrap();

        let plan = launcher.build_backend_plan_from_observations(
            vec![BackendCandidate::new(
                "vulkan".to_string(),
                BackendCandidateReason::GpuDetection,
            )],
            Vec::new(),
        );

        assert_eq!(
            plan.installed
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["cuda"]
        );
        assert!(plan.authority_download.is_none());
        assert!(plan.skipped_authority_download.is_some());

        let _ = fs::remove_dir_all(&base);
    }
}
