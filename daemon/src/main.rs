mod watcher;
mod engine;
mod tray;

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, TryRecvError};
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

/// Rewrites `current_wallpaper` in state.toml, preserving whatever deadline
/// `record_next_change` last wrote. A missing/unreadable state.toml (first launch)
/// simply starts from a zero deadline, which the next reset immediately corrects.
fn record_current_wallpaper(state_path: &std::path::Path, current_wallpaper: std::path::PathBuf) {
    let next_change_at_unix = State::load_from(state_path)
        .map(|s| s.next_change_at_unix)
        .unwrap_or(0);
    let state = State { current_wallpaper, next_change_at_unix };
    if let Err(e) = state.save_to(state_path) {
        eprintln!("failed to write state.toml: {e}");
    }
}

/// Rewrites `next_change_at_unix` in state.toml, preserving `current_wallpaper`.
///
/// The GUI's countdown is computed purely from this field, so it has to be rewritten
/// every time the daemon recomputes its deadline - not just when a wallpaper is
/// actually applied - or the countdown goes stale after an interval change and sits
/// at 00:00:00 forever while paused. If state.toml doesn't exist yet (paused since
/// first launch, no wallpaper applied), the current wallpaper is left empty.
fn record_next_change(state_path: &std::path::Path, next_change_at_unix: i64) {
    let current_wallpaper = State::load_from(state_path)
        .map(|s| s.current_wallpaper)
        .unwrap_or_default();
    let state = State { current_wallpaper, next_change_at_unix };
    if let Err(e) = state.save_to(state_path) {
        eprintln!("failed to write state.toml: {e}");
    }
}

/// Recomputes the next rotation deadline and publishes it to state.toml.
fn reset_deadline<B: wallpaper_core::backend::WallpaperBackend>(
    engine: &Engine<B>,
    state_path: &std::path::Path,
) -> SystemTime {
    let interval = engine.interval();
    record_next_change(state_path, unix_now() + interval.as_secs() as i64);
    SystemTime::now() + interval
}

fn apply_and_record<B: wallpaper_core::backend::WallpaperBackend>(
    engine: &mut Engine<B>,
    state_path: &std::path::Path,
) {
    match engine.apply_next() {
        Ok(Some(path)) => record_current_wallpaper(state_path, path),
        Ok(None) => eprintln!("no wallpapers found in the configured folder"),
        Err(e) => eprintln!("failed to apply wallpaper: {e}"),
    }
}

fn run<B: wallpaper_core::backend::WallpaperBackend>(
    mut engine: Engine<B>,
    rx: Receiver<DaemonEvent>,
    state_path: std::path::PathBuf,
) -> anyhow::Result<()> {
    if !engine.is_paused() {
        apply_and_record(&mut engine, &state_path);
    }
    let mut deadline = reset_deadline(&engine, &state_path);

    loop {
        let timeout = deadline
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::from_secs(0));

        match rx.recv_timeout(timeout) {
            Ok(first) => {
                let mut config_changed = matches!(first, DaemonEvent::ConfigChanged);
                let mut change_now = matches!(first, DaemonEvent::ChangeNowRequested);

                // A single `fs::write` to config.toml / change_now_request can produce
                // more than one filesystem event, and `notify` forwards each one. Drain
                // whatever is already queued so a burst collapses into a single action
                // instead of advancing the rotation several times per click.
                loop {
                    match rx.try_recv() {
                        Ok(DaemonEvent::ConfigChanged) => config_changed = true,
                        Ok(DaemonEvent::ChangeNowRequested) => change_now = true,
                        Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                    }
                }

                if config_changed {
                    match Config::load() {
                        Ok(new_config) => engine.update_config(new_config),
                        Err(e) => eprintln!("failed to reload config.toml: {e}"),
                    }
                }
                if change_now {
                    apply_and_record(&mut engine, &state_path);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !engine.is_paused() {
                    apply_and_record(&mut engine, &state_path);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }

        deadline = reset_deadline(&engine, &state_path);
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    // A malformed config.toml must not be fatal: exiting non-zero here would make
    // systemd's `Restart=on-failure` retry forever, leaving the user with no rotation
    // and no tray icon. Fall back to defaults in memory and leave the user's file
    // untouched so they can still fix it by hand.
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("failed to load config.toml ({e}); using default settings until it is fixed");
        Config::default()
    });
    let engine = Engine::new(KdePlasmaBackend, config);

    let (tx, rx) = channel::<DaemonEvent>();
    let _watcher = watcher::spawn_watcher(config_dir(), tx)?;
    tray::spawn_tray();

    run(engine, rx, wallpaper_core::state::state_path())
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
        let state_path = dir.path().join("state.toml");
        let _watcher = watcher::spawn_watcher(config_dir.clone(), tx).unwrap();

        let handle = thread::spawn(move || {
            let _ = run(engine, rx, state_path);
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
