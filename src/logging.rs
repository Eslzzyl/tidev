use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Utc};
use crossterm::style::{Color, Stylize};

use crate::config::LogConfig;

static LOG_STATE: OnceLock<Mutex<LogState>> = OnceLock::new();

struct LogState {
    config: LogConfig,
    log_path: PathBuf,
    file: Option<std::fs::File>,
}

impl LogState {
    fn new(config: LogConfig, log_path: PathBuf, file: std::fs::File) -> Self {
        Self {
            config,
            log_path,
            file: Some(file),
        }
    }
}

fn level_to_int(level: &str) -> u8 {
    match level.to_uppercase().as_str() {
        "DEBUG" => 0,
        "INFO" => 1,
        "WARN" => 2,
        "ERROR" => 3,
        _ => 1,
    }
}

pub fn init(data_dir: &Path, config: LogConfig) {
    if !config.enabled {
        return;
    }

    let log_path = data_dir.join("tidev.log");
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    if let Some(file) = file {
        let _ = LOG_STATE.set(Mutex::new(LogState::new(config, log_path, file)));
    }
}

fn rotate_if_needed(state: &mut LogState) {
    let max_bytes = (state.config.max_size_mb as u64) * 1024 * 1024;

    let needs_rotation = state
        .file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .map(|m| m.len() >= max_bytes)
        .unwrap_or(false);

    if !needs_rotation {
        return;
    }

    state.file = None;

    let log_dir = state.log_path.parent().unwrap_or(std::path::Path::new("."));
    let stem = state
        .log_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let ext = state
        .log_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy();

    for i in (1..state.config.max_files).rev() {
        let old_path = log_dir.join(format!("{}.{}.{}", stem, i, ext));
        let new_path = log_dir.join(format!("{}.{}.{}", stem, i + 1, ext));
        let _ = fs::rename(&old_path, &new_path);
    }

    let rotated_path = log_dir.join(format!("{}.1.{}", stem, ext));
    let _ = fs::rename(&state.log_path, &rotated_path);

    state.file = Some(
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&state.log_path)
            .expect("failed to create log file"),
    );
}

pub fn log(level: &str, target: &str, message: &str) {
    if let Some(mutex) = LOG_STATE.get()
        && let Ok(mut guard) = mutex.lock()
    {
        let min_level = level_to_int(&guard.config.level);
        if level_to_int(level) < min_level {
            return;
        }

        rotate_if_needed(&mut guard);

        let timestamp: DateTime<Utc> = Utc::now();
        let formatted_timestamp = timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        // Write to file (always uncolored)
        if let Some(ref mut file) = guard.file {
            let file_line = format!("[{} {} {}] {}", formatted_timestamp, level, target, message);
            let _ = file.write_all(file_line.as_bytes());
            let _ = file.write_all(b"\n");
            let _ = file.flush();
        }

        // Write to console (stderr) if enabled
        if guard.config.console {
            let colored_level = match level.to_uppercase().as_str() {
                "DEBUG" => level.with(Color::Grey),
                "INFO" => level.with(Color::Green),
                "WARN" => level.with(Color::Yellow),
                "ERROR" => level.with(Color::Red),
                _ => level.stylize(),
            };

            let colored_target = target.with(Color::Cyan);
            let colored_timestamp = formatted_timestamp.with(Color::DarkGrey);

            eprintln!(
                "[{}] [{}] [{}] {}",
                colored_timestamp, colored_level, colored_target, message
            );
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
