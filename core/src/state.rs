use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub current_wallpaper: PathBuf,
    pub next_change_at_unix: i64,
}

pub fn state_path() -> PathBuf {
    crate::config::config_dir().join("state.toml")
}

impl State {
    pub fn load_from(path: &Path) -> anyhow::Result<State> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fs_util::atomic_write(path, &text)
    }

    pub fn load() -> anyhow::Result<State> {
        Self::load_from(&state_path())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&state_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let state = State {
            current_wallpaper: PathBuf::from("/tmp/wallpapers/a.png"),
            next_change_at_unix: 1_800_000_000,
        };

        state.save_to(&path).unwrap();
        let loaded = State::load_from(&path).unwrap();

        assert_eq!(loaded, state);
    }
}
