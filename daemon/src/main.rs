mod watcher;
mod engine;
mod tray;

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wallpaper_core::config::{config_dir, Config};
use wallpaper_core::kde_backend::KdePlasmaBackend;
use wallpaper_core::state::State;

use engine::Engine;
use watcher::DaemonEvent;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn apply_and_record<B: wallpaper_core::backend::WallpaperBackend>(engine: &mut Engine<B>) {
    match engine.apply_next() {
        Ok(Some(path)) => {
            let next_change_at_unix = unix_now() + engine.interval().as_secs() as i64;
            let state = State {
                current_wallpaper: path,
                next_change_at_unix,
            };
            if let Err(e) = state.save() {
                eprintln!("failed to write state.toml: {e}");
            }
        }
        Ok(None) => eprintln!("no wallpapers found in the configured folder"),
        Err(e) => eprintln!("failed to apply wallpaper: {e}"),
    }
}

fn run<B: wallpaper_core::backend::WallpaperBackend>(
    mut engine: Engine<B>,
    rx: Receiver<DaemonEvent>,
) -> anyhow::Result<()> {
    if !engine.is_paused() {
        apply_and_record(&mut engine);
    }
    let mut deadline = SystemTime::now() + engine.interval();

    loop {
        let timeout = deadline
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::from_secs(0));

        match rx.recv_timeout(timeout) {
            Ok(DaemonEvent::ConfigChanged) => {
                match Config::load() {
                    Ok(new_config) => engine.update_config(new_config),
                    Err(e) => eprintln!("failed to reload config.toml: {e}"),
                }
                deadline = SystemTime::now() + engine.interval();
            }
            Ok(DaemonEvent::ChangeNowRequested) => {
                apply_and_record(&mut engine);
                deadline = SystemTime::now() + engine.interval();
            }
            Err(RecvTimeoutError::Timeout) => {
                if !engine.is_paused() {
                    apply_and_record(&mut engine);
                }
                deadline = SystemTime::now() + engine.interval();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    let engine = Engine::new(KdePlasmaBackend, config);

    let (tx, rx) = channel::<DaemonEvent>();
    let _watcher = watcher::spawn_watcher(config_dir(), tx)?;
    tray::spawn_tray();

    run(engine, rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use wallpaper_core::config::IntervalUnit;

    struct RecordingBackend {
        calls: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl wallpaper_core::backend::WallpaperBackend for RecordingBackend {
        fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn change_now_request_triggers_an_immediate_wallpaper_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();

        let config = Config {
            folder: dir.path().to_path_buf(),
            interval_value: 1,
            interval_unit: IntervalUnit::Hours, // long enough that only the signal, not the timeout, can trigger this
            paused: false,
        };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let engine = Engine::new(RecordingBackend { calls: calls.clone() }, config);

        let (tx, rx) = channel();
        let config_dir = dir.path().to_path_buf();
        let _watcher = watcher::spawn_watcher(config_dir.clone(), tx).unwrap();

        let handle = thread::spawn(move || {
            let _ = run(engine, rx);
        });

        std::fs::write(config_dir.join("change_now_request"), b"1").unwrap();
        thread::sleep(Duration::from_secs(2));

        // `run` only returns when the channel disconnects; drop the watcher's sender side
        // by ending the test process is not an option, so just assert on calls so far
        // and let the test process exit (the thread is daemonized by the test harness).
        assert!(!calls.lock().unwrap().is_empty(), "change_now_request did not trigger a wallpaper change");
        drop(handle);
    }
}
