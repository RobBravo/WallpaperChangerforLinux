use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

const SHOW_MESSAGE: &[u8; 4] = b"show";

pub enum Singleton {
    Primary(UnixListener),
    AlreadyRunning,
}

/// Claims `socket_path` as the single running instance, or detects that another
/// instance already holds it.
///
/// A stale socket file left behind by a process that didn't exit cleanly (e.g. a
/// crash) would otherwise make every future launch see `AlreadyRunning` forever
/// with nothing actually listening, so a failed connect always clears the path
/// before binding fresh - `UnixListener::bind` fails with `AddrInUse` if a file
/// already exists there.
pub fn claim(socket_path: &Path) -> anyhow::Result<Singleton> {
    if UnixStream::connect(socket_path).is_ok() {
        return Ok(Singleton::AlreadyRunning);
    }

    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    Ok(Singleton::Primary(listener))
}

/// Tells the already-running primary instance to show its window.
pub fn notify_running_instance(socket_path: &Path) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(SHOW_MESSAGE)?;
    stream.flush()?;
    Ok(())
}

/// Runs the primary instance's side of the protocol: accepts connections on a
/// background thread for as long as the process lives, and calls `on_show` for
/// each one that sends exactly the expected message. Anything else - a short
/// read, garbage bytes, a connection that closes early - is silently dropped.
pub fn spawn_accept_loop(listener: UnixListener, on_show: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else { continue };
            let mut buf = [0u8; SHOW_MESSAGE.len()];
            if stream.read_exact(&mut buf).is_ok() && &buf == SHOW_MESSAGE {
                on_show();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    #[test]
    fn claiming_a_fresh_path_becomes_the_primary_instance() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        match claim(&socket_path).unwrap() {
            Singleton::Primary(_listener) => {}
            Singleton::AlreadyRunning => panic!("expected Primary for a fresh socket path"),
        }
    }

    #[test]
    fn claiming_an_already_claimed_path_detects_the_running_instance() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        let _primary = claim(&socket_path).unwrap();

        match claim(&socket_path).unwrap() {
            Singleton::AlreadyRunning => {}
            Singleton::Primary(_) => panic!("expected AlreadyRunning while the primary is alive"),
        }
    }

    #[test]
    fn notifying_the_primary_reaches_its_accept_loop() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        let listener = match claim(&socket_path).unwrap() {
            Singleton::Primary(listener) => listener,
            Singleton::AlreadyRunning => panic!("expected to become the primary instance"),
        };

        let (tx, rx) = channel();
        spawn_accept_loop(listener, move || {
            let _ = tx.send(());
        });

        notify_running_instance(&socket_path).unwrap();

        rx.recv_timeout(Duration::from_secs(5))
            .expect("accept loop did not receive the show notification");
    }

    #[test]
    fn a_short_or_unrecognized_message_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        let listener = match claim(&socket_path).unwrap() {
            Singleton::Primary(listener) => listener,
            Singleton::AlreadyRunning => panic!("expected to become the primary instance"),
        };

        let (tx, rx) = channel::<()>();
        spawn_accept_loop(listener, move || {
            let _ = tx.send(());
        });

        let mut stream = UnixStream::connect(&socket_path).unwrap();
        stream.write_all(b"no").unwrap();
        drop(stream);

        match rx.recv_timeout(Duration::from_millis(500)) {
            Err(RecvTimeoutError::Timeout) => {}
            other => panic!("expected no notification for an unrecognized message, got {other:?}"),
        }
    }
}
