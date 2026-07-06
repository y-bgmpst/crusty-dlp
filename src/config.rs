use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub output_dir: PathBuf,
    pub default_mode: String,
    pub custom_format: String,
    pub impersonation: String,
    pub cookies_browser: String,
}

impl Default for Config {
    fn default() -> Self {
        let output_dir = BaseDirs::new()
            .map(|dirs| dirs.home_dir().join("Downloads"))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            output_dir,
            default_mode: "video".into(),
            custom_format: "bestvideo+bestaudio/best".into(),
            impersonation: "none".into(),
            cookies_browser: "none".into(),
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        ProjectDirs::from("org", "crusty-dlp", "crusty-dlp")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .ok_or_else(|| AppError::ConfigDirectory.into())
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid config file: {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text).with_context(|| format!("could not write {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(&dir.path().join("missing.toml")).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn loads_partial_config_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "output_dir = '/tmp/media'\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.output_dir, PathBuf::from("/tmp/media"));
        assert_eq!(config.default_mode, "video");
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let config = Config::default();
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
    }
}
