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

KDE Plasma (via KScreen/KWin) assigns every physical monitor a stable UUID, independent of which port it's plugged into or the order multiple monitors were connected in. Confirmed on this project's development machine:

This design combines two clean JSON sources rather than parsing decorated CLI text (an initial version of this design shelled out to `kscreen-doctor -o`'s human-readable output, but that output embeds ANSI color escape codes unconditionally — confirmed during research that they survive even when the command's stdout is piped to a non-terminal, e.g. `kscreen-doctor -o | cat -v` still shows raw `^[[01;32m` sequences — making it fragile to parse reliably):

1. **`kscreen-doctor --json`** (note: `--json` *without* `-o` — combining both flags appends the legacy colored text after the JSON block, which defeats the purpose) returns clean, structured JSON with one object per output, each including `connected: bool`, `name: string` (the connector name, e.g. `"LVDS-1"`), and `priority: number` (`1` for the primary display) — but *no* persistent UUID.
2. **`~/.config/kwinoutputconfig.json`**, a plain JSON file KWin itself already maintains (confirmed present and populated on this project's development machine), is a JSON array whose entry with `"name": "outputs"` has a `"data"` array of per-monitor objects, each with `connectorName` and the same persistent `uuid` `kscreen-doctor -o`'s text output shows (verified identical UUID string across both sources on this machine: `e01e245f-8f3a-496f-bb9f-d6a02c263502` for connector `LVDS-1`). This file is read directly (`std::fs::read_to_string` + `serde_json`), not queried via any command.

Cross-referencing both by connector name (`kscreen-doctor --json`'s `name` field ≡ `kwinoutputconfig.json`'s `connectorName` field) gives every currently-connected monitor's stable UUID with zero text-parsing fragility — both sources are well-formed JSON, parsed with `serde_json` (new dependency; this project already uses `serde`/`toml` for its own files, so this is a natural extension, not a new parsing paradigm). `kscreen-doctor` itself is still a real subprocess call (this remains the one place the project shells out to a system CLI instead of using D-Bus/`zbus` directly — `org.kde.KScreen`'s D-Bus service exists but its object tree isn't straightforwardly introspectable, confirmed via `busctl --user tree org.kde.KScreen` returning nothing usable during design research), but its output is now consumed as data, not scraped as decorated text.

A new `wallpaper_core::monitors` module exposes:

```rust
pub struct Monitor {
    pub uuid: String,
    pub connector: String,   // e.g. "LVDS-1" - informational only, not used as an identifier
    pub is_primary: bool,    // true for the output with priority == 1
    pub x: i32,               // physical position, from kscreen-doctor's `pos.x`/`pos.y` -
    pub y: i32,               // used only to correlate with a Plasma `desktops()` entry (below)
}

pub fn list_connected_monitors() -> anyhow::Result<Vec<Monitor>>;
```

`is_primary` is derived from `kscreen-doctor --json`'s `priority` field, not tracked separately anywhere in this project's own config — "which monitor is primary" always reflects whatever KDE's own display settings say, so it stays correct automatically if the user changes their primary display in System Settings. A connected output with no matching entry in `kwinoutputconfig.json` (a monitor KWin has genuinely never configured before — rare, but possible on a brand-new connection before KWin has written its own config) is skipped from the returned list rather than erroring, since there is no stable UUID to assign it yet; it will appear on its next poll once KWin has persisted its own config for it (in practice this resolves within the same session, well before this project's 30-second poll cadence would notice a difference).

**Mapping a UUID to a Plasma `desktops()` script target** (needed by the KDE backend to address the right screen) was flagged as an open implementation-time question in an earlier version of this design. It's now resolved by research into how others have solved the identical problem: the Plasma scripting API's `Desktop` objects have no hardware/connector identifier at all (confirmed — KDE's own scripting API provides no stable per-screen ID), but `screenGeometry(d.screen)` exposes each desktop's physical `.left`/`.top` position, and `kscreen-doctor --json`'s `pos.x`/`pos.y` (now carried on `Monitor`, above) gives the same physical position from the Rust side. Sorting both lists by position (top, then left) and matching by index correlates a `Monitor` to its `desktops()` entry reliably, without guessing at index ordering or reverse-engineering anything undocumented — this is the same strategy an existing published solution to per-monitor KDE wallpapers uses. See the "KDE Backend" subsection under "Daemon Changes" below for the concrete script shape.

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

Monitor connect/disconnect is detected by re-running `list_connected_monitors()` on a fixed poll cadence (proposed: every 30 seconds, decoupled from any individual monitor's rotation interval) rather than waiting for a KScreen D-Bus signal — consistent with the "don't reverse-engineer a private D-Bus schema" decision above. A newly-connected UUID triggers the new-monitor-copies-primary behavior; a disconnected one is simply excluded from the next rotation cycle without touching its saved config.

A `ChangeNowRequested` event (the watcher noticing `change_now_request` changed) makes the daemon read that file's content — now a monitor UUID, per the GUI Changes section above — and force an immediate rotation for that one monitor only, leaving every other monitor's own deadline untouched.

`wallpaper_core::backend::WallpaperBackend`'s trait method changes from `set_wallpaper(&self, path: &Path)` to `set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path)`: `target` identifies which monitor this call is for, `all_monitors` (the full currently-connected list) is what lets the backend compute *where* `target` ranks among them positionally.

`core/src/kde_backend.rs`'s `KdePlasmaBackend::set_wallpaper` then: sorts `all_monitors` by `(y, x)` (top-to-bottom, then left-to-right — matching typical multi-monitor reading order) to find `target`'s rank, and generates a script that sorts `desktops()` the same way and writes only to the desktop at that same rank:

```js
var sorted = desktops().filter(function(d) { return d.screen != -1; }).sort(function(a, b) {
    var ga = screenGeometry(a.screen), gb = screenGeometry(b.screen);
    if (ga.top !== gb.top) return ga.top - gb.top;
    return ga.left - gb.left;
});
var d = sorted[{rank}];
if (d) {
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
    d.writeConfig("Image", "file://{path}");
}
```

`{rank}` is computed in Rust (position of `target` within `all_monitors` sorted the same way) and `{path}` goes through the same JS-string escaping this project's `kde_backend.rs` already has (`escape_js_string`, unchanged). The `if (d)` guard makes a rank that's momentarily out of bounds (e.g. a monitor disconnected between this project's own connected-monitor poll and the script actually running) a silent no-op rather than a script error. Each monitor is set independently, one `evaluateScript` call per rotation, matching the per-monitor independent rotation timing described under "Daemon Changes" — this does mean every call re-sorts and re-queries `desktops()`/`screenGeometry()` fresh (cheap, a handful of JS objects) rather than batching all monitors into one script call.

## GUI Changes

`gui/ui/app-window.slint`'s single form becomes one tab per *connected* monitor (Slint's `TabWidget`, from `std-widgets.slint`, is the natural fit — same family of widget this project already imports `Button`/`ComboBox`/`SpinBox`/`LineEdit` from). Each tab is the existing form (folder picker, interval, preview image, countdown, pause/change-now/save) unchanged in its own layout — only wrapped in a per-monitor context instead of operating on one global `Config`. `gui/src/main.rs` reads all monitors' config/state, populates one tab per connected monitor, and each tab's callbacks (`choose-folder`, `toggle-pause`, `save`) operate on that tab's own `MonitorConfig` entry within the shared `Config` list rather than the whole file.

**`change-now` needs to target one monitor, not all of them** — each tab has its own button, and clicking it should only force an immediate rotation on *that* tab's monitor, consistent with every other per-tab control. The shared `change_now_request` file's *content* becomes the target monitor's UUID (previously just a timestamp, whose value the daemon never actually read — only the file's modification was the signal). `wallpaper_core::config::change_now_request_path()` itself is unchanged; only what gets written to it changes. The daemon's handling (see "Daemon Changes" below) reads that UUID after the file-changed signal fires and applies it to that one monitor only.

## Error Handling

- `list_connected_monitors()` failing (e.g. `kscreen-doctor` not installed, or its output format changes in a future KDE release and fails to parse) is treated the same way a missing/malformed `config.toml` already is: logged, and the daemon/GUI fall back to a single-monitor-equivalent mode using whatever monitor data was last successfully read (or `Config::default()` on a fully fresh install) rather than crashing or refusing to start.
- A monitor's own D-Bus `set_wallpaper` call failing (e.g. a transient Plasma shell issue) is scoped to that one monitor — it must not prevent other monitors from rotating on the same cycle.

## Testing

- `wallpaper_core::monitors`'s parsing/cross-referencing logic is pure data-in/struct-out and gets standard unit tests against captured sample JSON (both a `kscreen-doctor --json` sample and a `kwinoutputconfig.json` sample, including single-monitor and multi-monitor cases, and a case where a connected output has no matching `kwinoutputconfig.json` entry), following this project's established TDD conventions.
- `Config`'s migration-from-old-format logic gets unit tests: loading an old-format file produces the correct single-entry new-format `Config`, and the migrated form round-trips correctly when saved and reloaded.
- `Engine`'s per-monitor queue/deadline logic is unit-testable the same way the current single-queue `Engine` already is (fake backend, temp directories) — extended to assert independence between monitors (rotating one doesn't affect another's queue state).
- The KDE backend's position-based rank computation (sorting `all_monitors`, finding `target`'s index) is pure Rust logic and gets standard unit tests, matching this project's existing precedent for `build_wallpaper_script` (`core/src/kde_backend.rs`'s existing tests already verify script string content without a live Plasma session). Only the actual `desktops().sort(...)` JS logic executing correctly against a real multi-monitor KWin session (i.e., that KWin's `screenGeometry()` ordering genuinely agrees with `kscreen-doctor`'s `pos` ordering in practice) needs manual verification — this project's design-time research only had a single-monitor machine available.
- Hot-plug behavior (new-monitor-copies-primary, disconnected-monitor-preserved) is manually verified on real hardware by physically connecting/disconnecting a monitor, for the same reason.

## Out of Scope

- Monitors sharing a folder/rotation (rejected in favor of always-independent, per the approved design decision).
- Any GNOME/XFCE-specific monitor handling (Fases 2-3 of the roadmap, separate work).
- A KScreen D-Bus signal-based (push, not poll) hot-plug detection mechanism — the 30-second poll is simpler and sufficient; revisit only if it proves too slow or too heavy in practice.
