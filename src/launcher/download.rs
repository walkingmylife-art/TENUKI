//! Download + verification logic for runtime and model.
//! No egui types allowed.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};

use super::app_config::AppConfig;
use super::app_config::ModelConfig;
use super::progress::LaunchProgress;
use super::runtime_downloader::{find_llama_server_exe, RuntimeDownloader};

fn t(ui_lang: &str, en: &str, ja: &str) -> String {
    if ui_lang == "en" { en.to_string() } else { ja.to_string() }
}

pub fn diag(progress_tx: &Sender<LaunchProgress>, base_dir: &Path, msg: &str) {
    progress_tx.send(LaunchProgress::SubStatus(msg.to_string())).ok();
    let log_path = base_dir.join("launcher_debug.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn diag_file(base_dir: &Path, msg: &str) {
    let log_path = base_dir.join("launcher_debug.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn model_is_complete(model_path: &Path, expected_size: u64) -> bool {
    if expected_size == 0 {
        return false;
    }
    let actual = match fs::metadata(model_path) {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    if actual != expected_size {
        return false;
    }
    cleanup_model_resume_artifacts(model_path);
    true
}

fn cleanup_model_resume_artifacts(model_path: &Path) {
    let sidecar = PathBuf::from(format!("{}.sidecar.json", model_path.display()));
    if sidecar.exists() {
        let _ = fs::remove_file(&sidecar);
    }
    let part = model_path.with_extension("part");
    if part.exists() {
        let _ = fs::remove_file(&part);
    }
}

pub fn check_ready_detail(base_dir: &Path) -> Result<(), super::CheckReadyReason> {
    use super::CheckReadyReason;
    use super::runtime_downloader::runtime_is_complete;

    let install_root = super::resolve_install_root();
    let config_path = install_root.join("launcher_config.toml");
    let config = match AppConfig::load(&config_path) {
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
    diag_file(base_dir, &format!("[check_ready] runtime_is_complete backend={} rt_ok={}", backend, rt_ok));
    if !rt_ok {
        return Err(CheckReadyReason::RuntimeIncomplete { backend: backend.clone() });
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

    let actual_size = fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0);
    diag_file(base_dir, &format!(
        "[check_ready] model filename={} kind={} expected={} actual={}",
        filename,
        if is_known { "Known" } else { "Local" },
        expected_size,
        actual_size
    ));

    if actual_size == expected_size && expected_size > 0 {
        cleanup_model_resume_artifacts(&model_path);
        diag_file(base_dir, "[check_ready] true (authority model matched)");
        return Ok(());
    }

    if crate::backend::resolve_startup_model(&config, &available_models).is_some() {
        diag_file(base_dir, &format!(
            "[check_ready] authority model unavailable, but startup model resolved from {} candidate(s)",
            available_models.len()
        ));
        return Ok(());
    }
    if available_models.len() > 1 {
        diag_file(base_dir, &format!(
            "[check_ready] {} candidates exist but startup model unresolvable",
            available_models.len()
        ));
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
            Err(CheckReadyReason::ModelSizeMismatch { filename, expected: expected_size, actual: actual_size })
        } else {
            Err(CheckReadyReason::LocalModelChanged { filename, expected: expected_size, actual: actual_size })
        };
    }

    cleanup_model_resume_artifacts(&model_path);
    diag_file(base_dir, "[check_ready] → true");
    Ok(())
}

pub fn check_ready(base_dir: &Path) -> bool {
    check_ready_detail(base_dir).is_ok()
}

pub fn ensure_model(
    base_dir: &Path,
    model: &ModelConfig,
    ui_lang: &str,
    progress_tx: &Sender<LaunchProgress>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<PathBuf> {
    let expected = model.expected_size();
    let filename = model.filename().to_string();
    if expected == 0 {
        diag(progress_tx, base_dir, "[ensure_model] ERROR: expected_size == 0 in launcher_config.toml");
        anyhow::bail!("model.expected_size is 0 in launcher_config.toml. Set it to the correct file size in bytes.");
    }

    let model_dir = base_dir.join("models");
    let model_path = model_dir.join(&filename);

    let complete = model_is_complete(&model_path, expected);
    diag(progress_tx, base_dir, &format!(
        "[ensure_model] filename={} kind={} exists={} expected_size={} complete={}",
        filename,
        if model.is_known() { "Known" } else { "Local" },
        model_path.exists(),
        expected,
        complete
    ));
    if complete {
        diag(progress_tx, base_dir, "[ensure_model] → reuse existing model");
        return Ok(model_path);
    }

    let alternative = fs::read_dir(&model_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_gguf = path.extension().map(|e| e == "gguf").unwrap_or(false);
            let is_other = path.file_name().and_then(|n| n.to_str()) != Some(filename.as_str());
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if is_gguf && is_other && size > 0 { Some(path) } else { None }
        })
        .next();
    if let Some(alt_path) = alternative {
        diag(progress_tx, base_dir, &format!(
            "[ensure_model] authority incomplete, using alternative: {}", alt_path.display()
        ));
        return Ok(alt_path);
    }

    if let ModelConfig::Local { .. } = model {
        diag(progress_tx, base_dir, &format!("[ensure_model] Local model missing, cannot download: {}", filename));
        anyhow::bail!("Local model '{}' not found in models/. Restore the file or select another model.", filename);
    }

    let ModelConfig::Known { urls, .. } = model else { unreachable!() };

    if model_path.exists() {
        let sidecar = PathBuf::from(format!("{}.sidecar.json", model_path.display()));
        if !sidecar.exists() {
            diag(progress_tx, base_dir, "[ensure_model] incomplete (no sidecar, bad size) → remove and re-download");
            fs::remove_file(&model_path)?;
        }
    }

    progress_tx.send(LaunchProgress::Status(t(ui_lang, "Downloading model...", "モデルをダウンロード中..."))).ok();
    progress_tx.send(LaunchProgress::SubStatus(t(ui_lang, &format!("Fetching: {}", filename), &format!("取得中: {}", filename)))).ok();

    let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
    let _used_url_opt = downloader.download_model(
        &urls.primary,
        urls.fallback.as_deref(),
        &model_path,
        expected,
        |progress, _| {
            progress_tx.send(LaunchProgress::Progress(progress)).ok();
        },
    )?;

    Ok(model_path)
}

pub fn ensure_runtime(
    base_dir: &Path,
    config: &AppConfig,
    name: &str,
    dest_dir: &Path,
    ui_lang: &str,
    progress_tx: &Sender<LaunchProgress>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<()> {
    let rt_ok = super::runtime_downloader::runtime_is_complete(dest_dir, name);
    diag(progress_tx, base_dir, &format!("[ensure_runtime] backend={} rt_ok={}", name, rt_ok));
    if rt_ok {
        return Ok(());
    }

    progress_tx.send(LaunchProgress::Status(t(ui_lang,
        &format!("Downloading {} runtime...", name),
        &format!("{} ランタイムをダウンロード中...", name),
    ))).ok();

    let assets = config.runtime_urls.for_backend(name)
        .ok_or_else(|| anyhow::anyhow!("Unsupported backend: {}", name))?;

    let downloader = RuntimeDownloader::with_cancel_flag(cancel_flag.clone())?;
    let _used_url = downloader.download_backend(
        name, &assets.primary, &assets.extra_assets, assets.fallback.as_deref(),
        dest_dir,
        |progress, _| { progress_tx.send(LaunchProgress::Progress(progress)).ok(); },
    )?;
    Ok(())
}

use crate::launcher::app_launcher::BackendCandidate;

/// 空きポートを動的に取得する（127.0.0.1 固定）
pub fn find_free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

pub fn try_backend(
    base_dir: &Path,
    config: &AppConfig,
    http_client: &reqwest::blocking::Client,
    ui_lang: &str,
    candidate: &BackendCandidate,
    model_path: &Path,
    progress_tx: &Sender<LaunchProgress>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<super::VerifiedBackendCandidate> {
    let name = &candidate.name;
    let runtime_dir = base_dir.join("runtime").join(name);

    ensure_runtime(base_dir, config, name, &runtime_dir, ui_lang, progress_tx, cancel_flag)?;
    let exe_path = find_llama_server_exe(&runtime_dir)
        .ok_or_else(|| anyhow::anyhow!("llama-server not found in {}", runtime_dir.display()))?;
    test_backend_exe(http_client, config, &exe_path, model_path, base_dir)?;

    diag(progress_tx, base_dir, &format!(
        "[try_backend] verified backend={} reason={} exe={}",
        name, candidate.reason.as_str(), exe_path.display()
    ));

    Ok(super::VerifiedBackendCandidate { candidate: candidate.clone(), exe_path })
}

/// バックエンド実行ファイルの検証。動的ポートを使用。
pub fn test_backend_exe(
    http_client: &reqwest::blocking::Client,
    config: &AppConfig,
    exe_path: &Path,
    model_path: &Path,
    base_dir: &Path,
) -> Result<()> {
    let test_port = find_free_port()?;
    let s = &config.server;
    let mut cmd = crate::backend::process::build_llama_command(
        exe_path, model_path, test_port, s.ngl, s.ctx_size, s.parallel_slots, &s.extra_args,
    );

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn backend for verification")?;

    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for _ in BufReader::new(stderr).lines() {}
        });
    }

    let result = wait_for_healthy_process(
        &mut child,
        http_client,
        test_port,
        std::time::Duration::from_secs(120),
    );
    let _ = child.kill();
    let _ = child.wait();
    if result.is_ok() {
        diag_file(base_dir, &format!("[test_backend_exe] port={} exe={}", test_port, exe_path.display()));
    }
    result
}

fn wait_for_healthy_process(
    child: &mut std::process::Child,
    http_client: &reqwest::blocking::Client,
    port: u16,
    timeout: std::time::Duration,
) -> Result<()> {
    let start = std::time::Instant::now();
    let interval = std::time::Duration::from_millis(500);
    loop {
        match child.try_wait()? {
            Some(status) => anyhow::bail!("llama-server exited early: {}", status),
            None => {
                if check_health(http_client, port).is_ok() {
                    return Ok(());
                }
                if start.elapsed() > timeout {
                    anyhow::bail!("Health check timeout");
                }
            }
        }
        std::thread::sleep(interval);
    }
}

fn check_health(http_client: &reqwest::blocking::Client, port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/health", port);
    let resp = http_client.get(&url).send()?;
    if resp.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("Health endpoint returned {}", resp.status())
    }
}
