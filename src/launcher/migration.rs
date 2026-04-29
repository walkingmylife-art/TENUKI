//! Legacy `config.toml` migration helpers for the launcher.

use anyhow::{anyhow, Context, Result};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::path::Path;
use toml::Value;

use super::app_config::AppConfig;
use super::translation_profile::{GameTextOptions, TranslationProfile};
use crate::config::Config as LegacyConfig;

fn is_legacy_format(root: &Value) -> bool {
    let Some(table) = root.as_table() else {
        return false;
    };

    table.contains_key("prompt_template")
        || table.contains_key("translation_mode")
        || table.contains_key("mode")
        || table.contains_key("enable_model_wrap")
        || table.contains_key("structural")
        || table.contains_key("game_text")
}

fn log_migration_decision(message: &str) {
    log::info!("[migration] {}", message);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigProvision {
    Unchanged,
    ProvisionedMissing,
    RebuiltCurrentShape,
    MigratedLegacyShape,
}

pub(crate) fn provision_runtime_config_shape(config_path: &Path) -> Result<RuntimeConfigProvision> {
    let install_root = super::resolve_install_root();
    let launcher_config_path = install_root.join("launcher_config.toml");
    provision_runtime_config_for_startup_impl(config_path, &launcher_config_path)
}

pub(crate) fn migrate_config_if_needed(config_path: &Path) -> Result<bool> {
    if !config_path.exists() {
        return Ok(false);
    }

    let base_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("Invalid config path: no parent directory"))?;

    let install_root = super::resolve_install_root();
    let launcher_config_path = install_root.join("launcher_config.toml");
    if launcher_config_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    let root: Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse TOML in {}", config_path.display()))?;

    if !is_legacy_format(&root) {
        return Ok(false);
    }

    log_migration_decision(&format!(
        "detected legacy config format at {}",
        config_path.display()
    ));
    log_migration_decision(&format!(
        "launcher_config.toml missing; migration will provision {}",
        launcher_config_path.display()
    ));

    let legacy: LegacyConfig =
        toml::from_str(&content).with_context(|| "Failed to parse legacy config")?;

    backup_config(config_path, "toml.old")?;

    let launcher_config = build_app_config(&legacy, &root);
    launcher_config.save(&launcher_config_path)?;
    log_migration_decision(&format!(
        "committed migrated launcher_config.toml to {} with backend={}",
        launcher_config_path.display(),
        launcher_config.backend
    ));

    write_runtime_config_from_legacy(config_path, base_dir, &legacy, &root)?;

    Ok(true)
}

fn provision_runtime_config_for_startup_impl(
    config_path: &Path,
    launcher_config_path: &Path,
) -> Result<RuntimeConfigProvision> {
    if !config_path.exists() {
        log_migration_decision(&format!(
            "startup preflight: config.toml missing at {} — provisioning from current shape defaults",
            config_path.display()
        ));
        let config = crate::config::Config::new();
        crate::config::save(config_path, &config).with_context(|| {
            format!(
                "Failed to write default config.toml at {}",
                config_path.display()
            )
        })?;
        return Ok(RuntimeConfigProvision::ProvisionedMissing);
    }

    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let root: Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse TOML in {}", config_path.display()))?;

    if is_legacy_format(&root) {
        log_migration_decision(&format!(
            "startup preflight detected legacy runtime config at {}",
            config_path.display()
        ));

        let legacy: LegacyConfig =
            toml::from_str(&content).with_context(|| "Failed to parse legacy config")?;
        let base_dir = config_path
            .parent()
            .ok_or_else(|| anyhow!("Invalid config path: no parent directory"))?;

        if !launcher_config_path.exists() {
            let launcher_config = build_app_config(&legacy, &root);
            launcher_config.save(launcher_config_path)?;
            log_migration_decision(&format!(
                "startup preflight provisioned launcher authority at {} with backend={}",
                launcher_config_path.display(),
                launcher_config.backend
            ));
        } else {
            log_migration_decision(&format!(
                "startup preflight kept existing launcher authority at {} while rebuilding runtime config",
                launcher_config_path.display()
            ));
        }

        backup_config(config_path, "toml.old")?;
        write_runtime_config_from_legacy(config_path, base_dir, &legacy, &root)?;
        verify_runtime_config_loadable(config_path)?;
        return Ok(RuntimeConfigProvision::MigratedLegacyShape);
    }

    if crate::config::load(config_path).is_ok() {
        return Ok(RuntimeConfigProvision::Unchanged);
    }

    log_migration_decision(&format!(
        "startup preflight rebuilding current runtime config shape from {}",
        config_path.display()
    ));
    backup_config(config_path, "toml.rebuild.old")?;
    rebuild_runtime_config_from_observation(config_path, &root)?;
    verify_runtime_config_loadable(config_path)?;
    Ok(RuntimeConfigProvision::RebuiltCurrentShape)
}

fn verify_runtime_config_loadable(config_path: &Path) -> Result<()> {
    crate::config::load(config_path)
        .map(|_| ())
        .with_context(|| {
            format!(
                "Rebuilt config.toml is still not loadable: {}",
                config_path.display()
            )
        })
}

fn backup_config(config_path: &Path, extension: &str) -> Result<()> {
    let backup_path = config_path.with_extension(extension);
    std::fs::copy(config_path, &backup_path)
        .with_context(|| format!("Failed to backup config to {}", backup_path.display()))?;
    Ok(())
}

fn write_runtime_config_from_legacy(
    config_path: &Path,
    base_dir: &Path,
    legacy: &LegacyConfig,
    raw: &Value,
) -> Result<()> {
    let mut clean_config = legacy.clone();
    let old_code = val_str(raw, "custom_lang_code");
    if !old_code.trim().is_empty() {
        clean_config.tgt_lang = old_code.trim().to_string();
        clean_config.custom_lang_name = val_str(raw, "custom_lang_name").to_string();
    }
    crate::config::save(config_path, &clean_config)
        .with_context(|| "Failed to write updated config.toml")?;

    provision_profiles_from_legacy(base_dir, legacy)
}

fn provision_profiles_from_legacy(base_dir: &Path, legacy: &LegacyConfig) -> Result<()> {
    let profiles_dir = base_dir.join("profiles");
    std::fs::create_dir_all(&profiles_dir)?;

    let profile_name = sanitize_profile_name(&legacy.profile);
    let profile_path = profiles_dir.join(format!("{}.toml", profile_name));
    if !profile_path.exists() {
        let profile = build_translation_profile(legacy, &profile_name);
        profile.save(&profile_path)?;
    }

    let game_profile_path = profiles_dir.join("game.toml");
    if !game_profile_path.exists() {
        TranslationProfile::game_default().save(&game_profile_path)?;
    }

    let normal_profile_path = profiles_dir.join("normal.toml");
    if !normal_profile_path.exists() {
        TranslationProfile::normal_default().save(&normal_profile_path)?;
    }

    Ok(())
}

fn rebuild_runtime_config_from_observation(config_path: &Path, raw: &Value) -> Result<()> {
    let mut config = crate::config::Config::new();

    if let Some(src_lang) = val_opt_string(raw, "src_lang") {
        config.src_lang = src_lang;
    }

    if let Some(tgt_lang) =
        legacy_custom_target_code(raw).or_else(|| val_opt_string(raw, "tgt_lang"))
    {
        config.tgt_lang = tgt_lang;
    }

    if let Some(dict_slot) = val_opt_string(raw, "dict_slot") {
        config.dict_slot = Some(dict_slot);
    }

    if let Some(server_host) =
        val_opt_string(raw, "server_host").or_else(|| val_opt_string(raw, "llama_server_host"))
    {
        config.server_host = server_host;
    }

    if let Some(server_port) =
        val_opt_u16(raw, "server_port").or_else(|| val_opt_u16(raw, "llama_server_port"))
    {
        config.server_port = server_port;
    }

    if let Some(ui_lang) = val_opt_string(raw, "ui_lang") {
        config.ui_lang = ui_lang;
    }

    if let Some(custom_lang_name) = val_opt_string(raw, "custom_lang_name") {
        config.custom_lang_name = custom_lang_name;
    }

    if let Some(profile) = val_opt_string(raw, "profile") {
        config.profile = profile;
    }

    if let Some(language_models) = val_string_map(raw, "language_models") {
        config.language_models = language_models;
    }

    crate::config::save(config_path, &config).with_context(|| {
        format!(
            "Failed to rebuild current config shape at {}",
            config_path.display()
        )
    })
}

fn legacy_custom_target_code(raw: &Value) -> Option<String> {
    val_opt_string(raw, "custom_lang_code")
}

fn val_opt_string(root: &Value, key: &str) -> Option<String> {
    root.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn val_opt_u16(root: &Value, key: &str) -> Option<u16> {
    root.get(key)
        .and_then(|v| v.as_integer())
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn val_string_map(root: &Value, key: &str) -> Option<HashMap<String, String>> {
    let table = root.get(key)?.as_table()?;
    let mut out = HashMap::new();
    for (map_key, map_value) in table {
        if let Some(value) = map_value.as_str() {
            out.insert(map_key.clone(), value.to_string());
        }
    }
    Some(out)
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

    if let Some(backend) = legacy_backend_for_migration(raw) {
        log_migration_decision(&format!(
            "adopting legacy backend '{}' because legacy config explicitly declared backend",
            backend
        ));
        config.backend = backend;
    }

    let llama_host = val_str(raw, "llama_server_host");
    if !llama_host.is_empty() {
        config.server.host = llama_host.to_string();
    }
    let llama_port = val_u32(raw, "llama_server_port");
    if llama_port != 0 {
        config.server.port = llama_port as u16;
    }

    let ctx = val_u32(raw, "ctx_size");
    config.server.ctx_size = if ctx > 0 { ctx } else { 1024 };
    let batch = val_u32(raw, "batch_size");
    config.server.batch_size = if batch > 0 { batch } else { 128 };
    let ub = val_u32(raw, "ubatch_size");
    config.server.ubatch_size = if ub > 0 { ub } else { 64 };
    let ngl = val_u32(raw, "ngl");
    config.server.ngl = if ngl > 0 { ngl } else { 999 };
    let par = val_u32(raw, "parallel_slots");
    config.server.parallel_slots = if par > 0 { par } else { 2 };
    config.server.cont_batching = val_bool(raw, "cont_batching", true);

    let _ = legacy;
    config
}

fn legacy_backend_for_migration(raw: &Value) -> Option<String> {
    let backend = val_str(raw, "backend").trim();
    if backend.is_empty() {
        None
    } else {
        Some(backend.to_string())
    }
}

fn build_translation_profile(legacy: &LegacyConfig, profile_name: &str) -> TranslationProfile {
    let mut profile = if profile_name == "game" {
        TranslationProfile::game_default()
    } else {
        TranslationProfile::default()
    };

    profile.prompt_template = legacy.prompt_template.clone();

    profile.mode = crate::config::normalize_mode_value(&legacy.mode);

    profile.game_text = GameTextOptions {
        protect_tags: legacy.game_text.protect_tags,
        protect_brackets: legacy.game_text.protect_brackets,
        protect_escaped_sequences: legacy.game_text.protect_escaped_sequences,
        protect_placeholders: legacy.game_text.protect_placeholders,
        split_symbolic_segments: legacy.game_text.split_symbolic_segments,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::app_config::AppConfig;
    use std::fs;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tenuki_mig_{}", tag));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- is_legacy_format ---

    #[test]
    fn legacy_format_detected_by_prompt_template_key() {
        let toml: Value = toml::from_str(r#"prompt_template = "Translate {target}""#).unwrap();
        assert!(is_legacy_format(&toml));
    }

    #[test]
    fn legacy_format_detected_by_legacy_mode_key() {
        let toml: Value = toml::from_str(r#"translation_mode = "structural""#).unwrap();
        assert!(is_legacy_format(&toml));
    }

    #[test]
    fn non_legacy_format_returns_false() {
        let toml: Value = toml::from_str(r#"backend = "cuda""#).unwrap();
        assert!(!is_legacy_format(&toml));
    }

    // --- build_app_config ---

    fn minimal_legacy_config() -> crate::config::Config {
        crate::config::Config::new()
    }

    #[test]
    fn build_app_config_sets_backend_from_raw() {
        let raw: Value = toml::from_str(
            r#"
            prompt_template = "x"
            backend = "vulkan"
            "#,
        )
        .unwrap();
        let legacy = minimal_legacy_config();
        let cfg = build_app_config(&legacy, &raw);
        assert_eq!(cfg.backend, "vulkan");
    }

    #[test]
    fn build_app_config_empty_backend_uses_default() {
        let raw: Value = toml::from_str(r#"prompt_template = "x""#).unwrap();
        let legacy = minimal_legacy_config();
        let cfg = build_app_config(&legacy, &raw);
        assert_eq!(cfg.backend, AppConfig::default().backend);
    }

    // --- migrate_config_if_needed: existing launcher_config blocks migration ---
    // NOTE: この統合テストは resolve_install_root() に依存するため、
    // dev 環境で launcher_config.toml が存在すると常に Ok(false) を返す。
    // path injection seam を追加後に完全なテストに置き換える。

    /// migrate_config_if_needed は config_path が存在しなければ Ok(false)
    #[test]
    fn migrate_returns_false_when_config_missing() {
        let dir = temp_dir("mig_no_config");
        let path = dir.join("nonexistent_config.toml");
        let result = migrate_config_if_needed(&path).unwrap();
        assert!(!result, "missing config must not trigger migration");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 非 legacy TOML (legacy キー不在) では migration しない
    #[test]
    fn migrate_skips_non_legacy_toml() {
        let dir = temp_dir("mig_non_legacy");
        let path = dir.join("config.toml");
        fs::write(&path, r#"backend = "cuda""#).unwrap();
        // launcher_config.toml が install_root に存在する限り Ok(false) だが、
        // 仮に存在しなくても is_legacy_format == false で Ok(false)
        // → どちらのパスでも migration は走らない
        let result = migrate_config_if_needed(&path).unwrap();
        assert!(!result, "non-legacy config must not trigger migration");
        let _ = fs::remove_dir_all(&dir);
    }

    // --- default backend 差し戻し経路の可視化 ---
    // migration.rs lines 83-88:
    //   if lc.backend == AppConfig::default().backend {
    //       lc.backend = val_str(&root, "backend"); // legacy backend で上書き
    //   }
    // これは「launcher_config に default backend が入っていたとき、
    // legacy backend で差し戻す」経路。
    // 意図: 移行直後の launcher_config は default(cuda) になるが、
    //       legacy に別の backend が記録されていれば尊重する。
    // リスク: launcher_config を意図的に cuda にしても legacy が vulkan なら vulkan に戻される。
    //
    // TODO: migrate_impl(config_path, launcher_config_path) seam を追加して
    //       「legacy backend=vulkan → launcher_config に vulkan が書き込まれる」を
    //       integration test で固定する。
    #[test]
    fn default_backend_overwrite_route_is_documented() {
        // build_app_config は backend を raw から取る。
        // raw に backend がなければ AppConfig::default().backend を保持。
        // その後 migrate_config_if_needed の 83-88 行でさらに上書きされる経路が存在する。
        let raw_with_vulkan: Value =
            toml::from_str("prompt_template = \"x\"\nbackend = \"vulkan\"").unwrap();
        let raw_no_backend: Value = toml::from_str(r#"prompt_template = "x""#).unwrap();

        let legacy = minimal_legacy_config();
        let cfg_vulkan = build_app_config(&legacy, &raw_with_vulkan);
        let cfg_default = build_app_config(&legacy, &raw_no_backend);

        assert_eq!(cfg_vulkan.backend, "vulkan");
        assert_eq!(cfg_default.backend, AppConfig::default().backend);
        // 差し戻し経路: cfg_default.backend == default() であれば
        // migrate_config_if_needed が raw の backend で上書きする
    }
}
