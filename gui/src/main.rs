slint::include_modules!();

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

fn refresh_state(ui: &AppWindow) {
    let Ok(state) = State::load() else { return };

    if let Ok(image) = slint::Image::load_from_path(&state.current_wallpaper) {
        ui.set_preview_image(image);
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
    refresh_state(&ui);

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
                let config = Config {
                    folder: std::path::PathBuf::from(ui.get_folder_path().to_string()),
                    interval_value: ui.get_interval_value() as u64,
                    interval_unit: index_to_unit(ui.get_interval_unit_index()),
                    paused: ui.get_paused(),
                };
                let _ = config.save();
            }
        }
    });

    let timer = slint::Timer::default();
    {
        let ui_handle = ui.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    refresh_state(&ui);
                }
            },
        );
    }

    ui.run()?;
    Ok(())
}
