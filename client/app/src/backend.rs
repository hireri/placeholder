//! Daemon plumbing (spawn, handshake, HTTP calls, sign-in/out). Not wired into the shell UI yet.
#![allow(dead_code)]

use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static DAEMON: Mutex<Option<Child>> = Mutex::new(None);

#[derive(Clone)]
pub struct Conn {
    pub url: String,
    pub token: String,
}

#[derive(Deserialize, Clone, Default)]
pub struct User {
    pub name: String,
    pub subtitle: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub logged_in: bool,
    pub user: Option<User>,
    pub error: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct Status {
    pub youtube: Service,
    pub spotify: Service,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub status: Status,
    pub youtube_playlists: Option<usize>,
    pub spotify_playlists: Option<usize>,
}

pub fn data_dir() -> PathBuf {
    std::env::var("METROLIST_DATA").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/metrolist-desktop")
    })
}

pub fn daemon_path() -> PathBuf {
    std::env::var("METROLIST_DAEMON").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../daemon/build/install/daemon/bin/daemon"))
    })
}

pub fn login_path() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("metrolist-login"))).unwrap_or_else(|| "metrolist-login".into())
}

/// Starts the JVM daemon and reads its `{"port":..,"token":..}` handshake line.
pub fn start_daemon() -> Result<Conn, String> {
    let mut child = Command::new(daemon_path())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start daemon ({}): {e}", daemon_path().display()))?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).map_err(|e| e.to_string())?;
    *DAEMON.lock().unwrap() = Some(child);
    let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| format!("bad handshake {line:?}: {e}"))?;
    Ok(Conn { url: format!("http://127.0.0.1:{}", v["port"]), token: v["token"].as_str().unwrap_or_default().to_string() })
}

pub fn stop_daemon() {
    if let Some(mut c) = DAEMON.lock().unwrap().take() {
        let _ = c.kill();
    }
}

pub fn call(conn: &Conn, method: &str, path: &str) -> Result<ureq::Response, String> {
    ureq::request(method, &format!("{}{}", conn.url, path)).set("X-Metrolist-Token", &conn.token).call().map_err(|e| e.to_string())
}

pub fn fetch_all(conn: &Conn) -> Result<Snapshot, String> {
    let status: Status = call(conn, "GET", "/auth/status")?.into_json().map_err(|e| e.to_string())?;
    let count = |path: &str| call(conn, "GET", path).ok().and_then(|r| r.into_json::<Vec<serde_json::Value>>().ok()).map(|v| v.len());
    Ok(Snapshot {
        youtube_playlists: if status.youtube.logged_in { count("/youtube/playlists") } else { None },
        spotify_playlists: if status.spotify.logged_in { count("/spotify/playlists") } else { None },
        status,
    })
}

#[derive(Clone, Copy)]
pub enum Svc {
    YouTube,
    Spotify,
}

impl Svc {
    pub fn key(self) -> &'static str {
        match self { Svc::YouTube => "youtube", Svc::Spotify => "spotify" }
    }
}

/// Blocking: runs the wry login popup, then re-reads account state.
pub fn sign_in(conn: &Conn, svc: Svc) -> Result<Snapshot, String> {
    let st = Command::new(login_path())
        .arg(svc.key()).env("METROLIST_URL", &conn.url).env("METROLIST_TOKEN", &conn.token).status()
        .map_err(|e| format!("cannot start login helper: {e}"))?;
    if st.success() { fetch_all(conn) } else { Err("sign-in cancelled".into()) }
}

/// Blocking: drops stored credentials and the webview profile, then re-reads account state.
pub fn sign_out(conn: &Conn, svc: Svc) -> Result<Snapshot, String> {
    call(conn, "DELETE", &format!("/auth/{}", svc.key()))?;
    let _ = std::fs::remove_dir_all(data_dir().join("webview").join(svc.key()));
    fetch_all(conn)
}
