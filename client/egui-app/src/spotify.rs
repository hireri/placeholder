//! Direct Spotify playback and Spotify Connect, both through `serafin-login spotify-play` (see its `spotify_play.rs` and
//! `spotify_connect.rs`): audio comes back as raw PCM, Connect takes our state on stdin and gives commands on stdout.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

/// The setting: Spotify songs stream from Spotify (Premium) instead of playing their YouTube Music match.
static DIRECT: AtomicBool = AtomicBool::new(false);
/// The helper process (holding its stdin open keeps it alive), its port and its key.
static HELPER: Mutex<Option<(Child, u16, String)>> = Mutex::new(None);
/// Counts helpers started, so a new one is told the state the last one knew.
static STARTED: AtomicU64 = AtomicU64::new(0);
static STARTING: AtomicBool = AtomicBool::new(false);
/// Connect commands the app has not handled yet.
static COMMANDS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

pub fn set_direct(on: bool) {
    DIRECT.store(on, Relaxed);
}

pub fn direct() -> bool {
    DIRECT.load(Relaxed)
}

/// Port and key of the running helper, starting it if needed.
fn helper() -> Result<(u16, String), String> {
    let mut held = HELPER.lock().unwrap();
    if let Some((child, port, key)) = held.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return Ok((*port, key.clone()));
        }
    }
    let mut cmd = Command::new(crate::backend::login_helper());
    cmd.arg("spotify-play").env("METROLIST_DATA", crate::backend::data_dir()).stdin(Stdio::piped()).stdout(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| format!("cannot start the Spotify helper: {e}"))?;
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).map_err(|e| e.to_string())?;
    let (port, key) = line.trim().split_once(' ').and_then(|(p, k)| Some((p.parse().ok()?, k.to_string()))).ok_or("the Spotify helper is out of date")?;
    *held = Some((child, port, key.clone()));
    STARTED.fetch_add(1, Relaxed);
    std::thread::spawn(move || {
        for line in out.lines().map_while(Result::ok) {
            if let Ok(command) = serde_json::from_str(&line) {
                COMMANDS.lock().unwrap().push(command);
                crate::tray::wake();
            }
        }
    });
    Ok((port, key))
}

/// Which helper is running (0 before the first). Never blocks the UI.
pub fn generation() -> u64 {
    STARTED.load(Relaxed)
}

/// Hands the helper the app's Connect state. False when there is no helper to take it yet; with `start`, one is started
/// in the background and the caller tries again later.
pub fn publish(state: &str, start: bool) -> bool {
    let Ok(mut held) = HELPER.try_lock() else { return false };
    if let Some((child, ..)) = held.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return child.stdin.as_mut().is_some_and(|stdin| writeln!(stdin, "{state}").and_then(|()| stdin.flush()).is_ok());
        }
    }
    drop(held);
    if start && !STARTING.swap(true, Relaxed) {
        std::thread::spawn(|| {
            if let Err(e) = helper() {
                eprintln!("spotify connect: {e}");
                // A missing or broken helper is not retried every frame.
                std::thread::sleep(Duration::from_secs(30));
            }
            STARTING.store(false, Relaxed);
            crate::tray::wake();
        });
    }
    false
}

pub fn take_commands() -> Vec<serde_json::Value> {
    std::mem::take(&mut *COMMANDS.lock().unwrap())
}

/// Drops the helper and its Spotify session, for sign-in and sign-out.
pub fn stop() {
    if let Some((mut child, ..)) = HELPER.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Blocking: 44.1 kHz stereo f32le of track `id` from `start` seconds. `gain` is the loudness pregain in dB, or none to leave levels alone.
pub fn open(id: &str, start: f32, gain: Option<f32>) -> Result<BufReader<TcpStream>, String> {
    let (port, key) = helper()?;
    let io = |e: std::io::Error| format!("Spotify stream: {e}");
    let mut conn = TcpStream::connect(("127.0.0.1", port)).map_err(io)?;
    conn.set_read_timeout(Some(Duration::from_secs(40))).map_err(io)?;
    let gain = gain.map_or("-".into(), |g| g.to_string());
    writeln!(conn, "{key} {} {} {gain}", id.trim_start_matches("spotify:"), (start.max(0.) * 1000.) as u32).map_err(io)?;
    let mut conn = BufReader::new(conn);
    let mut status = String::new();
    (&mut conn).take(512).read_line(&mut status).map_err(io)?;
    match status.trim_end() {
        "OK" => Ok(conn),
        other => Err(other.strip_prefix("ERR ").unwrap_or("Spotify stream closed").to_string()),
    }
}
