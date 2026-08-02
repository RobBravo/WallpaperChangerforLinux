use std::path::{Path, PathBuf};
use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntervalUnit {
    Minutes,
    Hours,
    Days,
}

impl IntervalUnit {
    pub fn to_duration(self, value: u64) -> Duration {
        let secs = match self {
            IntervalUnit::Minutes => value * 60,
            IntervalUnit::Hours => value * 60 * 60,
            IntervalUnit::Days => value * 60 * 60 * 24,
        };
        Duration::from_secs(secs)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub folder: PathBuf,
    pub interval_value: u64,
    pub interval_unit: IntervalUnit,
    pub paused: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            folder: dirs::picture_dir().unwrap_or_else(|| PathBuf::from(".")),
            interval_value: 30,
            interval_unit: IntervalUnit::Minutes,
            paused: false,
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wallpaper-changer")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn change_now_request_path() -> PathBuf {
    config_dir().join("change_now_request")
}

pub fn gui_socket_path() -> PathBuf {
    config_dir().join("gui.sock")
}

impl Config {
    pub fn load_from(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fs_util::atomic_write(path, &text)
    }

    pub fn load() -> anyhow::Result<Config> {
        let path = config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.save()?;
            return Ok(cfg);
        }
        Self::load_from(&path)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&config_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_unit_converts_to_duration() {
        assert_eq!(IntervalUnit::Minutes.to_duration(2), Duration::from_secs(120));
        assert_eq!(IntervalUnit::Hours.to_duration(1), Duration::from_secs(3600));
        assert_eq!(IntervalUnit::Days.to_duration(1), Duration::from_secs(86400));
    }

    #[test]
    fn config_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config {
            folder: PathBuf::from("/tmp/wallpapers"),
            interval_value: 45,
            interval_unit: IntervalUnit::Hours,
            paused: true,
        };

        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();

        assert_eq!(loaded, cfg);
    }
}
