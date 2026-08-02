# Multi-Monitor Support (Fase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each connected monitor gets its own independent wallpaper folder, rotation interval, and pause state, identified by KDE's own stable per-monitor UUID — replacing today's "same image on every monitor" behavior.

**Architecture:** `wallpaper-core` gains a `monitors` module (enumerates connected monitors + their stable UUIDs by combining `kscreen-doctor --json` and KWin's own `kwinoutputconfig.json`) and its `Config`/`State` change from flat single-monitor structs to per-monitor lists keyed by UUID, with automatic one-time migration from the old flat format. `daemon`'s `Engine` owns one rotation queue per connected monitor and the main loop tracks one deadline per monitor plus a 30-second hot-plug poll. The KDE backend targets one physical monitor per call by correlating monitor position (from `kscreen-doctor`) with Plasma's `desktops()` (via `screenGeometry()`), since Plasma's scripting API has no direct hardware identifier. The GUI replaces its single form with a monitor-selector dropdown (not tabs — Slint's `TabWidget` doesn't support a runtime-variable tab count) driving the same form, reused.

**Tech Stack:** New dependency: `serde_json` (in `wallpaper-core`, for parsing the two JSON monitor-identification sources). No other new dependencies.

## Global Constraints

- KDE Plasma only, single-DE — this plan does not touch GNOME/XFCE support.
- Every runtime file stays under `~/.config/wallpaper-changer/`, resolved via `wallpaper_core::config`/`wallpaper_core::state` helpers — except `~/.config/kwinoutputconfig.json`, which is deliberately read directly via `dirs::config_dir()` since it's KWin's own file, not this app's.
- Supported image extensions, top-level-only folder scanning, and the shuffle-and-consume rotation algorithm (`wallpaper_core::scanner`/`queue`) are unchanged.
- No async runtime in the daemon — plain OS threads + `std::sync::mpsc`.
- Add third-party dependencies with `cargo add`.
- Every `git commit` step commits only the files listed in that step.
- Third-party/system-integration behavior referenced in this plan (`kscreen-doctor --json`'s exact field names, `kwinoutputconfig.json`'s exact structure, Plasma's `screenGeometry()` scripting function) was confirmed against this project's real development machine and public KDE documentation at planning time, but if it doesn't match exactly during implementation, check the actual installed `kscreen-doctor --json` output / `~/.config/kwinoutputconfig.json` content / `develop.kde.org`'s scripting docs and adapt while keeping the same shape — this is expected integration work, not a sign a task is wrong.
- Only one real monitor was available on this project's development machine during planning — anything requiring genuine 2+ monitor hardware to verify is explicitly called out as manual verification in Task 9, and the plan does not claim to have proven it beyond that.

---

### Task 1: `wallpaper-core` — monitor identification module

**Files:**
- Create: `core/src/monitors.rs`
- Modify: `core/src/lib.rs`
- Modify: `core/Cargo.toml`

**Interfaces:**
- Produces:
  - `wallpaper_core::monitors::Monitor { uuid: String, connector: String, is_primary: bool, x: i32, y: i32 }` (derives `Debug, Clone, PartialEq`).
  - `wallpaper_core::monitors::list_connected_monitors() -> anyhow::Result<Vec<Monitor>>`.

- [ ] **Step 1: Add the `serde_json` dependency**

Run:
```bash
cd core
cargo add serde_json
cd ..
```

- [ ] **Step 2: Write the failing tests**

Create `core/src/monitors.rs` with only this content for now (references types/functions that don't exist yet — this is the expected RED):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_KSCREEN_JSON: &str = r#"{
        "features": 255,
        "outputs": [
            {
                "connected": true,
                "enabled": true,
                "id": 1,
                "name": "LVDS-1",
                "pos": {"x": 0, "y": 0},
                "priority": 1
            }
        ],
        "screen": {"id": 0}
    }"#;

    const SAMPLE_KWIN_CONFIG_JSON: &str = r#"[
        {
            "data": [
                {
                    "connectorName": "LVDS-1",
                    "uuid": "e01e245f-8f3a-496f-bb9f-d6a02c263502",
                    "edidHash": "68fe312b5ef0e0a1bcd88890b73c7b3a"
                }
            ],
            "name": "outputs"
        },
        {
            "data": [
                {"id": 1},
                {"id": 2}
            ],
            "name": "setups"
        }
    ]"#;

    #[test]
    fn parses_a_single_connected_monitor() {
        let uuids = parse_kwin_output_uuids(SAMPLE_KWIN_CONFIG_JSON);
        let outputs = parse_kscreen_outputs(SAMPLE_KSCREEN_JSON).unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(
            uuids.get("LVDS-1").map(String::as_str),
            Some("e01e245f-8f3a-496f-bb9f-d6a02c263502")
        );
    }

    #[test]
    fn parses_two_connected_monitors_with_correct_positions_and_primary_flag() {
        let kscreen_json = r#"{
            "outputs": [
                {"connected": true, "name": "LVDS-1", "priority": 1, "pos": {"x": 0, "y": 0}},
                {"connected": true, "name": "HDMI-A-1", "priority": 2, "pos": {"x": 1280, "y": 0}}
            ]
        }"#;
        let kwin_json = r#"[
            {"name": "outputs", "data": [
                {"connectorName": "LVDS-1", "uuid": "uuid-a"},
                {"connectorName": "HDMI-A-1", "uuid": "uuid-b"}
            ]}
        ]"#;

        let outputs = parse_kscreen_outputs(kscreen_json).unwrap();
        let uuids = parse_kwin_output_uuids(kwin_json);

        assert_eq!(outputs.len(), 2);
        assert_eq!(uuids.len(), 2);
        assert_eq!(uuids.get("HDMI-A-1").map(String::as_str), Some("uuid-b"));
    }

    #[test]
    fn a_disconnected_output_is_marked_as_such() {
        let kscreen_json = r#"{
            "outputs": [
                {"connected": false, "name": "HDMI-A-1", "priority": 2, "pos": {"x": 1280, "y": 0}}
            ]
        }"#;
        let outputs = parse_kscreen_outputs(kscreen_json).unwrap();
        assert!(!outputs[0].connected);
    }

    #[test]
    fn kwin_config_with_no_outputs_entry_yields_an_empty_map() {
        let kwin_json = r#"[
            {"name": "setups", "data": [{"id": 1}]}
        ]"#;
        let uuids = parse_kwin_output_uuids(kwin_json);
        assert!(uuids.is_empty());
    }

    #[test]
    fn malformed_kwin_config_json_returns_an_empty_map_instead_of_panicking() {
        let uuids = parse_kwin_output_uuids("not valid json{{{");
        assert!(uuids.is_empty());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail to compile**

Run: `cargo test -p wallpaper-core monitors::`
Expected: compile error — `parse_kscreen_outputs`/`parse_kwin_output_uuids` are not defined. This is the expected RED.

- [ ] **Step 4: Write the implementation**

At the top of `core/src/monitors.rs`, above the `#[cfg(test)]` module, add:

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use serde::Deserialize;

/// A currently-connected physical monitor, identified by KDE's own stable UUID (see
/// `list_connected_monitors` for how it's obtained - it survives reboots and the
/// monitor being plugged into a different port).
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub uuid: String,
    pub connector: String,
    pub is_primary: bool,
    pub x: i32,
    pub y: i32,
}

#[derive(Deserialize)]
struct KscreenJson {
    outputs: Vec<KscreenOutput>,
}

#[derive(Deserialize)]
struct KscreenOutput {
    connected: bool,
    name: String,
    priority: u32,
    pos: KscreenPos,
}

#[derive(Deserialize)]
struct KscreenPos {
    x: i32,
    y: i32,
}

/// Parses `kscreen-doctor --json`'s output (note: `--json` *without* `-o` - combining
/// both prints legacy ANSI-colored text after the JSON block).
fn parse_kscreen_outputs(json_text: &str) -> anyhow::Result<Vec<KscreenOutput>> {
    let parsed: KscreenJson = serde_json::from_str(json_text)?;
    Ok(parsed.outputs)
}

/// Parses `~/.config/kwinoutputconfig.json` into a connector-name -> UUID map.
///
/// This file mixes multiple unrelated entry shapes under one top-level array (an
/// "outputs" entry with one object per monitor, and a "setups" entry describing
/// multi-monitor arrangements with a completely different shape), so this parses it
/// as loosely-typed JSON and only pulls out what it recognizes from the "outputs"
/// entry, rather than deserializing the whole file into a fixed struct - a "setups"
/// entry contributes nothing rather than being a hard parse error, and a single
/// malformed monitor entry is skipped instead of failing the whole read.
fn parse_kwin_output_uuids(json_text: &str) -> HashMap<String, String> {
    let mut uuids = HashMap::new();
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return uuids;
    };
    let Some(entries) = root.as_array() else { return uuids };
    for entry in entries {
        if entry.get("name").and_then(|v| v.as_str()) != Some("outputs") {
            continue;
        }
        let Some(data) = entry.get("data").and_then(|v| v.as_array()) else { continue };
        for item in data {
            let connector = item.get("connectorName").and_then(|v| v.as_str());
            let uuid = item.get("uuid").and_then(|v| v.as_str());
            if let (Some(connector), Some(uuid)) = (connector, uuid) {
                uuids.insert(connector.to_string(), uuid.to_string());
            }
        }
    }
    uuids
}

fn kwin_output_config_path() -> PathBuf {
    // KWin's own config file - deliberately NOT under this project's
    // `wallpaper_core::config::config_dir()`, since it belongs to KWin, not this app.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kwinoutputconfig.json")
}

/// Lists every currently-connected monitor, each with KDE's own stable UUID.
///
/// Combines two sources: `kscreen-doctor --json` for live connected/priority/position
/// data, and KWin's own `kwinoutputconfig.json` for the persistent per-monitor UUID
/// (cross-referenced by connector name - `kscreen-doctor --json` alone has no UUID). A
/// monitor connected but not yet present in `kwinoutputconfig.json` (KWin hasn't
/// persisted its config for it yet - rare, self-resolving within the same session) is
/// silently omitted rather than erroring.
pub fn list_connected_monitors() -> anyhow::Result<Vec<Monitor>> {
    let output = std::process::Command::new("kscreen-doctor")
        .arg("--json")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "kscreen-doctor exited with {}",
        output.status
    );
    let kscreen_json = String::from_utf8(output.stdout)?;
    let outputs = parse_kscreen_outputs(&kscreen_json)?;

    let kwin_config_text = std::fs::read_to_string(kwin_output_config_path()).unwrap_or_default();
    let uuids = parse_kwin_output_uuids(&kwin_config_text);

    Ok(outputs
        .into_iter()
        .filter(|o| o.connected)
        .filter_map(|o| {
            let uuid = uuids.get(&o.name)?.clone();
            Some(Monitor {
                uuid,
                connector: o.name,
                is_primary: o.priority == 1,
                x: o.pos.x,
                y: o.pos.y,
            })
        })
        .collect())
}
```

Add `pub mod monitors;` to `core/src/lib.rs`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core monitors::`
Expected: all five tests PASS.

- [ ] **Step 6: Run the full `wallpaper-core` test suite**

Run: `cargo test -p wallpaper-core`
Expected: every pre-existing test still passes, plus the five new ones.

- [ ] **Step 7: Commit**

```bash
git add core/Cargo.toml core/src/lib.rs core/src/monitors.rs
git commit -m "feat(core): add monitor identification via kscreen-doctor + kwinoutputconfig.json"
```

---

### Task 2: `wallpaper-core` — per-monitor `Config` with migration

**Files:**
- Modify: `core/src/config.rs`

**Interfaces:**
- Consumes: `wallpaper_core::monitors::list_connected_monitors` (Task 1).
- Produces:
  - `wallpaper_core::config::MonitorConfig { uuid: String, folder: PathBuf, interval_value: u64, interval_unit: IntervalUnit, paused: bool }`.
  - `wallpaper_core::config::Config { monitors: Vec<MonitorConfig> }` (replaces the old flat fields), implementing `Default` (empty `monitors`).
  - `Config::monitor(&self, uuid: &str) -> Option<&MonitorConfig>`.
  - `Config::for_new_monitor(&self, uuid: &str, primary_uuid: Option<&str>) -> MonitorConfig`.
  - `Config::load()` now migrates an old-format `config.toml` automatically (see below).

- [ ] **Step 1: Replace the full contents of `core/src/config.rs`**

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntervalUnit {
    Minutes,
    Hours,
    Days,
}

impl IntervalUnit {
    pub fn to_duration(self, value: u64) -> Duration {
        let secs = match self {
            IntervalUnit::Minutes => value * 60,
            IntervalUnit::Hours => value * 60 * 60,
            IntervalUnit::Days => value * 60 * 60 * 24,
        };
        Duration::from_secs(secs)
    }
}

/// One monitor's own folder, rotation interval, and pause state, keyed by its stable
/// KDE-assigned UUID (see `wallpaper_core::monitors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub uuid: String,
    pub folder: PathBuf,
    pub interval_value: u64,
    pub interval_unit: IntervalUnit,
    pub paused: bool,
}

impl MonitorConfig {
    /// This project's original single-monitor defaults, for a monitor with no other
    /// config to copy from (see `Config::for_new_monitor`).
    fn default_for(uuid: String) -> Self {
        MonitorConfig {
            uuid,
            folder: dirs::picture_dir().unwrap_or_else(|| PathBuf::from(".")),
            interval_value: 30,
            interval_unit: IntervalUnit::Minutes,
            paused: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub monitors: Vec<MonitorConfig>,
}

/// The pre-multi-monitor file shape, parsed only to migrate an existing user's
/// settings the first time they run a version of this app that understands multiple
/// monitors. See `Config::load()`.
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    folder: PathBuf,
    interval_value: u64,
    interval_unit: IntervalUnit,
    paused: bool,
}

impl Config {
    /// Finds this monitor's own config entry, if it has one.
    pub fn monitor(&self, uuid: &str) -> Option<&MonitorConfig> {
        self.monitors.iter().find(|m| m.uuid == uuid)
    }

    /// Builds a config entry for a monitor never seen before: copies the primary
    /// monitor's folder/interval/pause state if there is one and it has a config
    /// entry, otherwise falls back to this project's original single-monitor
    /// defaults.
    pub fn for_new_monitor(&self, uuid: &str, primary_uuid: Option<&str>) -> MonitorConfig {
        let primary = primary_uuid.and_then(|p| self.monitor(p));
        match primary {
            Some(primary) => MonitorConfig {
                uuid: uuid.to_string(),
                folder: primary.folder.clone(),
                interval_value: primary.interval_value,
                interval_unit: primary.interval_unit,
                paused: primary.paused,
            },
            None => MonitorConfig::default_for(uuid.to_string()),
        }
    }

    fn from_legacy(legacy: LegacyConfig, uuid: String) -> Config {
        Config {
            monitors: vec![MonitorConfig {
                uuid,
                folder: legacy.folder,
                interval_value: legacy.interval_value,
                interval_unit: legacy.interval_unit,
                paused: legacy.paused,
            }],
        }
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fs_util::atomic_write(path, &text)
    }

    /// Loads `config.toml`, creating it with an empty monitor list if it doesn't
    /// exist yet, and migrating it in place if it's still in the pre-multi-monitor
    /// single-folder format.
    ///
    /// Migration needs to know which monitor to assign the old settings to (the old
    /// format has no UUID), so it asks
    /// `wallpaper_core::monitors::list_connected_monitors()` for whichever monitor is
    /// currently primary. If that fails (e.g. `kscreen-doctor` isn't installed) or no
    /// monitor is connected, migration is skipped for now and an empty config is
    /// used instead - the old file on disk is left untouched unless migration
    /// actually succeeds, so a later successful detection can retry it.
    pub fn load() -> anyhow::Result<Config> {
        let path = config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.save()?;
            return Ok(cfg);
        }

        let text = std::fs::read_to_string(&path)?;
        let value: toml::Value = toml::from_str(&text)?;
        if value.get("monitors").is_some() {
            return Ok(toml::from_str(&text)?);
        }

        let Ok(legacy) = toml::from_str::<LegacyConfig>(&text) else {
            // Neither shape is recognized - start fresh rather than erroring,
            // matching this project's existing "malformed config must never be
            // fatal" policy.
            return Ok(Config::default());
        };
        let Some(primary_uuid) = crate::monitors::list_connected_monitors()
            .ok()
            .and_then(|monitors| monitors.into_iter().find(|m| m.is_primary).map(|m| m.uuid))
        else {
            return Ok(Config::default());
        };

        let migrated = Config::from_legacy(legacy, primary_uuid);
        migrated.save()?;
        Ok(migrated)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&config_path())
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wallpaper-changer")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn change_now_request_path() -> PathBuf {
    config_dir().join("change_now_request")
}

pub fn gui_socket_path() -> PathBuf {
    config_dir().join("gui.sock")
}

pub fn gui_lock_path() -> PathBuf {
    config_dir().join("gui.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_unit_converts_to_duration() {
        assert_eq!(IntervalUnit::Minutes.to_duration(2), Duration::from_secs(120));
        assert_eq!(IntervalUnit::Hours.to_duration(1), Duration::from_secs(3600));
        assert_eq!(IntervalUnit::Days.to_duration(1), Duration::from_secs(86400));
    }

    fn sample_monitor_config(uuid: &str) -> MonitorConfig {
        MonitorConfig {
            uuid: uuid.to_string(),
            folder: PathBuf::from("/tmp/wallpapers"),
            interval_value: 45,
            interval_unit: IntervalUnit::Hours,
            paused: true,
        }
    }

    #[test]
    fn config_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config { monitors: vec![sample_monitor_config("uuid-a")] };

        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();

        assert_eq!(loaded, cfg);
    }

    #[test]
    fn monitor_finds_the_matching_entry_by_uuid() {
        let cfg = Config { monitors: vec![sample_monitor_config("uuid-a")] };
        assert!(cfg.monitor("uuid-a").is_some());
        assert!(cfg.monitor("uuid-b").is_none());
    }

    #[test]
    fn for_new_monitor_copies_the_primary_monitors_settings() {
        let cfg = Config { monitors: vec![sample_monitor_config("primary")] };

        let fresh = cfg.for_new_monitor("new-uuid", Some("primary"));

        assert_eq!(fresh.uuid, "new-uuid");
        assert_eq!(fresh.folder, PathBuf::from("/tmp/wallpapers"));
        assert_eq!(fresh.interval_value, 45);
        assert_eq!(fresh.interval_unit, IntervalUnit::Hours);
        assert!(fresh.paused);
    }

    #[test]
    fn for_new_monitor_falls_back_to_defaults_when_there_is_no_primary() {
        let cfg = Config::default();

        let fresh = cfg.for_new_monitor("new-uuid", None);

        assert_eq!(fresh.uuid, "new-uuid");
        assert_eq!(fresh.interval_value, 30);
        assert_eq!(fresh.interval_unit, IntervalUnit::Minutes);
        assert!(!fresh.paused);
    }

    #[test]
    fn from_legacy_converts_the_old_flat_shape_into_one_monitor_entry() {
        let legacy_toml = r#"
            folder = "/home/user/Wallpapers"
            interval_value = 45
            interval_unit = "hours"
            paused = true
        "#;
        let legacy: LegacyConfig = toml::from_str(legacy_toml).unwrap();

        let migrated = Config::from_legacy(legacy, "primary-uuid".to_string());

        assert_eq!(migrated.monitors.len(), 1);
        assert_eq!(migrated.monitors[0].uuid, "primary-uuid");
        assert_eq!(migrated.monitors[0].folder, PathBuf::from("/home/user/Wallpapers"));
        assert_eq!(migrated.monitors[0].interval_value, 45);
        assert_eq!(migrated.monitors[0].interval_unit, IntervalUnit::Hours);
        assert!(migrated.monitors[0].paused);

        // and the migrated shape round-trips cleanly, same as any other Config
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migrated.toml");
        migrated.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), migrated);
    }
}
```

Note: `Config::load()`'s full migration path (reading the real `config_path()`, detecting the old shape, calling the real `monitors::list_connected_monitors()`) is deliberately not unit-tested end-to-end — it isn't parameterized by path, and calling the real `list_connected_monitors()` from a test would make the test depend on this machine's actual connected monitors rather than controlled inputs. Its pieces (`from_legacy`, `for_new_monitor`) are tested directly above; the full path is covered by Task 9's manual verification.

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core config::`
Expected: all seven tests PASS.

- [ ] **Step 3: Run the full `wallpaper-core` test suite**

Run: `cargo test -p wallpaper-core`
Expected: everything passes. (`crate::monitors` and `crate::fs_util` are both already present from Task 1 and earlier work, so this should compile cleanly.)

- [ ] **Step 4: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): per-monitor Config with automatic migration from the old flat format"
```

---

### Task 3: `wallpaper-core` — per-monitor `State`

**Files:**
- Modify: `core/src/state.rs`

**Interfaces:**
- Produces:
  - `wallpaper_core::state::MonitorState { uuid: String, current_wallpaper: PathBuf, next_change_at_unix: i64 }`.
  - `wallpaper_core::state::State { monitors: Vec<MonitorState> }` (replaces the old flat fields), implementing `Default`.
  - `State::monitor(&self, uuid: &str) -> Option<&MonitorState>`.
  - `State::set_monitor(&mut self, monitor_state: MonitorState)` — upserts one entry, leaving others untouched.

- [ ] **Step 1: Replace the full contents of `core/src/state.rs`**

```rust
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorState {
    pub uuid: String,
    pub current_wallpaper: PathBuf,
    pub next_change_at_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub monitors: Vec<MonitorState>,
}

impl State {
    pub fn monitor(&self, uuid: &str) -> Option<&MonitorState> {
        self.monitors.iter().find(|m| m.uuid == uuid)
    }

    /// Replaces (or inserts) one monitor's state entry, leaving every other entry
    /// untouched.
    pub fn set_monitor(&mut self, monitor_state: MonitorState) {
        match self.monitors.iter_mut().find(|m| m.uuid == monitor_state.uuid) {
            Some(existing) => *existing = monitor_state,
            None => self.monitors.push(monitor_state),
        }
    }

    pub fn load_from(path: &Path) -> anyhow::Result<State> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fs_util::atomic_write(path, &text)
    }

    pub fn load() -> anyhow::Result<State> {
        Self::load_from(&state_path())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&state_path())
    }
}

pub fn state_path() -> PathBuf {
    crate::config::config_dir().join("state.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(uuid: &str) -> MonitorState {
        MonitorState {
            uuid: uuid.to_string(),
            current_wallpaper: PathBuf::from("/tmp/wallpapers/a.png"),
            next_change_at_unix: 1_800_000_000,
        }
    }

    #[test]
    fn state_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let state = State { monitors: vec![sample("uuid-a")] };

        state.save_to(&path).unwrap();
        let loaded = State::load_from(&path).unwrap();

        assert_eq!(loaded, state);
    }

    #[test]
    fn monitor_finds_the_matching_entry() {
        let state = State { monitors: vec![sample("uuid-a")] };
        assert!(state.monitor("uuid-a").is_some());
        assert!(state.monitor("uuid-b").is_none());
    }

    #[test]
    fn set_monitor_updates_an_existing_entry_in_place_without_touching_others() {
        let mut state = State { monitors: vec![sample("a"), sample("b")] };

        state.set_monitor(MonitorState {
            uuid: "a".to_string(),
            current_wallpaper: PathBuf::from("/new-a.png"),
            next_change_at_unix: 99,
        });

        assert_eq!(state.monitors.len(), 2);
        assert_eq!(state.monitor("a").unwrap().current_wallpaper, PathBuf::from("/new-a.png"));
        assert_eq!(state.monitor("b").unwrap().current_wallpaper, PathBuf::from("/tmp/wallpapers/a.png"));
    }

    #[test]
    fn set_monitor_inserts_a_new_entry_when_the_uuid_is_unseen() {
        let mut state = State::default();
        state.set_monitor(sample("new"));
        assert_eq!(state.monitors.len(), 1);
    }

    #[test]
    fn loading_an_old_flat_format_file_yields_an_empty_monitor_list_instead_of_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "current_wallpaper = \"/a.png\"\nnext_change_at_unix = 123\n").unwrap();

        let loaded = State::load_from(&path).unwrap();

        assert!(loaded.monitors.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core state::`
Expected: all five tests PASS.

- [ ] **Step 3: Run the full `wallpaper-core` test suite**

Run: `cargo test -p wallpaper-core`
Expected: everything passes.

- [ ] **Step 4: Commit**

```bash
git add core/src/state.rs
git commit -m "feat(core): per-monitor State"
```

---

### Task 4: `wallpaper-core` — per-monitor KDE backend targeting

**Files:**
- Modify: `core/src/backend.rs`
- Modify: `core/src/kde_backend.rs`

**Interfaces:**
- Consumes: `wallpaper_core::monitors::Monitor` (Task 1).
- Produces: `WallpaperBackend::set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()>` (signature change — was `set_wallpaper(&self, path: &Path)`).

- [ ] **Step 1: Update the trait**

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
```

- [ ] **Step 2: Write the failing tests**

Replace the full contents of `core/src/kde_backend.rs`:

```rust
use std::path::Path;
use crate::backend::WallpaperBackend;
use crate::monitors::Monitor;

pub struct KdePlasmaBackend;

/// Escapes a string for embedding inside a double-quoted JavaScript string literal.
///
/// Wallpaper paths come from a user-chosen folder, so a filename may legitimately
/// contain `"`, `\`, or (on Linux) even a newline. Interpolating those raw into the
/// Plasma shell script would break the literal or let a crafted filename inject
/// arbitrary Plasma scripting code.
fn escape_js_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Computes `target`'s rank (0-based) among `all_monitors` when sorted top-to-bottom,
/// then left-to-right - the same order `build_wallpaper_script`'s generated script
/// sorts Plasma's `desktops()` by, so index `rank` in both refers to the same
/// physical monitor. `None` if `target` isn't actually present in `all_monitors`
/// (shouldn't happen in practice - callers always pass `target` as one of
/// `all_monitors` - but this avoids a panic if that invariant is ever violated).
fn position_rank(all_monitors: &[Monitor], target: &Monitor) -> Option<usize> {
    let mut sorted: Vec<&Monitor> = all_monitors.iter().collect();
    sorted.sort_by_key(|m| (m.y, m.x));
    sorted.iter().position(|m| m.uuid == target.uuid)
}

/// Plasma's scripting API has no hardware/connector identifier on a `Desktop` object
/// (`desktops()[i].screen` is only a KWin screen index) - `screenGeometry(screen)`'s
/// physical position is the only reliable correlation key, matching `position_rank`'s
/// use of `Monitor.x`/`.y` from `kscreen-doctor`.
fn build_wallpaper_script(rank: usize, path: &Path) -> String {
    format!(
        r#"var sorted = desktops().filter(function(d) {{ return d.screen != -1; }}).sort(function(a, b) {{
    var ga = screenGeometry(a.screen), gb = screenGeometry(b.screen);
    if (ga.top !== gb.top) return ga.top - gb.top;
    return ga.left - gb.left;
}});
var d = sorted[{rank}];
if (d) {{
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
    d.writeConfig("Image", "file://{}");
}}"#,
        escape_js_string(&path.display().to_string())
    )
}

impl WallpaperBackend for KdePlasmaBackend {
    fn set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        let Some(rank) = position_rank(all_monitors, target) else {
            anyhow::bail!("target monitor {} is not present in all_monitors", target.uuid);
        };
        let script = build_wallpaper_script(rank, path);
        let connection = zbus::blocking::Connection::session()?;
        connection.call_method(
            Some("org.kde.plasmashell"),
            "/PlasmaShell",
            Some("org.kde.PlasmaShell"),
            "evaluateScript",
            &(script,),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn monitor(uuid: &str, x: i32, y: i32) -> Monitor {
        Monitor { uuid: uuid.to_string(), connector: uuid.to_string(), is_primary: false, x, y }
    }

    #[test]
    fn script_embeds_the_image_path_as_a_file_url() {
        let script = build_wallpaper_script(0, &PathBuf::from("/home/user/Pictures/a.png"));
        assert!(script.contains(r#"file:///home/user/Pictures/a.png"#));
        assert!(script.contains(r#"wallpaperPlugin = "org.kde.image""#));
    }

    #[test]
    fn a_quote_in_the_path_is_escaped_instead_of_ending_the_string_literal() {
        let script = build_wallpaper_script(0, &PathBuf::from(r#"/home/user/a".png"#));
        assert!(script.contains(r#"file:///home/user/a\".png"#), "script was: {script}");
        assert!(!script.contains(r#"a".png"#));
    }

    #[test]
    fn backslashes_and_control_characters_in_the_path_are_escaped() {
        let script = build_wallpaper_script(0, &PathBuf::from("/home/user/a\\b\nc.png"));
        assert!(script.contains(r"a\\b"), "script was: {script}");
        assert!(script.contains(r"b\nc.png"), "script was: {script}");
        assert!(!script.contains("b\nc.png"));
    }

    #[test]
    fn script_targets_the_computed_rank_index() {
        let script = build_wallpaper_script(2, &PathBuf::from("/a.png"));
        assert!(script.contains("sorted[2]"), "script was: {script}");
    }

    #[test]
    fn position_rank_orders_monitors_left_to_right_when_at_the_same_height() {
        let left = monitor("left", 0, 0);
        let right = monitor("right", 1920, 0);
        let all = vec![right.clone(), left.clone()]; // deliberately out of order

        assert_eq!(position_rank(&all, &left), Some(0));
        assert_eq!(position_rank(&all, &right), Some(1));
    }

    #[test]
    fn position_rank_prioritizes_vertical_position_over_horizontal() {
        let top = monitor("top", 1000, 0);
        let bottom_left = monitor("bottom-left", 0, 1080);
        let all = vec![top.clone(), bottom_left.clone()];

        // even though bottom-left has a smaller x, its larger y means it ranks after top
        assert_eq!(position_rank(&all, &top), Some(0));
        assert_eq!(position_rank(&all, &bottom_left), Some(1));
    }

    #[test]
    fn position_rank_returns_none_for_a_monitor_not_in_the_list() {
        let all = vec![monitor("a", 0, 0)];
        let stranger = monitor("b", 100, 100);
        assert_eq!(position_rank(&all, &stranger), None);
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core kde_backend::`
Expected: all seven tests PASS.

- [ ] **Step 4: Run the full `wallpaper-core` test suite**

Run: `cargo test -p wallpaper-core`
Expected: everything passes. (`daemon` and `gui` will fail to compile until Tasks 5-8 update their own use of `WallpaperBackend` - that's expected and resolved by those tasks, not this one.)

- [ ] **Step 5: Commit**

```bash
git add core/src/backend.rs core/src/kde_backend.rs
git commit -m "feat(core): target one physical monitor via position-based correlation with desktops()"
```

---

### Task 5: `daemon` — per-monitor `Engine`

**Files:**
- Modify: `daemon/src/engine.rs`

**Interfaces:**
- Consumes: `wallpaper_core::backend::WallpaperBackend` (Task 4, new signature), `wallpaper_core::config::{Config, MonitorConfig}` (Task 2), `wallpaper_core::monitors::Monitor` (Task 1).
- Produces:
  - `Engine::new(backend: B, config: Config, monitors: Vec<Monitor>) -> Self` (signature change — was `new(backend, config)`).
  - `Engine::is_paused(&self, uuid: &str) -> bool`, `Engine::interval(&self, uuid: &str) -> Duration` (both now take a `uuid` parameter).
  - `Engine::apply_next(&mut self, uuid: &str) -> anyhow::Result<Option<PathBuf>>` (now takes a `uuid` parameter).
  - `Engine::update_config(&mut self, new_config: Config)` (unchanged signature, new per-monitor behavior).
  - `Engine::update_monitors(&mut self, monitors: Vec<Monitor>) -> Config` — new: reconciles the live monitor list, gives any newly-seen UUID a config entry (copying the primary's settings) and a fresh queue, and returns the (possibly updated) `Config` for the caller to persist.

- [ ] **Step 1: Replace the full contents of `daemon/src/engine.rs`**

```rust
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
    /// to disk itself - returns the updated `Config` so the caller can persist it,
    /// keeping `Engine` free of file I/O concerns (matching this project's existing
    /// separation between rotation logic and persistence, done in `main.rs`).
    pub fn update_monitors(&mut self, monitors: Vec<Monitor>) -> Config {
        let primary_uuid = monitors.iter().find(|m| m.is_primary).map(|m| m.uuid.clone());
        for monitor in &monitors {
            if self.config.monitor(&monitor.uuid).is_none() {
                let fresh = self.config.for_new_monitor(&monitor.uuid, primary_uuid.as_deref());
                self.queues
                    .insert(monitor.uuid.clone(), WallpaperQueue::new(list_wallpapers(&fresh.folder)));
                self.config.monitors.push(fresh);
            }
        }
        self.monitors = monitors;
        self.config.clone()
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

        let updated_config = engine.update_monitors(vec![monitor("primary", true), monitor("new", false)]);

        let new_entry = updated_config.monitor("new").unwrap();
        assert_eq!(new_entry.interval_value, 45);
        assert_eq!(new_entry.interval_unit, IntervalUnit::Hours);
    }

    #[test]
    fn update_monitors_leaves_an_existing_monitors_config_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg_a = monitor_config("uuid-a", dir.path().to_path_buf());
        cfg_a.interval_value = 99;
        let config = Config { monitors: vec![cfg_a] };
        let backend = FakeBackend { calls: Arc::new(Mutex::new(Vec::new())) };
        let mut engine = Engine::new(backend, config, vec![monitor("uuid-a", true)]);

        let updated_config = engine.update_monitors(vec![monitor("uuid-a", true)]);

        assert_eq!(updated_config.monitor("uuid-a").unwrap().interval_value, 99);
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
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-changer-daemon engine::`
Expected: all nine tests PASS. (`daemon/src/main.rs` will fail to compile until Task 6 - expected, resolved by that task.)

- [ ] **Step 3: Commit**

```bash
git add daemon/src/engine.rs
git commit -m "feat(daemon): per-monitor rotation queues and deadlines in Engine"
```

---

### Task 6: `daemon` — per-monitor main loop

**Files:**
- Modify: `daemon/src/main.rs`

**Interfaces:**
- Consumes: `engine::Engine` (Task 5), `wallpaper_core::monitors::list_connected_monitors` (Task 1), `wallpaper_core::config::{Config, change_now_request_path}` (Task 2), `wallpaper_core::state::{State, MonitorState}` (Task 3), `watcher::{DaemonEvent, spawn_watcher}` (unchanged, from the base project).
- Produces: the daemon's final `fn main()` — terminal node for this plan.

- [ ] **Step 1: Replace the full contents of `daemon/src/main.rs`**

```rust
mod watcher;
mod engine;
mod tray;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wallpaper_core::config::{change_now_request_path, config_dir, Config};
use wallpaper_core::kde_backend::KdePlasmaBackend;
use wallpaper_core::monitors::list_connected_monitors;
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
    rx: Receiver<DaemonEvent>,
    state_path: std::path::PathBuf,
) -> anyhow::Result<()> {
    let mut deadlines: HashMap<String, SystemTime> = HashMap::new();
    let mut next_monitor_poll = SystemTime::now();

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
                    match std::fs::read_to_string(change_now_request_path()) {
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
    let engine = Engine::new(KdePlasmaBackend, config, monitors);

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
        let engine = Engine::new(RecordingBackend { calls: calls.clone() }, config, vec![monitor]);

        let (tx, rx) = channel();
        let config_dir = dir.path().to_path_buf();
        let state_path = dir.path().join("state.toml");
        let _watcher = watcher::spawn_watcher(config_dir.clone(), tx).unwrap();

        let handle = thread::spawn(move || {
            let _ = run(engine, rx, state_path);
        });

        std::fs::write(config_dir.join("change_now_request"), b"uuid-a").unwrap();
        thread::sleep(Duration::from_secs(2));

        // `run` only returns when the channel disconnects; drop the watcher's sender
        // side by ending the test process is not an option, so just assert on calls
        // so far and let the test process exit (the thread is daemonized by the test
        // harness).
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.as_slice(), &[("uuid-a".to_string(), dir.path().join("a.png"))]);
        drop(handle);
    }
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p wallpaper-changer-daemon`
Expected: compiles cleanly.

- [ ] **Step 3: Run the test**

Run: `cargo test -p wallpaper-changer-daemon main`
Expected: `change_now_request_triggers_an_immediate_wallpaper_change_for_the_named_monitor_only` PASSes within a few seconds.

- [ ] **Step 4: Run the full daemon test suite**

Run: `cargo test -p wallpaper-changer-daemon`
Expected: everything passes, including `engine::` (Task 5) and `watcher::` (unchanged).

- [ ] **Step 5: Commit**

```bash
git add daemon/src/main.rs
git commit -m "feat(daemon): per-monitor deadlines, hot-plug polling, UUID-targeted change-now"
```

---

### Task 7: `gui` — monitor-selector UI

**Files:**
- Modify: `gui/ui/app-window.slint`

**Interfaces:**
- Produces: `AppWindow` gains `in property <[string]> monitor-labels`, `in-out property <int> selected-monitor-index`, and `callback monitor-selected()`. All other existing properties/callbacks are unchanged.

- [ ] **Step 1: Add the monitor selector**

Replace the full contents of `gui/ui/app-window.slint`:

```slint
export { GuiTray } from "tray-icon.slint";

import { Button, ComboBox, SpinBox, LineEdit } from "std-widgets.slint";

export component AppWindow inherits Window {
    title: "Wallpaper Changer";
    width: 420px;
    height: 520px;

    in property <[string]> monitor-labels;
    in-out property <int> selected-monitor-index: 0;

    in property <image> preview-image;
    in-out property <string> folder-path;
    in-out property <int> interval-value: 30;
    in-out property <int> interval-unit-index: 0;
    in property <string> countdown-text: "";
    in property <bool> paused: false;

    callback monitor-selected();
    callback choose-folder();
    callback toggle-pause();
    callback change-now();
    callback save();

    VerticalLayout {
        padding: 16px;
        spacing: 12px;

        Text { text: "Monitor"; }
        ComboBox {
            model: monitor-labels;
            current-index <=> selected-monitor-index;
            selected => { monitor-selected(); }
        }

        Image {
            source: preview-image;
            height: 160px;
            image-fit: contain;
        }

        Text { text: "Carpeta de fondos"; }
        HorizontalLayout {
            spacing: 6px;
            LineEdit {
                text: folder-path;
                enabled: false;
            }
            Button {
                text: "Elegir…";
                clicked => { choose-folder(); }
            }
        }

        Text { text: "Cambiar cada"; }
        HorizontalLayout {
            spacing: 6px;
            SpinBox {
                value <=> interval-value;
                minimum: 1;
                maximum: 999;
            }
            ComboBox {
                model: ["minutos", "horas", "días"];
                current-index <=> interval-unit-index;
            }
        }

        Text { text: countdown-text; }

        HorizontalLayout {
            spacing: 8px;
            Button {
                text: paused ? "Reanudar" : "Pausar";
                clicked => { toggle-pause(); }
            }
            Button {
                text: "Cambiar ahora";
                clicked => { change-now(); }
            }
        }

        Button {
            text: "Guardar";
            clicked => { save(); }
        }
    }
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles. This only proves the `.slint` compiles via `build.rs` — `gui/src/main.rs` doesn't reference the new properties/callback yet (Task 8's job), which is expected. If `ComboBox`'s `selected` callback signature doesn't match `selected => { ... }` (it's declared as `callback selected(current-value: string);` upstream, but the handler here ignores the string and just reacts to the fact a selection happened - check `cargo doc`/the installed `std-widgets.slint` if the zero-argument handler form doesn't compile, and adapt to `selected(value) => { ... }` while ignoring `value`) - keep the same shape: read `selected-monitor-index` (kept in sync via `current-index <=> selected-monitor-index`) from Rust after the callback fires.

- [ ] **Step 3: Commit**

```bash
git add gui/ui/app-window.slint
git commit -m "feat(gui): replace the single form with a monitor-selector dropdown"
```

---

### Task 8: `gui` — wire the monitor selector and per-monitor callbacks

**Files:**
- Modify: `gui/src/main.rs`

**Interfaces:**
- Consumes: `wallpaper_core::monitors::{list_connected_monitors, Monitor}` (Task 1), `wallpaper_core::config::{Config, MonitorConfig}` (Task 2), `wallpaper_core::state::State` (Task 3), the `AppWindow`'s new `monitor-labels`/`selected-monitor-index`/`monitor-selected` (Task 7).
- Produces: the GUI's final `fn main()` — terminal node for this plan.

- [ ] **Step 1: Replace the full contents of `gui/src/main.rs`**

```rust
slint::include_modules!();

mod singleton;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wallpaper_core::config::{change_now_request_path, gui_lock_path, gui_socket_path, Config, IntervalUnit};
use wallpaper_core::monitors::{list_connected_monitors, Monitor};
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

    let mut monitors = list_connected_monitors().unwrap_or_default();
    monitors.sort_by(|a, b| a.connector.cmp(&b.connector));
    let primary_uuid = monitors.iter().find(|m| m.is_primary).map(|m| m.uuid.clone());
    let uuids: Rc<Vec<String>> = Rc::new(monitors.iter().map(|m| m.uuid.clone()).collect());

    let labels: Vec<slint::SharedString> = monitors
        .iter()
        .enumerate()
        .map(|(i, m)| monitor_label(m, i).into())
        .collect();
    let labels_model = Rc::new(slint::VecModel::from(labels));
    ui.set_monitor_labels(labels_model.into());
    ui.set_selected_monitor_index(0);

    let current_uuid: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(uuids.first().cloned()));
    let shown_wallpaper: Rc<RefCell<Option<(String, PathBuf)>>> = Rc::new(RefCell::new(None));

    if let Some(uuid) = current_uuid.borrow().clone() {
        if let Ok(config) = Config::load() {
            populate_form(&ui, &uuid, &config, primary_uuid.as_deref());
        }
        refresh_state(&ui, &uuid, &shown_wallpaper);
    }

    ui.on_monitor_selected({
        let ui_handle = ui.as_weak();
        let uuids = uuids.clone();
        let current_uuid = current_uuid.clone();
        let shown_wallpaper = shown_wallpaper.clone();
        let primary_uuid = primary_uuid.clone();
        move || {
            let Some(ui) = ui_handle.upgrade() else { return };
            let index = ui.get_selected_monitor_index();
            let Some(uuid) = uuids.get(index as usize) else { return };
            *current_uuid.borrow_mut() = Some(uuid.clone());
            *shown_wallpaper.borrow_mut() = None; // force a fresh decode for the newly-selected monitor
            if let Ok(config) = Config::load() {
                populate_form(&ui, uuid, &config, primary_uuid.as_deref());
            }
            refresh_state(&ui, uuid, &shown_wallpaper);
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
                    let mut fresh = config.for_new_monitor(&uuid, primary_uuid.as_deref());
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

    ui.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles cleanly, no warnings.

If `ui.set_monitor_labels(labels_model.into())` doesn't accept an `Rc<VecModel<SharedString>>` directly, check `cargo doc --open -p slint` for `ModelRc`'s conversion methods for the version resolved (the shape needed: wrap a `Vec<SharedString>` in something implementing Slint's `Model` trait, then convert that into whatever the generated `set_monitor_labels` setter expects — `Rc::new(VecModel::from(vec)).into()` or `ModelRc::new(VecModel::from(vec))` are the two common forms depending on version).

- [ ] **Step 3: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test from Tasks 1-6 passes (`gui` itself has no automated tests, matching this project's established precedent that Slint UI wiring is verified manually, not automated).

- [ ] **Step 4: Commit**

```bash
git add gui/src/main.rs
git commit -m "feat(gui): wire the monitor selector to per-monitor config/state"
```

---

### Task 9: End-to-end manual verification on real KDE Plasma

**Files:** none (verification only).

**Interfaces:** none — this task validates Tasks 1-8 together, including the parts that couldn't be exercised by an automated test (real `kscreen-doctor`/`kwinoutputconfig.json` integration, real multi-monitor `desktops()` targeting).

- [ ] **Step 1: Fresh install with the existing single monitor**

Run `./install.sh`. Confirm the daemon starts (`systemctl --user status wallpaper-changer-daemon`), and that your existing `config.toml` (single-monitor format from before this plan) is migrated automatically: check `cat ~/.config/wallpaper-changer/config.toml` shows the new `[[monitors]]` format with your existing folder/interval carried over, keyed by your primary monitor's UUID (cross-check against `kscreen-doctor --json | grep -A2 priority` or the relevant entry in `~/.config/kwinoutputconfig.json`).

- [ ] **Step 2: Verify the GUI shows the migrated monitor correctly**

Open the GUI. Confirm the monitor dropdown shows one entry (e.g. "Monitor 1 (LVDS-1)"), and the form is pre-filled with your migrated folder/interval/pause state.

- [ ] **Step 3: Verify rotation still works on a single monitor**

Set the interval to 1 minute, save, and confirm (same as the base project's own Task 14) that the wallpaper rotates automatically after slightly over a minute, and `state.toml` reflects the change for that monitor's UUID.

- [ ] **Step 4: Verify "Cambiar ahora" targets only the selected monitor**

If you have access to a second monitor for this step (connect one temporarily if possible): with two monitors configured with different folders, click "Cambiar ahora" while one monitor is selected in the dropdown. Expected: only that monitor's wallpaper changes; the other monitor's `state.toml` entry and actual desktop wallpaper are untouched. If a second monitor genuinely isn't available, at minimum confirm `change_now_request`'s content is the selected monitor's UUID (`cat ~/.config/wallpaper-changer/change_now_request`) after clicking the button.

- [ ] **Step 5: Verify hot-plug behavior (requires a second monitor)**

Connect a second monitor. Within 30 seconds, confirm: a new entry appears in `config.toml` for its UUID (copying your primary monitor's folder/interval/pause), the GUI's dropdown gains a second entry once reopened or refreshed, and the daemon applies a wallpaper to it (check its own `state.toml` entry gets populated). Disconnect it again and confirm its `config.toml`/`state.toml` entries are NOT removed, and the GUI dropdown drops back to one entry.

- [ ] **Step 6: Verify per-monitor targeting is actually correct on real hardware**

With two monitors connected side by side, set clearly different folders for each (e.g. one folder of solid-red test images, one of solid-blue), trigger "Cambiar ahora" for each individually, and visually confirm each physical monitor shows the correct color — this is the one thing that can only be verified on real multi-monitor hardware, proving the `screenGeometry()`-based position correlation (Task 4) actually holds up in practice, not just in the unit-tested rank-computation logic.

- [ ] **Step 7: Verify pause is per-monitor**

Pause one monitor (via its tab... via the dropdown's selected monitor) and confirm the other keeps rotating on schedule while the paused one doesn't.

No commit for this task — it's pure verification. If any step fails, fix the relevant task and re-run that task's own tests before re-attempting this task. Steps 4-7 that need a second monitor and can't be run: note explicitly in your final report which steps were skipped and why, rather than silently marking the task complete.
