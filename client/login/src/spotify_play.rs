//! Spotify audio for the app. It lives here, not in the app, because librespot calls `exit()` on some failures.
//!
//! Prints `<port> <key>` and serves until stdin closes. A connection sends `<key> <track id> <start ms> <gain dB or ->`
//! and gets `OK` followed by 44.1 kHz stereo f32le until the track ends, or `ERR <reason>`. One connection is one
//! stream, so the app seeks by opening another.
//!
//! Lines on stdin are the app's playback state for Spotify Connect (see `spotify_connect.rs`).

use crate::spotify::Login;
use librespot_core::{config::SessionConfig, session::Session, SpotifyUri};
use librespot_playback::{
    audio_backend::{Sink, SinkResult},
    config::{Bitrate, PlayerConfig},
    convert::Converter,
    decoder::AudioPacket,
    mixer::NoOpVolume,
    player::{Player, PlayerEvent},
};
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc::UnboundedReceiver, Mutex};

/// Where a player's audio goes at the moment: the connection and whether `OK` has been sent on it.
#[derive(Default)]
struct Out {
    conn: Option<(TcpStream, bool)>,
    /// The app hung up.
    closed: bool,
}

struct Pipe(Arc<std::sync::Mutex<Out>>);

/// A player kept for the life of the process. Dropping one shuts down the runtime that the session's HTTP connections
/// were opened on, and the next load then fails.
struct Worker {
    gain: Option<f64>,
    session: Session,
    player: Arc<Player>,
    events: UnboundedReceiver<PlayerEvent>,
    out: Arc<std::sync::Mutex<Out>>,
}

#[derive(Default)]
struct State {
    session: Mutex<Option<Session>>,
    idle: std::sync::Mutex<Vec<Worker>>,
}

impl Sink for Pipe {
    // Never returns an error: librespot exits the process on one.
    fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
        let AudioPacket::Samples(samples) = packet else { return Ok(()) };
        // Held across the write: `closed` is only read between packets, so a blocked write is ended by shutting the socket.
        let Some((mut conn, started)) = self.0.lock().unwrap().conn.as_ref().and_then(|(c, started)| Some((c.try_clone().ok()?, *started))) else { return Ok(()) };
        let mut bytes = if started { vec![] } else { b"OK\n".to_vec() };
        bytes.extend(samples.iter().flat_map(|s| (*s as f32).to_le_bytes()));
        let sent = conn.write_all(&bytes).is_ok();
        let mut out = self.0.lock().unwrap();
        match out.conn.as_mut() {
            Some((_, started)) if sent => *started = true,
            _ => (out.conn, out.closed) = (None, true),
        }
        Ok(())
    }
}

/// The saved login, read from the daemon's credential file so it never passes through the app.
fn saved_login() -> Option<Login> {
    let dir = std::env::var_os("METROLIST_DATA")?;
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("credentials.json")).ok()?;
    let mut v: serde_json::Value = serde_json::from_str(&text).ok()?;
    serde_json::from_value(v["spotify"]["login"].take()).ok()
}

/// One session for all streams, replaced when Spotify drops it.
pub async fn session(shared: &Mutex<Option<Session>>) -> Result<Session, String> {
    let mut current = shared.lock().await;
    if let Some(s) = current.as_ref().filter(|s| !s.is_invalid()) {
        return Ok(s.clone());
    }
    let login = saved_login().ok_or("Sign in to Spotify under Settings")?;
    let s = Session::new(SessionConfig { device_id: login.device_id, ..SessionConfig::default() }, None);
    tokio::time::timeout(Duration::from_secs(20), s.connect(login.credentials, false))
        .await
        .map_err(|_| "Couldn't reach Spotify")?
        .map_err(|e| format!("Spotify: {e}"))?;
    *current = Some(s.clone());
    Ok(s)
}

async fn stream(mut conn: TcpStream, key: &str, state: &State) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    conn.set_read_timeout(Some(Duration::from_secs(5))).map_err(io)?;
    let mut line = String::new();
    BufReader::new((&conn).take(256)).read_line(&mut line).map_err(io)?;
    let [k, id, start, gain] = line.split_whitespace().collect::<Vec<_>>()[..] else { return Err("bad request".into()) };
    if k != key {
        return Err("bad key".into());
    }
    let fail = |conn: &mut TcpStream, why: String| {
        let _ = writeln!(conn, "ERR {why}");
        why
    };
    let uri = match SpotifyUri::from_uri(&format!("spotify:track:{id}")) {
        Ok(u) => u,
        Err(e) => return Err(fail(&mut conn, e.to_string())),
    };
    let session = match session(&state.session).await {
        Ok(s) => s,
        Err(e) => return Err(fail(&mut conn, e)),
    };
    let gain: Option<f64> = gain.parse().ok();
    let idle = {
        let mut idle = state.idle.lock().unwrap();
        // One made for another loudness setting, or for a session since dropped, stays parked.
        idle.iter().position(|w| w.gain == gain && !w.session.is_invalid()).map(|i| idle.swap_remove(i))
    };
    let mut w = idle.unwrap_or_else(|| {
        let config = PlayerConfig {
            bitrate: Bitrate::Bitrate320,
            normalisation: gain.is_some(),
            normalisation_pregain_db: gain.unwrap_or(0.),
            ..PlayerConfig::default()
        };
        let out = Arc::new(std::sync::Mutex::new(Out::default()));
        let pipe = Pipe(out.clone());
        let player = Player::new(config, session.clone(), Box::new(NoOpVolume), move || Box::new(pipe));
        let events = player.get_player_event_channel();
        Worker { gain, session, player, events, out }
    });
    while w.events.try_recv().is_ok() {}
    *w.out.lock().unwrap() = Out { conn: Some((conn.try_clone().map_err(io)?, false)), closed: false };
    w.player.load(uri, true, start.parse().unwrap_or(0));
    while !w.out.lock().unwrap().closed {
        match tokio::time::timeout(Duration::from_millis(200), w.events.recv()).await {
            Ok(Some(PlayerEvent::Unavailable { .. })) => {
                fail(&mut conn, "Spotify can't play this track".into());
                break;
            }
            Ok(Some(PlayerEvent::EndOfTrack { .. })) => break,
            Ok(None) => return Err("the player stopped".into()),
            _ => {}
        }
    }
    // Unblocks a sink stuck writing; the player is only free again once it has stopped.
    let _ = conn.shutdown(Shutdown::Both);
    w.player.stop();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while !matches!(w.events.recv().await, Some(PlayerEvent::Stopped { .. }) | None) {}
    })
    .await;
    *w.out.lock().unwrap() = Out::default();
    state.idle.lock().unwrap().push(w);
    Ok(())
}

pub fn serve() -> ! {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    // Other local users can reach the port; the key from stdout keeps them off the account.
    let random = || std::collections::hash_map::RandomState::new().build_hasher().finish();
    let key = format!("{:016x}{:016x}", random(), random());
    println!("{} {key}", listener.local_addr().expect("address").port());
    // The app holds stdin open for as long as it runs.
    let (states, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            if let Ok(state) = serde_json::from_str(&line) {
                let _ = states.send(state);
            }
        }
        std::process::exit(0);
    });
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
    let (state, key, handle) = (Arc::new(State::default()), Arc::new(key), runtime.handle().clone());
    let shared = state.clone();
    runtime.spawn(async move { crate::spotify_connect::run(&shared.session, rx).await });
    for conn in listener.incoming().flatten() {
        let (state, key, handle) = (state.clone(), key.clone(), handle.clone());
        // A thread each: the sink blocks.
        std::thread::spawn(move || {
            if let Err(e) = handle.block_on(stream(conn, &key, &state)) {
                eprintln!("spotify play: {e}");
            }
        });
    }
    std::process::exit(1)
}
