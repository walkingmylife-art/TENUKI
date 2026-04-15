// src/launcher/app_launcher.rs

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use std::fs::File;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};
use walkdir::WalkDir;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use super::app_config::{AppConfig, ServerConfig};
use super::backend_detector::BackendDetector;

use super::progress::{LaunchProgress, LauncherStage, SetupMode};
use super::runtime_downloader::RuntimeDownloader;

// 固定ランタイム URL
const CUDA_RUNTIME_URL: &str =
    "https://github.com/ggerganov/llama.cpp/releases/download/b8783/llama-b8783-bin-win-cuda-12.4-x64.zip";
const VULKAN_RUNTIME_URL: &str =
    "https://github.com/ggerganov/llama.cpp/releases/download/b8783/llama-b8783-bin-win-vulkan-x64.zip";
const ROCM_RUNTIME_URL: &str =
    "https://github.com/ggerganov/llama.cpp/releases/download/b8783/llama-b8783-bin-win-hip-radeon-x64.zip";

/// ランチャー画面の文言を ui_lang に応じて返す
fn launcher_text(ui_lang: &str) -> LauncherText<'static> {
    match ui_lang {
        "en" => LauncherText {
            checking_directories: "Checking directories...",
            detecting_gpu: "Detecting GPU...",
            preparing_backend: "Preparing backend...",
            checking_model: "Checking model...",
            downloading_model: "Downloading model...",
            verifying_backend: "Verifying llama-server...",
            saving_config: "Saving config...",
            candidate_prefix: "Candidate:",
            fetching_prefix: "Fetching:",
            trying_backend_suffix: "...",
            downloading_percent: "Downloading {:.0}%",
            extracting_percent: "Extracting {:.0}%",
            runtime_downloading: "Downloading {} runtime...",
            runtime_failed: "{} runtime download failed: {}",
            runtime_not_found: "{} executable not found",
            model_downloading: "Downloading model...",
            model_download_progress: "Model download {:.0}%",
            cancelled: " cancelled",
            unexpected_exit: "Launcher exited unexpectedly",
        },
        _ => LauncherText {
            checking_directories: "ディレクトリを確認中...",
            detecting_gpu: "GPUを検出中...",
            preparing_backend: "バックエンドを準備中...",
            checking_model: "モデルを確認中...",
            downloading_model: "モデルをダウンロード中...",
            verifying_backend: "llama-server を検証中...",
            saving_config: "設定を保存中...",
            candidate_prefix: "候補:",
            fetching_prefix: "取得中:",
            trying_backend_suffix: " を試行中...",
            downloading_percent: "ダウンロード中 {:.0}%",
            extracting_percent: "展開中 {:.0}%",
            runtime_downloading: "{} ランタイムをダウンロード中...",
            runtime_failed: "{} ランタイムの取得に失敗: {}",
            runtime_not_found: "{} の実行ファイルが見つかりません",
            model_downloading: "モデルをダウンロード中...",
            model_download_progress: "モデルダウンロード中 {:.0}%",
            cancelled: " されました",
            unexpected_exit: "起動処理が予期せず終了しました",
        },
    }
}

pub struct LauncherText<'a> {
    pub checking_directories: &'a str,
    pub detecting_gpu: &'a str,
    pub preparing_backend: &'a str,
    pub checking_model: &'a str,
    pub downloading_model: &'a str,
    pub verifying_backend: &'a str,
    pub saving_config: &'a str,
    pub candidate_prefix: &'a str,
    pub fetching_prefix: &'a str,
    pub trying_backend_suffix: &'a str,
    pub downloading_percent: &'a str,
    pub extracting_percent: &'a str,
    pub runtime_downloading: &'a str,
    pub runtime_failed: &'a str,
    pub runtime_not_found: &'a str,
    pub model_downloading: &'a str,
    pub model_download_progress: &'a str,
    pub cancelled: &'a str,
    pub unexpected_exit: &'a str,
}

/// 通常起動に必要なものが全部揃っているか確認する。
/// migration・HTTP クライアント構築は一切しない。起動パスのちらつきを防ぐため軽量に保つ。
pub fn check_ready(base_dir: &Path) -> bool {
    let config_path = base_dir.join("launcher_config.toml");
    let config = match AppConfig::load_with_mode(&config_path, "structural") {
        Ok(c) => c,
        Err(_) => return false,
    };

    let runtime_ok = if config.backend.is_empty() || config.backend == "unknown" {
        ["cuda", "vulkan", "rocm"].iter().any(|name| {
            find_llama_server_exe(&base_dir.join("runtime").join(name)).is_some()
        })
    } else {
        find_llama_server_exe(&base_dir.join("runtime").join(&config.backend)).is_some()
    };

    if !runtime_ok {
        return false;
    }

    let model_path = base_dir.join("models").join(&config.model.filename);
    model_path.exists()
        && std::fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0) >= 10 * 1024 * 1024
}

pub struct AppLauncher {
    base_dir: PathBuf,
    config: AppConfig,
    http_client: Client,
    ui_lang: String,
}

impl AppLauncher {
    pub fn new(base_dir: PathBuf, ui_lang: String) -> Result<Self> {
        // 旧 config.toml があれば launcher_config.toml に移行する
        let config_toml = base_dir.join("config.toml");
        if let Err(e) = super::migration::migrate_config_if_needed(&config_toml) {
            log::warn!("Config migration failed (continuing with defaults): {:#}", e);
        }

        let config_path = base_dir.join("launcher_config.toml");
        // config.toml の translation_mode を見て初回デフォルトを決める
        let translation_mode = crate::config::load(&config_toml)
            .map(|c| c.translation_mode)
            .unwrap_or_else(|_| "structural".to_string());
        let mut config = AppConfig::load_with_mode(&config_path, &translation_mode)?;

        // state.json が残っていて backend が記録されていれば launcher_config.toml に一度だけ引き継ぐ
        // （state.json 廃止移行期の救済。引き継いだら state.json は削除する）
        let state_path = base_dir.join("state.json");
        if state_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&state_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(backend) = val.get("backend").and_then(|v| v.as_str()) {
                        if !backend.is_empty() && config.backend != backend {
                            log::info!("Migrating backend '{}' from state.json to launcher_config.toml", backend);
                            config.backend = backend.to_string();
                            let _ = config.save(&config_path);
                        }
                    }
                }
            }
            // state.json を削除して廃止完了
            let _ = std::fs::remove_file(&state_path);
        }

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

    pub fn run(
        &mut self,
        mode: SetupMode,
        progress_tx: Sender<LaunchProgress>,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        match mode {
            SetupMode::Full         => self.run_full(&progress_tx, &cancel_flag),
            SetupMode::RepairRuntime => self.run_repair_runtime(&progress_tx, &cancel_flag),
            SetupMode::RepairModel   => self.run_repair_model(&progress_tx, &cancel_flag),
        }
    }

    // Full: Dir → GPU → Runtime(DL/展開) → Model(DL) → Verify → Save
    fn run_full(
        &mut self,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<()> {
        let txt = launcher_text(&self.ui_lang);
        macro_rules! check_cancel {
            () => {
                if cancel_flag.load(Ordering::Relaxed) {
                    progress_tx.send(LaunchProgress::Cancelled).ok();
                    anyhow::bail!("Cancelled by user");
                }
            };
        }

        // 1. ディレクトリ準備  0%–10%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Directories)).ok();
        progress_tx.send(LaunchProgress::Status(txt.checking_directories.to_string())).ok();
        report_stage_progress(progress_tx, 0.0, 0.10, 0.0);
        self.create_directories()?;
        report_stage_progress(progress_tx, 0.0, 0.10, 1.0);
        check_cancel!();

        // 2. GPU検出  10%–20%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Gpu)).ok();
        progress_tx.send(LaunchProgress::Status(txt.detecting_gpu.to_string())).ok();
        report_stage_progress(progress_tx, 0.10, 0.20, 0.0);
        let candidates = self.build_backend_candidates();
        let gpu_msg = if candidates.is_empty() {
            if self.ui_lang == "en" { "No backend candidates available".to_string() }
            else { "利用可能なバックエンド候補が見つかりません".to_string() }
        } else {
            format!("{} {}", txt.candidate_prefix, candidates.join(" → "))
        };
        progress_tx.send(LaunchProgress::SubStatus(gpu_msg)).ok();
        report_stage_progress(progress_tx, 0.10, 0.20, 1.0);
        check_cancel!();

        // 3. Runtime 取得  20%–45%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Runtime)).ok();
        progress_tx.send(LaunchProgress::Status(txt.preparing_backend.to_string())).ok();
        report_stage_progress(progress_tx, 0.20, 0.45, 0.0);
        let (backend_name, exe_path) = self.prepare_backend_runtime_with_progress(
            &candidates, progress_tx, cancel_flag, 0.20, 0.45,
        )?;
        report_stage_progress(progress_tx, 0.20, 0.45, 1.0);
        check_cancel!();

        // 4. Model 取得  45%–85%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Model)).ok();
        progress_tx.send(LaunchProgress::Status(txt.checking_model.to_string())).ok();
        report_stage_progress(progress_tx, 0.45, 0.85, 0.0);
        let model_path = self.ensure_model_with_progress(progress_tx, cancel_flag, 0.45, 0.85)?;
        report_stage_progress(progress_tx, 0.45, 0.85, 1.0);
        check_cancel!();

        // 5. Verify  85%–95%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Verify)).ok();
        progress_tx.send(LaunchProgress::Status(txt.verifying_backend.to_string())).ok();
        report_stage_progress(progress_tx, 0.85, 0.95, 0.0);
        let ctx_msg = if self.ui_lang == "en" { "Failed to load model with selected backend" }
                      else { "選択されたバックエンドでモデルを読み込めませんでした" };
        self.test_backend_exe(&exe_path, &model_path).context(ctx_msg)?;
        report_stage_progress(progress_tx, 0.85, 0.95, 1.0);
        check_cancel!();

        // 6. Save  95%–100%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Save)).ok();
        progress_tx.send(LaunchProgress::Status(txt.saving_config.to_string())).ok();
        report_stage_progress(progress_tx, 0.95, 1.0, 0.0);
        self.config.backend = backend_name;
        self.config.save(&self.base_dir.join("launcher_config.toml"))?;
        report_stage_progress(progress_tx, 0.95, 1.0, 1.0);

        progress_tx.send(LaunchProgress::Stage(LauncherStage::Complete)).ok();
        progress_tx.send(LaunchProgress::Complete).ok();
        Ok(())
    }

    // RepairRuntime: Dir → GPU → Runtime(DL/展開) → Verify → Save（model は既存を使う）
    fn run_repair_runtime(
        &mut self,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<()> {
        let txt = launcher_text(&self.ui_lang);
        macro_rules! check_cancel {
            () => {
                if cancel_flag.load(Ordering::Relaxed) {
                    progress_tx.send(LaunchProgress::Cancelled).ok();
                    anyhow::bail!("Cancelled by user");
                }
            };
        }

        // 1. Dir  0%–10%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Directories)).ok();
        progress_tx.send(LaunchProgress::Status(txt.checking_directories.to_string())).ok();
        report_stage_progress(progress_tx, 0.0, 0.10, 0.0);
        self.create_directories()?;
        report_stage_progress(progress_tx, 0.0, 0.10, 1.0);
        check_cancel!();

        // 2. GPU  10%–25%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Gpu)).ok();
        progress_tx.send(LaunchProgress::Status(txt.detecting_gpu.to_string())).ok();
        report_stage_progress(progress_tx, 0.10, 0.25, 0.0);
        let candidates = self.build_backend_candidates();
        report_stage_progress(progress_tx, 0.10, 0.25, 1.0);
        check_cancel!();

        // 3. Runtime  25%–75%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Runtime)).ok();
        progress_tx.send(LaunchProgress::Status(txt.preparing_backend.to_string())).ok();
        report_stage_progress(progress_tx, 0.25, 0.75, 0.0);
        let (backend_name, exe_path) = self.prepare_backend_runtime_with_progress(
            &candidates, progress_tx, cancel_flag, 0.25, 0.75,
        )?;
        report_stage_progress(progress_tx, 0.25, 0.75, 1.0);
        check_cancel!();

        // 4. Verify（既存 model で）  75%–90%
        let model_path = {
            let p = self.base_dir.join("models").join(&self.config.model.filename);
            if !p.exists() {
                anyhow::bail!("Model file not found for verify: {}", p.display());
            }
            p
        };
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Verify)).ok();
        progress_tx.send(LaunchProgress::Status(txt.verifying_backend.to_string())).ok();
        report_stage_progress(progress_tx, 0.75, 0.90, 0.0);
        let ctx_msg = if self.ui_lang == "en" { "Failed to load model with selected backend" }
                      else { "選択されたバックエンドでモデルを読み込めませんでした" };
        self.test_backend_exe(&exe_path, &model_path).context(ctx_msg)?;
        report_stage_progress(progress_tx, 0.75, 0.90, 1.0);
        check_cancel!();

        // 5. Save  90%–100%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Save)).ok();
        progress_tx.send(LaunchProgress::Status(txt.saving_config.to_string())).ok();
        report_stage_progress(progress_tx, 0.90, 1.0, 0.0);
        self.config.backend = backend_name;
        self.config.save(&self.base_dir.join("launcher_config.toml"))?;
        report_stage_progress(progress_tx, 0.90, 1.0, 1.0);

        progress_tx.send(LaunchProgress::Stage(LauncherStage::Complete)).ok();
        progress_tx.send(LaunchProgress::Complete).ok();
        Ok(())
    }

    // RepairModel: Dir → Model(DL) → Save（runtime・verify は触らない）
    fn run_repair_model(
        &mut self,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<()> {
        let txt = launcher_text(&self.ui_lang);
        macro_rules! check_cancel {
            () => {
                if cancel_flag.load(Ordering::Relaxed) {
                    progress_tx.send(LaunchProgress::Cancelled).ok();
                    anyhow::bail!("Cancelled by user");
                }
            };
        }

        // 1. Dir  0%–10%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Directories)).ok();
        progress_tx.send(LaunchProgress::Status(txt.checking_directories.to_string())).ok();
        report_stage_progress(progress_tx, 0.0, 0.10, 0.0);
        self.create_directories()?;
        report_stage_progress(progress_tx, 0.0, 0.10, 1.0);
        check_cancel!();

        // 2. Model  10%–90%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Model)).ok();
        progress_tx.send(LaunchProgress::Status(txt.checking_model.to_string())).ok();
        report_stage_progress(progress_tx, 0.10, 0.90, 0.0);
        self.ensure_model_with_progress(progress_tx, cancel_flag, 0.10, 0.90)?;
        report_stage_progress(progress_tx, 0.10, 0.90, 1.0);
        check_cancel!();

        // 3. Save  90%–100%
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Save)).ok();
        progress_tx.send(LaunchProgress::Status(txt.saving_config.to_string())).ok();
        report_stage_progress(progress_tx, 0.90, 1.0, 0.0);
        self.config.save(&self.base_dir.join("launcher_config.toml"))?;
        report_stage_progress(progress_tx, 0.90, 1.0, 1.0);

        progress_tx.send(LaunchProgress::Stage(LauncherStage::Complete)).ok();
        progress_tx.send(LaunchProgress::Complete).ok();
        Ok(())
    }

    fn prepare_backend_runtime_with_progress(
        &self,
        candidates: &[String],
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
        stage_start: f32,
        stage_end: f32,
    ) -> Result<(String, PathBuf)> {
        if candidates.is_empty() {
            anyhow::bail!("利用可能なバックエンド候補がありません");
        }

        let txt = launcher_text(&self.ui_lang);
        let mut errors = Vec::new();
        let total_candidates = candidates.len() as f32;

        for (idx, name) in candidates.iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                progress_tx.send(LaunchProgress::Cancelled).ok();
                anyhow::bail!("Cancelled by user");
            }

            // 候補ごとの進捗範囲
            let candidate_start = stage_start + (idx as f32 / total_candidates) * (stage_end - stage_start);
            let candidate_end = stage_start + ((idx + 1) as f32 / total_candidates) * (stage_end - stage_start);
            report_stage_progress(progress_tx, candidate_start, candidate_end, 0.0);

            progress_tx
                .send(LaunchProgress::SubStatus(format!("{}{}", name, txt.trying_backend_suffix)))
                .ok();

            let runtime_dir = self.base_dir.join("runtime").join(name);

            if let Err(e) = self.ensure_runtime_with_progress(
                name,
                &runtime_dir,
                progress_tx,
                cancel_flag,
                candidate_start,
                candidate_end,
            ) {
                let msg = txt.runtime_failed.replacen("{}", name, 1).replacen("{}", &e.to_string(), 1);
                progress_tx.send(LaunchProgress::SubStatus(msg.clone())).ok();
                errors.push(msg);
                continue;
            }

            let exe_path = match find_llama_server_exe(&runtime_dir) {
                Some(p) => p,
                None => {
                    let msg = txt.runtime_not_found.replacen("{}", name, 1);
                    progress_tx.send(LaunchProgress::SubStatus(msg.clone())).ok();
                    errors.push(msg);
                    continue;
                }
            };

            report_stage_progress(progress_tx, candidate_start, candidate_end, 1.0);
            return Ok((name.to_string(), exe_path));
        }

        if errors.is_empty() {
            anyhow::bail!("利用可能なバックエンドが見つかりませんでした")
        } else {
            anyhow::bail!(
                "利用可能なバックエンドが見つかりませんでした。\n詳細: {}",
                errors.join(" / ")
            )
        }
    }

    fn ensure_runtime_with_progress(
        &self,
        name: &str,
        dest_dir: &Path,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
        stage_start: f32,
        stage_end: f32,
    ) -> Result<()> {
        let txt = launcher_text(&self.ui_lang);

        if find_llama_server_exe(dest_dir).is_some() {
            report_stage_progress(progress_tx, stage_start, stage_end, 1.0);
            return Ok(());
        }

        progress_tx.send(LaunchProgress::Status(txt.runtime_downloading.replacen("{}", name, 1))).ok();
        report_stage_progress(progress_tx, stage_start, stage_end, 0.0);

        let url = runtime_download_url(name)?;
        let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
        // download_backend 内部でダウンロード(0.0〜0.8)→展開(0.8〜1.0)に分割済み
        // progress はすでに単調増加が保証されている
        const EXTRACT_THRESHOLD: f32 = 0.8;
        downloader.download_backend(
            &url,
            dest_dir,
            |progress, _status| {
                let sub_status = if progress < EXTRACT_THRESHOLD {
                    let dl_pct = (progress / EXTRACT_THRESHOLD * 100.0).min(100.0);
                    txt.downloading_percent.replacen("{:.0}", &format!("{:.0}", dl_pct), 1)
                } else {
                    let ex_pct = ((progress - EXTRACT_THRESHOLD) / (1.0 - EXTRACT_THRESHOLD) * 100.0).min(100.0);
                    txt.extracting_percent.replacen("{:.0}", &format!("{:.0}", ex_pct), 1)
                };
                let _ = progress_tx.send(LaunchProgress::SubStatus(sub_status));
                report_stage_progress(progress_tx, stage_start, stage_end, progress);
            },
        )?;

        report_stage_progress(progress_tx, stage_start, stage_end, 1.0);
        Ok(())
    }

    fn ensure_model_with_progress(
        &self,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
        stage_start: f32,
        stage_end: f32,
    ) -> Result<PathBuf> {
        let txt = launcher_text(&self.ui_lang);
        let model_dir = self.base_dir.join("models");
        let model_path = model_dir.join(&self.config.model.filename);

        if model_path.exists() {
            let metadata = std::fs::metadata(&model_path)?;
            if metadata.len() >= 10 * 1024 * 1024 {
                report_stage_progress(progress_tx, stage_start, stage_end, 1.0);
                return Ok(model_path);
            }
        }

        progress_tx.send(LaunchProgress::Status(txt.model_downloading.to_string())).ok();
        progress_tx.send(LaunchProgress::SubStatus(format!("{} {}", txt.fetching_prefix, self.config.model.filename))).ok();
        report_stage_progress(progress_tx, stage_start, stage_end, 0.0);
    
        // expected_size が0の場合は HuggingFace API からサイズを取得
        let expected_size = if self.config.model.expected_size > 0 {
            Some(self.config.model.expected_size)
        } else {
            match RuntimeDownloader::fetch_huggingface_size(&self.config.model.url) {
                Ok(size) => {
                    log::info!("Fetched model size from HuggingFace: {} bytes", size);
                    Some(size)
                }
                Err(e) => {
                    log::warn!("Failed to fetch model size: {}, proceeding without size check", e);
                    None
                }
            }
        };
    
        let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
        downloader.download_model(
            &self.config.model.url,
            &model_path,
            expected_size,
            |progress, _status| {
                let sub_status = txt.model_download_progress.replacen("{:.0}", &format!("{:.0}", progress * 100.0), 1);
                let _ = progress_tx.send(LaunchProgress::SubStatus(sub_status));
                report_stage_progress(progress_tx, stage_start, stage_end, progress);
            },
        )?;

        report_stage_progress(progress_tx, stage_start, stage_end, 1.0);
        Ok(model_path)
    }

    fn create_directories(&self) -> Result<()> {
        let dirs = ["runtime", "models", "profiles", "logs", "tmp", "dicts"];
        for d in dirs {
            std::fs::create_dir_all(self.base_dir.join(d))?;
        }

        // profiles/ が空なら default.toml と game.toml を生成する
        let profiles_dir = self.base_dir.join("profiles");
        let default_path = profiles_dir.join("default.toml");
        if !default_path.exists() {
            crate::launcher::translation_profile::TranslationProfile::default()
                .save(&default_path)
                .ok();
        }
        let game_path = profiles_dir.join("game.toml");
        if !game_path.exists() {
            crate::launcher::translation_profile::TranslationProfile::game_default()
                .save(&game_path)
                .ok();
        }

        Ok(())
    }

    fn build_backend_candidates(&self) -> Vec<String> {
        // 優先順:
        //   1. GPU 検出結果（カードに合った backend を正とする）
        //   2. runtime/ に既に展開済みのもの（フォールバック）
        // 前回成功値（config.backend）は候補生成には使わない。
        // GPU 検出と一致していれば自然に先頭に来る。
        let mut candidates = Vec::new();
        let gpus = BackendDetector::enumerate_gpus().unwrap_or_default();
        let detected = BackendDetector::build_backend_candidate_names(&gpus);
        for name in detected {
            if !candidates.iter().any(|c| c == &name) {
                candidates.push(name);
            }
        }
        for name in ["cuda", "rocm", "vulkan"] {
            if !candidates.iter().any(|c| c == name) {
                let dir = self.base_dir.join("runtime").join(name);
                if dir.exists() {
                    candidates.push(name.to_string());
                }
            }
        }
        candidates
    }

    fn test_backend_exe(&self, exe_path: &Path, model_path: &Path) -> Result<()> {
        let test_port = find_free_port()?;
        let log_path = self.base_dir.join("logs").join("launcher_llama_stderr.txt");
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stderr_file = File::create(&log_path)
            .with_context(|| format!("Failed to create stderr log: {}", log_path.display()))?;
    
        let mut cmd = build_server_command(exe_path, test_port, model_path, &self.config.server);
        if let Some(parent) = exe_path.parent() {
            cmd.current_dir(parent);
        }
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .with_context(|| format!("Failed to spawn test backend: {}", exe_path.display()))?;
    
        self.wait_for_healthy_process(child, test_port, Duration::from_secs(90), &log_path)?;

        // Health OK 後に stderr ログで GPU オフロードを確認する
        // Vulkan backend が読み込まれていて、かつ 1 層以上 GPU にオフロードされていることを要求する
        let stderr_log = read_log_tail(&log_path, 32000);
        check_gpu_offload(&stderr_log, &self.config.backend, &self.ui_lang)?;

        Ok(())
    }
    
    fn wait_for_healthy_process(
        &self,
        mut child: Child,
        port: u16,
        timeout: Duration,
        log_path: &Path,
    ) -> Result<()> {
        let start = Instant::now();
        let interval = Duration::from_millis(500);
        loop {
            match child.try_wait()? {
                Some(status) => {
                    let stderr_tail = read_log_tail(log_path, 8000);
                    terminate_child(child);
                    if stderr_tail.is_empty() {
                        anyhow::bail!("llama-server exited early with status: {}", status);
                    } else {
                        anyhow::bail!(
                            "llama-server exited early with status: {}\nstderr:\n{}",
                            status,
                            stderr_tail
                        );
                    }
                }
                None => {
                    if self.check_health(port).is_ok() {
                        terminate_child(child);
                        return Ok(());
                    }
                    if start.elapsed() > timeout {
                        terminate_child(child);
                        let stderr_tail = read_log_tail(log_path, 8000);
                        if stderr_tail.is_empty() {
                            anyhow::bail!("Health check timeout");
                        } else {
                            anyhow::bail!("Health check timeout\nstderr:\n{}", stderr_tail);
                        }
                    }
                }
            }
            thread::sleep(interval);
        }
    }

    fn check_health(&self, port: u16) -> Result<()> {
        let host = &self.config.server.host;
        let url = format!("http://{}:{}/health", host, port);
        let resp = self.http_client.get(&url).send()?;
        if resp.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("Health endpoint returned {}", resp.status());
        }
    }
}

// ----------------------------------------------------------------------------
// ユーティリティ
// ----------------------------------------------------------------------------

fn report_stage_progress(
    tx: &Sender<LaunchProgress>,
    start: f32,
    end: f32,
    local: f32,
) {
    // 整数演算で累積誤差を防ぐ（10000分の1精度）
    const SCALE: i32 = 10000;
    let start_bp = (start * SCALE as f32) as i32;
    let end_bp = (end * SCALE as f32) as i32;
    let local_bp = (local.clamp(0.0, 1.0) * SCALE as f32) as i32;
    let global_bp = start_bp + (end_bp - start_bp) * local_bp / SCALE;
    let global = (global_bp as f32) / SCALE as f32;
    let _ = tx.send(LaunchProgress::Progress(global));
}

fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn llama_server_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

fn find_llama_server_exe(dir: &Path) -> Option<PathBuf> {
    let name = llama_server_binary_name();
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy() == name)
        .map(|e| e.path().to_path_buf())
}

fn build_server_command(exe: &Path, port: u16, model: &Path, server_cfg: &ServerConfig) -> Command {
    crate::backend::process::build_llama_command(
        exe,
        model,
        port,
        server_cfg.ngl,
        server_cfg.ctx_size,
        server_cfg.batch_size,
        server_cfg.ubatch_size,
        server_cfg.cont_batching,
        server_cfg.parallel_slots,
        &server_cfg.extra_args,
    )
}

fn read_log_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let start = buf.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&buf[start..])
        .replace('\r', "")
        .trim()
        .to_string()
}

fn terminate_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// llama-server の起動ログから GPU オフロードが実際に行われたか確認する。
///
/// 合格条件:
///   - backend が vulkan/cuda/rocm のとき、対応する DLL ロードログが存在すること
///   - "offloaded N/M layers to GPU" の N が 1 以上であること
///
/// CPU fallback（offloaded 0/M layers）は失敗扱い。
fn check_gpu_offload(log: &str, backend: &str, ui_lang: &str) -> Result<()> {
    // 1. backend DLL がロードされたか確認
    let backend_loaded = match backend {
        "vulkan" => log.contains("loaded Vulkan backend") || log.contains("ggml-vulkan"),
        "cuda"   => log.contains("loaded CUDA backend")  || log.contains("ggml-cuda"),
        "rocm"   => log.contains("loaded ROCm backend")  || log.contains("ggml-hipblas"),
        _        => true, // 未知 backend は通過させる
    };

    if !backend_loaded {
        if ui_lang == "en" {
            anyhow::bail!(
                "{} backend DLL was not loaded. Runtime may be mismatched or DLL is missing.",
                backend
            );
        } else {
            anyhow::bail!(
                "{} バックエンド DLL がロードされませんでした。Runtime の配置または DLL が不正です。",
                backend
            );
        }
    }

    // 2. GPU オフロード層数を確認 ("offloaded N/M layers to GPU")
    // llama.cpp の出力例: "llm_load_tensors: offloaded 29/29 layers to GPU"
    for line in log.lines() {
        if line.contains("offloaded") && line.contains("layers to GPU") {
            // "offloaded N/M" の N を抽出
            if let Some(n) = line.split("offloaded")
                .nth(1)
                .and_then(|s| s.trim().split('/').next())
                .and_then(|s| s.trim().parse::<u32>().ok())
            {
                if n == 0 {
                    if ui_lang == "en" {
                        anyhow::bail!(
                            "GPU offload failed: 0 layers offloaded to {}. Falling back to CPU is not acceptable.",
                            backend
                        );
                    } else {
                        anyhow::bail!(
                            "GPU オフロード失敗: {} への GPU オフロードが 0 層です。CPU fallback は許容されません。",
                            backend
                        );
                    }
                }
                // N > 0 → GPU オフロード確認済み
                return Ok(());
            }
        }
    }

    // "offloaded N/M layers to GPU" が見つからなかった場合
    // llama.cpp のバージョンによってログ形式が異なる可能性があるため、
    // backend DLL がロードされていれば通過させる（厳密すぎる拒否を避ける）
    Ok(())
}

fn runtime_download_url(name: &str) -> Result<String> {
    match name {
        "cuda" => Ok(CUDA_RUNTIME_URL.to_string()),
        "vulkan" => Ok(VULKAN_RUNTIME_URL.to_string()),
        "rocm" => Ok(ROCM_RUNTIME_URL.to_string()),
        _ => anyhow::bail!("Unsupported backend: {}", name),
    }
}