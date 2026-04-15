//! src/launcher/runtime_downloader.rs
//!
//! バックエンドランタイムとモデルのダウンロード・展開
//! - HTTPS + Range レジューム対応（サーバ非対応時は自動で再ダウンロード）
//! - ZIP 展開（構造維持、パストラバーサル対策済み）
//! - キャンセルフラグ対応
//! - ダウンロード完全性チェック
//! - 進捗コールバック対応

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use walkdir::WalkDir;
use zip::ZipArchive;

pub struct RuntimeDownloader {
    client: Client,
    cancel_flag: Arc<AtomicBool>,
}

impl RuntimeDownloader {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .user_agent("TENUKI-Launcher/1.0")
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            client,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn with_cancel_flag(cancel_flag: Arc<AtomicBool>) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .user_agent("TENUKI-Launcher/1.0")
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            client,
            cancel_flag,
        })
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Download cancelled");
        }
        Ok(())
    }

    /// HuggingFace URL からファイルサイズを取得する
    pub fn fetch_huggingface_size(url: &str) -> Result<u64> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("TENUKI-Launcher/1.0")
            .build()
            .context("Failed to build HTTP client")?;

        // HEADリクエストでサイズを取得
        let response = client.head(url).send()
            .with_context(|| format!("HEAD request failed for {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch size: HTTP {}", response.status());
        }

        response.content_length()
            .ok_or_else(|| anyhow!("Content-Length header not found"))
    }

    /// バックエンドランタイムをダウンロードし、指定ディレクトリに展開する
    pub fn download_backend(
        &self,
        url: &str,
        dest_dir: &Path,
        progress_callback: impl Fn(f32, &str) + Send + Sync,
    ) -> Result<()> {
        self.check_cancel()?;
        fs::create_dir_all(dest_dir)?;

        if find_llama_server_exe(dest_dir).is_some() {
            log::info!("Runtime already exists in {}", dest_dir.display());
            progress_callback(1.0, "Using existing runtime");
            return Ok(());
        }

        let zip_name = url
            .split('/')
            .last()
            .ok_or_else(|| anyhow!("Invalid URL"))?;
        let zip_path = dest_dir.join(zip_name);

        // ダウンロード: 全体進捗の 0.0〜0.8
        // 展開:         全体進捗の 0.8〜1.0
        const DOWNLOAD_END: f32 = 0.8;

        log::info!("Downloading runtime from {}", url);
        self.download_file(url, &zip_path, None, |progress, _total| {
            progress_callback(progress * DOWNLOAD_END, "");
        })?;

        // ダウンロード完了時点でバーを 0.8 に固定してから展開開始
        progress_callback(DOWNLOAD_END, "");

        log::info!("Extracting runtime to {}", dest_dir.display());
        let extract_result = self.extract_zip(&zip_path, dest_dir, |progress, _total| {
            progress_callback(DOWNLOAD_END + progress * (1.0 - DOWNLOAD_END), "");
        });

        // 展開に失敗した場合もZIPを削除試行
        let _ = fs::remove_file(&zip_path);
        extract_result?;

        if find_llama_server_exe(dest_dir).is_none() {
            anyhow::bail!("llama-server executable not found after extraction");
        }

        progress_callback(1.0, "Extraction complete");
        Ok(())
    }

    /// モデルファイルをダウンロードする（コールバック付き）
    pub fn download_model(
        &self,
        url: &str,
        dest_path: &Path,
        expected_size: Option<u64>,
        progress_callback: impl Fn(f32, &str) + Send + Sync,
    ) -> Result<()> {
        self.check_cancel()?;

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if dest_path.exists() {
            let metadata = fs::metadata(dest_path)?;
            if let Some(expected) = expected_size {
                if metadata.len() == expected {
                    log::info!("Model already exists with correct size: {}", dest_path.display());
                    progress_callback(1.0, "");
                    return Ok(());
                }
            } else if metadata.len() >= 10 * 1024 * 1024 {
                log::info!("Model already exists (min size satisfied): {}", dest_path.display());
                progress_callback(1.0, "");
                return Ok(());
            }
            fs::remove_file(dest_path)?;
        }

        log::info!("Downloading model from {}", url);
        self.download_file(url, dest_path, expected_size, |progress, _total| {
            progress_callback(progress, "");
        })?;
        Ok(())
    }

    // ---------- 内部ヘルパー ----------
    fn download_file(
        &self,
        url: &str,
        dest_path: &Path,
        expected_size: Option<u64>,
        progress_callback: impl Fn(f32, &str) + Send + Sync,
    ) -> Result<()> {
        // expected_size = 0 は「未指定」を意味するため、無効化する
        let expected_size = expected_size.filter(|&size| size > 0);
        
        let tmp_path = dest_path.with_extension("part");

        // ダウンロード中にキャンセルされた場合やエラーが発生した場合に .part を削除するためのヘルパー
        let result = (|| {
            self.check_cancel()?;
            let (file, downloaded_before) = if tmp_path.exists() {
                let size = fs::metadata(&tmp_path)?.len();
                let f = OpenOptions::new()
                    .write(true)
                    .append(true)
                    .open(&tmp_path)
                    .with_context(|| format!("Failed to open partial file: {}", tmp_path.display()))?;
                (f, size)
            } else {
                let f = File::create(&tmp_path)
                    .with_context(|| format!("Failed to create temporary file: {}", tmp_path.display()))?;
                (f, 0)
            };

            let mut request = self.client.get(url);
            if downloaded_before > 0 {
                request = request.header("Range", format!("bytes={}-", downloaded_before));
            }

            let mut response = request.send()?;
            let status = response.status();
            let (mut file, mut downloaded) = if downloaded_before > 0 && status.as_u16() == 200 {
                log::warn!("Server does not support Range requests, restarting download from beginning");
                let f = File::create(&tmp_path)?;
                (f, 0u64)
            } else {
                (file, downloaded_before)
            };

            if !status.is_success() && status.as_u16() != 206 {
                anyhow::bail!("HTTP error: {}", status);
            }

            let total_size = response
                .content_length()
                .map(|len| downloaded + len)
                .or_else(|| expected_size);

            let mut buffer = [0u8; 65536];
            loop {
                self.check_cancel()?;
                let bytes_read = response.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                file.write_all(&buffer[..bytes_read])?;
                downloaded += bytes_read as u64;

                if let Some(total) = total_size {
                    if total > 0 {
                        let progress = downloaded as f32 / total as f32;
                        progress_callback(progress, "");
                    }
                }
            }

            if let Some(total) = total_size {
                if downloaded != total {
                    anyhow::bail!(
                        "Incomplete download: expected {} bytes, got {} bytes",
                        total,
                        downloaded
                    );
                }
            }
            if let Some(expected) = expected_size {
                if downloaded != expected {
                    anyhow::bail!(
                        "Downloaded size {} does not match expected size {}",
                        downloaded,
                        expected
                    );
                }
            }

            file.sync_all()?;
            fs::rename(&tmp_path, dest_path)?;
            progress_callback(1.0, "");
            Ok(())
        })();

        if let Err(e) = result {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
        Ok(())
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
            let entry_name = entry.name();

            let out_path = safe_join(dest_dir, entry_name)?;

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

            let progress = (i + 1) as f32 / total as f32;
            progress_callback(progress, "");
        }
        Ok(())
    }
}

impl Default for RuntimeDownloader {
    fn default() -> Self {
        Self::new().expect("Failed to create RuntimeDownloader")
    }
}

// ---------- ユーティリティ関数 ----------
fn safe_join(base: &Path, entry_name: &str) -> Result<PathBuf> {
    // パス区切り文字の正規化（Windows対応）
    let normalized_entry = entry_name.replace('\\', "/");
    
    // パストラバーサル攻撃の防止: ".." を含むエントリを拒否
    if normalized_entry.contains("..") {
        anyhow::bail!("Zip entry contains path traversal: {}", entry_name);
    }
    
    // ベースディレクトリを正規化（まずディレクトリが存在することを確認）
    let canonical_base = if base.exists() {
        base.canonicalize()
            .unwrap_or_else(|_| base.to_path_buf())
    } else {
        base.to_path_buf()
    };
    
    let out_path = base.join(&normalized_entry);
    
    // 出力先のパスを正規化（ファイルがまだ存在しない場合は親ディレクトリでチェック）
    let canonical_out = if out_path.exists() {
        out_path.canonicalize()
            .unwrap_or_else(|_| out_path.to_path_buf())
    } else if let Some(parent) = out_path.parent() {
        if parent.exists() {
            parent.canonicalize()
                .unwrap_or_else(|_| out_path.to_path_buf())
        } else {
            out_path.to_path_buf()
        }
    } else {
        out_path.to_path_buf()
    };
    
    // パストラバーサルチェック
    if !canonical_out.starts_with(&canonical_base) {
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
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy() == name)
        .map(|e| e.path().to_path_buf())
}