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

/// Builds the argument list for one `xfconf-query -c xfce4-desktop ...` write, without
/// running anything - kept pure and separate from the actual `Command` so it's
/// directly testable, matching this project's existing split in `gnome_backend.rs`'s
/// `gsettings_args` between building a command's arguments and running it.
///
/// `is_fallback` must be `true` only when `property` is the freshly-synthesized
/// `fallback_property_for_monitor` path rather than one already returned by
/// `last_image_properties_for_monitor`'s `-l` listing. `xfconf-query -s` refuses to
/// write a property that doesn't already exist in the channel unless also given
/// `-n`/`--create` and `-t`/`--type` - confirmed against XFCE's own documented CLI
/// behavior - so only the fallback case needs those two extra flags; a property found
/// via `-l` is already known to exist and must NOT be passed `-n` (creating an
/// already-existing property is not what `-n` is for and needlessly changes the
/// command XFCE itself would have used).
fn xfconf_write_args(property: &str, is_fallback: bool) -> Vec<String> {
    let mut args = vec!["-c".to_string(), "xfce4-desktop".to_string(), "-p".to_string(), property.to_string()];
    if is_fallback {
        args.push("-n".to_string());
        args.push("-t".to_string());
        args.push("string".to_string());
    }
    args.push("-s".to_string());
    args
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
        // Tracks whether `properties` holds already-existing paths (found via `-l`) or
        // the single freshly-synthesized fallback path pushed below - `xfconf_write_args`
        // needs to know which, since only the fallback path requires `-n`/`-t` to be
        // creatable at all.
        let is_fallback = properties.is_empty();
        if is_fallback {
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
                .args(xfconf_write_args(&property, is_fallback))
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

    #[test]
    fn xfconf_write_args_only_adds_create_flags_for_the_fallback_case() {
        let property = "/backdrop/screen0/monitorDP-1/workspace0/last-image";

        let normal_args = xfconf_write_args(property, false);
        let fallback_args = xfconf_write_args(property, true);

        assert_eq!(normal_args, vec!["-c", "xfce4-desktop", "-p", property, "-s"]);
        assert_eq!(
            fallback_args,
            vec!["-c", "xfce4-desktop", "-p", property, "-n", "-t", "string", "-s"]
        );
        assert_ne!(normal_args, fallback_args, "the fallback write must differ from the normal write");
    }
}
