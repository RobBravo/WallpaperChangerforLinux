# Fase 0 — Deuda técnica pendiente: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close out the four code-level items parked from earlier reviews and listed in `ROADMAP.md`'s Fase 0: non-atomic config/state writes, zombie GUI child processes left by the daemon's tray, the TOCTOU race in the GUI's single-instance guard, and a missing `.desktop` launcher entry.

**Architecture:** Four independent, small fixes. (1) A new `wallpaper_core::fs_util::atomic_write` helper, shared by `Config::save_to` and `State::save_to`, replacing their direct `fs::write` calls with a write-to-temp-then-rename sequence. (2) The daemon's tray reaps the GUI process it spawns on a background thread instead of dropping the `Child` handle. (3) The GUI's single-instance module switches its coordination signal from "try to connect, then race to bind" to an exclusive `flock` on a dedicated lock file — the lock, not the socket, becomes the single source of truth for "is a primary instance alive," which removes the race entirely (a `flock` is released atomically by the kernel the instant its holder dies, for any reason) and also makes stale-socket cleanup unconditionally safe (holding the lock proves nothing else can be listening). (4) A `.desktop` file, installed by `install.sh` with `install.sh`'s own `$HOME` substituted in at install time (`.desktop` `Exec=` lines are not shell-expanded, so a literal absolute path has to be written in).

**Tech Stack:** No new dependencies for tasks 1, 2, 4. Task 3 adds `fs4` (cross-platform advisory file locking) to `gui/Cargo.toml`.

## Global Constraints

- Every runtime file path is resolved through `wallpaper_core::config`'s shared helper functions (`config_dir()`, `config_path()`, etc.) — never hardcoded. This plan adds one more: `gui_lock_path()`.
- Add third-party dependencies with `cargo add <crate> [--features ...]` rather than hand-writing version numbers into `Cargo.toml`.
- Third-party crate APIs referenced in this plan (`fs4`) were not independently re-verified against the exact version `cargo add` will resolve at implementation time (unlike `slint`/`ksni` earlier in this project, which were checked against installed source). If `fs4::fs_std::FileExt` or its method names don't match, check `cargo doc --open -p fs4` and adapt while keeping the same shape: a `std::fs::File`-based exclusive, non-blocking lock that fails fast (does not block) when already held.
- Every `git commit` step commits only the files listed in that step.
- Every existing test must keep passing; this plan does not remove test coverage, only extends or adapts it where an interface's signature changes.

---

### Task 1: `wallpaper-core` — atomic writes for config.toml and state.toml

**Files:**
- Create: `core/src/fs_util.rs`
- Modify: `core/src/lib.rs`
- Modify: `core/src/config.rs`
- Modify: `core/src/state.rs`

**Interfaces:**
- Produces: `wallpaper_core::fs_util::atomic_write(path: &Path, contents: &str) -> anyhow::Result<()>`.
- Consumes (by `Config::save_to` and `State::save_to`, both modified in this task): the function above.

- [ ] **Step 1: Write the failing tests**

Create `core/src/fs_util.rs`:

```rust
use std::path::Path;

/// Writes `contents` to `path` atomically: writes to a sibling temp file first, then
/// renames it over `path`. A reader can never observe a partially-written or empty
/// file, because `rename` on Linux is atomic at the filesystem level - the file at
/// `path` is always either the complete old content or the complete new content,
/// never a torn mix of both.
///
/// The temp file's name includes this process's PID so two processes writing the
/// same `path` concurrently (e.g. the GUI's "Guardar" and the tray's pause toggle,
/// both targeting config.toml) never write into the same temp file.
pub fn atomic_write(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, contents)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_parent_dir_and_writes_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.toml");

        atomic_write(&path, "hello = 1").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello = 1");
    }

    #[test]
    fn atomic_write_replaces_existing_content_and_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.toml");
        std::fs::write(&path, "old").unwrap();

        atomic_write(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover.is_empty(), "temp file left behind: {leftover:?}");
    }
}
```

Add `pub mod fs_util;` to `core/src/lib.rs`, alongside the existing `pub mod config;`/`pub mod state;` lines.

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p wallpaper-core fs_util::`
Expected: both tests PASS. (There's no separate "write it failing first" step here beyond this — the module didn't exist before this step, so writing it and its tests together and then running them once is this task's equivalent of red-then-green: the function is short enough to be obviously correct from its signature, same rationale the original plan used for `Config`/`State`'s own load/save methods.)

- [ ] **Step 3: Use it from `Config::save_to`**

In `core/src/config.rs`, replace the body of `Config::save_to`:

```rust
pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
    let text = toml::to_string_pretty(self)?;
    crate::fs_util::atomic_write(path, &text)
}
```

(This removes the `create_dir_all` call and the direct `std::fs::write` call from this function — `atomic_write` now does both.)

- [ ] **Step 4: Use it from `State::save_to`**

In `core/src/state.rs`, replace the body of `State::save_to`:

```rust
pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
    let text = toml::to_string_pretty(self)?;
    crate::fs_util::atomic_write(path, &text)
}
```

- [ ] **Step 5: Run the full `wallpaper-core` test suite**

Run: `cargo test -p wallpaper-core`
Expected: every existing test still passes (in particular `config::tests::config_round_trips_through_toml_file` and `state::tests::state_round_trips_through_toml_file`, which exercise `save_to` end-to-end), plus the two new `fs_util::` tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/fs_util.rs core/src/lib.rs core/src/config.rs core/src/state.rs
git commit -m "feat(core): write config.toml/state.toml atomically to avoid torn reads"
```

---

### Task 2: `daemon` — reap the GUI process the tray spawns

**Files:**
- Modify: `daemon/src/tray.rs`

**Interfaces:**
- Produces: `reap_in_background(child: std::process::Child)` (private to `tray.rs`).
- Consumes: nothing new from other tasks.

- [ ] **Step 1: Write the failing test**

Add to `daemon/src/tray.rs`, at the bottom of the file (there is no `#[cfg(test)] mod tests` block in this file yet — create one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reap_in_background_waits_for_the_child_so_it_does_not_stay_a_zombie() {
        let child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();

        reap_in_background(child);

        // give the reaper thread time to call wait()
        std::thread::sleep(Duration::from_millis(300));

        // once a child is reaped, the kernel removes its /proc entry entirely -
        // a zombie (unreaped-but-exited) child would still have one, with State: Z
        let proc_entry_exists = std::path::Path::new(&format!("/proc/{pid}")).exists();
        assert!(!proc_entry_exists, "child process {pid} was not reaped (zombie left behind)");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p wallpaper-changer-daemon tray::tests::`
Expected: compile error — `reap_in_background` is not defined. This is the expected RED.

- [ ] **Step 3: Write the implementation**

In `daemon/src/tray.rs`, add this function (near `open_config_gui`, above the new test module):

```rust
/// Spawns a background thread that waits for `child` to exit, so the OS can reap it
/// instead of leaving a zombie process behind once it exits. `Command::spawn` returns
/// a `Child` that is never awaited otherwise - and on this project's fast path (the
/// GUI's own single-instance guard makes most launches here exit within
/// milliseconds, having only delegated to an already-running instance), that would
/// mean a zombie left behind on every click of "Abrir configuración" while the GUI
/// is already open, for as long as the daemon keeps running.
fn reap_in_background(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}
```

Then change `open_config_gui` to use it. Replace:

```rust
fn open_config_gui() {
    let path = dirs::home_dir()
        .map(|home| home.join(".local/bin/wallpaper-changer-gui"))
        .unwrap_or_else(|| std::path::PathBuf::from("wallpaper-changer-gui"));
    if let Err(e) = std::process::Command::new(path).spawn() {
        eprintln!("tray: failed to launch wallpaper-changer-gui: {e}");
    }
}
```

with:

```rust
fn open_config_gui() {
    let path = dirs::home_dir()
        .map(|home| home.join(".local/bin/wallpaper-changer-gui"))
        .unwrap_or_else(|| std::path::PathBuf::from("wallpaper-changer-gui"));
    match std::process::Command::new(path).spawn() {
        Ok(child) => reap_in_background(child),
        Err(e) => eprintln!("tray: failed to launch wallpaper-changer-gui: {e}"),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wallpaper-changer-daemon tray::tests::`
Expected: `reap_in_background_waits_for_the_child_so_it_does_not_stay_a_zombie` PASSes. If it's flaky (the 300ms wait isn't always enough on a loaded machine), that's a real signal — increase the wait rather than deleting the test, and note it in your report.

- [ ] **Step 5: Run the full daemon test suite**

Run: `cargo test -p wallpaper-changer-daemon`
Expected: all tests pass, including the pre-existing ones untouched by this task.

- [ ] **Step 6: Commit**

```bash
git add daemon/src/tray.rs
git commit -m "fix(daemon): reap the GUI process the tray spawns to avoid zombies"
```

---

### Task 3: `wallpaper-core` + `gui` — flock-based single-instance guard

**Files:**
- Modify: `core/src/config.rs`
- Modify: `gui/Cargo.toml`
- Modify: `gui/src/singleton.rs`
- Modify: `gui/src/main.rs`

**Interfaces:**
- Consumes: nothing from Tasks 1-2.
- Produces:
  - `wallpaper_core::config::gui_lock_path() -> PathBuf`.
  - `gui::singleton::Singleton::Primary(UnixListener, std::fs::File)` — the `File` variant field changes shape from Task 2 of the *original* minimize-to-tray plan; it must be kept alive (bound to a variable, not dropped) for the whole process lifetime, exactly like the `UnixListener` already was.
  - `gui::singleton::claim(socket_path: &Path, lock_path: &Path) -> anyhow::Result<Singleton>` — signature gains a second parameter.

- [ ] **Step 1: Add the `fs4` dependency**

Run:
```bash
cd gui
cargo add fs4
cd ..
```

- [ ] **Step 2: Add the lock path helper**

In `core/src/config.rs`, immediately after `gui_socket_path()`, add:

```rust
pub fn gui_lock_path() -> PathBuf {
    config_dir().join("gui.lock")
}
```

No dedicated test, matching this file's existing convention for these one-line path helpers.

- [ ] **Step 3: Rewrite `claim()` and update the existing tests**

Replace the full contents of `gui/src/singleton.rs` with:

```rust
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use fs4::fs_std::FileExt;

const SHOW_MESSAGE: &[u8; 4] = b"show";

pub enum Singleton {
    Primary(UnixListener, File),
    AlreadyRunning,
}

/// Claims `socket_path` as the single running instance, coordinated through an
/// exclusive, non-blocking `flock` on `lock_path`.
///
/// The lock - not the socket file - is the source of truth for "is a primary
/// instance alive": a `flock` is released by the kernel the instant the process
/// holding it exits, for any reason, including a crash. That removes the race a
/// plain connect-then-bind approach has (two processes launched close enough
/// together can both see "nobody's listening" and both try to become primary), and
/// it also makes stale-socket cleanup unconditionally safe - once this process
/// holds the lock exclusively, nothing else can possibly be listening on
/// `socket_path`, so any file left there is guaranteed dead and safe to remove.
pub fn claim(socket_path: &Path, lock_path: &Path) -> anyhow::Result<Singleton> {
    let lock_file = OpenOptions::new().create(true).write(true).open(lock_path)?;
    if lock_file.try_lock_exclusive().is_err() {
        return Ok(Singleton::AlreadyRunning);
    }

    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    Ok(Singleton::Primary(listener, lock_file))
}

/// Tells the already-running primary instance to show its window.
pub fn notify_running_instance(socket_path: &Path) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(SHOW_MESSAGE)?;
    stream.flush()?;
    Ok(())
}

/// Runs the primary instance's side of the protocol: accepts connections on a
/// background thread for as long as the process lives, and calls `on_show` for
/// each one that sends exactly the expected message. Anything else - a short
/// read, garbage bytes, a connection that closes early, a client that connects and
/// then never writes - is silently dropped.
///
/// This is a single-threaded loop, so it must never block on one client: every
/// accepted stream gets a read timeout, and a failed `accept` pauses briefly rather
/// than spinning at full speed (`incoming()` never ends, so a persistent error such
/// as descriptor exhaustion would otherwise busy-loop this thread forever).
pub fn spawn_accept_loop(listener: UnixListener, on_show: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut buf = [0u8; SHOW_MESSAGE.len()];
            if stream.read_exact(&mut buf).is_ok() && &buf == SHOW_MESSAGE {
                on_show();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    fn paths(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        (dir.join("gui.sock"), dir.join("gui.lock"))
    }

    #[test]
    fn claiming_a_fresh_path_becomes_the_primary_instance() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(_listener, _lock_file) => {}
            Singleton::AlreadyRunning => panic!("expected Primary for a fresh socket path"),
        }
    }

    #[test]
    fn claiming_an_already_claimed_path_detects_the_running_instance() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let _primary = claim(&socket_path, &lock_path).unwrap();

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::AlreadyRunning => {}
            Singleton::Primary(..) => panic!("expected AlreadyRunning while the primary is alive"),
        }
    }

    #[test]
    fn dropping_the_primary_releases_the_lock_for_the_next_claim() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let primary = claim(&socket_path, &lock_path).unwrap();
        drop(primary);

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(..) => {}
            Singleton::AlreadyRunning => {
                panic!("expected Primary after the previous primary's lock was released")
            }
        }
    }

    #[test]
    fn claiming_a_path_with_a_stale_socket_file_recovers_and_becomes_primary() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        // Dropping a `UnixListener` closes the socket but leaves its file on disk -
        // exactly what a crashed primary instance leaves behind. Crucially, its lock
        // is also released by the crash (simulated here by simply never taking it),
        // so a fresh `claim` must recover cleanly.
        let dead = UnixListener::bind(&socket_path).unwrap();
        drop(dead);
        assert!(
            socket_path.exists(),
            "expected the dropped listener to leave a stale socket file behind"
        );

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(..) => {}
            Singleton::AlreadyRunning => {
                panic!("expected Primary after recovering a stale socket file")
            }
        }
    }

    #[test]
    fn notifying_the_primary_reaches_its_accept_loop() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let listener = match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(listener, _lock_file) => listener,
            Singleton::AlreadyRunning => panic!("expected to become the primary instance"),
        };

        let (tx, rx) = channel();
        spawn_accept_loop(listener, move || {
            let _ = tx.send(());
        });

        notify_running_instance(&socket_path).unwrap();

        rx.recv_timeout(Duration::from_secs(5))
            .expect("accept loop did not receive the show notification");
    }

    #[test]
    fn a_short_or_unrecognized_message_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let listener = match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(listener, _lock_file) => listener,
            Singleton::AlreadyRunning => panic!("expected to become the primary instance"),
        };

        let (tx, rx) = channel::<()>();
        spawn_accept_loop(listener, move || {
            let _ = tx.send(());
        });

        let mut stream = UnixStream::connect(&socket_path).unwrap();
        stream.write_all(b"no").unwrap();
        drop(stream);

        match rx.recv_timeout(Duration::from_millis(500)) {
            Err(RecvTimeoutError::Timeout) => {}
            other => panic!("expected no notification for an unrecognized message, got {other:?}"),
        }
    }
}
```

This adds one genuinely new test (`dropping_the_primary_releases_the_lock_for_the_next_claim`, proving the core correctness property this task exists for) and updates every existing test to the new two-argument `claim` and the new `Primary(listener, lock_file)` shape.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wallpaper-changer-gui singleton::`
Expected: all six tests PASS. If `fs4::fs_std::FileExt` doesn't resolve or its method names differ from `try_lock_exclusive`, check `cargo doc --open -p fs4` for the version `cargo add` pulled in and adapt - the required shape is: a `std::fs::File`-based lock, exclusive, non-blocking (fails immediately rather than waiting when already held), released automatically when the `File` is dropped or the process dies.

- [ ] **Step 5: Update `gui/src/main.rs`'s call site**

In `gui/src/main.rs`, replace:

```rust
fn main() -> anyhow::Result<()> {
    let socket_path = gui_socket_path();
    // On a fresh install the config directory doesn't exist yet (it's only created
    // later, by `Config::save`), and binding the socket inside a missing directory
    // fails - which would silently disable single-instance detection for this run.
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = match singleton::claim(&socket_path) {
        Ok(singleton::Singleton::AlreadyRunning) => {
            if let Err(e) = singleton::notify_running_instance(&socket_path) {
                eprintln!("gui: failed to notify the running instance: {e}");
            }
            return Ok(());
        }
        Ok(singleton::Singleton::Primary(listener)) => Some(listener),
        Err(e) => {
            // Single-instance detection is a convenience, not a hard requirement -
            // its failure (e.g. a permissions problem on the config dir) must never
            // block the GUI from opening.
            eprintln!("gui: single-instance detection unavailable, continuing anyway: {e}");
            None
        }
    };
```

with:

```rust
fn main() -> anyhow::Result<()> {
    let socket_path = gui_socket_path();
    let lock_path = gui_lock_path();
    // On a fresh install the config directory doesn't exist yet (it's only created
    // later, by `Config::save`), and binding the socket inside a missing directory
    // fails - which would silently disable single-instance detection for this run.
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
            // Single-instance detection is a convenience, not a hard requirement -
            // its failure (e.g. a permissions problem on the config dir) must never
            // block the GUI from opening.
            eprintln!("gui: single-instance detection unavailable, continuing anyway: {e}");
            (None, None)
        }
    };
```

`_lock_file` is intentionally never read again after this point - its only job is to stay alive (held, not dropped) for the rest of `main()`, which a `let`-bound local does automatically regardless of whether it's later referenced. Do not rename it to something that would make a linter suggest removing it; the leading underscore already tells `cargo build`/clippy this is deliberate.

Also update the import line near the top of the file from:
```rust
use wallpaper_core::config::{change_now_request_path, gui_socket_path, Config, IntervalUnit};
```
to:
```rust
use wallpaper_core::config::{change_now_request_path, gui_lock_path, gui_socket_path, Config, IntervalUnit};
```

No other part of `main()` changes - the `if let Some(listener) = listener { ... spawn_accept_loop ... }` block further down stays exactly as it is, since `listener`'s type (`Option<UnixListener>`) is unchanged.

- [ ] **Step 6: Verify the whole crate builds and its tests pass**

Run: `cargo build -p wallpaper-changer-gui && cargo test -p wallpaper-changer-gui`
Expected: clean build, no warnings, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add core/src/config.rs gui/Cargo.toml gui/src/singleton.rs gui/src/main.rs
git commit -m "fix(gui): replace the single-instance race with an exclusive flock"
```

---

### Task 4: packaging — `.desktop` launcher entry for the GUI

**Files:**
- Create: `packaging/wallpaper-changer-gui.desktop`
- Modify: `install.sh`

**Interfaces:**
- Consumes: the `wallpaper-changer-gui` binary (already built and installed by earlier steps of `install.sh`).
- Produces: a discoverable application-launcher entry. Nothing else depends on this.

- [ ] **Step 1: Write the desktop entry template**

Create `packaging/wallpaper-changer-gui.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Wallpaper Changer
Comment=Configura la rotación automática de fondos de pantalla
Exec=@BINDIR@/wallpaper-changer-gui
Icon=preferences-desktop-wallpaper
Terminal=false
Categories=Settings;DesktopSettings;
```

`@BINDIR@` is a placeholder, not valid `.desktop` syntax on its own - `Exec=` lines are not shell-expanded, so `~` or `$HOME` would be taken literally rather than resolved, and this project already installs the binary to a per-user path (`$HOME/.local/bin`) that can only be known at install time. `install.sh` substitutes the placeholder with the real path in the next step, and the file that actually gets installed on disk always has a literal absolute path in it.

- [ ] **Step 2: Install it from `install.sh`, substituted**

In `install.sh`, immediately after the existing block that copies the systemd service file (the `mkdir -p "$HOME/.config/systemd/user"` / `cp packaging/wallpaper-changer-daemon.service ...` lines) and before the `systemctl --user daemon-reload` line, add:

```bash
mkdir -p "$HOME/.local/share/applications"
sed "s|@BINDIR@|$HOME/.local/bin|" packaging/wallpaper-changer-gui.desktop \
    > "$HOME/.local/share/applications/wallpaper-changer-gui.desktop"
```

- [ ] **Step 3: Verify manually**

Run: `./install.sh`
Expected: install succeeds as before, and `cat ~/.local/share/applications/wallpaper-changer-gui.desktop` shows `Exec=` with your real home directory substituted in (e.g. `Exec=/home/youruser/.local/bin/wallpaper-changer-gui`), not the literal `@BINDIR@` placeholder. Open your desktop's application launcher (KRunner / the Plasma menu) and search "Wallpaper Changer" - the entry should appear and launch the GUI when activated.

- [ ] **Step 4: Commit**

```bash
git add packaging/wallpaper-changer-gui.desktop install.sh
git commit -m "feat(packaging): add a .desktop launcher entry for the GUI"
```

---

### Task 5: End-to-end manual verification

**Files:** none (verification only).

**Interfaces:** none.

- [ ] **Step 1: Reinstall**

Run: `./install.sh` on your KDE Plasma machine, from a clean build of this branch.

- [ ] **Step 2: Verify no torn reads under real concurrent use**

With the daemon running, rapidly toggle pause several times in a row from both the tray and the GUI while watching `journalctl --user -u wallpaper-changer-daemon -f`.
Expected: no `failed to reload config.toml: TOML parse error` lines (the exact failure this project hit for real during an earlier manual test session, before this fix).

- [ ] **Step 3: Verify no zombie processes accumulate**

With the GUI already open (minimized or visible), click "Abrir configuración" from the daemon's tray icon 5-10 times in a row.
Expected: `ps aux | grep wallpaper-changer-gui` shows exactly one process throughout, and `ps aux | grep defunct` (or `ps -eo stat,pid,cmd | grep '^Z'`) shows no zombie `wallpaper-changer-gui` entries accumulating.

- [ ] **Step 4: Verify the `.desktop` entry works**

Open the KDE Plasma application launcher, search "Wallpaper Changer", and launch it from there (not from a terminal).
Expected: the GUI opens normally.

- [ ] **Step 5: Re-run the original minimize-to-tray manual verification**

Since Task 3 changed `singleton.rs`'s public shape, re-run the manual checks from `docs/superpowers/plans/2026-08-01-gui-minimize-to-tray.md`'s Task 5 (close-to-tray, restore, single-instance reuse from both the daemon's tray and a direct relaunch, "Salir", and the stale-socket-recovery scenario via `kill -9`) to confirm nothing regressed.

No commit for this task - it's pure verification. If any step fails, fix the relevant task and re-run that task's own tests before re-attempting this task.
