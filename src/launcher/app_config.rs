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

    pub fn with_extras(primary: impl Into<String>, extra_assets: Vec<String>) -> Self {
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

/// Model authority object. Exactly one of two kinds:
/// - `Known`: TENUKI-managed model with download URLs; filename must be in KNOWN_MODELS.
/// - `Local`: user-placed model; no URLs, missing = wait/re-select.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ModelConfig {
    Known {
        filename: String,
        expected_size: u64,
        urls: UrlPair,
    },
    Local {
        filename: String,
        expected_size: u64,
    },
}

impl ModelConfig {
    pub fn filename(&self) -> &str {
        match self {
            Self::Known { filename, .. } | Self::Local { filename, .. } => filename,
        }
    }

    pub fn expected_size(&self) -> u64 {
        match self {
            Self::Known { expected_size, .. } | Self::Local { expected_size, .. } => *expected_size,
        }
    }

    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known { .. })
    }
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
        Self::Known {
            filename: "HY-MT1.5-1.8B-Q6_K.gguf".to_string(),
            expected_size: 1_474_785_216,
            urls: UrlPair::single(
                "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q6_K.gguf?download=true",
            ),
        }
    }
}

// --- Migration structs (old formats without `kind` tag) ---

/// Old struct-based ModelConfig (had `urls: UrlPair` — no `kind` field).
#[derive(Debug, Deserialize)]
struct LegacyModelConfigV2 {
    urls: UrlPair,
    filename: String,
    expected_size: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyAppConfigV2 {
    backend: String,
    server: ServerConfig,
    model: LegacyModelConfigV2,
    #[serde(default)]
    runtime_urls: RuntimeUrls,
}

/// Oldest single-url format.
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

fn migrate_legacy_model(
    filename: String,
    url: String,
    expected_size: u64,
    path: &Path,
) -> Result<ModelConfig> {
    if let Some(known) = known_model_tuple(&filename) {
        return Ok(ModelConfig::Known {
            filename: known.filename.to_string(),
            expected_size: known.expected_size,
            urls: UrlPair::single(known.url),
        });
    }
    if known_model_tuple_by_url(&url).is_some() {
        // URL は known だが filename が不明 → Known にも Local にも解釈できない混成状態。
        // filename を書き換えることは authority 破壊なので fail fast。
        anyhow::bail!(
            "Config in {} cannot be interpreted as either Known or Local: \
             filename='{}' is not in the known table, but url matches a known model. \
             To fix: either use the correct known filename, or change the url to a \
             non-known URL and set a valid expected_size (Local model).",
            path.display(),
            filename
        );
    }
    if expected_size == 0 {
        // size がないと Local としても authority を確定できない。
        anyhow::bail!(
            "Config in {} cannot be interpreted as either Known or Local: \
             filename='{}' is not in the known table and expected_size=0. \
             Set expected_size to the correct file size in bytes to treat this as a Local model.",
            path.display(),
            filename
        );
    }
    // Unknown filename, unknown url, size > 0 → Local (discard url)
    Ok(ModelConfig::Local {
        filename,
        expected_size,
    })
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            anyhow::bail!(
                "launcher_config.toml not found at {}. Run setup to generate it.",
                path.display()
            );
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;

        // Step 1: current format (ModelConfig enum with `kind` tag)
        if let Ok(mut config) = toml::from_str::<AppConfig>(&content) {
            config.normalize();
            config.repair_and_save(path)?;
            config.validate(path)?;
            return Ok(config);
        }

        // Step 2: old struct format (urls: UrlPair, no `kind` field)
        if let Ok(legacy) = toml::from_str::<LegacyAppConfigV2>(&content) {
            log::info!("Migrating config from legacy struct format (no kind field)");
            let model = migrate_legacy_model(
                legacy.model.filename,
                legacy.model.urls.primary,
                legacy.model.expected_size,
                path,
            )?;
            let mut config = AppConfig {
                backend: legacy.backend,
                server: legacy.server,
                model,
                runtime_urls: legacy.runtime_urls,
            };
            config.normalize();
            config.repair_and_save(path)?;
            config.validate(path)?;
            config.save(path)?;
            return Ok(config);
        }

        // Step 3: oldest single-url format
        if let Ok(legacy) = toml::from_str::<LegacyAppConfig>(&content) {
            log::info!("Migrating config from legacy single-URL format");
            let model = migrate_legacy_model(
                legacy.model.filename,
                legacy.model.url,
                legacy.model.expected_size,
                path,
            )?;
            let mut config = AppConfig {
                backend: legacy.backend,
                server: legacy.server,
                model,
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
            "normal" => ServerConfig {
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

    /// Known: filename を known table で検証し、diverged tuple を修復保存する。
    /// Local: expected_size > 0 を確認するだけ。URL repair はしない。
    fn repair_and_save(&mut self, path: &Path) -> Result<()> {
        match &self.model {
            ModelConfig::Known { filename, .. } => {
                let filename = filename.clone();
                let known = known_model_tuple(&filename).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Config in {} has kind=Known but filename '{}' is not in the known \
                         table. Cannot be interpreted as Known. \
                         Change kind to Local (and remove urls) or use a recognized filename.",
                        path.display(),
                        filename
                    )
                })?;
                let needs_repair = match &self.model {
                    ModelConfig::Known {
                        urls,
                        expected_size,
                        ..
                    } => urls.primary != known.url || *expected_size != known.expected_size,
                    _ => unreachable!(),
                };
                if needs_repair {
                    log::warn!(
                        "Authority tuple diverged for '{}': repairing from known table",
                        filename
                    );
                    self.model = ModelConfig::Known {
                        filename: known.filename.to_string(),
                        expected_size: known.expected_size,
                        urls: UrlPair {
                            primary: known.url.to_string(),
                            fallback: None,
                        },
                    };
                    self.save(path)?;
                }
            }
            ModelConfig::Local {
                filename,
                expected_size,
            } => {
                let (filename, expected_size) = (filename.clone(), *expected_size);
                if filename.is_empty() {
                    anyhow::bail!("model.filename is empty (Local) in {}", path.display());
                }
                if expected_size == 0 {
                    anyhow::bail!(
                        "model.expected_size is 0 (Local) in {}. \
                         Set it to the correct file size in bytes.",
                        path.display()
                    );
                }
            }
        }
        Ok(())
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.model.filename().is_empty() {
            anyhow::bail!("model.filename is empty in {}", path.display());
        }
        if self.model.expected_size() == 0 {
            anyhow::bail!(
                "model.expected_size is 0 in {}. Set it to the correct file size in bytes.",
                path.display()
            );
        }
        if let ModelConfig::Known { urls, .. } = &self.model {
            if urls.primary.is_empty() {
                anyhow::bail!("model.urls.primary is empty in {}", path.display());
            }
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    fn normalize(&mut self) {
        if let ModelConfig::Known { urls, .. } = &mut self.model {
            urls.fallback = normalize_fallback(urls.fallback.take());
        }
        self.runtime_urls.cuda.fallback =
            normalize_fallback(self.runtime_urls.cuda.fallback.take());
        self.runtime_urls.vulkan.fallback =
            normalize_fallback(self.runtime_urls.vulkan.fallback.take());
        self.runtime_urls.rocm.fallback =
            normalize_fallback(self.runtime_urls.rocm.fallback.take());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_config_path(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tenuki_appcfg_{}", tag));
        fs::create_dir_all(&dir).unwrap();
        dir.join("launcher_config.toml")
    }

    // --- Known: authority tuple 修復 ---

    #[test]
    fn repair_known_filename_fixes_diverged_url_and_size() {
        let path = temp_config_path("repair_known");
        let mut cfg = AppConfig::default();
        cfg.model = ModelConfig::Known {
            filename: "HY-MT1.5-1.8B-Q6_K.gguf".to_string(),
            expected_size: 1,
            urls: UrlPair::single("https://wrong.example.com/bad.gguf"),
        };
        cfg.save(&path).unwrap();

        let loaded = AppConfig::load(&path).unwrap();
        let known = known_model_tuple("HY-MT1.5-1.8B-Q6_K.gguf").unwrap();

        assert_eq!(loaded.model.filename(), "HY-MT1.5-1.8B-Q6_K.gguf");
        assert!(
            matches!(&loaded.model, ModelConfig::Known { urls, expected_size, .. }
            if urls.primary == known.url && *expected_size == known.expected_size)
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn repair_preserves_filename_when_only_url_diverges() {
        let path = temp_config_path("repair_url_only");
        let mut cfg = AppConfig::default();
        cfg.model = ModelConfig::Known {
            filename: "HY-MT1.5-1.8B-Q6_K.gguf".to_string(),
            expected_size: 1_474_785_216,
            urls: UrlPair::single("https://wrong.example.com/bad.gguf"),
        };
        cfg.save(&path).unwrap();

        let loaded = AppConfig::load(&path).unwrap();
        assert_eq!(loaded.model.filename(), "HY-MT1.5-1.8B-Q6_K.gguf");
        let known = known_model_tuple("HY-MT1.5-1.8B-Q6_K.gguf").unwrap();
        assert!(
            matches!(&loaded.model, ModelConfig::Known { urls, .. } if urls.primary == known.url)
        );

        let _ = fs::remove_file(&path);
    }

    // --- Known: unknown filename は fail fast ---

    #[test]
    fn known_kind_with_unknown_filename_fails() {
        let path = temp_config_path("known_unknown_fn");
        let mut cfg = AppConfig::default();
        cfg.model = ModelConfig::Known {
            filename: "not-a-known-model.gguf".to_string(),
            expected_size: 1_474_785_216,
            urls: UrlPair::single("https://example.com/custom.gguf"),
        };
        cfg.save(&path).unwrap();

        let result = AppConfig::load(&path);
        assert!(
            result.is_err(),
            "kind=Known with unknown filename must fail"
        );

        let _ = fs::remove_file(&path);
    }

    // --- Local: valid size is accepted ---

    #[test]
    fn local_model_with_nonzero_size_ok() {
        let path = temp_config_path("local_ok");
        let mut cfg = AppConfig::default();
        cfg.model = ModelConfig::Local {
            filename: "my-custom-model.gguf".to_string(),
            expected_size: 9_000_000,
        };
        cfg.save(&path).unwrap();

        let result = AppConfig::load(&path);
        assert!(
            result.is_ok(),
            "Local model with valid size should load: {:?}",
            result
        );
        let loaded = result.unwrap();
        assert!(
            matches!(&loaded.model, ModelConfig::Local { filename, .. } if filename == "my-custom-model.gguf")
        );
        assert!(!loaded.model.is_known());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn local_model_with_zero_size_fails() {
        let path = temp_config_path("local_zero_size");
        let mut cfg = AppConfig::default();
        cfg.model = ModelConfig::Local {
            filename: "my-custom-model.gguf".to_string(),
            expected_size: 0,
        };
        cfg.save(&path).unwrap();

        let result = AppConfig::load(&path);
        assert!(
            result.is_err(),
            "Local model with expected_size=0 must fail"
        );

        let _ = fs::remove_file(&path);
    }

    // --- migration: old struct format (no `kind` field) ---

    #[test]
    fn migration_v2_known_filename_produces_known() {
        let path = temp_config_path("mig_v2_known");
        // write legacy struct format (no `kind` field)
        let toml = r#"
backend = "cuda"

[server]
host = "127.0.0.1"
port = 8080
ctx_size = 1024
ngl = 999
batch_size = 128
ubatch_size = 64
parallel_slots = 2
cont_batching = true
extra_args = []

[model]
filename = "HY-MT1.5-1.8B-Q6_K.gguf"
expected_size = 1474785216

[model.urls]
primary = "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q6_K.gguf?download=true"
"#;
        fs::write(&path, toml).unwrap();
        let loaded = AppConfig::load(&path).unwrap();
        assert!(loaded.model.is_known());
        assert_eq!(loaded.model.filename(), "HY-MT1.5-1.8B-Q6_K.gguf");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn migration_v2_unknown_filename_known_url_fails() {
        let path = temp_config_path("mig_v2_unknown_known_url");
        let toml = r#"
backend = "cuda"

[server]
host = "127.0.0.1"
port = 8080
ctx_size = 1024
ngl = 999
batch_size = 128
ubatch_size = 64
parallel_slots = 2
cont_batching = true
extra_args = []

[model]
filename = "not-a-known-model.gguf"
expected_size = 1474785216

[model.urls]
primary = "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q6_K.gguf?download=true"
"#;
        fs::write(&path, toml).unwrap();
        let result = AppConfig::load(&path);
        assert!(
            result.is_err(),
            "unknown filename + known URL must fail during migration"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn migration_v2_unknown_filename_unknown_url_ok_produces_local() {
        let path = temp_config_path("mig_v2_local");
        let toml = r#"
backend = "cuda"

[server]
host = "127.0.0.1"
port = 8080
ctx_size = 1024
ngl = 999
batch_size = 128
ubatch_size = 64
parallel_slots = 2
cont_batching = true
extra_args = []

[model]
filename = "my-custom-model.gguf"
expected_size = 9000000

[model.urls]
primary = "https://example.com/custom.gguf"
"#;
        fs::write(&path, toml).unwrap();
        let result = AppConfig::load(&path);
        assert!(
            result.is_ok(),
            "unknown filename + unknown url + size>0 should migrate to Local: {:?}",
            result
        );
        let loaded = result.unwrap();
        assert!(!loaded.model.is_known(), "migrated model must be Local");
        assert_eq!(loaded.model.filename(), "my-custom-model.gguf");
        let _ = fs::remove_file(&path);
    }
}
