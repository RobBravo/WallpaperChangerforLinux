# GNOME Support (Fase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add GNOME as a second supported desktop environment, selected automatically at runtime, alongside the existing KDE Plasma support.

**Architecture:** GNOME has no native per-monitor wallpaper support (`gsettings`'s `org.gnome.desktop.background` applies one image across the entire virtual desktop, not one per screen), so under GNOME the whole app behaves like a single shared configuration — exactly how this project behaved before Fase 1 added per-monitor support on KDE. This is implemented by having GNOME's monitor-listing function always return exactly one synthetic `Monitor`, so every per-monitor mechanism from Fase 1 (`Config`, `State`, `Engine`, the GUI's selector) works completely unchanged. `daemon/src/main.rs` and `gui/src/main.rs` each detect the current desktop environment once at startup (`$XDG_CURRENT_DESKTOP`) and pick the matching backend/monitor-listing function; `Engine`'s generic bound is satisfied at runtime via `Box<dyn WallpaperBackend>` and a new blanket impl.

**Tech Stack:** Same as the rest of the project — no new dependencies. `GnomeBackend` invokes the `gsettings` binary via `std::process::Command` (no shell, so no injection surface, unlike KDE's D-Bus script).

## Global Constraints

- No live GNOME environment is available to verify this feature against — every task's tests are unit-level only. This is documented in `ROADMAP.md` once the plan completes, the same way Fase 1 documented its two-monitor-dependent gaps.
- `$XDG_CURRENT_DESKTOP` detection must handle colon-separated compound values (e.g. `"ubuntu:GNOME"`, `"budgie:GNOME"`), not just a bare `"GNOME"`/`"KDE"` — check whether any segment matches, not the whole string.
- An unrecognized or unset `$XDG_CURRENT_DESKTOP` must never be silently treated as KDE (today's implicit behavior) — the daemon logs a clear error and exits non-zero; the GUI degrades to an empty monitor list (same as an `list_connected_monitors()` failure already does today), no crash.
- `GnomeBackend` sets both `picture-uri` and `picture-uri-dark` to the same image on every call — never track or detect the user's light/dark theme preference.
- `GnomeBackend` never touches `picture-options` (GNOME's scaling-mode key) — this app rotates which image is shown, not how GNOME displays it.
- Under GNOME, the single synthetic `Monitor`'s UUID is a fixed constant (`GNOME_SHARED_MONITOR_UUID`), never derived from hardware, and always has `is_primary: true` — every downstream per-monitor mechanism (`tray.rs`'s "act on the primary monitor" stopgap included) depends on this being stable and always-primary.
- `list_connected_monitors()` (the existing KDE-specific function) keeps its current name — it is not renamed for symmetry with the new `list_gnome_monitors()`, since it's already called from several already-reviewed places.

---

### Task 1: `core` — desktop-environment detection

**Files:**
- Create: `core/src/desktop.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Produces: `wallpaper_core::desktop::DesktopEnvironment` (enum: `Kde`, `Gnome`), `wallpaper_core::desktop::detect_desktop_environment() -> Option<DesktopEnvironment>`.

- [ ] **Step 1: Write the failing tests**

Create `core/src/desktop.rs`:

```rust
/// The desktop environment this app is currently running under, detected once at
/// daemon/GUI startup via `detect_desktop_environment()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Kde,
    Gnome,
}

/// Reads `$XDG_CURRENT_DESKTOP` and picks a supported desktop environment, if any.
///
/// The value can be a colon-separated list (e.g. `"ubuntu:GNOME"`, `"budgie:GNOME"`)
/// rather than a bare `"GNOME"`/`"KDE"` - some distributions prepend their own name -
/// so this checks whether any segment matches, not the whole string. `None` means
/// "not KDE, not GNOME" - callers must not silently default to one or the other.
pub fn detect_desktop_environment() -> Option<DesktopEnvironment> {
    let value = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    detect_from_value(&value)
}

fn detect_from_value(value: &str) -> Option<DesktopEnvironment> {
    if value.split(':').any(|part| part.eq_ignore_ascii_case("KDE")) {
        Some(DesktopEnvironment::Kde)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")) {
        Some(DesktopEnvironment::Gnome)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kde_from_a_bare_value() {
        assert_eq!(detect_from_value("KDE"), Some(DesktopEnvironment::Kde));
    }

    #[test]
    fn detects_gnome_from_a_bare_value() {
        assert_eq!(detect_from_value("GNOME"), Some(DesktopEnvironment::Gnome));
    }

    #[test]
    fn detects_gnome_from_a_distro_prefixed_value() {
        assert_eq!(detect_from_value("ubuntu:GNOME"), Some(DesktopEnvironment::Gnome));
        assert_eq!(detect_from_value("budgie:GNOME"), Some(DesktopEnvironment::Gnome));
    }

    #[test]
    fn detects_kde_even_when_not_the_first_segment() {
        assert_eq!(detect_from_value("something:KDE"), Some(DesktopEnvironment::Kde));
    }

    #[test]
    fn returns_none_for_an_unrecognized_desktop() {
        assert_eq!(detect_from_value("XFCE"), None);
    }

    #[test]
    fn returns_none_for_an_empty_value() {
        assert_eq!(detect_from_value(""), None);
    }
}
```

Add `pub mod desktop;` to `core/src/lib.rs`, alongside the other `pub mod` lines (order doesn't matter, but keep it alphabetically near `config`/`fs_util` for consistency with the existing list).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wallpaper-core desktop::`
Expected: FAIL — `desktop` module doesn't exist yet (this file didn't exist before Step 1, so this is really "verify Step 1's own tests compile and pass" — there's no meaningful separate red state for a from-scratch file; proceed directly to Step 3's build+test).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core desktop::`
Expected: all six tests PASS.

- [ ] **Step 4: Commit**

```bash
git add core/src/desktop.rs core/src/lib.rs
git commit -m "feat(core): detect the current desktop environment from \$XDG_CURRENT_DESKTOP"
```

---

### Task 2: `core` — GNOME's shared synthetic monitor

**Files:**
- Modify: `core/src/monitors.rs`

**Interfaces:**
- Consumes: `Monitor` (already defined in this file).
- Produces: `wallpaper_core::monitors::GNOME_SHARED_MONITOR_UUID: &str`, `wallpaper_core::monitors::list_gnome_monitors() -> anyhow::Result<Vec<Monitor>>`.

Note: this returns `anyhow::Result<Vec<Monitor>>` (not a bare `Vec<Monitor>`, even though it can never actually fail) so it shares the exact same function signature as `list_connected_monitors()` — Task 4 picks between the two as an interchangeable `fn() -> anyhow::Result<Vec<Monitor>>` value at runtime, which only works if both functions have identical signatures.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `core/src/monitors.rs` (after the existing tests, before the closing `}`):

```rust
    #[test]
    fn list_gnome_monitors_always_returns_one_shared_entry() {
        let monitors = list_gnome_monitors().unwrap();
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].uuid, GNOME_SHARED_MONITOR_UUID);
        assert!(monitors[0].is_primary);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wallpaper-core monitors::list_gnome_monitors_always_returns_one_shared_entry`
Expected: FAIL with "cannot find function `list_gnome_monitors`" / "cannot find value `GNOME_SHARED_MONITOR_UUID`".

- [ ] **Step 3: Write the implementation**

Add above `list_connected_monitors` (or anywhere at module scope) in `core/src/monitors.rs`:

```rust
/// The fixed UUID used to represent "the whole desktop" under GNOME, which has no
/// native per-monitor wallpaper support (see `list_gnome_monitors`). Not a real UUID
/// format on purpose - KDE's UUIDs (from `kwinoutputconfig.json`) always look like
/// `xxxxxxxx-xxxx-...`, so this can never collide with one.
pub const GNOME_SHARED_MONITOR_UUID: &str = "gnome-shared-desktop";

/// GNOME has no native way to give each connected monitor its own wallpaper - the
/// `org.gnome.desktop.background` gsettings key applies one image across the entire
/// virtual desktop, spanning every monitor. Rather than reimplement per-monitor image
/// composition (see the design spec's "Multi-monitor behavior under GNOME" section),
/// this always returns exactly one synthetic `Monitor` representing that shared
/// desktop - so every per-monitor mechanism elsewhere in this project (`Config`,
/// `State`, `Engine`, the GUI's selector) degenerates to "exactly one entry" under
/// GNOME, with zero changes needed to any of it.
///
/// Returns a `Result` (rather than a bare `Vec`) purely so this has the exact same
/// signature as `list_connected_monitors` - callers pick between the two as an
/// interchangeable function value at runtime - even though this specific
/// implementation can never actually fail.
pub fn list_gnome_monitors() -> anyhow::Result<Vec<Monitor>> {
    Ok(vec![Monitor {
        uuid: GNOME_SHARED_MONITOR_UUID.to_string(),
        connector: "GNOME".to_string(),
        is_primary: true,
        x: 0,
        y: 0,
    }])
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p wallpaper-core monitors::`
Expected: all tests PASS, including the new one.

- [ ] **Step 5: Commit**

```bash
git add core/src/monitors.rs
git commit -m "feat(core): GNOME's single shared synthetic monitor"
```

---

### Task 3: `core` — `GnomeBackend` and `Box<dyn WallpaperBackend>` support

**Files:**
- Create: `core/src/gnome_backend.rs`
- Modify: `core/src/backend.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Consumes: `WallpaperBackend` trait, `Monitor` (both already defined).
- Produces: `wallpaper_core::gnome_backend::GnomeBackend` (unit struct implementing `WallpaperBackend`), `impl WallpaperBackend for Box<dyn WallpaperBackend>`.

- [ ] **Step 1: Write the failing tests**

Create `core/src/gnome_backend.rs`:

```rust
use std::path::Path;
use crate::backend::WallpaperBackend;
use crate::monitors::Monitor;

pub struct GnomeBackend;

/// Builds the argument list for one `gsettings set org.gnome.desktop.background
/// <key> file://<path>` invocation, without running anything - kept pure and
/// separate from the actual `Command` so it's directly testable, matching this
/// project's existing split in `kde_backend.rs` between building a script/command and
/// running it.
///
/// Unlike `kde_backend.rs`'s D-Bus script (which embeds the path inside a JavaScript
/// string literal, and therefore needs `escape_js_string`), these arguments are
/// passed straight to `Command::arg` - never through a shell - so there is no
/// injection surface to escape here at all; a path containing quotes, spaces, or any
/// other special character reaches `gsettings` as exactly one argument, verbatim.
fn gsettings_args(key: &str, path: &Path) -> Vec<String> {
    vec![
        "set".to_string(),
        "org.gnome.desktop.background".to_string(),
        key.to_string(),
        format!("file://{}", path.display()),
    ]
}

impl WallpaperBackend for GnomeBackend {
    /// `all_monitors`/`target` are unused: GNOME has exactly one shared wallpaper
    /// setting (see `wallpaper_core::monitors::list_gnome_monitors`'s doc comment),
    /// so every call sets the same two global gsettings keys regardless of which
    /// (synthetic) monitor this was called for.
    fn set_wallpaper(&self, _all_monitors: &[Monitor], _target: &Monitor, path: &Path) -> anyhow::Result<()> {
        // Both the light and dark variants are set to the same image, so the correct
        // wallpaper shows regardless of which GTK theme variant is currently active -
        // this app has no reason to track the user's light/dark preference itself.
        for key in ["picture-uri", "picture-uri-dark"] {
            let status = std::process::Command::new("gsettings")
                .args(gsettings_args(key, path))
                .status()?;
            anyhow::ensure!(status.success(), "gsettings set {key} exited with {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn gsettings_args_builds_the_expected_command_line() {
        let args = gsettings_args("picture-uri", &PathBuf::from("/home/user/a.png"));
        assert_eq!(
            args,
            vec!["set", "org.gnome.desktop.background", "picture-uri", "file:///home/user/a.png"]
        );
    }

    #[test]
    fn gsettings_args_builds_the_dark_variant_with_the_same_path() {
        let args = gsettings_args("picture-uri-dark", &PathBuf::from("/home/user/a.png"));
        assert_eq!(
            args,
            vec!["set", "org.gnome.desktop.background", "picture-uri-dark", "file:///home/user/a.png"]
        );
    }

    #[test]
    fn a_path_with_a_space_reaches_gsettings_as_one_argument() {
        let args = gsettings_args("picture-uri", &PathBuf::from("/home/user/my pictures/a.png"));
        assert_eq!(args[3], "file:///home/user/my pictures/a.png");
    }
}
```

Replace the full contents of `core/src/backend.rs`:

```rust
use std::path::Path;
use crate::monitors::Monitor;

pub trait WallpaperBackend: Send {
    /// Applies `path` as `target`'s wallpaper. `all_monitors` (the full set of
    /// currently-connected monitors, including `target`) is needed by implementations
    /// that have to figure out *where* `target` is on screen relative to the others
    /// (see `KdePlasmaBackend`, which has no other way to identify a specific
    /// monitor).
    fn set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()>;
}

/// Lets a boxed trait object satisfy the same bound as a concrete backend, so
/// `daemon/src/main.rs` can pick a backend at runtime (KDE vs. GNOME) and still
/// construct one `Engine<Box<dyn WallpaperBackend>>` regardless of which concrete
/// type was chosen - `Engine<B: WallpaperBackend>`'s own code doesn't change at all.
impl WallpaperBackend for Box<dyn WallpaperBackend> {
    fn set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        (**self).set_wallpaper(all_monitors, target, path)
    }
}
```

Add `pub mod gnome_backend;` to `core/src/lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wallpaper-core gnome_backend::`
Expected: FAIL — module doesn't exist yet until Step 1's file is in place (same "from-scratch file" situation as Task 1 — there's no separate red state to observe beyond confirming the module doesn't yet exist before this step. Proceed to Step 3's build+test after adding the code above).

- [ ] **Step 3: Run tests and verify the whole crate builds**

Run: `cargo build -p wallpaper-core && cargo test -p wallpaper-core`
Expected: clean build (confirms the `Box<dyn WallpaperBackend>` blanket impl compiles - this is the riskiest part of this task, a real Rust trait-object interaction, not just data), all tests pass including the three new `gnome_backend::` ones.

- [ ] **Step 4: Commit**

```bash
git add core/src/gnome_backend.rs core/src/backend.rs core/src/lib.rs
git commit -m "feat(core): GnomeBackend + Box<dyn WallpaperBackend> support"
```

---

### Task 4: `daemon` — desktop-environment-aware backend selection

**Files:**
- Modify: `daemon/src/main.rs`

**Interfaces:**
- Consumes: `wallpaper_core::desktop::{DesktopEnvironment, detect_desktop_environment}` (Task 1), `wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, Monitor}` (Task 2), `wallpaper_core::gnome_backend::GnomeBackend`, `wallpaper_core::backend::WallpaperBackend`'s `Box<dyn WallpaperBackend>` impl (Task 3), `wallpaper_core::kde_backend::KdePlasmaBackend` (unchanged).
- Produces: the daemon's final `fn main()` for this plan; a new private `select_backend(env: DesktopEnvironment) -> (Box<dyn WallpaperBackend>, fn() -> anyhow::Result<Vec<Monitor>>)`.

This task also fixes a real bug this design would otherwise introduce: `run()`'s 30-second hot-plug poll currently calls the KDE-specific `list_connected_monitors()` unconditionally - under GNOME this would shell out to a nonexistent `kscreen-doctor` every 30 seconds, failing silently every time (logged, not crash-worthy, but wrong and noisy). `run()` gains a `list_monitors` parameter so the poll uses whichever function was actually selected for the current desktop.

- [ ] **Step 1: Replace the full contents of `daemon/src/main.rs`**

```rust
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
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, Monitor};
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
    tray::spawn_tray();

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
        assert_eq!(list_monitors as usize, list_connected_monitors as usize);
    }

    #[test]
    fn select_backend_picks_gnome_monitor_listing_for_gnome() {
        let (_backend, list_monitors) = select_backend(DesktopEnvironment::Gnome);
        assert_eq!(list_monitors as usize, list_gnome_monitors as usize);
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
```

- [ ] **Step 2: Run the two new unit tests**

Run: `cargo test -p wallpaper-changer-daemon tests::select_backend`
Expected: both `select_backend_picks_kde_monitor_listing_for_kde` and `select_backend_picks_gnome_monitor_listing_for_gnome` PASS.

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p wallpaper-changer-daemon`
Expected: compiles cleanly, no warnings.

- [ ] **Step 4: Run the full daemon test suite**

Run: `cargo test -p wallpaper-changer-daemon`
Expected: all tests pass, including `engine::` and `watcher::` (unchanged) and the three `tests::` integration tests (unchanged behavior, just an added sixth argument to their `run(...)` calls).

- [ ] **Step 5: Commit**

```bash
git add daemon/src/main.rs
git commit -m "feat(daemon): select KDE or GNOME backend by \$XDG_CURRENT_DESKTOP at startup"
```

---

### Task 5: `gui` — desktop-environment-aware monitor listing

**Files:**
- Modify: `gui/src/main.rs`

**Interfaces:**
- Consumes: `wallpaper_core::desktop::{DesktopEnvironment, detect_desktop_environment}` (Task 1), `wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, Monitor}` (Task 2).
- Produces: the GUI's final `fn main()` for this plan; a new private `monitor_source(env: Option<DesktopEnvironment>) -> (fn() -> anyhow::Result<Vec<Monitor>>, bool)`.

- [ ] **Step 1: Replace the full contents of `gui/src/main.rs`**

```rust
slint::include_modules!();

mod singleton;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wallpaper_core::config::{change_now_request_path, gui_lock_path, gui_socket_path, Config, IntervalUnit};
use wallpaper_core::desktop::{detect_desktop_environment, DesktopEnvironment};
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, Monitor};
use wallpaper_core::state::State;

fn unit_to_index(unit: IntervalUnit) -> i32 {
    match unit {
        IntervalUnit::Minutes => 0,
        IntervalUnit::Hours => 1,
        IntervalUnit::Days => 2,
    }
}

fn index_to_unit(index: i32) -> IntervalUnit {
    match index {
        1 => IntervalUnit::Hours,
        2 => IntervalUnit::Days,
        _ => IntervalUnit::Minutes,
    }
}

fn monitor_label(monitor: &Monitor, position: usize) -> String {
    format!("Monitor {} ({})", position + 1, monitor.connector)
}

/// Picks which monitor-listing function to use, and whether the dropdown should show
/// GNOME's single shared-desktop label instead of per-monitor labels. Pulled out as
/// its own function so this decision is unit-testable without a live desktop
/// session - unlike the KDE case, GNOME (and an unrecognized desktop, which falls
/// back to the same behavior `list_connected_monitors()` failing already has today -
/// an empty dropdown, no crash) can't be exercised by an automated GUI test at all.
fn monitor_source(env: Option<DesktopEnvironment>) -> (fn() -> anyhow::Result<Vec<Monitor>>, bool) {
    match env {
        Some(DesktopEnvironment::Gnome) => (list_gnome_monitors, true),
        _ => (list_connected_monitors, false),
    }
}

/// Populates the form fields for `uuid` from `config`. Falls back to
/// `Config::for_new_monitor`'s defaults if this monitor has no config entry yet (it
/// just connected and the daemon hasn't caught up during its own 30-second poll -
/// this self-corrects on the next reload once it has).
fn populate_form(ui: &AppWindow, uuid: &str, config: &Config, primary_uuid: Option<&str>) {
    let monitor_config = config
        .monitor(uuid)
        .cloned()
        .unwrap_or_else(|| config.for_new_monitor(uuid, primary_uuid));
    ui.set_folder_path(monitor_config.folder.display().to_string().into());
    ui.set_interval_value(monitor_config.interval_value as i32);
    ui.set_interval_unit_index(unit_to_index(monitor_config.interval_unit));
    ui.set_paused(monitor_config.paused);
}

/// Refreshes the currently-selected monitor's preview image and countdown from
/// state.toml. `shown_wallpaper` remembers `(uuid, path)` so switching monitors, or
/// that monitor's wallpaper actually changing, both correctly trigger a fresh image
/// decode - but repeated ticks with nothing new don't.
fn refresh_state(ui: &AppWindow, uuid: &str, shown_wallpaper: &RefCell<Option<(String, PathBuf)>>) {
    // The daemon's tray menu, or another monitor's tab, can pause/resume this
    // monitor behind our back, so re-read the flag rather than trusting stale state.
    if let Ok(config) = Config::load() {
        if let Some(monitor_config) = config.monitor(uuid) {
            if ui.get_paused() != monitor_config.paused {
                ui.set_paused(monitor_config.paused);
            }
        }
    }

    let Ok(state) = State::load() else { return };
    let Some(monitor_state) = state.monitor(uuid) else { return };

    let already_shown = shown_wallpaper
        .borrow()
        .as_ref()
        .is_some_and(|(shown_uuid, shown_path)| shown_uuid == uuid && shown_path == &monitor_state.current_wallpaper);
    if !already_shown {
        if let Ok(image) = slint::Image::load_from_path(&monitor_state.current_wallpaper) {
            ui.set_preview_image(image);
            *shown_wallpaper.borrow_mut() = Some((uuid.to_string(), monitor_state.current_wallpaper.clone()));
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let remaining = (monitor_state.next_change_at_unix - now).max(0);
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;
    ui.set_countdown_text(format!("Próximo cambio en {hours:02}:{minutes:02}:{seconds:02}").into());
}

/// Re-enumerates connected monitors and updates the dropdown if the set changed.
/// The GUI is a tray-resident, long-lived singleton (unlike the daemon, it isn't
/// restarted when a monitor is hot-plugged), so without this the dropdown would
/// permanently reflect only whatever was connected at the moment the GUI first
/// launched - plugging in a second monitor would never make it selectable, even by
/// closing and reopening the window (only killing the whole process would help).
///
/// Preserves the current selection across a refresh when the selected monitor is
/// still connected (its index may have moved after a resort); falls back to the
/// first available monitor if it was unplugged, or to no selection if none remain.
///
/// `list_monitors`/`is_gnome` come from `monitor_source` - under GNOME this always
/// returns the same one-entry list, so after the first successful call this becomes
/// a permanent no-op (the `if *uuids.borrow() == new_uuids` check below), which is
/// correct: there is nothing to re-detect under GNOME's single shared desktop model.
fn refresh_monitor_list(
    ui: &AppWindow,
    list_monitors: fn() -> anyhow::Result<Vec<Monitor>>,
    is_gnome: bool,
    uuids: &RefCell<Vec<String>>,
    primary_uuid: &RefCell<Option<String>>,
    current_uuid: &RefCell<Option<String>>,
    shown_wallpaper: &RefCell<Option<(String, PathBuf)>>,
) {
    // A transient failure (e.g. `kscreen-doctor` briefly unavailable during a display
    // reconfiguration) must not be treated as "every monitor just disconnected" - that
    // would wipe an already-populated dropdown and, on the next successful poll,
    // overwrite any unsaved form edits when the selection snaps back. Leaving the
    // existing list untouched is a no-op the first time this runs (it starts empty).
    let Ok(mut monitors) = list_monitors() else { return };
    monitors.sort_by(|a, b| a.connector.cmp(&b.connector));
    let new_uuids: Vec<String> = monitors.iter().map(|m| m.uuid.clone()).collect();

    if *uuids.borrow() == new_uuids {
        return; // nothing connected/disconnected since the last check
    }

    let new_primary = monitors.iter().find(|m| m.is_primary).map(|m| m.uuid.clone());
    let labels: Vec<slint::SharedString> = if is_gnome {
        vec!["Todos los monitores".into()]
    } else {
        monitors.iter().enumerate().map(|(i, m)| monitor_label(m, i).into()).collect()
    };
    ui.set_monitor_labels(Rc::new(slint::VecModel::from(labels)).into());

    let still_connected = current_uuid.borrow().as_ref().is_some_and(|uuid| new_uuids.contains(uuid));
    let new_current = if still_connected { current_uuid.borrow().clone() } else { new_uuids.first().cloned() };
    let new_index = new_current.as_ref().and_then(|uuid| new_uuids.iter().position(|u| u == uuid)).unwrap_or(0);

    *uuids.borrow_mut() = new_uuids;
    *primary_uuid.borrow_mut() = new_primary;
    ui.set_selected_monitor_index(new_index as i32);

    if new_current != *current_uuid.borrow() {
        *shown_wallpaper.borrow_mut() = None; // force a fresh decode for the newly-selected monitor
    }
    *current_uuid.borrow_mut() = new_current.clone();

    if let Some(uuid) = new_current {
        if let Ok(config) = Config::load() {
            populate_form(ui, &uuid, &config, primary_uuid.borrow().as_deref());
        }
        refresh_state(ui, &uuid, shown_wallpaper);
    }
}

/// Shows the window if it's hidden, hides it if it's visible. Shared by the tray
/// menu's "Mostrar/Ocultar ventana" and the window's own close button.
fn toggle_visibility(ui: &AppWindow) {
    let window = ui.window();
    if window.is_visible() && !window.is_minimized() {
        let _ = window.hide();
    } else {
        show_and_restore(window);
    }
}

fn show_and_restore(window: &slint::Window) {
    let _ = window.show();
    window.set_minimized(false);
}

fn main() -> anyhow::Result<()> {
    let socket_path = gui_socket_path();
    let lock_path = gui_lock_path();
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let (listener, _lock_file) = match singleton::claim(&socket_path, &lock_path) {
        Ok(singleton::Singleton::AlreadyRunning) => {
            if let Err(e) = singleton::notify_running_instance(&socket_path) {
                eprintln!("gui: failed to notify the running instance: {e}");
            }
            return Ok(());
        }
        Ok(singleton::Singleton::Primary(listener, lock_file)) => (Some(listener), Some(lock_file)),
        Err(e) => {
            eprintln!("gui: single-instance detection unavailable, continuing anyway: {e}");
            (None, None)
        }
    };

    let ui = AppWindow::new()?;
    let tray = GuiTray::new()?;

    let (list_monitors, is_gnome) = monitor_source(detect_desktop_environment());

    let uuids: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let primary_uuid: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let current_uuid: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let shown_wallpaper: Rc<RefCell<Option<(String, PathBuf)>>> = Rc::new(RefCell::new(None));

    refresh_monitor_list(&ui, list_monitors, is_gnome, &uuids, &primary_uuid, &current_uuid, &shown_wallpaper);

    ui.on_monitor_selected({
        let ui_handle = ui.as_weak();
        let uuids = uuids.clone();
        let current_uuid = current_uuid.clone();
        let shown_wallpaper = shown_wallpaper.clone();
        let primary_uuid = primary_uuid.clone();
        move || {
            let Some(ui) = ui_handle.upgrade() else { return };
            let index = ui.get_selected_monitor_index();
            let Some(uuid) = uuids.borrow().get(index as usize).cloned() else { return };
            *current_uuid.borrow_mut() = Some(uuid.clone());
            *shown_wallpaper.borrow_mut() = None; // force a fresh decode for the newly-selected monitor
            if let Ok(config) = Config::load() {
                populate_form(&ui, &uuid, &config, primary_uuid.borrow().as_deref());
            }
            refresh_state(&ui, &uuid, &shown_wallpaper);
        }
    });

    ui.on_choose_folder({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_folder_path(folder.display().to_string().into());
                }
            }
        }
    });

    ui.on_toggle_pause({
        let ui_handle = ui.as_weak();
        let current_uuid = current_uuid.clone();
        move || {
            let Some(uuid) = current_uuid.borrow().clone() else { return };
            let Ok(mut config) = Config::load() else { return };
            let Some(monitor) = config.monitors.iter_mut().find(|m| m.uuid == uuid) else { return };
            monitor.paused = !monitor.paused;
            let new_paused = monitor.paused;
            if config.save().is_ok() {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_paused(new_paused);
                }
            }
        }
    });

    ui.on_change_now({
        let current_uuid = current_uuid.clone();
        move || {
            let Some(uuid) = current_uuid.borrow().clone() else { return };
            let _ = std::fs::write(change_now_request_path(), uuid);
        }
    });

    ui.on_save({
        let ui_handle = ui.as_weak();
        let current_uuid = current_uuid.clone();
        let primary_uuid = primary_uuid.clone();
        move || {
            let Some(ui) = ui_handle.upgrade() else { return };
            let Some(uuid) = current_uuid.borrow().clone() else { return };
            let Ok(mut config) = Config::load() else { return };
            let folder = PathBuf::from(ui.get_folder_path().to_string());
            let interval_value = ui.get_interval_value() as u64;
            let interval_unit = index_to_unit(ui.get_interval_unit_index());
            match config.monitors.iter_mut().find(|m| m.uuid == uuid) {
                Some(existing) => {
                    existing.folder = folder;
                    existing.interval_value = interval_value;
                    existing.interval_unit = interval_unit;
                    // `paused` is intentionally left untouched - owned by the pause
                    // toggle, not the save button (same rule as before this plan).
                }
                None => {
                    let mut fresh = config.for_new_monitor(&uuid, primary_uuid.borrow().as_deref());
                    fresh.folder = folder;
                    fresh.interval_value = interval_value;
                    fresh.interval_unit = interval_unit;
                    config.monitors.push(fresh);
                }
            }
            let _ = config.save();
        }
    });

    ui.window().on_close_requested({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                toggle_visibility(&ui);
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    tray.on_toggle_visibility({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                toggle_visibility(&ui);
            }
        }
    });

    tray.on_quit(move || {
        let _ = slint::quit_event_loop();
    });

    if let Some(listener) = listener {
        let ui_handle = ui.as_weak();
        singleton::spawn_accept_loop(listener, move || {
            let ui_handle = ui_handle.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_handle.upgrade() {
                    show_and_restore(ui.window());
                }
            });
        });
    }

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let shown_wallpaper = shown_wallpaper.clone();
        let current_uuid = current_uuid.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    if !ui.window().is_visible() {
                        return;
                    }
                    if let Some(uuid) = current_uuid.borrow().clone() {
                        refresh_state(&ui, &uuid, &shown_wallpaper);
                    }
                }
            },
        );
    }

    // Separate from the 1-second state timer above: re-enumerating monitors spawns a
    // `kscreen-doctor` subprocess (or is a no-op under GNOME), so this runs on its own,
    // coarser cadence rather than every tick. The GUI is a tray-resident singleton
    // (see `refresh_monitor_list`'s doc comment) so this is the only thing that ever
    // notices a hot-plugged monitor while it's already running.
    let monitor_poll_timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let uuids = uuids.clone();
        let primary_uuid = primary_uuid.clone();
        let current_uuid = current_uuid.clone();
        let shown_wallpaper = shown_wallpaper.clone();
        monitor_poll_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(5),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    if !ui.window().is_visible() {
                        return;
                    }
                    refresh_monitor_list(&ui, list_monitors, is_gnome, &uuids, &primary_uuid, &current_uuid, &shown_wallpaper);
                }
            },
        );
    }

    ui.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_source_picks_the_gnome_shared_label_and_listing_for_gnome() {
        let (list_monitors, is_gnome) = monitor_source(Some(DesktopEnvironment::Gnome));
        assert_eq!(list_monitors as usize, list_gnome_monitors as usize);
        assert!(is_gnome);
    }

    #[test]
    fn monitor_source_picks_the_kde_listing_for_kde() {
        let (list_monitors, is_gnome) = monitor_source(Some(DesktopEnvironment::Kde));
        assert_eq!(list_monitors as usize, list_connected_monitors as usize);
        assert!(!is_gnome);
    }

    #[test]
    fn monitor_source_falls_back_to_kde_listing_for_an_unrecognized_desktop() {
        // Matches list_connected_monitors()'s own existing failure behavior (empty
        // dropdown, no crash) rather than refusing to show anything at all - a user on
        // an unsupported desktop might still want to inspect the GUI's settings.
        let (list_monitors, is_gnome) = monitor_source(None);
        assert_eq!(list_monitors as usize, list_connected_monitors as usize);
        assert!(!is_gnome);
    }
}
```

- [ ] **Step 2: Run the new unit tests**

Run: `cargo test -p wallpaper-changer-gui tests::monitor_source`
Expected: all three PASS.

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles cleanly, no warnings.

- [ ] **Step 4: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test from Tasks 1-5 passes, plus everything from Fase 1 (unchanged).

- [ ] **Step 5: Commit**

```bash
git add gui/src/main.rs
git commit -m "feat(gui): wire desktop-environment-aware monitor listing and the GNOME shared-desktop label"
```

---
