use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Utc};
use crossterm::style::{Color, Stylize};
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

use crate::config::LogConfig;

static LOGGER: TidevLogger = TidevLogger;

struct LogState {
    config: LogConfig,
    log_path: PathBuf,
    file: Option<std::fs::File>,
}

static LOG_STATE: OnceLock<Mutex<LogState>> = OnceLock::new();

struct TidevLogger;

impl Log for TidevLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true // top-level filtering handled by set_max_level
    }

    fn log(&self, record: &Record) {
        let Some(mutex) = LOG_STATE.get() else {
            return;
        };
        let Ok(mut guard) = mutex.lock() else {
            return;
        };

        rotate_if_needed(&mut guard);

        let level = record.level().as_str();
        let target = record.target();
        let message = record.args().to_string();
        let timestamp: DateTime<Utc> = Utc::now();
        let formatted_timestamp = timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        // Write to file (always uncolored)
        if let Some(ref mut file) = guard.file {
            let file_line =
                format!("[{} {} {}] {}", formatted_timestamp, level, target, message);
            let _ = file.write_all(file_line.as_bytes());
            let _ = file.write_all(b"\n");
            let _ = file.flush();
        }

        // Write to console (stderr) if enabled
        if guard.config.console {
            let colored_level = match record.level() {
                Level::Debug => level.with(Color::Grey),
                Level::Info => level.with(Color::Green),
                Level::Warn => level.with(Color::Yellow),
                Level::Error => level.with(Color::Red),
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

    fn flush(&self) {
        let Some(mutex) = LOG_STATE.get() else {
            return;
        };
        let Ok(mut guard) = mutex.lock() else {
            return;
        };
        if let Some(ref mut file) = guard.file {
            let _ = file.flush();
        }
    }
}

fn level_to_filter(level: &str) -> LevelFilter {
    match level.to_uppercase().as_str() {
        "ERROR" => LevelFilter::Error,
        "WARN" => LevelFilter::Warn,
        "INFO" => LevelFilter::Info,
        "DEBUG" => LevelFilter::Debug,
        _ => LevelFilter::Info,
    }
}

pub fn init(data_dir: &Path, config: LogConfig) -> Result<(), SetLoggerError> {
    if config.enabled {
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
            let _ = LOG_STATE.set(Mutex::new(LogState {
                config: config.clone(),
                log_path,
                file: Some(file),
            }));
        }
    }

    let max_level = level_to_filter(&config.level);
    log::set_logger(&LOGGER).map(|()| log::set_max_level(max_level))
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
