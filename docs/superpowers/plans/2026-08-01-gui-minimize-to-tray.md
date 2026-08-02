# GUI Minimize-to-Tray Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Closing the `wallpaper-changer-gui` window hides it to its own system tray icon instead of ending the process, and only one instance is ever visible at a time.

**Architecture:** A Unix-domain-socket single-instance guard (`gui/src/singleton.rs`) decides on startup whether this process becomes the visible instance or hands off to an already-running one. The visible instance owns a native Slint `SystemTrayIcon` (`GuiTray`, declared in `gui/ui/tray-icon.slint`) with a two-item menu, and intercepts the window's close request to hide instead of exit, running the event loop via `slint::run_event_loop_until_quit()` so it survives the window being hidden.

**Tech Stack:** Rust, Slint 1.17.1's native `SystemTrayIcon` component (the `system-tray` Cargo feature, already enabled by `slint`'s `default` features — no new dependency), `std::os::unix::net` for the single-instance socket (no new dependency), `tempfile` as a new gui dev-dependency for the socket tests.

## Global Constraints

- The daemon and GUI communicate only through the shared files under `~/.config/wallpaper-changer/`, resolved via `wallpaper_core::config`/`wallpaper_core::state` helper functions — never hardcoded paths.
- `daemon/src/tray.rs` is not modified. `core/` gains exactly one small addition: a `gui_socket_path()` helper in `core/src/config.rs`, following the existing one-helper-per-runtime-file pattern (`config_path()`, `change_now_request_path()`). No other part of `core/` changes.
- Add third-party dependencies with `cargo add <crate> [--features ...]` rather than hand-writing version numbers into `Cargo.toml`.
- KDE Plasma only, single monitor only.
- Every `git commit` step commits only the files listed in that step.
- Third-party/framework APIs referenced in this plan (Slint's `Window`, `SystemTrayIcon`, event-loop functions) were confirmed by reading the actual installed `slint`/`i-slint-core` 1.17.1 source at planning time, but if a version resolved later doesn't match, check `cargo doc --open -p slint` and adapt while keeping the same shape — this is expected integration work, not a sign the task is wrong.

---

### Task 1: `wallpaper-core` — add the GUI's socket path helper

**Files:**
- Modify: `core/src/config.rs`

**Interfaces:**
- Produces: `wallpaper_core::config::gui_socket_path() -> PathBuf`.

- [ ] **Step 1: Add the helper**

In `core/src/config.rs`, immediately after the existing `change_now_request_path()` function, add:

```rust
pub fn gui_socket_path() -> PathBuf {
    config_dir().join("gui.sock")
}
```

No dedicated test is added for this function — it follows the same one-line, untested pattern already established by its neighbors `config_path()` and `change_now_request_path()` in this same file, which are exercised only indirectly (through `Config::load`/`save` and the daemon's watcher tests) rather than with a standalone unit test.

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p wallpaper-core`
Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): add gui_socket_path helper for the GUI's single-instance socket"
```

---

### Task 2: `gui` — single-instance detection module

**Files:**
- Create: `gui/src/singleton.rs`
- Modify: `gui/src/main.rs` (add `mod singleton;` only — this task does not wire it into `main()`)
- Modify: `gui/Cargo.toml`

**Interfaces:**
- Consumes: nothing from earlier tasks (works on plain `Path`s; the caller supplies the path via `wallpaper_core::config::gui_socket_path()` in Task 4).
- Produces:
  - `gui::singleton::Singleton` enum: `Primary(std::os::unix::net::UnixListener)`, `AlreadyRunning`.
  - `gui::singleton::claim(socket_path: &Path) -> anyhow::Result<Singleton>`.
  - `gui::singleton::notify_running_instance(socket_path: &Path) -> anyhow::Result<()>`.
  - `gui::singleton::spawn_accept_loop(listener: UnixListener, on_show: impl Fn() + Send + 'static)`.

- [ ] **Step 1: Add the `tempfile` dev-dependency**

Run:
```bash
cd gui
cargo add tempfile --dev
cd ..
```

- [ ] **Step 2: Write the failing tests**

Create `gui/src/singleton.rs` with only this content for now (no implementation yet — these tests reference types and functions that don't exist, which is the point):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    #[test]
    fn claiming_a_fresh_path_becomes_the_primary_instance() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        match claim(&socket_path).unwrap() {
            Singleton::Primary(_listener) => {}
            Singleton::AlreadyRunning => panic!("expected Primary for a fresh socket path"),
        }
    }

    #[test]
    fn claiming_an_already_claimed_path_detects_the_running_instance() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        let _primary = claim(&socket_path).unwrap();

        match claim(&socket_path).unwrap() {
            Singleton::AlreadyRunning => {}
            Singleton::Primary(_) => panic!("expected AlreadyRunning while the primary is alive"),
        }
    }

    #[test]
    fn notifying_the_primary_reaches_its_accept_loop() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        let listener = match claim(&socket_path).unwrap() {
            Singleton::Primary(listener) => listener,
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
        let socket_path = dir.path().join("gui.sock");

        let listener = match claim(&socket_path).unwrap() {
            Singleton::Primary(listener) => listener,
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

- [ ] **Step 3: Run tests to verify they fail to compile**

Run: `cargo test -p wallpaper-changer-gui singleton::`
Expected: compile error — `claim`, `Singleton`, `spawn_accept_loop`, `notify_running_instance` are not defined. This is the expected RED: the test module references an implementation that doesn't exist yet.

- [ ] **Step 4: Write the implementation**

At the top of `gui/src/singleton.rs`, above the existing `#[cfg(test)]` module, add:

```rust
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

const SHOW_MESSAGE: &[u8; 4] = b"show";

pub enum Singleton {
    Primary(UnixListener),
    AlreadyRunning,
}

/// Claims `socket_path` as the single running instance, or detects that another
/// instance already holds it.
///
/// A stale socket file left behind by a process that didn't exit cleanly (e.g. a
/// crash) would otherwise make every future launch see `AlreadyRunning` forever
/// with nothing actually listening, so a failed connect always clears the path
/// before binding fresh - `UnixListener::bind` fails with `AddrInUse` if a file
/// already exists there.
pub fn claim(socket_path: &Path) -> anyhow::Result<Singleton> {
    if UnixStream::connect(socket_path).is_ok() {
        return Ok(Singleton::AlreadyRunning);
    }

    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    Ok(Singleton::Primary(listener))
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
/// read, garbage bytes, a connection that closes early - is silently dropped.
pub fn spawn_accept_loop(listener: UnixListener, on_show: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else { continue };
            let mut buf = [0u8; SHOW_MESSAGE.len()];
            if stream.read_exact(&mut buf).is_ok() && &buf == SHOW_MESSAGE {
                on_show();
            }
        }
    });
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p wallpaper-changer-gui singleton::`
Expected: all four tests PASS.

- [ ] **Step 6: Wire the module in, without using it yet**

Add `mod singleton;` near the top of `gui/src/main.rs`, right after the existing `slint::include_modules!();` line.

- [ ] **Step 7: Verify the whole crate still builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles. Warnings that `claim`, `notify_running_instance`, `spawn_accept_loop`, and `Singleton`'s variants are never used are expected at this point — Task 4 wires them into `main()` and resolves them.

- [ ] **Step 8: Commit**

```bash
git add gui/Cargo.toml gui/src/singleton.rs gui/src/main.rs
git commit -m "feat(gui): add Unix-socket single-instance detection module"
```

---

### Task 3: `gui` — tray icon asset and Slint component

**Files:**
- Create: `gui/ui/icons/tray-icon.svg`
- Create: `gui/ui/tray-icon.slint`
- Modify: `gui/ui/app-window.slint`

**Interfaces:**
- Produces: a `GuiTray` Slint component (generated Rust type `GuiTray`, reachable via the same `slint::include_modules!()` call `AppWindow` already comes from) with callbacks `toggle-visibility()` and `quit()`.

- [ ] **Step 1: Add the tray icon asset**

Create `gui/ui/icons/tray-icon.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">
  <rect x="2" y="3" width="20" height="16" rx="2" fill="none" stroke="#4a4a4a" stroke-width="2"/>
  <circle cx="8" cy="9" r="2" fill="#4a4a4a"/>
  <path d="M3 17l5-5 4 4 3-3 6 5" fill="none" stroke="#4a4a4a" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
```

This is a simple picture-frame-with-mountains glyph (the generic "image" pictogram), solid dark gray so it stays visible on both light and dark panel backgrounds — no theme-color adaptation, matching the fact that Slint's `SystemTrayIcon.icon` takes rendered pixels, not a themeable icon name.

- [ ] **Step 2: Define the `GuiTray` component**

Create `gui/ui/tray-icon.slint`:

```slint
export component GuiTray inherits SystemTrayIcon {
    icon: @image-url("icons/tray-icon.svg");
    tooltip: "Wallpaper Changer";

    callback toggle-visibility();
    callback quit();

    Menu {
        MenuItem {
            title: "Mostrar/Ocultar ventana";
            activated => { toggle-visibility(); }
        }
        MenuItem {
            title: "Salir";
            activated => { quit(); }
        }
    }
}
```

`SystemTrayIcon`, `Menu`, and `MenuItem` are Slint core builtins (like `Window`), so no `import` statement is needed for them — same as `app-window.slint` never imports `Window`.

- [ ] **Step 3: Re-export `GuiTray` from the file `build.rs` compiles**

`gui/build.rs` only compiles `ui/app-window.slint`, and `slint::include_modules!()` only pulls in the output of that one compiled file. Add a re-export line so `GuiTray` (defined in the *other* file) is still reachable from the single generated Rust module. At the very top of `gui/ui/app-window.slint`, before the existing `import { ... } from "std-widgets.slint";` line, add:

```slint
export { GuiTray } from "tray-icon.slint";
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles. This only proves the `.slint` files compile via `build.rs` — `gui/src/main.rs` doesn't reference `GuiTray` yet (that's Task 4), which is the point of this task. If `SystemTrayIcon`/`Menu`/`MenuItem`/`@image-url` syntax doesn't match what's shown here against the resolved `slint` version, check `docs.slint.dev`'s language reference for that version and adapt, keeping the same shape (a component inheriting `SystemTrayIcon`, an `icon` and `tooltip` property, one `Menu` child with `MenuItem`s calling callbacks).

- [ ] **Step 5: Commit**

```bash
git add gui/ui/icons/tray-icon.svg gui/ui/tray-icon.slint gui/ui/app-window.slint
git commit -m "feat(gui): add native SystemTrayIcon component and its icon asset"
```

---

### Task 4: `gui` — wire single-instance detection, tray icon, and close-to-tray into `main()`

**Files:**
- Modify: `gui/src/main.rs`

**Interfaces:**
- Consumes: `singleton::{Singleton, claim, notify_running_instance, spawn_accept_loop}` (Task 2), `wallpaper_core::config::gui_socket_path` (Task 1), the generated `GuiTray` type (Task 3).
- Produces: the GUI's final `fn main()` — terminal node, nothing depends on its internals further.

- [ ] **Step 1: Replace `gui/src/main.rs`**

Replace the full contents of `gui/src/main.rs` with:

```rust
slint::include_modules!();

mod singleton;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wallpaper_core::config::{change_now_request_path, gui_socket_path, Config, IntervalUnit};
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

/// Refreshes everything the window shows from the on-disk config/state.
///
/// `shown_wallpaper` remembers which image is currently displayed: the underlying
/// wallpaper only changes every few minutes, so decoding it again on every one-second
/// tick would mean a full 4K decode per second for as long as the window is open.
fn refresh_state(ui: &AppWindow, shown_wallpaper: &RefCell<Option<PathBuf>>) {
    // The daemon's tray menu can pause/resume behind our back, so re-read the flag
    // rather than trusting the value the window was started with.
    if let Ok(config) = Config::load() {
        if ui.get_paused() != config.paused {
            ui.set_paused(config.paused);
        }
    }

    let Ok(state) = State::load() else { return };

    let already_shown = shown_wallpaper
        .borrow()
        .as_deref()
        .is_some_and(|shown| shown == state.current_wallpaper.as_path());
    if !already_shown {
        if let Ok(image) = slint::Image::load_from_path(&state.current_wallpaper) {
            ui.set_preview_image(image);
            *shown_wallpaper.borrow_mut() = Some(state.current_wallpaper.clone());
        }
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

/// Shows the window if it's hidden, hides it if it's visible. Shared by the tray
/// menu's "Mostrar/Ocultar ventana" and the window's own close button - by the time
/// a close request fires the window is always visible, so this always hides it there.
fn toggle_visibility(ui: &AppWindow) {
    let window = ui.window();
    if window.is_visible() {
        let _ = window.hide();
    } else {
        let _ = window.show();
    }
}

fn main() -> anyhow::Result<()> {
    let socket_path = gui_socket_path();
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

    let ui = AppWindow::new()?;
    // Kept alive for the whole process lifetime: per Slint's docs, a SystemTrayIcon's
    // icon appears as soon as the instance exists and disappears when it's dropped -
    // there's no explicit show() call for it, unlike the window.
    let tray = GuiTray::new()?;
    let config = Config::load()?;

    ui.set_folder_path(config.folder.display().to_string().into());
    ui.set_interval_value(config.interval_value as i32);
    ui.set_interval_unit_index(unit_to_index(config.interval_unit));
    ui.set_paused(config.paused);

    let shown_wallpaper: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    refresh_state(&ui, &shown_wallpaper);

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
                // Only the fields this window owns are written back. `paused` is owned by
                // the pause toggle (here and in the daemon's tray), so it is carried over
                // from the freshly-loaded file - otherwise saving here would silently undo
                // a pause set from the tray while this window was open.
                let Ok(mut config) = Config::load() else { return };
                config.folder = PathBuf::from(ui.get_folder_path().to_string());
                config.interval_value = ui.get_interval_value() as u64;
                config.interval_unit = index_to_unit(ui.get_interval_unit_index());
                let _ = config.save();
            }
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
            // `spawn_accept_loop`'s callback runs on its own OS thread, not the Slint
            // event loop thread, so touching `ui` has to be scheduled back onto it.
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_handle.upgrade() {
                    let _ = ui.window().show();
                }
            });
        });
    }

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let shown_wallpaper = shown_wallpaper.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    refresh_state(&ui, &shown_wallpaper);
                }
            },
        );
    }

    // Not `ui.run()`: that convenience method runs the event loop configured to quit
    // as soon as the last window closes, which would end the process the moment the
    // window is hidden. `run_event_loop_until_quit` only stops at `quit_event_loop()`
    // (wired to the tray's "Salir" above), so hiding the window just leaves the tray
    // icon behind.
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p wallpaper-changer-gui`
Expected: compiles cleanly, no warnings about unused `singleton` items (all four are now used).

If `Window::is_visible`, `Window::on_close_requested`, `slint::CloseRequestResponse`, `slint::run_event_loop_until_quit`, `slint::quit_event_loop`, or `slint::invoke_from_event_loop` don't match the resolved `slint` version's actual signatures, check `cargo doc --open -p slint` and adapt while keeping the same shape (a close-request callback returning a response enum; a variant meaning "hide instead of closing"; a way to keep the event loop alive past the last window closing; a way to end it on demand; a way to safely touch UI state from a non-Slint thread).

- [ ] **Step 3: Commit**

```bash
git add gui/src/main.rs
git commit -m "feat(gui): minimize to tray on close, single-instance aware"
```

---

### Task 5: End-to-end manual verification on real KDE Plasma

**Files:** none (verification only).

**Interfaces:** none — this task validates Tasks 1-4 together against a real desktop session, the same way the base plan's Task 14 validated the whole daemon+GUI system.

- [ ] **Step 1: Rebuild and reinstall**

Run: `./install.sh` (rebuilds and reinstalls the daemon; the GUI binary isn't installed by that script today — build it directly instead: `cargo build --release -p wallpaper-changer-gui` and run `target/release/wallpaper-changer-gui` for the steps below, or copy it to `~/.local/bin/wallpaper-changer-gui` the same way `install.sh` does for the other two binaries).

- [ ] **Step 2: Verify minimize-on-close**

Launch the GUI.
Expected: a new tray icon for the GUI appears (distinct from the daemon's existing one) as soon as the window opens. If it does not, `GuiTray`'s icon may need an explicit `tray.show()?;` call right after `let tray = GuiTray::new()?;` in `gui/src/main.rs` (Task 4's plan text assumes it appears automatically per Slint's docs — treat this as the same kind of version-drift adaptation called out in the Global Constraints if it turns out not to hold for the resolved `slint` version).

Click the window's close button (X).
Expected: the window disappears, the process keeps running (check with `pgrep -a wallpaper-changer-gui`), and the tray icon from above is still present.

- [ ] **Step 3: Verify restore from the tray**

Click the GUI's tray icon and choose "Mostrar/Ocultar ventana".
Expected: the window reappears with its previous state intact (folder, interval, etc.). Click the same menu item again.
Expected: the window hides again (this is a toggle).

- [ ] **Step 4: Verify single-instance reuse from the daemon's tray**

With the GUI minimized (window hidden, tray icon present), open the daemon's tray menu and click "Abrir configuración".
Expected: the existing window is restored — check `pgrep -a wallpaper-changer-gui` shows exactly one process the whole time, not two.

- [ ] **Step 5: Verify a fresh launch also reuses the running instance**

With the GUI still running (visible or minimized), run `~/.local/bin/wallpaper-changer-gui` (or the built binary path) again directly from a terminal.
Expected: the command returns almost immediately (no new window opens), the existing window is shown/raised, and there is still only one `wallpaper-changer-gui` process running.

- [ ] **Step 6: Verify "Salir" actually quits**

From the GUI's own tray icon menu, click "Salir".
Expected: the window (if visible) disappears, the GUI's tray icon disappears, and `pgrep -a wallpaper-changer-gui` shows no process. The daemon and its own tray icon are unaffected.

- [ ] **Step 7: Verify a stale socket doesn't wedge future launches**

With the GUI not running, run: `kill -9 $(pgrep wallpaper-changer-gui)` immediately after launching it (simulating a crash before it can clean up), then launch it again normally.
Expected: the second launch succeeds and shows a window — it does not report "already running" against the dead process's leftover socket file.

No commit for this task — it's pure verification. If any step fails, fix the relevant task and re-run that task's own tests before re-attempting this task.
