# GUI Minimize-to-Tray — Design

**Goal:** The `wallpaper-changer-gui` window should minimize to its own system tray icon instead of exiting when closed, and only one instance should ever be visible at a time.

**Context:** Today, closing the GUI window (the X button) ends the process, via Slint's default window-close behavior. The daemon's own tray icon has an "Abrir configuración" item that unconditionally spawns a new `wallpaper-changer-gui` process (`daemon/src/tray.rs::open_config_gui`), with no awareness of whether one is already running. This spec changes the GUI so closing it hides it to a tray icon instead, and adds single-instance detection so repeated "Abrir configuración" clicks (or manual launches) reuse the existing process instead of spawning duplicates.

## Global Constraints (inherited from the base design)

- The daemon and GUI remain fully decoupled: they communicate only through the shared files under `~/.config/wallpaper-changer/` (`config.toml`, `state.toml`, `change_now_request`), resolved via `wallpaper_core::config`/`wallpaper_core::state` helper functions — never hardcoded paths. This spec adds one more such shared runtime file (`gui.sock`), following the same convention.
- `daemon/src/tray.rs` is **not modified** by this feature. `open_config_gui()` keeps spawning the GUI binary exactly as it does today; the GUI binary itself decides whether to become the visible instance or delegate to an already-running one. `core/` gains exactly one small addition — a `gui_socket_path()` helper in `core/src/config.rs`, alongside the existing `config_path()`/`change_now_request_path()` — following the codebase's established one-helper-per-runtime-file pattern rather than inlining `.join("gui.sock")` at call sites. No other part of `core/` changes.
- Add third-party dependencies with `cargo add <crate> [--features ...]` rather than hand-writing version numbers.
- KDE Plasma only, single monitor only — consistent with the rest of the project.

## Architecture

Three additions inside the `gui` crate only:

1. **Single-instance module** (`gui/src/singleton.rs`) — claims or detects an existing instance via a Unix domain socket.
2. **GUI tray icon** — a `SystemTrayIcon` component declared directly in a new `.slint` file, using Slint 1.17.1's built-in native tray support (`system-tray` Cargo feature, already on by default — no extra dependency needed). Slint's own Linux backend implements this via `ksni` internally, so there is no manual `ksni` usage, no extra OS thread, and no cross-thread `invoke_from_event_loop` plumbing for the menu: `SystemTrayIcon`'s `Menu`/`MenuItem` callbacks are dispatched on the same Slint event loop as the window.
3. **Window close interception** (in `gui/src/main.rs`) — hides the window instead of exiting the process when the user clicks the close button, and keeps the event loop alive via `slint::run_event_loop_until_quit()` instead of the generated `AppWindow::run()` convenience method.

`daemon` and `core` are untouched.

**Deviation from the originally-approved sketch:** the design initially assumed a hand-rolled `ksni`-based tray (mirroring `daemon/src/tray.rs`) on its own OS thread. Researching Slint 1.17.1's actual API for this plan surfaced a native `SystemTrayIcon` component that covers the same requirement with less code and no manual threading — confirmed with the user before writing this plan. The trade-off: Slint's `SystemTrayIcon.icon` is a real `image` (rendered pixels), not a theme icon *name* like `ksni::Tray::icon_name()` — it cannot reuse the daemon's `"preferences-desktop-wallpaper"` string. This spec now bundles a small SVG icon asset instead (see GUI Tray Icon below).

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

A new `gui/ui/tray-icon.slint` file defines a component inheriting `SystemTrayIcon`:

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

A small bundled SVG asset, `gui/ui/icons/tray-icon.svg`, is embedded at compile time via `@image-url` — `SystemTrayIcon.icon` is a real rendered image, not a theme icon name, so it can't reference `"preferences-desktop-wallpaper"` by string like the daemon's `ksni::Tray::icon_name()` does.

`gui/src/main.rs` instantiates both `AppWindow::new()?` and `GuiTray::new()?`, wires `GuiTray`'s `toggle-visibility` callback to show the window if hidden or hide it if shown, and its `quit` callback to `slint::quit_event_loop()`. Both callbacks run on the Slint event loop thread (Slint's own dispatcher hands tray clicks to it), so no `Weak`/`invoke_from_event_loop` indirection is needed there — only the singleton listener thread (a genuine separate OS thread) needs it.

The tray icon is created once at startup and lives for the process's whole lifetime (per the approved design: always present while the GUI runs, not created/destroyed on minimize/restore) — its instance is kept alive by binding it to a variable that outlives the event loop call, exactly like `ui`.

## Window Close Interception and Event Loop

`ui.window().on_close_requested(...)` is wired to hide the window and return `slint::CloseRequestResponse::HideWindow` — actually already the default response, but registering the callback explicitly documents the intent and is where the shared `toggle_visibility`/`hide` helper is called from, keeping window-hide logic in one place. The default `AppWindow::run()` convenience method cannot be used here — it internally runs the event loop configured to quit once the last window closes, which would still end the process the moment the window is hidden. Instead, `main()` calls `ui.show()?`, `tray.show()?`, then `slint::run_event_loop_until_quit()`, and only `slint::quit_event_loop()` (from the tray's "Salir") ends the loop.

Both the close-intercept and the tray menu's "Mostrar/Ocultar ventana" converge on the same small helper (e.g., `fn toggle_visibility(ui: &AppWindow)`, checking `ui.window().is_visible()` to decide whether to call `.show()` or `.hide()`) instead of duplicating the show/hide logic.

## Error Handling

- Socket bind/connect errors beyond the expected "nobody's listening" case (e.g., permission denied, disk full) are logged via `eprintln!`, matching the project's existing convention (`daemon/src/tray.rs`'s `toggle_pause`/`request_change_now` do the same). The GUI must still start and function normally in this case — single-instance detection is a convenience, not a hard requirement, and its failure must never block the app from opening.
- A malformed/short read on the accept-loop's socket (fewer than 4 bytes, or garbage) is treated as "not a recognized command" and ignored; the connection is simply dropped.

## Testing

- `gui/src/singleton.rs` gets inline `#[cfg(test)]` tests using `tempfile::tempdir()` for the socket path (matching `daemon/src/watcher.rs`'s established pattern for filesystem/IPC-adjacent tests):
  - Claiming a fresh path returns `Singleton::Primary`.
  - Claiming the same path a second time (while the first `UnixListener` is still alive) returns `Singleton::AlreadyRunning`.
  - Writing `"show"` to a connected `AlreadyRunning`-detected socket is received by the primary's accept loop (assert on a channel the test's accept-loop stand-in signals, without needing a real Slint window).
- The tray icon and window-hide/show behavior are verified manually (this project has no automated Slint UI tests anywhere — `gui/src/main.rs`'s existing callbacks are the established precedent), as part of the same kind of manual pass Task 12 already used: open the GUI, close it (confirm it minimizes, not exits, and the tray icon appears), click the tray icon's "Mostrar/Ocultar ventana" (confirm it restores, then re-hides), click "Abrir configuración" from the daemon's tray while the GUI is minimized (confirm it restores the existing window rather than opening a second one), click "Salir" from the GUI's own tray icon (confirm the process actually ends and both the window and tray icon disappear).

## Out of Scope

- No settings/toggle to disable close-to-tray behavior — it's always on, per the approved design.
- No visual distinction between the daemon's and GUI's tray icons beyond their tooltips — the two are necessarily different image assets now (theme icon name vs. bundled SVG), but no further polish (e.g., dark/light variants) is in scope.
- No changes to `daemon` or `core` — the daemon's existing "Abrir configuración" continues to work unmodified.
