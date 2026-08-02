use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorState {
    pub uuid: String,
    pub current_wallpaper: PathBuf,
    pub next_change_at_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub monitors: Vec<MonitorState>,
}

impl State {
    pub fn monitor(&self, uuid: &str) -> Option<&MonitorState> {
        self.monitors.iter().find(|m| m.uuid == uuid)
    }

    /// Replaces (or inserts) one monitor's state entry, leaving every other entry
    /// untouched.
    pub fn set_monitor(&mut self, monitor_state: MonitorState) {
        match self.monitors.iter_mut().find(|m| m.uuid == monitor_state.uuid) {
            Some(existing) => *existing = monitor_state,
            None => self.monitors.push(monitor_state),
        }
    }

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

pub fn state_path() -> PathBuf {
    crate::config::config_dir().join("state.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(uuid: &str) -> MonitorState {
        MonitorState {
            uuid: uuid.to_string(),
            current_wallpaper: PathBuf::from("/tmp/wallpapers/a.png"),
            next_change_at_unix: 1_800_000_000,
        }
    }

    #[test]
    fn state_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let state = State { monitors: vec![sample("uuid-a")] };

        state.save_to(&path).unwrap();
        let loaded = State::load_from(&path).unwrap();

        assert_eq!(loaded, state);
    }

    #[test]
    fn monitor_finds_the_matching_entry() {
        let state = State { monitors: vec![sample("uuid-a")] };
        assert!(state.monitor("uuid-a").is_some());
        assert!(state.monitor("uuid-b").is_none());
    }

    #[test]
    fn set_monitor_updates_an_existing_entry_in_place_without_touching_others() {
        let mut state = State { monitors: vec![sample("a"), sample("b")] };

        state.set_monitor(MonitorState {
            uuid: "a".to_string(),
            current_wallpaper: PathBuf::from("/new-a.png"),
            next_change_at_unix: 99,
        });

        assert_eq!(state.monitors.len(), 2);
        assert_eq!(state.monitor("a").unwrap().current_wallpaper, PathBuf::from("/new-a.png"));
        assert_eq!(state.monitor("b").unwrap().current_wallpaper, PathBuf::from("/tmp/wallpapers/a.png"));
    }

    #[test]
    fn set_monitor_inserts_a_new_entry_when_the_uuid_is_unseen() {
        let mut state = State::default();
        state.set_monitor(sample("new"));
        assert_eq!(state.monitors.len(), 1);
    }

    #[test]
    fn loading_an_old_flat_format_file_yields_an_empty_monitor_list_instead_of_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "current_wallpaper = \"/a.png\"\nnext_change_at_unix = 123\n").unwrap();

        let loaded = State::load_from(&path).unwrap();

        assert!(loaded.monitors.is_empty());
    }
}
