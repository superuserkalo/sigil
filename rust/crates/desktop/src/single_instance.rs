//! One Sigil per user. A second launch (the system starts one for every clicked link) hands its
//! arguments to the running Sigil, which comes to the front, and exits before it opens a window.
//!
//! - Windows and Linux: Rune's `single_instance` (a named pipe per user session on Windows, a
//!   Unix socket in `$XDG_RUNTIME_DIR` with a lock file on Linux). It forwards `scheme://`
//!   links to `App::on_open_url` and brings the window forward on a plain relaunch.
//! - macOS: LaunchServices already keeps one process per app bundle and sends links as Apple
//!   events. Launching the executable itself (a terminal, `open -n`) bypasses that, so Sigil
//!   claims a lock file and a Unix socket in its data folder: `sigil.lock` (an exclusive
//!   `flock`, so two first launches can't both win) and `sigil.sock` (mode 600).
//!
//! Nothing here touches the network. `Sigil cli`, `Sigil agent-host` and headless runs
//! (`--snap`, `--tree`, scripts) never claim: `main` dispatches them first.

use std::sync::Mutex;

use rune::AppHandle;

use crate::app::State;

/// The app's reverse-DNS id, as in the macOS bundle (`CFBundleIdentifier`).
pub const APP_ID: &str = "dev.deadlock.harness";

/// The running app, once it exists, and launches that arrived before it did.
struct Inbox {
    handle: Option<AppHandle<State>>,
    waiting: Vec<Vec<String>>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox {
    handle: None,
    waiting: Vec::new(),
});

/// Before any window: true to go on and open one; false when a running Sigil took this launch
/// (return from `main`). Also true when the running one can't be reached, so a launch is never
/// lost to a hung process.
pub fn claim() -> bool {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        rune::single_instance(APP_ID)
    }
    #[cfg(target_os = "macos")]
    {
        // Rune's call waits for the process a `Tx::relaunch` replaces to exit; then the lock is free.
        rune::single_instance(APP_ID);
        let args: Vec<String> = std::env::args().skip(1).collect();
        !matches!(
            unix::claim_in(
                &crate::data::data_dir(),
                &args,
                std::sync::Arc::new(deliver)
            ),
            unix::Claim::Handed
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        true
    }
}

/// Once the app is built: launches from now on reach it. Also delivers this launch's own links
/// that Rune does not (every `sigil:` argument on macOS; GameBanana's opaque `sigil:https://...`
/// form typed by hand on Windows and Linux, where the registered command wraps it already).
pub fn attach(handle: AppHandle<State>) {
    let own: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| {
            crate::deeplink::is_sigil_link(a) && (cfg!(target_os = "macos") || !rune_forwards(a))
        })
        .collect();
    let waiting = {
        let mut inbox = INBOX.lock().unwrap_or_else(|e| e.into_inner());
        inbox.handle = Some(handle.clone());
        std::mem::take(&mut inbox.waiting)
    };
    for link in own {
        handle.update(move |s, tx| crate::deeplink::on_open_url(s, &link, tx));
    }
    for args in waiting {
        forward(&handle, args);
    }
}

/// Whether Rune's launch-argument and hand-off code passes this argument on: `scheme://...`.
fn rune_forwards(arg: &str) -> bool {
    arg.split_once(':')
        .is_some_and(|(_, rest)| rest.starts_with("//"))
}

/// Brings the window to the front, restored. A link should land where the player can see it.
pub fn bring_forward() {
    if let Some(handle) = INBOX
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handle
        .as_ref()
    {
        handle.open();
    }
}

/// A later launch's arguments, from the socket's thread.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn deliver(args: Vec<String>) {
    let handle = {
        let mut inbox = INBOX.lock().unwrap_or_else(|e| e.into_inner());
        match inbox.handle.clone() {
            Some(handle) => handle,
            None => {
                inbox.waiting.push(args);
                return;
            }
        }
    };
    forward(&handle, args);
}

/// Its links go to the link handler; a launch without links only brings the window forward.
fn forward(handle: &AppHandle<State>, args: Vec<String>) {
    let links: Vec<String> = args
        .into_iter()
        .filter(|a| crate::deeplink::is_sigil_link(a))
        .collect();
    if links.is_empty() {
        handle.open();
    }
    for link in links {
        handle.update(move |s, tx| crate::deeplink::on_open_url(s, &link, tx));
    }
}

/// The lock-and-socket claim. Unix in general so the tests run on any Unix; only macOS uses it.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod unix {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    pub const LOCK: &str = "sigil.lock";
    pub const SOCKET: &str = "sigil.sock";
    /// Opens every message, so a stray client's bytes are not taken for arguments.
    const MAGIC: &[u8] = b"sigil-instance 1\n";
    /// The most a launch's arguments may take.
    const MAX_BODY: usize = 64 * 1024;
    /// The running Sigil took the launch.
    const ACCEPTED: u8 = b'A';
    /// How long a launch keeps trying to reach a first Sigil that is still starting.
    const REACH_FOR: Duration = Duration::from_secs(2);
    /// How long either side waits on the other once connected.
    const ANSWER_WITHIN: Duration = Duration::from_millis(500);
    /// `sun_path` holds 104 bytes on macOS (108 on Linux), the NUL included.
    const MAX_SOCKET_PATH: usize = 103;

    pub type Sink = Arc<dyn Fn(Vec<String>) + Send + Sync>;

    #[derive(Debug, PartialEq, Eq)]
    pub enum Claim {
        /// This process is the first: it holds the lock and serves the socket.
        First,
        /// A running Sigil took this launch: exit.
        Handed,
        /// Neither (no data folder, a path too long, a first Sigil that never answered): run.
        Unclaimed,
    }

    /// The lock file stays open (and locked) for the life of the process; the OS lets go of it
    /// when the process ends, however it ends.
    static HELD: Mutex<Vec<File>> = Mutex::new(Vec::new());

    fn hold(lock: File) {
        HELD.lock().unwrap_or_else(|e| e.into_inner()).push(lock);
    }

    pub fn claim_in(dir: &Path, args: &[String], sink: Sink) -> Claim {
        let socket = dir.join(SOCKET);
        if socket.as_os_str().len() > MAX_SOCKET_PATH || std::fs::create_dir_all(dir).is_err() {
            return Claim::Unclaimed;
        }
        let Ok(lock) = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK))
        else {
            return Claim::Unclaimed;
        };
        let deadline = Instant::now() + REACH_FOR;
        loop {
            match lock.try_lock() {
                Ok(()) => return serve_first(lock, &socket, sink),
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(_)) => return Claim::Unclaimed,
            }
            // Another Sigil holds the lock: hand it this launch once its socket answers.
            if let Ok(stream) = UnixStream::connect(&socket) {
                return if send(stream, args) {
                    Claim::Handed
                } else {
                    Claim::Unclaimed
                };
            }
            if Instant::now() >= deadline {
                return Claim::Unclaimed;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Holds the lock and serves the socket from a thread of its own, so the launch pays only
    /// for the lock (a later launch retries until the socket answers).
    fn serve_first(lock: File, socket: &Path, sink: Sink) -> Claim {
        hold(lock);
        let socket = socket.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("sigil-instance".into())
            .spawn(move || {
                // A socket left by a Sigil that died: the lock says nobody serves it.
                let _ = std::fs::remove_file(&socket);
                let Ok(listener) = UnixListener::bind(&socket) else {
                    return;
                };
                let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
                for stream in listener.incoming().flatten() {
                    if let Some(args) = receive(stream) {
                        sink(args);
                    }
                }
            });
        Claim::First
    }

    /// `MAGIC`, the body's length (u32, little-endian), the arguments NUL-separated; then one
    /// byte back.
    pub fn message(args: &[String]) -> Vec<u8> {
        let body = args.join("\0").into_bytes();
        let body = &body[..body.len().min(MAX_BODY)];
        let mut message = MAGIC.to_vec();
        message.extend((body.len() as u32).to_le_bytes());
        message.extend(body);
        message
    }

    fn send(mut stream: UnixStream, args: &[String]) -> bool {
        let _ = stream.set_write_timeout(Some(ANSWER_WITHIN));
        let _ = stream.set_read_timeout(Some(ANSWER_WITHIN));
        if stream.write_all(&message(args)).is_err() {
            return false;
        }
        let mut answer = [0u8; 1];
        stream.read_exact(&mut answer).is_ok() && answer[0] == ACCEPTED
    }

    fn receive(mut stream: UnixStream) -> Option<Vec<String>> {
        let _ = stream.set_read_timeout(Some(ANSWER_WITHIN));
        let _ = stream.set_write_timeout(Some(ANSWER_WITHIN));
        let mut magic = [0u8; MAGIC.len()];
        stream.read_exact(&mut magic).ok()?;
        if magic != MAGIC {
            return None;
        }
        let mut len = [0u8; 4];
        stream.read_exact(&mut len).ok()?;
        let len = u32::from_le_bytes(len) as usize;
        if len > MAX_BODY {
            return None;
        }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).ok()?;
        let args = std::str::from_utf8(&body)
            .ok()?
            .split('\0')
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .collect();
        let _ = stream.write_all(&[ACCEPTED]);
        Some(args)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Mutex;
        use std::sync::mpsc;

        fn temp_dir(name: &str) -> std::path::PathBuf {
            // Short on purpose: the socket path has to fit `sun_path`.
            let dir = std::env::temp_dir().join(format!("sgl-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            dir
        }

        #[test]
        fn a_second_launch_hands_its_arguments_to_the_first() {
            let dir = temp_dir("hand");
            let (tx, rx) = mpsc::channel();
            let tx = Mutex::new(tx);
            // The data folder and lock file exist after Sigil's first run.
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(LOCK), b"").unwrap();
            let started = Instant::now();
            let first = claim_in(
                &dir,
                &[],
                Arc::new(move |args| tx.lock().unwrap().send(args).unwrap()),
            );
            let claim_cost = started.elapsed();
            assert_eq!(first, Claim::First);
            assert!(
                claim_cost < Duration::from_millis(50),
                "first claim took {claim_cost:?}"
            );

            // A second claim in the same process opens its own lock file description, which
            // `flock` treats as another owner, as it would another process.
            let link = "sigil:https://gamebanana.com/mmdl/12,Mod,42".to_string();
            let started = Instant::now();
            let second = claim_in(
                &dir,
                std::slice::from_ref(&link),
                Arc::new(|_| panic!("not the first")),
            );
            let hand_cost = started.elapsed();
            assert_eq!(second, Claim::Handed);
            assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), vec![link]);
            let mode = std::fs::metadata(dir.join(SOCKET))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
            eprintln!("single instance: first claim {claim_cost:?}, hand-off {hand_cost:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn stray_bytes_are_not_arguments() {
            let dir = temp_dir("stray");
            let (tx, rx) = mpsc::channel::<Vec<String>>();
            let tx = Mutex::new(tx);
            assert_eq!(
                claim_in(
                    &dir,
                    &[],
                    Arc::new(move |a| tx.lock().unwrap().send(a).unwrap())
                ),
                Claim::First
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while UnixStream::connect(dir.join(SOCKET)).is_err() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            let mut stream = UnixStream::connect(dir.join(SOCKET)).unwrap();
            stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
            drop(stream);
            // An oversized length is refused before any body is read.
            let mut stream = UnixStream::connect(dir.join(SOCKET)).unwrap();
            let mut bad = MAGIC.to_vec();
            bad.extend((u32::MAX).to_le_bytes());
            stream.write_all(&bad).unwrap();
            drop(stream);
            assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_path_too_long_for_a_socket_runs_unclaimed() {
            let dir = std::env::temp_dir().join("s".repeat(120));
            assert_eq!(claim_in(&dir, &[], Arc::new(|_| {})), Claim::Unclaimed);
        }

        #[test]
        fn messages_round_trip() {
            let args = vec!["a".to_string(), "sigil://install/mod/7".to_string()];
            let message = message(&args);
            assert!(message.starts_with(MAGIC));
            let len = u32::from_le_bytes(message[MAGIC.len()..MAGIC.len() + 4].try_into().unwrap())
                as usize;
            assert_eq!(len, message.len() - MAGIC.len() - 4);
        }
    }
}
