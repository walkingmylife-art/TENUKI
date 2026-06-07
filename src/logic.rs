// src/logic.rs

use std::path::{Path, PathBuf};

pub fn check_llama_server(base_dir: &Path) -> bool {
    find_llama_server_exe(base_dir).is_some()
}

pub fn find_llama_server_exe(base_dir: &Path) -> Option<PathBuf> {
    let backend = authority_backend()?;
    let runtime_dir = base_dir.join("runtime").join(&backend);
    if !crate::launcher::runtime_downloader::runtime_is_complete(&runtime_dir, &backend) {
        return None;
    }
    crate::launcher::runtime_downloader::find_llama_server_exe(&runtime_dir)
}

fn authority_backend() -> Option<String> {
    let install_root = crate::launcher::resolve_install_root();
    let config_path = install_root.join("launcher_config.toml");
    let config = crate::launcher::app_config::AppConfig::load(&config_path).ok()?;
    if config.backend.trim().is_empty() {
        None
    } else {
        Some(config.backend)
    }
}
