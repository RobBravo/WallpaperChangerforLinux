use std::path::Path;
use crate::backend::WallpaperBackend;
use crate::monitors::Monitor;

pub struct XfceBackend;

/// Lists every `workspaceN/last-image` xfconf property currently defined for
/// `monitor_id`, given the same `-l` listing text `list_xfce_monitors` parses - kept
/// pure (parses text, spawns nothing) and separate from the actual `xfconf-query`
/// calls in `set_wallpaper`, matching this project's established split in
/// `kde_backend.rs`/`gnome_backend.rs` between building a command/script and running
/// it.
fn last_image_properties_for_monitor(listing: &str, monitor_id: &str) -> Vec<String> {
    let expected_monitor_segment = format!("monitor{monitor_id}");
    listing
        .lines()
        .filter(|line| {
            let segments: Vec<&str> = line.split('/').filter(|s| !s.is_empty()).collect();
            matches!(
                segments.as_slice(),
                [backdrop, screen, monitor, workspace, property]
                    if *backdrop == "backdrop"
                        && screen.starts_with("screen")
                        && *monitor == expected_monitor_segment
                        && workspace.starts_with("workspace")
                        && *property == "last-image"
            )
        })
        .map(|line| line.to_string())
        .collect()
}

/// The property to write when `monitor_id` has no existing `last-image` property at
/// all (see `set_wallpaper`'s fallback below).
fn fallback_property_for_monitor(monitor_id: &str) -> String {
    format!("/backdrop/screen0/monitor{monitor_id}/workspace0/last-image")
}

impl WallpaperBackend for XfceBackend {
    /// `all_monitors` is unused: XFCE's xfconf property paths are already
    /// monitor-specific by construction, unlike KDE's position-based correlation
    /// (there's no equivalent of Plasma's index-only `desktops()` to correlate
    /// against here).
    fn set_wallpaper(&self, _all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        let output = std::process::Command::new("xfconf-query")
            .args(["-c", "xfce4-desktop", "-l"])
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "xfconf-query -l exited with {}",
            output.status
        );
        let listing = String::from_utf8(output.stdout)?;

        let mut properties = last_image_properties_for_monitor(&listing, &target.uuid);
        if properties.is_empty() {
            // XFCE has never written a last-image property for this monitor - write a
            // best-effort default rather than silently doing nothing.
            properties.push(fallback_property_for_monitor(&target.uuid));
        }

        for property in properties {
            // last-image's value is a plain absolute filesystem path, not a file://
            // URI (unlike GnomeBackend's gsettings keys) - confirmed against XFCE's
            // own documented examples. `.arg(path)` passes it as a single OS-string
            // argument, never through a shell, so no escaping is needed for quotes,
            // spaces, or any other character a filename might contain.
            let status = std::process::Command::new("xfconf-query")
                .args(["-c", "xfce4-desktop", "-p", &property, "-s"])
                .arg(path)
                .status()?;
            anyhow::ensure!(status.success(), "xfconf-query set {property} exited with {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_image_properties_for_monitor_finds_every_workspace_entry_for_that_monitor_only() {
        let listing = "\
/backdrop/screen0/monitor0/workspace0/last-image
/backdrop/screen0/monitor0/workspace1/last-image
/backdrop/screen0/monitor0/workspace0/color-style
/backdrop/screen0/monitorDP-1/workspace0/last-image
";
        let props = last_image_properties_for_monitor(listing, "0");
        assert_eq!(
            props,
            vec![
                "/backdrop/screen0/monitor0/workspace0/last-image".to_string(),
                "/backdrop/screen0/monitor0/workspace1/last-image".to_string(),
            ]
        );
    }

    #[test]
    fn last_image_properties_for_monitor_returns_empty_for_an_unknown_monitor() {
        let listing = "/backdrop/screen0/monitor0/workspace0/last-image\n";
        assert!(last_image_properties_for_monitor(listing, "DP-1").is_empty());
    }

    #[test]
    fn fallback_property_for_monitor_builds_a_workspace0_path() {
        assert_eq!(
            fallback_property_for_monitor("DP-1"),
            "/backdrop/screen0/monitorDP-1/workspace0/last-image"
        );
    }
}
