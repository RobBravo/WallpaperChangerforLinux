# GNOME Support (Fase 2) — Design Spec

**Context:** The daemon and GUI currently assume KDE Plasma unconditionally —
`daemon/src/main.rs` always constructs a `KdePlasmaBackend`, and
`wallpaper_core::monitors::list_connected_monitors()` shells out to
`kscreen-doctor`/reads `kwinoutputconfig.json`, both KDE-specific. This spec
adds GNOME as a second supported desktop environment, selected automatically
at runtime.

**Key finding from research (not previously known when this phase was
scoped in the roadmap):** GNOME has no native per-monitor wallpaper support.
`gsettings`'s `org.gnome.desktop.background` key applies **one image across
the entire virtual desktop**, spanning all connected monitors — unlike KDE
Plasma, which lets each `Desktop` object have its own image. Third-party
tools (HydraPaper, Dual Wallpaper Engine) work around this by composing
multiple images into one stitched canvas sized to the full monitor layout.
This spec does **not** implement that composition — see "Multi-monitor
behavior under GNOME" below for the chosen, much simpler alternative.

**No live GNOME environment was available to verify this feature against.**
Everything in this phase is unit-tested only; nothing here has been run
against a real GNOME session. This is called out explicitly in the roadmap
once implemented, the same way Fase 1 called out its two-monitor-dependent
gaps.

---

## Multi-monitor behavior under GNOME

Under GNOME, the whole app behaves like a single shared configuration — the
same way this project behaved before Fase 1 added per-monitor support on
KDE. There is exactly **one** wallpaper, one folder, one interval, one pause
state, shared across every connected monitor, because GNOME itself has
nothing more granular to offer without image composition (explicitly out of
scope here).

This is implemented without touching `Config`, `State`, `Engine`, or any of
Fase 1's per-monitor machinery: `wallpaper_core::monitors` gains a new
function, `list_gnome_monitors() -> Vec<Monitor>`, which always returns
exactly one synthetic `Monitor`:

```rust
Monitor {
    uuid: GNOME_SHARED_MONITOR_UUID.to_string(), // a fixed constant, not derived from hardware
    connector: "GNOME".to_string(),
    is_primary: true,
    x: 0,
    y: 0,
}
```

`list_connected_monitors()` (the existing KDE-specific function) keeps its
current name unchanged — it is not renamed to something like
`list_kde_monitors()` for symmetry with the new `list_gnome_monitors()`,
since it's already called from several already-reviewed places (`daemon/`,
`gui/`, and `Config::load()`'s own migration logic in `core/src/config.rs`)
and a purely cosmetic rename isn't worth touching all of them.

Because this one `Monitor` always has `is_primary: true` and is the only
entry in the list, every existing per-monitor code path keeps working
unchanged:

- `Config::for_new_monitor`/`Config::monitor` — operate on this single UUID
  like any other.
- `Engine`'s per-UUID queues/deadlines — degenerate to exactly one queue,
  one deadline, which is exactly the desired "one shared rotation" behavior.
- `daemon/src/tray.rs`'s "act on the primary monitor" stopgap (from Fase 1)
  — keeps working with zero changes, since the one GNOME monitor entry is
  always primary.
- The GUI's monitor-selector `ComboBox` — shows exactly one entry. Its label
  is `"Todos los monitores"` instead of the KDE-style `"Monitor 1 (LVDS-1)"`
  format when running under GNOME (a small conditional in
  `gui/src/main.rs`'s `monitor_label` call site), since "Monitor 1" would
  misleadingly imply one physical screen.

## Desktop-environment detection

Both `daemon/src/main.rs` (to pick a backend + monitor-listing function) and
`gui/src/main.rs` (to pick the same monitor-listing function, and the
GNOME-specific dropdown label) need to know the current desktop environment
— so this lives once in `wallpaper_core`, in a new module
`core/src/desktop.rs`, not duplicated in either binary:

```rust
pub enum DesktopEnvironment {
    Kde,
    Gnome,
}

pub fn detect_desktop_environment() -> Option<DesktopEnvironment> {
    let value = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if value.split(':').any(|part| part.eq_ignore_ascii_case("KDE")) {
        Some(DesktopEnvironment::Kde)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")) {
        Some(DesktopEnvironment::Gnome)
    } else {
        None
    }
}
```

It reads `$XDG_CURRENT_DESKTOP` and splits it on `:` (some distros report
compound values like `ubuntu:GNOME` or `budgie:GNOME`, not a bare
`"GNOME"`). An unrecognized or unset value is **not** silently treated as
KDE (today's implicit behavior, which this removes).

`daemon/src/main.rs` matches on the result: `Kde` constructs
`KdePlasmaBackend` + calls `list_connected_monitors()` (unchanged), `Gnome`
constructs `GnomeBackend` + calls `list_gnome_monitors()`, and `None` logs a
clear error — `"desktop environment '{value}' is not supported - this app
supports KDE Plasma and GNOME"` — and exits with a non-zero status. This is
treated as a wrong-install-target condition, not a transient one: unlike a
malformed `config.toml` (which self-heals once the user fixes the file),
there is nothing to retry here, so unlike that case this does exit rather
than run in a degraded loop. Under systemd's `Restart=on-failure` this will
restart and immediately fail again, which is acceptable — it surfaces the
problem clearly in `journalctl` rather than papering over it.

`gui/src/main.rs` calls the same `detect_desktop_environment()` once at
startup and stores the result alongside the existing monitor-list state: on
`Gnome`, call `list_gnome_monitors()` instead of `list_connected_monitors()`
everywhere the GUI currently does (both at startup and in the 5-second
`refresh_monitor_list` poll from Fase 1), and use the `"Todos los
monitores"` label instead of `monitor_label`'s KDE-style formatting. On
`None` (unrecognized desktop), the GUI degrades the same way it already does
today when `list_connected_monitors()` fails or returns nothing: an empty
dropdown, no crash — the daemon is the one that refuses to start in that
case, not the GUI, since a user might reasonably want to at least inspect
settings even on an unsupported desktop.

### `Engine`'s backend type

`Engine<B: WallpaperBackend>` is currently generic over one concrete backend
type, fixed at compile time via the type parameter — `main()` can no longer
know `B` at compile time once backend selection happens at runtime. The
fix: box the chosen backend as `Box<dyn WallpaperBackend>` and add a blanket
impl so the box itself satisfies the trait bound, forwarding to the boxed
value:

```rust
impl WallpaperBackend for Box<dyn WallpaperBackend> {
    fn set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        (**self).set_wallpaper(all_monitors, target, path)
    }
}
```

`main()` then constructs `Engine<Box<dyn WallpaperBackend>>` regardless of
which concrete backend was chosen. No other part of `Engine`'s logic
changes — this is a type-level change only, not a behavioral one.

## `GnomeBackend`

New file `core/src/gnome_backend.rs`, implementing the same
`WallpaperBackend` trait `core/src/backend.rs` already defines:

```rust
pub struct GnomeBackend;

impl WallpaperBackend for GnomeBackend {
    fn set_wallpaper(&self, _all_monitors: &[Monitor], _target: &Monitor, path: &Path) -> anyhow::Result<()> {
        let uri = format!("file://{}", path.display());
        run_gsettings(&["set", "org.gnome.desktop.background", "picture-uri", &uri])?;
        run_gsettings(&["set", "org.gnome.desktop.background", "picture-uri-dark", &uri])?;
        Ok(())
    }
}
```

`all_monitors`/`target` are ignored — irrelevant under the single-shared-
wallpaper model above; every call sets the one global key. Both
`picture-uri` and `picture-uri-dark` (the GNOME 42+ dark-theme variant) are
set to the same image, so the wallpaper is correct regardless of which GTK
theme variant is currently active, without this app needing to detect or
track the user's light/dark preference itself.

Implemented via `std::process::Command::new("gsettings")` with each argument
passed as a separate `Command::arg()` (not a shell string) — so, unlike
`kde_backend.rs`'s D-Bus script (which embeds the path inside a JavaScript
string literal and therefore needs `escape_js_string`), there is no
shell/script injection surface here at all: arguments go straight to
`execve`, never through a shell that could reinterpret special characters.

`picture-options` (GNOME's scaling-mode key: zoom/scaled/stretched/etc.) is
deliberately left untouched — this app's job is rotating which image is
shown, not reconfiguring how GNOME displays it. A user's existing scaling
preference, set through GNOME's own Settings app, is preserved.

A `gsettings` invocation failing (binary missing, or the command itself
errors) is logged and does not crash the daemon, matching
`KdePlasmaBackend`'s existing D-Bus-failure handling.

## Testing

All unit-level, no live GNOME session available:

- `list_gnome_monitors()` always returns the one fixed synthetic `Monitor`
  with the correct constant UUID, `is_primary: true`.
- `detect_desktop_environment()` against realistic `$XDG_CURRENT_DESKTOP`
  values: `"GNOME"`, `"ubuntu:GNOME"`, `"budgie:GNOME"`, `"KDE"`, `"XFCE"`,
  empty string, and a value containing neither KDE nor GNOME anywhere in its
  colon-separated parts.
- `GnomeBackend`'s command-argument construction: a pure function (e.g.
  `gsettings_args(key: &str, path: &Path) -> Vec<String>`) builds the
  argument list without spawning anything, mirroring `kde_backend.rs`'s
  existing split between `build_wallpaper_script` (pure, tested directly)
  and the actual D-Bus call. Tests confirm the exact
  `["set", "org.gnome.desktop.background", "picture-uri", "file://<path>"]`
  arguments (and the `-dark` variant), including a path containing spaces
  (works fine as a `Command::arg()`, unlike a shell string).
- The GUI's GNOME-specific monitor label (`"Todos los monitores"`).

## Out of scope

- Image composition for genuinely distinct per-monitor wallpapers under
  GNOME (the HydraPaper-style approach) — rejected in favor of the
  single-shared-wallpaper model above, per the approved design decision.
- Any live verification on a real GNOME session — none was available; this
  phase merges on unit tests alone, documented as an accepted gap in
  `ROADMAP.md`.
- Desktop-environment-specific packaging (separate builds per DE) — runtime
  detection was chosen instead, so this remains a single binary.
- Cinnamon, Budgie, or other GNOME-Shell-adjacent desktops that also expose
  `org.gnome.desktop.background` — only `GNOME` itself is detected and
  routed to `GnomeBackend`; a future phase could extend the detection list
  if there's demand, but this phase doesn't claim support for them.
