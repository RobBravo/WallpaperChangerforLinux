use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use wallpaper_core::backend::WallpaperBackend;
use wallpaper_core::config::Config;
use wallpaper_core::monitors::Monitor;
use wallpaper_core::queue::WallpaperQueue;
use wallpaper_core::scanner::list_wallpapers;

/// Lower bound on the rotation interval. `config.toml` is hand-editable and carries no
/// validation, so an `interval_value = 0` would otherwise make the daemon's loop spin
/// on that one monitor, hammering D-Bus and the disk.
const MIN_INTERVAL: Duration = Duration::from_secs(60);

pub struct Engine<B: WallpaperBackend> {
    backend: B,
    config: Config,
    monitors: Vec<Monitor>,
    queues: HashMap<String, WallpaperQueue>,
}

impl<B: WallpaperBackend> Engine<B> {
    pub fn new(backend: B, config: Config, monitors: Vec<Monitor>) -> Self {
        let queues = config
            .monitors
            .iter()
            .map(|m| (m.uuid.clone(), WallpaperQueue::new(list_wallpapers(&m.folder))))
            .collect();
        Engine { backend, config, monitors, queues }
    }

    pub fn is_paused(&self, uuid: &str) -> bool {
        self.config.monitor(uuid).map(|m| m.paused).unwrap_or(true)
    }

    pub fn interval(&self, uuid: &str) -> Duration {
        self.config
            .monitor(uuid)
            .map(|m| m.interval_unit.to_duration(m.interval_value).max(MIN_INTERVAL))
            .unwrap_or(MIN_INTERVAL)
    }

    fn refresh_queue(&mut self, uuid: &str, folder: &std::path::Path) {
        let scanned = list_wallpapers(folder);
        let needs_rebuild = self
            .queues
            .get(uuid)
            .map(|q| scanned.as_slice() != q.all())
            .unwrap_or(true);
        if needs_rebuild {
            self.queues.insert(uuid.to_string(), WallpaperQueue::new(scanned));
        }
    }

    /// Applies the next wallpaper for one monitor. `Ok(None)` if that monitor isn't
    /// currently connected (no `MonitorConfig` entry, or not in the last-known
    /// connected list) or its folder has no images.
    pub fn apply_next(&mut self, uuid: &str) -> anyhow::Result<Option<PathBuf>> {
        let Some(monitor_config) = self.config.monitor(uuid).cloned() else { return Ok(None) };
        let Some(target) = self.monitors.iter().find(|m| m.uuid == uuid).cloned() else {
            return Ok(None);
        };
        self.refresh_queue(uuid, &monitor_config.folder);
        let Some(queue) = self.queues.get_mut(uuid) else { return Ok(None) };
        match queue.next() {
            Some(path) => {
                self.backend.set_wallpaper(&self.monitors, &target, &path)?;
                Ok(Some(path))
            }
            None => Ok(None),
        }
    }

    /// Rebuilds only the queues of monitors whose folder actually changed, leaving
    /// every other monitor's shuffle progress untouched.
    pub fn update_config(&mut self, new_config: Config) {
        for monitor in &new_config.monitors {
            let folder_changed = self
                .config
                .monitor(&monitor.uuid)
                .map(|old| old.folder != monitor.folder)
                .unwrap_or(true);
            if folder_changed {
                self.queues
                    .insert(monitor.uuid.clone(), WallpaperQueue::new(list_wallpapers(&monitor.folder)));
            }
        }
        self.config = new_config;
    }

    /// Reconciles the live connected-monitor list: any UUID never seen before gets a
    /// fresh `MonitorConfig` entry (copying the primary monitor's settings, per
    /// `Config::for_new_monitor`) and a fresh rotation queue. Does not write anything
    /// to disk itself - returns `Some(Config)` for the caller to persist only when a
    /// monitor was actually added, `None` when the connected set didn't introduce
    /// anything new to `self.config` (keeping `Engine` free of file I/O concerns,
    /// matching this project's existing separation between rotation logic and
    /// persistence, done in `main.rs`).
    ///
    /// The `None` case matters: the caller polls this every 30 seconds regardless of
    /// whether anything changed, and persisting an unconditional `self.config.clone()`
    /// every cycle made every poll indistinguishable from a genuine edit to the
    /// filesystem watcher - found during live testing to both churn config.toml
    /// needlessly and risk clobbering an in-flight migration or a GUI save that landed
    /// mid-poll with stale in-memory data.
    pub fn update_monitors(&mut self, monitors: Vec<Monitor>) -> Option<Config> {
        // Prefer the monitor currently flagged primary as the settings donor, but only
        // when it's already configured - for KDE/GNOME this is nearly always true (a
        // primary monitor was almost always already known before a sibling monitor
        // shows up), so this matches prior behavior exactly there. Under XFCE, though,
        // "primary" is just whichever monitor id sorts first alphabetically (see
        // `list_xfce_monitors`'s doc comment) - a monitor that is itself brand new in
        // this very batch can be alphabetically first, and it has no config history of
        // its own to donate. In that case, fall back to any OTHER monitor in the batch
        // that's already configured; if none of the connected monitors have a config
        // entry yet (e.g. this is the very first monitor(s) ever seen), `primary_uuid`
        // stays `None`, exactly matching prior behavior for that edge case.
        let primary_uuid = monitors
            .iter()
            .find(|m| m.is_primary && self.config.monitor(&m.uuid).is_some())
            .or_else(|| monitors.iter().find(|m| self.config.monitor(&m.uuid).is_some()))
            .map(|m| m.uuid.clone());
        let mut added_a_monitor = false;
        for monitor in &monitors {
            if self.config.monitor(&monitor.uuid).is_none() {
                let fresh = self.config.for_new_monitor(&monitor.uuid, primary_uuid.as_deref());
                self.queues
                    .insert(monitor.uuid.clone(), WallpaperQueue::new(list_wallpapers(&fresh.folder)));
                self.config.monitors.push(fresh);
                added_a_monitor = true;
            }
        }
        self.monitors = monitors;
        added_a_monitor.then(|| self.config.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use wallpaper_core::config::{IntervalUnit, MonitorConfig};

    struct FakeBackend {
        calls: Arc<Mutex<Vec<(String, PathBuf)>>>,
    }

    impl WallpaperBackend for FakeBackend {
        fn set_wallpaper(&self, _all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push((target.uuid.clone(), path.to_path_buf()));
            Ok(())
        }
    }

    fn monitor(uuid: &str, is_primary: bool) -> Monitor {
        Monitor { uuid: uuid.to_string(), connector: uuid.to_string(), is_primary, x: 0, y: 0 }
    }

    fn monitor_config(uuid: &str, folder: PathBuf) -> MonitorConfig {
        MonitorConfig {
            uuid: uuid.to_string(),
            folder,
            interval_value: 1,
            interval_unit: IntervalUnit::Minutes,
            paused: false,
        }
    }

    #[test]
    fn apply_next_calls_the_backend_with_an_image_from_that_monitors_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        let config = Config { monitors: vec![monitor_config("uuid-a", dir.path().to_path_buf())] };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend { calls: calls.clone() };
        let mut engine = Engine::new(backend, config, vec![monitor("uuid-a", true)]);

        let applied = engine.apply_next("uuid-a").unwrap();

        assert_eq!(applied, Some(dir.path().join("a.png")));
        assert_eq!(calls.lock().unwrap().as_slice(), &[("uuid-a".to_string(), dir.path().join("a.png"))]);
    }

    #[test]
    fn two_monitors_rotate_independently() {
        let dir_a = tempfile::tempdir().unwrap();
        std::fs::write(dir_a.path().join("a.png"), b"x").unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::write(dir_b.path().join("b.png"), b"x").unwrap();

        let config = Config {
            monitors: vec![
                monitor_config("uuid-a", dir_a.path().to_path_buf()),
                monitor_config("uuid-b", dir_b.path().to_path_buf()),
            ],
        };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend { calls: calls.clone() };
        let mut engine = Engine::new(backend, config, vec![monitor("uuid-a", true), monitor("uuid-b", false)]);

        engine.apply_next("uuid-a").unwrap();
        engine.apply_next("uuid-b").unwrap();

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded[0], ("uuid-a".to_string(), dir_a.path().join("a.png")));
        assert_eq!(recorded[1], ("uuid-b".to_string(), dir_b.path().join("b.png")));
    }

    #[test]
    fn apply_next_returns_none_for_an_unconfigured_monitor() {
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, Config::default(), vec![]);
        assert_eq!(engine.apply_next("unknown-uuid").unwrap(), None);
    }

    #[test]
    fn is_paused_and_interval_are_read_per_monitor() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg_a = monitor_config("uuid-a", dir.path().to_path_buf());
        cfg_a.paused = true;
        cfg_a.interval_value = 2;
        cfg_a.interval_unit = IntervalUnit::Hours;
        let cfg_b = monitor_config("uuid-b", dir.path().to_path_buf());

        let config = Config { monitors: vec![cfg_a, cfg_b] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let engine = Engine::new(backend, config, vec![monitor("uuid-a", true), monitor("uuid-b", false)]);

        assert!(engine.is_paused("uuid-a"));
        assert_eq!(engine.interval("uuid-a"), Duration::from_secs(2 * 3600));
        assert!(!engine.is_paused("uuid-b"));
        assert_eq!(engine.interval("uuid-b"), MIN_INTERVAL);
    }

    #[test]
    fn interval_is_clamped_to_a_sane_minimum_when_config_says_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = monitor_config("uuid-a", dir.path().to_path_buf());
        cfg.interval_value = 0;
        let config = Config { monitors: vec![cfg] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let engine = Engine::new(backend, config, vec![monitor("uuid-a", true)]);

        assert!(engine.interval("uuid-a") >= Duration::from_secs(60));
    }

    #[test]
    fn update_monitors_gives_a_newly_connected_monitor_the_primarys_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut primary_cfg = monitor_config("primary", dir.path().to_path_buf());
        primary_cfg.interval_value = 45;
        primary_cfg.interval_unit = IntervalUnit::Hours;
        let config = Config { monitors: vec![primary_cfg] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, config, vec![monitor("primary", true)]);

        let updated_config = engine
            .update_monitors(vec![monitor("primary", true), monitor("new", false)])
            .expect("a new monitor connected, so a Config to persist was expected");

        let new_entry = updated_config.monitor("new").unwrap();
        assert_eq!(new_entry.interval_value, 45);
        assert_eq!(new_entry.interval_unit, IntervalUnit::Hours);
    }

    /// Regression test for the XFCE case where a brand-new monitor happens to sort
    /// alphabetically first and gets flagged `is_primary` in the very same batch that
    /// introduces it - it must NOT be treated as the settings donor over a sibling
    /// monitor that's actually already configured. See `update_monitors`'s doc comment
    /// on `primary_uuid` for the full scenario.
    #[test]
    fn update_monitors_does_not_let_a_newly_primary_monitor_shadow_an_already_configured_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let mut hdmi_cfg = monitor_config("HDMI-1", dir.path().to_path_buf());
        hdmi_cfg.interval_value = 240;
        let config = Config { monitors: vec![hdmi_cfg] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, config, vec![monitor("HDMI-1", false)]);

        let updated_config = engine
            .update_monitors(vec![monitor("DP-1", true), monitor("HDMI-1", false)])
            .expect("a new monitor connected, so a Config to persist was expected");

        let new_entry = updated_config.monitor("DP-1").unwrap();
        assert_eq!(
            new_entry.interval_value, 240,
            "DP-1 should inherit already-configured HDMI-1's settings, not hardcoded defaults"
        );
    }

    /// Regression test: the 30-second hot-plug poll calls `update_monitors` on every
    /// cycle regardless of whether anything actually changed. Persisting `None`'s
    /// absence (rather than unconditionally re-saving `self.config` every cycle) is
    /// what lets `main.rs` skip a needless config.toml re-save when the connected set
    /// is unchanged - found necessary during Task 9's live testing: an unconditional
    /// re-save every 30s made every reload look like a genuine edit to the watcher,
    /// and (before this fix) could overwrite an in-flight migration or a GUI save that
    /// landed mid-poll with stale in-memory data.
    #[test]
    fn update_monitors_returns_none_when_nothing_new_connects() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config { monitors: vec![monitor_config("uuid-a", dir.path().to_path_buf())] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, config, vec![monitor("uuid-a", true)]);

        let updated_config = engine.update_monitors(vec![monitor("uuid-a", true)]);

        assert!(updated_config.is_none(), "no new monitor connected, so nothing should need persisting");
    }

    #[test]
    fn update_monitors_leaves_an_existing_monitors_config_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg_a = monitor_config("uuid-a", dir.path().to_path_buf());
        cfg_a.interval_value = 99;
        let config = Config { monitors: vec![cfg_a] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, config, vec![monitor("uuid-a", true)]);

        let updated_config = engine.update_monitors(vec![monitor("uuid-a", true), monitor("new", false)]);

        assert_eq!(updated_config.unwrap().monitor("uuid-a").unwrap().interval_value, 99);
    }

    #[test]
    fn update_config_rebuilds_only_the_queue_whose_folder_changed() {
        let dir_a1 = tempfile::tempdir().unwrap();
        std::fs::write(dir_a1.path().join("old.png"), b"x").unwrap();
        let dir_a2 = tempfile::tempdir().unwrap();
        std::fs::write(dir_a2.path().join("new.png"), b"x").unwrap();

        let config = Config { monitors: vec![monitor_config("uuid-a", dir_a1.path().to_path_buf())] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, config, vec![monitor("uuid-a", true)]);

        engine.update_config(Config { monitors: vec![monitor_config("uuid-a", dir_a2.path().to_path_buf())] });

        assert_eq!(engine.apply_next("uuid-a").unwrap(), Some(dir_a2.path().join("new.png")));
    }
}
