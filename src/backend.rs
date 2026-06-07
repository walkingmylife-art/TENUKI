//! バックエンドモジュール

//!
//! Backend runtime wiring and public module boundary.
//!
//! The backend adopts committed launcher/config authority and starts the
//! inference engine plus TENUKI entry server. Input analysis is produced from
//! translation completion payloads and replayed from saved snapshots; there is
//! no public processor module in the backend live path.

mod analysis;
mod dictionary;
pub(crate) mod engine;
mod logger;
pub mod manager;
pub(crate) mod metrics;
mod normalize;
pub(crate) mod pdh_vram;
pub(crate) mod process;
mod server;
pub(crate) mod slot;
pub mod translator;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::launcher::app_config::{known_model_tuple, AppConfig};
use crate::messages::{
    BackendEvent, FrontendCommand, ModelCandidate, ModelCandidateKind, ProcessType,
};
use manager::{ProcessManager, RestartScope};

/// launcher_config.toml の model.filename が権威。
/// candidates の中に一致するファイルがあれば PathBuf を返す。なければ None。
pub(crate) fn resolve_authority_model(
    app_config: &AppConfig,
    candidates: &[ModelCandidate],
) -> Option<PathBuf> {
    let expected_size = app_config.model.expected_size();
    candidates
        .iter()
        .find(|candidate| {
            candidate.filename == app_config.model.filename() && candidate.size == expected_size
        })
        .map(|candidate| candidate.path.clone())
}

/// setup/recovery で使う既存救済込みの解決。
/// normal startup の authority exact match だけが必要な箇所では
/// resolve_authority_model を使う。
pub(crate) fn resolve_startup_model(
    app_config: &AppConfig,
    candidates: &[ModelCandidate],
) -> Option<PathBuf> {
    if let Some(authority) = resolve_authority_model(app_config, candidates) {
        return Some(authority);
    }

    let usable = candidates
        .iter()
        .filter(|candidate| candidate.size > 0)
        .collect::<Vec<_>>();
    if usable.len() == 1 {
        return Some(usable[0].path.clone());
    }

    None
}

pub fn find_available_models(base_dir: &PathBuf) -> Vec<ModelCandidate> {
    let models_dir = base_dir.join("models");
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.extension().map(|e| e == "gguf").unwrap_or(false) {
                continue;
            }
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let kind = if known_model_tuple(&filename).is_some() {
                ModelCandidateKind::Known
            } else {
                ModelCandidateKind::Local
            };
            candidates.push(ModelCandidate {
                filename,
                path,
                size,
                kind,
            });
        }
    }
    candidates.sort_by(|a, b| a.filename.cmp(&b.filename));
    candidates
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
        let selected_model = resolve_startup_model(&app_config, &models);
        let _ = event_tx.send(BackendEvent::SelectedModelResolved(selected_model.clone()));

        // dict_slot は preflight で commit 済みの authority。backend は読むだけ。
        match config
            .dict_slot
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(slot) => {
                let _ = std::fs::create_dir_all(slot);
            }
            None => {
                let _ = event_tx.send(BackendEvent::Log(
                    crate::messages::LogSource::Tenuki,
                    "dict_slot が未確定です。preflight が通っていない可能性があります。"
                        .to_string(),
                    crate::messages::LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
            }
        }

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
                    FrontendCommand::SetLanguagePair {
                        src,
                        tgt,
                        tgt_name,
                        dict_slot,
                    } => {
                        let mut config = match crate::config::load(&config_path) {
                            Ok(config) => config,
                            Err(_) => return,
                        };
                        config.src_lang = src.clone();
                        config.tgt_lang = tgt.clone();
                        config.custom_lang_name = tgt_name.unwrap_or_default();
                        config.dict_slot = Some(dict_slot.clone());
                        let _ = crate::config::save(&config_path, &config);
                        manager.reload_config(&config_path);
                        let _ = event_tx.send(BackendEvent::DictSlotChanged(dict_slot));
                        let (engine_ok, translator_ok) =
                            manager.apply_restart(RestartScope::TranslatorOnly);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                        let _ = event_tx.send(BackendEvent::LanguageChanged(tgt));
                    }
                    FrontendCommand::CommitModelSelection(model_config) => {
                        let install_root = crate::launcher::resolve_install_root();
                        let launcher_config_path = install_root.join("launcher_config.toml");
                        match AppConfig::load(&launcher_config_path) {
                            Ok(mut app_cfg) => {
                                // backend は adopt して save するだけ。URL/size の再推測禁止。
                                app_cfg.model = model_config;
                                if let Err(e) = app_cfg.save(&launcher_config_path) {
                                    let _ = event_tx.send(BackendEvent::Log(
                                        crate::messages::LogSource::Tenuki,
                                        format!("CommitModelSelection: save failed: {}", e),
                                        crate::messages::LogLevel::Error,
                                        crate::messages::current_timestamp(),
                                    ));
                                } else {
                                    let models = find_available_models(&base_dir);
                                    let selected = resolve_authority_model(&app_cfg, &models);
                                    let _ = event_tx.send(BackendEvent::AvailableModels(models));
                                    let _ = event_tx.send(BackendEvent::SelectedModelResolved(
                                        selected.clone(),
                                    ));
                                    manager.check_alive();
                                    let engine_was_running = manager.is_engine_running();
                                    manager.set_selected_model(selected);

                                    let title = if ui_lang == "en" {
                                        "Model selected".to_string()
                                    } else {
                                        "モデルを選択しました".to_string()
                                    };
                                    if engine_was_running {
                                        let _ = event_tx.send(BackendEvent::StatusNotice {
                                            title,
                                            message: if ui_lang == "en" {
                                                "Restarting backend to apply the selected model..."
                                                    .to_string()
                                            } else {
                                                "選択したモデルを適用するため再起動しています..."
                                                    .to_string()
                                            },
                                        });
                                        manager.restart_for_model_switch();
                                    } else {
                                        let _ = event_tx.send(BackendEvent::StatusNotice {
                                            title,
                                            message: if ui_lang == "en" {
                                                "Restart TENUKI backend to apply the selected model."
                                                    .to_string()
                                            } else {
                                                "選択したモデルは再起動後に適用されます。".to_string()
                                            },
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.send(BackendEvent::Log(
                                    crate::messages::LogSource::Tenuki,
                                    format!(
                                        "CommitModelSelection: load launcher_config failed: {}",
                                        e
                                    ),
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
                            crate::messages::LogSource::Tenuki,
                            profile_msg,
                            crate::messages::LogLevel::Info,
                            crate::messages::current_timestamp(),
                        ));
                    }
                    FrontendCommand::SetDictSlot(raw) => {
                        let path = std::path::PathBuf::from(raw.trim());
                        let _ = std::fs::create_dir_all(&path);
                        let slot = path.to_string_lossy().to_string();
                        if let Ok(mut config) = crate::config::load(&config_path) {
                            config.dict_slot = Some(slot.clone());
                            let _ = crate::config::save(&config_path, &config);
                        }
                        manager.reload_config(&config_path);
                        let (engine_ok, translator_ok) =
                            manager.apply_restart(RestartScope::TranslatorOnly);
                        let _ = event_tx.send(BackendEvent::BackendReady {
                            engine_success: engine_ok,
                            translator_success: translator_ok,
                        });
                        let _ = event_tx.send(BackendEvent::DictSlotChanged(slot));
                    }
                    FrontendCommand::UpdateSettings {
                        game_text,
                        server_port,
                        server_host,
                    } => {
                        let mut scope = RestartScope::TranslatorOnly;
                        if let Some(game_text_options) = game_text {
                            manager.set_game_text_options(game_text_options);
                            if let Ok(config) = crate::config::load(&config_path) {
                                let _ = crate::config::save_profile_game_text(
                                    &config_path,
                                    &config.profile,
                                    game_text_options,
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

#[cfg(test)]
mod tests {
    use super::{
        resolve_authority_model, resolve_startup_model, ModelCandidate, ModelCandidateKind,
    };
    use crate::launcher::app_config::{ModelConfig, UrlPair};
    use std::path::PathBuf;

    fn known_config(filename: &str, expected_size: u64) -> crate::launcher::app_config::AppConfig {
        let mut cfg = crate::launcher::app_config::AppConfig::default();
        cfg.model = ModelConfig::Known {
            filename: filename.to_string(),
            expected_size,
            urls: UrlPair::single("https://example.com/model.gguf"),
        };
        cfg
    }

    #[test]
    fn startup_model_uses_authority_when_exact_match_exists() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![
            ModelCandidate {
                filename: "authority.gguf".to_string(),
                path: PathBuf::from("models/authority.gguf"),
                size: 100,
                kind: ModelCandidateKind::Local,
            },
            ModelCandidate {
                filename: "other.gguf".to_string(),
                path: PathBuf::from("models/other.gguf"),
                size: 200,
                kind: ModelCandidateKind::Local,
            },
        ];

        assert_eq!(
            resolve_startup_model(&cfg, &candidates),
            Some(PathBuf::from("models/authority.gguf"))
        );
    }

    #[test]
    fn authority_model_uses_exact_filename_and_size_match() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![ModelCandidate {
            filename: "authority.gguf".to_string(),
            path: PathBuf::from("models/authority.gguf"),
            size: 100,
            kind: ModelCandidateKind::Local,
        }];

        assert_eq!(
            resolve_authority_model(&cfg, &candidates),
            Some(PathBuf::from("models/authority.gguf"))
        );
    }

    #[test]
    fn authority_model_does_not_fallback_to_single_usable_model() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![ModelCandidate {
            filename: "local-7b.gguf".to_string(),
            path: PathBuf::from("models/local-7b.gguf"),
            size: 777,
            kind: ModelCandidateKind::Local,
        }];

        assert_eq!(resolve_authority_model(&cfg, &candidates), None);
    }

    #[test]
    fn startup_model_falls_back_to_single_usable_model() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![ModelCandidate {
            filename: "local-7b.gguf".to_string(),
            path: PathBuf::from("models/local-7b.gguf"),
            size: 777,
            kind: ModelCandidateKind::Local,
        }];

        assert_eq!(
            resolve_startup_model(&cfg, &candidates),
            Some(PathBuf::from("models/local-7b.gguf"))
        );
    }

    #[test]
    fn startup_model_stays_unresolved_when_multiple_alternatives_exist() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![
            ModelCandidate {
                filename: "local-a.gguf".to_string(),
                path: PathBuf::from("models/local-a.gguf"),
                size: 777,
                kind: ModelCandidateKind::Local,
            },
            ModelCandidate {
                filename: "local-b.gguf".to_string(),
                path: PathBuf::from("models/local-b.gguf"),
                size: 888,
                kind: ModelCandidateKind::Local,
            },
        ];

        assert_eq!(resolve_startup_model(&cfg, &candidates), None);
    }

    #[test]
    fn startup_model_returns_none_for_empty_candidates() {
        let cfg = known_config("authority.gguf", 100);
        assert_eq!(resolve_startup_model(&cfg, &[]), None);
    }

    // Contract: check_ready_detail の Ok() に対応する入力条件では
    // resolve_startup_model は必ず Some を返す。
    // Case A: authority exact match
    #[test]
    fn contract_ok_authority_exact_match_resolves() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![ModelCandidate {
            filename: "authority.gguf".to_string(),
            path: PathBuf::from("models/authority.gguf"),
            size: 100,
            kind: ModelCandidateKind::Known,
        }];
        assert!(resolve_startup_model(&cfg, &candidates).is_some());
    }

    // Case B: authority absent, exactly 1 usable alternative
    #[test]
    fn contract_ok_single_alternative_resolves() {
        let cfg = known_config("authority.gguf", 100);
        let candidates = vec![ModelCandidate {
            filename: "other.gguf".to_string(),
            path: PathBuf::from("models/other.gguf"),
            size: 500,
            kind: ModelCandidateKind::Local,
        }];
        assert!(resolve_startup_model(&cfg, &candidates).is_some());
    }
}
