# GNOME Backend Docker Verification — Design Spec

**Context:** Fase 2 (GNOME support, merged) shipped with unit tests only — no live GNOME session was available anywhere in this development environment, so `core/src/gnome_backend.rs`'s actual `std::process::Command::new("gsettings")` invocation has never run against a real `gsettings`/dconf stack. This is a one-off manual verification task, not a new project feature: confirm the real code talks to real `gsettings` correctly, close that gap, and leave no permanent trace in the repository.

**Goal:** Prove that `GnomeBackend::set_wallpaper` — the actual compiled code, not a hand-typed approximation of what it does — successfully sets `org.gnome.desktop.background`'s `picture-uri` key via a real `gsettings`/dconf stack, and that the value can be read back correctly.

**Out of scope:** Full GNOME Shell rendering, visual wallpaper confirmation, the tray icon's StatusNotifierItem behavior, and the daemon's full config/event-loop machinery. None of these need a display server or window manager — `gsettings`/dconf work over just a D-Bus session bus plus the installed schema, which is all this verification sets up.

## Approach

A Docker container, built and run once from the session's scratchpad directory (not committed to the repository — this is disposable tooling for a single verification, not permanent test infrastructure, per the approved design).

**Image:** `rust:slim-bookworm` (or the closest current Debian-based Rust image), with `dbus`, `gsettings-desktop-schemas`, and `dconf-gsettings-backend` installed via `apt-get`. These three packages are the complete real-world dependency set for `gsettings`/dconf to function — no GNOME Shell, no display server, no window manager. The Rust toolchain in the base image builds the actual `wallpaper-core` crate from the repository's real source (copied or bind-mounted into the container), so the code under test is byte-for-byte what's already merged on `master`, not a reimplementation.

**Code under test:** A temporary example, `core/examples/gnome_smoke_test.rs`, added just for this verification and deleted afterward (never committed). It constructs a `wallpaper_core::monitors::Monitor` with arbitrary field values (irrelevant — `GnomeBackend` ignores both `all_monitors` and `target`, per its own doc comment) and a `GnomeBackend`, then calls `set_wallpaper(&[], &monitor, &test_image_path)` directly — the exact same trait method the daemon calls in production — against a small placeholder image file created inside the container. It prints whether the call succeeded (its `anyhow::Result` surfaces any `gsettings` failure directly).

**Running it:** `gsettings`/dconf need a working D-Bus session bus to operate; the standard way to get one without a full desktop session is `dbus-run-session -- <command>` (a well-established pattern for testing GTK/GLib-based tooling in CI-like environments). Inside the container: `dbus-run-session -- cargo run --example gnome_smoke_test -p wallpaper-core`, which runs the real code under a real (if minimal) D-Bus session. Verification of the actual persisted value happens as a separate `gsettings get org.gnome.desktop.background picture-uri` call — this can run under an entirely separate `dbus-run-session` invocation (a fresh, different bus instance) and still see the correct value, because dconf's actual storage is a binary database file under `$HOME`, not something tied to any particular bus's lifetime; the bus is only the write path, not the storage.

**Success criteria:** The example's `set_wallpaper` call reports success (no error printed), and the follow-up `gsettings get` prints a `file://` URI matching the test image's path inside the container.

**Cleanup:** After verification, `core/examples/gnome_smoke_test.rs` is deleted from the working tree (never committed — `git status` must be clean of it before this task is considered done), and the Docker image/container are removed. Only the *result* of this verification (pass/fail, and anything genuinely learned about `GnomeBackend`'s real-world behavior) persists — as a report to the user, and, if it changes anything about the app itself, as a normal code change through this project's existing brainstorm → spec → plan cycle, not as leftover test scaffolding.

## Testing

This whole task *is* the test — there's no further "testing the test" beyond confirming the success criteria above. If `gsettings set` fails inside the container (e.g. a missing schema, a dconf permission issue specific to running as root in a container), that's itself a real, useful finding to report, not a blocker to work around silently.
