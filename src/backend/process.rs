// src/backend/process.rs

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::messages::{BackendEvent, LogLevel, LogSource};

/// llama-server に渡す引数を組み立てた Command を返す。
/// launcher 検証起動と本番起動の両方がこれを使うことで引数を1か所に集約する。
/// - current_dir の設定は呼び出し側の責務（exe の親ディレクトリを推奨）
/// - stdout/stderr のリダイレクトも呼び出し側で行う
/// - CREATE_NO_WINDOW も呼び出し側で付ける
pub fn build_llama_command(
    exe: &Path,
    model: &Path,
    port: u16,
    ngl: u32,
    ctx_size: u32,
    batch_size: u32,
    ubatch_size: u32,
    cont_batching: bool,
    parallel: u32,
    extra_args: &[String],
) -> Command {
    let parallel = parallel.max(1);
    // Vulkan バックエンドで ngl=0 がCPU処理にフォールバックするバグ回避。
    // 0 は「未設定」扱いとして全レイヤーGPUオフロードにする。
    let ngl = if ngl == 0 { 999 } else { ngl };

    let mut cmd = Command::new(exe);
    cmd.args(["-m", model.to_str().unwrap_or_default()])
        .args(["-ngl", &ngl.to_string()])
        .args(["--port", &port.to_string()])
        .args(["--ctx-size", &ctx_size.to_string()])
        .args(["--batch-size", &batch_size.to_string()])
        .args(["-ub", &ubatch_size.to_string()])
        .arg(if cont_batching {
            "--cont-batching"
        } else {
            "--no-cont-batching"
        })
        .args(["--parallel", &parallel.to_string()])
        .args(["--cache-ram", "0"])
        .args(["--metrics"]);
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }
    cmd
}

pub struct LlamaProcess {
    pub child: Child,
}

impl LlamaProcess {
    pub fn start(
        llama_exe: &PathBuf,
        model: &PathBuf,
        ngl: u32,
        ctx_size: u32,
        batch_size: u32,
        ubatch_size: u32,
        cont_batching: bool,
        parallel: u32,
        port: u16,
        event_tx: mpsc::Sender<BackendEvent>,
    ) -> Result<Self, String> {
        let mut cmd = build_llama_command(
            llama_exe,
            model,
            port,
            ngl,
            ctx_size,
            batch_size,
            ubatch_size,
            cont_batching,
            parallel,
            &[],
        );

        // DLL 同梱パスを確実にロードさせるため、exe のあるディレクトリを作業ディレクトリに設定
        if let Some(parent) = llama_exe.parent() {
            cmd.current_dir(parent);
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn llama-server: {}", e))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let tx = event_tx.clone();
        Self::monitor_output(stdout, stderr, tx);
        Ok(Self { child })
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn monitor_output(
        stdout: Option<std::process::ChildStdout>,
        stderr: Option<std::process::ChildStderr>,
        event_tx: mpsc::Sender<BackendEvent>,
    ) {
        if let Some(stdout) = stdout {
            let tx = event_tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        let line = line.trim_end_matches('\r').to_string();
                        if !line.is_empty() {
                            let _ = tx.send(BackendEvent::Log(
                                LogSource::LlamaCpp,
                                line,
                                LogLevel::Info,
                                crate::messages::current_timestamp(),
                            ));
                        }
                    }
                }
            });
        }

        if let Some(stderr) = stderr {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        let line = line.trim_end_matches('\r').to_string();
                        if !line.is_empty() {
                            let _ = event_tx.send(BackendEvent::Log(
                                LogSource::LlamaCpp,
                                line,
                                LogLevel::Error,
                                crate::messages::current_timestamp(),
                            ));
                        }
                    }
                }
            });
        }
    }
}
