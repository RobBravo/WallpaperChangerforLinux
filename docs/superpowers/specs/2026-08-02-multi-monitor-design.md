# Multi-Monitor Support — Design

**Goal:** Each connected monitor gets its own independent wallpaper folder, rotation interval, and pause state — instead of every monitor showing the same image, as today.

**Context:** Today, `core/src/kde_backend.rs::build_wallpaper_script` loops over Plasma's `desktops()` and writes the *same* image to every one. `config.toml`/`state.toml` are flat, single-monitor structures. This spec replaces that with a per-monitor model across all three crates, identified by a stable per-monitor UUID that KDE Plasma already tracks internally (via KScreen), so configuration survives reboots, port changes, and monitors being reconnected in a different order.

## Global Constraints (inherited from the base project)

- KDE Plasma only, still — this spec does not touch GNOME/XFCE support (Fases 2-3 of `ROADMAP.md`), and doesn't need to: it only changes how many desktops the KDE backend addresses, not the backend abstraction itself.
- All shared runtime files stay under `~/.config/wallpaper-changer/`, resolved via `wallpaper_core::config`/`wallpaper_core::state` helper functions.
- Supported image extensions, top-level-only folder scanning, and the shuffle-and-consume rotation algorithm (`wallpaper_core::queue`) are unchanged — they just now run once per monitor instead of once globally.
- No async runtime in the daemon (plain OS threads + `std::sync::mpsc`), matching the base project's existing constraint.
- Add third-party dependencies with `cargo add`.

## Monitor Identification

KDE Plasma (via KScreen) assigns every physical monitor a stable UUID, independent of which port it's plugged into or the order multiple monitors were connected in. Confirmed on this project's development machine via `kscreen-doctor -o`:

```
Output: 1 LVDS-1 e01e245f-8f3a-496f-bb9f-d6a02c263502
	enabled
	connected
	priority 1
	...
```

`org.kde.KScreen`'s D-Bus service exists but its object tree isn't straightforwardly introspectable (checked during design research — `busctl --user tree org.kde.KScreen` returns nothing usable), so this design uses `kscreen-doctor -o`'s plain-text output directly rather than reverse-engineering a private D-Bus schema. `kscreen-doctor` ships as part of `libkscreen`/`plasma-workspace`, present on every real KDE Plasma install this project already targets — this is the one place the project shells out to a system CLI instead of using D-Bus/`zbus`, a deliberate, scoped exception noted here so it isn't mistaken for an oversight later.

A new `wallpaper_core::monitors` module parses this output into:

```rust
pub struct Monitor {
    pub uuid: String,
    pub connector: String,   // e.g. "LVDS-1" - informational only, not used as an identifier
    pub is_primary: bool,    // true for the output with "priority 1"
}

pub fn list_connected_monitors() -> anyhow::Result<Vec<Monitor>>;
```

`is_primary` is derived from the `priority 1` line, not tracked separately anywhere in this project's own config — "which monitor is primary" always reflects whatever KDE's own display settings say, so it stays correct automatically if the user changes their primary display in System Settings.

**Mapping a UUID to a Plasma `desktops()` script target** (needed by the KDE backend to address the right screen) is an implementation-time detail: `desktops()[i].screen` gives a KWin screen index, and `kscreen-doctor -o`'s numeric `Output: <id> ...` prefix is a candidate correlation key, but the exact correspondence needs to be verified empirically against a real multi-monitor KDE session during implementation (this project's only test hardware during design research is a single-monitor laptop). This is flagged explicitly rather than guessed at.

## Config and State Schema

`core/src/config.rs`'s `Config` changes from a flat struct to a list of per-monitor entries:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub uuid: String,
    pub folder: PathBuf,
    pub interval_value: u64,
    pub interval_unit: IntervalUnit,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Config {
    pub monitors: Vec<MonitorConfig>,
}
```

Serialized as TOML array-of-tables:

```toml
[[monitors]]
uuid = "e01e245f-8f3a-496f-bb9f-d6a02c263502"
folder = "/home/user/Wallpapers"
interval_value = 30
interval_unit = "minutes"
paused = false

[[monitors]]
uuid = "..."
folder = "..."
interval_value = 60
interval_unit = "minutes"
paused = false
```

`core/src/state.rs`'s `State` gains the same per-monitor shape (`Vec<MonitorState>`, each with `uuid`, `current_wallpaper`, `next_change_at_unix`), for the same reason `Config` does: one entry per monitor, keyed by UUID.

**A monitor never disappears from `config.toml`/`state.toml` just because it's disconnected.** Its entry stays exactly as last configured; only *connected* monitors (per `list_connected_monitors()`) are actively rotated by the daemon or shown as a tab in the GUI. Reconnecting a previously-seen UUID (same port or a different one) picks its saved entry back up automatically.

### Migration from the old single-monitor format

`Config::load()` first attempts to parse `config.toml` in the new `[[monitors]]` shape. If that fails, it falls back to parsing the *old* flat shape (`folder`/`interval_value`/`interval_unit`/`paused` at the top level — exactly today's format) and converts it into a single-entry `Config` whose one `MonitorConfig` uses the UUID of whichever monitor is currently connected and marked primary (via `list_connected_monitors()`), then immediately saves the migrated form back to disk. This is a one-time, automatic, silent upgrade — an existing user's folder and interval survive untouched, just now scoped to their primary monitor's UUID. If `list_connected_monitors()` itself fails during migration (e.g. `kscreen-doctor` isn't found), the migration is skipped and `Config::load()` falls through to today's error-handling path (log and use defaults) rather than crashing.

## New-Monitor and Disconnected-Monitor Behavior

- **A UUID never seen before** (not present in `config.toml`'s `monitors` list) gets a new `MonitorConfig` entry copying the *primary* monitor's `folder`/`interval_value`/`interval_unit`/`paused` (per the approved design decision), with its own independent rotation queue from the moment it's created. If there is no primary (a KScreen quirk, or the very first monitor ever seen) it falls back to `Config::default()`'s single-monitor values (`dirs::picture_dir()`, 30 minutes, unpaused) exactly as today.
- **A previously-configured UUID that's no longer connected** is simply skipped by the daemon's rotation loop and hidden from the GUI's tab list — its `MonitorConfig`/`MonitorState` entries are left untouched in the files.

## Daemon Changes

`daemon/src/engine.rs`'s `Engine` changes from owning one `WallpaperQueue` to owning one per connected monitor (`HashMap<String, WallpaperQueue>`, keyed by UUID), each independently tracking its own folder scan, shuffle state, and interval/pause (read from that monitor's own `MonitorConfig`). `apply_next`-equivalent logic runs per monitor: each connected monitor has its own deadline, and the main loop's `recv_timeout` waits for the *soonest* upcoming deadline across all monitors rather than a single global one.

Monitor connect/disconnect is detected by re-running `list_connected_monitors()` on a fixed poll cadence (proposed: every 30 seconds, decoupled from any individual monitor's rotation interval) rather than waiting for a KScreen D-Bus signal — consistent with the "use the plain-text CLI, don't reverse-engineer a private D-Bus schema" decision above. A newly-connected UUID triggers the new-monitor-copies-primary behavior; a disconnected one is simply excluded from the next rotation cycle without touching its saved config.

`core/src/kde_backend.rs`'s `KdePlasmaBackend::set_wallpaper` changes from a single-argument `(path)` call that loops over every `desktops()` entry, to something that also identifies *which* desktop to target for a given monitor UUID (exact mechanism per the "Monitor Identification" section's open implementation detail above).

## GUI Changes

`gui/ui/app-window.slint`'s single form becomes one tab per *connected* monitor (Slint's `TabWidget`, from `std-widgets.slint`, is the natural fit — same family of widget this project already imports `Button`/`ComboBox`/`SpinBox`/`LineEdit` from). Each tab is the existing form (folder picker, interval, preview image, countdown, pause/change-now/save) unchanged in its own layout — only wrapped in a per-monitor context instead of operating on one global `Config`. `gui/src/main.rs` reads all monitors' config/state, populates one tab per connected monitor, and each tab's callbacks (`choose-folder`, `toggle-pause`, `change-now`, `save`) operate on that tab's own `MonitorConfig` entry within the shared `Config` list rather than the whole file.

## Error Handling

- `list_connected_monitors()` failing (e.g. `kscreen-doctor` not installed, or its output format changes in a future KDE release and fails to parse) is treated the same way a missing/malformed `config.toml` already is: logged, and the daemon/GUI fall back to a single-monitor-equivalent mode using whatever monitor data was last successfully read (or `Config::default()` on a fully fresh install) rather than crashing or refusing to start.
- A monitor's own D-Bus `set_wallpaper` call failing (e.g. a transient Plasma shell issue) is scoped to that one monitor — it must not prevent other monitors from rotating on the same cycle.

## Testing

- `wallpaper_core::monitors`'s output-parsing logic is pure text-in/struct-out and gets standard unit tests against captured sample `kscreen-doctor -o` output (including a single-monitor sample and a multi-monitor sample), following this project's established TDD conventions.
- `Config`'s migration-from-old-format logic gets unit tests: loading an old-format file produces the correct single-entry new-format `Config`, and the migrated form round-trips correctly when saved and reloaded.
- `Engine`'s per-monitor queue/deadline logic is unit-testable the same way the current single-queue `Engine` already is (fake backend, temp directories) — extended to assert independence between monitors (rotating one doesn't affect another's queue state).
- The KDE backend's actual per-desktop targeting (the open implementation-time question above) can only be verified against a real multi-monitor KDE Plasma session — this project's design-time research only had a single-monitor machine available, so this is manual verification, matching the project's existing precedent for D-Bus-integration code that can't be meaningfully mocked.
- Hot-plug behavior (new-monitor-copies-primary, disconnected-monitor-preserved) is manually verified on real hardware by physically connecting/disconnecting a monitor, for the same reason.

## Out of Scope

- Monitors sharing a folder/rotation (rejected in favor of always-independent, per the approved design decision).
- Any GNOME/XFCE-specific monitor handling (Fases 2-3 of the roadmap, separate work).
- A KScreen D-Bus signal-based (push, not poll) hot-plug detection mechanism — the 30-second poll is simpler and sufficient; revisit only if it proves too slow or too heavy in practice.
