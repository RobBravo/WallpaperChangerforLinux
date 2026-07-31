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
}
