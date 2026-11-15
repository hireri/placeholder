//! Daemon client: spawns the JVM daemon on first use and reads its HTTP API (only `/lyrics` is consumed so far).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

static DAEMON: Mutex<Option<Child>> = Mutex::new(None);
static CONN: OnceLock<Result<(String, String), String>> = OnceLock::new();

pub struct Lyrics {
    pub provider: String,
    /// `(start seconds, text)`; starts are all 0 when `synced` is false.
    pub lines: Vec<(f32, String)>,
    pub synced: bool,
}

/// Release layout: `daemon/` (installDist) and `jre/` sit next to the executable; dev falls back to the Gradle build tree.
fn exe_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Into::into))
}

fn daemon_command() -> Command {
    let script = if cfg!(windows) { "daemon.bat" } else { "daemon" };
    let bundled = exe_dir().map(|d| d.join("daemon/bin").join(script)).filter(|p| p.exists());
    let path = std::env::var_os("METROLIST_DAEMON").map(Into::into).or(bundled).unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../daemon/build/install/daemon/bin").join(script)
    });
    let mut cmd = Command::new(path);
    if let Some(jre) = exe_dir().map(|d| d.join("jre")).filter(|p| p.exists()) {
        cmd.env("JAVA_HOME", jre);
    }
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// Blocking on first call: starts the daemon and reads its `{"port":..,"token":..}` handshake line.
fn conn() -> Result<&'static (String, String), String> {
    CONN.get_or_init(|| {
        let mut child = daemon_command().stdout(Stdio::piped()).spawn().map_err(|e| format!("cannot start daemon: {e}"))?;
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).map_err(|e| e.to_string())?;
        *DAEMON.lock().unwrap() = Some(child);
        let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| format!("bad handshake {line:?}: {e}"))?;
        Ok((format!("http://127.0.0.1:{}", v["port"]), v["token"].as_str().unwrap_or_default().to_string()))
    })
    .as_ref()
    .map_err(Clone::clone)
}

pub fn stop() {
    if let Some(mut c) = DAEMON.lock().unwrap().take() {
        let _ = c.kill();
    }
}

/// Blocking; call from a worker thread. `Ok(None)` means no provider has lyrics.
pub fn lyrics(id: &str, title: &str, artist: &str, duration: u32, album: &str) -> Result<Option<Lyrics>, String> {
    let (url, token) = conn()?;
    let r = ureq::get(&format!("{url}/lyrics")).set("X-Metrolist-Token", token)
        .query("id", id).query("title", title).query("artist", artist).query("duration", &duration.to_string()).query("album", album).call();
    match r {
        Ok(r) => {
            let v: serde_json::Value = r.into_json().map_err(|e| e.to_string())?;
            let (provider, lrc) = (v["provider"].as_str().unwrap_or_default(), v["lrc"].as_str().unwrap_or_default());
            let (lines, synced) = parse_lrc(lrc);
            Ok(Some(Lyrics { provider: provider.into(), lines, synced }))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// LRC (`[mm:ss.xx]text`, several stamps per line allowed, `<..>` word tags dropped) or plain text.
fn parse_lrc(lrc: &str) -> (Vec<(f32, String)>, bool) {
    let mut out = vec![];
    let mut synced = false;
    for raw in lrc.lines() {
        let (mut rest, mut stamps) = (raw.trim(), vec![]);
        while let Some((tag, after)) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            let stamp = tag.split_once(':').and_then(|(m, s)| Some(m.parse::<f32>().ok()? * 60. + s.parse::<f32>().ok()?));
            // Metadata tags like [ar:..] carry no lyric text.
            rest = if stamp.is_some() { after } else { "" };
            stamps.extend(stamp);
        }
        let mut text = String::new();
        let mut in_tag = false;
        for ch in rest.chars() {
            match ch { '<' => in_tag = true, '>' => in_tag = false, c if !in_tag => text.push(c), _ => {} }
        }
        let text = text.trim().to_string();
        if stamps.is_empty() {
            out.push((0., text));
        } else {
            synced = true;
            out.extend(stamps.into_iter().map(|s| (s, text.clone())));
        }
    }
    if synced {
        out.retain(|(_, t)| !t.is_empty());
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    (out, synced)
}

/// Percent-encodes a query value.
pub fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Blocking GET of a daemon JSON endpoint; call from a worker thread.
pub fn get<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let (url, token) = conn()?;
    match ureq::get(&format!("{url}{path}")).set("X-Metrolist-Token", token).call() {
        Ok(r) => r.into_json().map_err(|e| e.to_string()),
        Err(e) => Err(failure(e)),
    }
}

/// URL of a song's video-only stream starting at `start_ms`, and the token its request must carry (for GStreamer to fetch).
#[cfg(feature = "video")]
pub fn video_url(id: &str, start_ms: u64, max_height: u32) -> Result<(String, String), String> {
    let (url, token) = conn()?;
    Ok((format!("{url}/video/{}?startMs={start_ms}&maxHeight={max_height}", enc(id)), token.clone()))
}

/// Blocking POST with no body (the daemon answers 204); call from a worker thread.
pub fn post(path: &str) -> Result<(), String> {
    let (url, token) = conn()?;
    ureq::post(&format!("{url}{path}")).set("X-Metrolist-Token", token).call().map(drop).map_err(failure)
}

/// The daemon's `error` message for an HTTP failure, else the transport error.
fn failure(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(_, r) => r.into_json::<serde_json::Value>().ok().and_then(|v| v["error"].as_str().map(String::from)).unwrap_or_else(|| "request failed".into()),
        e => e.to_string(),
    }
}

#[derive(serde::Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Service {
    pub logged_in: bool,
    pub user: Option<User>,
    pub error: Option<String>,
}

#[derive(serde::Deserialize, Clone, Default)]
#[serde(default)]
pub struct User {
    pub name: String,
    pub subtitle: Option<String>,
}

#[derive(serde::Deserialize, Clone, Default)]
pub struct Status {
    pub youtube: Service,
    pub spotify: Service,
}

/// Blocking: opens the login helper window (the credentials go straight from it to the daemon).
pub fn sign_in(svc: &str) -> Result<(), String> {
    let (url, token) = conn()?;
    let helper = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(format!("metrolist-login{}", std::env::consts::EXE_SUFFIX)))).unwrap_or_else(|| "metrolist-login".into());
    let st = Command::new(helper).arg(svc).env("METROLIST_URL", url).env("METROLIST_TOKEN", token).status().map_err(|e| format!("cannot start login helper: {e}"))?;
    if st.success() { Ok(()) } else { Err("sign-in cancelled".into()) }
}

/// Blocking: drops the stored credentials and the webview profile.
pub fn sign_out(svc: &str) -> Result<(), String> {
    let (url, token) = conn()?;
    ureq::delete(&format!("{url}/auth/{svc}")).set("X-Metrolist-Token", token).call().map_err(|e| e.to_string())?;
    let base = std::env::var("METROLIST_DATA").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/metrolist-desktop"));
    let _ = std::fs::remove_dir_all(base.join("webview").join(svc));
    Ok(())
}
