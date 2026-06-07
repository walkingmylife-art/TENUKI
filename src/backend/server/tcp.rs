//! TCP translation endpoint (length-prefixed binary protocol)

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::messages::{BackendEvent, LogLevel, LogSource};

use super::{AppState, PipelineBehavior};

pub(crate) const TCP_MAX_PAYLOAD_BYTES: usize = 64 * 1024;

pub(crate) async fn serve_tcp_connection(
    stream: tokio::net::TcpStream,
    state: Arc<AppState>,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = TokioBufReader::new(reader);
    let mut len_buf = [0u8; 4];

    loop {
        if reader.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let payload_len = u32::from_le_bytes(len_buf) as usize;

        if payload_len > TCP_MAX_PAYLOAD_BYTES {
            let _ = event_tx.try_send(BackendEvent::Log(
                LogSource::Tenuki,
                format!("tcp payload too large: {} (max {})", payload_len, TCP_MAX_PAYLOAD_BYTES),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            break;
        }

        let mut payload = vec![0u8; payload_len];
        if reader.read_exact(&mut payload).await.is_err() {
            break;
        }

        let text = String::from_utf8_lossy(&payload).into_owned();

        let result = super::run_pipeline(
            &state,
            "tcp",
            PipelineBehavior::normal_translate(),
            vec![text],
        )
        .await;

        let response = match result {
            Ok(r) => r.translated_text.into_bytes(),
            Err(e) => {
                let _ = event_tx.try_send(BackendEvent::Log(
                    LogSource::Tenuki,
                    format!("tcp pipeline error: {}", e),
                    LogLevel::Error,
                    crate::messages::current_timestamp(),
                ));
                continue;
            }
        };

        let resp_len = response.len() as u32;
        if writer.write_all(&resp_len.to_le_bytes()).await.is_err() {
            break;
        }
        if writer.write_all(&response).await.is_err() {
            break;
        }
    }
}

pub(crate) async fn run_tcp_listener(
    state: Arc<AppState>,
    host: &str,
    port: u16,
    event_tx: tokio::sync::mpsc::Sender<BackendEvent>,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let addr: std::net::SocketAddr = match format!("{}:{}", host, port).parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = event_tx.try_send(BackendEvent::Log(
                LogSource::Tenuki,
                format!("tcp invalid address {}:{} ({})", host, port, e),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            return;
        }
    };

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            let _ = event_tx.try_send(BackendEvent::Log(
                LogSource::Tenuki,
                format!("tcp bind failed {}:{} ({})", host, port, e),
                LogLevel::Error,
                crate::messages::current_timestamp(),
            ));
            return;
        }
    };

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        format!("tcp translation listening on {}:{}", host, port),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));

    let server_handle = {
        let state = state.clone();
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut join_set = JoinSet::new();

            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((stream, _)) => {
                                let state = state.clone();
                                let event_tx = event_tx.clone();
                                join_set.spawn(async move {
                                    serve_tcp_connection(stream, state, event_tx).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }

                    Some(join_result) = join_set.join_next() => {
                        if let Err(e) = join_result {
                            let _ = event_tx.try_send(BackendEvent::Log(
                                LogSource::Tenuki,
                                format!("tcp connection handler panicked: {}", e),
                                LogLevel::Error,
                                crate::messages::current_timestamp(),
                            ));
                        }
                    }
                }
            }

            join_set.abort_all();
        })
    };

    let _ = shutdown_rx.await;
    server_handle.abort();

    let _ = event_tx.try_send(BackendEvent::Log(
        LogSource::Tenuki,
        "tcp server stopped".to_string(),
        LogLevel::Info,
        crate::messages::current_timestamp(),
    ));
}
