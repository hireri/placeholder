//! Spotify Connect: Serafin as a device Spotify apps can control. The app owns the queue; this only reports what it is
//! playing and passes commands back, so non-Spotify songs work too. (librespot's own Connect module would own the queue.)
//!
//! The app writes its state as one JSON object per line on stdin; commands go out as JSON lines on stdout.

use futures_util::StreamExt;
use librespot_core::{
    dealer::{manager::Reply, protocol::{Command, Message}},
    session::Session,
    spotify_id::SpotifyId,
    SpotifyUri,
};
use librespot_metadata::Metadata;
use librespot_protocol::{
    connect::{Capabilities, ClusterUpdate, Device, DeviceInfo, MemberType, PutStateReason, PutStateRequest, SetVolumeCommand},
    context_track::ContextTrack,
    devices::DeviceType,
    media::AudioQuality,
    player::ProvidedTrack,
};
use protobuf::{EnumOrUnknown, MessageField};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc::UnboundedReceiver, Mutex};

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn emit(event: Value) {
    println!("{event}");
}

/// How Spotify writes a local file's URI; it is how a song Spotify does not have gets a title in other apps.
fn local_uri(artist: &str, album: &str, title: &str, secs: u64) -> String {
    let enc = |s: &str| s.bytes().map(|b| match b {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
        b' ' => "+".into(),
        _ => format!("%{b:02X}"),
    }).collect::<String>();
    format!("spotify:local:{}:{}:{}:{secs}", enc(artist), enc(album), enc(title))
}

/// One of the app's queue rows (`{uid, uri?, title, artist, album, dur, art?}`) as Connect describes tracks.
fn provided(t: &Value) -> ProvidedTrack {
    let s = |k: &str| t[k].as_str().unwrap_or_default();
    let uri = match t["uri"].as_str() {
        Some(u) => u.to_string(),
        None => local_uri(s("artist"), s("album"), s("title"), t["dur"].as_u64().unwrap_or(0) / 1000),
    };
    let mut p = ProvidedTrack { uri, uid: s("uid").into(), provider: "context".into(), ..Default::default() };
    for (key, value) in [("title", s("title")), ("artist_name", s("artist")), ("album_title", s("album")), ("image_url", s("art"))] {
        if !value.is_empty() {
            p.metadata.insert(key.into(), value.into());
        }
    }
    p
}

fn device_info(session: &Session) -> DeviceInfo {
    DeviceInfo {
        can_play: true,
        volume: u32::from(u16::MAX / 2),
        name: "Serafin".into(),
        device_id: session.device_id().to_string(),
        device_type: EnumOrUnknown::new(DeviceType::COMPUTER),
        device_software_version: librespot_core::version::SEMVER.into(),
        spirc_version: librespot_core::version::SPOTIFY_SPIRC_VERSION.into(),
        client_id: session.client_id(),
        capabilities: MessageField::some(Capabilities {
            volume_steps: 64,
            gaia_eq_connect_id: true,
            can_be_player: true,
            needs_full_player_state: true,
            is_observable: true,
            is_controllable: true,
            supports_gzip_pushes: true,
            supported_types: vec!["audio/track".into(), "audio/local".into()],
            supports_playlist_v2: true,
            supports_transfer_command: true,
            supports_command_request: true,
            supports_set_options_command: true,
            supported_audio_quality: EnumOrUnknown::new(AudioQuality::VERY_HIGH),
            command_acks: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Fills in the request from the app's latest state. Only an active device describes a player.
fn describe(req: &mut PutStateRequest, session_id: String, app: &Value, since: u64) {
    if let Some(v) = app["volume"].as_u64() {
        req.device.mut_or_insert_default().device_info.mut_or_insert_default().volume = v as u32;
    }
    let device = req.device.mut_or_insert_default();
    let p = device.player_state.mut_or_insert_default();
    p.session_id = session_id;
    p.timestamp = now() as i64;
    if !req.is_active {
        return;
    }
    // The server only makes a device active if it says when that device started playing.
    req.has_been_playing_for_ms = now().saturating_sub(req.started_playing_at);
    let paused = !app["playing"].as_bool().unwrap_or(false);
    let buffering = app["buffering"].as_bool().unwrap_or(false);
    // Position was true when the app sent it; the time since has passed if it was playing.
    let age = if paused || buffering { 0 } else { now().saturating_sub(since) as i64 };
    p.position_as_of_timestamp = app["pos"].as_i64().unwrap_or(0) + age;
    p.duration = app["cur"]["dur"].as_i64().unwrap_or(0);
    // Paused is reported as playing+paused+buffering, or other apps grey out their play button.
    p.is_playing = true;
    p.is_paused = paused;
    p.is_buffering = paused || buffering;
    p.playback_speed = if paused || buffering { 0. } else { 1. };
    p.play_origin.mut_or_insert_default();
    p.suppressions.mut_or_insert_default();
    let o = p.options.mut_or_insert_default();
    o.shuffling_context = app["shuffle"].as_bool().unwrap_or(false);
    o.repeating_context = app["repeat"] == 1;
    o.repeating_track = app["repeat"] == 2;
    p.context_uri = app["context"].as_str().unwrap_or("spotify:unknown").into();
    p.context_url = format!("context://{}", p.context_uri);
    p.track = app["cur"].is_object().then(|| provided(&app["cur"])).into();
    let rows = |k: &str| app[k].as_array().into_iter().flatten().map(provided).collect();
    p.next_tracks = rows("next");
    p.prev_tracks = rows("prev");
}

fn track_uri(t: &ContextTrack) -> Option<String> {
    t.uri.clone().filter(|u| !u.is_empty()).or_else(|| Some(format!("spotify:track:{}", SpotifyId::from_raw(t.gid()).ok()?.to_base62().ok()?)))
}

/// Title, artists and the rest of a song Spotify named only by its URI.
async fn lookup(session: &Session, uri: &str) -> Option<Value> {
    let id = SpotifyUri::from_uri(uri).ok().filter(|u| matches!(u, SpotifyUri::Track { .. }))?;
    let get = librespot_metadata::Track::get(session, &id);
    let t = tokio::time::timeout(Duration::from_secs(5), get).await.ok()?.ok()?;
    let art = t.album.covers.first().and_then(|c| c.id.to_base16().ok()).map(|h| format!("https://i.scdn.co/image/{h}"));
    Some(json!({
        "uri": uri, "title": t.name, "artists": t.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "album": t.album.name, "dur": t.duration, "art": art,
    }))
}

/// What a command asks of the app. `None` for the ones Serafin has nothing to do for.
async fn translate(session: &Session, command: Command) -> Option<Value> {
    Some(match command {
        Command::Transfer(t) => {
            let d = t.data?;
            let from_queue = d.queue.is_playing_queue();
            let current = if from_queue { d.queue.tracks.first() } else { d.playback.current_track.as_ref() };
            let uri = current.and_then(track_uri)?;
            let paused = d.playback.is_paused();
            let age = if paused { 0 } else { (now() as i64 - d.playback.timestamp()).clamp(0, 600_000) };
            let mut queued = vec![];
            // ponytail: the first ten songs queued by hand; each needs its own metadata request.
            for t in d.queue.tracks.iter().skip(usize::from(from_queue)).take(10) {
                if let Some(v) = match track_uri(t) { Some(u) => lookup(session, &u).await, None => None } {
                    queued.push(v);
                }
            }
            json!({
                "cmd": "load", "context": d.current_session.context.uri(), "track": lookup(session, &uri).await,
                "pos": i64::from(d.playback.position_as_of_timestamp()) + age, "paused": paused,
                "shuffle": d.options.shuffling_context(), "queue": queued,
            })
        }
        Command::Play(p) => {
            let listed: Vec<&ContextTrack> = p.context.pages.iter().flat_map(|g| &g.tracks).collect();
            let skip = p.options.skip_to.as_ref();
            let uri = skip.and_then(|s| s.track_uri.clone()).or_else(|| {
                let index = skip.and_then(|s| s.track_index).unwrap_or(0) as usize;
                listed.get(index).copied().and_then(track_uri)
            });
            let track = match &uri { Some(u) => lookup(session, u).await, None => None };
            json!({
                "cmd": "load", "context": p.context.uri(), "track": track, "index": skip.and_then(|s| s.track_index),
                "pos": p.options.seek_to.unwrap_or(0), "paused": p.options.initially_paused.unwrap_or(false),
                "shuffle": p.options.player_options_override.as_ref().and_then(|o| o.shuffling_context),
            })
        }
        Command::Pause(_) => json!({ "cmd": "pause" }),
        Command::Resume(_) => json!({ "cmd": "resume" }),
        Command::SeekTo(s) => json!({ "cmd": "seek", "pos": s.value }),
        Command::SkipNext(s) => json!({ "cmd": "next", "uid": s.track.map(|t| t.uid) }),
        Command::SkipPrev(_) => json!({ "cmd": "prev" }),
        Command::SetShufflingContext(v) => json!({ "cmd": "options", "shuffle": v.value }),
        Command::SetRepeatingContext(v) => json!({ "cmd": "options", "repeat_context": v.value }),
        Command::SetRepeatingTrack(v) => json!({ "cmd": "options", "repeat_track": v.value }),
        Command::SetOptions(o) => json!({ "cmd": "options", "shuffle": o.shuffling_context, "repeat_context": o.repeating_context, "repeat_track": o.repeating_track }),
        Command::AddToQueue(a) => json!({ "cmd": "queue_add", "track": lookup(session, &a.track.uri).await? }),
        Command::SetQueue(q) => json!({ "cmd": "queue_set", "uids": q.next_tracks.iter().map(|t| t.uid.clone()).collect::<Vec<_>>() }),
        other => {
            eprintln!("spotify connect: ignored {other}");
            return None;
        }
    })
}

/// The app's newest state and when it arrived.
struct App {
    state: Value,
    at: u64,
}

impl App {
    fn on(&self) -> bool {
        self.state["on"].as_bool().unwrap_or(false)
    }
}

/// Runs the device on `session` until the session drops.
async fn serve(session: Session, rx: &mut UnboundedReceiver<Value>, app: &mut App) -> Result<(), librespot_core::Error> {
    let mut connection = session.dealer().listen_for("hm://pusher/v1/connections/", |m: Message| {
        m.headers.get("Spotify-Connection-Id").cloned().ok_or_else(|| librespot_core::Error::unavailable("no connection id"))
    })?;
    let mut cluster = session.dealer().listen_for("hm://connect-state/v1/cluster", Message::from_raw::<ClusterUpdate>)?;
    let mut volume = session.dealer().listen_for("hm://connect-state/v1/connect/volume", Message::from_raw::<SetVolumeCommand>)?;
    let mut commands = session.dealer().handle_for("hm://connect-state/v1/player/command")?;
    session.spclient().client_token().await?;
    session.login5().auth_token().await?;
    session.dealer().start().await?;

    let mut req = PutStateRequest {
        member_type: EnumOrUnknown::new(MemberType::CONNECT_STATE),
        device: MessageField::some(Device { device_info: MessageField::some(device_info(&session)), ..Default::default() }),
        ..Default::default()
    };
    // `claimed`: Spotify has been told we are the active device, so its saying otherwise now means we lost that.
    let (mut listed, mut connected, mut claimed) = (false, false, false);
    let mut dirty: Option<PutStateReason> = None;
    let mut alive = tokio::time::interval(Duration::from_secs(5));
    loop {
        // Changes arrive in bursts (a dragged volume slider); one request covers those that land together.
        let settle = tokio::time::sleep(Duration::from_millis(200));
        tokio::select! {
            id = connection.next() => {
                let Some(id) = id else { break };
                session.set_connection_id(&id?);
                (connected, listed) = (true, false);
                dirty = Some(PutStateReason::NEW_DEVICE);
            }
            state = rx.recv() => {
                let Some(state) = state else { break };
                *app = App { state, at: now() };
                dirty.get_or_insert(PutStateReason::PLAYER_STATE_CHANGED);
            }
            update = cluster.next() => {
                let Some(update) = update else { break };
                let Ok(update) = update else { continue };
                let other = &update.cluster.active_device_id;
                if claimed && req.is_active && !other.is_empty() && other != session.device_id() {
                    (req.is_active, req.started_playing_at, claimed) = (false, 0, false);
                    emit(json!({ "cmd": "pause" }));
                    if let Err(e) = session.spclient().put_connect_state_inactive(false).await {
                        eprintln!("spotify connect: {e}");
                    }
                }
            }
            v = volume.next() => {
                let Some(v) = v else { break };
                if let Ok(v) = v {
                    emit(json!({ "cmd": "volume", "v": v.volume }));
                }
            }
            command = commands.next() => {
                let Some((request, reply)) = command else { break };
                if !app.on() {
                    let _ = reply.send(Reply::Failure);
                    continue;
                }
                req.last_command_message_id = request.message_id;
                req.last_command_sent_by_device_id = request.sent_by_device_id;
                // Picking this device, or something to play on it, is what makes it the active one.
                if matches!(request.command, Command::Transfer(_) | Command::Play(_)) && !req.is_active {
                    (req.is_active, req.started_playing_at) = (true, now());
                }
                let _ = reply.send(Reply::Success);
                if let Some(event) = translate(&session, request.command).await {
                    emit(event);
                }
                dirty = Some(PutStateReason::PLAYER_STATE_CHANGED);
            }
            _ = settle, if dirty.is_some() && connected => {
                let reason = dirty.take().unwrap_or_default();
                if app.on() {
                    describe(&mut req, session.session_id(), &app.state, app.at);
                    req.put_state_reason = EnumOrUnknown::new(if listed { reason } else { PutStateReason::NEW_DEVICE });
                    req.client_side_timestamp = now();
                    match session.spclient().put_connect_state_request(&req).await {
                        Ok(_) => {
                            if !listed {
                                eprintln!("spotify connect: listed as a device");
                            }
                            (listed, claimed) = (true, req.is_active);
                        }
                        Err(e) => eprintln!("spotify connect: {e}"),
                    }
                } else if listed {
                    (listed, claimed, req.is_active, req.started_playing_at) = (false, false, false, 0);
                    if let Err(e) = session.spclient().delete_connect_state_request().await {
                        eprintln!("spotify connect: {e}");
                    }
                }
            }
            _ = alive.tick() => if session.is_invalid() { break },
        }
    }
    session.shutdown();
    Ok(())
}

/// Waits for the app to turn Connect on, then keeps the device up, on a new session whenever Spotify drops one.
pub async fn run(shared: &Mutex<Option<Session>>, mut rx: UnboundedReceiver<Value>) {
    let mut app = App { state: Value::Null, at: 0 };
    loop {
        while !app.on() {
            let Some(state) = rx.recv().await else { return };
            app = App { state, at: now() };
        }
        let result = match crate::spotify_play::session(shared).await {
            Ok(session) => serve(session.clone(), &mut rx, &mut app).await.map_err(|e| {
                // Its dealer cannot be started a second time.
                session.shutdown();
                e.to_string()
            }),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            eprintln!("spotify connect: {e}");
            emit(json!({ "cmd": "error", "message": e }));
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
        while let Ok(state) = rx.try_recv() {
            app = App { state, at: now() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn songs_spotify_lacks_are_described_as_local_files() {
        let t = provided(&json!({ "uid": "v1", "title": "A: b", "artist": "C D", "album": "", "dur": 200_500 }));
        assert_eq!(t.uri, "spotify:local:C+D::A%3A+b:200");
        assert_eq!(t.metadata.get("title").map(String::as_str), Some("A: b"));
        assert!(!t.metadata.contains_key("album_title"));
    }

    #[test]
    fn only_the_active_device_describes_a_player_and_pause_keeps_its_position() {
        let app = json!({ "playing": false, "pos": 4000, "volume": 100, "cur": { "uid": "a", "uri": "spotify:track:x", "title": "t", "dur": 9000 }, "next": [], "prev": [] });
        let mut req = PutStateRequest::default();
        describe(&mut req, "s".into(), &app, 0);
        assert!(req.device.player_state.track.is_none());
        assert_eq!(req.device.device_info.volume, 100);
        (req.is_active, req.started_playing_at) = (true, now());
        describe(&mut req, "s".into(), &app, 0);
        let p = &req.device.player_state;
        assert_eq!((p.position_as_of_timestamp, p.is_paused, p.is_playing), (4000, true, true));
        assert_eq!(p.track.uri, "spotify:track:x");
    }
}
