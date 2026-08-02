use std::path::Path;
use crate::backend::WallpaperBackend;
use crate::monitors::Monitor;

pub struct KdePlasmaBackend;

/// Escapes a string for embedding inside a double-quoted JavaScript string literal.
///
/// Wallpaper paths come from a user-chosen folder, so a filename may legitimately
/// contain `"`, `\`, or (on Linux) even a newline. Interpolating those raw into the
/// Plasma shell script would break the literal or let a crafted filename inject
/// arbitrary Plasma scripting code.
///
/// U+2028 (LINE SEPARATOR) and U+2029 (PARAGRAPH SEPARATOR) get their own arms
/// because `char::is_control()` doesn't consider them control characters (they're
/// category Zl/Zp, not Cc) even though ECMAScript treats them as `LineTerminator`s -
/// on a JS engine that hasn't adopted the ES2019 relaxation permitting them raw
/// inside string literals, either one would terminate the literal early, same as an
/// unescaped `\n` would.
fn escape_js_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Computes `target`'s rank (0-based) among `all_monitors` when sorted top-to-bottom,
/// then left-to-right - the same order `build_wallpaper_script`'s generated script
/// sorts Plasma's `desktops()` by, so index `rank` in both refers to the same
/// physical monitor. `None` if `target` isn't actually present in `all_monitors`
/// (shouldn't happen in practice - callers always pass `target` as one of
/// `all_monitors` - but this avoids a panic if that invariant is ever violated).
fn position_rank(all_monitors: &[Monitor], target: &Monitor) -> Option<usize> {
    let mut sorted: Vec<&Monitor> = all_monitors.iter().collect();
    sorted.sort_by_key(|m| (m.y, m.x));
    sorted.iter().position(|m| m.uuid == target.uuid)
}

/// Plasma's scripting API has no hardware/connector identifier on a `Desktop` object
/// (`desktops()[i].screen` is only a KWin screen index) - `screenGeometry(screen)`'s
/// physical position is the only reliable correlation key, matching `position_rank`'s
/// use of `Monitor.x`/`.y` from `kscreen-doctor`.
fn build_wallpaper_script(rank: usize, path: &Path) -> String {
    format!(
        r#"var sorted = desktops().filter(function(d) {{ return d.screen != -1; }}).sort(function(a, b) {{
    var ga = screenGeometry(a.screen), gb = screenGeometry(b.screen);
    if (ga.top !== gb.top) return ga.top - gb.top;
    return ga.left - gb.left;
}});
var d = sorted[{rank}];
if (d) {{
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
    d.writeConfig("Image", "file://{}");
}}"#,
        escape_js_string(&path.display().to_string())
    )
}

impl WallpaperBackend for KdePlasmaBackend {
    fn set_wallpaper(&self, all_monitors: &[Monitor], target: &Monitor, path: &Path) -> anyhow::Result<()> {
        let Some(rank) = position_rank(all_monitors, target) else {
            anyhow::bail!("target monitor {} is not present in all_monitors", target.uuid);
        };
        let script = build_wallpaper_script(rank, path);
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

    fn monitor(uuid: &str, x: i32, y: i32) -> Monitor {
        Monitor { uuid: uuid.to_string(), connector: uuid.to_string(), is_primary: false, x, y }
    }

    #[test]
    fn script_embeds_the_image_path_as_a_file_url() {
        let script = build_wallpaper_script(0, &PathBuf::from("/home/user/Pictures/a.png"));
        assert!(script.contains(r#"file:///home/user/Pictures/a.png"#));
        assert!(script.contains(r#"wallpaperPlugin = "org.kde.image""#));
    }

    #[test]
    fn a_quote_in_the_path_is_escaped_instead_of_ending_the_string_literal() {
        let script = build_wallpaper_script(0, &PathBuf::from(r#"/home/user/a".png"#));
        assert!(script.contains(r#"file:///home/user/a\".png"#), "script was: {script}");
        assert!(!script.contains(r#"a".png"#));
    }

    #[test]
    fn backslashes_and_control_characters_in_the_path_are_escaped() {
        let script = build_wallpaper_script(0, &PathBuf::from("/home/user/a\\b\nc.png"));
        assert!(script.contains(r"a\\b"), "script was: {script}");
        assert!(script.contains(r"b\nc.png"), "script was: {script}");
        assert!(!script.contains("b\nc.png"));
    }

    #[test]
    fn unicode_line_and_paragraph_separators_in_the_path_are_escaped() {
        let line_sep = '\u{2028}';
        let para_sep = '\u{2029}';
        let raw_path = format!("/home/user/a{line_sep}b{para_sep}c.png");
        let script = build_wallpaper_script(0, &PathBuf::from(raw_path));

        assert!(script.contains(r"a\u2028b"), "script was: {script}");
        assert!(script.contains(r"b\u2029c.png"), "script was: {script}");
        // the raw separators must not reach the script body: some JS engines treat
        // U+2028/U+2029 as line terminators even inside a double-quoted string
        // literal, which would end the literal early just like an unescaped `\n`
        assert!(!script.contains(&format!("a{line_sep}b")));
        assert!(!script.contains(&format!("b{para_sep}c.png")));
    }

    #[test]
    fn script_targets_the_computed_rank_index() {
        let script = build_wallpaper_script(2, &PathBuf::from("/a.png"));
        assert!(script.contains("sorted[2]"), "script was: {script}");
    }

    #[test]
    fn position_rank_orders_monitors_left_to_right_when_at_the_same_height() {
        let left = monitor("left", 0, 0);
        let right = monitor("right", 1920, 0);
        let all = vec![right.clone(), left.clone()]; // deliberately out of order

        assert_eq!(position_rank(&all, &left), Some(0));
        assert_eq!(position_rank(&all, &right), Some(1));
    }

    #[test]
    fn position_rank_prioritizes_vertical_position_over_horizontal() {
        let top = monitor("top", 1000, 0);
        let bottom_left = monitor("bottom-left", 0, 1080);
        let all = vec![top.clone(), bottom_left.clone()];

        // even though bottom-left has a smaller x, its larger y means it ranks after top
        assert_eq!(position_rank(&all, &top), Some(0));
        assert_eq!(position_rank(&all, &bottom_left), Some(1));
    }

    #[test]
    fn position_rank_returns_none_for_a_monitor_not_in_the_list() {
        let all = vec![monitor("a", 0, 0)];
        let stranger = monitor("b", 100, 100);
        assert_eq!(position_rank(&all, &stranger), None);
    }
}
