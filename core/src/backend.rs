use std::path::Path;

pub trait WallpaperBackend: Send {
    fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()>;
}
