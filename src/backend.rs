//! バックエンドモジュール

mod logger;
pub(crate) mod process;
pub mod manager;
mod analysis;
mod dictionary;
mod normalize;
pub mod translator;
mod server;
pub mod processor;

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::messages::{BackendEvent, FrontendCommand, ProcessType};
use crate::config::Config;
use crate::launcher::app_config::AppConfig;
use manager::{ProcessManager, RestartScope};

pub use processor::TranslationMode;

fn resolve_selected_model(filename: &str, models: &[PathBuf]) -> Option<PathBuf> {
    if !filename.trim().is_empty() {
        let saved_path = PathBuf::from(filename);
        if let Some(found) = models.iter().find(|m| **m == saved_path) {
            return Some(found.clone());
        }
        if let Some(saved_name) = saved_path.file_name() {
            if let Some(found) = models.iter().find(|m| m.file_name() == Some(saved_name)) {
                return Some(found.clone());
            }
        }
    }
    models.first().cloned()
}

fn resolve_dict_slot_for_language_pair(
    config: &Config,
    base_dir: &PathBuf,
    tgt: &str,
    keep_dict: bool,
) -> String {
    if !keep_dict {
        return manager::create_new_slot(tgt, base_dir)
            .to_string_lossy()
            .to_string();
    }

    if config.tgt_lang == tgt {
        return config.dict_slot.clone().unwrap_or_else(|| {
            manager::find_or_create_slot_under(
                &base_dir.join("dicts").join(tgt).join("text"),
                tgt,
            )
            .to_string_lossy()
            .to_string()
        });
    }

    manager::find_or_create_slot_under(&base_dir.join("dicts").join(tgt).join("text"), tgt)
        .to_string_lossy()
        .to_string()
}

pub fn find_available_models(base_dir: &PathBuf) -> Vec<PathBuf> {
    let models_dir = base_dir.join("models");
    let mut models = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "gguf").unwrap_or(false) {
                models.push(path);
            }
        }
    }
    models.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    models
}

pub fn start_backend(
    config: Config,
    app_config: AppConfig,
    base_dir: PathBuf,
    shutdown: Arc<AtomicBool>,
    event_tx: mpsc::Sender<BackendEvent>,
    command_rx: mpsc::Receiver<FrontendCommand>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let models = find_available_models(&base_dir);
        let _ = event_tx.send(BackendEvent::AvailableModels(models.clone()));
        let selected_model = resolve_selected_model(&app_config.model.filename, &models);

        // dict_slot 初期化
        let config = {
            let raw = config.dict_slot.as_deref().map(str::trim).unwrap_or("");
            let resolved_slot = if raw.is_empty() {
                Some(manager::find_or_create_slot_under(
                    &base_dir.join("dicts").join(&config.tgt_lang).join("text"),
                    &config.tgt_lang,
                ))
            } else {
                let _ = std::fs::create_dir_all(raw);
                None
            };
            if let Some(slot) = resolved_slot {
                let slot_str = slot.to_string_lossy().to_string();
                let dict_msg = if config.ui_lang == "en" {
                    format!("Dictionary folder initialized: {}", slot_str)
                } else {
                    format!("辞書フォルダを初期化しました: {}", slot_str)
                };
                let _ = event_tx.send(BackendEvent::Log(
                    crate::messages::LogSource::Tenuki,
                    dict_msg,
                    crate::messages::LogLevel::Info,
                    crate::messages::current_timestamp(),
                ));
                let _ = event_tx.send(BackendEvent::DictSlotChanged(slot_str.clone()));
                let config_path = base_dir.join("config.toml");
                let mut new_config = config;
                new_config.dict_slot = Some(slot_str);
                let _ = crate::config::save(&config_path, &new_config);
                new_config
            } else {
                config
            }
        };

        let mut manager = ProcessManager::new(
            config.clone(),
            app_config.server.clone(),
            base_dir.clone(),
            event_tx.clone(),
            selected_model,
            shutdown.clone(),
        );

        let _ = event_tx.send(BackendEvent::Log(
            crate::messages::LogSource::Tenuki,
            "TENUKI Backend starting...".to_string(),
            crate::messages::LogLevel::Info,
            crate::messages::current_timestamp(),
        ));

        let config_path = base_dir.join("config.toml");
        let ui_lang = config.ui_lang.clone();
        let mut last_metrics_poll = std::time::Instant::now();

        while !shutdown.load(Ordering::Relaxed) {
            // エンジンメトリクスを3秒ごとに送信
            if last_metrics_poll.elapsed() >= Duration::from_secs(3) {
                last_metrics_poll = std::time::Instant::now();
                if let Some((tps, vram, shared)) = manager.poll_metrics() {
                    let _ = event_tx.send(BackendEvent::ServerMetrics {
                        vram_mb: vram,
                        shared_mb: shared,
                        tokens_per_second: tps,
                    });
                }
            }

            if let Ok(cmd) = command_rx.recv_timeout(Duration::from_millis(100)) {
                match cmd {
                    FrontendCommand::Start => {
                        manager.start_all();
                    }
                    FrontendCommand::Stop => {
                        manager.stop_all();
                    }
                    FrontendCommand::Restart => {
                        manager.stop_all();
                        manager.start_all();
                    }
                    FrontendCommand::SetLanguagePair { src, tgt, keep_dict } => {
                        let mut config = match crate::config::load(&config_path) {
                            Ok(config) => config,
                            Err(_) => return,
                        };
                        let slot_str =
                            resolve_dict_slot_for_language_pair(&config, &base_dir, &tgt, keep_dict);
                        config.src_lang = src.clone();
                        config.tgt_lang = tgt.clone();
                        config.dict_slot = Some(slot_str.clone());
                        config.enable_model_wrap = tgt != "ar";
                        let _ = crate::config::save(&config_path, &config);
                        manager.set_enable_model_wrap(tgt != "ar");
                        manager.reload_config(&config_path);
                        let _ = event_tx.send(BackendEvent::DictSlotChanged(slot_str));
                        let (engine_ok, translator_ok) =
                            manager.apply_restart(RestartScope::TranslatorOnly);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                        let _ = event_tx.send(BackendEvent::LanguageChanged(tgt));
                    }
                    FrontendCommand::SetCustomLanguage { code, name } => {
                        if let Ok(mut config) = crate::config::load(&config_path) {
                            config.custom_lang_code = code.clone();
                            config.custom_lang_name = name.clone();
                            let _ = crate::config::save(&config_path, &config);
                        }
                        manager.reload_config(&config_path);
                        let (engine_ok, translator_ok) =
                            manager.apply_restart(RestartScope::TranslatorOnly);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                        let display_name = if name.trim().is_empty() { code.clone() }
                            else { format!("{} ({})", name, code) };
                        let lang_msg = if ui_lang == "en" {
                            format!("Custom language updated: {}", display_name)
                        } else {
                            format!("カスタム言語を更新しました: {}", display_name)
                        };
                        let _ = event_tx.send(BackendEvent::Log(
                            crate::messages::LogSource::Tenuki, lang_msg,
                            crate::messages::LogLevel::Info, crate::messages::current_timestamp(),
                        ));
                    }
                    FrontendCommand::SetProfile(profile_name) => {
                        if let Ok(mut config) = crate::config::load(&config_path) {
                            config.profile = profile_name.clone();
                            let _ = crate::config::save(&config_path, &config);
                        }
                        manager.reload_config(&config_path);
                        let (engine_ok, translator_ok) =
                            manager.apply_restart(RestartScope::TranslatorOnly);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                        let profile_msg = if ui_lang == "en" {
                            format!("Profile changed: {}", profile_name)
                        } else {
                            format!("プロファイルを変更しました: {}", profile_name)
                        };
                        let _ = event_tx.send(BackendEvent::Log(
                            crate::messages::LogSource::Tenuki, profile_msg,
                            crate::messages::LogLevel::Info, crate::messages::current_timestamp(),
                        ));
                    }
                    FrontendCommand::SetDictSlot(slot) => {
                        let selected_slot = slot.map(|raw| {
                            let path = std::path::PathBuf::from(raw.trim());
                            let _ = std::fs::create_dir_all(&path);
                            path.to_string_lossy().to_string()
                        });
                        if let Ok(mut config) = crate::config::load(&config_path) {
                            config.dict_slot = selected_slot.clone();
                            let _ = crate::config::save(&config_path, &config);
                        }
                        manager.reload_config(&config_path);
                        let (engine_ok, translator_ok) =
                            manager.apply_restart(RestartScope::TranslatorOnly);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                        if let Some(s) = &selected_slot {
                            let _ = event_tx.send(BackendEvent::DictSlotChanged(s.clone()));
                        }
                    }
                    FrontendCommand::UpdateSettings {
                        ctx_size,
                        model,
                        structural,
                        translation_mode,
                        server_port,
                        server_host,
                    } => {
                        let mut scope = RestartScope::TranslatorOnly;

                        if let Some(mode) = translation_mode {
                            if let Ok(mut config) = crate::config::load(&config_path) {
                                config.translation_mode = mode.clone();
                                let _ = crate::config::save(&config_path, &config);
                            }
                            // server_cfg の差し替えを含むため Full 再起動が必要
                            manager.reload_config(&config_path);
                            scope = RestartScope::Full;
                        }
                        if let Some(structural_options) = structural {
                            manager.set_structural_options(structural_options);
                            if let Ok(mut config) = crate::config::load(&config_path) {
                                config.structural = structural_options;
                                let _ = crate::config::save(&config_path, &config);
                            }
                        }
                        if let Some(port) = server_port {
                            manager.set_server_port(port);
                            scope = RestartScope::Full;
                            if let Ok(mut config) = crate::config::load(&config_path) {
                                config.server_port = port;
                                let _ = crate::config::save(&config_path, &config);
                            }
                        }
                        if let Some(host) = server_host {
                            manager.set_server_host(&host);
                            scope = RestartScope::Full;
                            if let Ok(mut config) = crate::config::load(&config_path) {
                                config.server_host = host;
                                let _ = crate::config::save(&config_path, &config);
                            }
                        }
                        if model.is_some() || ctx_size.is_some() {
                            scope = RestartScope::Full;
                            if let Some(model_path) = model.as_ref() {
                                let launcher_config_path = config_path.parent().unwrap().join("launcher_config.toml");
                                if let Ok(mut app_config) = AppConfig::load(&launcher_config_path) {
                                    app_config.model.filename = model_path.file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let _ = app_config.save(&launcher_config_path);
                                }
                            }
                            manager.update_settings(ctx_size, model);
                        }

                        let _ = manager.save_dictionary();
                        let (engine_ok, translator_ok) = manager.apply_restart(scope);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                    }
                }
            }
            manager.check_alive();
        }

        // サーバーを先に停止（stop_translation_server 内で n_cache → dict.buffer へ登録）
        manager.stop_all();
        // サーバー停止後に TXT へ書き込む（dict.write() の競合なし）
        let _ = manager.save_dictionary();

        let _ = event_tx.send(BackendEvent::ProcessStatus(ProcessType::Tenuki, false));
        let _ = event_tx.send(BackendEvent::Log(
            crate::messages::LogSource::Tenuki,
            "TENUKI Backend stopped".to_string(),
            crate::messages::LogLevel::Info,
            crate::messages::current_timestamp(),
        ));
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_dict_slot_for_language_pair;
    use crate::config::Config;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_base_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("tenuki_backend_slot_test_{}", unique))
    }

    #[test]
    fn keeps_existing_slot_when_language_is_unchanged() {
        let base_dir = unique_base_dir();
        let slot = base_dir.join("dicts").join("ja").join("text").join("ja_003");
        std::fs::create_dir_all(&slot).unwrap();

        let mut config = Config::new();
        config.tgt_lang = "ja".to_string();
        config.dict_slot = Some(slot.to_string_lossy().to_string());

        let resolved = resolve_dict_slot_for_language_pair(&config, &base_dir, "ja", true);
        assert_eq!(resolved, slot.to_string_lossy().to_string());

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn switches_to_target_language_slot_when_language_changes() {
        let base_dir = unique_base_dir();
        let ja_slot = base_dir.join("dicts").join("ja").join("text").join("ja_003");
        std::fs::create_dir_all(&ja_slot).unwrap();

        let mut config = Config::new();
        config.tgt_lang = "ja".to_string();
        config.dict_slot = Some(ja_slot.to_string_lossy().to_string());

        let resolved = resolve_dict_slot_for_language_pair(&config, &base_dir, "en", true);
        let expected = base_dir.join("dicts").join("en").join("text").join("en_001");

        assert_eq!(resolved, expected.to_string_lossy().to_string());
        assert!(expected.is_dir());

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn creates_fresh_slot_when_keep_dict_is_disabled() {
        let base_dir = unique_base_dir();
        let existing = base_dir.join("dicts").join("en").join("text").join("en_001");
        std::fs::create_dir_all(&existing).unwrap();

        let mut config = Config::new();
        config.tgt_lang = "en".to_string();
        config.dict_slot = Some(existing.to_string_lossy().to_string());

        let resolved = resolve_dict_slot_for_language_pair(&config, &base_dir, "en", false);
        let expected = base_dir.join("dicts").join("en").join("text").join("en_002");

        assert_eq!(resolved, expected.to_string_lossy().to_string());
        assert!(expected.is_dir());

        let _ = std::fs::remove_dir_all(&base_dir);
    }
}
