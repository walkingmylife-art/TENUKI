//! 辞書スロットのディレクトリ命名規則・発見・作成ユーティリティ

use std::path::{Path, PathBuf};

use crate::config::Config;

fn max_existing_slot_num(text_dir: &Path, lang: &str) -> Option<u32> {
    std::fs::read_dir(text_dir)
        .ok()?
        .filter_map(|e| {
            let e = e.ok()?;
            if !e.file_type().ok()?.is_dir() {
                return None;
            }

            let name = e.file_name().to_string_lossy().into_owned();
            current_slot_num_from_name(&name, lang)
        })
        .max()
}

pub fn is_slot_dir_name_for_lang(name: &str, lang: &str) -> bool {
    compatible_slot_num_from_name(name, lang).is_some()
}

fn current_slot_num_from_name(name: &str, lang: &str) -> Option<u32> {
    let (prefix, suffix) = name.rsplit_once('_')?;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let is_current_lang_slot = !lang.is_empty() && prefix == lang;

    if is_current_lang_slot {
        suffix.parse::<u32>().ok()
    } else if lang.is_empty()
        && !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        suffix.parse::<u32>().ok()
    } else {
        None
    }
}

fn compatible_slot_num_from_name(name: &str, lang: &str) -> Option<u32> {
    let current = current_slot_num_from_name(name, lang);
    if current.is_some() {
        return current;
    }

    let (prefix, suffix) = name.rsplit_once('_')?;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let is_legacy_slot = prefix == "S" && suffix.len() == 4;
    if !lang.is_empty() && is_legacy_slot {
        suffix.parse::<u32>().ok()
    } else {
        None
    }
}

fn find_slot_ancestor(path: &Path, lang: &str) -> Option<PathBuf> {
    path.ancestors()
        .filter(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| is_slot_dir_name_for_lang(name, lang))
        })
        .last()
        .map(Path::to_path_buf)
}

pub fn dict_slot_matches_target(slot: &Path, target_lang: &str) -> bool {
    let target_lang = target_lang.trim();
    if target_lang.is_empty() {
        return false;
    }

    slot.ancestors().any(|candidate| {
        let Some(name) = candidate.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        if compatible_slot_num_from_name(name, target_lang).is_none() {
            return false;
        }

        let Some(text_dir) = candidate.parent() else {
            return false;
        };
        let Some(lang_dir) = text_dir.parent() else {
            return false;
        };
        let Some(dicts_dir) = lang_dir.parent() else {
            return false;
        };

        text_dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("text"))
            .unwrap_or(false)
            && lang_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == target_lang)
                .unwrap_or(false)
            && dicts_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("dicts"))
                .unwrap_or(false)
    })
}

pub fn is_slot_dir(p: &Path) -> bool {
    let parent_is_text = p
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("text"))
        .unwrap_or(false);

    parent_is_text
        && p.file_name()
            .and_then(|n| n.to_str())
            .map(|name| compatible_slot_num_from_name(name, "").is_some())
            .unwrap_or(false)
}

fn find_existing_slot_under(container: &Path, lang: &str) -> Option<PathBuf> {
    find_slot_ancestor(container, lang)
}

pub fn provision_slot_under(container: &Path, lang: &str) -> PathBuf {
    if let Some(existing_slot) = find_existing_slot_under(container, lang) {
        let _ = std::fs::create_dir_all(&existing_slot);
        return existing_slot;
    }

    let _ = std::fs::create_dir_all(container);
    if let Some(max) = max_existing_slot_num(container, lang) {
        let next_num = max + 1;
        let slot = container.join(format!("{}_{:03}", lang, next_num));
        let _ = std::fs::create_dir_all(&slot);
        slot
    } else {
        let slot = container.join(format!("{}_001", lang));
        let _ = std::fs::create_dir_all(&slot);
        slot
    }
}

pub fn find_or_create_slot_under(container: &Path, lang: &str) -> PathBuf {
    provision_slot_under(container, lang)
}

pub fn create_new_slot(tgt_lang: &str, base_dir: &PathBuf) -> PathBuf {
    let text_dir = base_dir.join("dicts").join(tgt_lang).join("text");
    let _ = std::fs::create_dir_all(&text_dir);
    let next_num = max_existing_slot_num(&text_dir, tgt_lang)
        .map(|n| n + 1)
        .unwrap_or(1);
    let slot = text_dir.join(format!("{}_{:03}", tgt_lang, next_num));
    let _ = std::fs::create_dir_all(&slot);
    slot
}

pub fn resolve_lang_pair_dict_slot(
    dict_slot: Option<&str>,
    target_lang: &str,
    base_dir: &PathBuf,
) -> String {
    dict_slot
        .map(str::trim)
        .filter(|slot| !slot.is_empty())
        .filter(|slot| dict_slot_matches_target(Path::new(slot), target_lang))
        .map(str::to_string)
        .unwrap_or_else(|| {
            create_new_slot(target_lang, base_dir)
                .to_string_lossy()
                .to_string()
        })
}

fn resolve_explicit_slot_dir(config: &Config) -> Option<PathBuf> {
    if let Some(slot) = &config.dict_slot {
        if !slot.is_empty() {
            return Some(PathBuf::from(slot));
        }
    }
    None
}

pub fn provision_slot_dir(config: &Config, base_dir: &PathBuf) -> PathBuf {
    if let Some(p) = resolve_explicit_slot_dir(config) {
        let _ = std::fs::create_dir_all(&p);
        return p;
    }

    let text_dir = base_dir.join("dicts").join(&config.tgt_lang).join("text");
    provision_slot_under(&text_dir, &config.tgt_lang)
}

pub fn resolve_slot_dir(config: &Config, base_dir: &PathBuf) -> PathBuf {
    if resolve_explicit_slot_dir(config).is_none() {
        log::error!(
            "[manager] dict_slot が未確定のまま resolve_slot_dir が呼ばれました。preflight が通っていない可能性があります。tgt_lang={}",
            config.tgt_lang
        );
    }
    provision_slot_dir(config, base_dir)
}

pub fn get_exact_dict_path(config: &Config, base_dir: &PathBuf) -> PathBuf {
    resolve_slot_dir(config, base_dir).join("Tenuki.dict.txt")
}

pub fn get_regex_dict_path(config: &Config, base_dir: &PathBuf) -> PathBuf {
    resolve_slot_dir(config, base_dir).join("Tenuki.regex.txt")
}

pub fn get_split_dict_path(config: &Config, base_dir: &PathBuf) -> PathBuf {
    resolve_slot_dir(config, base_dir).join("Tenuki.split.txt")
}

pub fn get_dict_path(config: &Config, base_dir: &PathBuf) -> PathBuf {
    get_exact_dict_path(config, base_dir)
}

pub fn get_bin_path(_config: &Config, base_dir: &PathBuf) -> PathBuf {
    let dir = base_dir.join("dicts");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("dict.bin")
}
