# XFCE Support (Fase 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add XFCE as a third supported desktop environment, with genuine per-monitor rotation (unlike GNOME's forced single-shared model), selected automatically at runtime alongside the existing KDE and GNOME support.

**Architecture:** `wallpaper_core::desktop::DesktopEnvironment` gains an `Xfce` variant. A new `list_xfce_monitors()` discovers connected monitors by parsing `xfconf-query -c xfce4-desktop -l`'s live property listing (no fixed paths assumed - XFCE's monitor identifier scheme varies by version and by hardware). A new `XfceBackend` writes the rotated image to every `workspaceN/last-image` property that currently exists for the target monitor, as a plain filesystem path (not a `file://` URI - confirmed by research, differs from both `KdePlasmaBackend` and `GnomeBackend`). `daemon/src/main.rs`, `gui/src/main.rs`, and `core/src/config.rs`'s legacy-migration path each gain one new match arm for `Xfce`, reusing every other per-monitor mechanism from Fase 1 unchanged. `daemon/src/tray.rs` needs no changes at all.

**Tech Stack:** No new dependencies - `xfconf-query` invoked via `std::process::Command` (no shell), same pattern as `kscreen-doctor`/`gsettings` elsewhere in this project. Monitor-identifier parsing uses plain string/slice operations, not a `regex` crate dependency, matching this project's existing minimalism.

## Global Constraints

- No live XFCE environment is available to verify this feature against - every task's tests are unit-level only. Documented in `ROADMAP.md` once this plan completes, matching Fases 1-2's precedent.
- XFCE's `last-image` xfconf property value is a **plain absolute filesystem path** - never prefix it with `file://` (confirmed via XFCE's own documented examples; this is the one detail in this plan most likely to be silently gotten wrong by assuming GNOME's URI convention applies here too).
- `list_xfce_monitors()` must return `anyhow::Result<Vec<Monitor>>` (not a bare `Vec<Monitor>`), exactly matching `list_connected_monitors`/`list_gnome_monitors`'s signature - callers pick between all three as an interchangeable `fn() -> anyhow::Result<Vec<Monitor>>` value at runtime.
- `XfceBackend::set_wallpaper` must write to **every** `workspaceN/last-image` property currently defined for the target monitor, not just `workspace0` - so the rotated wallpaper is correct regardless of which XFCE virtual desktop the user is viewing.
- A monitor with zero existing `last-image` properties (XFCE has never written one for it) gets a best-effort fallback write to a constructed `workspace0` path, rather than silently doing nothing.
- The monitor identifier used as both `Monitor.uuid` and `Monitor.connector` for XFCE is whatever string xfconf currently reports in its property paths - no version detection, no cross-referencing against another tool (e.g. `xrandr`), per the approved design decision.
- Do not add a `regex` crate dependency - parse property paths with `str::split`/`strip_prefix`, matching this project's established preference for minimal dependencies.

---

### Task 1: `core` — extend desktop-environment detection with XFCE

**Files:**
- Modify: `core/src/desktop.rs`

**Interfaces:**
- Produces: `DesktopEnvironment::Xfce` (new enum variant, alongside the existing `Kde`/`Gnome`).

Note: the existing test `returns_none_for_an_unrecognized_desktop` currently asserts `detect_from_value("XFCE")` is `None` - this task changes that fixture value to `"MATE"` (still genuinely unrecognized after this task) rather than leaving a now-false assertion in place.

- [ ] **Step 1: Replace the full contents of `core/src/desktop.rs`**

```rust
/// The desktop environment this app is currently running under, detected once at
/// daemon/GUI startup via `detect_desktop_environment()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Kde,
    Gnome,
    Xfce,
}

/// Reads `$XDG_CURRENT_DESKTOP` and picks a supported desktop environment, if any.
///
/// The value can be a colon-separated list (e.g. `"ubuntu:GNOME"`, `"budgie:GNOME"`)
/// rather than a bare `"GNOME"`/`"KDE"`/`"XFCE"` - some distributions prepend their
/// own name - so this checks whether any segment matches, not the whole string.
/// `None` means "not KDE, GNOME, or XFCE" - callers must not silently default to any
/// of them.
pub fn detect_desktop_environment() -> Option<DesktopEnvironment> {
    let value = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    detect_from_value(&value)
}

fn detect_from_value(value: &str) -> Option<DesktopEnvironment> {
    if value.split(':').any(|part| part.eq_ignore_ascii_case("KDE")) {
        Some(DesktopEnvironment::Kde)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")) {
        Some(DesktopEnvironment::Gnome)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("XFCE")) {
        Some(DesktopEnvironment::Xfce)
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
    fn detects_xfce_from_a_bare_value() {
        assert_eq!(detect_from_value("XFCE"), Some(DesktopEnvironment::Xfce));
    }

    #[test]
    fn detects_xfce_from_a_distro_prefixed_value() {
        assert_eq!(detect_from_value("X-Generic:XFCE"), Some(DesktopEnvironment::Xfce));
    }

    #[test]
    fn returns_none_for_an_unrecognized_desktop() {
        assert_eq!(detect_from_value("MATE"), None);
    }

    #[test]
    fn returns_none_for_an_empty_value() {
        assert_eq!(detect_from_value(""), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core desktop::`
Expected: all eight tests PASS.

- [ ] **Step 3: Commit**

```bash
git add core/src/desktop.rs
git commit -m "feat(core): detect XFCE alongside KDE and GNOME"
```

---

### Task 2: `core` — XFCE monitor enumeration

**Files:**
- Modify: `core/src/monitors.rs`

**Interfaces:**
- Consumes: `Monitor` (already defined in this file).
- Produces: `wallpaper_core::monitors::list_xfce_monitors() -> anyhow::Result<Vec<Monitor>>`.

- [ ] **Step 1: Write the failing tests**

Add to `core/src/monitors.rs`, after the existing `list_gnome_monitors` function (before `list_connected_monitors`):

```rust
/// Extracts the monitor identifier from one xfconf property path, if it's a
/// `last-image` property shaped like `/backdrop/screen{N}/monitor{id}/workspace{N}/
/// last-image`. Anything else (a different property name like `color-style` or
/// `image-path`, or an unexpected segment count) is `None` rather than a guess -
/// `xfconf-query -c xfce4-desktop -l` lists every property in the channel, most of
/// which aren't about which image is shown and must be ignored, not misparsed.
fn xfce_monitor_id_from_property_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let [backdrop, screen, monitor, workspace, property] = segments.as_slice() else {
        return None;
    };
    if *backdrop != "backdrop" || *property != "last-image" {
        return None;
    }
    if !screen.starts_with("screen") || !workspace.starts_with("workspace") {
        return None;
    }
    let monitor_id = monitor.strip_prefix("monitor")?;
    if monitor_id.is_empty() {
        return None;
    }
    Some(monitor_id.to_string())
}

/// Parses `xfconf-query -c xfce4-desktop -l`'s full output (one property path per
/// line) into a sorted, deduplicated list of monitor identifiers - sorted because
/// XFCE's xfconf schema has no "primary monitor" concept to read (unlike KDE's
/// `priority`), so this project picks whichever identifier sorts first alphabetically
/// as a deterministic stand-in, used only for Fase 1's "a new monitor copies the
/// primary's settings" behavior.
fn parse_xfce_monitor_listing(listing: &str) -> Vec<String> {
    let mut monitor_ids: Vec<String> = listing
        .lines()
        .filter_map(xfce_monitor_id_from_property_path)
        .collect();
    monitor_ids.sort();
    monitor_ids.dedup();
    monitor_ids
}

/// Lists every monitor XFCE's own `xfconf` currently has a `last-image` property for.
///
/// Unlike KDE (`kscreen-doctor` + `kwinoutputconfig.json`, an independent source of
/// truth for which monitors are physically connected) or GNOME (no per-monitor
/// concept at all), XFCE is inferred purely from xfconf's own already-populated
/// properties - a monitor XFCE's own `xfdesktop` process has never written a
/// `last-image` property for (e.g. freshly connected, before the user has opened
/// XFCE's own Appearance settings) will not appear here. This is a known,
/// intentionally-accepted limitation for this phase (no independent monitor-listing
/// tool like `xrandr` is cross-checked), documented for live-hardware verification
/// once available.
pub fn list_xfce_monitors() -> anyhow::Result<Vec<Monitor>> {
    let output = std::process::Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-l"])
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "xfconf-query -l exited with {}",
        output.status
    );
    let listing = String::from_utf8(output.stdout)?;

    Ok(parse_xfce_monitor_listing(&listing)
        .into_iter()
        .enumerate()
        .map(|(i, id)| Monitor {
            uuid: id.clone(),
            connector: id,
            is_primary: i == 0,
            x: 0,
            y: 0,
        })
        .collect())
}
```

Add to the existing `#[cfg(test)] mod tests` block at the bottom of the file:

```rust
    #[test]
    fn xfce_monitor_id_from_property_path_extracts_the_monitor_segment() {
        assert_eq!(
            xfce_monitor_id_from_property_path("/backdrop/screen0/monitorDP-1/workspace0/last-image"),
            Some("DP-1".to_string())
        );
    }

    #[test]
    fn xfce_monitor_id_from_property_path_handles_numeric_monitor_ids() {
        assert_eq!(
            xfce_monitor_id_from_property_path("/backdrop/screen0/monitor0/workspace1/last-image"),
            Some("0".to_string())
        );
    }

    #[test]
    fn xfce_monitor_id_from_property_path_ignores_unrelated_properties() {
        assert_eq!(
            xfce_monitor_id_from_property_path("/backdrop/screen0/monitor0/workspace0/color-style"),
            None
        );
        assert_eq!(xfce_monitor_id_from_property_path("/backdrop/single-workspace-mode"), None);
        assert_eq!(xfce_monitor_id_from_property_path("/backdrop/screen0/monitor0/image-path"), None);
    }

    #[test]
    fn parse_xfce_monitor_listing_extracts_unique_monitors_sorted() {
        let listing = "\
/backdrop/screen0/monitor0/workspace0/last-image
/backdrop/screen0/monitor0/workspace0/color-style
/backdrop/screen0/monitor0/workspace1/last-image
/backdrop/screen0/monitorDP-1/workspace0/last-image
/backdrop/single-workspace-mode
";
        assert_eq!(parse_xfce_monitor_listing(listing), vec!["0".to_string(), "DP-1".to_string()]);
    }

    #[test]
    fn parse_xfce_monitor_listing_returns_empty_for_a_channel_with_no_monitors_configured() {
        assert_eq!(parse_xfce_monitor_listing("/backdrop/single-workspace-mode\n"), Vec::<String>::new());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wallpaper-core monitors::xfce`
Expected: FAIL with "cannot find function `xfce_monitor_id_from_property_path`" (or similar - the functions don't exist yet before Step 1's code is added).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core monitors::`
Expected: all tests PASS, including the five new ones, and every pre-existing KDE/GNOME test in this file untouched.

- [ ] **Step 4: Commit**

```bash
git add core/src/monitors.rs
git commit -m "feat(core): XFCE monitor enumeration via xfconf-query"
```

---

### Task 3: `core` — `XfceBackend`

**Files:**
- Create: `core/src/xfce_backend.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Consumes: `WallpaperBackend` trait, `Monitor` (both already defined; unchanged by this task).
- Produces: `wallpaper_core::xfce_backend::XfceBackend` (unit struct implementing `WallpaperBackend`).

- [ ] **Step 1: Write the failing tests**

Create `core/src/xfce_backend.rs`:

```rust
use std::path::Path;
use crate::backend::WallpaperBackend;
use crate::monitors::Monitor;

pub struct XfceBackend;

/// Lists every `workspaceN/last-image` xfconf property currently defined for
/// `monitor_id`, given the same `-l` listing text `list_xfce_monitors` parses - kept
/// pure (parses text, spawns nothing) and separate from the actual `xfconf-query`
/// calls in `set_wallpaper`, matching this project's established split in
/// `kde_backend.rs`/`gnome_backend.rs` between building a command/script and running
/// it.
fn last_image_properties_for_monitor(listing: &str, monitor_id: &str) -> Vec<String> {
    let expected_monitor_segment = format!("monitor{monitor_id}");
    listing
        .lines()
        .filter(|line| {
            let segments: Vec<&str> = line.split('/').filter(|s| !s.is_empty()).collect();
            matches!(
                segments.as_slice(),
                [backdrop, screen, monitor, workspace, property]
                    if *backdrop == "backdrop"
                        && screen.starts_with("screen")
                        && *monitor == expected_monitor_segment
                        && workspace.starts_with("workspace")
                        && *property == "last-image"
            )
        })
        .map(|line| line.to_string())
        .collect()
}

/// The property to write when `monitor_id` has no existing `last-image` property at
/// all (see `set_wallpaper`'s fallback below).
fn fallback_property_for_monitor(monitor_id: &str) -> String {
    format!("/backdrop/screen0/monitor{monitor_id}/workspace0/last-image")
}

impl WallpaperBackend for XfceBackend {
    /// `all_monitors` is unused: XFCE's xfconf property paths are already
    /// monitor-specific by construction, unlike KDE's position-based correlation
    /// (there's no equivalent of Plasma's index-only `desktops()` to correlate
    /// against here).
    fn set_wallpaper(&self, _all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        let output = std::process::Command::new("xfconf-query")
            .args(["-c", "xfce4-desktop", "-l"])
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "xfconf-query -l exited with {}",
            output.status
        );
        let listing = String::from_utf8(output.stdout)?;

        let mut properties = last_image_properties_for_monitor(&listing, &target.uuid);
        if properties.is_empty() {
            // XFCE has never written a last-image property for this monitor - write a
            // best-effort default rather than silently doing nothing.
            properties.push(fallback_property_for_monitor(&target.uuid));
        }

        for property in properties {
            // last-image's value is a plain absolute filesystem path, not a file://
            // URI (unlike GnomeBackend's gsettings keys) - confirmed against XFCE's
            // own documented examples. `.arg(path)` passes it as a single OS-string
            // argument, never through a shell, so no escaping is needed for quotes,
            // spaces, or any other character a filename might contain.
            let status = std::process::Command::new("xfconf-query")
                .args(["-c", "xfce4-desktop", "-p", &property, "-s"])
                .arg(path)
                .status()?;
            anyhow::ensure!(status.success(), "xfconf-query set {property} exited with {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_image_properties_for_monitor_finds_every_workspace_entry_for_that_monitor_only() {
        let listing = "\
/backdrop/screen0/monitor0/workspace0/last-image
/backdrop/screen0/monitor0/workspace1/last-image
/backdrop/screen0/monitor0/workspace0/color-style
/backdrop/screen0/monitorDP-1/workspace0/last-image
";
        let props = last_image_properties_for_monitor(listing, "0");
        assert_eq!(
            props,
            vec![
                "/backdrop/screen0/monitor0/workspace0/last-image".to_string(),
                "/backdrop/screen0/monitor0/workspace1/last-image".to_string(),
            ]
        );
    }

    #[test]
    fn last_image_properties_for_monitor_returns_empty_for_an_unknown_monitor() {
        let listing = "/backdrop/screen0/monitor0/workspace0/last-image\n";
        assert!(last_image_properties_for_monitor(listing, "DP-1").is_empty());
    }

    #[test]
    fn fallback_property_for_monitor_builds_a_workspace0_path() {
        assert_eq!(
            fallback_property_for_monitor("DP-1"),
            "/backdrop/screen0/monitorDP-1/workspace0/last-image"
        );
    }
}
```

Add `pub mod xfce_backend;` to `core/src/lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wallpaper-core xfce_backend::`
Expected: FAIL - the module doesn't exist yet before Step 1's file is created (same "from-scratch file" situation as Fase 2's `desktop.rs`/`gnome_backend.rs` - proceed to Step 3 after adding the code above).

- [ ] **Step 3: Run tests and verify the whole crate builds**

Run: `cargo build -p wallpaper-core && cargo test -p wallpaper-core`
Expected: clean build, all tests pass including the three new `xfce_backend::` ones.

- [ ] **Step 4: Commit**

```bash
git add core/src/xfce_backend.rs core/src/lib.rs
git commit -m "feat(core): XfceBackend"
```

---

### Task 4: `core` — extend legacy-config migration with XFCE

**Files:**
- Modify: `core/src/config.rs`

**Interfaces:**
- Consumes: `DesktopEnvironment::Xfce` (Task 1), `list_xfce_monitors` (Task 2).
- Produces: `Config::migration_list_monitors` gains an `Xfce` arm (signature unchanged).

This closes, proactively, the same class of gap Fase 2's final whole-branch review found after the fact: a legacy single-monitor `config.toml` migrating under an unrecognized-by-migration desktop silently loses the user's real settings. `migration_list_monitors` already exists (added during Fase 2's fix) specifically to prevent this - this task just adds XFCE's case instead of letting it fall through to the KDE default.

- [ ] **Step 1: Write the failing test**

Add to `core/src/config.rs`'s existing `#[cfg(test)] mod tests` block, near the existing `migration_list_monitors_*` tests:

```rust
    #[test]
    fn migration_list_monitors_resolves_xfce_listing_for_xfce() {
        let xfce = Config::migration_list_monitors(Some(crate::desktop::DesktopEnvironment::Xfce));
        assert_eq!(
            xfce as *const () as usize,
            crate::monitors::list_xfce_monitors as *const () as usize
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wallpaper-core config::migration_list_monitors_resolves_xfce_listing_for_xfce`
Expected: FAIL - `migration_list_monitors`'s current match has no `Xfce` arm, so it falls through to the `_` (KDE) arm, and the assertion comparing against `list_xfce_monitors`'s address fails.

- [ ] **Step 3: Add the XFCE arm**

In `core/src/config.rs`, change:

```rust
    fn migration_list_monitors(
        env: Option<crate::desktop::DesktopEnvironment>,
    ) -> fn() -> anyhow::Result<Vec<crate::monitors::Monitor>> {
        match env {
            Some(crate::desktop::DesktopEnvironment::Gnome) => crate::monitors::list_gnome_monitors,
            _ => crate::monitors::list_connected_monitors,
        }
    }
```

to:

```rust
    fn migration_list_monitors(
        env: Option<crate::desktop::DesktopEnvironment>,
    ) -> fn() -> anyhow::Result<Vec<crate::monitors::Monitor>> {
        match env {
            Some(crate::desktop::DesktopEnvironment::Gnome) => crate::monitors::list_gnome_monitors,
            Some(crate::desktop::DesktopEnvironment::Xfce) => crate::monitors::list_xfce_monitors,
            _ => crate::monitors::list_connected_monitors,
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p wallpaper-core config::`
Expected: all tests PASS, including the new one and every pre-existing test in this file.

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): legacy-config migration resolves XFCE's monitor listing"
```

---

### Task 5: `daemon` — select XFCE backend

**Files:**
- Modify: `daemon/src/main.rs`

**Interfaces:**
- Consumes: `DesktopEnvironment::Xfce` (Task 1), `wallpaper_core::monitors::list_xfce_monitors` (Task 2), `wallpaper_core::xfce_backend::XfceBackend` (Task 3).
- Produces: `select_backend` gains an `Xfce` arm (signature unchanged).

`daemon/src/tray.rs` needs **no changes** for this task - it already receives whichever `list_monitors` `select_backend` chose, threaded through as a parameter since Fase 2's final-review fix.

- [ ] **Step 1: Write the failing test**

Add to `daemon/src/main.rs`'s existing `#[cfg(test)] mod tests` block, near the existing `select_backend_picks_*` tests:

```rust
    #[test]
    fn select_backend_picks_xfce_monitor_listing_for_xfce() {
        let (_backend, list_monitors) = select_backend(DesktopEnvironment::Xfce);
        assert_eq!(list_monitors as *const () as usize, list_xfce_monitors as *const () as usize);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wallpaper-changer-daemon tests::select_backend_picks_xfce_monitor_listing_for_xfce`
Expected: FAIL to compile - `DesktopEnvironment::Xfce` isn't a match arm in `select_backend` yet, and `list_xfce_monitors`/`XfceBackend` aren't imported into `daemon/src/main.rs` yet.

- [ ] **Step 3: Add the imports and the XFCE arm**

In `daemon/src/main.rs`, change the imports:

```rust
use wallpaper_core::gnome_backend::GnomeBackend;
use wallpaper_core::kde_backend::KdePlasmaBackend;
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, Monitor};
```

to:

```rust
use wallpaper_core::gnome_backend::GnomeBackend;
use wallpaper_core::kde_backend::KdePlasmaBackend;
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, list_xfce_monitors, Monitor};
use wallpaper_core::xfce_backend::XfceBackend;
```

And change `select_backend`:

```rust
fn select_backend(env: DesktopEnvironment) -> (Box<dyn WallpaperBackend>, fn() -> anyhow::Result<Vec<Monitor>>) {
    match env {
        DesktopEnvironment::Kde => (Box::new(KdePlasmaBackend), list_connected_monitors),
        DesktopEnvironment::Gnome => (Box::new(GnomeBackend), list_gnome_monitors),
    }
}
```

to:

```rust
fn select_backend(env: DesktopEnvironment) -> (Box<dyn WallpaperBackend>, fn() -> anyhow::Result<Vec<Monitor>>) {
    match env {
        DesktopEnvironment::Kde => (Box::new(KdePlasmaBackend), list_connected_monitors),
        DesktopEnvironment::Gnome => (Box::new(GnomeBackend), list_gnome_monitors),
        DesktopEnvironment::Xfce => (Box::new(XfceBackend), list_xfce_monitors),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p wallpaper-changer-daemon tests::select_backend`
Expected: all three `select_backend_picks_*` tests PASS (KDE, GNOME, and the new XFCE one).

- [ ] **Step 5: Run the full daemon test suite**

Run: `cargo test -p wallpaper-changer-daemon`
Expected: all tests pass, including `engine::`/`watcher::`/`tray::` (untouched by this task) and the pre-existing `change_now_request_*`/`shortening_the_interval_*`/`a_no_op_config_resave_*` integration tests (unaffected - they construct `select_backend` inputs directly and never touch XFCE).

- [ ] **Step 6: Commit**

```bash
git add daemon/src/main.rs
git commit -m "feat(daemon): select XfceBackend under XFCE"
```

---

### Task 6: `gui` — select XFCE monitor listing

**Files:**
- Modify: `gui/src/main.rs`

**Interfaces:**
- Consumes: `DesktopEnvironment::Xfce` (Task 1), `wallpaper_core::monitors::list_xfce_monitors` (Task 2).
- Produces: `monitor_source` gains an `Xfce` arm (signature unchanged).

XFCE gets real per-monitor labels, exactly like KDE - `monitor_source`'s existing `bool` (GNOME's "show the single shared-desktop label instead" flag) is already correct as `false` for XFCE without any new parameter.

- [ ] **Step 1: Write the failing test**

Add to `gui/src/main.rs`'s existing `#[cfg(test)] mod tests` block, near the existing `monitor_source_*` tests:

```rust
    #[test]
    fn monitor_source_picks_the_xfce_listing_for_xfce() {
        let (list_monitors, is_gnome) = monitor_source(Some(DesktopEnvironment::Xfce));
        assert_eq!(list_monitors as *const () as usize, list_xfce_monitors as *const () as usize);
        assert!(!is_gnome);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wallpaper-changer-gui tests::monitor_source_picks_the_xfce_listing_for_xfce`
Expected: FAIL to compile - `DesktopEnvironment::Xfce` isn't a match arm in `monitor_source` yet, and `list_xfce_monitors` isn't imported into `gui/src/main.rs` yet.

- [ ] **Step 3: Add the import and the XFCE arm**

In `gui/src/main.rs`, change:

```rust
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, Monitor};
```

to:

```rust
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, list_xfce_monitors, Monitor};
```

And change `monitor_source`:

```rust
fn monitor_source(env: Option<DesktopEnvironment>) -> (fn() -> anyhow::Result<Vec<Monitor>>, bool) {
    match env {
        Some(DesktopEnvironment::Gnome) => (list_gnome_monitors, true),
        _ => (list_connected_monitors, false),
    }
}
```

to:

```rust
fn monitor_source(env: Option<DesktopEnvironment>) -> (fn() -> anyhow::Result<Vec<Monitor>>, bool) {
    match env {
        Some(DesktopEnvironment::Gnome) => (list_gnome_monitors, true),
        Some(DesktopEnvironment::Xfce) => (list_xfce_monitors, false),
        _ => (list_connected_monitors, false),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p wallpaper-changer-gui tests::monitor_source`
Expected: all four `monitor_source_*` tests PASS (GNOME, KDE, unrecognized-desktop fallback, and the new XFCE one).

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test from Tasks 1-6 passes, plus everything from Fases 1-2, unchanged.

- [ ] **Step 6: Commit**

```bash
git add gui/src/main.rs
git commit -m "feat(gui): wire XFCE monitor listing into the selector"
```

---
