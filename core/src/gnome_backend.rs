use std::path::Path;
use crate::backend::WallpaperBackend;
use crate::monitors::Monitor;

pub struct GnomeBackend;

/// Builds the argument list for one `gsettings set org.gnome.desktop.background
/// <key> file://<path>` invocation, without running anything - kept pure and
/// separate from the actual `Command` so it's directly testable, matching this
/// project's existing split in `kde_backend.rs` between building a script/command and
/// running it.
///
/// Unlike `kde_backend.rs`'s D-Bus script (which embeds the path inside a JavaScript
/// string literal, and therefore needs `escape_js_string`), these arguments are
/// passed straight to `Command::arg` - never through a shell - so there is no
/// injection surface to escape here at all; a path containing quotes, spaces, or any
/// other special character reaches `gsettings` as exactly one argument, verbatim.
fn gsettings_args(key: &str, path: &Path) -> Vec<String> {
    vec![
        "set".to_string(),
        "org.gnome.desktop.background".to_string(),
        key.to_string(),
        format!("file://{}", path.display()),
    ]
}

impl WallpaperBackend for GnomeBackend {
    /// `all_monitors`/`target` are unused: GNOME has exactly one shared wallpaper
    /// setting (see `wallpaper_core::monitors::list_gnome_monitors`'s doc comment),
    /// so every call sets the same two global gsettings keys regardless of which
    /// (synthetic) monitor this was called for.
    fn set_wallpaper(&self, _all_monitors: &[Monitor], _target: &Monitor, path: &Path) -> anyhow::Result<()> {
        // Both the light and dark variants are set to the same image, so the correct
        // wallpaper shows regardless of which GTK theme variant is currently active -
        // this app has no reason to track the user's light/dark preference itself.
        for key in ["picture-uri", "picture-uri-dark"] {
            let status = std::process::Command::new("gsettings")
                .args(gsettings_args(key, path))
                .status()?;
            anyhow::ensure!(status.success(), "gsettings set {key} exited with {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn gsettings_args_builds_the_expected_command_line() {
        let args = gsettings_args("picture-uri", &PathBuf::from("/home/user/a.png"));
        assert_eq!(
            args,
            vec!["set", "org.gnome.desktop.background", "picture-uri", "file:///home/user/a.png"]
        );
    }

    #[test]
    fn gsettings_args_builds_the_dark_variant_with_the_same_path() {
        let args = gsettings_args("picture-uri-dark", &PathBuf::from("/home/user/a.png"));
        assert_eq!(
            args,
            vec!["set", "org.gnome.desktop.background", "picture-uri-dark", "file:///home/user/a.png"]
        );
    }

    #[test]
    fn a_path_with_a_space_reaches_gsettings_as_one_argument() {
        let args = gsettings_args("picture-uri", &PathBuf::from("/home/user/my pictures/a.png"));
        assert_eq!(args[3], "file:///home/user/my pictures/a.png");
    }
}
