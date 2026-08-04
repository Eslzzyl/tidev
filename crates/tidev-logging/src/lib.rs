//! Custom file + console logging for tidev.
//!
//! Provides a [`log::Log`] implementation that writes to a file (with rotation)
//! and optionally to stderr with coloured output.
//!
//! # Usage
//!
//! Call [`init`] once at startup:
//!
//! ```ignore
//! tidev_logging::init(&paths.data_dir, &config.logging);
//! ```

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Local};
use log::{Level, LevelFilter, Log, Metadata, Record};
use tidev_config::LogConfig;

// ---------------------------------------------------------------------------
// ANSI colour helpers (no crossterm dependency)
// ---------------------------------------------------------------------------

mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const GREY: &str = "\x1b[90m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RED: &str = "\x1b[31m";
    pub const CYAN: &str = "\x1b[36m";
    pub const DARK_GREY: &str = "\x1b[2m";
}

fn colour_for_level(level: Level) -> &'static str {
    match level {
        Level::Debug => ansi::GREY,
        Level::Info => ansi::GREEN,
        Level::Warn => ansi::YELLOW,
        Level::Error => ansi::RED,
        _ => ansi::RESET,
    }
}

// ---------------------------------------------------------------------------
// Global logger instance
// ---------------------------------------------------------------------------

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
        let timestamp: DateTime<Local> = Local::now();
        let formatted_timestamp = timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        // ── Write to file (always uncolored) ──
        if let Some(ref mut file) = guard.file {
            let file_line = format!("[{} {} {}] {}", formatted_timestamp, level, target, message);
            let _ = file.write_all(file_line.as_bytes());
            let _ = file.write_all(b"\n");
            let _ = file.flush();
        }

        // ── Write to stderr (coloured) if enabled ──
        if guard.config.console {
            let c_level = colour_for_level(record.level());
            eprintln!(
                "[{adim}{ts}{reset}] [{c}{level}{reset}] [{cc}{target}{reset}] {msg}",
                adim = ansi::DARK_GREY,
                ts = formatted_timestamp,
                reset = ansi::RESET,
                c = c_level,
                level = level,
                cc = ansi::CYAN,
                target = target,
                msg = message,
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

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialise the tidev logger.
///
/// * `data_dir` – directory where `tidev.log` will be created.
/// * `config`   – logging configuration.
///
/// If the global logger has already been set, this is a no-op (the second
/// `set_logger` call returns an error which is silently ignored).
pub fn init(data_dir: &Path, config: &LogConfig) {
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
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(max_level));
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn level_to_filter_maps_correctly() {
        assert_eq!(level_to_filter("ERROR"), LevelFilter::Error);
        assert_eq!(level_to_filter("WARN"), LevelFilter::Warn);
        assert_eq!(level_to_filter("INFO"), LevelFilter::Info);
        assert_eq!(level_to_filter("DEBUG"), LevelFilter::Debug);
    }

    #[test]
    fn level_to_filter_unknown_defaults_to_info() {
        assert_eq!(level_to_filter("TRACE"), LevelFilter::Info);
        assert_eq!(level_to_filter(""), LevelFilter::Info);
        assert_eq!(level_to_filter("garbage"), LevelFilter::Info);
    }

    #[test]
    fn level_to_filter_case_insensitive() {
        assert_eq!(level_to_filter("error"), LevelFilter::Error);
        assert_eq!(level_to_filter("Info"), LevelFilter::Info);
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

// ---------------------------------------------------------------------------
// Log rotation
// ---------------------------------------------------------------------------

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

    let log_dir = state.log_path.parent().unwrap_or(Path::new("."));
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
