use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

const SHOW_MESSAGE: &[u8; 4] = b"show";

pub enum Singleton {
    Primary(UnixListener, File),
    AlreadyRunning,
}

/// Claims `socket_path` as the single running instance, coordinated through an
/// exclusive, non-blocking `flock` on `lock_path`.
///
/// The lock - not the socket file - is the source of truth for "is a primary
/// instance alive": a `flock` is released by the kernel the instant the process
/// holding it exits, for any reason, including a crash. That removes the race a
/// plain connect-then-bind approach has (two processes launched close enough
/// together can both see "nobody's listening" and both try to become primary), and
/// it also makes stale-socket cleanup unconditionally safe - once this process
/// holds the lock exclusively, nothing else can possibly be listening on
/// `socket_path`, so any file left there is guaranteed dead and safe to remove.
pub fn claim(socket_path: &Path, lock_path: &Path) -> anyhow::Result<Singleton> {
    let lock_file = OpenOptions::new().create(true).write(true).open(lock_path)?;
    if lock_file.try_lock().is_err() {
        return Ok(Singleton::AlreadyRunning);
    }

    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    Ok(Singleton::Primary(listener, lock_file))
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

    fn paths(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        (dir.join("gui.sock"), dir.join("gui.lock"))
    }

    #[test]
    fn claiming_a_fresh_path_becomes_the_primary_instance() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(_listener, _lock_file) => {}
            Singleton::AlreadyRunning => panic!("expected Primary for a fresh socket path"),
        }
    }

    #[test]
    fn claiming_an_already_claimed_path_detects_the_running_instance() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let _primary = claim(&socket_path, &lock_path).unwrap();

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::AlreadyRunning => {}
            Singleton::Primary(..) => panic!("expected AlreadyRunning while the primary is alive"),
        }
    }

    #[test]
    fn dropping_the_primary_releases_the_lock_for_the_next_claim() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let primary = claim(&socket_path, &lock_path).unwrap();
        drop(primary);

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(..) => {}
            Singleton::AlreadyRunning => {
                panic!("expected Primary after the previous primary's lock was released")
            }
        }
    }

    #[test]
    fn claiming_a_path_with_a_stale_socket_file_recovers_and_becomes_primary() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        // Dropping a `UnixListener` closes the socket but leaves its file on disk -
        // exactly what a crashed primary instance leaves behind. Crucially, its lock
        // is also released by the crash (simulated here by simply never taking it),
        // so a fresh `claim` must recover cleanly.
        let dead = UnixListener::bind(&socket_path).unwrap();
        drop(dead);
        assert!(
            socket_path.exists(),
            "expected the dropped listener to leave a stale socket file behind"
        );

        match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(..) => {}
            Singleton::AlreadyRunning => {
                panic!("expected Primary after recovering a stale socket file")
            }
        }
    }

    #[test]
    fn notifying_the_primary_reaches_its_accept_loop() {
        let dir = tempfile::tempdir().unwrap();
        let (socket_path, lock_path) = paths(dir.path());

        let listener = match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(listener, _lock_file) => listener,
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
        let (socket_path, lock_path) = paths(dir.path());

        let listener = match claim(&socket_path, &lock_path).unwrap() {
            Singleton::Primary(listener, _lock_file) => listener,
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
