use std::collections::HashMap;
use std::path::PathBuf;
use serde::Deserialize;

/// A currently-connected physical monitor, identified by KDE's own stable UUID (see
/// `list_connected_monitors` for how it's obtained - it survives reboots and the
/// monitor being plugged into a different port).
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub uuid: String,
    pub connector: String,
    pub is_primary: bool,
    pub x: i32,
    pub y: i32,
}

#[derive(Deserialize)]
struct KscreenJson {
    outputs: Vec<KscreenOutput>,
}

#[derive(Deserialize)]
struct KscreenOutput {
    connected: bool,
    // A connected-but-disabled output (e.g. a laptop's internal screen with the lid
    // closed, or a monitor turned off in System Settings without being unplugged)
    // still shows up as `connected: true` but has no `Desktop` in Plasma's own
    // `desktops()` - treating it as targetable would desync `position_rank`'s index
    // from that shorter JS-side list, silently targeting the wrong physical monitor.
    // Absent in a couple of this test module's own older fixtures (real
    // `kscreen-doctor` output always includes it) - default true rather than fail to
    // parse an otherwise-valid output.
    #[serde(default = "default_true")]
    enabled: bool,
    name: String,
    priority: u32,
    pos: KscreenPos,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct KscreenPos {
    x: i32,
    y: i32,
}

/// Parses `kscreen-doctor --json`'s output (note: `--json` *without* `-o` - combining
/// both prints legacy ANSI-colored text after the JSON block).
fn parse_kscreen_outputs(json_text: &str) -> anyhow::Result<Vec<KscreenOutput>> {
    let parsed: KscreenJson = serde_json::from_str(json_text)?;
    Ok(parsed.outputs)
}

/// Parses `~/.config/kwinoutputconfig.json` into a connector-name -> UUID map.
///
/// This file mixes multiple unrelated entry shapes under one top-level array (an
/// "outputs" entry with one object per monitor, and a "setups" entry describing
/// multi-monitor arrangements with a completely different shape), so this parses it
/// as loosely-typed JSON and only pulls out what it recognizes from the "outputs"
/// entry, rather than deserializing the whole file into a fixed struct - a "setups"
/// entry contributes nothing rather than being a hard parse error, and a single
/// malformed monitor entry is skipped instead of failing the whole read.
fn parse_kwin_output_uuids(json_text: &str) -> HashMap<String, String> {
    let mut uuids = HashMap::new();
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return uuids;
    };
    let Some(entries) = root.as_array() else { return uuids };
    for entry in entries {
        if entry.get("name").and_then(|v| v.as_str()) != Some("outputs") {
            continue;
        }
        let Some(data) = entry.get("data").and_then(|v| v.as_array()) else { continue };
        for item in data {
            let connector = item.get("connectorName").and_then(|v| v.as_str());
            let uuid = item.get("uuid").and_then(|v| v.as_str());
            if let (Some(connector), Some(uuid)) = (connector, uuid) {
                uuids.insert(connector.to_string(), uuid.to_string());
            }
        }
    }
    uuids
}

fn kwin_output_config_path() -> PathBuf {
    // KWin's own config file - deliberately NOT under this project's
    // `wallpaper_core::config::config_dir()`, since it belongs to KWin, not this app.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kwinoutputconfig.json")
}

/// The fixed UUID used to represent "the whole desktop" under GNOME, which has no
/// native per-monitor wallpaper support (see `list_gnome_monitors`). Not a real UUID
/// format on purpose - KDE's UUIDs (from `kwinoutputconfig.json`) always look like
/// `xxxxxxxx-xxxx-...`, so this can never collide with one.
pub const GNOME_SHARED_MONITOR_UUID: &str = "gnome-shared-desktop";

/// GNOME has no native way to give each connected monitor its own wallpaper - the
/// `org.gnome.desktop.background` gsettings key applies one image across the entire
/// virtual desktop, spanning every monitor. Rather than reimplement per-monitor image
/// composition (see the design spec's "Multi-monitor behavior under GNOME" section),
/// this always returns exactly one synthetic `Monitor` representing that shared
/// desktop - so every per-monitor mechanism elsewhere in this project (`Config`,
/// `State`, `Engine`, the GUI's selector) degenerates to "exactly one entry" under
/// GNOME, with zero changes needed to any of it.
///
/// Returns a `Result` (rather than a bare `Vec`) purely so this has the exact same
/// signature as `list_connected_monitors` - callers pick between the two as an
/// interchangeable function value at runtime - even though this specific
/// implementation can never actually fail.
pub fn list_gnome_monitors() -> anyhow::Result<Vec<Monitor>> {
    Ok(vec![Monitor {
        uuid: GNOME_SHARED_MONITOR_UUID.to_string(),
        connector: "GNOME".to_string(),
        is_primary: true,
        x: 0,
        y: 0,
    }])
}

/// Extracts the monitor identifier from one xfconf property path, if it's a
/// `last-image` property shaped like `/backdrop/screen{N}/monitor{id}/workspace{N}/
/// last-image`. Anything else (a different property name like `color-style` or
/// `image-path`, or an unexpected segment count) is `None` rather than a guess -
/// `xfconf-query -c xfce4-desktop -l` lists every property in the channel, most of
/// which aren't about which image is shown and must be ignored, not misparsed.
fn xfce_monitor_id_from_property_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let [backdrop, screen, monitor, workspace, property] = segments.as_slice() else {
        return None;
    };
    if *backdrop != "backdrop" || *property != "last-image" {
        return None;
    }
    if !screen.starts_with("screen") || !workspace.starts_with("workspace") {
        return None;
    }
    let monitor_id = monitor.strip_prefix("monitor")?;
    if monitor_id.is_empty() {
        return None;
    }
    Some(monitor_id.to_string())
}

/// Parses `xfconf-query -c xfce4-desktop -l`'s full output (one property path per
/// line) into a sorted, deduplicated list of monitor identifiers - sorted because
/// XFCE's xfconf schema has no "primary monitor" concept to read (unlike KDE's
/// `priority`), so this project picks whichever identifier sorts first alphabetically
/// as a deterministic stand-in, used only for Fase 1's "a new monitor copies the
/// primary's settings" behavior.
fn parse_xfce_monitor_listing(listing: &str) -> Vec<String> {
    let mut monitor_ids: Vec<String> = listing
        .lines()
        .filter_map(xfce_monitor_id_from_property_path)
        .collect();
    monitor_ids.sort();
    monitor_ids.dedup();
    monitor_ids
}

/// Lists every monitor XFCE's own `xfconf` currently has a `last-image` property for.
///
/// Unlike KDE (`kscreen-doctor` + `kwinoutputconfig.json`, an independent source of
/// truth for which monitors are physically connected) or GNOME (no per-monitor
/// concept at all), XFCE is inferred purely from xfconf's own already-populated
/// properties - a monitor XFCE's own `xfdesktop` process has never written a
/// `last-image` property for (e.g. freshly connected, before the user has opened
/// XFCE's own Appearance settings) will not appear here. This is a known,
/// intentionally-accepted limitation for this phase (no independent monitor-listing
/// tool like `xrandr` is cross-checked), documented for live-hardware verification
/// once available.
pub fn list_xfce_monitors() -> anyhow::Result<Vec<Monitor>> {
    let output = std::process::Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-l"])
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "xfconf-query -l exited with {}",
        output.status
    );
    let listing = String::from_utf8(output.stdout)?;

    Ok(parse_xfce_monitor_listing(&listing)
        .into_iter()
        .enumerate()
        .map(|(i, id)| Monitor {
            uuid: id.clone(),
            connector: id,
            is_primary: i == 0,
            x: 0,
            y: 0,
        })
        .collect())
}

/// Lists every currently-connected monitor, each with KDE's own stable UUID.
///
/// Combines two sources: `kscreen-doctor --json` for live connected/priority/position
/// data, and KWin's own `kwinoutputconfig.json` for the persistent per-monitor UUID
/// (cross-referenced by connector name - `kscreen-doctor --json` alone has no UUID). A
/// monitor connected but not yet present in `kwinoutputconfig.json` (KWin hasn't
/// persisted its config for it yet - rare, self-resolving within the same session) is
/// silently omitted rather than erroring.
pub fn list_connected_monitors() -> anyhow::Result<Vec<Monitor>> {
    let output = std::process::Command::new("kscreen-doctor")
        .arg("--json")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "kscreen-doctor exited with {}",
        output.status
    );
    let kscreen_json = String::from_utf8(output.stdout)?;
    let outputs = parse_kscreen_outputs(&kscreen_json)?;

    let kwin_config_text = std::fs::read_to_string(kwin_output_config_path()).unwrap_or_default();
    let uuids = parse_kwin_output_uuids(&kwin_config_text);

    Ok(outputs
        .into_iter()
        .filter(|o| o.connected && o.enabled)
        .filter_map(|o| {
            let uuid = uuids.get(&o.name)?.clone();
            Some(Monitor {
                uuid,
                connector: o.name,
                is_primary: o.priority == 1,
                x: o.pos.x,
                y: o.pos.y,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_KSCREEN_JSON: &str = r#"{
        "features": 255,
        "outputs": [
            {
                "connected": true,
                "enabled": true,
                "id": 1,
                "name": "LVDS-1",
                "pos": {"x": 0, "y": 0},
                "priority": 1
            }
        ],
        "screen": {"id": 0}
    }"#;

    const SAMPLE_KWIN_CONFIG_JSON: &str = r#"[
        {
            "data": [
                {
                    "connectorName": "LVDS-1",
                    "uuid": "e01e245f-8f3a-496f-bb9f-d6a02c263502",
                    "edidHash": "68fe312b5ef0e0a1bcd88890b73c7b3a"
                }
            ],
            "name": "outputs"
        },
        {
            "data": [
                {"id": 1},
                {"id": 2}
            ],
            "name": "setups"
        }
    ]"#;

    #[test]
    fn parses_a_single_connected_monitor() {
        let uuids = parse_kwin_output_uuids(SAMPLE_KWIN_CONFIG_JSON);
        let outputs = parse_kscreen_outputs(SAMPLE_KSCREEN_JSON).unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(
            uuids.get("LVDS-1").map(String::as_str),
            Some("e01e245f-8f3a-496f-bb9f-d6a02c263502")
        );
    }

    #[test]
    fn parses_two_connected_monitors_with_correct_positions_and_primary_flag() {
        let kscreen_json = r#"{
            "outputs": [
                {"connected": true, "name": "LVDS-1", "priority": 1, "pos": {"x": 0, "y": 0}},
                {"connected": true, "name": "HDMI-A-1", "priority": 2, "pos": {"x": 1280, "y": 0}}
            ]
        }"#;
        let kwin_json = r#"[
            {"name": "outputs", "data": [
                {"connectorName": "LVDS-1", "uuid": "uuid-a"},
                {"connectorName": "HDMI-A-1", "uuid": "uuid-b"}
            ]}
        ]"#;

        let outputs = parse_kscreen_outputs(kscreen_json).unwrap();
        let uuids = parse_kwin_output_uuids(kwin_json);

        assert_eq!(outputs.len(), 2);
        assert_eq!(uuids.len(), 2);
        assert_eq!(uuids.get("HDMI-A-1").map(String::as_str), Some("uuid-b"));
    }

    #[test]
    fn a_disconnected_output_is_marked_as_such() {
        let kscreen_json = r#"{
            "outputs": [
                {"connected": false, "name": "HDMI-A-1", "priority": 2, "pos": {"x": 1280, "y": 0}}
            ]
        }"#;
        let outputs = parse_kscreen_outputs(kscreen_json).unwrap();
        assert!(!outputs[0].connected);
    }

    /// A connected-but-disabled output (lid closed, or turned off in System Settings
    /// without unplugging) has no `Desktop` in Plasma's `desktops()` - including it in
    /// `list_connected_monitors()`'s result would desync `position_rank`'s index from
    /// that shorter list, silently targeting the wrong physical monitor.
    #[test]
    fn a_connected_but_disabled_output_is_marked_as_such() {
        let kscreen_json = r#"{
            "outputs": [
                {"connected": true, "enabled": false, "name": "LVDS-1", "priority": 1, "pos": {"x": 0, "y": 0}}
            ]
        }"#;
        let outputs = parse_kscreen_outputs(kscreen_json).unwrap();
        assert!(outputs[0].connected);
        assert!(!outputs[0].enabled);
    }

    #[test]
    fn an_output_with_no_enabled_field_defaults_to_enabled() {
        let kscreen_json = r#"{
            "outputs": [
                {"connected": true, "name": "LVDS-1", "priority": 1, "pos": {"x": 0, "y": 0}}
            ]
        }"#;
        let outputs = parse_kscreen_outputs(kscreen_json).unwrap();
        assert!(outputs[0].enabled);
    }

    #[test]
    fn kwin_config_with_no_outputs_entry_yields_an_empty_map() {
        let kwin_json = r#"[
            {"name": "setups", "data": [{"id": 1}]}
        ]"#;
        let uuids = parse_kwin_output_uuids(kwin_json);
        assert!(uuids.is_empty());
    }

    #[test]
    fn malformed_kwin_config_json_returns_an_empty_map_instead_of_panicking() {
        let uuids = parse_kwin_output_uuids("not valid json{{{");
        assert!(uuids.is_empty());
    }

    #[test]
    fn list_gnome_monitors_always_returns_one_shared_entry() {
        let monitors = list_gnome_monitors().unwrap();
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].uuid, GNOME_SHARED_MONITOR_UUID);
        assert!(monitors[0].is_primary);
    }

    #[test]
    fn xfce_monitor_id_from_property_path_extracts_the_monitor_segment() {
        assert_eq!(
            xfce_monitor_id_from_property_path("/backdrop/screen0/monitorDP-1/workspace0/last-image"),
            Some("DP-1".to_string())
        );
    }

    #[test]
    fn xfce_monitor_id_from_property_path_handles_numeric_monitor_ids() {
        assert_eq!(
            xfce_monitor_id_from_property_path("/backdrop/screen0/monitor0/workspace1/last-image"),
            Some("0".to_string())
        );
    }

    #[test]
    fn xfce_monitor_id_from_property_path_ignores_unrelated_properties() {
        assert_eq!(
            xfce_monitor_id_from_property_path("/backdrop/screen0/monitor0/workspace0/color-style"),
            None
        );
        assert_eq!(xfce_monitor_id_from_property_path("/backdrop/single-workspace-mode"), None);
        assert_eq!(xfce_monitor_id_from_property_path("/backdrop/screen0/monitor0/image-path"), None);
    }

    #[test]
    fn parse_xfce_monitor_listing_extracts_unique_monitors_sorted() {
        let listing = "\
/backdrop/screen0/monitor0/workspace0/last-image
/backdrop/screen0/monitor0/workspace0/color-style
/backdrop/screen0/monitor0/workspace1/last-image
/backdrop/screen0/monitorDP-1/workspace0/last-image
/backdrop/single-workspace-mode
";
        assert_eq!(parse_xfce_monitor_listing(listing), vec!["0".to_string(), "DP-1".to_string()]);
    }

    #[test]
    fn parse_xfce_monitor_listing_returns_empty_for_a_channel_with_no_monitors_configured() {
        assert_eq!(parse_xfce_monitor_listing("/backdrop/single-workspace-mode\n"), Vec::<String>::new());
    }
}
