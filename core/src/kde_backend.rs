use std::path::Path;
use crate::backend::WallpaperBackend;

pub struct KdePlasmaBackend;

fn build_wallpaper_script(path: &Path) -> String {
    format!(
        r#"var allDesktops = desktops();
for (i = 0; i < allDesktops.length; i++) {{
    d = allDesktops[i];
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
    d.writeConfig("Image", "file://{}");
}}"#,
        path.display()
    )
}

impl WallpaperBackend for KdePlasmaBackend {
    fn set_wallpaper(&self, path: &Path) -> anyhow::Result<()> {
        let script = build_wallpaper_script(path);
        let connection = zbus::blocking::Connection::session()?;
        connection.call_method(
            Some("org.kde.plasmashell"),
            "/PlasmaShell",
            Some("org.kde.PlasmaShell"),
            "evaluateScript",
            &(script,),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn script_embeds_the_image_path_as_a_file_url() {
        let script = build_wallpaper_script(&PathBuf::from("/home/user/Pictures/a.png"));
        assert!(script.contains(r#"file:///home/user/Pictures/a.png"#));
        assert!(script.contains(r#"wallpaperPlugin = "org.kde.image""#));
    }
}
