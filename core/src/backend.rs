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
