//! Legacy `config.toml` migration helpers for the launcher.

use anyhow::{anyhow, Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use toml::Value;

use super::app_config::AppConfig;
use super::translation_profile::{StructuralOptions, TranslationProfile};
use crate::config::Config as LegacyConfig;

fn is_legacy_format(root: &Value) -> bool {
    let Some(table) = root.as_table() else {
        return false;
    };

    table.contains_key("prompt_template")
        || table.contains_key("translation_mode")
        || table.contains_key("enable_model_wrap")
        || table.contains_key("structural")
}

pub fn migrate_config_if_needed(config_path: &Path) -> Result<bool> {
    if !config_path.exists() {
        return Ok(false);
    }

    let base_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("Invalid config path: no parent directory"))?;

    // launcher_config.toml が既存なら移行済み → 毎起動の誤実行を防ぐ
    if base_dir.join("launcher_config.toml").exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    let root: Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse TOML in {}", config_path.display()))?;

    if !is_legacy_format(&root) {
        return Ok(false);
    }

    let legacy: LegacyConfig = toml::from_str(&content)
        .with_context(|| "Failed to parse legacy config")?;

    let backup_path = config_path.with_extension("toml.old");
    std::fs::copy(config_path, &backup_path)
        .with_context(|| format!("Failed to backup config to {}", backup_path.display()))?;

    // launcher_config.toml にランチャー専用設定を書き出す
    let launcher_config = build_app_config(&legacy, &root);
    launcher_config.save(&base_dir.join("launcher_config.toml"))?;

    // config.toml をクリーンな Config 形式で上書き
    // selected_model, backend は AppConfig (launcher_config.toml) で管理するため削除
    let clean_config = legacy.clone();
    crate::config::save(config_path, &clean_config)
        .with_context(|| "Failed to write updated config.toml")?;

    let profiles_dir = base_dir.join("profiles");
    std::fs::create_dir_all(&profiles_dir)?;

    let profile_name = sanitize_profile_name(&legacy.profile);
    let profile_path = profiles_dir.join(format!("{}.toml", profile_name));
    if !profile_path.exists() {
        let profile = build_translation_profile(&legacy, &profile_name);
        profile.save(&profile_path)?;
    }

    let default_profile_path = profiles_dir.join("default.toml");
    if !default_profile_path.exists() {
        TranslationProfile::default().save(&default_profile_path)?;
    }

    // legacy の backend を launcher_config.toml に書き戻す（state.json は使わない）
    let launcher_config_path = base_dir.join("launcher_config.toml");
    if let Ok(mut lc) = AppConfig::load(&launcher_config_path) {
        if lc.backend == AppConfig::default().backend {
            lc.backend = val_str(&root, "backend").to_string();
            let _ = lc.save(&launcher_config_path);
        }
    }

    Ok(true)
}

fn val_str<'a>(root: &'a Value, key: &str) -> &'a str {
    root.get(key).and_then(|v| v.as_str()).unwrap_or("")
}
fn val_u32(root: &Value, key: &str) -> u32 {
    root.get(key).and_then(|v| v.as_integer()).unwrap_or(0) as u32
}
fn val_bool(root: &Value, key: &str, default: bool) -> bool {
    root.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn build_app_config(legacy: &LegacyConfig, raw: &Value) -> AppConfig {
    let mut config = AppConfig::default();

    let backend = val_str(raw, "backend");
    if !backend.is_empty() {
        config.backend = backend.to_string();
    }

    let llama_host = val_str(raw, "llama_server_host");
    if !llama_host.is_empty() {
        config.server.host = llama_host.to_string();
    }
    let llama_port = val_u32(raw, "llama_server_port");
    if llama_port != 0 {
        config.server.port = llama_port as u16;
    }

    let ctx   = val_u32(raw, "ctx_size");      config.server.ctx_size      = if ctx   > 0 { ctx   } else { 1024 };
    let batch = val_u32(raw, "batch_size");    config.server.batch_size    = if batch > 0 { batch } else { 128  };
    let ub    = val_u32(raw, "ubatch_size");   config.server.ubatch_size   = if ub    > 0 { ub    } else { 64   };
    let ngl   = val_u32(raw, "ngl");           config.server.ngl           = if ngl   > 0 { ngl   } else { 999  };
    let par   = val_u32(raw, "parallel_slots");config.server.parallel_slots= if par   > 0 { par   } else { 2    };
    config.server.cont_batching = val_bool(raw, "cont_batching", true);

    let selected = val_str(raw, "selected_model");
    if !selected.trim().is_empty() {
        config.model.filename = std::path::Path::new(selected)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(selected)
            .to_string();
    }

    let _ = legacy; // 翻訳設定側フィールドは build_translation_profile が使う
    config
}

fn build_translation_profile(legacy: &LegacyConfig, profile_name: &str) -> TranslationProfile {
    let mut profile = if profile_name == "game" {
        TranslationProfile::game_default()
    } else {
        TranslationProfile::default()
    };

    profile.prompt_template = legacy.prompt_template.clone();

    if ["structural", "passthrough"].contains(&legacy.translation_mode.as_str()) {
        profile.translation_mode = legacy.translation_mode.clone();
    } else {
        log::warn!(
            "Unknown translation_mode '{}', using default",
            legacy.translation_mode
        );
    }

    profile.structural = StructuralOptions {
        protect_tags: legacy.structural.protect_tags,
        protect_brackets: legacy.structural.protect_brackets,
        protect_escaped_sequences: legacy.structural.protect_escaped_sequences,
        protect_placeholders: legacy.structural.protect_placeholders,
        split_symbolic_segments: legacy.structural.split_symbolic_segments,
    };
    profile.model_processing.enable_model_wrap = legacy.enable_model_wrap;
    profile.model_processing.model_wrap_min_chars = legacy.model_wrap_min_chars as usize;
    profile.model_processing.model_wrap_min_tail_chars = legacy.model_wrap_min_tail_chars as usize;
    profile.model_processing.enable_model_symbol_cleanup = legacy.enable_model_symbol_cleanup;

    profile
}

fn sanitize_profile_name(name: &str) -> String {
    if name.is_empty() {
        return "default".to_string();
    }

    let filtered: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();

    if filtered.is_empty() {
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        format!("profile_{:x}", hasher.finish())
    } else {
        filtered
    }
}