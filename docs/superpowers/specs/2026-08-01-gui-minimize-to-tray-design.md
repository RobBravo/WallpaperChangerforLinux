# GUI Minimize-to-Tray — Design

**Goal:** The `wallpaper-changer-gui` window should minimize to its own system tray icon instead of exiting when closed, and only one instance should ever be visible at a time.

**Context:** Today, closing the GUI window (the X button) ends the process, via Slint's default window-close behavior. The daemon's own tray icon has an "Abrir configuración" item that unconditionally spawns a new `wallpaper-changer-gui` process (`daemon/src/tray.rs::open_config_gui`), with no awareness of whether one is already running. This spec changes the GUI so closing it hides it to a tray icon instead, and adds single-instance detection so repeated "Abrir configuración" clicks (or manual launches) reuse the existing process instead of spawning duplicates.

## Global Constraints (inherited from the base design)

- The daemon and GUI remain fully decoupled: they communicate only through the shared files under `~/.config/wallpaper-changer/` (`config.toml`, `state.toml`, `change_now_request`), resolved via `wallpaper_core::config`/`wallpaper_core::state` helper functions — never hardcoded paths. This spec adds one more such shared runtime file (`gui.sock`), following the same convention.
- `daemon/src/tray.rs` and `core/` are **not modified** by this feature. `open_config_gui()` keeps spawning the GUI binary exactly as it does today; the GUI binary itself decides whether to become the visible instance or delegate to an already-running one.
- Add third-party dependencies with `cargo add <crate> [--features ...]` rather than hand-writing version numbers.
- KDE Plasma only, single monitor only — consistent with the rest of the project.

## Architecture

Three additions inside the `gui` crate only:

1. **Single-instance module** (`gui/src/singleton.rs`) — claims or detects an existing instance via a Unix domain socket.
2. **GUI tray icon** (`gui/src/tray.rs`) — a `ksni`-based tray icon, structurally similar to `daemon/src/tray.rs`, with a 2-item menu.
3. **Window close interception** (in `gui/src/main.rs`) — hides the window instead of exiting the process when the user clicks the close button.

`daemon` and `core` are untouched.

## Single-Instance Protocol

Socket path: `wallpaper_core::config::config_dir().join("gui.sock")`. A new helper `wallpaper_core::config::gui_socket_path()` is added alongside the existing `config_path()`/`change_now_request_path()` functions, so the path is resolved through the same shared mechanism as every other runtime file (per Global Constraints).

On GUI startup, before building any Slint window:

- **Try to connect** (`UnixStream::connect`) to `gui.sock`.
  - **Success** → another instance is already running as the primary. Write the ASCII bytes `"show"` to the stream, flush, close it, and exit the process immediately (exit code 0). No window is ever constructed.
  - **Failure** (`ConnectionRefused`, `NotFound`, or any other connect error) → this process becomes the primary instance:
    1. Remove any stale socket file left at that path (e.g., from a previous crash that didn't clean up) — `std::fs::remove_file`, ignoring a "not found" error.
    2. Bind a `UnixListener` to that path.
    3. Spawn a background OS thread that loops `accept()`-ing connections. For each connection, read up to a small fixed number of bytes; if the message is `"show"`, call `slint::invoke_from_event_loop` with a closure that upgrades the window's `Weak` handle and shows/raises it. Any other message or a read error is ignored (logged via `eprintln!`, connection dropped).
    4. Continue into normal GUI startup (build `AppWindow`, load config, etc., exactly as today).

The primary instance does not explicitly unlink the socket on clean exit — the "remove stale socket before bind" step on the next startup is the cleanup mechanism, so a crash (which skips any at-exit cleanup) doesn't leave the next launch permanently unable to bind.

**Function shape** (`gui/src/singleton.rs`):

```rust
pub enum Singleton {
    Primary(std::os::unix::net::UnixListener),
    AlreadyRunning,
}

pub fn claim(socket_path: &Path) -> anyhow::Result<Singleton>;
```

`main()` calls `claim(&gui_socket_path())`; on `AlreadyRunning`, it sends `"show"` over a fresh `UnixStream::connect` and returns before touching Slint. On `Primary(listener)`, `main()` spawns the accept-loop thread (moving the listener in) and proceeds.

## GUI Tray Icon

`gui/src/tray.rs` mirrors `daemon/src/tray.rs`'s structure: a unit struct implementing `ksni::Tray`, spawned via `ksni::blocking::TrayMethods::spawn` on its own OS thread (same pattern already established and working in the daemon, including the `default-features = false, features = ["blocking", "async-io"]` dependency configuration that keeps `tokio` out of the dependency tree).

- `icon_name()`: reuses `"preferences-desktop-wallpaper"` (same as the daemon's icon — both represent the same application).
- Menu (2 items):
  - **"Mostrar/Ocultar ventana"** — toggles window visibility via `invoke_from_event_loop` (same cross-thread pattern as the singleton listener thread).
  - **"Salir"** — `std::process::exit(0)`, the only way the GUI process actually terminates once minimized.

The tray icon is created once, from the primary instance's startup path, and lives for the process's whole lifetime (per the approved design: always present while the GUI runs, not created/destroyed on minimize/restore).

## Window Close Interception

Slint's window-close-request callback (exact API name to be confirmed against the installed `slint = "1.17.1"` at implementation time, consistent with how this project has always handled minor API drift in fast-moving crates — see `kde_backend.rs`/`tray.rs`'s precedents) is wired to call `ui.hide()` (or equivalent) and return a "don't close" response, instead of letting the default handler exit the process.

Interaction with the tray icon's "Mostrar/Ocultar ventana": both code paths converge on the same show/hide logic, so a small shared helper (e.g., `fn toggle_visibility(ui: &AppWindow)` or two `show`/`hide` functions) is used by both the close-intercept and the tray menu instead of duplicating the visibility logic.

## Error Handling

- Socket bind/connect errors beyond the expected "nobody's listening" case (e.g., permission denied, disk full) are logged via `eprintln!`, matching the project's existing convention (`daemon/src/tray.rs`'s `toggle_pause`/`request_change_now` do the same). The GUI must still start and function normally in this case — single-instance detection is a convenience, not a hard requirement, and its failure must never block the app from opening.
- A malformed/short read on the accept-loop's socket (fewer than 4 bytes, or garbage) is treated as "not a recognized command" and ignored; the connection is simply dropped.

## Testing

- `gui/src/singleton.rs` gets inline `#[cfg(test)]` tests using `tempfile::tempdir()` for the socket path (matching `daemon/src/watcher.rs`'s established pattern for filesystem/IPC-adjacent tests):
  - Claiming a fresh path returns `Singleton::Primary`.
  - Claiming the same path a second time (while the first `UnixListener` is still alive) returns `Singleton::AlreadyRunning`.
  - Writing `"show"` to a connected `AlreadyRunning`-detected socket is received by the primary's accept loop (assert on a channel the test's accept-loop stand-in signals, without needing a real Slint window).
- The tray icon and window-hide/show behavior are verified manually (this project has no automated Slint UI tests anywhere — `gui/src/main.rs`'s existing callbacks are the established precedent), as part of the same kind of manual pass Task 12 already used: open the GUI, close it (confirm it minimizes, not exits), click the tray icon (confirm it restores), click "Abrir configuración" from the daemon's tray while the GUI is minimized (confirm it restores the existing window rather than opening a second one), click "Salir" from the GUI's own tray icon (confirm the process actually ends).

## Out of Scope

- No settings/toggle to disable close-to-tray behavior — it's always on, per the approved design.
- No visual distinction between the daemon's and GUI's tray icons beyond their tooltips/titles — both reuse the same icon name.
- No changes to `daemon` or `core` — the daemon's existing "Abrir configuración" continues to work unmodified.
