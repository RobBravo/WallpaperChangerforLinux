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
/// with nothing actually listening, so the path is cleared before binding fresh -
/// `UnixListener::bind` fails with `AddrInUse` if a file already exists there.
///
/// Only `ConnectionRefused` (the file is there but nothing is listening: the
/// definitive stale signal) and `NotFound` (nothing there at all, so removing is a
/// no-op) are treated as safe to reclaim. Any other connect error - a permissions
/// problem, a descriptor limit - says nothing about whether a healthy primary is
/// alive, so its socket file is left alone and the bind is simply attempted.
pub fn claim(socket_path: &Path) -> anyhow::Result<Singleton> {
    match UnixStream::connect(socket_path) {
        Ok(_) => return Ok(Singleton::AlreadyRunning),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            let _ = std::fs::remove_file(socket_path);
        }
        Err(_) => {}
    }

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
/// read, garbage bytes, a connection that closes early, a client that connects and
/// then never writes - is silently dropped.
///
/// This is a single-threaded loop, so it must never block on one client: every
/// accepted stream gets a read timeout, and a failed `accept` pauses briefly rather
/// than spinning at full speed (`incoming()` never ends, so a persistent error such
/// as descriptor exhaustion would otherwise busy-loop this thread forever).
pub fn spawn_accept_loop(listener: UnixListener, on_show: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
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
    fn claiming_a_path_with_a_stale_socket_file_recovers_and_becomes_primary() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("gui.sock");

        // Dropping a `UnixListener` closes the socket but leaves its file on disk -
        // exactly what a crashed primary instance leaves behind.
        let dead = UnixListener::bind(&socket_path).unwrap();
        drop(dead);
        assert!(
            socket_path.exists(),
            "expected the dropped listener to leave a stale socket file behind"
        );

        match claim(&socket_path).unwrap() {
            Singleton::Primary(_listener) => {}
            Singleton::AlreadyRunning => {
                panic!("expected Primary after recovering a stale socket file")
            }
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
