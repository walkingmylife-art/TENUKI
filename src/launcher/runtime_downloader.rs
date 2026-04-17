//! src/launcher/runtime_downloader.rs
//!
//! 再開容易性と完走性を最適化したダウンローダ。
//! - 固定チャンク (16MB) を安全再開境界として使用する。
//! - .part ファイルは決して削除せず、常に confirmed_bytes まで truncate してから追記する。
//! - 再開状態は sidecar (.sidecar.json) に永続化する。
//! - 区間失敗回数 (chunk_fail_count) が上限に達した場合のみ fallback URL へ切り替える。
//! - expected_size == 0 (runtime zip 用): EOF 到着をもってダウンロード完了とみなす。
//!   モデルダウンロードでは download_model() を使うこと。expected_size == 0 は拒否される。

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use walkdir::WalkDir;
use zip::ZipArchive;

const CHUNK_SIZE: u64 = 16 * 1024 * 1024;
const MAX_CHUNK_FAILS: u32 = 3;
const RETRY_WAIT_SECS: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadSidecar {
    primary_url: String,
    fallback_url: Option<String>,
    current_url: String,
    expected_size: u64,
    confirmed_bytes: u64,
    chunk_fail_count: u32,
    current_chunk_start: u64,
}

fn sidecar_path(dest_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.sidecar.json", dest_path.display()))
}

fn cleanup_model_resume_artifacts(dest_path: &Path) {
    let sc_path = sidecar_path(dest_path);
    if sc_path.exists() {
        let _ = fs::remove_file(&sc_path);
    }
    let part_path = dest_path.with_extension("part");
    if part_path.exists() {
        let _ = fs::remove_file(&part_path);
    }
}

fn load_sidecar(path: &Path) -> Option<DownloadSidecar> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_sidecar(path: &Path, sidecar: &DownloadSidecar) -> Result<()> {
    let content = serde_json::to_string_pretty(sidecar).context("Failed to serialize sidecar")?;
    fs::write(path, content)?;
    Ok(())
}

struct FetchResult {
    bytes_written: u64,
    revealed_total: Option<u64>,
    is_eof: bool,
}

pub struct RuntimeDownloader {
    client: Client,
    cancel_flag: Arc<AtomicBool>,
}

impl RuntimeDownloader {
    pub fn with_cancel_flag(cancel_flag: Arc<AtomicBool>) -> Result<Self> {
        Self::build(cancel_flag)
    }

    fn build(cancel_flag: Arc<AtomicBool>) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .user_agent("TENUKI-Launcher/1.0")
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            client,
            cancel_flag,
        })
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Download cancelled");
        }
        Ok(())
    }

    pub fn download_backend(
        &self,
        backend: &str,
        primary_url: &str,
        extra_asset_urls: &[String],
        fallback_url: Option<&str>,
        dest_dir: &Path,
        progress_callback: impl Fn(f32, &str) + Send + Sync,
    ) -> Result<Option<String>> {
        self.check_cancel()?;
        fs::create_dir_all(dest_dir)?;

        if runtime_is_complete(dest_dir, backend) {
            log::info!(
                "Runtime already complete ({}): {}",
                backend,
                dest_dir.display()
            );
            return Ok(None);
        }

        let mut used_url: Option<String> = None;

        let mut asset_urls = Vec::with_capacity(1 + extra_asset_urls.len());
        asset_urls.push(primary_url.to_string());
        asset_urls.extend(extra_asset_urls.iter().cloned());
        let total_units = (asset_urls.len() * 2).max(1) as f32;

        for (index, asset_url) in asset_urls.iter().enumerate() {
            let zip_name = asset_url
                .split('/')
                .last()
                .and_then(|s| s.split('?').next())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("Cannot extract filename from URL: {}", asset_url))?;
            let zip_path = dest_dir.join(zip_name);
            let download_unit = (index * 2) as f32;
            let extract_unit = download_unit + 1.0;

            progress_callback(
                download_unit / total_units,
                &format!("取得中 {}/{}: {}", index + 1, asset_urls.len(), zip_name),
            );

            // fallback は primary asset にのみ適用する
            // extra_assets は補助 DLL 等であり、独自の fallback は持たない
            let fb = if index == 0 { fallback_url } else { None };

            let current_used =
                self.download_file(asset_url, fb, &zip_path, None, &|progress, status| {
                    let combined = (download_unit + progress.clamp(0.0, 1.0)) / total_units;
                    progress_callback(combined, status);
                })?;

            if used_url.is_none() {
                used_url = Some(current_used);
            }

            log::info!(
                "Extracting asset {}/{}: {}",
                index + 1,
                asset_urls.len(),
                zip_name
            );
            self.extract_zip(&zip_path, dest_dir, |p, _| {
                let combined = (extract_unit + p.clamp(0.0, 1.0)) / total_units;
                progress_callback(
                    combined,
                    &format!(
                        "展開中 {}/{}: {:.1}%",
                        index + 1,
                        asset_urls.len(),
                        p * 100.0
                    ),
                );
            })?;

            let _ = fs::remove_file(&zip_path);
        }

        if find_llama_server_exe(dest_dir).is_none() {
            anyhow::bail!(
                "llama-server not found after extraction: {}",
                dest_dir.display()
            );
        }

        Ok(used_url)
    }

    pub fn download_model(
        &self,
        primary_url: &str,
        fallback_url: Option<&str>,
        dest_path: &Path,
        expected_size: u64,
        progress_callback: impl Fn(f32, &str) + Send + Sync,
    ) -> Result<Option<String>> {
        if expected_size == 0 {
            anyhow::bail!(
                "download_model called with expected_size=0. \
                 Set model.expected_size in launcher_config.toml."
            );
        }
        self.check_cancel()?;

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if dest_path.exists() {
            let size = fs::metadata(dest_path)?.len();
            if size == expected_size {
                // サイズ一致 = 完成。sidecar が残っていても stale なので掃除して終了。
                cleanup_model_resume_artifacts(dest_path);
                log::info!("Model already complete: {}", dest_path.display());
                return Ok(None);
            }
            let sc_path = sidecar_path(dest_path);
            if sc_path.exists() {
                // sidecar あり・サイズ不一致 = 中断状態。再開する。
                log::info!(
                    "Model download incomplete (sidecar present), resuming: {}",
                    dest_path.display()
                );
            } else {
                // sidecar なし・サイズ不一致 = 壊れたファイル。削除して再取得。
                log::warn!(
                    "Model size mismatch (got {}, expected {}), re-downloading: {}",
                    size,
                    expected_size,
                    dest_path.display()
                );
                fs::remove_file(dest_path)?;
            }
        }

        log::info!("Downloading model: {}", primary_url);
        let used_url = self.download_file(
            primary_url,
            fallback_url,
            dest_path,
            Some(expected_size),
            &progress_callback,
        )?;
        Ok(Some(used_url))
    }

    fn download_file(
        &self,
        primary_url: &str,
        fallback_url: Option<&str>,
        dest_path: &Path,
        expected_size: Option<u64>,
        progress_callback: &(impl Fn(f32, &str) + Send + Sync),
    ) -> Result<String> {
        let part_path = dest_path.with_extension("part");
        let sc_path = sidecar_path(dest_path);

        let mut sc = if let Some(existing) = load_sidecar(&sc_path) {
            log::info!(
                "Resuming from byte {} (chunk start {}) via {} (fails={})",
                existing.confirmed_bytes,
                existing.current_chunk_start,
                existing.current_url,
                existing.chunk_fail_count
            );
            existing
        } else {
            let new_sc = DownloadSidecar {
                primary_url: primary_url.to_string(),
                fallback_url: fallback_url.map(|s| s.to_string()),
                current_url: primary_url.to_string(),
                expected_size: expected_size.unwrap_or(0),
                confirmed_bytes: 0,
                chunk_fail_count: 0,
                current_chunk_start: 0,
            };
            save_sidecar(&sc_path, &new_sc)?;
            new_sc
        };

        sc.primary_url = primary_url.to_string();
        sc.fallback_url = fallback_url.map(|s| s.to_string());
        if let Some(expected) = expected_size {
            sc.expected_size = expected;
        }
        if sc.current_url.is_empty() {
            sc.current_url = sc.primary_url.clone();
        }
        save_sidecar(&sc_path, &sc)?;

        truncate_part_file(&part_path, sc.confirmed_bytes)?;

        loop {
            self.check_cancel()?;

            let offset = sc.confirmed_bytes;
            sc.current_chunk_start = offset;
            save_sidecar(&sc_path, &sc)?;

            let total_opt = if sc.expected_size > 0 {
                Some(sc.expected_size)
            } else {
                None
            };

            match self.fetch_chunk(&sc.current_url, offset, total_opt, &part_path) {
                Ok(fr) => {
                    let chunk_completed = fr.is_eof || fr.bytes_written == CHUNK_SIZE;

                    if sc.expected_size == 0 {
                        if let Some(t) = fr.revealed_total {
                            sc.expected_size = t;
                        }
                    }

                    if chunk_completed {
                        // 区間完全取得成功
                        sc.confirmed_bytes += fr.bytes_written;
                        sc.chunk_fail_count = 0;
                        save_sidecar(&sc_path, &sc)?;

                        if sc.expected_size > 0 {
                            let p = (sc.confirmed_bytes as f32 / sc.expected_size as f32).min(1.0);
                            progress_callback(p, "");
                        }

                        let reached_expected =
                            sc.expected_size > 0 && sc.confirmed_bytes >= sc.expected_size;
                        let done = if sc.expected_size > 0 {
                            reached_expected
                        } else {
                            fr.is_eof
                        };
                        if done {
                            break;
                        }
                        if fr.is_eof && sc.expected_size > 0 {
                            anyhow::bail!(
                                "Download ended early at {} bytes; expected {} bytes",
                                sc.confirmed_bytes,
                                sc.expected_size
                            );
                        }
                    } else {
                        // 部分書き込み・非EOF → 同一区間の失敗として扱う
                        sc.chunk_fail_count += 1;
                        save_sidecar(&sc_path, &sc)?;

                        log::warn!(
                            "Chunk incomplete {}/{} at byte {} (chunk start {}): wrote {} bytes",
                            sc.chunk_fail_count,
                            MAX_CHUNK_FAILS,
                            offset,
                            sc.current_chunk_start,
                            fr.bytes_written
                        );

                        // ファイルを安全境界まで切り戻す
                        truncate_part_file(&part_path, sc.confirmed_bytes)?;

                        if sc.chunk_fail_count >= MAX_CHUNK_FAILS {
                            if sc.current_url == sc.primary_url {
                                if let Some(fb) = &sc.fallback_url {
                                    log::warn!("Switching to fallback URL: {}", fb);
                                    sc.current_url = fb.clone();
                                    sc.chunk_fail_count = 0;
                                    save_sidecar(&sc_path, &sc)?;
                                    continue;
                                }
                            }
                            anyhow::bail!(
                                "Both primary and fallback URLs failed at byte {}. \
                                 Download incomplete. Resume will be attempted on next launch.",
                                offset
                            );
                        } else {
                            thread::sleep(Duration::from_secs(RETRY_WAIT_SECS));
                            // 次のループ先頭で truncate されるが、ここでも明示的に切り詰める
                            truncate_part_file(&part_path, sc.confirmed_bytes)?;
                            continue;
                        }
                    }
                }
                Err(e) => {
                    sc.chunk_fail_count += 1;
                    save_sidecar(&sc_path, &sc)?;
                    log::warn!(
                        "Chunk fail {}/{} at byte {} (chunk start {}): {}",
                        sc.chunk_fail_count,
                        MAX_CHUNK_FAILS,
                        offset,
                        sc.current_chunk_start,
                        e
                    );

                    truncate_part_file(&part_path, sc.confirmed_bytes)?;

                    if sc.chunk_fail_count >= MAX_CHUNK_FAILS {
                        if sc.current_url == sc.primary_url {
                            if let Some(fb) = &sc.fallback_url {
                                log::warn!("Switching to fallback URL: {}", fb);
                                sc.current_url = fb.clone();
                                sc.chunk_fail_count = 0;
                                save_sidecar(&sc_path, &sc)?;
                                continue;
                            }
                        }
                        anyhow::bail!(
                            "Both primary and fallback URLs failed at byte {}. \
                             Download incomplete. Resume will be attempted on next launch.",
                            offset
                        );
                    } else {
                        thread::sleep(Duration::from_secs(RETRY_WAIT_SECS));
                        truncate_part_file(&part_path, sc.confirmed_bytes)?;
                    }
                }
            }
        }

        if sc.expected_size > 0 && sc.confirmed_bytes != sc.expected_size {
            anyhow::bail!(
                "Downloaded size {} does not match expected size {}",
                sc.confirmed_bytes,
                sc.expected_size
            );
        }
        fs::rename(&part_path, dest_path)?;
        let used_url = sc.current_url.clone();
        let _ = fs::remove_file(&sc_path);
        progress_callback(1.0, "");
        Ok(used_url)
    }

    fn fetch_chunk(
        &self,
        url: &str,
        offset: u64,
        total_size: Option<u64>,
        part_path: &Path,
    ) -> Result<FetchResult> {
        let range_header = match total_size {
            Some(total) => {
                let end = (offset + CHUNK_SIZE - 1).min(total.saturating_sub(1));
                format!("bytes={}-{}", offset, end)
            }
            None => format!("bytes={}-", offset),
        };

        let response = self
            .client
            .get(url)
            .header("Range", &range_header)
            .send()
            .with_context(|| format!("Connection failed: {}", url))?;

        let status = response.status();
        if offset > 0 && status.as_u16() == 200 {
            anyhow::bail!("Server does not support Range requests (returned 200, not 206)");
        }
        if !status.is_success() && status.as_u16() != 206 {
            anyhow::bail!("HTTP {} from {}", status, url);
        }

        let revealed_total = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rfind('/').and_then(|i| v[i + 1..].trim().parse().ok()));

        truncate_part_file(part_path, offset)?;

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(part_path)?;

        let mut reader = response;
        let mut buf = [0u8; 65536];
        let mut bytes_written: u64 = 0;

        loop {
            self.check_cancel()?;
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            bytes_written += n as u64;
            if bytes_written >= CHUNK_SIZE {
                break;
            }
        }
        file.sync_all()?;
        let is_eof = bytes_written < CHUNK_SIZE;
        Ok(FetchResult {
            bytes_written,
            revealed_total,
            is_eof,
        })
    }

    fn extract_zip(
        &self,
        zip_path: &Path,
        dest_dir: &Path,
        progress_callback: impl Fn(f32, &str) + Send + Sync,
    ) -> Result<()> {
        let file = File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        let total = archive.len();

        for i in 0..total {
            self.check_cancel()?;
            let mut entry = archive.by_index(i)?;
            let out_path = safe_join(dest_dir, entry.name())?;

            if entry.is_dir() {
                fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out_file = File::create(&out_path)?;
                std::io::copy(&mut entry, &mut out_file)?;
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
                }
            }

            progress_callback((i + 1) as f32 / total as f32, "");
        }
        Ok(())
    }
}

fn truncate_part_file(part_path: &Path, len: u64) -> Result<()> {
    if part_path.exists() {
        let f = OpenOptions::new().write(true).open(part_path)?;
        f.set_len(len)?;
    }
    Ok(())
}

fn safe_join(base: &Path, entry_name: &str) -> Result<PathBuf> {
    let out_path = base.join(entry_name);
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let parent = out_path.parent().unwrap_or(base);
    let canonical_parent = if parent.exists() {
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
    } else {
        let mut existing = parent;
        while !existing.exists() {
            match existing.parent() {
                Some(p) => existing = p,
                None => break,
            }
        }
        let ec = existing
            .canonicalize()
            .unwrap_or_else(|_| existing.to_path_buf());
        let rel = parent.strip_prefix(existing).unwrap_or(Path::new(""));
        ec.join(rel)
    };
    let final_path = match out_path.file_name() {
        Some(name) => canonical_parent.join(name),
        None => canonical_parent,
    };
    if !final_path.starts_with(&canonical_base) {
        anyhow::bail!("Zip entry escapes destination: {}", entry_name);
    }
    Ok(out_path)
}

pub fn find_llama_server_exe(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    WalkDir::new(dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy() == name)
        .map(|e| e.path().to_path_buf())
}

/// バックエンドのインストールが完了しているか確認する。
///
/// - Vulkan / ROCm: llama-server.exe が存在すれば完了
/// - CUDA: llama-server.exe に加えて *.dll が 1 件以上必要
///   (cudart-llama zip の展開確認。DLL なしでは CUDA 推論が動かない)
pub fn runtime_is_complete(dir: &Path, backend: &str) -> bool {
    if find_llama_server_exe(dir).is_none() {
        return false;
    }
    if backend == "cuda" {
        // cudart asset から展開された DLL が少なくとも 1 件あること
        let has_dll = WalkDir::new(dir)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_type().is_file()
                    && e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("dll"))
                        .unwrap_or(false)
            });
        if !has_dll {
            log::warn!(
                "CUDA runtime incomplete: llama-server.exe found but no DLL in {}",
                dir.display()
            );
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_rt_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tenuki_rt_{}", tag));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const EXE: &str = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    // --- vulkan: exe のみで complete ---

    #[test]
    fn vulkan_complete_with_exe_only() {
        let dir = temp_rt_dir("vk_complete");
        fs::write(dir.join(EXE), b"").unwrap();

        assert!(runtime_is_complete(&dir, "vulkan"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn vulkan_incomplete_without_exe() {
        let dir = temp_rt_dir("vk_no_exe");

        assert!(!runtime_is_complete(&dir, "vulkan"));
        let _ = fs::remove_dir_all(&dir);
    }

    // --- cuda: exe だけでは incomplete ---

    #[test]
    fn cuda_incomplete_exe_only_no_dll() {
        let dir = temp_rt_dir("cuda_no_dll");
        fs::write(dir.join(EXE), b"").unwrap();

        assert!(!runtime_is_complete(&dir, "cuda"));
        let _ = fs::remove_dir_all(&dir);
    }

    // --- cuda: exe + dll 1個以上で complete ---

    #[test]
    fn cuda_complete_with_exe_and_dll() {
        let dir = temp_rt_dir("cuda_with_dll");
        fs::write(dir.join(EXE), b"").unwrap();
        fs::write(dir.join("nvcuda.dll"), b"").unwrap();

        assert!(runtime_is_complete(&dir, "cuda"));
        let _ = fs::remove_dir_all(&dir);
    }
}
