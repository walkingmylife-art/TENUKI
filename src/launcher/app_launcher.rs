// src/launcher/app_launcher.rs
//
// 役割：権威設定（AppConfig）に基づきモデルとバックエンドを準備し、
//       実際に使用した URL や実行ファイルのパスをキャッシュ（LauncherState）に保存する。
// - LauncherState はあくまでキャッシュであり、権威設定を上書きしない。
// - 検証（test_backend_exe）は必ず 127.0.0.1 + 動的ポートを使用する。
// - 起動引数の組み立ては build_server_command() に一元化し、本番起動とテスト起動で完全に一致させる。
// - LaunchProgress::Progress は「現在のフェーズ（モデルDL/ランタイムDL）単体の進捗（0.0～1.0）」であり、
//   UI 側で全体進捗に換算することを前提とする。

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

/// 診断メッセージを SubStatus で流し、launcher_debug.log にも追記する。
fn diag(progress_tx: &Sender<LaunchProgress>, base_dir: &Path, msg: &str) {
    progress_tx.send(LaunchProgress::SubStatus(msg.to_string())).ok();
    let log_path = base_dir.join("launcher_debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

use super::app_config::AppConfig;
use super::backend_detector::BackendDetector;
use super::launcher_state::LauncherState;
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

/// runtime とモデルが揃っているか確認する。
/// main.rs でモードを決定するために使用される。
/// - launcher_config.toml は install_root から読む（権威位置）
/// - models/, runtime/, state は base_dir から読む
pub fn check_ready(base_dir: &std::path::Path) -> bool {
    use crate::launcher::runtime_downloader::runtime_is_complete;

    // launcher_config.toml は install_root（権威位置）から読む
    let install_root = super::resolve_install_root();
    let config_path = install_root.join("launcher_config.toml");
    let config = match super::app_config::AppConfig::load(&config_path) {
        Ok(c) => c,
        Err(_) => {
            diag_file(base_dir, "[check_ready] config load failed → false");
            return false;
        }
    };

    // runtime 確認: launcher_state のキャッシュを使って backend dir を特定
    let state_path = base_dir.join("launcher_state.json");
    let state = super::launcher_state::LauncherState::load(&state_path).unwrap_or_default();
    let backend = match &state.backend {
        Some(b) => b.clone(),
        None => {
            diag_file(base_dir, "[check_ready] no backend in state → false");
            return false;
        }
    };
    let runtime_dir = base_dir.join("runtime").join(&backend);
    let rt_ok = runtime_is_complete(&runtime_dir, &backend);
    diag_file(base_dir, &format!(
        "[check_ready] runtime_is_complete backend={} rt_ok={}",
        backend, rt_ok
    ));
    if !rt_ok {
        return false;
    }

    let model_path = base_dir.join("models").join(&config.model.filename);
    let expected_size = config.model.expected_size;
    let ok = model_is_complete(&model_path, expected_size);
    diag_file(base_dir, &format!(
        "[check_ready] model_is_complete={} filename={} expected_size={}",
        ok, config.model.filename, expected_size
    ));
    if !ok {
        return false;
    }
    diag_file(base_dir, "[check_ready] → true");
    true
}

pub struct AppLauncher {
    base_dir: PathBuf,
    config: AppConfig,
    state: LauncherState,
    http_client: Client,
    ui_lang: String,
}

impl AppLauncher {
    pub fn new(base_dir: PathBuf, ui_lang: String) -> Result<Self> {
        // launcher_config.toml は install_root（権威位置）から読む
        let install_root = super::resolve_install_root();
        let config_path = install_root.join("launcher_config.toml");
        let config = AppConfig::load(&config_path)?;

        let state_path = base_dir.join("launcher_state.json");
        let mut state = LauncherState::load(&state_path).unwrap_or_default();

        let old_state_path = base_dir.join("state.json");
        if old_state_path.exists() && !state_path.exists() {
            if let Ok(old_state) = LauncherState::load(&old_state_path) {
                state = old_state;
                state.save(&state_path)?;
            }
        }

        let http_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { base_dir, config, state, http_client, ui_lang })
    }

    fn t(&self, en: &str, ja: &str) -> String {
        if self.ui_lang == "en" { en.to_string() } else { ja.to_string() }
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
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Directories)).ok();
        progress_tx.send(LaunchProgress::Status(self.t("Preparing directories...", "ディレクトリを確認中..."))).ok();
        self.create_directories()?;
        check_cancel!();

        // Stage: Gpu
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Gpu)).ok();
        progress_tx.send(LaunchProgress::Status(self.t("Detecting GPU...", "GPUを検出中..."))).ok();
        let candidates = self.build_backend_candidates();
        check_cancel!();

        // Stage: Model
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Model)).ok();
        progress_tx.send(LaunchProgress::Status(self.t("Checking model...", "モデルを確認中..."))).ok();
        let (model_path, model_url) = self.ensure_model(&progress_tx, &cancel_flag)?;
        check_cancel!();

        // Stage: Runtime
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Runtime)).ok();
        progress_tx.send(LaunchProgress::Status(self.t("Selecting backend...", "バックエンドを選定中..."))).ok();
        let mut working_backend = None;
        for name in candidates.iter() {
            check_cancel!();
            progress_tx.send(LaunchProgress::SubStatus(
                self.t(&format!("Trying {}...", name), &format!("{} を試行中...", name))
            )).ok();
            if let Ok((exe_path, backend_url)) = self.try_backend(name, &model_path, &progress_tx, &cancel_flag) {
                working_backend = Some((name.clone(), exe_path, backend_url));
                break;
            }
        }

        let (backend_name, exe_path, backend_url) =
            working_backend.ok_or_else(|| anyhow!("No working backend found"))?;
        check_cancel!();

        // Stage: Verify (test_backend_exe already ran inside try_backend)
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Verify)).ok();
        progress_tx.send(LaunchProgress::Status(self.t("Verified.", "検証完了."))).ok();
        check_cancel!();

        // Stage: Save
        progress_tx.send(LaunchProgress::Stage(LauncherStage::Save)).ok();
        progress_tx.send(LaunchProgress::Status(self.t("Saving state...", "状態を保存中..."))).ok();
        // キャッシュ更新：今回実際に採用した URL を保存する（権威設定ではない）
        self.state.backend = Some(backend_name.clone());
        self.state.model_filename = Some(self.config.model.filename.clone());
        self.state.runtime_exe_path = Some(exe_path);
        self.state.model_url = Some(model_url);
        self.state.backend_url = Some(backend_url);
        self.state.save(&self.base_dir.join("launcher_state.json"))?;
        self.seed_profiles()?;

        diag(&progress_tx, &self.base_dir, &format!(
            "[run] COMPLETE backend={} model={} exe={} url={}",
            self.state.backend.as_deref().unwrap_or("?"),
            self.state.model_filename.as_deref().unwrap_or("?"),
            self.state.runtime_exe_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "?".into()),
            self.state.backend_url.as_deref().unwrap_or("?"),
        ));
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

    /// モデルを準備し、(ファイルパス, 今回採用する URL) を返す。
    fn ensure_model(
        &self,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<(PathBuf, String)> {
        let expected = self.config.model.expected_size;
        if expected == 0 {
            diag(progress_tx, &self.base_dir,
                "[ensure_model] ERROR: expected_size == 0 in launcher_config.toml");
            anyhow::bail!(
                "model.expected_size is 0 in launcher_config.toml. \
                 Set it to the correct file size in bytes."
            );
        }

        let model_dir = self.base_dir.join("models");
        let model_path = model_dir.join(&self.config.model.filename);

        let complete = model_is_complete(&model_path, expected);
        diag(progress_tx, &self.base_dir, &format!(
            "[ensure_model] filename={} exists={} expected_size={} complete={}",
            self.config.model.filename, model_path.exists(), expected, complete
        ));
        if complete {
            let url = self.state.model_url.clone()
                .unwrap_or_else(|| self.config.model.urls.primary.clone());
            diag(progress_tx, &self.base_dir, "[ensure_model] → reuse existing model");
            return Ok((model_path, url));
        }

        // authority filename がまだない／不完全だが、models/ に同サイズの別名ggufがあれば再利用する。
        // sidecar なし・サイズ完全一致・拡張子 .gguf の先頭ヒットを採用。
        if let Some(found) = self.find_alternative_model(&model_dir, expected) {
            diag(progress_tx, &self.base_dir, &format!(
                "[ensure_model] found alternative model by size: {} → rename to {}",
                found.display(), model_path.display()
            ));
            std::fs::rename(&found, &model_path)?;
            let url = self.state.model_url.clone()
                .unwrap_or_else(|| self.config.model.urls.primary.clone());
            return Ok((model_path, url));
        }

        // 不完全なファイルが残っている場合は削除して再取得
        // (sidecarありの場合はdownload_model側が再開する)
        if model_path.exists() {
            let sidecar = PathBuf::from(format!("{}.sidecar.json", model_path.display()));
            if !sidecar.exists() {
                diag(progress_tx, &self.base_dir, "[ensure_model] incomplete (no sidecar, bad size) → remove and re-download");
                std::fs::remove_file(&model_path)?;
            }
        }

        progress_tx.send(LaunchProgress::Status(self.t("Downloading model...", "モデルをダウンロード中..."))).ok();
        progress_tx.send(LaunchProgress::SubStatus(
            self.t(&format!("Fetching: {}", self.config.model.filename),
                   &format!("取得中: {}", self.config.model.filename)),
        )).ok();

        let urls = &self.config.model.urls;
        let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
        let used_url_opt = downloader.download_model(
            &urls.primary,
            urls.fallback.as_deref(),
            &model_path,
            self.config.model.expected_size,
            |progress, _| {
                // この progress はモデルダウンロード単体の進捗（0.0～1.0）
                progress_tx.send(LaunchProgress::Progress(progress)).ok();
            },
        )?;

        let used_url = used_url_opt.unwrap_or_else(|| {
            self.state.model_url.clone()
                .unwrap_or_else(|| self.config.model.urls.primary.clone())
        });
        Ok((model_path, used_url))
    }

    /// models/ ディレクトリ内から authority filename 以外の .gguf を探し、
    /// sidecar なし・expected_size 一致のものを収集する。
    ///
    /// 採用ルール（優先順）:
    ///   1. state.model_filename と一致する候補が唯一 → それを採用
    ///   2. 候補が唯一 → 採用
    ///   3. 候補が複数 → 採用不能（ambiguous）→ None
    fn find_alternative_model(&self, model_dir: &Path, expected_size: u64) -> Option<PathBuf> {
        let authority_name = &self.config.model.filename;
        let entries = std::fs::read_dir(model_dir).ok()?;

        let candidates: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("gguf") {
                    return None;
                }
                if path.file_name().and_then(|n| n.to_str()) == Some(authority_name.as_str()) {
                    return None;
                }
                if std::fs::metadata(&path).map(|m| m.len() == expected_size).unwrap_or(false) {
                    cleanup_model_resume_artifacts(&path);
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // state.model_filename と一致する候補を優先
        if let Some(state_name) = &self.state.model_filename {
            let preferred: Vec<&PathBuf> = candidates
                .iter()
                .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some(state_name.as_str()))
                .collect();
            if preferred.len() == 1 {
                return Some(preferred[0].clone());
            }
        }

        // 候補が一意なら採用。複数は選択根拠なし → None（再DL）。
        if candidates.len() == 1 {
            candidates.into_iter().next()
        } else {
            None
        }
    }

    fn build_backend_candidates(&self) -> Vec<String> {
        let mut candidates = Vec::new();
        if let Some(backend) = &self.state.backend {
            candidates.push(backend.clone());
        }
        let gpus = BackendDetector::enumerate_gpus().unwrap_or_default();
        for name in BackendDetector::build_backend_candidate_names(&gpus) {
            if !candidates.contains(&name) {
                candidates.push(name);
            }
        }
        for name in ["cuda", "rocm", "vulkan"] {
            if !candidates.iter().any(|c| c == name) {
                if self.base_dir.join("runtime").join(name).exists() {
                    candidates.push(name.to_string());
                }
            }
        }
        candidates
    }

    /// バックエンドを試行し、(実行ファイルパス, 採用する URL) を返す。
    fn try_backend(
        &self,
        name: &str,
        model_path: &Path,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<(PathBuf, String)> {
        let runtime_dir = self.base_dir.join("runtime").join(name);

        // キャッシュ済み exe を優先試行する。
        // backend 名一致は不要 — exe が runtime/<name>/ 配下にあれば十分。
        // backend 名が違っても同一 artifact が別ディレクトリにある場合をカバーする。
        if let (Some(cached_path), Some(cached_url)) = (
            self.state.runtime_exe_path.as_ref(),
            self.state.backend_url.as_ref(),
        ) {
            let in_target_dir = cached_path
                .ancestors()
                .any(|a| a == runtime_dir.as_path());
            if cached_path.exists() && in_target_dir
                && self.test_backend_exe(cached_path, model_path).is_ok()
            {
                diag(progress_tx, &self.base_dir, &format!(
                    "[try_backend] reuse cached exe: {}", cached_path.display()
                ));
                return Ok((cached_path.clone(), cached_url.clone()));
            }
        }

        // orphan 救済: runtime/<other>/ にある完成済み artifact を runtime/<name>/ に移設する。
        if let Some(orphan_url) = self.rescue_orphan_runtime(name, &runtime_dir, progress_tx) {
            if let Some(exe_path) = find_llama_server_exe(&runtime_dir) {
                if self.test_backend_exe(&exe_path, model_path).is_ok() {
                    diag(progress_tx, &self.base_dir, &format!(
                        "[try_backend] rescued orphan runtime → {}", exe_path.display()
                    ));
                    return Ok((exe_path, orphan_url));
                }
            }
        }

        let used_url_opt = self.ensure_runtime(name, &runtime_dir, progress_tx, cancel_flag)?;
        let exe_path = find_llama_server_exe(&runtime_dir)
            .ok_or_else(|| anyhow!("llama-server not found in {}", runtime_dir.display()))?;

        self.test_backend_exe(&exe_path, model_path)?;

        let used_url = used_url_opt.unwrap_or_else(|| {
            self.state.backend_url.clone()
                .unwrap_or_else(|| self.config.runtime_urls.for_backend(name).unwrap().primary.clone())
        });
        Ok((exe_path, used_url))
    }

    /// runtime/<other>/ に `name` と同一 backend の完成済み artifact があれば
    /// runtime/<name>/ に移設（rename）して、採用した URL を返す。
    ///
    /// 移設条件:
    ///   - `runtime/<other>/` が `runtime_is_complete(other)` を満たす
    ///   - `state.backend_url` が `name` 用の authority URL（プライマリ / ベース一致）と一致
    ///   - `other != name`
    fn rescue_orphan_runtime(&self, name: &str, dest_dir: &Path, progress_tx: &Sender<LaunchProgress>) -> Option<String> {
        use crate::launcher::runtime_downloader::runtime_is_complete;

        let runtime_root = self.base_dir.join("runtime");
        let authority_url = self.config.runtime_urls.for_backend(name)
            .map(|a| url_base(&a.primary))?;

        let cached_url = self.state.backend_url.as_deref()?;
        if url_base(cached_url) != authority_url {
            return None;
        }

        let entries = std::fs::read_dir(&runtime_root).ok()?;
        for entry in entries.flatten() {
            let dir = entry.path();
            let other = dir.file_name()?.to_str()?;
            if other == name || !dir.is_dir() {
                continue;
            }
            if !runtime_is_complete(&dir, other) {
                continue;
            }
            // 同一 URL 由来なら移設する
            diag(progress_tx, &self.base_dir, &format!(
                "[rescue_orphan_runtime] moving runtime/{} → runtime/{}", other, name
            ));
            if std::fs::rename(&dir, dest_dir).is_ok() {
                return Some(cached_url.to_string());
            }
        }
        None
    }

    fn ensure_runtime(
        &self,
        name: &str,
        dest_dir: &Path,
        progress_tx: &Sender<LaunchProgress>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Option<String>> {
        let rt_ok = crate::launcher::runtime_downloader::runtime_is_complete(dest_dir, name);
        diag(progress_tx, &self.base_dir, &format!(
            "[ensure_runtime] backend={} rt_ok={}",
            name, rt_ok
        ));
        if rt_ok {
            return Ok(None);
        }

        progress_tx.send(LaunchProgress::Status(
            self.t(&format!("Downloading {} runtime...", name),
                   &format!("{} ランタイムをダウンロード中...", name))
        )).ok();

        let assets = self.config.runtime_urls.for_backend(name)
            .ok_or_else(|| anyhow!("Unsupported backend: {}", name))?;

        let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
        downloader.download_backend(
            name,
            &assets.primary,
            &assets.extra_assets,
            assets.fallback.as_deref(),
            dest_dir,
            |progress, _| {
                // ランタイムダウンロード単体の進捗（0.0～1.0）
                progress_tx.send(LaunchProgress::Progress(progress)).ok();
            },
        )
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
            s.batch_size,
            s.ubatch_size,
            s.cont_batching,
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

    /// profiles/default.toml と profiles/game.toml を shipped default で生成する。
    /// 既存ファイルは上書きしない。
    fn seed_profiles(&self) -> Result<()> {
        use super::translation_profile::TranslationProfile;
        let profiles_dir = self.base_dir.join("profiles");
        std::fs::create_dir_all(&profiles_dir)?;

        let default_path = profiles_dir.join("default.toml");
        if !default_path.exists() {
            TranslationProfile::default().save(&default_path)?;
        }

        let game_path = profiles_dir.join("game.toml");
        if !game_path.exists() {
            TranslationProfile::game_default().save(&game_path)?;
        }

        Ok(())
    }
}

/// check_ready 用：progress_tx なしでファイルだけに書く。
fn diag_file(base_dir: &Path, msg: &str) {
    let log_path = base_dir.join("launcher_debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
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

/// URL の ?query 部分を除いたベース文字列を返す。
fn url_base(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}
