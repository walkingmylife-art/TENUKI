//! Configuration management.
//!
//! config.toml は翻訳挙動・UI 設定・翻訳サーバー設定のみを保持する。
//! llama-server 起動条件（backend, ctx_size, ngl, model など）は
//! launcher_config.toml (AppConfig) が唯一の権威。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn default_true() -> bool {
    true
}
fn default_wrap_min_chars() -> u32 {
    30
}
fn default_wrap_min_tail_chars() -> u32 {
    10
}
fn default_prompt_template() -> String {
    "Translate the following segment into {target}, without additional explanation.".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TranslationProfile {
    #[serde(default = "default_prompt_template")]
    pub prompt_template: String,
    #[serde(default = "default_true")]
    pub enable_model_wrap: bool,
    #[serde(default = "default_wrap_min_chars")]
    pub model_wrap_min_chars: u32,
    #[serde(default = "default_wrap_min_tail_chars")]
    pub model_wrap_min_tail_chars: u32,
    #[serde(default = "default_true")]
    pub enable_model_symbol_cleanup: bool,
}

impl Default for TranslationProfile {
    fn default() -> Self {
        Self {
            prompt_template: default_prompt_template(),
            enable_model_wrap: true,
            model_wrap_min_chars: default_wrap_min_chars(),
            model_wrap_min_tail_chars: default_wrap_min_tail_chars(),
            enable_model_symbol_cleanup: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuralOptions {
    #[serde(default = "default_true")]
    pub protect_tags: bool,
    #[serde(default = "default_true")]
    pub protect_brackets: bool,
    #[serde(default = "default_true")]
    pub protect_escaped_sequences: bool,
    #[serde(default = "default_true")]
    pub protect_placeholders: bool,
    #[serde(default = "default_true")]
    pub split_symbolic_segments: bool,
}

impl Default for StructuralOptions {
    fn default() -> Self {
        Self {
            protect_tags: true,
            protect_brackets: true,
            protect_escaped_sequences: true,
            protect_placeholders: true,
            split_symbolic_segments: true,
        }
    }
}

/// 翻訳挙動・UI・翻訳サーバー設定。
/// llama-server 起動条件は含まない。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default = "default_src_lang")]
    pub src_lang: String,
    #[serde(default = "default_tgt_lang")]
    pub tgt_lang: String,
    #[serde(default)]
    pub dict_slot: Option<String>,
    #[serde(default = "default_translation_mode")]
    pub translation_mode: String,
    #[serde(default)]
    pub structural: StructuralOptions,
    #[serde(default = "default_true")]
    pub enable_model_wrap: bool,
    #[serde(default = "default_wrap_min_chars")]
    pub model_wrap_min_chars: u32,
    #[serde(default = "default_wrap_min_tail_chars")]
    pub model_wrap_min_tail_chars: u32,
    #[serde(default = "default_true")]
    pub enable_model_symbol_cleanup: bool,
    #[serde(default = "default_prompt_template")]
    pub prompt_template: String,
    /// 翻訳サーバー（TENUKI 内部）のホスト
    #[serde(default = "default_server_host")]
    pub server_host: String,
    /// 翻訳サーバー（TENUKI 内部）のポート
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default = "default_ui_lang")]
    pub ui_lang: String,
    /// Short code for a custom language, for example "vi" or "th".
    #[serde(default)]
    pub custom_lang_code: String,
    /// Display name for a custom language, for example "Vietnamese".
    #[serde(default)]
    pub custom_lang_name: String,
    /// プロファイル名（profiles/{name}.toml を読み込む）。省略時は "game"。
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default = "default_language_models")]
    pub language_models: HashMap<String, String>,
}

fn default_src_lang() -> String {
    "en".to_string()
}
fn default_tgt_lang() -> String {
    "ja".to_string()
}
fn default_translation_mode() -> String {
    "structural".to_string()
}
fn default_profile() -> String {
    "game".to_string()
}
fn default_server_host() -> String {
    "127.0.0.1".to_string()
}
fn default_server_port() -> u16 {
    14371
}
fn default_ui_lang() -> String {
    "en".to_string()
}
fn default_language_models() -> HashMap<String, String> {
    HashMap::new()
}

fn normalize_translation_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "passthrough" => "passthrough".to_string(),
        _ => default_translation_mode(),
    }
}

fn normalize_ui_lang(value: &str) -> String {
    match value.trim() {
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        _ => default_ui_lang(),
    }
}

fn normalize_profile_name(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "default" => "default".to_string(),
        "game" => "game".to_string(),
        "unity_ja" | "rpgmaker_ja" => "game".to_string(),
        _ => default_profile(),
    }
}

impl Config {
    fn normalize(&mut self) {
        self.translation_mode = normalize_translation_mode(&self.translation_mode);
        self.profile = normalize_profile_name(&self.profile);

        if self.src_lang.trim().is_empty() {
            self.src_lang = default_src_lang();
        }
        if self.tgt_lang.trim().is_empty() {
            self.tgt_lang = default_tgt_lang();
        }
        if self.server_host.trim().is_empty() {
            self.server_host = default_server_host();
        }
        if self.server_port == 0 {
            self.server_port = default_server_port();
        }
        if self.model_wrap_min_chars == 0 {
            self.model_wrap_min_chars = default_wrap_min_chars();
        }
        if self.model_wrap_min_tail_chars == 0 {
            self.model_wrap_min_tail_chars = default_wrap_min_tail_chars();
        }

        self.ui_lang = normalize_ui_lang(&self.ui_lang);

        if self.dict_slot.as_deref().is_some_and(|v| v.trim().is_empty()) {
            self.dict_slot = None;
        }
        if self.custom_lang_code.trim().is_empty() {
            self.custom_lang_name.clear();
        }
        if self.prompt_template.trim().is_empty() {
            self.prompt_template = default_prompt_template();
        }
    }

    pub fn new() -> Self {
        Self {
            src_lang: default_src_lang(),
            tgt_lang: default_tgt_lang(),
            dict_slot: None,
            translation_mode: default_translation_mode(),
            structural: StructuralOptions::default(),
            enable_model_wrap: true,
            model_wrap_min_chars: default_wrap_min_chars(),
            model_wrap_min_tail_chars: default_wrap_min_tail_chars(),
            enable_model_symbol_cleanup: true,
            prompt_template: default_prompt_template(),
            server_host: default_server_host(),
            server_port: default_server_port(),
            ui_lang: default_ui_lang(),
            custom_lang_code: String::new(),
            custom_lang_name: String::new(),
            profile: default_profile(),
            language_models: default_language_models(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.translation_mode != "structural" && self.translation_mode != "passthrough" {
            anyhow::bail!(
                "translation_mode must be 'structural' or 'passthrough', got: {}",
                self.translation_mode
            );
        }
        if self.server_port == 0 {
            anyhow::bail!("server_port must not be 0");
        }
        if self.prompt_template.trim().is_empty() {
            anyhow::bail!("prompt_template must not be empty");
        }
        if self.model_wrap_min_chars == 0 {
            anyhow::bail!("model_wrap_min_chars must not be 0");
        }
        if self.model_wrap_min_tail_chars == 0 {
            anyhow::bail!("model_wrap_min_tail_chars must not be 0");
        }
        if self.profile != "default" && self.profile != "game" {
            anyhow::bail!("profile must be 'default' or 'game', got: {}", self.profile);
        }
        Ok(())
    }
}

fn profile_file_path(config_path: &Path, profile_name: &str) -> PathBuf {
    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let name = if profile_name.trim().is_empty() {
        default_profile()
    } else {
        profile_name.trim().to_string()
    };
    base_dir.join("profiles").join(format!("{}.toml", name))
}

fn apply_translation_profile(config: &mut Config, profile: &TranslationProfile) {
    config.prompt_template = profile.prompt_template.clone();
    config.enable_model_wrap = profile.enable_model_wrap;
    config.model_wrap_min_chars = profile.model_wrap_min_chars;
    config.model_wrap_min_tail_chars = profile.model_wrap_min_tail_chars;
    config.enable_model_symbol_cleanup = profile.enable_model_symbol_cleanup;
}

pub fn current_translation_profile(config: &Config) -> TranslationProfile {
    TranslationProfile {
        prompt_template: config.prompt_template.clone(),
        enable_model_wrap: config.enable_model_wrap,
        model_wrap_min_chars: config.model_wrap_min_chars,
        model_wrap_min_tail_chars: config.model_wrap_min_tail_chars,
        enable_model_symbol_cleanup: config.enable_model_symbol_cleanup,
    }
}

pub fn save_active_profile(config_path: &Path, config: &Config) -> Result<()> {
    let profile_path = profile_file_path(config_path, &config.profile);
    if let Some(parent) = profile_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(&current_translation_profile(config))?;
    fs::write(profile_path, content)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let mut config: Config = toml::from_str(&content)?;
    config.normalize();
    let profile_path = profile_file_path(path, &config.profile);
    if let Ok(profile_content) = fs::read_to_string(profile_path) {
        if let Ok(profile) = toml::from_str::<TranslationProfile>(&profile_content) {
            apply_translation_profile(&mut config, &profile);
        }
    }
    config.validate()?;
    Ok(config)
}

pub fn save(path: &Path, config: &Config) -> Result<()> {
    let mut normalized = config.clone();
    normalized.normalize();
    normalized.validate()?;
    let content = toml::to_string_pretty(&normalized)?;
    fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Config, StructuralOptions};

    #[test]
    fn normalizes_missing_values() {
        let mut config: Config = toml::from_str(
            r#"
src_lang = ""
tgt_lang = ""
dict_slot = ""
translation_mode = "legacy"
server_port = 0
ui_lang = "de"
custom_lang_code = ""
custom_lang_name = "Vietnamese"
"#,
        )
        .unwrap();

        config.normalize();

        assert_eq!(config.src_lang, "en");
        assert_eq!(config.tgt_lang, "ja");
        assert_eq!(config.dict_slot, None);
        assert_eq!(config.translation_mode, "structural");
        assert_eq!(config.structural, StructuralOptions::default());
        assert!(config.enable_model_wrap);
        assert_eq!(config.model_wrap_min_chars, 30);
        assert_eq!(config.model_wrap_min_tail_chars, 10);
        assert!(config.enable_model_symbol_cleanup);
        assert_eq!(
            config.prompt_template,
            "Translate the following segment into {target}, without additional explanation."
        );
        assert_eq!(config.server_port, 14371);
        assert_eq!(config.ui_lang, "en");
        assert_eq!(config.custom_lang_name, "");
    }

    #[test]
    fn normalizes_unknown_translation_mode_to_structural() {
        let mut config: Config = toml::from_str(
            r#"
src_lang = "en"
tgt_lang = "ja"
translation_mode = "ANALYSIS"
"#,
        )
        .unwrap();

        config.normalize();

        assert_eq!(config.translation_mode, "structural");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn missing_structural_table_defaults_to_all_enabled() {
        let config: Config = toml::from_str(
            r#"
src_lang = "en"
tgt_lang = "ja"
"#,
        )
        .unwrap();

        assert_eq!(config.structural, StructuralOptions::default());
        assert!(config.enable_model_wrap);
        assert_eq!(config.model_wrap_min_chars, 30);
        assert_eq!(config.model_wrap_min_tail_chars, 10);
        assert!(config.enable_model_symbol_cleanup);
        assert_eq!(
            config.prompt_template,
            "Translate the following segment into {target}, without additional explanation."
        );
    }

    #[test]
    fn old_llama_fields_in_toml_are_ignored() {
        // 旧 config.toml に llama 系フィールドが残っていても parse エラーにならない
        let config: Config = toml::from_str(
            r#"
src_lang = "en"
tgt_lang = "ja"
backend = "vulkan"
llama_server_host = "127.0.0.1"
llama_server_port = 8080
ctx_size = 2048
ngl = 999
batch_size = 128
ubatch_size = 64
parallel_slots = 2
cont_batching = true
selected_model = "model.gguf"
"#,
        )
        .unwrap();

        assert_eq!(config.src_lang, "en");
        assert_eq!(config.tgt_lang, "ja");
        assert!(config.validate().is_ok());
    }
}
