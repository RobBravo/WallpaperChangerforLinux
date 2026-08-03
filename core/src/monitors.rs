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
}
