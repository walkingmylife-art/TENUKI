//! Configuration management.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn default_true() -> bool {
    true
}

fn default_wrap_min_chars() -> u32 {
    80
}

fn default_wrap_space_fallback_min_chars() -> u32 {
    100
}

fn default_profile_wrap_min_chars() -> usize {
    80
}

fn default_profile_wrap_space_fallback_min_chars() -> usize {
    100
}

fn default_prompt_template() -> String {
    "Translate the following segment into {target}, without additional explanation.".to_string()
}

fn default_profile_version() -> u32 {
    1
}

fn default_profile_mode() -> String {
    "game".to_string()
}

pub const TARGET_LANGUAGE_PRESETS: &[&str] = &["ja", "en", "zh-CN", "zh-TW", "ko"];

pub fn is_target_language_preset(code: &str) -> bool {
    TARGET_LANGUAGE_PRESETS.contains(&code)
}

fn sanitize_profile_name(name: &str) -> String {
    if name.is_empty() {
        return "game".to_string();
    }

    let filtered: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();

    if filtered.is_empty() {
        "game".to_string()
    } else {
        filtered
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameTextOptions {
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

impl Default for GameTextOptions {
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileGameTextOptions {
    #[serde(default)]
    pub protect_tags: bool,
    #[serde(default)]
    pub protect_brackets: bool,
    #[serde(default)]
    pub protect_escaped_sequences: bool,
    #[serde(default)]
    pub protect_placeholders: bool,
    #[serde(default)]
    pub split_symbolic_segments: bool,
}

impl Default for ProfileGameTextOptions {
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileModelProcessingOptions {
    #[serde(default)]
    pub enable_model_wrap: bool,
    #[serde(default = "default_profile_wrap_min_chars")]
    pub model_wrap_min_chars: usize,
    #[serde(
        default = "default_profile_wrap_space_fallback_min_chars",
        alias = "model_wrap_min_tail_chars"
    )]
    pub model_wrap_space_fallback_min_chars: usize,
    #[serde(default)]
    pub enable_model_symbol_cleanup: bool,
}

impl Default for ProfileModelProcessingOptions {
    fn default() -> Self {
        Self {
            enable_model_wrap: false,
            model_wrap_min_chars: 80,
            model_wrap_space_fallback_min_chars: 100,
            enable_model_symbol_cleanup: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TranslationProfile {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default = "default_profile_mode", alias = "translation_mode")]
    pub mode: String,
    #[serde(default = "default_prompt_template")]
    pub prompt_template: String,
    #[serde(default, alias = "structural")]
    pub game_text: ProfileGameTextOptions,
    #[serde(default)]
    pub model_processing: ProfileModelProcessingOptions,
}

impl Default for TranslationProfile {
    fn default() -> Self {
        Self::game_default()
    }
}

impl TranslationProfile {
    fn default_for_name(name: &str) -> Self {
        match normalize_profile_name(name).as_str() {
            "normal" => Self::normal_default(),
            _ => Self::game_default(),
        }
    }

    pub fn game_default() -> Self {
        Self {
            version: default_profile_version(),
            mode: "game".to_string(),
            prompt_template: default_prompt_template(),
            game_text: ProfileGameTextOptions {
                protect_tags: true,
                protect_brackets: true,
                protect_escaped_sequences: true,
                protect_placeholders: true,
                split_symbolic_segments: true,
            },
            model_processing: ProfileModelProcessingOptions {
                enable_model_wrap: true,
                model_wrap_min_chars: 80,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        }
    }

    pub fn normal_default() -> Self {
        Self {
            version: default_profile_version(),
            mode: "normal".to_string(),
            prompt_template: default_prompt_template(),
            game_text: ProfileGameTextOptions::default(),
            model_processing: ProfileModelProcessingOptions {
                enable_model_wrap: true,
                model_wrap_min_chars: 80,
                model_wrap_space_fallback_min_chars: 100,
                enable_model_symbol_cleanup: true,
            },
        }
    }

    pub fn load(profile_dir: &Path, name: &str) -> Result<Self> {
        Self::provision_profile_if_missing(profile_dir, name)?;
        Self::load_existing_profile(profile_dir, name)
    }

    fn provision_profile_if_missing(profile_dir: &Path, name: &str) -> Result<()> {
        fs::create_dir_all(profile_dir)?;

        let name = normalize_profile_name(name);
        let path = profile_dir.join(format!("{}.toml", name));

        if !path.exists() {
            TranslationProfile::default_for_name(&name).save(&path)?;
        }

        Ok(())
    }

    fn load_existing_profile(profile_dir: &Path, name: &str) -> Result<Self> {
        let name = normalize_profile_name(name);
        let path = profile_dir.join(format!("{}.toml", name));

        let content = fs::read_to_string(&path)?;
        let mut profile: TranslationProfile = toml::from_str(&content)?;
        profile.normalize();
        Ok(profile)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut normalized = self.clone();
        normalized.normalize();
        let content = toml::to_string_pretty(&normalized)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn normalize(&mut self) {
        self.mode = normalize_mode_value(&self.mode);
        if self.prompt_template.trim().is_empty() {
            self.prompt_template = default_prompt_template();
        }
        if self.model_processing.model_wrap_min_chars == 0 {
            self.model_processing.model_wrap_min_chars = 80;
        }
        if self.model_processing.model_wrap_space_fallback_min_chars
            < self.model_processing.model_wrap_min_chars
        {
            self.model_processing.model_wrap_space_fallback_min_chars = 100;
        }
    }
}

impl From<GameTextOptions> for ProfileGameTextOptions {
    fn from(value: GameTextOptions) -> Self {
        Self {
            protect_tags: value.protect_tags,
            protect_brackets: value.protect_brackets,
            protect_escaped_sequences: value.protect_escaped_sequences,
            protect_placeholders: value.protect_placeholders,
            split_symbolic_segments: value.split_symbolic_segments,
        }
    }
}

impl From<ProfileGameTextOptions> for GameTextOptions {
    fn from(value: ProfileGameTextOptions) -> Self {
        Self {
            protect_tags: value.protect_tags,
            protect_brackets: value.protect_brackets,
            protect_escaped_sequences: value.protect_escaped_sequences,
            protect_placeholders: value.protect_placeholders,
            split_symbolic_segments: value.split_symbolic_segments,
        }
    }
}

fn default_list_request_timeout() -> u64 {
    60
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ListConfig {
    #[serde(default)]
    pub input_root: std::path::PathBuf,
    #[serde(default)]
    pub output_path: std::path::PathBuf,
    #[serde(default = "default_list_request_timeout")]
    pub request_timeout: u64,
    #[serde(default)]
    pub chunk_size: usize,
}

impl Default for ListConfig {
    fn default() -> Self {
        Self {
            input_root: std::path::PathBuf::new(),
            output_path: std::path::PathBuf::new(),
            request_timeout: default_list_request_timeout(),
            chunk_size: 0,
        }
    }
}

impl ListConfig {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn effective_chunk_size(&self, parallel_slots: usize) -> usize {
        if self.chunk_size == 0 {
            parallel_slots.max(4)
        } else {
            self.chunk_size
        }
    }

    pub fn effective_timeout_secs(&self) -> u64 {
        if self.request_timeout == 0 {
            default_list_request_timeout()
        } else {
            self.request_timeout
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default = "default_src_lang")]
    pub src_lang: String,
    #[serde(default = "default_tgt_lang")]
    pub tgt_lang: String,
    #[serde(default)]
    pub dict_slot: Option<String>,
    #[serde(default = "default_mode", alias = "translation_mode")]
    pub mode: String,
    #[serde(default, alias = "structural", skip_serializing)]
    pub game_text: GameTextOptions,
    #[serde(default = "default_true", skip_serializing)]
    pub enable_model_wrap: bool,
    #[serde(default, skip_serializing)]
    pub wrap_override: Option<bool>,
    #[serde(default = "default_wrap_min_chars", skip_serializing)]
    pub model_wrap_min_chars: u32,
    #[serde(default = "default_wrap_space_fallback_min_chars", alias = "model_wrap_min_tail_chars", skip_serializing)]
    pub model_wrap_space_fallback_min_chars: u32,
    #[serde(default = "default_true", skip_serializing)]
    pub enable_model_symbol_cleanup: bool,
    #[serde(default = "default_prompt_template", skip_serializing)]
    pub prompt_template: String,
    #[serde(default = "default_server_host")]
    pub server_host: String,
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default = "default_ui_lang")]
    pub ui_lang: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custom_lang_name: String,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default = "default_language_models")]
    pub language_models: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "ListConfig::is_default")]
    pub list: ListConfig,
}

fn default_src_lang() -> String {
    "en".to_string()
}

fn default_tgt_lang() -> String {
    "en".to_string()
}

fn default_mode() -> String {
    "game".to_string()
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

pub fn normalize_mode_value(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" | "passthrough" => "normal".to_string(),
        "game" | "structural" => "game".to_string(),
        _ => default_mode(),
    }
}

fn normalize_ui_lang(value: &str) -> String {
    match value.trim() {
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        "zh-CN" => "zh-CN".to_string(),
        _ => default_ui_lang(),
    }
}

fn normalize_profile_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return default_profile();
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "unity_ja" | "rpgmaker_ja" | "default" => "game".to_string(),
        "game" | "normal" => trimmed.to_ascii_lowercase(),
        _ => sanitize_profile_name(trimmed),
    }
}

impl Config {
    fn normalize(&mut self) {
        self.profile = normalize_profile_name(&self.profile);
        self.mode = normalize_mode_value(&self.mode);

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

        self.ui_lang = normalize_ui_lang(&self.ui_lang);

        if self
            .dict_slot
            .as_deref()
            .is_some_and(|v| v.trim().is_empty())
        {
            self.dict_slot = None;
        }
        if is_target_language_preset(&self.tgt_lang) {
            self.custom_lang_name.clear();
        }

        if self.model_wrap_min_chars == 0 {
            self.model_wrap_min_chars = 80;
        }
        if self.model_wrap_space_fallback_min_chars < self.model_wrap_min_chars {
            self.model_wrap_space_fallback_min_chars = 100;
        }
    }

    pub fn new() -> Self {
        let profile = TranslationProfile::game_default();
        Self {
            src_lang: default_src_lang(),
            tgt_lang: default_tgt_lang(),
            dict_slot: None,
            mode: profile.mode.clone(),
            game_text: profile.game_text.into(),
            enable_model_wrap: profile.model_processing.enable_model_wrap,
            wrap_override: None,
            model_wrap_min_chars: profile.model_processing.model_wrap_min_chars as u32,
            model_wrap_space_fallback_min_chars: profile.model_processing.model_wrap_space_fallback_min_chars as u32,
            enable_model_symbol_cleanup: profile.model_processing.enable_model_symbol_cleanup,
            prompt_template: profile.prompt_template.clone(),
            server_host: default_server_host(),
            server_port: default_server_port(),
            ui_lang: default_ui_lang(),
            custom_lang_name: String::new(),
            profile: default_profile(),
            language_models: default_language_models(),
            list: ListConfig::default(),
        }
    }

    pub fn effective_model_wrap(&self) -> bool {
        self.wrap_override.unwrap_or(self.enable_model_wrap)
    }

    pub fn validate(&self) -> Result<()> {
        if self.server_port == 0 {
            anyhow::bail!("server_port must not be 0");
        }
        if self.profile.trim().is_empty() {
            anyhow::bail!("profile must not be empty");
        }
        Ok(())
    }
}

fn apply_translation_profile(config: &mut Config, profile: &TranslationProfile) {
    config.mode = profile.mode.clone();
    config.prompt_template = profile.prompt_template.clone();
    config.game_text = profile.game_text.into();
    config.enable_model_wrap = profile.model_processing.enable_model_wrap;
    config.model_wrap_min_chars = profile.model_processing.model_wrap_min_chars as u32;
    config.model_wrap_space_fallback_min_chars = profile.model_processing.model_wrap_space_fallback_min_chars as u32;
    config.enable_model_symbol_cleanup = profile.model_processing.enable_model_symbol_cleanup;
}

/// Applies a small runtime overlay derived from the current target language.
fn apply_target_language_policy(config: &mut Config) {
    config.wrap_override = None;
}

/// Saves game-text protection options into the active profile, not the root config.
pub fn save_profile_game_text(
    config_path: &Path,
    profile_name: &str,
    options: crate::config::GameTextOptions,
) -> Result<()> {
    let profile_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("profiles");
    let mut profile = TranslationProfile::load(&profile_dir, profile_name).unwrap_or_default();
    profile.game_text = options.into();
    let path = profile_dir.join(format!("{}.toml", normalize_profile_name(profile_name)));
    profile.save(&path)
}

pub fn load_profile(config_path: &Path, profile_name: &str) -> Result<TranslationProfile> {
    let profile_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("profiles");
    TranslationProfile::load(&profile_dir, profile_name)
}

pub fn load(path: &Path) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let mut config: Config = toml::from_str(&content)?;
    config.normalize();

    let profile = load_profile(path, &config.profile).unwrap_or_default();
    apply_translation_profile(&mut config, &profile);
    apply_target_language_policy(&mut config);

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
    use super::{load, normalize_mode_value, save, Config, GameTextOptions, TranslationProfile};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn normalizes_missing_values() {
        let mut config: Config = toml::from_str(
            r#"
src_lang = ""
tgt_lang = ""
dict_slot = ""
server_port = 0
ui_lang = "de"
custom_lang_name = "Brazilian Portuguese"
"#,
        )
        .unwrap();

        config.normalize();

        assert_eq!(config.src_lang, "en");
        assert_eq!(config.tgt_lang, "en");
        assert_eq!(config.dict_slot, None);
        assert_eq!(config.game_text, GameTextOptions::default());
        assert_eq!(config.server_port, 14371);
        assert_eq!(config.ui_lang, "en");
        assert_eq!(config.custom_lang_name, "");
        assert_eq!(config.wrap_override, None);
        assert_eq!(config.mode, "game");
    }

    #[test]
    fn missing_game_text_table_defaults_to_all_enabled() {
        let config: Config = toml::from_str(
            r#"
src_lang = "en"
tgt_lang = "ja"
"#,
        )
        .unwrap();

        assert_eq!(config.game_text, GameTextOptions::default());
        assert_eq!(config.enable_model_wrap, true);
        assert_eq!(config.model_wrap_min_chars, 80);
        assert_eq!(config.model_wrap_space_fallback_min_chars, 100);
        assert_eq!(config.enable_model_symbol_cleanup, true);
        assert_eq!(config.mode, "game");
    }

    #[test]
    fn loads_profile_into_runtime_config_with_normal_mode() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_config_test_{}", unique));
        std::fs::create_dir_all(base_dir.join("profiles")).unwrap();

        std::fs::write(
            base_dir.join("config.toml"),
            r#"
src_lang = "en"
tgt_lang = "ja"
profile = "normal"
"#,
        )
        .unwrap();

        std::fs::write(
            base_dir.join("profiles").join("normal.toml"),
            r#"
version = 1
mode = "normal"
prompt_template = "Profile prompt"

[game_text]
protect_tags = true
protect_brackets = false
protect_escaped_sequences = true
protect_placeholders = false
split_symbolic_segments = true

[model_processing]
enable_model_wrap = true
model_wrap_min_chars = 40
model_wrap_space_fallback_min_chars = 120
enable_model_symbol_cleanup = true
"#,
        )
        .unwrap();

        let config = load(&base_dir.join("config.toml")).unwrap();

        assert_eq!(config.profile, "normal");
        assert_eq!(config.mode, "normal");
        assert_eq!(config.prompt_template, "Profile prompt");
        assert!(config.game_text.protect_tags);
        assert!(!config.game_text.protect_brackets);
        assert_eq!(config.model_wrap_min_chars, 40);
        assert_eq!(config.model_wrap_space_fallback_min_chars, 120);
        assert!(config.enable_model_symbol_cleanup);
        assert_eq!(config.wrap_override, None);

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn legacy_mode_value_is_normalized_at_load_boundary() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_config_legacy_test_{}", unique));
        std::fs::create_dir_all(base_dir.join("profiles")).unwrap();

        std::fs::write(
            base_dir.join("config.toml"),
            r#"
src_lang = "en"
tgt_lang = "ja"
profile = "custom"
"#,
        )
        .unwrap();

        std::fs::write(
            base_dir.join("profiles").join("custom.toml"),
            r#"
version = 1
translation_mode = "passthrough"

[structural]
protect_tags = true
protect_brackets = false
protect_escaped_sequences = true
protect_placeholders = false
split_symbolic_segments = true
"#,
        )
        .unwrap();

        let config = load(&base_dir.join("config.toml")).unwrap();
        assert_eq!(config.mode, "normal");
        assert!(config.game_text.protect_tags);
        assert!(!config.game_text.protect_brackets);

        let saved_profile =
            std::fs::read_to_string(base_dir.join("profiles").join("custom.toml")).unwrap();
        assert!(saved_profile.contains("translation_mode"));

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn save_writes_mode_but_not_legacy_profile_owned_keys() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_config_save_test_{}", unique));
        std::fs::create_dir_all(&base_dir).unwrap();

        let path = base_dir.join("config.toml");
        let mut config = Config::new();
        config.profile = "game".to_string();
        save(&path, &config).unwrap();

        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("mode = \"game\""));
        assert!(!saved.contains("translation_mode"));
        assert!(!saved.contains("prompt_template"));
        assert!(!saved.contains("enable_model_wrap"));
        assert!(!saved.contains("wrap_override"));
        assert!(!saved.contains("[structural]"));

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn profile_roundtrip_keeps_game_mode() {
        let profile = TranslationProfile::game_default();
        assert_eq!(profile.mode, "game");
        assert!(profile.game_text.protect_tags);
        assert!(profile.model_processing.enable_model_wrap);
    }

    #[test]
    fn missing_game_profile_uses_game_defaults() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("tenuki_profile_seed_test_{}", unique));
        let profiles_dir = base_dir.join("profiles");

        let profile = TranslationProfile::load(&profiles_dir, "game").unwrap();
        let saved = std::fs::read_to_string(profiles_dir.join("game.toml")).unwrap();

        assert!(profile.game_text.protect_tags);
        assert!(profile.game_text.protect_brackets);
        assert!(profile.game_text.protect_escaped_sequences);
        assert!(profile.game_text.protect_placeholders);
        assert!(profile.game_text.split_symbolic_segments);
        assert!(profile.model_processing.enable_model_wrap);
        assert!(profile.model_processing.enable_model_symbol_cleanup);
        assert!(saved.contains("mode = \"game\""));
        assert!(saved.contains("[game_text]"));

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn old_llama_fields_in_toml_are_ignored() {
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

    #[test]
    fn non_ui_target_keeps_custom_lang_name() {
        let mut config: Config = toml::from_str(
            r#"
src_lang = "en"
tgt_lang = "pt-BR"
custom_lang_name = "Brazilian Portuguese"
"#,
        )
        .unwrap();

        config.normalize();

        assert_eq!(config.tgt_lang, "pt-BR");
        assert_eq!(config.custom_lang_name, "Brazilian Portuguese");
    }

    #[test]
    fn normalize_mode_maps_legacy_values_to_new_names() {
        assert_eq!(normalize_mode_value("structural"), "game");
        assert_eq!(normalize_mode_value("passthrough"), "normal");
        assert_eq!(normalize_mode_value("game"), "game");
        assert_eq!(normalize_mode_value("normal"), "normal");
    }

    #[test]
    fn legacy_profile_model_wrap_min_tail_chars_is_normalized() {
        let tmp = std::env::temp_dir().join(format!("tenuki_test_profile_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("profiles")).unwrap();

        let config_path = tmp.join("config.toml");
        let profile_toml = r#"
[model_processing]
enable_model_wrap = true
model_wrap_min_chars = 80
model_wrap_min_tail_chars = 10
enable_model_symbol_cleanup = true
"#;
        std::fs::write(tmp.join("profiles").join("game.toml"), profile_toml).unwrap();
        std::fs::write(
            &config_path,
            format!(
                "profile = \"game\"\nsrc_lang = \"en\"\ntgt_lang = \"ja\"\nserver_port = {}\n",
                18000 + (std::process::id() % 10000) as u16
            ),
        )
        .unwrap();

        let config = load(&config_path).unwrap();
        assert_eq!(config.model_wrap_space_fallback_min_chars, 100);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
