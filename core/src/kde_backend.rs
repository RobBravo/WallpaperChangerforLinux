use std::path::Path;
use crate::backend::WallpaperBackend;

pub struct KdePlasmaBackend;

/// Escapes a string for embedding inside a double-quoted JavaScript string literal.
///
/// Wallpaper paths come from a user-chosen folder, so a filename may legitimately
/// contain `"`, `\`, or (on Linux) even a newline. Interpolating those raw into the
/// Plasma shell script would break the literal or let a crafted filename inject
/// arbitrary Plasma scripting code.
fn escape_js_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn build_wallpaper_script(path: &Path) -> String {
    format!(
        r#"var allDesktops = desktops();
for (i = 0; i < allDesktops.length; i++) {{
    d = allDesktops[i];
    d.wallpaperPlugin = "org.kde.image";
    d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
    d.writeConfig("Image", "file://{}");
}}"#,
        escape_js_string(&path.display().to_string())
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

    #[test]
    fn a_quote_in_the_path_is_escaped_instead_of_ending_the_string_literal() {
        let script = build_wallpaper_script(&PathBuf::from(r#"/home/user/a".png"#));

        // the quote survives as an escaped quote, so the literal is never terminated early
        assert!(script.contains(r#"file:///home/user/a\".png"#), "script was: {script}");
        // and no injected code can follow it
        assert!(!script.contains(r#"a".png"#));
    }

    #[test]
    fn backslashes_and_control_characters_in_the_path_are_escaped() {
        let script = build_wallpaper_script(&PathBuf::from("/home/user/a\\b\nc.png"));

        assert!(script.contains(r"a\\b"), "script was: {script}");
        assert!(script.contains(r"b\nc.png"), "script was: {script}");
        // the raw newline must not reach the script body
        assert!(!script.contains("b\nc.png"));
    }
}
