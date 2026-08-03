/// The desktop environment this app is currently running under, detected once at
/// daemon/GUI startup via `detect_desktop_environment()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Kde,
    Gnome,
    Xfce,
}

/// Reads `$XDG_CURRENT_DESKTOP` and picks a supported desktop environment, if any.
///
/// The value can be a colon-separated list (e.g. `"ubuntu:GNOME"`, `"budgie:GNOME"`)
/// rather than a bare `"GNOME"`/`"KDE"`/`"XFCE"` - some distributions prepend their
/// own name - so this checks whether any segment matches, not the whole string.
/// `None` means "not KDE, GNOME, or XFCE" - callers must not silently default to any
/// of them.
pub fn detect_desktop_environment() -> Option<DesktopEnvironment> {
    let value = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    detect_from_value(&value)
}

fn detect_from_value(value: &str) -> Option<DesktopEnvironment> {
    if value.split(':').any(|part| part.eq_ignore_ascii_case("KDE")) {
        Some(DesktopEnvironment::Kde)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")) {
        Some(DesktopEnvironment::Gnome)
    } else if value.split(':').any(|part| part.eq_ignore_ascii_case("XFCE")) {
        Some(DesktopEnvironment::Xfce)
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
    fn detects_xfce_from_a_bare_value() {
        assert_eq!(detect_from_value("XFCE"), Some(DesktopEnvironment::Xfce));
    }

    #[test]
    fn detects_xfce_from_a_distro_prefixed_value() {
        assert_eq!(detect_from_value("X-Generic:XFCE"), Some(DesktopEnvironment::Xfce));
    }

    #[test]
    fn returns_none_for_an_unrecognized_desktop() {
        assert_eq!(detect_from_value("MATE"), None);
    }

    #[test]
    fn returns_none_for_an_empty_value() {
        assert_eq!(detect_from_value(""), None);
    }
}
