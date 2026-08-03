/// The desktop environment this app is currently running under, detected once at
/// daemon/GUI startup via `detect_desktop_environment()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Kde,
    Gnome,
}

/// Reads `$XDG_CURRENT_DESKTOP` and picks a supported desktop environment, if any.
///
/// The value can be a colon-separated list (e.g. `"ubuntu:GNOME"`, `"budgie:GNOME"`)
/// rather than a bare `"GNOME"`/`"KDE"` - some distributions prepend their own name -
/// so this checks whether any segment matches, not the whole string. `None` means
/// "not KDE, not GNOME" - callers must not silently default to one or the other.
pub fn detect_desktop_environment() -> Option<DesktopEnvironment> {
    let value = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    detect_from_value(&value)
}

fn detect_from_value(value: &str) -> Option<DesktopEnvironment> {
    if value.split(':').any(|part| part.eq_ignore_ascii_case("KDE")) {
        Some(DesktopEnvironment::Kde)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")) {
        Some(DesktopEnvironment::Gnome)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kde_from_a_bare_value() {
        assert_eq!(detect_from_value("KDE"), Some(DesktopEnvironment::Kde));
    }

    #[test]
    fn detects_gnome_from_a_bare_value() {
        assert_eq!(detect_from_value("GNOME"), Some(DesktopEnvironment::Gnome));
    }

    #[test]
    fn detects_gnome_from_a_distro_prefixed_value() {
        assert_eq!(detect_from_value("ubuntu:GNOME"), Some(DesktopEnvironment::Gnome));
        assert_eq!(detect_from_value("budgie:GNOME"), Some(DesktopEnvironment::Gnome));
    }

    #[test]
    fn detects_kde_even_when_not_the_first_segment() {
        assert_eq!(detect_from_value("something:KDE"), Some(DesktopEnvironment::Kde));
    }

    #[test]
    fn returns_none_for_an_unrecognized_desktop() {
        assert_eq!(detect_from_value("XFCE"), None);
    }

    #[test]
    fn returns_none_for_an_empty_value() {
        assert_eq!(detect_from_value(""), None);
    }
}
