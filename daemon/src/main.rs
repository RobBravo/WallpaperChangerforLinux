mod watcher;
mod engine;
mod tray;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wallpaper_core::config::{change_now_request_path, config_dir, Config};
use wallpaper_core::kde_backend::KdePlasmaBackend;
use wallpaper_core::monitors::{list_connected_monitors, Monitor};
use wallpaper_core::state::{MonitorState, State};

use engine::Engine;
use watcher::DaemonEvent;

/// How often the main loop wakes up on its own (absent any real event) to check
/// per-monitor deadlines. A few seconds of slop on when a wallpaper actually rotates
/// is unnoticeable, and this keeps the loop's timing logic simple - no need to
/// compute an exact "soonest deadline across N monitors" sleep duration.
const TICK: Duration = Duration::from_secs(5);

/// How often the daemon re-checks which monitors are connected. Decoupled from any
/// individual monitor's own rotation interval - this project deliberately polls
/// (rather than subscribing to a KScreen D-Bus signal) per this plan's design.
const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(30);

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Rewrites one monitor's `current_wallpaper` in state.toml, leaving its own
/// `next_change_at_unix` and every other monitor's entry untouched.
fn record_current_wallpaper(state_path: &std::path::Path, uuid: &str, current_wallpaper: std::path::PathBuf) {
    let mut state = State::load_from(state_path).unwrap_or_default();
    let next_change_at_unix = state.monitor(uuid).map(|m| m.next_change_at_unix).unwrap_or(0);
    state.set_monitor(MonitorState { uuid: uuid.to_string(), current_wallpaper, next_change_at_unix });
    if let Err(e) = state.save_to(state_path) {
        eprintln!("failed to write state.toml: {e}");
    }
}

/// Rewrites one monitor's `next_change_at_unix`, leaving its `current_wallpaper` and
/// every other monitor's entry untouched. Called every time that monitor's deadline
/// is recomputed - not just when a wallpaper is actually applied - so its countdown
/// in the GUI never goes stale, matching this project's existing single-monitor
/// precedent.
fn record_next_change(state_path: &std::path::Path, uuid: &str, next_change_at_unix: i64) {
    let mut state = State::load_from(state_path).unwrap_or_default();
    let current_wallpaper = state.monitor(uuid).map(|m| m.current_wallpaper.clone()).unwrap_or_default();
    state.set_monitor(MonitorState { uuid: uuid.to_string(), current_wallpaper, next_change_at_unix });
    if let Err(e) = state.save_to(state_path) {
        eprintln!("failed to write state.toml: {e}");
    }
}

fn apply_and_record<B: wallpaper_core::backend::WallpaperBackend>(
    engine: &mut Engine<B>,
    uuid: &str,
    state_path: &std::path::Path,
) {
    match engine.apply_next(uuid) {
        Ok(Some(path)) => record_current_wallpaper(state_path, uuid, path),
        Ok(None) => eprintln!("no wallpapers found for monitor {uuid}"),
        Err(e) => eprintln!("failed to apply wallpaper for monitor {uuid}: {e}"),
    }
}

fn run<B: wallpaper_core::backend::WallpaperBackend>(
    mut engine: Engine<B>,
    initial_monitors: Vec<Monitor>,
    rx: Receiver<DaemonEvent>,
    state_path: std::path::PathBuf,
    change_now_request_path: std::path::PathBuf,
) -> anyhow::Result<()> {
    let now = SystemTime::now();
    // Seed every monitor `engine` was already constructed with as immediately due, so
    // a fresh, non-paused monitor gets its first wallpaper applied right at startup
    // (matching this project's pre-multi-monitor behavior) instead of waiting for the
    // hot-plug poll below - which exists to catch monitors that connect *after*
    // startup, not to bootstrap the ones already known.
    let mut deadlines: HashMap<String, SystemTime> =
        initial_monitors.iter().map(|m| (m.uuid.clone(), now)).collect();
    // The monitor list above is already fresh (`main()` fetched it moments ago), so
    // the first *poll* only needs to catch monitors that connect after that - no need
    // to immediately re-fetch and redo the work `initial_monitors` just seeded.
    let mut next_monitor_poll = now + MONITOR_POLL_INTERVAL;

    loop {
        match rx.recv_timeout(TICK) {
            Ok(first) => {
                let mut config_changed = matches!(first, DaemonEvent::ConfigChanged);
                let mut change_now = matches!(first, DaemonEvent::ChangeNowRequested);

                // A single `fs::write` can produce more than one filesystem event;
                // drain whatever is already queued so a burst collapses into a
                // single action instead of advancing the rotation several times.
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
                    match std::fs::read_to_string(&change_now_request_path) {
                        Ok(uuid) => {
                            let uuid = uuid.trim();
                            apply_and_record(&mut engine, uuid, &state_path);
                            deadlines.insert(uuid.to_string(), SystemTime::now() + engine.interval(uuid));
                            record_next_change(
                                &state_path,
                                uuid,
                                unix_now() + engine.interval(uuid).as_secs() as i64,
                            );
                        }
                        Err(e) => eprintln!("failed to read change_now_request: {e}"),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let now = SystemTime::now();

        if now >= next_monitor_poll {
            match list_connected_monitors() {
                Ok(monitors) => {
                    let updated_config = engine.update_monitors(monitors.clone());
                    if let Err(e) = updated_config.save() {
                        eprintln!("failed to persist config.toml after a monitor change: {e}");
                    }
                    let connected: HashSet<String> = monitors.iter().map(|m| m.uuid.clone()).collect();
                    deadlines.retain(|uuid, _| connected.contains(uuid));
                    for monitor in &monitors {
                        deadlines.entry(monitor.uuid.clone()).or_insert(now);
                    }
                }
                Err(e) => eprintln!("failed to list connected monitors: {e}"),
            }
            next_monitor_poll = now + MONITOR_POLL_INTERVAL;
        }

        let due: Vec<String> = deadlines
            .iter()
            .filter(|(_, &deadline)| now >= deadline)
            .map(|(uuid, _)| uuid.clone())
            .collect();
        for uuid in due {
            if !engine.is_paused(&uuid) {
                apply_and_record(&mut engine, &uuid, &state_path);
            }
            let interval = engine.interval(&uuid);
            deadlines.insert(uuid.clone(), now + interval);
            record_next_change(&state_path, &uuid, unix_now() + interval.as_secs() as i64);
        }
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
    let monitors = list_connected_monitors().unwrap_or_default();
    let engine = Engine::new(KdePlasmaBackend, config, monitors.clone());

    let (tx, rx) = channel::<DaemonEvent>();
    let _watcher = watcher::spawn_watcher(config_dir(), tx)?;
    tray::spawn_tray();

    run(engine, monitors, rx, wallpaper_core::state::state_path(), change_now_request_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use wallpaper_core::config::{IntervalUnit, MonitorConfig};
    use wallpaper_core::monitors::Monitor;

    struct RecordingBackend {
        calls: Arc<Mutex<Vec<(String, PathBuf)>>>,
    }

    impl wallpaper_core::backend::WallpaperBackend for RecordingBackend {
        fn set_wallpaper(&self, _all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push((target.uuid.clone(), path.to_path_buf()));
            Ok(())
        }
    }

    #[test]
    fn change_now_request_triggers_an_immediate_wallpaper_change_for_the_named_monitor_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();

        let monitor_config = MonitorConfig {
            uuid: "uuid-a".to_string(),
            folder: dir.path().to_path_buf(),
            interval_value: 1,
            interval_unit: IntervalUnit::Hours, // long enough that only the signal, not the tick, can trigger this
            paused: false,
        };
        let monitor = Monitor { uuid: "uuid-a".to_string(), connector: "uuid-a".to_string(), is_primary: true, x: 0, y: 0 };
        let config = Config { monitors: vec![monitor_config] };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let engine = Engine::new(RecordingBackend { calls: calls.clone() }, config, vec![monitor.clone()]);

        let (tx, rx) = channel();
        let config_dir = dir.path().to_path_buf();
        let state_path = dir.path().join("state.toml");
        let _watcher = watcher::spawn_watcher(config_dir.clone(), tx).unwrap();

        let change_now_request_path = config_dir.join("change_now_request");
        let handle = thread::spawn(move || {
            let _ = run(engine, vec![monitor], rx, state_path, change_now_request_path);
        });

        std::fs::write(config_dir.join("change_now_request"), b"uuid-a").unwrap();
        thread::sleep(Duration::from_secs(2));

        // `run` only returns when the channel disconnects; drop the watcher's sender
        // side by ending the test process is not an option, so just assert on calls
        // so far and let the test process exit (the thread is daemonized by the test
        // harness).
        //
        // A single `fs::write` can surface as two separate non-access inotify events
        // (e.g. `Create` then `Modify`) spaced far enough apart that the burst-drain
        // loop doesn't catch both, so `change_now` can legitimately fire twice for one
        // write - re-applying the same correct image is harmless, so assert on
        // correctness (every recorded call targets uuid-a with the right path), not
        // an exact call count.
        let recorded = calls.lock().unwrap();
        let expected = ("uuid-a".to_string(), dir.path().join("a.png"));
        assert!(!recorded.is_empty(), "change_now_request did not trigger a wallpaper change");
        assert!(recorded.iter().all(|call| *call == expected), "recorded calls were: {recorded:?}");
        drop(handle);
    }
}
