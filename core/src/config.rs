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

/// One monitor's own folder, rotation interval, and pause state, keyed by its stable
/// KDE-assigned UUID (see `wallpaper_core::monitors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub uuid: String,
    pub folder: PathBuf,
    pub interval_value: u64,
    pub interval_unit: IntervalUnit,
    pub paused: bool,
}

impl MonitorConfig {
    /// This project's original single-monitor defaults, for a monitor with no other
    /// config to copy from (see `Config::for_new_monitor`).
    fn default_for(uuid: String) -> Self {
        MonitorConfig {
            uuid,
            folder: dirs::picture_dir().unwrap_or_else(|| PathBuf::from(".")),
            interval_value: 30,
            interval_unit: IntervalUnit::Minutes,
            paused: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub monitors: Vec<MonitorConfig>,
}

/// The pre-multi-monitor file shape, parsed only to migrate an existing user's
/// settings the first time they run a version of this app that understands multiple
/// monitors. See `Config::load()`.
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    folder: PathBuf,
    interval_value: u64,
    interval_unit: IntervalUnit,
    paused: bool,
}

impl Config {
    /// Finds this monitor's own config entry, if it has one.
    pub fn monitor(&self, uuid: &str) -> Option<&MonitorConfig> {
        self.monitors.iter().find(|m| m.uuid == uuid)
    }

    /// Builds a config entry for a monitor never seen before: copies the primary
    /// monitor's folder/interval/pause state if there is one and it has a config
    /// entry, otherwise falls back to this project's original single-monitor
    /// defaults.
    pub fn for_new_monitor(&self, uuid: &str, primary_uuid: Option<&str>) -> MonitorConfig {
        let primary = primary_uuid.and_then(|p| self.monitor(p));
        match primary {
            Some(primary) => MonitorConfig {
                uuid: uuid.to_string(),
                folder: primary.folder.clone(),
                interval_value: primary.interval_value,
                interval_unit: primary.interval_unit,
                paused: primary.paused,
            },
            None => MonitorConfig::default_for(uuid.to_string()),
        }
    }

    fn migration_list_monitors(
        env: Option<crate::desktop::DesktopEnvironment>,
    ) -> fn() -> anyhow::Result<Vec<crate::monitors::Monitor>> {
        match env {
            Some(crate::desktop::DesktopEnvironment::Gnome) => crate::monitors::list_gnome_monitors,
            Some(crate::desktop::DesktopEnvironment::Xfce) => crate::monitors::list_xfce_monitors,
            _ => crate::monitors::list_connected_monitors,
        }
    }

    fn from_legacy(legacy: LegacyConfig, uuid: String) -> Config {
        Config {
            monitors: vec![MonitorConfig {
                uuid,
                folder: legacy.folder,
                interval_value: legacy.interval_value,
                interval_unit: legacy.interval_unit,
                paused: legacy.paused,
            }],
        }
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fs_util::atomic_write(path, &text)
    }

    /// Loads `config.toml`, creating it with an empty monitor list if it doesn't
    /// exist yet, and migrating it in place if it's still in the pre-multi-monitor
    /// single-folder format.
    ///
    /// Migration needs to know which monitor to assign the old settings to (the old
    /// format has no UUID), so it asks whichever monitor-listing function matches the
    /// currently detected desktop environment (`list_gnome_monitors` under GNOME,
    /// `list_connected_monitors` otherwise - the same KDE-vs-GNOME decision as
    /// `select_backend` in the daemon and `monitor_source` in the GUI, via
    /// `migration_list_monitors`) for whichever monitor is currently primary. If that
    /// fails (e.g. `kscreen-doctor` isn't installed) or no monitor is connected,
    /// migration is skipped for now and an empty config is used instead - the old file
    /// on disk is left untouched unless migration actually succeeds, so a later
    /// successful detection can retry it.
    pub fn load() -> anyhow::Result<Config> {
        let path = config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.save()?;
            return Ok(cfg);
        }

        let text = std::fs::read_to_string(&path)?;
        let value: toml::Value = toml::from_str(&text)?;
        if value.get("monitors").is_some() {
            return Ok(toml::from_str(&text)?);
        }

        let Ok(legacy) = toml::from_str::<LegacyConfig>(&text) else {
            // Neither shape is recognized - start fresh rather than erroring,
            // matching this project's existing "malformed config must never be
            // fatal" policy.
            return Ok(Config::default());
        };
        let list_monitors = Config::migration_list_monitors(crate::desktop::detect_desktop_environment());
        let Some(primary_uuid) = list_monitors()
            .ok()
            .and_then(|monitors| monitors.into_iter().find(|m| m.is_primary).map(|m| m.uuid))
        else {
            return Ok(Config::default());
        };

        let migrated = Config::from_legacy(legacy, primary_uuid);
        migrated.save()?;
        Ok(migrated)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&config_path())
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

pub fn gui_lock_path() -> PathBuf {
    config_dir().join("gui.lock")
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

    fn sample_monitor_config(uuid: &str) -> MonitorConfig {
        MonitorConfig {
            uuid: uuid.to_string(),
            folder: PathBuf::from("/tmp/wallpapers"),
            interval_value: 45,
            interval_unit: IntervalUnit::Hours,
            paused: true,
        }
    }

    #[test]
    fn config_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config { monitors: vec![sample_monitor_config("uuid-a")] };

        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();

        assert_eq!(loaded, cfg);
    }

    #[test]
    fn monitor_finds_the_matching_entry_by_uuid() {
        let cfg = Config { monitors: vec![sample_monitor_config("uuid-a")] };
        assert!(cfg.monitor("uuid-a").is_some());
        assert!(cfg.monitor("uuid-b").is_none());
    }

    #[test]
    fn for_new_monitor_copies_the_primary_monitors_settings() {
        let cfg = Config { monitors: vec![sample_monitor_config("primary")] };

        let fresh = cfg.for_new_monitor("new-uuid", Some("primary"));

        assert_eq!(fresh.uuid, "new-uuid");
        assert_eq!(fresh.folder, PathBuf::from("/tmp/wallpapers"));
        assert_eq!(fresh.interval_value, 45);
        assert_eq!(fresh.interval_unit, IntervalUnit::Hours);
        assert!(fresh.paused);
    }

    #[test]
    fn for_new_monitor_falls_back_to_defaults_when_there_is_no_primary() {
        let cfg = Config::default();

        let fresh = cfg.for_new_monitor("new-uuid", None);

        assert_eq!(fresh.uuid, "new-uuid");
        assert_eq!(fresh.interval_value, 30);
        assert_eq!(fresh.interval_unit, IntervalUnit::Minutes);
        assert!(!fresh.paused);
    }

    /// Regression test for the whole-branch review finding that `Config::load()`'s
    /// legacy-config migration hardcoded `list_connected_monitors()` (KDE-only), so
    /// under GNOME it always failed to find a primary monitor and silently discarded
    /// the user's real settings in favor of `Config::default()`. `detect_from_value`
    /// reads the real `$XDG_CURRENT_DESKTOP` env var, which other tests in this crate
    /// may race on if mutated directly, so this instead exercises
    /// `migration_list_monitors` (the exact decision `load()` now delegates to) in
    /// isolation with an explicit `DesktopEnvironment::Gnome`, proving migration would
    /// resolve to GNOME's single shared-desktop UUID rather than falling through to
    /// `Config::default()`.
    #[test]
    fn migration_list_monitors_resolves_the_gnome_shared_desktop_uuid_as_primary() {
        let list_monitors = Config::migration_list_monitors(Some(crate::desktop::DesktopEnvironment::Gnome));

        let primary_uuid = list_monitors()
            .unwrap()
            .into_iter()
            .find(|m| m.is_primary)
            .map(|m| m.uuid);

        assert_eq!(primary_uuid.as_deref(), Some(crate::monitors::GNOME_SHARED_MONITOR_UUID));
    }

    /// Companion to the test above: KDE (and an undetected desktop) must still resolve
    /// to `list_connected_monitors`, unchanged from this fix.
    #[test]
    fn migration_list_monitors_falls_back_to_kde_listing_for_non_gnome() {
        let kde = Config::migration_list_monitors(Some(crate::desktop::DesktopEnvironment::Kde));
        let undetected = Config::migration_list_monitors(None);

        assert_eq!(
            kde as *const () as usize,
            crate::monitors::list_connected_monitors as *const () as usize
        );
        assert_eq!(
            undetected as *const () as usize,
            crate::monitors::list_connected_monitors as *const () as usize
        );
    }

    #[test]
    fn migration_list_monitors_resolves_xfce_listing_for_xfce() {
        let xfce = Config::migration_list_monitors(Some(crate::desktop::DesktopEnvironment::Xfce));
        assert_eq!(
            xfce as *const () as usize,
            crate::monitors::list_xfce_monitors as *const () as usize
        );
    }

    #[test]
    fn from_legacy_converts_the_old_flat_shape_into_one_monitor_entry() {
        let legacy_toml = r#"
            folder = "/home/user/Wallpapers"
            interval_value = 45
            interval_unit = "hours"
            paused = true
        "#;
        let legacy: LegacyConfig = toml::from_str(legacy_toml).unwrap();

        let migrated = Config::from_legacy(legacy, "primary-uuid".to_string());

        assert_eq!(migrated.monitors.len(), 1);
        assert_eq!(migrated.monitors[0].uuid, "primary-uuid");
        assert_eq!(migrated.monitors[0].folder, PathBuf::from("/home/user/Wallpapers"));
        assert_eq!(migrated.monitors[0].interval_value, 45);
        assert_eq!(migrated.monitors[0].interval_unit, IntervalUnit::Hours);
        assert!(migrated.monitors[0].paused);

        // and the migrated shape round-trips cleanly, same as any other Config
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migrated.toml");
        migrated.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), migrated);
    }
}
