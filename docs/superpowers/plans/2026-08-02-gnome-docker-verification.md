# GNOME Backend Docker Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that `GnomeBackend::set_wallpaper`'s real, compiled code correctly sets and the value is correctly read back via a real `gsettings`/dconf stack running in a Docker container — no live GNOME session was available to verify this any other way.

**Architecture:** A disposable Docker image (Debian-based Rust toolchain + `dbus`/`gsettings-desktop-schemas`/`dconf-gsettings-backend`) built from a clean `git archive` snapshot of the repository — never the real working tree — containing one temporary example binary that calls `GnomeBackend::set_wallpaper` directly. The set and the read-back both run inside the *same* container invocation, under `dbus-run-session` for a working D-Bus session bus.

**Tech Stack:** Docker (already installed on this machine, version 29.7.0), `rust:slim-bookworm` base image, `git archive` for a clean source snapshot.

## Global Constraints

- Nothing about this task is committed to the repository, and nothing is left in the real working tree — the temporary example file, the Dockerfile, and every intermediate artifact live under the session's scratchpad directory, never inside `/home/blackzero/Documentos/GitHub/WallpaperChangerLinux` itself.
- The code under test must be the real, already-merged `GnomeBackend::set_wallpaper` — not a reimplementation or a hand-typed `gsettings` command sequence.
- The `gsettings set` call and the `gsettings get` read-back verification must run inside the *same* `docker run` invocation (the same container's filesystem/session), never two separate `docker run --rm` invocations — a fresh container each time would have no memory of the first one's dconf writes, since `--rm` discards the container's filesystem on exit.
- Docker image name and all scratch paths use a `gnome-backend-verify` prefix, to make cleanup unambiguous.

---

### Task 1: Build, run, and verify

**Files:**
- Create (scratch only, never in the repo): `$SCRATCH/gnome-verify-src/` (a `git archive` snapshot of the repo, with one added file: `core/examples/gnome_smoke_test.rs`), `$SCRATCH/gnome-verify-src/Dockerfile`.

Where `$SCRATCH` is `/tmp/claude-1000/-home-blackzero-Documentos-GitHub-WallpaperChangerLinux/c9e9f884-a1e7-4742-9c9e-1af575bdedd4/scratchpad` (this session's scratchpad directory).

**Interfaces:**
- Consumes: `wallpaper_core::gnome_backend::GnomeBackend` (implements `WallpaperBackend`), `wallpaper_core::monitors::Monitor`, `wallpaper_core::backend::WallpaperBackend` — all already defined and merged on `master`, unchanged by this task.
- Produces: nothing persists — this task's only output is a pass/fail report to the human partner.

- [ ] **Step 1: Export a clean snapshot of the repository's tracked files**

```bash
mkdir -p "$SCRATCH/gnome-verify-src"
cd /home/blackzero/Documentos/GitHub/WallpaperChangerLinux
git archive HEAD | tar -x -C "$SCRATCH/gnome-verify-src"
```

`git archive` exports only tracked files (automatically excludes `target/`, `.git/`, and anything gitignored like `.claude/worktrees/`), so the resulting directory is a clean, minimal Docker build context with no manual exclude-list needed.

- [ ] **Step 2: Add the temporary smoke-test example — to the snapshot, not the real repo**

Create `$SCRATCH/gnome-verify-src/core/examples/gnome_smoke_test.rs`:

```rust
use std::path::PathBuf;
use wallpaper_core::backend::WallpaperBackend;
use wallpaper_core::gnome_backend::GnomeBackend;
use wallpaper_core::monitors::Monitor;

fn main() {
    let image_path = PathBuf::from("/tmp/gnome-verify-test-image.png");
    std::fs::write(&image_path, b"placeholder bytes - only the path matters to gsettings")
        .expect("failed to write placeholder test image");

    // GnomeBackend ignores all_monitors/target entirely (see its own doc comment) -
    // every field here is a throwaway value, none of it reaches gsettings.
    let monitor = Monitor {
        uuid: "smoke-test".to_string(),
        connector: "smoke-test".to_string(),
        is_primary: true,
        x: 0,
        y: 0,
    };

    match GnomeBackend.set_wallpaper(&[], &monitor, &image_path) {
        Ok(()) => println!("SET_OK: {}", image_path.display()),
        Err(e) => {
            eprintln!("SET_FAILED: {e}");
            std::process::exit(1);
        }
    }
}
```

Cargo auto-discovers `examples/*.rs` — no `Cargo.toml` change needed anywhere.

- [ ] **Step 3: Write the Dockerfile**

Create `$SCRATCH/gnome-verify-src/Dockerfile`:

```dockerfile
FROM rust:slim-bookworm

# The complete real-world dependency set for gsettings/dconf to function without a
# display server, window manager, or GNOME Shell: dbus (for dbus-run-session, which
# provides a working D-Bus session bus), gsettings-desktop-schemas (installs and
# compiles the org.gnome.desktop.background schema our code writes to), and
# dconf-gsettings-backend (the actual storage backend GSettings writes through -
# without it, gsettings silently falls back to an in-memory backend that never
# persists, which would make this verification pass for the wrong reason).
RUN apt-get update && apt-get install -y --no-install-recommends \
    dbus \
    gsettings-desktop-schemas \
    dconf-gsettings-backend \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /repo
COPY . /repo

RUN cargo build --example gnome_smoke_test -p wallpaper-core
```

- [ ] **Step 4: Build the image**

```bash
docker build -t gnome-backend-verify "$SCRATCH/gnome-verify-src"
```

Expected: image builds successfully, ending with the `cargo build` step compiling `wallpaper-core` and the example with no errors.

- [ ] **Step 5: Run the set-then-read-back verification**

```bash
docker run --rm gnome-backend-verify \
  dbus-run-session -- bash -c '
    cargo run --quiet --example gnome_smoke_test -p wallpaper-core &&
    echo "---readback---" &&
    gsettings get org.gnome.desktop.background picture-uri
  '
```

Both the `cargo run` (which calls the real `GnomeBackend::set_wallpaper`, which runs `gsettings set`) and the `gsettings get` read-back run inside this one container invocation, under the same `dbus-run-session` bus — satisfying the Global Constraint above.

Expected output:
```
SET_OK: /tmp/gnome-verify-test-image.png
---readback---
'file:///tmp/gnome-verify-test-image.png'
```

If the actual output differs — a schema-not-found error, a permission error, a `gsettings get` value that doesn't match what was set — that is itself the finding this task exists to surface. Report exactly what happened rather than adjusting the Dockerfile or command to force a match; a mismatch here is real information about how `GnomeBackend` behaves against a real `gsettings`/dconf stack; changes to `core/src/gnome_backend.rs` in response to a genuine finding go through this project's normal brainstorm → spec → plan cycle, not as a fix folded into this verification task.

- [ ] **Step 6: Report the result**

Summarize for the human partner: did `SET_OK` print, did the `gsettings get` read-back match the path written, and the exact text of any error if either step failed.

- [ ] **Step 7: Clean up**

```bash
docker image rm gnome-backend-verify
rm -rf "$SCRATCH/gnome-verify-src"
cd /home/blackzero/Documentos/GitHub/WallpaperChangerLinux
git status --short
```

Expected: `git status --short` prints nothing related to this task (it should already be empty, or show only pre-existing unrelated state, since Step 1 never touched the real working tree — this is a final confirmation, not a fix-up).

No commit for this task — nothing produced by it is meant to persist in the repository.
