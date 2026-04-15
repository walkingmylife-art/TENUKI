use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

// -----------------------------------------------------------------------------
// TranslationProfile : 翻訳時の挙動を定義するプロファイル
// -----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranslationProfile {
    #[serde(default = "default_version")]
    pub version: u32,
    pub prompt_template: String,
    pub translation_mode: String,
    pub structural: StructuralOptions,
    pub model_processing: ModelProcessingOptions,
}

fn default_version() -> u32 {
    1
}

impl Default for TranslationProfile {
    fn default() -> Self {
        Self {
            version: 1,
            prompt_template: "Translate the following text into {target}:".to_string(),
            translation_mode: "structural".to_string(),
            structural: StructuralOptions::default(),
            model_processing: ModelProcessingOptions::default(),
        }
    }
}

impl TranslationProfile {
    /// ゲーム翻訳に特化したデフォルトプロファイルを生成する
    pub fn game_default() -> Self {
        Self {
            version: 1,
            prompt_template: "Translate the following segment into {target}, preserving all special symbols and tags exactly as they appear. Do not add any explanations.".to_string(),
            translation_mode: "structural".to_string(),
            structural: StructuralOptions {
                protect_tags: true,
                protect_brackets: true,
                protect_escaped_sequences: true,
                protect_placeholders: true,
                split_symbolic_segments: true,
            },
            model_processing: ModelProcessingOptions {
                enable_model_wrap: true,
                model_wrap_min_chars: 30,
                model_wrap_min_tail_chars: 10,
                enable_model_symbol_cleanup: true,
            },
        }
    }

    pub fn load(profile_dir: &Path, name: &str) -> Result<Self> {
        // プロファイルディレクトリが存在しない場合は作成
        std::fs::create_dir_all(profile_dir)
            .with_context(|| format!("Failed to create profile dir: {}", profile_dir.display()))?;

        let name = sanitize_profile_name(name);
        let path = profile_dir.join(format!("{}.toml", name));

        if !path.exists() {
            // 指定されたプロファイルがなければ default をコピー
            let default_path = profile_dir.join("default.toml");
            if !default_path.exists() {
                TranslationProfile::default().save(&default_path)?;
            }
            std::fs::copy(&default_path, &path)
                .with_context(|| format!("Failed to copy default profile to {}", path.display()))?;
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read profile: {}", path.display()))?;
        let profile: TranslationProfile = toml::from_str(&content)
            .with_context(|| format!("Failed to parse profile: {}", path.display()))?;
        Ok(profile)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize profile")?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write profile to {}", path.display()))?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// StructuralOptions : 構造保護オプション
// -----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StructuralOptions {
    pub protect_tags: bool,
    pub protect_brackets: bool,
    pub protect_escaped_sequences: bool,
    pub protect_placeholders: bool,
    pub split_symbolic_segments: bool,
}

impl Default for StructuralOptions {
    fn default() -> Self {
        Self {
            protect_tags: false,
            protect_brackets: false,
            protect_escaped_sequences: false,
            protect_placeholders: false,
            split_symbolic_segments: false,
        }
    }
}

// -----------------------------------------------------------------------------
// ModelProcessingOptions : モデル出力後処理オプション
// -----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelProcessingOptions {
    pub enable_model_wrap: bool,
    pub model_wrap_min_chars: usize,
    pub model_wrap_min_tail_chars: usize,
    pub enable_model_symbol_cleanup: bool,
}

impl Default for ModelProcessingOptions {
    fn default() -> Self {
        Self {
            enable_model_wrap: false,
            model_wrap_min_chars: 30,
            model_wrap_min_tail_chars: 10,
            enable_model_symbol_cleanup: false,
        }
    }
}

// -----------------------------------------------------------------------------
// 内部ヘルパー関数
// -----------------------------------------------------------------------------
fn sanitize_profile_name(name: &str) -> String {
    if name.is_empty() {
        return "default".to_string();
    }
    let filtered: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if filtered.is_empty() {
        "default".to_string()
    } else {
        filtered
    }
}