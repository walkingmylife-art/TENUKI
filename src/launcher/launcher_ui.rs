// src/launcher/launcher_ui.rs

use eframe::egui::{self, CentralPanel, Context, ProgressBar};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::progress::{LaunchProgress, LauncherStage};

#[derive(Clone, PartialEq)]
pub enum LauncherStep {
    Launching,
    Error(String),
}

pub struct LauncherUiState {
    pub step: LauncherStep,
    pub stage: LauncherStage,
    pub status: String,
    pub sub_status: String,
    pub progress: f32,
}

impl Default for LauncherUiState {
    fn default() -> Self {
        Self {
            step: LauncherStep::Launching,
            stage: LauncherStage::Directories,
            status: String::new(),  // 固定日本語をやめ、空文字に
            sub_status: String::new(),
            progress: 0.0,
        }
    }
}

/// ランチャー画面の文言を ui_lang に応じて返す
pub struct LauncherText {
    pub preparing: String,
    pub cancel: String,
    pub cancelled: String,
    pub error: String,
    pub retry: String,
    pub open_folder: String,
    pub unexpected_exit: String,
    pub retrying: String,
    pub step_label: String,
}

pub fn launcher_text(ui_lang: &str) -> LauncherText {
    match ui_lang {
        "en" => LauncherText {
            preparing: "Preparing...".to_string(),
            cancel: "Cancel".to_string(),
            cancelled: "Cancelled".to_string(),
            error: "Error".to_string(),
            retry: "Retry".to_string(),
            open_folder: "Open Folder".to_string(),
            unexpected_exit: "Launcher exited unexpectedly".to_string(),
            retrying: "Retrying...".to_string(),
            step_label: "Step".to_string(),
        },
        _ => LauncherText {
            preparing: "準備中...".to_string(),
            cancel: "キャンセル".to_string(),
            cancelled: "キャンセルされました".to_string(),
            error: "エラー".to_string(),
            retry: "再試行".to_string(),
            open_folder: "フォルダを開く".to_string(),
            unexpected_exit: "起動処理が予期せず終了しました".to_string(),
            retrying: "再試行中...".to_string(),
            step_label: "ステップ".to_string(),
        },
    }
}

/// ステージから (現在ステップ, 総ステップ数) を返す
fn stage_progress(stage: LauncherStage) -> (usize, usize) {
    match stage {
        LauncherStage::Directories => (1, 6),
        LauncherStage::Gpu => (2, 6),
        LauncherStage::Runtime => (3, 6),
        LauncherStage::Model => (4, 6),
        LauncherStage::Verify => (5, 6),
        LauncherStage::Save => (6, 6),
        LauncherStage::Complete => (6, 6),
        LauncherStage::Error => (0, 6),
    }
}

pub fn show_launcher_screen(
    ctx: &Context,
    state: &mut LauncherUiState,
    rx: &Receiver<LaunchProgress>,
    tx: &Sender<LaunchProgress>,
    launcher_thread: &mut Option<std::thread::JoinHandle<()>>,
    cancel_flag: &Arc<AtomicBool>,
    base_dir: &std::path::Path,
    ui_lang: &str,
) -> (bool, bool) {
    let mut needs_repaint = false;
    let mut switch_to_normal = false;
    let txt = launcher_text(ui_lang);

    while let Ok(progress) = rx.try_recv() {
        needs_repaint = true;
        match progress {
            LaunchProgress::Stage(stage) => {
                state.stage = stage;
            }
            LaunchProgress::Status(s) => state.status = s,
            LaunchProgress::SubStatus(s) => state.sub_status = s,
            LaunchProgress::Progress(p) => state.progress = p.clamp(0.0, 1.0),
            LaunchProgress::Complete => {
                state.stage = LauncherStage::Complete;
                switch_to_normal = true;
                return (needs_repaint, switch_to_normal);
            }
            LaunchProgress::Error(e) => {
                state.step = LauncherStep::Error(e);
                state.stage = LauncherStage::Error;
            }
            LaunchProgress::Cancelled => {
                state.step = LauncherStep::Error(txt.cancelled.clone());
                state.stage = LauncherStage::Error;
            }
        }
    }

    if let Some(handle) = launcher_thread.as_ref() {
        if handle.is_finished() && matches!(state.step, LauncherStep::Launching) {
            state.step = LauncherStep::Error(txt.unexpected_exit.clone());
            state.stage = LauncherStage::Error;
            needs_repaint = true;
        }
    }

    CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.heading("TENUKI");
            ui.add_space(20.0);

            // ステップ表示 (Step 3 / 6)
            let (current_step, total_steps) = stage_progress(state.stage);
            ui.label(format!("{} {} / {}", txt.step_label, current_step, total_steps));
            ui.add_space(10.0);

            // ステージインジケータ（完了／実行中／未着手）- ASCII文字で表記
            let stage_order = [
                LauncherStage::Directories,
                LauncherStage::Gpu,
                LauncherStage::Runtime,
                LauncherStage::Model,
                LauncherStage::Verify,
                LauncherStage::Save,
            ];
            let current_idx = match state.stage {
                LauncherStage::Complete => Some(stage_order.len()),
                LauncherStage::Error => None,
                s => stage_order.iter().position(|&x| x == s),
            };

            ui.horizontal(|ui| {
                for (idx, &s) in stage_order.iter().enumerate() {
                    let label = match s {
                        LauncherStage::Directories => "DIR",
                        LauncherStage::Gpu => "GPU",
                        LauncherStage::Runtime => "RUN",
                        LauncherStage::Model => "MOD",
                        LauncherStage::Verify => "CHK",
                        LauncherStage::Save => "CFG",
                        _ => "",
                    };

                    let color = if state.stage == LauncherStage::Error {
                        // エラー時は警告色（赤ではなく黄土色）
                        egui::Color32::from_rgb(180, 140, 60)
                    } else if let Some(cur) = current_idx {
                        if idx < cur {
                            egui::Color32::GREEN // 完了
                        } else if idx == cur {
                            egui::Color32::WHITE // 実行中
                        } else {
                            egui::Color32::GRAY // 未実行
                        }
                    } else {
                        egui::Color32::GRAY
                    };

                    ui.colored_label(color, label);
                }
            });
            ui.add_space(10.0);

            match &state.step {
                LauncherStep::Launching => {
                    ui.label(&state.status);
                    if !state.sub_status.is_empty() {
                        ui.label(&state.sub_status);
                    }
                    ui.add_space(10.0);
                    ui.add(ProgressBar::new(state.progress).animate(true));
                    ui.add_space(20.0);

                    if ui.button(&txt.cancel).clicked() {
                        cancel_flag.store(true, Ordering::Relaxed);
                    }
                }
                LauncherStep::Error(msg) => {
                    ui.colored_label(egui::Color32::ORANGE, &txt.error);
                    ui.label(msg);
                    ui.add_space(10.0);

                    if ui.button(&txt.retry).clicked() {
                        if let Some(handle) = launcher_thread.take() {
                            let _ = handle.join();
                        }
                        cancel_flag.store(false, Ordering::Relaxed);
                        state.step = LauncherStep::Launching;
                        state.stage = LauncherStage::Directories;
                        state.status = txt.retrying.clone();
                        state.sub_status.clear();
                        state.progress = 0.0;
                        *launcher_thread = Some(start_launcher_thread(
                            base_dir.to_path_buf(),
                            tx.clone(),
                            cancel_flag.clone(),
                            ui_lang.to_string(),
                        ));
                    }

                    ui.add_space(10.0);
                    if ui.button(&txt.open_folder).clicked() {
                        let _ = open::that(base_dir);
                    }
                }
            }
        });
    });

    if launcher_thread.is_none() {
        *launcher_thread = Some(start_launcher_thread(
            base_dir.to_path_buf(),
            tx.clone(),
            cancel_flag.clone(),
            ui_lang.to_string(),
        ));
        needs_repaint = true;
    }

    (needs_repaint, switch_to_normal)
}

fn start_launcher_thread(
    base_dir: std::path::PathBuf,
    tx: Sender<LaunchProgress>,
    cancel_flag: Arc<AtomicBool>,
    ui_lang: String,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let launcher = crate::launcher::AppLauncher::new(base_dir, ui_lang);
        match launcher {
            Ok(mut l) => {
                if let Err(e) = l.run(crate::launcher::SetupMode::Full, tx.clone(), cancel_flag) {
                    let _ = tx.send(LaunchProgress::Error(format!("{:#}", e)));
                }
            }
            Err(e) => {
                let _ = tx.send(LaunchProgress::Error(format!("{:#}", e)));
            }
        }
    })
}