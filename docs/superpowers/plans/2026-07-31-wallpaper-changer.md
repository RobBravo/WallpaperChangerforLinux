# Wallpaper Changer Linux Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust workspace that rotates the desktop wallpaper on KDE Plasma from a user-chosen folder at a configurable interval, running as an autostarted background daemon with a simple Slint GUI for configuration and a system tray icon for quick controls.

**Architecture:** Three-crate Cargo workspace: `wallpaper-core` (shared config/state models, folder scanner, random rotation queue, KDE Plasma backend), `wallpaper-changer-daemon` (background binary: timer loop + file watcher + tray icon), `wallpaper-changer-gui` (Slint binary: configuration window). The daemon and GUI never talk to each other directly — they only read/write shared files under `~/.config/wallpaper-changer/`, and the daemon reacts to filesystem-watch events. See `docs/superpowers/specs/2026-07-31-wallpaper-changer-design.md` for the full design rationale.

**Tech Stack:** Rust (edition 2021), Slint (GUI), `ksni` (KDE system tray / StatusNotifierItem), `zbus` (D-Bus call to Plasma), `notify` (file watching), `rfd` (native folder picker), `serde`/`toml` (config/state persistence), `rand` (shuffle), `anyhow` (errors), `dirs` (XDG paths), systemd user service (autostart).

## Global Constraints

- Single monitor only — no multi-monitor logic anywhere in this plan.
- KDE Plasma only for v1. The `WallpaperBackend` trait exists so other desktop environments can be added later, but do not implement them now.
- The shared library crate's **package name is `wallpaper-core`** (Rust identifier `wallpaper_core`), never `core` — `core` is the name of a built-in Rust crate and reusing it causes name-resolution conflicts.
- All shared runtime files live under `~/.config/wallpaper-changer/` (resolved via `dirs::config_dir()`): `config.toml`, `state.toml`, `change_now_request`. Never hardcode `~/.config` — always go through the shared helper functions built in Task 2.
- Supported image extensions: `png`, `jpg`, `jpeg`, `bmp` (case-insensitive), top-level of the chosen folder only — never recurse into subfolders.
- No async runtime (no `tokio`). The daemon uses plain OS threads and `std::sync::mpsc` channels with `recv_timeout` for its event loop — this is intentional, keep it that way.
- Add third-party dependencies with `cargo add <crate> [--features ...]` (run from inside the relevant crate directory) rather than hand-writing version numbers into `Cargo.toml` — this always resolves to a version that actually exists.
- Third-party crate APIs referenced in this plan (`ksni`, `zbus`, `slint`, `rfd`) were verified against public docs/examples at planning time, but these crates move fast. If a code sample in a task doesn't compile against the version `cargo add` pulls in, check `cargo doc --open -p <crate>` (or `docs.rs/<crate>`) for the current signature and adapt — that's expected integration work, not a sign the task itself is wrong.
- Every `git commit` step commits only the files listed in that step.

---

### Task 1: Workspace scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs`
- Create: `daemon/Cargo.toml`
- Create: `daemon/src/main.rs`
- Create: `gui/Cargo.toml`
- Create: `gui/src/main.rs`
- Create: `.gitignore` entries (already has `/target` and `.superpowers/` from brainstorming — verify, don't duplicate)

**Interfaces:**
- Produces: a workspace that builds three empty binaries/lib so every later task can `cargo build`/`cargo test` immediately.

- [ ] **Step 1: Create the workspace root `Cargo.toml`**

```toml
[workspace]
members = ["core", "daemon", "gui"]
resolver = "2"
```

- [ ] **Step 2: Create the `wallpaper-core` library crate**

`core/Cargo.toml`:

```toml
[package]
name = "wallpaper-core"
version = "0.1.0"
edition = "2021"

[lib]
name = "wallpaper_core"
path = "src/lib.rs"

[dependencies]
```

`core/src/lib.rs`:

```rust
pub fn placeholder() {}
```

(This function is deleted in Task 2 once real modules exist — it only exists so Step 4 has something to build.)

- [ ] **Step 3: Create the `wallpaper-changer-daemon` binary crate**

`daemon/Cargo.toml`:

```toml
[package]
name = "wallpaper-changer-daemon"
version = "0.1.0"
edition = "2021"

[dependencies]
wallpaper-core = { path = "../core" }
```

`daemon/src/main.rs`:

```rust
fn main() {
    println!("wallpaper-changer-daemon placeholder");
}
```

- [ ] **Step 4: Create the `wallpaper-changer-gui` binary crate**

`gui/Cargo.toml`:

```toml
[package]
name = "wallpaper-changer-gui"
version = "0.1.0"
edition = "2021"

[dependencies]
wallpaper-core = { path = "../core" }
```

`gui/src/main.rs`:

```rust
fn main() {
    println!("wallpaper-changer-gui placeholder");
}
```

- [ ] **Step 5: Verify the workspace builds**

Run: `cargo build`
Expected: compiles all three crates with no errors (warnings about unused `placeholder` are fine and will disappear in later tasks).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml core/Cargo.toml core/src/lib.rs daemon/Cargo.toml daemon/src/main.rs gui/Cargo.toml gui/src/main.rs
git commit -m "chore: scaffold wallpaper-core/daemon/gui workspace"
```

---

### Task 2: `wallpaper-core` — config model

**Files:**
- Modify: `core/src/lib.rs`
- Create: `core/src/config.rs`
- Test: inline `#[cfg(test)]` module in `core/src/config.rs`

**Interfaces:**
- Produces:
  - `wallpaper_core::config::IntervalUnit` enum (`Minutes`, `Hours`, `Days`) with `.to_duration(self, value: u64) -> std::time::Duration`.
  - `wallpaper_core::config::Config { folder: PathBuf, interval_value: u64, interval_unit: IntervalUnit, paused: bool }`, implementing `Default`.
  - `wallpaper_core::config::config_dir() -> PathBuf`, `config_path() -> PathBuf`, `change_now_request_path() -> PathBuf`.
  - `Config::load_from(path: &Path) -> anyhow::Result<Config>`, `Config::save_to(&self, path: &Path) -> anyhow::Result<()>`, `Config::load() -> anyhow::Result<Config>`, `Config::save(&self) -> anyhow::Result<()>`.
- Consumes: nothing yet (first real module).

- [ ] **Step 1: Add dependencies**

Run (from `core/`):
```bash
cd core
cargo add serde --features derive
cargo add toml
cargo add anyhow
cargo add dirs
cd ..
```

- [ ] **Step 2: Write the failing tests**

Create `core/src/config.rs`:

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub folder: PathBuf,
    pub interval_value: u64,
    pub interval_unit: IntervalUnit,
    pub paused: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            folder: dirs::picture_dir().unwrap_or_else(|| PathBuf::from(".")),
            interval_value: 30,
            interval_unit: IntervalUnit::Minutes,
            paused: false,
        }
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

impl Config {
    pub fn load_from(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn load() -> anyhow::Result<Config> {
        let path = config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.save()?;
            return Ok(cfg);
        }
        Self::load_from(&path)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&config_path())
    }
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

    #[test]
    fn config_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config {
            folder: PathBuf::from("/tmp/wallpapers"),
            interval_value: 45,
            interval_unit: IntervalUnit::Hours,
            paused: true,
        };

        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();

        assert_eq!(loaded, cfg);
    }
}
```

Add `wallpaper_core::config::*` needs to actually be reachable — update `core/src/lib.rs`:

```rust
pub mod config;
```

(remove the `placeholder()` function from Task 1)

- [ ] **Step 2b: Add `tempfile` as a dev-dependency**

Run:
```bash
cd core
cargo add tempfile --dev
cd ..
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core`
Expected: `interval_unit_converts_to_duration` and `config_round_trips_through_toml_file` both PASS. (There's no separate "write minimal implementation" step here because the module above is the implementation — this task combines writing the module and its tests in one pass since the logic is simple enough to be obviously correct from the type signatures; TDD's "see it fail first" is preserved by writing the test functions before running `cargo test` for the first time.)

- [ ] **Step 4: Commit**

```bash
git add core/Cargo.toml core/src/lib.rs core/src/config.rs
git commit -m "feat(core): add Config model with TOML persistence"
```

---

### Task 3: `wallpaper-core` — state model

**Files:**
- Modify: `core/src/lib.rs`
- Create: `core/src/state.rs`
- Test: inline `#[cfg(test)]` module in `core/src/state.rs`

**Interfaces:**
- Consumes: `wallpaper_core::config::config_dir()` (Task 2).
- Produces: `wallpaper_core::state::State { current_wallpaper: PathBuf, next_change_at_unix: i64 }`, `state_path() -> PathBuf`, `State::load_from(path: &Path)`, `State::save_to(&self, path: &Path)`, `State::load()`, `State::save(&self)`.

- [ ] **Step 1: Write the module with its test**

Create `core/src/state.rs`:

```rust
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub current_wallpaper: PathBuf,
    pub next_change_at_unix: i64,
}

pub fn state_path() -> PathBuf {
    crate::config::config_dir().join("state.toml")
}

impl State {
    pub fn load_from(path: &Path) -> anyhow::Result<State> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn load() -> anyhow::Result<State> {
        Self::load_from(&state_path())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&state_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let state = State {
            current_wallpaper: PathBuf::from("/tmp/wallpapers/a.png"),
            next_change_at_unix: 1_800_000_000,
        };

        state.save_to(&path).unwrap();
        let loaded = State::load_from(&path).unwrap();

        assert_eq!(loaded, state);
    }
}
```

Add to `core/src/lib.rs`:

```rust
pub mod state;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p wallpaper-core state::`
Expected: `state_round_trips_through_toml_file` PASSes.

- [ ] **Step 3: Commit**

```bash
git add core/src/lib.rs core/src/state.rs
git commit -m "feat(core): add State model with TOML persistence"
```

---

### Task 4: `wallpaper-core` — folder scanner

**Files:**
- Modify: `core/src/lib.rs`
- Create: `core/src/scanner.rs`
- Test: inline `#[cfg(test)]` module in `core/src/scanner.rs`

**Interfaces:**
- Produces: `wallpaper_core::scanner::list_wallpapers(folder: &Path) -> Vec<PathBuf>` — sorted, top-level only, filtered to `png`/`jpg`/`jpeg`/`bmp` (case-insensitive).

- [ ] **Step 1: Write the failing test**

Create `core/src/scanner.rs`:

```rust
use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp"];

pub fn list_wallpapers(folder: &Path) -> Vec<PathBuf> {
    let mut images: Vec<PathBuf> = match std::fs::read_dir(folder) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| is_supported_image(path))
            .collect(),
        Err(_) => Vec::new(),
    };
    images.sort();
    images
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_top_level_supported_images_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.png"), b"x").unwrap();
        std::fs::write(dir.path().join("a.JPG"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.png"), b"x").unwrap();

        let images = list_wallpapers(dir.path());

        assert_eq!(
            images,
            vec![dir.path().join("a.JPG"), dir.path().join("b.png")]
        );
    }

    #[test]
    fn returns_empty_vec_for_missing_folder() {
        let images = list_wallpapers(Path::new("/definitely/does/not/exist"));
        assert!(images.is_empty());
    }
}
```

Add to `core/src/lib.rs`:

```rust
pub mod scanner;
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core scanner::`
Expected: both tests PASS.

- [ ] **Step 3: Commit**

```bash
git add core/src/lib.rs core/src/scanner.rs
git commit -m "feat(core): add top-level wallpaper folder scanner"
```

---

### Task 5: `wallpaper-core` — random rotation queue

**Files:**
- Modify: `core/src/lib.rs`
- Create: `core/src/queue.rs`
- Test: inline `#[cfg(test)]` module in `core/src/queue.rs`

**Interfaces:**
- Produces: `wallpaper_core::queue::WallpaperQueue::new(images: Vec<PathBuf>) -> Self`, `.next(&mut self) -> Option<PathBuf>`, `.is_empty(&self) -> bool`.

- [ ] **Step 1: Add the `rand` dependency**

Run:
```bash
cd core
cargo add rand
cd ..
```

- [ ] **Step 2: Write the failing tests**

Create `core/src/queue.rs`:

```rust
use std::collections::HashSet;
use std::path::PathBuf;
use rand::seq::SliceRandom;

pub struct WallpaperQueue {
    all: Vec<PathBuf>,
    remaining: Vec<PathBuf>,
}

impl WallpaperQueue {
    pub fn new(images: Vec<PathBuf>) -> Self {
        let mut remaining = images.clone();
        remaining.shuffle(&mut rand::thread_rng());
        WallpaperQueue { all: images, remaining }
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    pub fn next(&mut self) -> Option<PathBuf> {
        if self.all.is_empty() {
            return None;
        }
        if self.remaining.is_empty() {
            self.remaining = self.all.clone();
            self.remaining.shuffle(&mut rand::thread_rng());
        }
        self.remaining.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("/wp/{i}.png"))).collect()
    }

    #[test]
    fn empty_queue_always_returns_none() {
        let mut queue = WallpaperQueue::new(vec![]);
        assert!(queue.is_empty());
        assert_eq!(queue.next(), None);
        assert_eq!(queue.next(), None);
    }

    #[test]
    fn every_image_appears_exactly_once_before_any_repeat() {
        let all = images(5);
        let mut queue = WallpaperQueue::new(all.clone());

        let mut seen = HashSet::new();
        for _ in 0..all.len() {
            let picked = queue.next().expect("queue should not be empty yet");
            assert!(seen.insert(picked), "image repeated before the folder was exhausted");
        }
        assert_eq!(seen.len(), all.len());
    }

    #[test]
    fn queue_reshuffles_and_keeps_producing_after_exhaustion() {
        let all = images(3);
        let mut queue = WallpaperQueue::new(all.clone());

        for _ in 0..all.len() {
            queue.next();
        }
        // one more pull past exhaustion must still yield a valid image, not None
        let picked = queue.next().expect("queue should reshuffle after exhaustion");
        assert!(all.contains(&picked));
    }

    #[test]
    fn single_image_folder_keeps_returning_the_same_image() {
        let all = images(1);
        let mut queue = WallpaperQueue::new(all.clone());

        assert_eq!(queue.next(), Some(all[0].clone()));
        assert_eq!(queue.next(), Some(all[0].clone()));
    }
}
```

Add to `core/src/lib.rs`:

```rust
pub mod queue;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core queue::`
Expected: all four tests PASS.

- [ ] **Step 4: Commit**

```bash
git add core/src/lib.rs core/src/queue.rs
git commit -m "feat(core): add shuffle-and-consume wallpaper rotation queue"
```

---

### Task 6: `wallpaper-core` — wallpaper backend trait + KDE Plasma backend

**Files:**
- Modify: `core/src/lib.rs`
- Create: `core/src/backend.rs`
- Create: `core/src/kde_backend.rs`
- Test: inline `#[cfg(test)]` module in `core/src/kde_backend.rs`

**Interfaces:**
- Produces: `wallpaper_core::backend::WallpaperBackend` trait with `fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()>`; `wallpaper_core::kde_backend::KdePlasmaBackend` (unit struct implementing the trait).

- [ ] **Step 1: Add the `zbus` dependency**

Run:
```bash
cd core
cargo add zbus
cd ..
```

- [ ] **Step 2: Define the backend trait**

Create `core/src/backend.rs`:

```rust
use std::path::Path;

pub trait WallpaperBackend: Send {
    fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()>;
}
```

- [ ] **Step 3: Write the failing test for the script-building logic**

Create `core/src/kde_backend.rs`:

```rust
use std::path::Path;
use crate::backend::WallpaperBackend;

pub struct KdePlasmaBackend;

fn build_wallpaper_script(path: &Path) -> String {
    format!(
        r#"var allDesktops = desktops();
for (i = 0; i < allDesktops.length; i++) {{
    d = allDesktops[i];
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
    d.writeConfig("Image", "file://{}");
}}"#,
        path.display()
    )
}

impl WallpaperBackend for KdePlasmaBackend {
    fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()> {
        let script = build_wallpaper_script(path);
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

    #[test]
    fn script_embeds_the_image_path_as_a_file_url() {
        let script = build_wallpaper_script(&PathBuf::from("/home/user/Pictures/a.png"));
        assert!(script.contains(r#"file:///home/user/Pictures/a.png"#));
        assert!(script.contains(r#"wallpaperPlugin = "org.kde.image""#));
    }
}
```

This isolates the only genuinely unit-testable part (the design doc calls this out explicitly: verifying the actual D-Bus round trip requires a live Plasma session, so that's covered manually in Task 14, not here).

Add to `core/src/lib.rs`:

```rust
pub mod backend;
pub mod kde_backend;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core kde_backend::`
Expected: `script_embeds_the_image_path_as_a_file_url` PASSes.

- [ ] **Step 5: Run the full `wallpaper-core` test suite**

Run: `cargo test -p wallpaper-core`
Expected: every test from Tasks 2–6 PASSes.

- [ ] **Step 6: Commit**

```bash
git add core/src/lib.rs core/src/backend.rs core/src/kde_backend.rs
git commit -m "feat(core): add WallpaperBackend trait and KDE Plasma D-Bus backend"
```

---

### Task 7: daemon — config/command file watcher

**Files:**
- Create: `daemon/src/watcher.rs`
- Modify: `daemon/src/main.rs` (add `mod watcher;`)
- Test: inline `#[cfg(test)]` module in `daemon/src/watcher.rs`

**Interfaces:**
- Consumes: nothing from `wallpaper-core` directly (works on plain paths/filenames).
- Produces: `watcher::DaemonEvent` enum (`ConfigChanged`, `ChangeNowRequested`); `watcher::spawn_watcher(config_dir: PathBuf, tx: std::sync::mpsc::Sender<DaemonEvent>) -> anyhow::Result<notify::RecommendedWatcher>` — the returned watcher **must be kept alive** by the caller (dropping it stops watching).

- [ ] **Step 1: Add dependencies**

Run:
```bash
cd daemon
cargo add notify
cargo add anyhow
cargo add tempfile --dev
cd ..
```

- [ ] **Step 2: Write the failing integration test**

Create `daemon/src/watcher.rs`:

```rust
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use notify::{RecursiveMode, Watcher};

pub enum DaemonEvent {
    ConfigChanged,
    ChangeNowRequested,
}

pub fn spawn_watcher(
    config_dir: PathBuf,
    tx: Sender<DaemonEvent>,
) -> anyhow::Result<notify::RecommendedWatcher> {
    let config_file_name = OsString::from("config.toml");
    let change_now_file_name = OsString::from("change_now_request");

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        for path in event.paths {
            match path.file_name() {
                Some(name) if name == config_file_name => {
                    let _ = tx.send(DaemonEvent::ConfigChanged);
                }
                Some(name) if name == change_now_file_name => {
                    let _ = tx.send(DaemonEvent::ChangeNowRequested);
                }
                _ => {}
            }
        }
    })?;
    watcher.watch(&config_dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    #[test]
    fn writing_config_toml_sends_config_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = channel();
        let _watcher = spawn_watcher(dir.path().to_path_buf(), tx).unwrap();

        std::fs::write(dir.path().join("config.toml"), b"paused = true").unwrap();

        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(DaemonEvent::ConfigChanged) => {}
            Ok(DaemonEvent::ChangeNowRequested) => panic!("expected ConfigChanged, got ChangeNowRequested"),
            Err(RecvTimeoutError::Timeout) => panic!("no event received within timeout"),
            Err(e) => panic!("channel error: {e}"),
        }
    }

    #[test]
    fn writing_change_now_request_sends_change_now_requested() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = channel();
        let _watcher = spawn_watcher(dir.path().to_path_buf(), tx).unwrap();

        std::fs::write(dir.path().join("change_now_request"), b"123").unwrap();

        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(DaemonEvent::ChangeNowRequested) => {}
            Ok(DaemonEvent::ConfigChanged) => panic!("expected ChangeNowRequested, got ConfigChanged"),
            Err(RecvTimeoutError::Timeout) => panic!("no event received within timeout"),
            Err(e) => panic!("channel error: {e}"),
        }
    }
}
```

Add `mod watcher;` near the top of `daemon/src/main.rs` (above `fn main()`).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p wallpaper-changer-daemon watcher::`
Expected: both tests PASS. (If they're flaky/slow on your filesystem, that's a real signal about `notify`'s backend on this system — investigate rather than deleting the test; do not shorten below 5s without checking a few runs.)

- [ ] **Step 4: Commit**

```bash
git add daemon/Cargo.toml daemon/src/main.rs daemon/src/watcher.rs
git commit -m "feat(daemon): watch config.toml and change_now_request for events"
```

---

### Task 8: daemon — rotation engine

**Files:**
- Create: `daemon/src/engine.rs`
- Modify: `daemon/src/main.rs` (add `mod engine;`)
- Test: inline `#[cfg(test)]` module in `daemon/src/engine.rs`

**Interfaces:**
- Consumes: `wallpaper_core::backend::WallpaperBackend` (Task 6), `wallpaper_core::config::Config` (Task 2), `wallpaper_core::queue::WallpaperQueue` (Task 5), `wallpaper_core::scanner::list_wallpapers` (Task 4).
- Produces: `engine::Engine<B: WallpaperBackend>::new(backend: B, config: Config) -> Self`, `.is_paused(&self) -> bool`, `.interval(&self) -> std::time::Duration`, `.apply_next(&mut self) -> anyhow::Result<Option<PathBuf>>`, `.update_config(&mut self, new_config: Config)`.

- [ ] **Step 1: Write the failing tests**

Create `daemon/src/engine.rs`:

```rust
use std::path::PathBuf;
use std::time::Duration;
use wallpaper_core::backend::WallpaperBackend;
use wallpaper_core::config::Config;
use wallpaper_core::queue::WallpaperQueue;
use wallpaper_core::scanner::list_wallpapers;

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
        self.config.interval_unit.to_duration(self.config.interval_value)
    }

    pub fn apply_next(&mut self) -> anyhow::Result<Option<PathBuf>> {
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
```

Add `mod engine;` to `daemon/src/main.rs`.

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-changer-daemon engine::`
Expected: all four tests PASS.

- [ ] **Step 3: Commit**

```bash
git add daemon/src/main.rs daemon/src/engine.rs
git commit -m "feat(daemon): add rotation engine driving the wallpaper backend"
```

---

### Task 9: daemon — main loop wiring

**Files:**
- Modify: `daemon/src/main.rs`

**Interfaces:**
- Consumes: `watcher::{DaemonEvent, spawn_watcher}` (Task 7), `engine::Engine` (Task 8), `wallpaper_core::config::{Config, config_dir}` (Task 2), `wallpaper_core::state::State` (Task 3), `wallpaper_core::kde_backend::KdePlasmaBackend` (Task 6).
- Produces: the daemon's `fn main()` — no further tasks depend on internals of this file beyond what's already listed above.

- [ ] **Step 1: Replace `daemon/src/main.rs`'s `fn main` with the real event loop**

Full contents of `daemon/src/main.rs` (keep the existing `mod watcher;` and `mod engine;` lines at the top, add `mod tray;` now even though Task 10 fills it in — create an empty `daemon/src/tray.rs` with `pub fn spawn_tray() {}` for now so this compiles):

```rust
mod watcher;
mod engine;
mod tray;

use std::sync::mpsc::{channel, RecvTimeoutError};
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

fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    let mut engine = Engine::new(KdePlasmaBackend, config);

    let (tx, rx) = channel::<DaemonEvent>();
    let _watcher = watcher::spawn_watcher(config_dir(), tx)?;
    tray::spawn_tray();

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
```

Create `daemon/src/tray.rs` with just:

```rust
pub fn spawn_tray() {}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p wallpaper-changer-daemon`
Expected: compiles cleanly.

- [ ] **Step 3: Write a short end-to-end integration test**

This exercises the real loop (minus the tray, minus the real KDE backend) using a fake backend and a very short interval, proving the watcher → engine → state.toml wiring actually works together. Since `main()` isn't unit-testable directly (it owns `KdePlasmaBackend` and never returns), extract the loop body into a testable `run` function. Replace the full contents of `daemon/src/main.rs` with:

```rust
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
```

Add this test at the bottom of the same `daemon/src/main.rs` file:

```rust
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
```

Note: this test intentionally leaves its background thread running until the test process exits — `run()` has no clean shutdown path (matching production, where the daemon only stops via the tray's "Salir" which calls `std::process::exit`), so there's nothing to join. This is acceptable for a single short-lived test binary process.

- [ ] **Step 4: Run the test**

Run: `cargo test -p wallpaper-changer-daemon main`
Expected: `change_now_request_triggers_an_immediate_wallpaper_change` PASSes within a few seconds.

- [ ] **Step 5: Commit**

```bash
git add daemon/src/main.rs daemon/src/tray.rs
git commit -m "feat(daemon): wire watcher, engine and state persistence into the main loop"
```

---

### Task 10: daemon — system tray icon

**Files:**
- Modify: `daemon/src/tray.rs`
- Modify: `daemon/Cargo.toml`

**Interfaces:**
- Consumes: `wallpaper_core::config::{Config, change_now_request_path}` (Task 2).
- Produces: `tray::spawn_tray()` — called once from `main()` (already wired in Task 9); no other task depends on its internals.

- [ ] **Step 1: Add dependencies**

Run:
```bash
cd daemon
cargo add ksni --features blocking
cargo add dirs
cd ..
```

- [ ] **Step 2: Implement the tray**

Replace the contents of `daemon/src/tray.rs`:

```rust
use wallpaper_core::config::{change_now_request_path, Config};

struct DaemonTray;

impl ksni::Tray for DaemonTray {
    fn id(&self) -> String {
        "wallpaper-changer".into()
    }

    fn title(&self) -> String {
        "Wallpaper Changer".into()
    }

    fn icon_name(&self) -> String {
        "preferences-desktop-wallpaper".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Pausar/Reanudar".into(),
                activate: Box::new(|_: &mut Self| toggle_pause()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Cambiar ahora".into(),
                activate: Box::new(|_: &mut Self| request_change_now()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Abrir configuración".into(),
                activate: Box::new(|_: &mut Self| open_config_gui()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Salir".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn toggle_pause() {
    match Config::load() {
        Ok(mut config) => {
            config.paused = !config.paused;
            if let Err(e) = config.save() {
                eprintln!("tray: failed to save config.toml: {e}");
            }
        }
        Err(e) => eprintln!("tray: failed to load config.toml: {e}"),
    }
}

fn request_change_now() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    if let Err(e) = std::fs::write(change_now_request_path(), now) {
        eprintln!("tray: failed to write change_now_request: {e}");
    }
}

fn open_config_gui() {
    let path = dirs::home_dir()
        .map(|home| home.join(".local/bin/wallpaper-changer-gui"))
        .unwrap_or_else(|| std::path::PathBuf::from("wallpaper-changer-gui"));
    if let Err(e) = std::process::Command::new(path).spawn() {
        eprintln!("tray: failed to launch wallpaper-changer-gui: {e}");
    }
}

pub fn spawn_tray() {
    std::thread::spawn(|| {
        let service = ksni::TrayService::new(DaemonTray);
        service.run_blocking();
    });
}
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p wallpaper-changer-daemon`
Expected: compiles. If `ksni::Tray`, `ksni::MenuItem`, `ksni::menu::StandardItem`, or `TrayService::run_blocking` don't match the version `cargo add` pulled in, run `cargo doc --open -p ksni` and adjust field/method names to match — the shape (one struct implementing a tray trait, a `Vec` of menu items built from a "standard item" type with a label and an activate closure, run on a background thread) stays the same per the Global Constraints note.

- [ ] **Step 4: Manual verification (no automated test — this needs a live Plasma session)**

Run: `cargo run -p wallpaper-changer-daemon` on your KDE Plasma desktop.
Expected: an icon appears in the system tray; right-clicking shows the four menu items; clicking "Cambiar ahora" creates/updates `~/.config/wallpaper-changer/change_now_request`; clicking "Pausar/Reanudar" toggles `paused` in `~/.config/wallpaper-changer/config.toml`; clicking "Salir" ends the process. Stop the daemon with Ctrl+C once verified (or via "Salir").

- [ ] **Step 5: Commit**

```bash
git add daemon/Cargo.toml daemon/src/tray.rs
git commit -m "feat(daemon): add KDE system tray icon with pause/change-now/open/quit"
```

---

### Task 11: gui — Slint window definition

**Files:**
- Create: `gui/ui/app-window.slint`
- Create: `gui/build.rs`
- Modify: `gui/Cargo.toml`

**Interfaces:**
- Produces: an `AppWindow` Slint component (generated Rust type available via `slint::include_modules!()`) with properties `folder-path: string`, `interval-value: int`, `interval-unit-index: int`, `countdown-text: string`, `paused: bool`, `preview-image: image`, and callbacks `choose-folder()`, `toggle-pause()`, `change-now()`, `save()`.

- [ ] **Step 1: Add dependencies**

Run:
```bash
cd gui
cargo add slint
cargo add slint-build --build
cd ..
```

- [ ] **Step 2: Write the Slint UI file**

Create `gui/ui/app-window.slint`:

```slint
import { Button, ComboBox, SpinBox, LineEdit } from "std-widgets.slint";

export component AppWindow inherits Window {
    title: "Wallpaper Changer";
    width: 420px;
    height: 480px;

    in property <image> preview-image;
    in-out property <string> folder-path;
    in-out property <int> interval-value: 30;
    in-out property <int> interval-unit-index: 0;
    in property <string> countdown-text: "";
    in property <bool> paused: false;

    callback choose-folder();
    callback toggle-pause();
    callback change-now();
    callback save();

    VerticalLayout {
        padding: 16px;
        spacing: 12px;

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

- [ ] **Step 3: Write `gui/build.rs`**

```rust
fn main() {
    slint_build::compile("ui/app-window.slint").unwrap();
}
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles (the existing placeholder `fn main()` from Task 1 doesn't reference `AppWindow` yet, so this only proves the `.slint` file itself compiles via `build.rs` — that's the point of this task). If the widget import path or property/callback kebab-case syntax doesn't match the `slint` version `cargo add` pulled in, check `docs.slint.dev` for that version's widget reference.

- [ ] **Step 5: Commit**

```bash
git add gui/Cargo.toml gui/build.rs gui/ui/app-window.slint
git commit -m "feat(gui): add Slint configuration window layout"
```

---

### Task 12: gui — wire callbacks and state polling

**Files:**
- Modify: `gui/src/main.rs`
- Modify: `gui/Cargo.toml`

**Interfaces:**
- Consumes: `AppWindow` (Task 11), `wallpaper_core::config::{Config, IntervalUnit, change_now_request_path}` (Task 2), `wallpaper_core::state::State` (Task 3).
- Produces: the GUI's `fn main()` — terminal node, nothing depends on it further.

- [ ] **Step 1: Add the `rfd` dependency**

Run:
```bash
cd gui
cargo add rfd
cargo add anyhow
cd ..
```

- [ ] **Step 2: Replace `gui/src/main.rs`**

```rust
slint::include_modules!();

use wallpaper_core::config::{change_now_request_path, Config, IntervalUnit};
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

fn refresh_state(ui: &AppWindow) {
    let Ok(state) = State::load() else { return };

    if let Ok(image) = slint::Image::load_from_path(&state.current_wallpaper) {
        ui.set_preview_image(image);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let remaining = (state.next_change_at_unix - now).max(0);
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;
    ui.set_countdown_text(format!("Próximo cambio en {hours:02}:{minutes:02}:{seconds:02}").into());
}

fn main() -> anyhow::Result<()> {
    let ui = AppWindow::new()?;
    let config = Config::load()?;

    ui.set_folder_path(config.folder.display().to_string().into());
    ui.set_interval_value(config.interval_value as i32);
    ui.set_interval_unit_index(unit_to_index(config.interval_unit));
    ui.set_paused(config.paused);
    refresh_state(&ui);

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
        move || {
            if let Ok(mut config) = Config::load() {
                config.paused = !config.paused;
                if config.save().is_ok() {
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_paused(config.paused);
                    }
                }
            }
        }
    });

    ui.on_change_now(move || {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let _ = std::fs::write(change_now_request_path(), now);
    });

    ui.on_save({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let config = Config {
                    folder: std::path::PathBuf::from(ui.get_folder_path().to_string()),
                    interval_value: ui.get_interval_value() as u64,
                    interval_unit: index_to_unit(ui.get_interval_unit_index()),
                    paused: ui.get_paused(),
                };
                let _ = config.save();
            }
        }
    });

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    refresh_state(&ui);
                }
            },
        );
    }

    ui.run()?;
    Ok(())
}
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles. If `slint::Image::load_from_path`, `slint::Timer`, or the generated `AppWindow` setter/getter names (`set_folder_path`, `get_folder_path`, `on_choose_folder`, etc.) don't match, check `docs.slint.dev` for the installed version — the kebab-case-to-snake_case naming convention is stable across versions even if exact helper method names shift.

- [ ] **Step 4: Manual verification**

Run: `cargo run -p wallpaper-changer-gui` (with `~/.config/wallpaper-changer/config.toml` either absent — a default gets created — or pre-populated from Task 9's manual run).
Expected: window opens showing the current folder/interval/paused state; clicking "Elegir…" opens a native folder picker and updates the field; clicking "Guardar" writes the new values to `~/.config/wallpaper-changer/config.toml` (verify with `cat`); clicking "Cambiar ahora" creates/updates `~/.config/wallpaper-changer/change_now_request` (verify with `cat`); clicking "Pausar/Reanudar" flips `paused` in `config.toml` immediately and updates the button label.

- [ ] **Step 5: Commit**

```bash
git add gui/Cargo.toml gui/src/main.rs
git commit -m "feat(gui): wire configuration window to config.toml/state.toml"
```

---

### Task 13: packaging — systemd service and install script

**Files:**
- Create: `packaging/wallpaper-changer-daemon.service`
- Create: `install.sh`

**Interfaces:**
- Consumes: the built `wallpaper-changer-daemon` and `wallpaper-changer-gui` binaries (Tasks 9–12).
- Produces: a repeatable local install of both binaries plus an enabled systemd user service. Nothing else in the plan depends on this.

- [ ] **Step 1: Write the systemd unit**

Create `packaging/wallpaper-changer-daemon.service`:

```ini
[Unit]
Description=Wallpaper Changer daemon
After=graphical-session.target

[Service]
ExecStart=%h/.local/bin/wallpaper-changer-daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=graphical-session.target
```

- [ ] **Step 2: Write the install script**

Create `install.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

cargo build --release

mkdir -p "$HOME/.local/bin"
cp target/release/wallpaper-changer-daemon "$HOME/.local/bin/"
cp target/release/wallpaper-changer-gui "$HOME/.local/bin/"

mkdir -p "$HOME/.config/systemd/user"
cp packaging/wallpaper-changer-daemon.service "$HOME/.config/systemd/user/"

systemctl --user daemon-reload
systemctl --user enable --now wallpaper-changer-daemon

echo "Installed. Check status with: systemctl --user status wallpaper-changer-daemon"
```

- [ ] **Step 3: Make it executable**

Run: `chmod +x install.sh`

- [ ] **Step 4: Manual verification**

Run: `./install.sh`
Expected: builds succeed, binaries land in `~/.local/bin/`, `systemctl --user status wallpaper-changer-daemon` shows `active (running)`, and the tray icon appears. Run `journalctl --user -u wallpaper-changer-daemon -f` briefly to confirm log output flows there.

- [ ] **Step 5: Commit**

```bash
git add packaging/wallpaper-changer-daemon.service install.sh
git commit -m "chore: add systemd user service and install script"
```

---

### Task 14: End-to-end manual verification on real KDE Plasma

**Files:** none (verification only).

**Interfaces:** none — this task validates the whole system built in Tasks 1–13 together.

- [ ] **Step 1: Fresh install**

Run: `./install.sh` on your Fedora KDE Plasma machine (from Task 13).

- [ ] **Step 2: Configure through the GUI**

Open the GUI (via the tray's "Abrir configuración" or `~/.local/bin/wallpaper-changer-gui`). Point it at a real folder containing at least 3 images of supported formats, set the interval to `1` `minutos`, click "Guardar".

- [ ] **Step 3: Verify automatic rotation**

Wait slightly over a minute without touching anything.
Expected: the actual KDE Plasma desktop wallpaper changes; `~/.config/wallpaper-changer/state.toml` reflects the new `current_wallpaper` and an updated `next_change_at_unix`.

- [ ] **Step 4: Verify "Cambiar ahora"**

Click "Cambiar ahora" in either the GUI or the tray menu.
Expected: the wallpaper changes immediately (not waiting for the 1-minute mark), and the countdown in the GUI resets to ~1 minute.

- [ ] **Step 5: Verify pause**

Click "Pausar" (GUI or tray). Wait over a minute.
Expected: the wallpaper does NOT change while paused. Click "Reanudar" and confirm rotation resumes.

- [ ] **Step 6: Verify autostart**

Run: `reboot` (or at minimum `systemctl --user restart wallpaper-changer-daemon` if a full reboot isn't convenient right now).
Expected: after logging back into the KDE Plasma session, the tray icon appears without manually starting anything, and `systemctl --user status wallpaper-changer-daemon` shows it running.

- [ ] **Step 7: Verify error resilience**

Empty the configured wallpaper folder (move the images elsewhere temporarily) and trigger "Cambiar ahora".
Expected: the daemon does not crash (`systemctl --user status wallpaper-changer-daemon` still shows `active`); `journalctl --user -u wallpaper-changer-daemon` shows the "no wallpapers found" message. Restore the images afterward.

No commit for this task — it's pure verification. If any step fails, go back to the relevant task, fix it, and re-run that task's own tests before re-attempting this task.
