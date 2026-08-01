use std::path::PathBuf;
use std::time::Duration;
use wallpaper_core::backend::WallpaperBackend;
use wallpaper_core::config::Config;
use wallpaper_core::queue::WallpaperQueue;
use wallpaper_core::scanner::list_wallpapers;

/// Lower bound on the rotation interval. `config.toml` is hand-editable and carries no
/// validation, so an `interval_value = 0` would otherwise make `recv_timeout` return
/// instantly and spin the main loop, hammering D-Bus and the disk.
const MIN_INTERVAL: Duration = Duration::from_secs(60);

pub struct Engine<B: WallpaperBackend> {
    backend: B,
    config: Config,
    queue: WallpaperQueue,
}

impl<B: WallpaperBackend> Engine<B> {
    pub fn new(backend: B, config: Config) -> Self {
        let queue = WallpaperQueue::new(list_wallpapers(&config.folder));
        Engine { backend, config, queue }
    }

    pub fn is_paused(&self) -> bool {
        self.config.paused
    }

    pub fn interval(&self) -> Duration {
        self.config
            .interval_unit
            .to_duration(self.config.interval_value)
            .max(MIN_INTERVAL)
    }

    /// Rescans the wallpaper folder and rebuilds the queue if its contents changed.
    ///
    /// The folder is a live directory: the user can drop new images into it or delete
    /// existing ones at any time. Without this the daemon would keep serving the
    /// snapshot taken at startup, handing deleted paths to the backend (which happily
    /// returns `Ok` for a nonexistent file) and never picking up new images.
    fn refresh_queue(&mut self) {
        let scanned = list_wallpapers(&self.config.folder);
        if scanned.as_slice() != self.queue.all() {
            self.queue = WallpaperQueue::new(scanned);
        }
    }

    pub fn apply_next(&mut self) -> anyhow::Result<Option<PathBuf>> {
        self.refresh_queue();
        match self.queue.next() {
            Some(path) => {
                self.backend.set_wallpaper(&path)?;
                Ok(Some(path))
            }
            None => Ok(None),
        }
    }

    pub fn update_config(&mut self, new_config: Config) {
        if new_config.folder != self.config.folder {
            self.queue = WallpaperQueue::new(list_wallpapers(&new_config.folder));
        }
        self.config = new_config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use wallpaper_core::config::IntervalUnit;

    struct FakeBackend {
        calls: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl WallpaperBackend for FakeBackend {
        fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    fn test_config(folder: PathBuf) -> Config {
        Config {
            folder,
            interval_value: 1,
            interval_unit: IntervalUnit::Minutes,
            paused: false,
        }
    }

    #[test]
    fn apply_next_calls_backend_with_an_image_from_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend { calls: calls.clone() };
        let mut engine = Engine::new(backend, test_config(dir.path().to_path_buf()));

        let applied = engine.apply_next().unwrap();

        assert_eq!(applied, Some(dir.path().join("a.png")));
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn apply_next_returns_none_when_folder_has_no_images() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, test_config(dir.path().to_path_buf()));

        assert_eq!(engine.apply_next().unwrap(), None);
    }

    #[test]
    fn update_config_rebuilds_queue_when_folder_changes() {
        let dir_a = tempfile::tempdir().unwrap();
        std::fs::write(dir_a.path().join("a.png"), b"x").unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::write(dir_b.path().join("b.png"), b"x").unwrap();

        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, test_config(dir_a.path().to_path_buf()));
        engine.update_config(test_config(dir_b.path().to_path_buf()));

        assert_eq!(engine.apply_next().unwrap(), Some(dir_b.path().join("b.png")));
    }

    #[test]
    fn apply_next_picks_up_an_image_added_after_the_engine_was_created() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, test_config(dir.path().to_path_buf()));

        assert_eq!(engine.apply_next().unwrap(), Some(dir.path().join("a.png")));

        let added = dir.path().join("b.png");
        std::fs::write(&added, b"x").unwrap();

        let mut seen_added = false;
        for _ in 0..10 {
            if engine.apply_next().unwrap() == Some(added.clone()) {
                seen_added = true;
                break;
            }
        }
        assert!(seen_added, "image added after startup was never returned by apply_next");
    }

    #[test]
    fn apply_next_stops_returning_an_image_removed_from_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        let removed = dir.path().join("b.png");
        std::fs::write(&removed, b"x").unwrap();
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, test_config(dir.path().to_path_buf()));

        engine.apply_next().unwrap();
        std::fs::remove_file(&removed).unwrap();

        for _ in 0..10 {
            assert_eq!(
                engine.apply_next().unwrap(),
                Some(dir.path().join("a.png")),
                "a deleted image was still handed to the backend"
            );
        }
    }

    #[test]
    fn apply_next_returns_none_once_the_folder_is_emptied() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, test_config(dir.path().to_path_buf()));

        assert!(engine.apply_next().unwrap().is_some());
        std::fs::remove_file(dir.path().join("a.png")).unwrap();

        assert_eq!(engine.apply_next().unwrap(), None);
    }

    #[test]
    fn interval_is_clamped_to_a_sane_minimum_when_config_says_zero() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut config = test_config(dir.path().to_path_buf());
        config.interval_value = 0;
        let engine = Engine::new(backend, config);

        assert!(
            engine.interval() >= Duration::from_secs(60),
            "interval_value = 0 must not produce a busy-loop timeout, got {:?}",
            engine.interval()
        );
    }

    #[test]
    fn is_paused_and_interval_reflect_current_config() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut config = test_config(dir.path().to_path_buf());
        config.paused = true;
        config.interval_value = 2;
        config.interval_unit = IntervalUnit::Hours;
        let engine = Engine::new(backend, config);

        assert!(engine.is_paused());
        assert_eq!(engine.interval(), Duration::from_secs(2 * 3600));
    }
}
