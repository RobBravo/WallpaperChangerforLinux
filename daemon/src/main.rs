mod watcher;
mod engine;
mod tray;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wallpaper_core::backend::WallpaperBackend;
use wallpaper_core::config::{change_now_request_path, config_dir, Config};
use wallpaper_core::desktop::{detect_desktop_environment, DesktopEnvironment};
use wallpaper_core::gnome_backend::GnomeBackend;
use wallpaper_core::kde_backend::KdePlasmaBackend;
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, list_xfce_monitors, Monitor};
use wallpaper_core::xfce_backend::XfceBackend;
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

/// Picks the backend and monitor-listing function for the given desktop environment.
/// Pulled out of `main()` as its own function so the KDE-vs-GNOME decision itself is
/// unit-testable without needing a live desktop session.
fn select_backend(env: DesktopEnvironment) -> (Box<dyn WallpaperBackend>, fn() -> anyhow::Result<Vec<Monitor>>) {
    match env {
        DesktopEnvironment::Kde => (Box::new(KdePlasmaBackend), list_connected_monitors),
        DesktopEnvironment::Gnome => (Box::new(GnomeBackend), list_gnome_monitors),
        DesktopEnvironment::Xfce => (Box::new(XfceBackend), list_xfce_monitors),
    }
}

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
    list_monitors: fn() -> anyhow::Result<Vec<Monitor>>,
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
                    // The 30-second hot-plug poll below unconditionally re-saves
                    // config.toml every cycle even when nothing changed, which the
                    // watcher reports as a `ConfigChanged` event just like a real
                    // edit - capture each tracked monitor's interval *before*
                    // reloading so a genuine interval change can be told apart from
                    // that routine, no-op re-save below.
                    let old_intervals: HashMap<String, Duration> =
                        deadlines.keys().map(|uuid| (uuid.clone(), engine.interval(uuid))).collect();
                    match Config::load() {
                        Ok(new_config) => {
                            engine.update_config(new_config);
                            // The pre-multi-monitor daemon restarted its one deadline
                            // fresh on every loop pass, so an interval change took
                            // effect immediately rather than only after whatever was
                            // left of the previous (possibly much longer) interval
                            // expired. Reproduce that per-monitor, but only for a
                            // monitor whose interval genuinely changed - resetting
                            // every tracked monitor unconditionally would also fire on
                            // the hot-plug poll's routine re-save below, perpetually
                            // postponing rotation before it can ever come due.
                            let now = SystemTime::now();
                            for (uuid, old_interval) in old_intervals {
                                let interval = engine.interval(&uuid);
                                if interval != old_interval {
                                    deadlines.insert(uuid.clone(), now + interval);
                                    record_next_change(&state_path, &uuid, unix_now() + interval.as_secs() as i64);
                                }
                            }
                        }
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
            match list_monitors() {
                Ok(monitors) => {
                    if let Some(updated_config) = engine.update_monitors(monitors.clone()) {
                        if let Err(e) = updated_config.save() {
                            eprintln!("failed to persist config.toml after a monitor change: {e}");
                        }
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
    let Some(desktop_environment) = detect_desktop_environment() else {
        let value = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        anyhow::bail!(
            "desktop environment '{value}' is not supported - this app supports KDE Plasma and GNOME"
        );
    };
    let (backend, list_monitors) = select_backend(desktop_environment);

    // A malformed config.toml must not be fatal: exiting non-zero here would make
    // systemd's `Restart=on-failure` retry forever, leaving the user with no rotation
    // and no tray icon. Fall back to defaults in memory and leave the user's file
    // untouched so they can still fix it by hand.
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("failed to load config.toml ({e}); using default settings until it is fixed");
        Config::default()
    });
    let monitors = list_monitors().unwrap_or_default();
    let engine = Engine::new(backend, config, monitors.clone());

    let (tx, rx) = channel::<DaemonEvent>();
    let _watcher = watcher::spawn_watcher(config_dir(), tx)?;
    tray::spawn_tray(list_monitors);

    run(
        engine,
        monitors,
        rx,
        wallpaper_core::state::state_path(),
        change_now_request_path(),
        list_monitors,
    )
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
    fn select_backend_picks_kde_monitor_listing_for_kde() {
        let (_backend, list_monitors) = select_backend(DesktopEnvironment::Kde);
        assert_eq!(list_monitors as *const () as usize, list_connected_monitors as *const () as usize);
    }

    #[test]
    fn select_backend_picks_gnome_monitor_listing_for_gnome() {
        let (_backend, list_monitors) = select_backend(DesktopEnvironment::Gnome);
        assert_eq!(list_monitors as *const () as usize, list_gnome_monitors as *const () as usize);
    }

    #[test]
    fn select_backend_picks_xfce_monitor_listing_for_xfce() {
        let (_backend, list_monitors) = select_backend(DesktopEnvironment::Xfce);
        assert_eq!(list_monitors as *const () as usize, list_xfce_monitors as *const () as usize);
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
            let _ = run(engine, vec![monitor], rx, state_path, change_now_request_path, list_connected_monitors);
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

    /// Regression test: shortening a monitor's interval must take effect immediately,
    /// not only after whatever remains of its *previous* (possibly much longer)
    /// interval finally expires. Found via live manual testing (Task 9 of the
    /// multi-monitor plan) - the pre-multi-monitor daemon reset its one deadline on
    /// every loop pass, so an interval change was picked up right away; the
    /// per-monitor rewrite only recomputed a monitor's deadline when it actually came
    /// due, so a monitor left running with a long interval would silently ignore a
    /// shorter one saved from the GUI until the old, much longer deadline happened to
    /// expire on its own.
    #[test]
    fn shortening_the_interval_resets_the_deadline_instead_of_waiting_out_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();

        let monitor_config = MonitorConfig {
            uuid: "uuid-a".to_string(),
            folder: dir.path().to_path_buf(),
            interval_value: 1,
            interval_unit: IntervalUnit::Hours, // long enough that only a config reload, not the tick, can shorten it
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
            let _ = run(engine, vec![monitor], rx, state_path, change_now_request_path, list_connected_monitors);
        });

        // Let the startup seed-and-apply happen AND let the loop cycle past its own
        // 5-second TICK at least once, so the deadline this test is about to shorten
        // is genuinely the "one hour away" deadline the due-loop set after consuming
        // the startup seed - not the startup seed itself, which a config-change event
        // arriving within the same first iteration would trivially fold into a single
        // pass and mask this exact regression.
        thread::sleep(Duration::from_secs(6));

        // Save a much shorter interval, same as the GUI's "Guardar" would.
        let shortened_toml = format!(
            "[[monitors]]\nuuid = \"uuid-a\"\nfolder = \"{}\"\ninterval_value = 1\ninterval_unit = \"minutes\"\npaused = false\n",
            dir.path().display()
        );
        std::fs::write(config_dir.join("config.toml"), shortened_toml).unwrap();
        thread::sleep(Duration::from_secs(2));

        let state = State::load_from(&dir.path().join("state.toml")).unwrap();
        let next_change_at_unix = state.monitor("uuid-a").unwrap().next_change_at_unix;
        let now = unix_now();
        // Unreset, the deadline would still be ~3600s away (the original one-hour
        // interval, computed once at startup). Reset, it's ~60s away (the new
        // interval, clamped up to Engine's MIN_INTERVAL floor).
        assert!(
            next_change_at_unix - now < 300,
            "interval change was not picked up promptly: next_change_at_unix is {}s away",
            next_change_at_unix - now
        );

        drop(handle);
    }

    /// Regression test: re-saving config.toml with *no actual change* (exactly what
    /// the hot-plug poll's own `updated_config.save()` does every 30 seconds,
    /// unconditionally, even when nothing about any monitor changed) must not keep
    /// resetting an already-scheduled deadline - found via live manual testing (Task 9
    /// of the multi-monitor plan) as a bug in the fix for the previous regression
    /// test: naively resetting every tracked monitor's deadline on *any*
    /// `ConfigChanged` event perpetually postponed rotation, since the routine poll
    /// re-save re-triggers that same event every 30 seconds, before a
    /// 60-second-minimum interval could ever actually come due.
    #[test]
    fn a_no_op_config_resave_does_not_postpone_an_already_scheduled_rotation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        std::fs::write(dir.path().join("b.png"), b"y").unwrap();

        let monitor_config = MonitorConfig {
            uuid: "uuid-a".to_string(),
            folder: dir.path().to_path_buf(),
            interval_value: 1,
            interval_unit: IntervalUnit::Minutes, // clamped up to Engine's 60s MIN_INTERVAL floor
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
            let _ = run(engine, vec![monitor], rx, state_path, change_now_request_path, list_connected_monitors);
        });

        // Let the startup seed-and-apply happen and the loop cycle past its own
        // 5-second TICK, so the deadline below is genuinely the due-loop's freshly
        // scheduled ~60s-away deadline, not the startup seed itself.
        thread::sleep(Duration::from_secs(6));

        let state_path_check = dir.path().join("state.toml");
        let deadline_before = State::load_from(&state_path_check).unwrap().monitor("uuid-a").unwrap().next_change_at_unix;

        let same_toml = "[[monitors]]\nuuid = \"uuid-a\"\nfolder = \"REPLACED\"\ninterval_value = 1\ninterval_unit = \"minutes\"\npaused = false\n"
            .replace("REPLACED", &dir.path().display().to_string());
        // Rewrite the *identical* config.toml a few times over ~9s, mimicking the
        // hot-plug poll's own unconditional, no-op re-save on its 30-second cadence -
        // kept well under 30s in total so this test's own run doesn't cross that real
        // poll boundary itself and call the live, unmocked `list_connected_monitors()`
        // (which corrupted an earlier test the same way before that poll was deferred
        // at startup - see the multi-monitor plan's Task 6 history - a risk that
        // reappears any time a test in this file runs past 30s).
        for _ in 0..3 {
            std::fs::write(config_dir.join("config.toml"), &same_toml).unwrap();
            thread::sleep(Duration::from_secs(3));
        }

        let deadline_after = State::load_from(&state_path_check).unwrap().monitor("uuid-a").unwrap().next_change_at_unix;
        // Unfixed, each no-op resave above would have pushed the deadline another 60s
        // out (three resaves = +180s or more); fixed, an interval that didn't actually
        // change leaves the deadline alone, so it's unchanged (or only a couple of
        // seconds off from re-saving at very nearly - but not exactly - the original
        // recorded second).
        assert!(
            (deadline_after - deadline_before).abs() <= 2,
            "a no-op config re-save moved the deadline from {deadline_before} to {deadline_after} ({}s) - it should have been left untouched",
            deadline_after - deadline_before
        );

        drop(handle);
    }
}
