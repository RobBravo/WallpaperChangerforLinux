use ksni::blocking::TrayMethods;
use wallpaper_core::config::{change_now_request_path, Config};

struct DaemonTray;

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
                activate: Box::new(|_: &mut Self| toggle_pause()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Cambiar ahora".into(),
                activate: Box::new(|_: &mut Self| request_change_now()),
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

fn toggle_pause() {
    match Config::load() {
        Ok(mut config) => {
            config.paused = !config.paused;
            if let Err(e) = config.save() {
                eprintln!("tray: failed to save config.toml: {e}");
            }
        }
        Err(e) => eprintln!("tray: failed to load config.toml: {e}"),
    }
}

fn request_change_now() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    if let Err(e) = std::fs::write(change_now_request_path(), now) {
        eprintln!("tray: failed to write change_now_request: {e}");
    }
}

fn open_config_gui() {
    let path = dirs::home_dir()
        .map(|home| home.join(".local/bin/wallpaper-changer-gui"))
        .unwrap_or_else(|| std::path::PathBuf::from("wallpaper-changer-gui"));
    if let Err(e) = std::process::Command::new(path).spawn() {
        eprintln!("tray: failed to launch wallpaper-changer-gui: {e}");
    }
}

pub fn spawn_tray() {
    // ksni 0.3.6's blocking API is `TrayMethods::spawn`. Its initial D-Bus
    // session connect + `request_name` + `register_status_notifier_item`
    // handshake run synchronously on the calling thread before `spawn()`
    // returns a `Handle` for the backgrounded service loop. To keep that
    // handshake from delaying the daemon's own startup, the whole tray
    // setup runs on its own OS thread rather than on `main()`'s thread.
    std::thread::spawn(|| {
        if let Err(e) = DaemonTray.spawn() {
            eprintln!("tray: failed to start system tray icon: {e}");
        }
    });
}
