slint::include_modules!();

mod singleton;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wallpaper_core::config::{change_now_request_path, gui_socket_path, Config, IntervalUnit};
use wallpaper_core::state::State;

fn unit_to_index(unit: IntervalUnit) -> i32 {
    match unit {
        IntervalUnit::Minutes => 0,
        IntervalUnit::Hours => 1,
        IntervalUnit::Days => 2,
    }
}

fn index_to_unit(index: i32) -> IntervalUnit {
    match index {
        1 => IntervalUnit::Hours,
        2 => IntervalUnit::Days,
        _ => IntervalUnit::Minutes,
    }
}

/// Refreshes everything the window shows from the on-disk config/state.
///
/// `shown_wallpaper` remembers which image is currently displayed: the underlying
/// wallpaper only changes every few minutes, so decoding it again on every one-second
/// tick would mean a full 4K decode per second for as long as the window is open.
fn refresh_state(ui: &AppWindow, shown_wallpaper: &RefCell<Option<PathBuf>>) {
    // The daemon's tray menu can pause/resume behind our back, so re-read the flag
    // rather than trusting the value the window was started with.
    if let Ok(config) = Config::load() {
        if ui.get_paused() != config.paused {
            ui.set_paused(config.paused);
        }
    }

    let Ok(state) = State::load() else { return };

    let already_shown = shown_wallpaper
        .borrow()
        .as_deref()
        .is_some_and(|shown| shown == state.current_wallpaper.as_path());
    if !already_shown {
        if let Ok(image) = slint::Image::load_from_path(&state.current_wallpaper) {
            ui.set_preview_image(image);
            *shown_wallpaper.borrow_mut() = Some(state.current_wallpaper.clone());
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let remaining = (state.next_change_at_unix - now).max(0);
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;
    ui.set_countdown_text(format!("Próximo cambio en {hours:02}:{minutes:02}:{seconds:02}").into());
}

/// Shows the window if it's hidden, hides it if it's visible. Shared by the tray
/// menu's "Mostrar/Ocultar ventana" and the window's own close button - by the time
/// a close request fires the window is always visible, so this always hides it there.
fn toggle_visibility(ui: &AppWindow) {
    let window = ui.window();
    if window.is_visible() {
        let _ = window.hide();
    } else {
        let _ = window.show();
    }
}

fn main() -> anyhow::Result<()> {
    let socket_path = gui_socket_path();
    let listener = match singleton::claim(&socket_path) {
        Ok(singleton::Singleton::AlreadyRunning) => {
            if let Err(e) = singleton::notify_running_instance(&socket_path) {
                eprintln!("gui: failed to notify the running instance: {e}");
            }
            return Ok(());
        }
        Ok(singleton::Singleton::Primary(listener)) => Some(listener),
        Err(e) => {
            // Single-instance detection is a convenience, not a hard requirement -
            // its failure (e.g. a permissions problem on the config dir) must never
            // block the GUI from opening.
            eprintln!("gui: single-instance detection unavailable, continuing anyway: {e}");
            None
        }
    };

    let ui = AppWindow::new()?;
    // Kept alive for the whole process lifetime: per Slint's docs, a SystemTrayIcon's
    // icon appears as soon as the instance exists and disappears when it's dropped -
    // there's no explicit show() call for it, unlike the window.
    let tray = GuiTray::new()?;
    let config = Config::load()?;

    ui.set_folder_path(config.folder.display().to_string().into());
    ui.set_interval_value(config.interval_value as i32);
    ui.set_interval_unit_index(unit_to_index(config.interval_unit));
    ui.set_paused(config.paused);

    let shown_wallpaper: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    refresh_state(&ui, &shown_wallpaper);

    ui.on_choose_folder({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_folder_path(folder.display().to_string().into());
                }
            }
        }
    });

    ui.on_toggle_pause({
        let ui_handle = ui.as_weak();
        move || {
            if let Ok(mut config) = Config::load() {
                config.paused = !config.paused;
                if config.save().is_ok() {
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_paused(config.paused);
                    }
                }
            }
        }
    });

    ui.on_change_now(move || {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let _ = std::fs::write(change_now_request_path(), now);
    });

    ui.on_save({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                // Only the fields this window owns are written back. `paused` is owned by
                // the pause toggle (here and in the daemon's tray), so it is carried over
                // from the freshly-loaded file - otherwise saving here would silently undo
                // a pause set from the tray while this window was open.
                let Ok(mut config) = Config::load() else { return };
                config.folder = PathBuf::from(ui.get_folder_path().to_string());
                config.interval_value = ui.get_interval_value() as u64;
                config.interval_unit = index_to_unit(ui.get_interval_unit_index());
                let _ = config.save();
            }
        }
    });

    ui.window().on_close_requested({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                toggle_visibility(&ui);
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    tray.on_toggle_visibility({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                toggle_visibility(&ui);
            }
        }
    });

    tray.on_quit(move || {
        let _ = slint::quit_event_loop();
    });

    if let Some(listener) = listener {
        let ui_handle = ui.as_weak();
        singleton::spawn_accept_loop(listener, move || {
            let ui_handle = ui_handle.clone();
            // `spawn_accept_loop`'s callback runs on its own OS thread, not the Slint
            // event loop thread, so touching `ui` has to be scheduled back onto it.
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_handle.upgrade() {
                    let _ = ui.window().show();
                }
            });
        });
    }

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let shown_wallpaper = shown_wallpaper.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    refresh_state(&ui, &shown_wallpaper);
                }
            },
        );
    }

    // Not `ui.run()`: that convenience method runs the event loop configured to quit
    // as soon as the last window closes, which would end the process the moment the
    // window is hidden. `run_event_loop_until_quit` only stops at `quit_event_loop()`
    // (wired to the tray's "Salir" above), so hiding the window just leaves the tray
    // icon behind.
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}
