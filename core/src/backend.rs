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

/// Lets a boxed trait object satisfy the same bound as a concrete backend, so
/// `daemon/src/main.rs` can pick a backend at runtime (KDE vs. GNOME) and still
/// construct one `Engine<Box<dyn WallpaperBackend>>` regardless of which concrete
/// type was chosen - `Engine<B: WallpaperBackend>`'s own code doesn't change at all.
impl WallpaperBackend for Box<dyn WallpaperBackend> {
    fn set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        (**self).set_wallpaper(all_monitors, target, path)
    }
}
