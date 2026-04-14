use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Utc};

static LOG_FILE: OnceLock<Mutex<(PathBuf, std::fs::File)>> = OnceLock::new();

pub fn init(data_dir: &std::path::Path) {
    let log_path = data_dir.join("tidev.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    if let Some(file) = file {
        let _ = LOG_FILE.set(Mutex::new((log_path, file)));
    }
}

pub fn log(level: &str, target: &str, message: &str) {
    if let Some(mutex) = LOG_FILE.get() {
        let timestamp: DateTime<Utc> = Utc::now();
        let line = format!(
            "[{} {} {}] {}\n",
            timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            level,
            target,
            message
        );
        if let Ok(mut guard) = mutex.lock() {
            let _ = guard.1.write_all(line.as_bytes());
        }
    }
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logging::log("DEBUG", module_path!(), &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logging::log("INFO", module_path!(), &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logging::log("WARN", module_path!(), &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logging::log("ERROR", module_path!(), &format!($($arg)*))
    };
}
