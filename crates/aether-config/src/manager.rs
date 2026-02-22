use crate::{AppConfig, ConfigError};
use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

const ORG: &str = "io";
const AUTHOR: &str = "AegisInbox";
const APP: &str = "AegisInbox";
const LEGACY_AUTHOR: &str = "Aether";
const LEGACY_APP: &str = "Aether";

#[derive(Debug, Clone)]
pub struct ConfigManager {
    config_path: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Result<Self, ConfigError> {
        let dirs = ProjectDirs::from(ORG, AUTHOR, APP).ok_or(ConfigError::MissingDirectories)?;
        let config_dir = dirs.config_dir().to_path_buf();
        let data_dir = dirs.data_dir().to_path_buf();
        let cache_dir = dirs.cache_dir().to_path_buf();

        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&cache_dir)?;

        let config_path = config_dir.join("config.toml");
        migrate_legacy_layout(&config_path, &data_dir, &cache_dir)?;
        if !config_path.exists() {
            let initial = AppConfig::default();
            let content = toml::to_string_pretty(&initial)?;
            fs::write(&config_path, content)?;
        }

        Ok(Self {
            config_path,
            data_dir,
            cache_dir,
        })
    }

    pub fn load(&self) -> Result<AppConfig, ConfigError> {
        let content = fs::read_to_string(&self.config_path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        let content = toml::to_string_pretty(config)?;
        fs::write(&self.config_path, content)?;
        Ok(())
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

fn migrate_legacy_layout(
    config_path: &Path,
    data_dir: &Path,
    cache_dir: &Path,
) -> Result<(), ConfigError> {
    let Some(legacy_dirs) = ProjectDirs::from(ORG, LEGACY_AUTHOR, LEGACY_APP) else {
        return Ok(());
    };

    let legacy_config = legacy_dirs.config_dir().join("config.toml");
    if !config_path.exists() && legacy_config.exists() {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&legacy_config, config_path)?;
    }

    copy_dir_contents_if_target_empty(legacy_dirs.data_dir(), data_dir)?;
    copy_dir_contents_if_target_empty(legacy_dirs.cache_dir(), cache_dir)?;

    Ok(())
}

fn copy_dir_contents_if_target_empty(source: &Path, target: &Path) -> Result<(), ConfigError> {
    if !source.exists() || !source.is_dir() {
        return Ok(());
    }

    if !is_dir_empty(target)? {
        return Ok(());
    }

    copy_dir_contents(source, target)
}

fn is_dir_empty(path: &Path) -> Result<bool, ConfigError> {
    let mut entries = fs::read_dir(path)?;
    Ok(entries.next().is_none())
}

fn copy_dir_contents(source: &Path, target: &Path) -> Result<(), ConfigError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        let destination = target.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_dir_contents(&entry_path, &destination)?;
            continue;
        }

        if file_type.is_file() && !destination.exists() {
            fs::copy(&entry_path, &destination)?;
        }
    }

    Ok(())
}
