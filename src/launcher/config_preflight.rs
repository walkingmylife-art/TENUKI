use anyhow::{anyhow, Context, Result};
use std::path::Path;

use super::migration::{self, RuntimeConfigProvision};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfigPreflight {
    pub shape: RuntimeConfigProvision,
    pub dict_slot_committed: bool,
}

pub fn preflight_runtime_config_for_startup(config_path: &Path) -> Result<RuntimeConfigPreflight> {
    let shape = migration::provision_runtime_config_shape(config_path)?;
    let mut config = crate::config::load(config_path).with_context(|| {
        format!(
            "config.toml is not loadable after shape preflight: {}",
            config_path.display()
        )
    })?;

    let base_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("Invalid config path: no parent directory"))?
        .to_path_buf();
    let slot_path = crate::backend::manager::provision_slot_dir(&config, &base_dir);
    let slot_string = slot_path.to_string_lossy().to_string();

    let dict_slot_committed = config
        .dict_slot
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        != Some(slot_string.as_str());

    if dict_slot_committed {
        log::info!(
            "[config_preflight] committing dict_slot='{}' to {}",
            slot_string,
            config_path.display()
        );
        config.dict_slot = Some(slot_string);
        crate::config::save(config_path, &config).with_context(|| {
            format!(
                "Failed to save preflighted config.toml with dict_slot: {}",
                config_path.display()
            )
        })?;
        crate::config::load(config_path).with_context(|| {
            format!(
                "config.toml became invalid after dict_slot preflight: {}",
                config_path.display()
            )
        })?;
    }

    Ok(RuntimeConfigPreflight {
        shape,
        dict_slot_committed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tenuki_preflight_{}_{}", tag, unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn current_shape_preflight_commits_missing_dict_slot() {
        let dir = temp_dir("dict_slot");
        let config_path = dir.join("config.toml");

        fs::write(
            &config_path,
            r#"
src_lang = "en"
tgt_lang = "ja"
profile = "game"
"#,
        )
        .unwrap();

        let outcome = preflight_runtime_config_for_startup(&config_path).unwrap();
        assert_eq!(outcome.shape, RuntimeConfigProvision::Unchanged);
        assert!(outcome.dict_slot_committed);

        let config = crate::config::load(&config_path).unwrap();
        let slot = config.dict_slot.expect("dict_slot should be committed");
        assert!(std::path::Path::new(&slot).exists());
        assert!(slot.contains("dicts"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preflight_missing_config_provisions_and_commits_dict_slot() {
        let dir = temp_dir("missing");
        let config_path = dir.join("config.toml");

        assert!(!config_path.exists());
        let outcome = preflight_runtime_config_for_startup(&config_path).unwrap();
        assert_eq!(outcome.shape, RuntimeConfigProvision::ProvisionedMissing);

        let config = crate::config::load(&config_path).unwrap();
        let slot = config
            .dict_slot
            .expect("dict_slot should be committed after provisioning");
        assert!(std::path::Path::new(&slot).exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
