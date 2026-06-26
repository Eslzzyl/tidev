use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u32,
    #[serde(default = "default_max_files")]
    pub max_files: u32,
    #[serde(default = "default_console")]
    pub console: bool,
    #[serde(default)]
    pub save_request_body: bool,
    #[serde(default = "default_max_request_files")]
    pub max_request_files: usize,
    #[serde(default)]
    pub save_response_body: bool,
    #[serde(default = "default_max_response_files")]
    pub max_response_files: usize,
}

/// Initialize the logging subsystem with optional file output and rotation.
///
/// When `enabled` is true a log file is created at `{data_dir}/tidev.log`.
/// File rotation is triggered when the log file exceeds `max_size_mb`.
/// The `console` flag controls whether log output also goes to stderr.
pub fn init(data_dir: &Path, config: LogConfig) -> Result<(), log::SetLoggerError> {
    let level = match config.level.to_uppercase().as_str() {
        "TRACE" => log::LevelFilter::Trace,
        "DEBUG" => log::LevelFilter::Debug,
        "WARN" => log::LevelFilter::Warn,
        "ERROR" => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };

    let file_logger = if config.enabled {
        let log_path = data_dir.join("tidev.log");
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();
        Some(FileLogger {
            log_path,
            file,
            max_bytes: (config.max_size_mb as u64) * 1024 * 1024,
            max_files: config.max_files,
        })
    } else {
        None
    };

    let logger = TidevLogger {
        file_logger: Mutex::new(file_logger),
        console: config.console,
    };

    log::set_logger(Box::leak(Box::new(logger))).map(|()| log::set_max_level(level))
}

struct FileLogger {
    log_path: PathBuf,
    file: Option<std::fs::File>,
    max_bytes: u64,
    max_files: u32,
}

impl FileLogger {
    fn rotate_if_needed(&mut self) {
        let needs_rotation = self
            .file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.len() >= self.max_bytes)
            .unwrap_or(false);

        if !needs_rotation {
            return;
        }

        self.file = None;

        let log_dir = self.log_path.parent().unwrap_or(Path::new("."));
        let stem = self
            .log_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let ext = self
            .log_path
            .extension()
            .unwrap_or_default()
            .to_string_lossy();

        for i in (1..self.max_files).rev() {
            let old_path = log_dir.join(format!("{}.{}.{}", stem, i, ext));
            let new_path = log_dir.join(format!("{}.{}.{}", stem, i + 1, ext));
            let _ = fs::rename(&old_path, &new_path);
        }

        let rotated_path = log_dir.join(format!("{}.1.{}", stem, ext));
        let _ = fs::rename(&self.log_path, &rotated_path);

        self.file = Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.log_path)
                .expect("failed to create log file"),
        );
    }
}

struct TidevLogger {
    file_logger: Mutex<Option<FileLogger>>,
    console: bool,
}

impl log::Log for TidevLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let Ok(mut guard) = self.file_logger.lock() else {
            return;
        };

        let level = record.level().as_str();
        let target = record.target();
        let message = record.args().to_string();

        // Write to file with rotation
        if let Some(logger) = &mut *guard {
            logger.rotate_if_needed();
            if let Some(file) = &mut logger.file {
                let line =
                    format!("[{} {} {}] {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), level, target, message);
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }

        // Write to console (stderr) if enabled
        if self.console {
            eprintln!("[{}] [{}] {}", level, target, message);
        }
    }

    fn flush(&self) {
        let Ok(mut guard) = self.file_logger.lock() else {
            return;
        };
        if let Some(logger) = &mut *guard
            && let Some(file) = &mut logger.file
        {
            let _ = file.flush();
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_level() -> String {
    "INFO".to_string()
}

fn default_max_size_mb() -> u32 {
    10
}

fn default_max_files() -> u32 {
    5
}

fn default_console() -> bool {
    false
}

fn default_max_request_files() -> usize {
    100
}

fn default_max_response_files() -> usize {
    100
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "INFO".to_string(),
            max_size_mb: 10,
            max_files: 5,
            console: false,
            save_request_body: false,
            max_request_files: 100,
            save_response_body: false,
            max_response_files: 100,
        }
    }
}
