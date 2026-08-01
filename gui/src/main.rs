slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wallpaper_core::config::{change_now_request_path, Config, IntervalUnit};
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

fn main() -> anyhow::Result<()> {
    let ui = AppWindow::new()?;
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

    ui.run()?;
    Ok(())
}
