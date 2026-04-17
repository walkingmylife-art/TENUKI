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

/// launcher_config.toml の model.filename が権威。
/// models/ に一致するファイルがあれば返す。なければ None（launcher が DL する）。
/// basename スキャンによる別ファイルへの fallback は行わない。
fn resolve_selected_model(filename: &str, models: &[PathBuf]) -> Option<PathBuf> {
    if filename.trim().is_empty() {
        return None;
    }
    let authority_name = PathBuf::from(filename);
    let authority_name = authority_name.file_name()?;
    models.iter().find(|m| m.file_name() == Some(authority_name)).cloned()
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
        let _ = event_tx.send(BackendEvent::SelectedModelResolved(selected_model.clone()));

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
                    FrontendCommand::SetLanguagePair { src, tgt, tgt_name, dict_slot } => {
                        let mut config = match crate::config::load(&config_path) {
                            Ok(config) => config,
                            Err(_) => return,
                        };
                        let slot_str = dict_slot.unwrap_or_else(|| {
                            manager::create_new_slot(&tgt, &base_dir)
                                .to_string_lossy()
                                .to_string()
                        });
                        config.src_lang = src.clone();
                        config.tgt_lang = tgt.clone();
                        config.custom_lang_name = tgt_name.unwrap_or_default();
                        config.dict_slot = Some(slot_str.clone());
                        let _ = crate::config::save(&config_path, &config);
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
                    FrontendCommand::SetModel(filename) => {
                        let install_root = crate::launcher::resolve_install_root();
                        let launcher_config_path = install_root.join("launcher_config.toml");
                        match crate::launcher::app_config::AppConfig::load(&launcher_config_path) {
                            Ok(mut app_cfg) => {
                                app_cfg.model.filename = filename.clone();
                                // known tuple で url / expected_size を原子的に整合
                                if let Some(known) = crate::launcher::app_config::known_model_tuple(&filename) {
                                    app_cfg.model.urls.primary = known.url.to_string();
                                    app_cfg.model.expected_size = known.expected_size;
                                }
                                if let Err(e) = app_cfg.save(&launcher_config_path) {
                                    let _ = event_tx.send(BackendEvent::Log(
                                        crate::messages::LogSource::Tenuki,
                                        format!("SetModel: save failed: {}", e),
                                        crate::messages::LogLevel::Error,
                                        crate::messages::current_timestamp(),
                                    ));
                                } else {
                                    let models = find_available_models(&base_dir);
                                    let selected = resolve_selected_model(&filename, &models);
                                    let _ = event_tx.send(BackendEvent::AvailableModels(models));
                                    let _ = event_tx.send(BackendEvent::SelectedModelResolved(selected.clone()));
                                    manager.selected_model = selected;
                                    manager.stop_all();
                                    manager.start_all();
                                    let _ = event_tx.send(BackendEvent::BackendReady {
                                        engine_success: manager.is_engine_running(),
                                        translator_success: manager.is_translation_server_running(),
                                    });
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.send(BackendEvent::Log(
                                    crate::messages::LogSource::Tenuki,
                                    format!("SetModel: load launcher_config failed: {}", e),
                                    crate::messages::LogLevel::Error,
                                    crate::messages::current_timestamp(),
                                ));
                            }
                        }
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
                        structural,
                        server_port,
                        server_host,
                    } => {
                        let mut scope = RestartScope::TranslatorOnly;
                        if let Some(structural_options) = structural {
                            manager.set_structural_options(structural_options);
                            if let Ok(config) = crate::config::load(&config_path) {
                                let _ = crate::config::save_profile_structural(
                                    &config_path, &config.profile, structural_options,
                                );
                            }
                        }
                        if let Some(port) = server_port {
                            if manager.set_server_port(port) {
                                scope = RestartScope::Full;
                                if let Ok(mut config) = crate::config::load(&config_path) {
                                    config.server_port = port;
                                    let _ = crate::config::save(&config_path, &config);
                                }
                            }
                        }
                        if let Some(host) = server_host {
                            if manager.set_server_host(&host) {
                                scope = RestartScope::Full;
                                if let Ok(mut config) = crate::config::load(&config_path) {
                                    config.server_host = host;
                                    let _ = crate::config::save(&config_path, &config);
                                }
                            }
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
