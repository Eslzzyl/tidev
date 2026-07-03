use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Standard tidev config and data directory paths.
#[derive(Clone, Debug)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub auth_file: PathBuf,
    pub database_file: PathBuf,
}

impl ConfigPaths {
    /// Discover paths using XDG conventions (~/.config/tidev, ~/.local/share/tidev).
    pub fn discover() -> Result<Self> {
        let home_dir = dirs::home_dir().context("unable to determine the home directory")?;

        let config_dir = home_dir.join(".config").join("tidev");
        let data_dir = home_dir.join(".local/share").join("tidev");

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            auth_file: data_dir.join("auth.json"),
            database_file: data_dir.join("sessions.sqlite3"),
            config_dir,
            data_dir,
        })
    }

    /// Ensure config and data directories exist.
    pub fn ensure_directories(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir).with_context(|| {
            format!(
                "failed to create config directory {}",
                self.config_dir.display()
            )
        })?;
        fs::create_dir_all(&self.data_dir).with_context(|| {
            format!(
                "failed to create data directory {}",
                self.data_dir.display()
            )
        })?;
        Ok(())
    }

    pub fn default_config_path(&self) -> &Path {
        &self.config_file
    }

    pub fn default_auth_path(&self) -> &Path {
        &self.auth_file
    }

    pub fn default_database_path(&self) -> &Path {
        &self.database_file
    }
}
