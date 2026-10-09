//! Discord Rich Presence over the local Discord IPC socket.
//!
//! The worker thread owns a [`DiscordIpcClient`] and applies the latest
//! requested activity. It reconnects with a backoff while Discord is absent
//! and never blocks the caller.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

use crate::instance::InstanceMetadata;

const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const CONNECT_BACKOFF: Duration = Duration::from_secs(15);

/// Presence diagnostics, enabled with `CTM_PRESENCE_LOG=1`. Goes to stderr so
/// a headless launcher never spams the UI.
fn presence_log(message: &str) {
    if std::env::var_os("CTM_PRESENCE_LOG").map_or(true, |v| v == "0") {
        return;
    }
    eprintln!("[presence] {message}");
}

/// One Discord activity: two text lines plus an optional elapsed timer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceActivity {
    pub details: String,
    pub state: String,
    pub started_unix: Option<u64>,
}

impl PresenceActivity {
    pub fn new(details: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            details: details.into(),
            state: state.into(),
            started_unix: None,
        }
    }

    /// Start the "elapsed since" counter at the given unix time (seconds).
    pub fn started_at(mut self, unix: u64) -> Self {
        self.started_unix = Some(unix);
        self
    }
}

/// Second activity line for a running instance: `Fabric 1.21.1 · Pack Name`.
///
/// The pack name is skipped when it just repeats the instance name.
pub fn instance_state(meta: &InstanceMetadata) -> String {
    let mut line = format!("{} {}", meta.loader.label(), meta.game_version);
    if let Some(pack) = meta.modpack.as_ref() {
        let pack_name = pack.name.trim();
        if !pack_name.is_empty() && !pack_name.eq_ignore_ascii_case(meta.name.trim()) {
            line.push_str(" · ");
            line.push_str(pack_name);
        }
    }
    line
}

enum Command {
    Set(PresenceActivity),
    Clear,
    Shutdown,
}

/// Sending side of the presence worker. Cheap to clone-free send; every call
/// is fire-and-forget and never fails visibly.
pub struct PresenceHandle {
    tx: Sender<Command>,
}

impl PresenceHandle {
    pub fn set(&self, activity: PresenceActivity) {
        let _ = self.tx.send(Command::Set(activity));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(Command::Clear);
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

/// Spawn the background presence worker for a Discord application id.
///
/// The thread exits when the handle is dropped, when it receives
/// [`PresenceHandle::shutdown`], or when the caller goes away.
pub fn spawn(app_id: &'static str) -> PresenceHandle {
    let (tx, rx) = mpsc::channel();
    // If the thread cannot be spawned the closure (and `rx`) are dropped, so
    // every later send just fails silently.
    let _ = thread::Builder::new()
        .name("discord-presence".into())
        .spawn(move || worker(app_id, rx));
    PresenceHandle { tx }
}

fn worker(app_id: &str, rx: Receiver<Command>) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut desired: Option<PresenceActivity> = None;
    let mut dirty = false;
    let mut retry_at = Instant::now();

    loop {
        match rx.recv_timeout(RETRY_INTERVAL) {
            Ok(Command::Set(activity)) => {
                presence_log(&format!("set requested: {activity:?}"));
                if desired.as_ref() != Some(&activity) {
                    desired = Some(activity);
                    dirty = true;
                }
            }
            Ok(Command::Clear) => {
                presence_log("clear requested");
                if desired.is_some() {
                    desired = None;
                    dirty = true;
                }
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                presence_log("shutting down");
                if let Some(mut c) = client.take() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        if !dirty || Instant::now() < retry_at {
            continue;
        }

        if client.is_none() {
            presence_log(&format!("connecting to app id {app_id}"));
            let mut fresh = DiscordIpcClient::new(app_id);
            match fresh.connect() {
                Ok(()) => {
                    presence_log("connected");
                    client = Some(fresh);
                }
                Err(err) => {
                    presence_log(&format!("connect failed: {err}; retrying in {CONNECT_BACKOFF:?}"));
                    retry_at = Instant::now() + CONNECT_BACKOFF;
                    continue;
                }
            }
        }

        let outcome = match desired.as_ref() {
            Some(activity) => {
                presence_log("sending activity");
                client.as_mut().map(|c| c.set_activity(build(activity)))
            }
            None => {
                presence_log("clearing activity");
                client.as_mut().map(|c| c.clear_activity())
            }
        };
        match outcome {
            Some(Ok(())) => {
                presence_log("activity accepted");
                dirty = false;
            }
            Some(Err(err)) => {
                presence_log(&format!("activity rejected: {err}; reconnecting in {CONNECT_BACKOFF:?}"));
                if let Some(mut c) = client.take() {
                    let _ = c.close();
                }
                retry_at = Instant::now() + CONNECT_BACKOFF;
            }
            None => {}
        }
    }
}

fn build(activity: &PresenceActivity) -> activity::Activity<'_> {
    let mut built = activity::Activity::new().details(activity.details.as_str());
    // Discord rejects empty strings, so only attach a non-empty state line.
    if !activity.state.is_empty() {
        built = built.state(activity.state.as_str());
    }
    if let Some(start) = activity.started_unix {
        built = built.timestamps(activity::Timestamps::new().start(start as i64));
    }
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    const APP_ID: &str = "123456789012345678";

    /// Env-var tweaks are process-wide, so the IPC tests must not overlap.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Point the pipe search at a temp dir and hide real Discord sockets
    /// (flatpak/snap live under XDG subpaths, TMP/TEMP at top level).
    struct EnvGuard {
        previous: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(dir: &std::path::Path) -> Self {
            let keys = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP", "SNAP"];
            let previous: Vec<_> = keys
                .iter()
                .map(|&key| (key, std::env::var(key).ok()))
                .collect();
            for key in ["TMPDIR", "TMP", "TEMP", "SNAP"] {
                std::env::remove_var(key);
            }
            std::env::set_var("XDG_RUNTIME_DIR", dir);
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.previous.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn temp_runtime_dir() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ctm-presence-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_frame(stream: &mut UnixStream) -> (u32, serde_json::Value) {
        let mut header = [0u8; 8];
        stream.read_exact(&mut header).unwrap();
        let op = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let mut body = vec![0u8; len as usize];
        stream.read_exact(&mut body).unwrap();
        (op, serde_json::from_slice(&body).unwrap())
    }

    fn write_frame(stream: &mut UnixStream, op: u32, body: &serde_json::Value) {
        let payload = body.to_string();
        let mut header = Vec::new();
        header.extend_from_slice(&op.to_le_bytes());
        header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        stream.write_all(&header).unwrap();
        stream.write_all(payload.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn instance_state_line_labels_loader_and_pack() {
        let mut meta = InstanceMetadata::new("inst-1", "My Pack", "1.21.1", crate::instance::LoaderType::Fabric, None);
        assert_eq!(instance_state(&meta), "Fabric 1.21.1");

        meta.modpack = Some(crate::instance::ModpackOrigin {
            name: "Fabulously Optimized".into(),
            version: None,
            source: None,
        });
        assert_eq!(instance_state(&meta), "Fabric 1.21.1 · Fabulously Optimized");

        meta.name = "fabulously optimized".into();
        assert_eq!(
            instance_state(&meta),
            "Fabric 1.21.1",
            "pack name repeating the instance name must be skipped"
        );
    }

    #[test]
    fn worker_handshakes_sets_and_clears_against_mock_discord() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_runtime_dir();
        let listener = UnixListener::bind(dir.join("discord-ipc-0")).unwrap();
        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();

            let (op, handshake) = read_frame(&mut stream);
            assert_eq!(op, 0, "first frame must be the handshake");
            assert_eq!(handshake["v"], 1);
            assert_eq!(handshake["client_id"], APP_ID);
            write_frame(
                &mut stream,
                1,
                &serde_json::json!({"cmd": "EVENT", "evt": "READY", "data": {"v": 1}}),
            );

            for _ in 0..3 {
                let (op, payload) = read_frame(&mut stream);
                assert_eq!(op, 1, "activity frames use opcode 1");
                sink.lock().unwrap().push(payload);
            }
        });

        let _env = EnvGuard::new(&dir);
        let handle = spawn(APP_ID);
        // Mirrors the launcher's idle activity: a fixed English details line
        // with no state, plus one stateful activity to cover the playing path.
        handle.set(PresenceActivity::new("Idle in CTMLauncher", ""));
        handle.set(
            PresenceActivity::new("Playing My Pack", "Fabric 1.21.1").started_at(1_700_000_000),
        );
        handle.clear();
        handle.shutdown();
        server.join().unwrap();
        drop(_env);

        let frames = received.lock().unwrap();
        assert_eq!(frames.len(), 3);

        let idle = &frames[0];
        assert_eq!(idle["cmd"], "SET_ACTIVITY");
        let idle_activity = &idle["args"]["activity"];
        assert_eq!(idle_activity["details"], "Idle in CTMLauncher");
        assert!(
            idle_activity.get("state").is_none(),
            "empty idle state must be omitted, not sent as an empty string"
        );

        let playing = &frames[1];
        assert_eq!(playing["cmd"], "SET_ACTIVITY");
        let playing_activity = &playing["args"]["activity"];
        assert_eq!(playing_activity["details"], "Playing My Pack");
        assert_eq!(playing_activity["state"], "Fabric 1.21.1");
        assert_eq!(playing_activity["timestamps"]["start"], 1_700_000_000);

        let cleared = &frames[2];
        assert_eq!(cleared["cmd"], "SET_ACTIVITY");
        assert!(cleared["args"]["activity"].is_null());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worker_survives_missing_discord_and_quits_on_drop() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_runtime_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let _env = EnvGuard::new(&dir);

        let handle = spawn(APP_ID);
        handle.set(PresenceActivity::new("details", "state"));
        handle.set(PresenceActivity::new("details", "state"));
        handle.clear();
        handle.shutdown();
        drop(handle);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
