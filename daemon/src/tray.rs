use ksni::blocking::TrayMethods;
use wallpaper_core::config::{change_now_request_path, Config};
use wallpaper_core::monitors::Monitor;

struct DaemonTray {
    list_monitors: fn() -> anyhow::Result<Vec<Monitor>>,
}

impl ksni::Tray for DaemonTray {
    fn id(&self) -> String {
        "wallpaper-changer".into()
    }

    fn title(&self) -> String {
        "Wallpaper Changer".into()
    }

    fn icon_name(&self) -> String {
        "preferences-desktop-wallpaper".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Pausar/Reanudar".into(),
                activate: Box::new(|tray: &mut Self| toggle_pause(tray.list_monitors)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Cambiar ahora".into(),
                activate: Box::new(|tray: &mut Self| request_change_now(tray.list_monitors)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Abrir configuración".into(),
                activate: Box::new(|_: &mut Self| open_config_gui()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Salir".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The tray has no per-monitor UI of its own, so "Pausar/Reanudar" and "Cambiar
/// ahora" act on the primary monitor as a stopgap - the GUI (with its own monitor
/// selector) is the way to control a non-primary monitor.
fn primary_monitor_uuid(list_monitors: fn() -> anyhow::Result<Vec<Monitor>>) -> Option<String> {
    list_monitors()
        .ok()?
        .into_iter()
        .find(|m| m.is_primary)
        .map(|m| m.uuid)
}

fn toggle_pause(list_monitors: fn() -> anyhow::Result<Vec<Monitor>>) {
    let Some(uuid) = primary_monitor_uuid(list_monitors) else {
        eprintln!("tray: no primary monitor detected, cannot toggle pause");
        return;
    };
    match Config::load() {
        Ok(mut config) => {
            match config.monitors.iter_mut().find(|m| m.uuid == uuid) {
                Some(monitor) => monitor.paused = !monitor.paused,
                None => eprintln!("tray: primary monitor {uuid} has no config entry yet"),
            }
            if let Err(e) = config.save() {
                eprintln!("tray: failed to save config.toml: {e}");
            }
        }
        Err(e) => eprintln!("tray: failed to load config.toml: {e}"),
    }
}

fn request_change_now(list_monitors: fn() -> anyhow::Result<Vec<Monitor>>) {
    let Some(uuid) = primary_monitor_uuid(list_monitors) else {
        eprintln!("tray: no primary monitor detected, cannot request an immediate change");
        return;
    };
    if let Err(e) = std::fs::write(change_now_request_path(), uuid) {
        eprintln!("tray: failed to write change_now_request: {e}");
    }
}

/// Spawns a background thread that waits for `child` to exit, so the OS can reap it
/// instead of leaving a zombie process behind once it exits. `Command::spawn` returns
/// a `Child` that is never awaited otherwise - and on this project's fast path (the
/// GUI's own single-instance guard makes most launches here exit within
/// milliseconds, having only delegated to an already-running instance), that would
/// mean a zombie left behind on every click of "Abrir configuración" while the GUI
/// is already open, for as long as the daemon keeps running.
fn reap_in_background(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

fn open_config_gui() {
    let path = dirs::home_dir()
        .map(|home| home.join(".local/bin/wallpaper-changer-gui"))
        .unwrap_or_else(|| std::path::PathBuf::from("wallpaper-changer-gui"));
    match std::process::Command::new(path).spawn() {
        Ok(child) => reap_in_background(child),
        Err(e) => eprintln!("tray: failed to launch wallpaper-changer-gui: {e}"),
    }
}

pub fn spawn_tray(list_monitors: fn() -> anyhow::Result<Vec<Monitor>>) {
    // ksni 0.3.6's blocking API is `TrayMethods::spawn`. Its initial D-Bus
    // session connect + `request_name` + `register_status_notifier_item`
    // handshake run synchronously on the calling thread before `spawn()`
    // returns a `Handle` for the backgrounded service loop. To keep that
    // handshake from delaying the daemon's own startup, the whole tray
    // setup runs on its own OS thread rather than on `main()`'s thread.
    std::thread::spawn(move || {
        if let Err(e) = (DaemonTray { list_monitors }).spawn() {
            eprintln!("tray: failed to start system tray icon: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A fake `list_monitors` standing in for a desktop-specific listing function
    /// (e.g. `list_gnome_monitors`) - a plain top-level `fn` so it coerces to the bare
    /// function pointer `primary_monitor_uuid` expects, same as a real listing
    /// function would.
    fn fake_list_monitors() -> anyhow::Result<Vec<Monitor>> {
        Ok(vec![Monitor {
            uuid: "fake-uuid".to_string(),
            connector: "FAKE".to_string(),
            is_primary: true,
            x: 0,
            y: 0,
        }])
    }

    /// Regression test for the whole-branch review finding that `primary_monitor_uuid`
    /// used to hardcode `list_connected_monitors` (KDE-only), so the tray's
    /// "Pausar/Reanudar" and "Cambiar ahora" silently did nothing under GNOME. Proves
    /// it now uses whichever `list_monitors` function it's given, rather than a
    /// hardcoded one.
    #[test]
    fn primary_monitor_uuid_uses_whichever_list_monitors_function_it_is_given() {
        assert_eq!(primary_monitor_uuid(fake_list_monitors), Some("fake-uuid".to_string()));
    }

    #[test]
    fn reap_in_background_waits_for_the_child_so_it_does_not_stay_a_zombie() {
        let child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();

        reap_in_background(child);

        // give the reaper thread time to call wait()
        std::thread::sleep(Duration::from_millis(300));

        // once a child is reaped, the kernel removes its /proc entry entirely -
        // a zombie (unreaped-but-exited) child would still have one, with State: Z
        let proc_entry_exists = std::path::Path::new(&format!("/proc/{pid}")).exists();
        assert!(!proc_entry_exists, "child process {pid} was not reaped (zombie left behind)");
    }
}
