// src/logic.rs

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

fn llama_server_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

pub fn check_models(base_dir: &Path, cache: &mut Option<(bool, Instant)>) -> bool {
    let now = Instant::now();
    if let Some((result, timestamp)) = cache {
        if timestamp.elapsed() < Duration::from_secs(1) {
            return *result;
        }
    }

    let models_dir = base_dir.join("models");
    let has_models = models_dir.exists()
        && std::fs::read_dir(&models_dir)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.filter_map(|e| e.ok()))
            .any(|e| e.path().extension().map_or(false, |ext| ext == "gguf"));

    *cache = Some((has_models, now));
    has_models
}

pub fn check_llama_server(base_dir: &Path) -> bool {
    find_llama_server_exe(base_dir).is_some()
}

pub fn find_llama_server_exe(base_dir: &Path) -> Option<PathBuf> {
    let name = llama_server_binary_name();

    let runtime_dir = base_dir.join("runtime");
    if runtime_dir.exists() {
        let found = WalkDir::new(&runtime_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy() == name)
            .map(|e| e.path().to_path_buf());
        if found.is_some() {
            return found;
        }
    }

    let fallback_paths = [
        base_dir.join(name),
        base_dir.join("llama-server").join(name),
    ];
    for p in fallback_paths {
        if p.is_file() {
            return Some(p);
        }
    }
    None
}