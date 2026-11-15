use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

fn default_workers() -> usize {
    2
}

/// 旧 config.json 没有 pairing_key 字段时用它，保证升级后设备仍能配对。
pub const DEFAULT_PAIRING_KEY: &str = "sam01101";

fn default_pairing_key() -> String {
    DEFAULT_PAIRING_KEY.to_owned()
}

/// 解码后长边超过它就在 pipeline 入口缩小。8000x12000 的图跑完一轮要 14GB，缩到 4096 后像素少 8 倍。
pub const DEFAULT_DOWNSCALE_LONG_EDGE: u32 = 4096;

fn default_downscale_long_edge() -> Option<u32> {
    Some(DEFAULT_DOWNSCALE_LONG_EDGE)
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default)]
    pub api_key: Option<String>,
    /// 配对密钥。修改它只影响之后的 Pairing，已签发的 Token 继续有效。
    #[serde(default = "default_pairing_key")]
    pub pairing_key: String,
    /// `None` 关闭缩放，序列化成 `null`，这样重启后「关闭」不会退回默认值。
    #[serde(default = "default_downscale_long_edge")]
    pub downscale_long_edge: Option<u32>,
    #[serde(flatten)]
    pub settings: Settings,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            workers: default_workers(),
            api_key: None,
            pairing_key: default_pairing_key(),
            downscale_long_edge: default_downscale_long_edge(),
            settings: Settings::default(),
        }
    }
}

impl DaemonConfig {
    pub fn settings_clone(&self) -> Settings {
        crate::server::jobs::clone_settings(&self.settings)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.pairing_key.trim().is_empty() {
            return Err(ConfigError::Invalid("Pairing Key 不能为空".into()));
        }
        match self.downscale_long_edge {
            Some(0) => Err(ConfigError::Invalid(
                "downscale_long_edge 必须大于 0，关闭缩放请设为 null".into(),
            )),
            Some(edge) if edge > crate::server::source::MAX_EDGE => {
                Err(ConfigError::Invalid(format!(
                    "downscale_long_edge 不能超过 {}",
                    crate::server::source::MAX_EDGE
                )))
            }
            _ => Ok(()),
        }
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
        cfg.validate()?;
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
    fn legacy_config_without_pairing_key_gets_the_default() {
        let dir = tmp_dir();
        fs::write(
            dir.join("config.json"),
            br#"{"workers": 3, "api_key": "sk-old"}"#,
        )
        .unwrap();
        let store = ConfigStore::open(dir).unwrap();
        assert_eq!(store.get().pairing_key, DEFAULT_PAIRING_KEY);
        assert_eq!(store.get().workers, 3);
    }

    #[test]
    fn pairing_key_roundtrips_through_disk() {
        let dir = tmp_dir();
        let mut store = ConfigStore::open(dir.clone()).unwrap();
        let mut cfg = DaemonConfig::default();
        cfg.pairing_key = "another-key".into();
        store.replace(cfg).unwrap();
        drop(store);
        assert_eq!(
            ConfigStore::open(dir).unwrap().get().pairing_key,
            "another-key"
        );
    }

    #[test]
    fn blank_pairing_key_is_rejected_and_old_value_kept() {
        let dir = tmp_dir();
        let mut store = ConfigStore::open(dir).unwrap();
        let mut cfg = DaemonConfig::default();
        cfg.pairing_key = "   ".into();
        assert!(matches!(store.replace(cfg), Err(ConfigError::Invalid(_))));
        assert_eq!(store.get().pairing_key, DEFAULT_PAIRING_KEY);
    }

    #[test]
    fn downscale_defaults_to_4096_when_the_field_is_missing() {
        let dir = tmp_dir();
        fs::write(dir.join("config.json"), br#"{"workers": 2}"#).unwrap();
        assert_eq!(
            ConfigStore::open(dir).unwrap().get().downscale_long_edge,
            Some(4096)
        );
    }

    #[test]
    fn disabled_downscale_stays_disabled_after_a_restart() {
        let dir = tmp_dir();
        let mut store = ConfigStore::open(dir.clone()).unwrap();
        let mut cfg = DaemonConfig::default();
        cfg.downscale_long_edge = None;
        store.replace(cfg).unwrap();
        drop(store);
        assert_eq!(
            ConfigStore::open(dir).unwrap().get().downscale_long_edge,
            None
        );
    }

    #[test]
    fn nonsensical_downscale_values_are_rejected() {
        let dir = tmp_dir();
        let mut store = ConfigStore::open(dir).unwrap();
        for bad in [Some(0), Some(65_536)] {
            let mut cfg = DaemonConfig::default();
            cfg.downscale_long_edge = bad;
            assert!(
                matches!(store.replace(cfg), Err(ConfigError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert_eq!(store.get().downscale_long_edge, Some(4096));
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
