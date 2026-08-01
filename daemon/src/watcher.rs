use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use notify::{RecursiveMode, Watcher};

pub enum DaemonEvent {
    ConfigChanged,
    ChangeNowRequested,
}

pub fn spawn_watcher(
    config_dir: PathBuf,
    tx: Sender<DaemonEvent>,
) -> anyhow::Result<notify::RecommendedWatcher> {
    let config_file_name = OsString::from("config.toml");
    let change_now_file_name = OsString::from("change_now_request");

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        // `Access` events (open/close/execute) are non-mutating. Reacting to them would
        // self-trigger: the daemon's own `Config::load()` call in response to a real change
        // opens config.toml, which without this guard produces another matching event forever.
        if event.kind.is_access() {
            return;
        }
        for path in event.paths {
            match path.file_name() {
                Some(name) if name == config_file_name => {
                    let _ = tx.send(DaemonEvent::ConfigChanged);
                }
                Some(name) if name == change_now_file_name => {
                    let _ = tx.send(DaemonEvent::ChangeNowRequested);
                }
                _ => {}
            }
        }
    })?;
    watcher.watch(&config_dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    #[test]
    fn writing_config_toml_sends_config_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = channel();
        let _watcher = spawn_watcher(dir.path().to_path_buf(), tx).unwrap();

        std::fs::write(dir.path().join("config.toml"), b"paused = true").unwrap();

        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(DaemonEvent::ConfigChanged) => {}
            Ok(DaemonEvent::ChangeNowRequested) => panic!("expected ConfigChanged, got ChangeNowRequested"),
            Err(RecvTimeoutError::Timeout) => panic!("no event received within timeout"),
            Err(e) => panic!("channel error: {e}"),
        }
    }

    #[test]
    fn writing_change_now_request_sends_change_now_requested() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = channel();
        let _watcher = spawn_watcher(dir.path().to_path_buf(), tx).unwrap();

        std::fs::write(dir.path().join("change_now_request"), b"123").unwrap();

        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(DaemonEvent::ChangeNowRequested) => {}
            Ok(DaemonEvent::ConfigChanged) => panic!("expected ChangeNowRequested, got ConfigChanged"),
            Err(RecvTimeoutError::Timeout) => panic!("no event received within timeout"),
            Err(e) => panic!("channel error: {e}"),
        }
    }

    /// Regression test for a self-triggering feedback loop: the daemon's real reaction to a
    /// `ConfigChanged` event is to call `Config::load()`, which *reads* (opens) config.toml.
    /// If the watcher treats that read the same as a real edit, every reload event spawns
    /// another reload event, forever, pegging the CPU and starving the daemon of the real
    /// 60-second wallpaper-rotation timeout.
    #[test]
    fn reading_config_toml_does_not_send_a_spurious_config_changed_event() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, b"paused = true").unwrap();

        let (tx, rx) = channel();
        let _watcher = spawn_watcher(dir.path().to_path_buf(), tx).unwrap();

        // A real edit, exactly like the daemon's own reload path or the GUI's "Guardar".
        std::fs::write(&config_path, b"paused = false").unwrap();
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(DaemonEvent::ConfigChanged) => {}
            Ok(DaemonEvent::ChangeNowRequested) => panic!("expected ConfigChanged, got ChangeNowRequested"),
            Err(RecvTimeoutError::Timeout) => panic!("no event received for the real edit"),
            Err(e) => panic!("channel error: {e}"),
        }

        // Drain any duplicate raw events the real write itself produced (already-established
        // behavior from `writing_config_toml_sends_config_changed`'s sibling tests), so only
        // events caused by the read below can land in the assertion below.
        std::thread::sleep(Duration::from_millis(200));
        while rx.try_recv().is_ok() {}

        // This mirrors exactly what the daemon does after receiving that event: reload the file.
        let _ = std::fs::read_to_string(&config_path);

        match rx.recv_timeout(Duration::from_millis(500)) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(DaemonEvent::ConfigChanged) => panic!(
                "reading config.toml sent a spurious ConfigChanged event - this is a self-triggering feedback loop"
            ),
            Ok(DaemonEvent::ChangeNowRequested) => {
                panic!("reading config.toml sent an unexpected ChangeNowRequested event")
            }
            Err(e) => panic!("channel error: {e}"),
        }
    }
}
