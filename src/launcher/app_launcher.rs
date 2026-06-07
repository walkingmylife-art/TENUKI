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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use super::download;

/// 診断メッセージを SubStatus で流し、launcher_debug.log にも追記する。
#[allow(dead_code)]
fn diag(progress_tx: &Sender<LaunchProgress>, base_dir: &Path, msg: &str) {
    download::diag(progress_tx, base_dir, msg)
}

use super::app_config::AppConfig;
use super::backend_detector::BackendDetector;
use super::progress::LaunchProgress;
use super::runtime_downloader::find_llama_server_exe;

/// モデルファイルが完成しているか判定する共通関数。
///
/// 完成条件:
/// - ファイルが存在する
/// - sidecar (.sidecar.json) が存在しない
/// - expected_size > 0 のとき、ファイルサイズが一致する
/// - expected_size == 0 のときは未完成扱い（サイズ検証不能）
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
pub fn check_ready_detail(base_dir: &std::path::Path) -> Result<(), CheckReadyReason> {
    download::check_ready_detail(base_dir)
}

pub fn check_ready(base_dir: &std::path::Path) -> bool {
    download::check_ready(base_dir)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendCandidateReason {
    AuthorityConfig,
    GpuDetection,
    InstalledRuntime,
}

impl BackendCandidateReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityConfig => "authority_config",
            Self::GpuDetection => "gpu_detection",
            Self::InstalledRuntime => "installed_runtime",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BackendCandidate {
    pub(crate) name: String,
    pub(crate) reason: BackendCandidateReason,
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
pub struct VerifiedBackendCandidate {
    pub candidate: BackendCandidate,
    pub exe_path: PathBuf,
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
        download::ensure_model(&self.base_dir, &self.config.model, &self.ui_lang, progress_tx, cancel_flag)
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
        download::test_backend_exe(&self.http_client, &self.config, &exe_path, model_path, &self.base_dir)?;
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
        download::try_backend(&self.base_dir, &self.config, &self.http_client, &self.ui_lang, candidate, model_path, progress_tx, cancel_flag)
    }

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
