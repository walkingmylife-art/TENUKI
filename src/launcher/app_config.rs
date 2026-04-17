// src/launcher/app_config.rs
//
// 役割：権威設定の唯一のソース。
// - このファイルに記述された内容のみが真実であり、他のファイルで上書きされない。
// - 旧形式からの移行は model.url のみ対応。runtime 側の旧 URL 互換は取らない。
// - ServerConfig.port は本番用であり、検証（verify）では絶対に使用しない。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlPair {
    pub primary: String,
    /// fallback が存在しない場合は None（空文字列や空白文字列も None に正規化される）
    pub fallback: Option<String>,
}

impl UrlPair {
    pub fn single(url: impl Into<String>) -> Self {
        Self {
            primary: url.into(),
            fallback: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAssetSet {
    pub primary: String,
    #[serde(default)]
    pub extra_assets: Vec<String>,
    /// fallback が存在しない場合は None（空文字列や空白文字列も None に正規化される）
    pub fallback: Option<String>,
}

impl RuntimeAssetSet {
    pub fn single(url: impl Into<String>) -> Self {
        Self {
            primary: url.into(),
            extra_assets: Vec::new(),
            fallback: None,
        }
    }

    pub fn with_extras(
        primary: impl Into<String>,
        extra_assets: Vec<String>,
    ) -> Self {
        Self {
            primary: primary.into(),
            extra_assets,
            fallback: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeUrls {
    pub cuda: RuntimeAssetSet,
    pub vulkan: RuntimeAssetSet,
    pub rocm: RuntimeAssetSet,
}

impl RuntimeUrls {
    pub fn for_backend(&self, name: &str) -> Option<&RuntimeAssetSet> {
        match name {
            "cuda" => Some(&self.cuda),
            "vulkan" => Some(&self.vulkan),
            "rocm" => Some(&self.rocm),
            _ => None,
        }
    }
}

impl Default for RuntimeUrls {
    fn default() -> Self {
        Self {
            cuda: RuntimeAssetSet::with_extras(
                "https://github.com/ggml-org/llama.cpp/releases/download/b8808/llama-b8808-bin-win-cuda-12.4-x64.zip",
                vec![
                    "https://github.com/ggml-org/llama.cpp/releases/download/b8808/cudart-llama-bin-win-cuda-12.4-x64.zip".to_string(),
                ],
            ),
            vulkan: RuntimeAssetSet::single(
                "https://github.com/ggml-org/llama.cpp/releases/download/b8808/llama-b8808-bin-win-vulkan-x64.zip",
            ),
            rocm: RuntimeAssetSet::single(
                "https://github.com/ggml-org/llama.cpp/releases/download/b8808/llama-b8808-bin-win-hip-radeon-x64.zip",
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub backend: String,
    pub server: ServerConfig,
    pub model: ModelConfig,
    #[serde(default)]
    pub runtime_urls: RuntimeUrls,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    /// 本番用に llama-server が listen するポート。
    /// **検証時はこの値を使わず、必ず動的ポート（find_free_port）を使用する。**
    pub port: u16,
    pub ctx_size: u32,
    pub ngl: u32,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub parallel_slots: u32,
    pub cont_batching: bool,
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub urls: UrlPair,
    pub filename: String,
    pub expected_size: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backend: "cuda".to_string(),
            server: ServerConfig::default(),
            model: ModelConfig::default(),
            runtime_urls: RuntimeUrls::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            ctx_size: 1024,
            ngl: 999,
            batch_size: 128,
            ubatch_size: 64,
            parallel_slots: 2,
            cont_batching: true,
            extra_args: vec![],
        }
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            urls: UrlPair::single(
                "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q6_K.gguf?download=true",
            ),
            filename: "HY-MT1.5-1.8B-Q6_K.gguf".to_string(),
            expected_size: 1_474_785_216,
        }
    }
}

// 旧形式マイグレーション用（model.url のみ）
#[derive(Debug, Deserialize)]
struct LegacyModelConfig {
    url: String,
    filename: String,
    expected_size: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyAppConfig {
    backend: String,
    server: ServerConfig,
    model: LegacyModelConfig,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let config = AppConfig::default();
            config.save(path)?;
            // default は expected_size が実値なので validate は不要
            return Ok(config);
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;

        if let Ok(mut config) = toml::from_str::<AppConfig>(&content) {
            config.normalize();
            config.repair_and_save(path)?;
            config.validate(path)?;
            return Ok(config);
        }

        if let Ok(legacy) = toml::from_str::<LegacyAppConfig>(&content) {
            log::info!("Migrating config from legacy single-URL format (model only)");
            let mut config = AppConfig {
                backend: legacy.backend,
                server: legacy.server,
                model: ModelConfig {
                    urls: UrlPair::single(legacy.model.url),
                    filename: legacy.model.filename,
                    // expected_size は旧形式に存在しないため 0。repair_and_save で補完を試みる。
                    expected_size: legacy.model.expected_size,
                },
                // runtime 側は旧形式に URL 情報が存在しないため、デフォルト値を使用する（互換性は取らない）
                runtime_urls: RuntimeUrls::default(),
            };
            config.normalize();
            config.repair_and_save(path)?;
            config.validate(path)?;
            config.save(path)?;
            return Ok(config);
        }

        anyhow::bail!("Failed to parse config file: {}", path.display())
    }

    /// モードに対応したデフォルト ServerConfig を持つ AppConfig を返す。
    /// model/runtime_urls は空のデフォルト値。server の batch 系のみ参照すること。
    pub fn default_for_mode(mode: &str) -> Self {
        let server = match mode {
            "passthrough" => ServerConfig {
                ctx_size: 2048,
                batch_size: 256,
                ubatch_size: 128,
                parallel_slots: 4,
                ..ServerConfig::default()
            },
            _ => ServerConfig {
                ctx_size: 1024,
                batch_size: 128,
                ubatch_size: 64,
                parallel_slots: 2,
                ..ServerConfig::default()
            },
        };
        Self {
            server,
            ..Self::default()
        }
    }

    /// authority tuple (filename / urls.primary / expected_size) の整合を検査し、
    /// known official tuple で修復できる場合は原子的に上書き保存する。
    /// 修復不能な divergence は hard-fail する。
    fn repair_and_save(&mut self, path: &Path) -> Result<()> {
        // まず filename で known tuple を引く
        if let Some(known) = known_model_tuple(&self.model.filename) {
            let url_matches = self.model.urls.primary == known.url;
            let size_matches = self.model.expected_size == known.expected_size;
            if !url_matches || !size_matches {
                log::warn!(
                    "Authority tuple diverged for filename '{}': url_ok={} size_ok={} — repairing from known table",
                    self.model.filename, url_matches, size_matches
                );
                self.model.urls.primary = known.url.to_string();
                self.model.expected_size = known.expected_size;
                self.save(path)?;
            }
            return Ok(());
        }

        // filename が unknown の場合、URL で known tuple を引く
        if let Some(known) = known_model_tuple_by_url(&self.model.urls.primary) {
            let name_matches = self.model.filename == known.filename;
            let size_matches = self.model.expected_size == known.expected_size;
            if !name_matches || !size_matches {
                log::warn!(
                    "Authority tuple diverged (url match): filename='{}' expected='{}' size_ok={} — repairing from known table",
                    self.model.filename, known.filename, size_matches
                );
                self.model.filename = known.filename.to_string();
                self.model.expected_size = known.expected_size;
                self.save(path)?;
            }
            return Ok(());
        }

        // filename も URL も unknown — tuple内部の整合だけ確認する
        // expected_size == 0 は実質的に divergence と同等なので hard-fail
        if self.model.expected_size == 0 {
            anyhow::bail!(
                "Authority tuple is unresolvable in {}: \
                 filename='{}' is not a known model and expected_size=0. \
                 Set a coherent (filename, url, expected_size) tuple in launcher_config.toml.",
                path.display(),
                self.model.filename
            );
        }

        Ok(())
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.model.expected_size == 0 {
            anyhow::bail!(
                "model.expected_size is 0 in {}. Set it to the correct file size in bytes.",
                path.display()
            );
        }
        if self.model.filename.is_empty() {
            anyhow::bail!("model.filename is empty in {}", path.display());
        }
        if self.model.urls.primary.is_empty() {
            anyhow::bail!("model.urls.primary is empty in {}", path.display());
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    fn normalize(&mut self) {
        // 空文字列・空白文字列はすべて None に正規化する
        self.model.urls.fallback = normalize_fallback(self.model.urls.fallback.take());
        self.runtime_urls.cuda.fallback = normalize_fallback(self.runtime_urls.cuda.fallback.take());
        self.runtime_urls.vulkan.fallback = normalize_fallback(self.runtime_urls.vulkan.fallback.take());
        self.runtime_urls.rocm.fallback = normalize_fallback(self.runtime_urls.rocm.fallback.take());
        // llama-server ポートが翻訳サーバー(14371)と衝突している旧設定を修復
        if self.server.port == 14371 {
            self.server.port = 8080;
        }
    }
}

fn normalize_fallback(fb: Option<String>) -> Option<String> {
    fb.filter(|s| !s.trim().is_empty())
}

pub struct KnownModelTuple {
    pub filename: &'static str,
    pub url: &'static str,
    pub expected_size: u64,
}

const KNOWN_MODELS: &[KnownModelTuple] = &[
    KnownModelTuple {
        filename: "HY-MT1.5-1.8B-Q6_K.gguf",
        url: "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q6_K.gguf?download=true",
        expected_size: 1_474_785_216,
    },
    KnownModelTuple {
        filename: "HY-MT1.5-7B-Q4_K_M.gguf",
        url: "https://huggingface.co/tencent/HY-MT1.5-7B-GGUF/resolve/main/HY-MT1.5-7B-Q4_K_M.gguf?download=true",
        expected_size: 4_624_649_312,
    },
];

/// filename で known tuple を引く
pub fn known_model_tuple(filename: &str) -> Option<&'static KnownModelTuple> {
    KNOWN_MODELS.iter().find(|t| t.filename == filename)
}

/// urls.primary で known tuple を逆引きする
pub fn known_model_tuple_by_url(url: &str) -> Option<&'static KnownModelTuple> {
    // ?download=true 等のクエリを除いたベースURLで照合
    let url_base = url.split('?').next().unwrap_or(url);
    KNOWN_MODELS.iter().find(|t| {
        let t_base = t.url.split('?').next().unwrap_or(t.url);
        t_base == url_base
    })
}
