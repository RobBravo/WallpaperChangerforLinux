slint::include_modules!();

mod singleton;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wallpaper_core::config::{change_now_request_path, gui_lock_path, gui_socket_path, Config, IntervalUnit};
use wallpaper_core::desktop::{detect_desktop_environment, DesktopEnvironment};
use wallpaper_core::monitors::{list_connected_monitors, list_gnome_monitors, list_xfce_monitors, Monitor};
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

fn monitor_label(monitor: &Monitor, position: usize) -> String {
    format!("Monitor {} ({})", position + 1, monitor.connector)
}

/// Picks which monitor-listing function to use, and whether the dropdown should show
/// GNOME's single shared-desktop label instead of per-monitor labels. Pulled out as
/// its own function so this decision is unit-testable without a live desktop
/// session - unlike the KDE case, GNOME (and an unrecognized desktop, which falls
/// back to the same behavior `list_connected_monitors()` failing already has today -
/// an empty dropdown, no crash) can't be exercised by an automated GUI test at all.
fn monitor_source(env: Option<DesktopEnvironment>) -> (fn() -> anyhow::Result<Vec<Monitor>>, bool) {
    match env {
        Some(DesktopEnvironment::Gnome) => (list_gnome_monitors, true),
        Some(DesktopEnvironment::Xfce) => (list_xfce_monitors, false),
        _ => (list_connected_monitors, false),
    }
}

/// Populates the form fields for `uuid` from `config`. Falls back to
/// `Config::for_new_monitor`'s defaults if this monitor has no config entry yet (it
/// just connected and the daemon hasn't caught up during its own 30-second poll -
/// this self-corrects on the next reload once it has).
fn populate_form(ui: &AppWindow, uuid: &str, config: &Config, primary_uuid: Option<&str>) {
    let monitor_config = config
        .monitor(uuid)
        .cloned()
        .unwrap_or_else(|| config.for_new_monitor(uuid, primary_uuid));
    ui.set_folder_path(monitor_config.folder.display().to_string().into());
    ui.set_interval_value(monitor_config.interval_value as i32);
    ui.set_interval_unit_index(unit_to_index(monitor_config.interval_unit));
    ui.set_paused(monitor_config.paused);
}

/// Refreshes the currently-selected monitor's preview image and countdown from
/// state.toml. `shown_wallpaper` remembers `(uuid, path)` so switching monitors, or
/// that monitor's wallpaper actually changing, both correctly trigger a fresh image
/// decode - but repeated ticks with nothing new don't.
fn refresh_state(ui: &AppWindow, uuid: &str, shown_wallpaper: &RefCell<Option<(String, PathBuf)>>) {
    // The daemon's tray menu, or another monitor's tab, can pause/resume this
    // monitor behind our back, so re-read the flag rather than trusting stale state.
    if let Ok(config) = Config::load() {
        if let Some(monitor_config) = config.monitor(uuid) {
            if ui.get_paused() != monitor_config.paused {
                ui.set_paused(monitor_config.paused);
            }
        }
    }

    let Ok(state) = State::load() else { return };
    let Some(monitor_state) = state.monitor(uuid) else { return };

    let already_shown = shown_wallpaper
        .borrow()
        .as_ref()
        .is_some_and(|(shown_uuid, shown_path)| shown_uuid == uuid && shown_path == &monitor_state.current_wallpaper);
    if !already_shown {
        if let Ok(image) = slint::Image::load_from_path(&monitor_state.current_wallpaper) {
            ui.set_preview_image(image);
            *shown_wallpaper.borrow_mut() = Some((uuid.to_string(), monitor_state.current_wallpaper.clone()));
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let remaining = (monitor_state.next_change_at_unix - now).max(0);
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;
    ui.set_countdown_text(format!("Próximo cambio en {hours:02}:{minutes:02}:{seconds:02}").into());
}

/// Re-enumerates connected monitors and updates the dropdown if the set changed.
/// The GUI is a tray-resident, long-lived singleton (unlike the daemon, it isn't
/// restarted when a monitor is hot-plugged), so without this the dropdown would
/// permanently reflect only whatever was connected at the moment the GUI first
/// launched - plugging in a second monitor would never make it selectable, even by
/// closing and reopening the window (only killing the whole process would help).
///
/// Preserves the current selection across a refresh when the selected monitor is
/// still connected (its index may have moved after a resort); falls back to the
/// first available monitor if it was unplugged, or to no selection if none remain.
///
/// `list_monitors`/`is_gnome` come from `monitor_source` - under GNOME this always
/// returns the same one-entry list, so after the first successful call this becomes
/// a permanent no-op (the `if *uuids.borrow() == new_uuids` check below), which is
/// correct: there is nothing to re-detect under GNOME's single shared desktop model.
fn refresh_monitor_list(
    ui: &AppWindow,
    list_monitors: fn() -> anyhow::Result<Vec<Monitor>>,
    is_gnome: bool,
    uuids: &RefCell<Vec<String>>,
    primary_uuid: &RefCell<Option<String>>,
    current_uuid: &RefCell<Option<String>>,
    shown_wallpaper: &RefCell<Option<(String, PathBuf)>>,
) {
    // A transient failure (e.g. `kscreen-doctor` briefly unavailable during a display
    // reconfiguration) must not be treated as "every monitor just disconnected" - that
    // would wipe an already-populated dropdown and, on the next successful poll,
    // overwrite any unsaved form edits when the selection snaps back. Leaving the
    // existing list untouched is a no-op the first time this runs (it starts empty).
    let Ok(mut monitors) = list_monitors() else { return };
    monitors.sort_by(|a, b| a.connector.cmp(&b.connector));
    let new_uuids: Vec<String> = monitors.iter().map(|m| m.uuid.clone()).collect();

    if *uuids.borrow() == new_uuids {
        return; // nothing connected/disconnected since the last check
    }

    let new_primary = monitors.iter().find(|m| m.is_primary).map(|m| m.uuid.clone());
    let labels: Vec<slint::SharedString> = if is_gnome {
        vec!["Todos los monitores".into()]
    } else {
        monitors.iter().enumerate().map(|(i, m)| monitor_label(m, i).into()).collect()
    };
    ui.set_monitor_labels(Rc::new(slint::VecModel::from(labels)).into());

    let still_connected = current_uuid.borrow().as_ref().is_some_and(|uuid| new_uuids.contains(uuid));
    let new_current = if still_connected { current_uuid.borrow().clone() } else { new_uuids.first().cloned() };
    let new_index = new_current.as_ref().and_then(|uuid| new_uuids.iter().position(|u| u == uuid)).unwrap_or(0);

    *uuids.borrow_mut() = new_uuids;
    *primary_uuid.borrow_mut() = new_primary;
    ui.set_selected_monitor_index(new_index as i32);

    if new_current != *current_uuid.borrow() {
        *shown_wallpaper.borrow_mut() = None; // force a fresh decode for the newly-selected monitor
    }
    *current_uuid.borrow_mut() = new_current.clone();

    if let Some(uuid) = new_current {
        if let Ok(config) = Config::load() {
            populate_form(ui, &uuid, &config, primary_uuid.borrow().as_deref());
        }
        refresh_state(ui, &uuid, shown_wallpaper);
    }
}

/// Shows the window if it's hidden, hides it if it's visible. Shared by the tray
/// menu's "Mostrar/Ocultar ventana" and the window's own close button.
fn toggle_visibility(ui: &AppWindow) {
    let window = ui.window();
    if window.is_visible() && !window.is_minimized() {
        let _ = window.hide();
    } else {
        show_and_restore(window);
    }
}

fn show_and_restore(window: &slint::Window) {
    let _ = window.show();
    window.set_minimized(false);
}

fn main() -> anyhow::Result<()> {
    let socket_path = gui_socket_path();
    let lock_path = gui_lock_path();
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let (listener, _lock_file) = match singleton::claim(&socket_path, &lock_path) {
        Ok(singleton::Singleton::AlreadyRunning) => {
            if let Err(e) = singleton::notify_running_instance(&socket_path) {
                eprintln!("gui: failed to notify the running instance: {e}");
            }
            return Ok(());
        }
        Ok(singleton::Singleton::Primary(listener, lock_file)) => (Some(listener), Some(lock_file)),
        Err(e) => {
            eprintln!("gui: single-instance detection unavailable, continuing anyway: {e}");
            (None, None)
        }
    };

    let ui = AppWindow::new()?;
    let tray = GuiTray::new()?;

    let (list_monitors, is_gnome) = monitor_source(detect_desktop_environment());

    let uuids: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let primary_uuid: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let current_uuid: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let shown_wallpaper: Rc<RefCell<Option<(String, PathBuf)>>> = Rc::new(RefCell::new(None));

    refresh_monitor_list(&ui, list_monitors, is_gnome, &uuids, &primary_uuid, &current_uuid, &shown_wallpaper);

    ui.on_monitor_selected({
        let ui_handle = ui.as_weak();
        let uuids = uuids.clone();
        let current_uuid = current_uuid.clone();
        let shown_wallpaper = shown_wallpaper.clone();
        let primary_uuid = primary_uuid.clone();
        move || {
            let Some(ui) = ui_handle.upgrade() else { return };
            let index = ui.get_selected_monitor_index();
            let Some(uuid) = uuids.borrow().get(index as usize).cloned() else { return };
            *current_uuid.borrow_mut() = Some(uuid.clone());
            *shown_wallpaper.borrow_mut() = None; // force a fresh decode for the newly-selected monitor
            if let Ok(config) = Config::load() {
                populate_form(&ui, &uuid, &config, primary_uuid.borrow().as_deref());
            }
            refresh_state(&ui, &uuid, &shown_wallpaper);
        }
    });

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
        let current_uuid = current_uuid.clone();
        move || {
            let Some(uuid) = current_uuid.borrow().clone() else { return };
            let Ok(mut config) = Config::load() else { return };
            let Some(monitor) = config.monitors.iter_mut().find(|m| m.uuid == uuid) else { return };
            monitor.paused = !monitor.paused;
            let new_paused = monitor.paused;
            if config.save().is_ok() {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_paused(new_paused);
                }
            }
        }
    });

    ui.on_change_now({
        let current_uuid = current_uuid.clone();
        move || {
            let Some(uuid) = current_uuid.borrow().clone() else { return };
            let _ = std::fs::write(change_now_request_path(), uuid);
        }
    });

    ui.on_save({
        let ui_handle = ui.as_weak();
        let current_uuid = current_uuid.clone();
        let primary_uuid = primary_uuid.clone();
        move || {
            let Some(ui) = ui_handle.upgrade() else { return };
            let Some(uuid) = current_uuid.borrow().clone() else { return };
            let Ok(mut config) = Config::load() else { return };
            let folder = PathBuf::from(ui.get_folder_path().to_string());
            let interval_value = ui.get_interval_value() as u64;
            let interval_unit = index_to_unit(ui.get_interval_unit_index());
            match config.monitors.iter_mut().find(|m| m.uuid == uuid) {
                Some(existing) => {
                    existing.folder = folder;
                    existing.interval_value = interval_value;
                    existing.interval_unit = interval_unit;
                    // `paused` is intentionally left untouched - owned by the pause
                    // toggle, not the save button (same rule as before this plan).
                }
                None => {
                    let mut fresh = config.for_new_monitor(&uuid, primary_uuid.borrow().as_deref());
                    fresh.folder = folder;
                    fresh.interval_value = interval_value;
                    fresh.interval_unit = interval_unit;
                    config.monitors.push(fresh);
                }
            }
            let _ = config.save();
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
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_handle.upgrade() {
                    show_and_restore(ui.window());
                }
            });
        });
    }

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let shown_wallpaper = shown_wallpaper.clone();
        let current_uuid = current_uuid.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    if !ui.window().is_visible() {
                        return;
                    }
                    if let Some(uuid) = current_uuid.borrow().clone() {
                        refresh_state(&ui, &uuid, &shown_wallpaper);
                    }
                }
            },
        );
    }

    // Separate from the 1-second state timer above: re-enumerating monitors spawns a
    // `kscreen-doctor` subprocess (or is a no-op under GNOME), so this runs on its own,
    // coarser cadence rather than every tick. The GUI is a tray-resident singleton
    // (see `refresh_monitor_list`'s doc comment) so this is the only thing that ever
    // notices a hot-plugged monitor while it's already running.
    let monitor_poll_timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        let uuids = uuids.clone();
        let primary_uuid = primary_uuid.clone();
        let current_uuid = current_uuid.clone();
        let shown_wallpaper = shown_wallpaper.clone();
        monitor_poll_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(5),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    if !ui.window().is_visible() {
                        return;
                    }
                    refresh_monitor_list(&ui, list_monitors, is_gnome, &uuids, &primary_uuid, &current_uuid, &shown_wallpaper);
                }
            },
        );
    }

    ui.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_source_picks_the_gnome_shared_label_and_listing_for_gnome() {
        let (list_monitors, is_gnome) = monitor_source(Some(DesktopEnvironment::Gnome));
        assert_eq!(list_monitors as *const () as usize, list_gnome_monitors as *const () as usize);
        assert!(is_gnome);
    }

    #[test]
    fn monitor_source_picks_the_kde_listing_for_kde() {
        let (list_monitors, is_gnome) = monitor_source(Some(DesktopEnvironment::Kde));
        assert_eq!(list_monitors as *const () as usize, list_connected_monitors as *const () as usize);
        assert!(!is_gnome);
    }

    #[test]
    fn monitor_source_falls_back_to_kde_listing_for_an_unrecognized_desktop() {
        // Matches list_connected_monitors()'s own existing failure behavior (empty
        // dropdown, no crash) rather than refusing to show anything at all - a user on
        // an unsupported desktop might still want to inspect the GUI's settings.
        let (list_monitors, is_gnome) = monitor_source(None);
        assert_eq!(list_monitors as *const () as usize, list_connected_monitors as *const () as usize);
        assert!(!is_gnome);
    }

    #[test]
    fn monitor_source_picks_the_xfce_listing_for_xfce() {
        let (list_monitors, is_gnome) = monitor_source(Some(DesktopEnvironment::Xfce));
        assert_eq!(list_monitors as *const () as usize, list_xfce_monitors as *const () as usize);
        assert!(!is_gnome);
    }
}
