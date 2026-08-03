# XFCE Support (Fase 3) — Design Spec

**Context:** The daemon/GUI currently support KDE Plasma (per-monitor, Fase 1) and GNOME (single shared config, Fase 2 — GNOME has no native per-monitor wallpaper support). This adds XFCE as a third desktop environment, selected automatically at runtime alongside the other two.

**Key research findings (not previously confirmed when this phase was scoped in the roadmap):**
- Unlike GNOME, **XFCE genuinely supports independent per-monitor wallpapers** — `xfconf`'s `xfce4-desktop` channel stores one `last-image` property per monitor, at a path like `/backdrop/screen0/monitorDP-1/workspace0/last-image`. This phase gives XFCE real per-monitor rotation, matching KDE's model, not GNOME's degraded single-shared one.
- XFCE also has a **workspace (virtual desktop) dimension** independent of monitors — a `/backdrop/single-workspace-mode` toggle controls whether one image applies everywhere or each workspace has its own. Property paths are not fixed and must be discovered at runtime (`xfconf-query -c xfce4-desktop -l`).
- The monitor identifier embedded in these paths varies by XFCE version — newer versions use the real output/connector name (e.g. `monitorDP-1`), older versions use a numeric index (`monitor0`) that isn't guaranteed stable across reconnecting monitors in a different order. Per the approved design decision, this project does **not** attempt to detect XFCE's version or reconcile the two schemes — whatever identifier currently appears in xfconf is used as-is.
- `last-image`'s value is a **plain absolute filesystem path**, not a `file://` URI — confirmed via XFCE's own documented examples, differing from both `KdePlasmaBackend`'s D-Bus script and `GnomeBackend`'s gsettings URI.

**No live XFCE environment was available to verify this feature against**, same situation as GNOME (Fase 2). Every task's tests are unit-level only, documented in `ROADMAP.md` once implemented.

---

## Monitor identification and enumeration

A new function, `wallpaper_core::monitors::list_xfce_monitors() -> anyhow::Result<Vec<Monitor>>` (added to the existing `core/src/monitors.rs`, alongside `list_connected_monitors`/`list_gnome_monitors`):

1. Runs `xfconf-query -c xfce4-desktop -l`, which lists every property path currently defined in that channel (no fixed paths assumed — this is the "discover at runtime" approach the original roadmap scoping called for).
2. Matches each line against a pattern shaped like `/backdrop/screen{N}/{monitor-id}/workspace{N}/last-image` and extracts the unique `{monitor-id}` segments — each becomes one connected monitor.
3. Builds one `Monitor` per unique `{monitor-id}`: `uuid` and `connector` are both set to that literal identifier string (no separate stable-UUID source exists for XFCE, unlike KDE's `kwinoutputconfig.json` — using the xfconf-reported identifier directly is the approved, simpler choice over attempting XFCE-version detection). `x`/`y` are `0` (XFCE's per-monitor property paths are already monitor-specific by construction — unlike `KdePlasmaBackend`, there's no position-based correlation step needed here, since there's no equivalent of Plasma's index-only `desktops()` to correlate against). `is_primary` is `true` for whichever `{monitor-id}` sorts first alphabetically (XFCE's xfconf schema has no "primary monitor" concept to read, unlike KDE's `priority`; this is used only for Fase 1's existing "a new monitor copies the primary's settings" behavior, not for any positional targeting).

**Known, documented limitation (approved, not fixed by this phase):** a monitor XFCE's own `xfdesktop` process has never written a `last-image` property for (e.g. a monitor connected for the first time, before the user has opened XFCE's own Appearance/Desktop settings) will not appear in this list, since there is no independent monitor-detection tool (e.g. `xrandr`) cross-checked against xfconf. This needs live-hardware verification once available; if it turns out to be a real practical problem, addressing it (most likely via `xrandr` as a second monitor-listing source) is separate follow-up work, not part of this phase.

## `XfceBackend`

New file `core/src/xfce_backend.rs`, implementing the same `WallpaperBackend` trait as `KdePlasmaBackend`/`GnomeBackend`:

- `set_wallpaper(&self, _all_monitors: &[Monitor], target: &Monitor, path: &Path)` ignores `all_monitors` entirely (no position correlation needed — see above).
- Re-runs `xfconf-query -c xfce4-desktop -l`, filters for every property path matching `/backdrop/screen{N}/{target.uuid}/workspace{N}/last-image` (i.e. every workspace entry that currently exists for this specific monitor).
- For each matching property, runs `xfconf-query -c xfce4-desktop -p <property> -s <plain-path>` via `std::process::Command` with each argument passed separately (never a shell string — matching `GnomeBackend`'s no-injection-surface property) — `<plain-path>` is `path.display()`'s output directly, with **no `file://` prefix**, per the confirmed property format above.
- If zero matching properties exist for `target.uuid` (the "XFCE never touched this monitor" gap from the enumeration section), writes to a single constructed fallback path, `/backdrop/screen0/{target.uuid}/workspace0/last-image`, as a best-effort default rather than silently doing nothing.
- Writing to every currently-existing `workspaceN/last-image` entry for the target monitor (not just `workspace0`) means the rotated wallpaper displays correctly regardless of which virtual desktop the user is currently viewing — the approved design decision for handling XFCE's independent per-workspace wallpaper capability, treating "which workspace" as an XFCE implementation detail this app overrides uniformly rather than a second dimension it models separately (this app rotates per *physical monitor*, matching Fase 1's model, not per monitor-workspace pair).

## Desktop-environment detection and integration

`wallpaper_core::desktop::DesktopEnvironment` (from Fase 2) gains a third variant, `Xfce`, detected from an `"XFCE"` segment in `$XDG_CURRENT_DESKTOP` (same colon-splitting logic already handling `"KDE"`/`"GNOME"`).

- `daemon/src/main.rs`'s `select_backend` gains one match arm: `Xfce => (Box::new(XfceBackend), list_xfce_monitors)`.
- `gui/src/main.rs`'s `monitor_source` gains one match arm: `Some(Xfce) => (list_xfce_monitors, false)` — the existing `bool` (originally meaning "show GNOME's single shared-desktop label") is already correct as `false` for XFCE, since XFCE gets real per-monitor labels exactly like KDE; no new parameter or GUI-visible behavior needed beyond this one match arm.
- `core/src/config.rs`'s `Config::migration_list_monitors()` (added during Fase 2's final-review fix, specifically to prevent a legacy-config migration from silently breaking on a non-KDE desktop) gains the matching `Some(Xfce) => list_xfce_monitors` arm.
- `daemon/src/tray.rs` needs **no code changes at all** — it already receives whichever `list_monitors` function the daemon selected (threaded through as a parameter since the Fase 2 final-review fix), so XFCE's tray "Pausar/Reanudar"/"Cambiar ahora" (acting on the primary monitor, same stopgap as KDE/GNOME) work correctly the moment `select_backend` knows about `Xfce`.
- The daemon's existing 30-second hot-plug poll (`Engine::update_monitors`) and the GUI's existing 5-second poll both work unchanged, since `list_xfce_monitors()` re-enumerates fresh from `xfconf-query` on every call, exactly like `list_connected_monitors()` does via `kscreen-doctor`.
- An unrecognized/unset desktop still logs a clear error and exits (daemon) / degrades to an empty dropdown (GUI) — unchanged from Fase 2, `Xfce` is simply a third recognized case alongside `Kde`/`Gnome`.

## Error handling

`xfconf-query` missing or failing (binary not installed, XFCE's own D-Bus/xfconfd not running) is treated exactly like a `gsettings`/`kscreen-doctor` failure elsewhere in this project: logged via `eprintln!`, the affected operation returns `Err`/an empty list, and the daemon/GUI do not crash.

## Testing

All unit-level, no live XFCE session available:
- `list_xfce_monitors`'s property-path parsing: given sample `xfconf-query -c xfce4-desktop -l` output (multiple monitors, multiple workspaces per monitor, and non-`last-image` properties like `image-path`/`color-style` that must be ignored), confirm the correct unique monitor set and `is_primary` assignment.
- `XfceBackend`'s command construction: a pure helper (matching `kde_backend.rs`'s `build_wallpaper_script`/`GnomeBackend`'s `gsettings_args` precedent) proving the plain-path (no `file://`) format, and that it targets every existing `workspaceN/last-image` property for the given monitor, not just `workspace0`.
- `DesktopEnvironment::Xfce` detection, mirroring the existing KDE/GNOME detection tests.
- `select_backend`/`monitor_source`/`migration_list_monitors`'s new `Xfce` arms, using the same fn-pointer-identity comparison pattern already established for `Kde`/`Gnome`.

## Out of scope

- Cross-checking xfconf-derived monitor identifiers against an independent tool like `xrandr` — the documented limitation above is accepted for this phase.
- XFCE version detection to prefer connector names over numeric indices — the approved design uses whatever xfconf currently reports, unconditionally.
- Any live verification on a real XFCE session — none was available; this phase merges on unit tests alone, same as GNOME.
