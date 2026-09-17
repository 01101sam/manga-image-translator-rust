use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

fn default_workers() -> usize {
    2
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(flatten)]
    pub settings: Settings,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            workers: default_workers(),
            api_key: None,
            settings: Settings::default(),
        }
    }
}

impl DaemonConfig {
    pub fn settings_clone(&self) -> Settings {
        crate::server::jobs::clone_settings(&self.settings)
    }

    pub fn apply_secrets(&self) {
        if let Some(key) = self.api_key.as_deref() {
            if !key.is_empty() {
                std::env::set_var("DEEPSEEK_API_KEY", key);
            }
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Invalid(String),
}

impl ConfigError {
    pub fn message(&self) -> String {
        match self {
            ConfigError::Io(e) => format!("config io error: {e}"),
            ConfigError::Invalid(e) => e.clone(),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

pub struct ConfigStore {
    dir: PathBuf,
    cfg: DaemonConfig,
}

impl ConfigStore {
    pub fn platform() -> Result<Self, ConfigError> {
        let dir = dirs::config_dir()
            .ok_or_else(|| ConfigError::Invalid("no platform config dir".into()))?
            .join("imagetranslator");
        Self::open(dir)
    }

    pub fn open(dir: PathBuf) -> Result<Self, ConfigError> {
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.json");
        let tmp = dir.join("config.json.tmp");
        if tmp.exists() {
            // A leftover temp file is never the source of truth.
            let _ = fs::remove_file(&tmp);
        }
        let cfg = if path.exists() {
            let data = fs::read_to_string(&path)?;
            serde_json::from_str(&data)
                .map_err(|e| ConfigError::Invalid(format!("invalid config.json: {e}")))?
        } else {
            DaemonConfig::default()
        };
        Ok(Self { dir, cfg })
    }

    pub fn get(&self) -> &DaemonConfig {
        &self.cfg
    }

    pub fn replace(&mut self, cfg: DaemonConfig) -> Result<(), ConfigError> {
        self.cfg = cfg;
        self.save()
    }

    fn save(&self) -> Result<(), ConfigError> {
        let path = self.dir.join("config.json");
        let tmp = self.dir.join("config.json.tmp");
        let data = serde_json::to_vec_pretty(&self.cfg)
            .map_err(|e| ConfigError::Invalid(format!("serialize config: {e}")))?;
        fs::write(&tmp, &data)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("it-cfg-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrip_persistence() {
        let dir = tmp_dir();
        let mut store = ConfigStore::open(dir.clone()).unwrap();
        let mut cfg = DaemonConfig::default();
        cfg.workers = 3;
        cfg.api_key = Some("sk-test".into());
        store.replace(cfg).unwrap();
        drop(store);
        let store = ConfigStore::open(dir).unwrap();
        assert_eq!(store.get().workers, 3);
        assert_eq!(store.get().api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn truncated_temp_is_ignored() {
        let dir = tmp_dir();
        let mut store = ConfigStore::open(dir.clone()).unwrap();
        let mut cfg = DaemonConfig::default();
        cfg.workers = 4;
        store.replace(cfg).unwrap();
        drop(store);
        fs::write(dir.join("config.json.tmp"), b"{").unwrap();
        let store = ConfigStore::open(dir.clone()).unwrap();
        assert_eq!(store.get().workers, 4);
        assert!(!dir.join("config.json.tmp").exists());
    }
}
