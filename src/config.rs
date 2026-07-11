use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub output_dir: PathBuf,
    pub output_template: String,
    pub default_mode: String,
    pub custom_format: String,
    pub impersonation: String,
    pub cookies_browser: String,
    pub concurrent_fragments: u8,
    pub use_aria2: bool,
    pub rate_limit: String,
    pub socket_timeout: String,
    pub retries: String,
    pub fragment_retries: String,
    pub extractor_args: String,
    pub playlist_subfolders: bool,
    pub embed_metadata: bool,
    pub write_info_json: bool,
    pub max_active_downloads: u8,
    pub allow_playlists: bool,
    pub search_platform: String,
    pub gui_theme: String,
    pub gui_opacity: f32,
    pub show_sensitive_urls: bool,
}

impl Default for Config {
    fn default() -> Self {
        let output_dir = BaseDirs::new()
            .map(|dirs| dirs.home_dir().join("Downloads"))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            output_dir,
            output_template: "%(title)s [%(id)s].%(ext)s".into(),
            default_mode: "video".into(),
            custom_format: "bestvideo+bestaudio/best".into(),
            impersonation: "none".into(),
            cookies_browser: "none".into(),
            concurrent_fragments: 4,
            use_aria2: false,
            rate_limit: String::new(),
            socket_timeout: String::new(),
            retries: String::new(),
            fragment_retries: String::new(),
            extractor_args: String::new(),
            playlist_subfolders: true,
            embed_metadata: false,
            write_info_json: false,
            max_active_downloads: 1,
            allow_playlists: true,
            search_platform: "youtube".into(),
            gui_theme: "graphite".into(),
            gui_opacity: 0.96,
            show_sensitive_urls: false,
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
        let mut config: Self = toml::from_str(&text)
            .with_context(|| format!("invalid config file: {}", path.display()))?;
        config.output_dir = normalize_output_dir(&config.output_dir)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("invalid output_dir in {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        let temporary = path.with_extension("toml.tmp");
        let mut file = fs::File::create(&temporary)
            .with_context(|| format!("could not create {}", temporary.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("could not write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("could not sync {}", temporary.display()))?;
        drop(file);
        replace_file(&temporary, path)
            .with_context(|| format!("could not replace {}", path.display()))
    }
}

pub fn validate_output_dir(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Output folder cannot be empty".into());
    }
    normalize_output_dir(Path::new(trimmed))
}

pub fn normalize_output_dir(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("Output folder cannot be empty".into());
    }
    if !path.is_absolute() {
        return Err("Output folder must be an absolute path".into());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err("Output folder cannot contain '.' or '..' segments".into());
    }

    let normalized = if path.exists() {
        path.canonicalize()
            .map_err(|error| format!("could not access output folder: {error}"))?
    } else {
        path.to_path_buf()
    };

    if normalized.exists() && !normalized.is_dir() {
        return Err("Output folder must point to a directory".into());
    }

    if !normalized.exists() {
        let Some(parent) = normalized.parent() else {
            return Err("Output folder must have an existing parent directory".into());
        };
        if !parent.is_dir() {
            return Err("Output folder parent directory must exist".into());
        }
    }

    Ok(normalized)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    // Windows rename does not replace an existing destination. Preserve the
    // complete temporary file until it is ready, then use the platform fallback.
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
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
        let output_dir = dir.path().join("media");
        fs::write(
            &path,
            format!("output_dir = {:?}\n", output_dir.display().to_string()),
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.output_dir, output_dir);
        assert_eq!(config.output_template, "%(title)s [%(id)s].%(ext)s");
        assert_eq!(config.default_mode, "video");
        assert_eq!(config.rate_limit, "");
        assert_eq!(config.socket_timeout, "");
        assert_eq!(config.retries, "");
        assert_eq!(config.fragment_retries, "");
        assert!(config.playlist_subfolders);
        assert!(!config.embed_metadata);
        assert!(!config.write_info_json);
        assert_eq!(config.max_active_downloads, 1);
        assert!(config.allow_playlists);
        assert_eq!(config.search_platform, "youtube");
        assert_eq!(config.gui_theme, "graphite");
        assert_eq!(config.gui_opacity, 0.96);
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let config = Config::default();
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn rejects_relative_output_dir() {
        let error = validate_output_dir("downloads").unwrap_err();
        assert!(error.contains("absolute path"));
    }

    #[test]
    fn rejects_parent_segments_in_output_dir() {
        let error = validate_output_dir("/tmp/../downloads").unwrap_err();
        assert!(error.contains("'.' or '..'"));
    }

    #[test]
    fn accepts_missing_absolute_output_dir_with_existing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let output_dir = dir.path().join("downloads");
        assert_eq!(normalize_output_dir(&output_dir).unwrap(), output_dir);
    }
}
