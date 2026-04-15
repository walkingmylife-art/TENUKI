//! ログ出力モジュール

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use once_cell::sync::Lazy;

#[derive(Debug, Clone)]
pub struct LogEvent {
    pub timestamp: String,
    pub level: String,
    pub msg: String,
}

fn runtime_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn ensure_utf8_bom(path: &Path) {
    const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

    match std::fs::read(path) {
        Ok(data) => {
            if data.starts_with(&UTF8_BOM) {
                return;
            }

            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
            {
                let _ = file.write_all(&UTF8_BOM);
                let _ = file.write_all(&data);
            }
        }
        Err(_) => {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
            {
                let _ = file.write_all(&UTF8_BOM);
            }
        }
    }
}

pub static LOG_TX: Lazy<mpsc::SyncSender<LogEvent>> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel::<LogEvent>(1000);
    let log_path = runtime_dir().join("tenuki.log");
    
    thread::spawn(move || {
        ensure_utf8_bom(&log_path);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            for event in rx {
                let _ = writeln!(file, "[{}] {} {}", event.timestamp, event.level, event.msg);
            }
        }
    });
    
    tx
});

pub static OBSERVE_TX: Lazy<mpsc::SyncSender<LogEvent>> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel::<LogEvent>(1000);
    let log_dir = runtime_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("observations.log");

    thread::spawn(move || {
        ensure_utf8_bom(&log_path);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            for event in rx {
                let _ = writeln!(file, "[{}] {} {}", event.timestamp, event.level, event.msg);
            }
        }
    });

    tx
});

pub static REQUEST_TX: Lazy<mpsc::SyncSender<LogEvent>> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel::<LogEvent>(1000);
    let log_dir = runtime_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("requests.log");

    thread::spawn(move || {
        ensure_utf8_bom(&log_path);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            for event in rx {
                let _ = writeln!(file, "[{}] {} {}", event.timestamp, event.level, event.msg);
            }
        }
    });

    tx
});

pub(crate) fn debug_logs_enabled() -> bool {
    cfg!(debug_assertions)
}

pub(crate) fn write_observation(message: String) {
    let _ = OBSERVE_TX.try_send(LogEvent {
        timestamp: crate::messages::current_timestamp(),
        level: "Info".to_string(),
        msg: message,
    });
}

pub(crate) fn write_request(message: String) {
    let _ = REQUEST_TX.try_send(LogEvent {
        timestamp: crate::messages::current_timestamp(),
        level: "Info".to_string(),
        msg: message,
    });
}

#[macro_export]
macro_rules! backend_log {
    ($event_tx:expr, $level:expr, $($arg:tt)*) => {{
        let msg = format!($($arg)*);
        let timestamp = $crate::messages::current_timestamp();

        let _ = $event_tx.send($crate::messages::BackendEvent::Log(
            $crate::messages::LogSource::Tenuki,
            msg.clone(),
            $level,
            timestamp.clone(),
        ));

        let _ = $crate::backend::logger::LOG_TX.try_send(
            $crate::backend::logger::LogEvent {
                timestamp,
                level: format!("{:?}", $level),
                msg,
            }
        );
    }};
}

#[macro_export]
macro_rules! backend_info {
    ($event_tx:expr, $($arg:tt)*) => {{
        if $crate::backend::logger::debug_logs_enabled() {
            $crate::backend_log!($event_tx, $crate::messages::LogLevel::Info, $($arg)*)
        }
    }};
}

#[macro_export]
macro_rules! backend_error {
    ($event_tx:expr, $($arg:tt)*) => {{
        $crate::backend_log!($event_tx, $crate::messages::LogLevel::Error, $($arg)*)
    }};
}
