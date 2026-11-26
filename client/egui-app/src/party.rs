//! Listening parties: Metrolist's Listen Together rooms, on its server and protocol, so a room can mix Serafin and
//! Metrolist listeners. A worker thread owns the WebSocket (protobuf envelopes, gzip when flagged) and the server clock;
//! the UI thread sends messages and applies what arrives once per frame. The host's player goes out as playback actions,
//! a guest's player follows them. Positions and times on the wire are milliseconds.

use crate::app::App;
use crate::data::Track;
use crate::gfx::*;
use eframe::egui::{self, pos2, vec2, Align2, Context, Id, Key, Modifiers, Rect};
use prost::Message;
use std::io::Read;
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;

const SERVER: &str = "wss://metroserverx.meowery.eu/ws";
/// Same share link Metrolist uses, so a code copied here opens in its app too.
pub const LINK: &str = "https://metrolist.cc/listen?code=";
const PING_EVERY: Duration = Duration::from_secs(25);
/// The host repeats PLAY this often while playing, so guests that drifted or paused locally fall back in line.
const HEARTBEAT: Duration = Duration::from_secs(8);
/// A playing guest only seeks when this far off; smaller drift is left alone (upstream also nudges playback speed).
const HARD_SYNC_MS: i64 = 2000;

/// Wire types (listentogether.proto in Metrolist); only the fields we read or write.
mod pb {
    use prost::Message;

    #[derive(Clone, PartialEq, Message)]
    pub struct Envelope {
        #[prost(string, tag = "1")]
        pub r#type: String,
        #[prost(bytes = "vec", tag = "2")]
        pub payload: Vec<u8>,
        #[prost(bool, tag = "3")]
        pub compressed: bool,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct TrackInfo {
        #[prost(string, tag = "1")]
        pub id: String,
        #[prost(string, tag = "2")]
        pub title: String,
        #[prost(string, tag = "3")]
        pub artist: String,
        #[prost(string, tag = "4")]
        pub album: String,
        #[prost(int64, tag = "5")]
        pub duration: i64,
        #[prost(string, tag = "6")]
        pub thumbnail: String,
        #[prost(string, tag = "7")]
        pub suggested_by: String,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct UserInfo {
        #[prost(string, tag = "1")]
        pub user_id: String,
        #[prost(string, tag = "2")]
        pub username: String,
        #[prost(bool, tag = "3")]
        pub is_host: bool,
        #[prost(bool, tag = "4")]
        pub is_connected: bool,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct RoomState {
        #[prost(string, tag = "1")]
        pub room_code: String,
        #[prost(string, tag = "2")]
        pub host_id: String,
        #[prost(message, repeated, tag = "3")]
        pub users: Vec<UserInfo>,
        #[prost(message, optional, tag = "4")]
        pub current_track: Option<TrackInfo>,
        #[prost(bool, tag = "5")]
        pub is_playing: bool,
        #[prost(int64, tag = "6")]
        pub position: i64,
        #[prost(int64, tag = "7")]
        pub last_update: i64,
        #[prost(message, repeated, tag = "9")]
        pub queue: Vec<TrackInfo>,
    }

    /// create_room (username), and the shape of join_room.
    #[derive(Clone, PartialEq, Message)]
    pub struct Join {
        #[prost(string, tag = "1")]
        pub room_code: String,
        #[prost(string, tag = "2")]
        pub username: String,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Name {
        #[prost(string, tag = "1")]
        pub username: String,
    }

    /// Any payload that is one id plus an optional reason: approve/reject join, kick, buffer_ready/complete,
    /// approve/reject suggestion, reconnect, join_rejected/kicked (reason in field 1 there).
    #[derive(Clone, PartialEq, Message)]
    pub struct Id {
        #[prost(string, tag = "1")]
        pub id: String,
        #[prost(string, tag = "2")]
        pub reason: String,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct PlaybackAction {
        #[prost(string, tag = "1")]
        pub action: String,
        #[prost(string, tag = "2")]
        pub track_id: String,
        #[prost(int64, tag = "3")]
        pub position: i64,
        #[prost(message, optional, tag = "4")]
        pub track_info: Option<TrackInfo>,
        #[prost(bool, tag = "5")]
        pub insert_next: bool,
        #[prost(message, repeated, tag = "6")]
        pub queue: Vec<TrackInfo>,
        #[prost(string, tag = "7")]
        pub queue_title: String,
        #[prost(float, tag = "8")]
        pub volume: f32,
        #[prost(int64, tag = "9")]
        pub server_time: i64,
        #[prost(uint64, tag = "10")]
        pub revision: u64,
        #[prost(int64, tag = "11")]
        pub captured_at_server_time: i64,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Ping {
        #[prost(int64, tag = "1")]
        pub client_time: i64,
        #[prost(uint64, tag = "2")]
        pub sequence: u64,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Pong {
        #[prost(int64, tag = "1")]
        pub client_time: i64,
        #[prost(int64, tag = "2")]
        pub server_receive_time: i64,
        #[prost(int64, tag = "3")]
        pub server_send_time: i64,
    }

    /// room_created, join_approved, reconnected (the last puts `state` at 3 and `is_host` at 4).
    #[derive(Clone, PartialEq, Message)]
    pub struct Session {
        #[prost(string, tag = "1")]
        pub room_code: String,
        #[prost(string, tag = "2")]
        pub user_id: String,
        #[prost(string, tag = "3")]
        pub session_token: String,
        #[prost(message, optional, tag = "4")]
        pub state: Option<RoomState>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Reconnected {
        #[prost(string, tag = "1")]
        pub room_code: String,
        #[prost(string, tag = "2")]
        pub user_id: String,
        #[prost(message, optional, tag = "3")]
        pub state: Option<RoomState>,
        #[prost(bool, tag = "4")]
        pub is_host: bool,
    }

    /// join_request, user_joined/left/reconnected/disconnected, host_changed: (user id, name).
    #[derive(Clone, PartialEq, Message)]
    pub struct User {
        #[prost(string, tag = "1")]
        pub user_id: String,
        #[prost(string, tag = "2")]
        pub username: String,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct BufferWait {
        #[prost(string, tag = "1")]
        pub track_id: String,
        #[prost(string, repeated, tag = "2")]
        pub waiting_for: Vec<String>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct SyncState {
        #[prost(message, optional, tag = "1")]
        pub current_track: Option<TrackInfo>,
        #[prost(bool, tag = "2")]
        pub is_playing: bool,
        #[prost(int64, tag = "3")]
        pub position: i64,
        #[prost(int64, tag = "4")]
        pub last_update: i64,
        #[prost(message, repeated, tag = "5")]
        pub queue: Vec<TrackInfo>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Suggestion {
        #[prost(string, tag = "1")]
        pub suggestion_id: String,
        #[prost(string, tag = "2")]
        pub from_user_id: String,
        #[prost(string, tag = "3")]
        pub from_username: String,
        #[prost(message, optional, tag = "4")]
        pub track_info: Option<TrackInfo>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct SuggestTrack {
        #[prost(message, optional, tag = "1")]
        pub track_info: Option<TrackInfo>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Error {
        #[prost(string, tag = "1")]
        pub code: String,
        #[prost(string, tag = "2")]
        pub message: String,
    }

    pub fn track(t: &crate::data::Track, vid: &str) -> TrackInfo {
        TrackInfo {
            id: vid.into(),
            title: t.title.clone(),
            artist: t.artists.join(", "),
            album: t.album.clone().unwrap_or_default(),
            // Upstream sends 3 minutes when the length is unknown.
            duration: t.duration_sec.filter(|d| *d > 0).map_or(180_000, |d| d as i64 * 1000),
            thumbnail: t.thumbnail.clone().unwrap_or_default(),
            suggested_by: String::new(),
        }
    }

    pub fn to_track(t: &TrackInfo) -> crate::data::Track {
        crate::data::Track {
            id: t.id.clone(),
            title: t.title.clone(),
            artists: t.artist.split(", ").filter(|a| !a.is_empty()).map(String::from).collect(),
            album: Some(t.album.clone()).filter(|a| !a.is_empty()),
            duration_sec: Some((t.duration / 1000) as u32).filter(|d| *d > 0),
            thumbnail: Some(t.thumbnail.clone()).filter(|u| !u.is_empty()),
            source: "youtube".into(),
            ..Default::default()
        }
    }
}

/// Milliseconds on a monotonic clock; pings carry it and pongs echo it back.
fn mono_ms() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as i64 + 1
}

/// Offset from our monotonic clock to the server's, smoothed over pongs (upstream `ServerClock`).
#[derive(Default)]
struct Clock {
    offset: Option<f64>,
    best_rtt: i64,
}

impl Clock {
    fn pong(&mut self, p: &pb::Pong) {
        let got = mono_ms();
        if p.client_time <= 0 || p.client_time > got || got - p.client_time > 60_000 || p.server_receive_time <= 0 || p.server_send_time < p.server_receive_time {
            return;
        }
        let rtt = (got - p.client_time - (p.server_send_time - p.server_receive_time)).max(0);
        let sample = p.server_send_time as f64 + rtt as f64 / 2. - got as f64;
        if self.offset.is_none() || rtt < self.best_rtt {
            self.best_rtt = rtt;
        }
        // Samples near the best round trip are trusted more; a slow one barely moves the estimate.
        let w = if rtt <= self.best_rtt + 50 { 0.25 } else { 0.05 };
        self.offset = Some(self.offset.map_or(sample, |o| o + w * (sample - o)));
    }

    fn now(&self) -> Option<i64> {
        self.offset.map(|o| (mono_ms() as f64 + o) as i64)
    }

    /// Where a room position taken at server time `at` has got to by now.
    fn position_at(&self, pos: i64, at: i64, playing: bool) -> i64 {
        match (playing && at > 0, self.now()) {
            (true, Some(now)) => pos + (now - at).max(0),
            _ => pos,
        }
    }
}

fn encode(kind: &str, payload: Option<&dyn MessageBytes>) -> Vec<u8> {
    pb::Envelope { r#type: kind.into(), payload: payload.map(|p| p.bytes()).unwrap_or_default(), compressed: false }.encode_to_vec()
}

/// Object-safe `encode_to_vec`, so `send` can take any payload.
trait MessageBytes {
    fn bytes(&self) -> Vec<u8>;
}

impl<M: Message> MessageBytes for M {
    fn bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

enum In {
    Msg(String, Vec<u8>),
    /// The socket dropped; the worker is retrying (with the session token once there is one).
    Lost,
    /// The worker gave up.
    Failed(String),
}

/// The socket thread: connects, resumes a session after a drop, pings, and forwards every message.
fn worker(ctx: Context, out: Receiver<Vec<u8>>, inbox: Sender<In>, clock: Arc<Mutex<Clock>>, token: Arc<Mutex<String>>) {
    let mut attempt = 0u32;
    loop {
        let (mut ws, _) = match tungstenite::connect(SERVER) {
            Ok(c) => c,
            Err(e) => {
                attempt += 1;
                if attempt > 8 {
                    let _ = inbox.send(In::Failed(format!("can't reach the party server: {e}")));
                    ctx.request_repaint();
                    return;
                }
                let _ = inbox.send(In::Lost);
                ctx.request_repaint();
                std::thread::sleep(Duration::from_secs(1 << attempt.min(5)));
                continue;
            }
        };
        attempt = 0;
        let sock: Option<&TcpStream> = match ws.get_ref() {
            MaybeTlsStream::Plain(s) => Some(s),
            MaybeTlsStream::Rustls(s) => Some(s.get_ref()),
            _ => None,
        };
        if let Some(s) = sock {
            let _ = s.set_read_timeout(Some(Duration::from_millis(50)));
        }
        let resume = token.lock().unwrap().clone();
        if !resume.is_empty() {
            let _ = ws.send(tungstenite::Message::Binary(encode("reconnect", Some(&pb::Id { id: resume, reason: String::new() })).into()));
        }
        // Three quick pings settle the clock before the first sync lands.
        let (mut seq, mut next_ping) = (0u64, Instant::now());
        let err = 'conn: loop {
            loop {
                match out.try_recv() {
                    Ok(frame) => {
                        if let Err(e) = ws.send(tungstenite::Message::Binary(frame.into())) {
                            break 'conn e.to_string();
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        let _ = ws.close(None);
                        let _ = ws.flush();
                        return;
                    }
                }
            }
            if Instant::now() >= next_ping {
                seq += 1;
                let _ = ws.send(tungstenite::Message::Binary(encode("ping", Some(&pb::Ping { client_time: mono_ms(), sequence: seq })).into()));
                next_ping = Instant::now() + if seq < 3 { Duration::from_millis(250) } else { PING_EVERY };
            }
            match ws.read() {
                Ok(tungstenite::Message::Binary(b)) => {
                    let Ok(env) = pb::Envelope::decode(&b[..]) else { continue };
                    let payload = if env.compressed { gunzip(&env.payload).unwrap_or(env.payload) } else { env.payload };
                    if env.r#type == "pong" {
                        if let Ok(p) = pb::Pong::decode(&payload[..]) {
                            clock.lock().unwrap().pong(&p);
                        }
                        continue;
                    }
                    let _ = inbox.send(In::Msg(env.r#type, payload));
                    ctx.request_repaint();
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => break e.to_string(),
            }
        };
        eprintln!("party: connection lost: {err}");
        let _ = inbox.send(In::Lost);
        ctx.request_repaint();
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn gunzip(b: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![];
    flate2::read::GzDecoder::new(b).read_to_end(&mut out).ok()?;
    Some(out)
}

pub struct Member {
    pub id: String,
    pub name: String,
    pub host: bool,
    pub connected: bool,
}

pub struct Suggestion {
    pub id: String,
    pub from: String,
    pub track: pb::TrackInfo,
}

pub struct Room {
    pub code: String,
    pub me: String,
    pub host_id: String,
    pub members: Vec<Member>,
    /// Join requests waiting on the host: (user id, name).
    pub requests: Vec<(String, String)>,
    pub suggestions: Vec<Suggestion>,
    /// Listeners still loading the current song.
    pub waiting: Vec<String>,
}

impl Room {
    pub fn is_host(&self) -> bool {
        !self.me.is_empty() && self.me == self.host_id
    }

    fn from_state(code: String, me: String, s: Option<&pb::RoomState>) -> Room {
        let (host_id, members) = s.map_or_else(Default::default, |s| {
            (s.host_id.clone(), s.users.iter().map(|u| Member { id: u.user_id.clone(), name: u.username.clone(), host: u.is_host, connected: u.is_connected }).collect())
        });
        Room { code, me, host_id, members, requests: vec![], suggestions: vec![], waiting: vec![] }
    }
}

/// Where the room says playback should be, for a guest to reach once its stream loads.
struct Target {
    id: String,
    playing: bool,
    pos: i64,
    at: i64,
}

#[derive(Default)]
pub struct Party {
    out: Option<Sender<Vec<u8>>>,
    inbox: Option<Receiver<In>>,
    clock: Arc<Mutex<Clock>>,
    token: Arc<Mutex<String>>,
    pub room: Option<Room>,
    /// "Connecting…", "Waiting for the host to let you in", a lost connection or a refusal; shown in the popover.
    pub status: Option<String>,
    /// Host setting: suggestions go straight into the queue.
    pub auto_add: bool,
    /// Host: what was last announced, to notice the player changing.
    sent: Option<(String, bool, u32)>,
    beat: Option<Instant>,
    /// Guest: the room's playback, applied once the song is loaded.
    target: Option<Target>,
    ready_sent: String,
    /// Guest: the song whose full details (cover, album, artist links) were looked up.
    filled: String,
}

impl Party {
    pub fn active(&self) -> bool {
        self.out.is_some()
    }

    pub fn is_guest(&self) -> bool {
        self.room.as_ref().is_some_and(|r| !r.is_host())
    }

    fn start(&mut self, ctx: &Context, first: Vec<u8>) {
        self.leave();
        let (out_tx, out_rx) = channel();
        let (in_tx, in_rx) = channel();
        let _ = out_tx.send(first);
        (self.clock, self.token) = Default::default();
        let (ctx, clock, token) = (ctx.clone(), self.clock.clone(), self.token.clone());
        std::thread::spawn(move || worker(ctx, out_rx, in_tx, clock, token));
        (self.out, self.inbox, self.status) = (Some(out_tx), Some(in_rx), Some("Connecting…".into()));
    }

    pub fn create(&mut self, ctx: &Context, name: &str) {
        self.start(ctx, encode("create_room", Some(&pb::Name { username: name.into() })));
    }

    /// Takes a bare code or a pasted share link.
    pub fn join(&mut self, ctx: &Context, code: &str, name: &str) {
        let code = code.trim().rsplit("code=").next().unwrap_or("").trim().to_uppercase();
        if !code.is_empty() {
            self.start(ctx, encode("join_room", Some(&pb::Join { room_code: code, username: name.into() })));
        }
    }

    pub fn leave(&mut self) {
        if self.room.is_some() {
            self.send("leave_room", None);
        }
        // Dropping the sender ends the worker once it has flushed what is queued.
        *self = Party { auto_add: self.auto_add, ..Default::default() };
    }

    fn send(&self, kind: &str, payload: Option<&dyn MessageBytes>) {
        if let Some(out) = &self.out {
            let _ = out.send(encode(kind, payload));
        }
    }

    fn id(&self, kind: &str, id: &str) {
        self.send(kind, Some(&pb::Id { id: id.into(), reason: String::new() }));
    }

    pub fn answer_join(&mut self, user: &str, yes: bool) {
        self.id(if yes { "approve_join" } else { "reject_join" }, user);
        if let Some(r) = &mut self.room {
            r.requests.retain(|q| q.0 != user);
        }
    }

    pub fn answer_suggestion(&mut self, id: &str, yes: bool) {
        self.id(if yes { "approve_suggestion" } else { "reject_suggestion" }, id);
        if let Some(r) = &mut self.room {
            r.suggestions.retain(|s| s.id != id);
        }
    }

    pub fn kick(&self, user: &str) {
        self.id("kick_user", user);
    }

    pub fn suggest(&self, t: &Track) {
        if t.source == "youtube" && !t.id.is_empty() {
            self.send("suggest_track", Some(&pb::SuggestTrack { track_info: Some(pb::track(t, &t.id)) }));
            crate::toast::info("Suggested to the party");
        } else {
            crate::toast::info("Only YouTube Music songs can be suggested");
        }
    }

    fn action(&self, action: &str, vid: &str, pos_s: f32, info: Option<pb::TrackInfo>, queue: Vec<pb::TrackInfo>) {
        let pos = (pos_s.max(0.) * 1000.) as i64;
        let captured = if matches!(action, "play" | "pause" | "seek") { self.clock.lock().unwrap().now().unwrap_or(0) } else { 0 };
        self.send("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: vid.into(), position: pos, track_info: info, queue, volume: 1., captured_at_server_time: captured, ..Default::default() }));
    }
}

impl App {
    /// Drains the room's messages and keeps the player and the room in step; called every frame.
    pub fn party_tick(&mut self, ctx: &Context) {
        let Some(inbox) = &self.party.inbox else { return };
        let mut msgs = vec![];
        loop {
            match inbox.try_recv() {
                Ok(m) => msgs.push(m),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    msgs.push(In::Failed("Disconnected from the party".into()));
                    break;
                }
            }
        }
        for m in msgs {
            match m {
                In::Msg(kind, payload) => self.party_msg(ctx, &kind, &payload),
                In::Lost if self.party.room.is_some() => self.party.status = Some("Reconnecting…".into()),
                In::Lost => {}
                In::Failed(e) => {
                    crate::toast::error(e.clone());
                    self.party.leave();
                    self.party.status = Some(e);
                }
            }
        }
        let Some(room) = &self.party.room else { return };
        if room.is_host() {
            self.party_host();
        } else {
            self.party_guest(ctx);
        }
    }

    fn party_msg(&mut self, ctx: &Context, kind: &str, b: &[u8]) {
        let user = || pb::User::decode(b).unwrap_or_default();
        match kind {
            "room_created" => {
                let s = pb::Session::decode(b).unwrap_or_default();
                *self.party.token.lock().unwrap() = s.session_token;
                let name = self.user_name().unwrap_or_else(|| "You".into());
                let mut room = Room::from_state(s.room_code, s.user_id.clone(), None);
                room.host_id = s.user_id.clone();
                room.members.push(Member { id: s.user_id, name, host: true, connected: true });
                (self.party.room, self.party.status, self.party.sent) = (Some(room), None, None);
            }
            "join_approved" => {
                let s = pb::Session::decode(b).unwrap_or_default();
                *self.party.token.lock().unwrap() = s.session_token;
                self.party.room = Some(Room::from_state(s.room_code, s.user_id, s.state.as_ref()));
                self.party.status = None;
                if let Some(st) = s.state {
                    self.party_follow(ctx, st.current_track, st.queue, st.is_playing, st.position, st.last_update);
                }
            }
            "reconnected" => {
                let s = pb::Reconnected::decode(b).unwrap_or_default();
                let me = s.user_id.clone();
                let keep = self.party.room.take();
                let mut room = Room::from_state(s.room_code, me, s.state.as_ref());
                if let Some(old) = keep {
                    (room.requests, room.suggestions) = (old.requests, old.suggestions);
                }
                if s.is_host {
                    room.host_id = room.me.clone();
                }
                (self.party.room, self.party.status, self.party.sent) = (Some(room), None, None);
                if let Some(st) = s.state.filter(|_| !s.is_host) {
                    self.party_follow(ctx, st.current_track, st.queue, st.is_playing, st.position, st.last_update);
                }
            }
            "join_request" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                crate::toast::info(format!("{} wants to join the party", u.username));
                room.requests.push((u.user_id, u.username));
            }
            "join_rejected" | "kicked" => {
                let reason = pb::Id::decode(b).unwrap_or_default().id;
                let what = if kind == "kicked" { "You were removed from the party" } else { "The host turned down your request" };
                let msg = if reason.is_empty() { what.to_string() } else { format!("{what}: {reason}") };
                crate::toast::info(msg.clone());
                self.party.leave();
                self.party.status = Some(msg);
            }
            "user_joined" | "user_reconnected" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                room.requests.retain(|q| q.0 != u.user_id);
                match room.members.iter_mut().find(|m| m.id == u.user_id) {
                    Some(m) => m.connected = true,
                    None => room.members.push(Member { id: u.user_id, name: u.username, host: false, connected: true }),
                }
            }
            "user_left" | "user_disconnected" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                if kind == "user_left" {
                    room.members.retain(|m| m.id != u.user_id);
                } else if let Some(m) = room.members.iter_mut().find(|m| m.id == u.user_id) {
                    m.connected = false;
                }
            }
            "host_changed" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                room.host_id = u.user_id.clone();
                for m in &mut room.members {
                    m.host = m.id == u.user_id;
                }
                (self.party.sent, self.party.target) = (None, None);
                if room.is_host() {
                    crate::toast::info("You are now the party host");
                }
            }
            "buffer_wait" => {
                if let Some(room) = &mut self.party.room {
                    room.waiting = pb::BufferWait::decode(b).unwrap_or_default().waiting_for;
                }
            }
            "buffer_complete" => {
                if let Some(room) = &mut self.party.room {
                    room.waiting.clear();
                }
            }
            "suggestion_received" => {
                let s = pb::Suggestion::decode(b).unwrap_or_default();
                let Some(track) = s.track_info else { return };
                if self.party.auto_add {
                    return self.party.answer_suggestion(&s.suggestion_id, true);
                }
                let Some(room) = &mut self.party.room else { return };
                crate::toast::info(format!("{} suggested {}", s.from_username, track.title));
                room.suggestions.push(Suggestion { id: s.suggestion_id, from: s.from_username, track });
            }
            "suggestion_rejected" => crate::toast::info("The host passed on your suggestion"),
            "sync_state" => {
                let s = pb::SyncState::decode(b).unwrap_or_default();
                if self.party.is_guest() {
                    self.party_follow(ctx, s.current_track, s.queue, s.is_playing, s.position, s.last_update);
                }
            }
            "sync_playback" => {
                let a = pb::PlaybackAction::decode(b).unwrap_or_default();
                self.party_action(ctx, a);
            }
            "error" => {
                let e = pb::Error::decode(b).unwrap_or_default();
                if e.code == "session_not_found" || (e.code == "not_in_room" && self.party.room.is_some()) {
                    *self.party.token.lock().unwrap() = String::new();
                    self.party.leave();
                    self.party.status = Some(if e.code == "not_in_room" { "You're no longer in the party" } else { "The party has ended" }.into());
                } else {
                    let msg = if e.message.is_empty() { e.code } else { e.message };
                    if self.party.room.is_none() {
                        self.party.leave();
                        self.party.status = Some(msg.clone());
                    }
                    crate::toast::error(msg);
                }
            }
            _ => {}
        }
    }

    /// A playback action from the host. The host only takes queue additions (approved suggestions) from these.
    fn party_action(&mut self, ctx: &Context, a: pb::PlaybackAction) {
        let host = self.party.room.as_ref().is_some_and(Room::is_host);
        if a.action == "queue_add" {
            if let (true, Some(t)) = (host, a.track_info.as_ref()) {
                self.pl.enqueue(ctx, &[pb::to_track(t)], a.insert_next);
            }
            return;
        }
        if host {
            return;
        }
        let at = if a.server_time > 0 { a.server_time } else { a.captured_at_server_time };
        match a.action.as_str() {
            // A host re-announces its song after a reconnect or host change; a guest already on it keeps playing.
            "change_track" if a.track_info.as_ref().map(|t| t.id.as_str()) == self.pl.song().map(|s| s.vid()) => {}
            "change_track" => self.party_follow(ctx, a.track_info, a.queue, false, 0, 0),
            "play" | "pause" | "seek" => {
                let cur = self.pl.song().map(|s| s.vid().to_string()).unwrap_or_default();
                let playing = match a.action.as_str() {
                    "play" => true,
                    "pause" => false,
                    _ => self.party.target.as_ref().map_or(self.pl.playing, |t| t.playing),
                };
                let id = if a.track_id.is_empty() { cur.clone() } else { a.track_id.clone() };
                if id != cur {
                    // A PLAY for a song we don't have (we joined mid-change, or missed it): ask for the full state.
                    self.party.send("request_sync", None);
                    return;
                }
                self.party.target = Some(Target { id, playing, pos: a.position, at });
            }
            _ => {}
        }
    }

    /// Loads the room's song (paused, at its position) unless it is already the current one, then aims for the room's playback.
    fn party_follow(&mut self, ctx: &Context, track: Option<pb::TrackInfo>, queue: Vec<pb::TrackInfo>, playing: bool, pos: i64, at: i64) {
        let Some(t) = track.filter(|t| !t.id.is_empty()) else { return };
        if self.pl.song().map(|s| s.vid()) != Some(t.id.as_str()) {
            let list: Vec<Track> = std::iter::once(&t).chain(&queue).map(pb::to_track).collect();
            self.pl.play_tracks(ctx, &list, 0, false);
            self.pl.playing = false;
            self.party.ready_sent.clear();
        }
        self.party.target = Some(Target { id: t.id, playing, pos, at });
    }

    fn party_guest(&mut self, ctx: &Context) {
        let Some(vid) = self.pl.song().map(|s| s.vid().to_string()) else { return };
        // The room only carries title, artist and a thumbnail; the rest comes from looking the song up, as a normal play has it.
        if self.party.filled != vid {
            match &*self.data.song_info(ctx, &vid).lock() {
                crate::data::State::Ready(t) => self.pl.fill_current(t),
                crate::data::State::Failed(_) => {}
                _ => return self.party_guest_sync(ctx, vid),
            }
            self.party.filled = vid.clone();
        }
        self.party_guest_sync(ctx, vid)
    }

    fn party_guest_sync(&mut self, ctx: &Context, vid: String) {
        if !self.pl.loaded() {
            return;
        }
        if self.party.ready_sent != vid {
            self.party.id("buffer_ready", &vid);
            self.party.ready_sent = vid.clone();
        }
        let Some(t) = self.party.target.take() else { return };
        if t.id != vid {
            return;
        }
        let want = self.party.clock.lock().unwrap().position_at(t.pos, t.at, t.playing);
        let have = (self.pl.pos * 1000.) as i64;
        // While playing, small drift is left alone; a pause or a jump lines up exactly.
        if !(t.playing && self.pl.playing) || (want - have).abs() > HARD_SYNC_MS {
            self.pl.seek(want as f32 / 1000.);
        }
        if t.playing != self.pl.playing {
            self.pl.toggle(ctx);
        }
    }

    /// Announces the host's song, play state and seeks as they change, plus a PLAY heartbeat while playing.
    fn party_host(&mut self) {
        let Some(s) = self.pl.song() else { return };
        // A Spotify song has no YouTube id until it is matched; guests could not play it before then.
        let vid = s.vid().to_string();
        if vid.is_empty() {
            return;
        }
        // Playing only counts once the stream runs: a PLAY sent while it loads would put guests ahead by the load time.
        let (playing, seeks, pos) = (self.pl.playing && self.pl.loaded(), self.pl.seeks, self.pl.pos);
        let party = &mut self.party;
        let (last_vid, last_playing, last_seeks) = party.sent.clone().unwrap_or_default();
        if last_vid != vid {
            let queue: Vec<pb::TrackInfo> = (1..=50).map_while(|i| self.pl.upcoming(i)).filter(|t| t.source == "youtube").map(|t| pb::track(&t, &t.id)).collect();
            party.action("change_track", &vid, 0., Some(pb::track(&s.track(), &vid)), queue);
            if playing {
                party.action("play", &vid, pos, None, vec![]);
            }
            party.beat = Some(Instant::now());
        } else if playing != last_playing {
            party.action(if playing { "play" } else { "pause" }, &vid, pos, None, vec![]);
            party.beat = Some(Instant::now());
        } else if seeks != last_seeks {
            party.action("seek", &vid, pos, None, vec![]);
        } else if playing && party.beat.is_none_or(|b| b.elapsed() >= HEARTBEAT) {
            party.action("play", &vid, pos, None, vec![]);
            party.beat = Some(Instant::now());
        }
        party.sent = Some((vid, playing, seeks));
    }
}

const POP_W: f32 = 320.;

fn initials(name: &str) -> String {
    let mut w = name.split_whitespace().filter_map(|w| w.chars().next());
    let s: String = [w.next(), w.next()].into_iter().flatten().collect();
    if s.is_empty() { "?".into() } else { s.to_uppercase() }
}

/// Small caps label over a section, as in the mockup.
fn label(p: &egui::Painter, t: &crate::theme::Theme, at: egui::Pos2, w: f32, text: &str) {
    trunc(p, at, w, &text.to_uppercase(), font_w(10.5, Weight::Semi), t.tf);
}

/// Rounded button; `strong` is the filled primary style.
fn btn(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, r: Rect, text: &str, strong: bool) -> bool {
    let hit = press(ui, ("party", key), r);
    let (bg, fg) = if strong { (t.tx, t.s0) } else { (if hit.hovered() { t.s1 } else { t.s3 }, t.tx) };
    ui.painter().rect_filled(r, rad(10.), bg);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), fg);
    hit.clicked()
}


impl App {
    /// Titlebar button beside the account pill; filled while a party is live.
    pub fn party_button(&mut self, ui: &mut egui::Ui, c: egui::Pos2) {
        let t = self.t;
        let open = self.party_open.is_some();
        let resp = icon_button(ui, &t, "party-btn", c, 32., if self.party.room.is_some() { icon!("partyf") } else { icon!("party") }, 16., if open { t.tx } else { t.tm });
        self.party_btn = Rect::from_center_size(c, vec2(32., 32.));
        let clicked = resp.clicked();
        crate::tip::Tip::tip(resp, &t, "Listening party");
        if clicked {
            if open {
                self.party_open = None;
            } else {
                (self.palette, self.ctx_menu, self.lib_menu, self.vol_open, self.me_menu) = (None, None, false, false, None);
                self.party_open = Some(ui.input(|i| i.time));
            }
        }
    }

    /// The party popover under its button: start or join when not in one; code, listeners, requests and settings when in one.
    pub fn party_popover(&mut self, ctx: &egui::Context) {
        let Some(t0) = self.party_open else { return };
        if self.palette.is_some() || self.me_menu.is_some() || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.party_open = None;
            return;
        }
        let (t, now) = (self.t, ctx.input(|i| i.time));
        let k = if reduced_motion() { 1. } else { ease_out((((now - t0) / 0.14) as f32).min(1.)) };
        if k < 1. {
            ctx.request_repaint();
        }
        let scr = ctx.content_rect();
        let x0 = (self.party_btn.max.x + 4. - POP_W).clamp(scr.min.x + 12., scr.max.x - 12. - POP_W);
        let pad = 14.;
        let (l, w) = (x0 + pad, POP_W - 2. * pad);
        // Laid out top-down; the height is measured on the way and used next frame (the first frame draws transparent).
        let id = Id::unique("party-h");
        let h = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.);
        let rect = Rect::from_min_size(pos2(x0, scr.min.y + 48. + 6. * (1. - k)), vec2(POP_W, h));
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !self.party_btn.contains(p)));
        if outside && h > 0. {
            self.party_open = None;
            return;
        }
        let name = self.user_name().unwrap_or_else(|| "Serafin listener".into());
        let my_pic = self.youtube_user().and_then(|u| u.0);
        let mut act: Option<Box<dyn FnOnce(&mut App)>> = None;
        egui::Area::new(Id::unique("party-pop")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            ui.set_opacity(if h > 0. { k } else { 0. });
            ui.allocate_rect(rect, egui::Sense::click());
            let p = ui.painter().clone().with_clip_rect(scr);
            shadow(&p, rect, 16., 16, 40, 102);
            p.rect_filled(rect, rad(16.), t.s2);
            let mut y = rect.min.y + pad;
            let live = self.party.room.as_ref().map(|r| r.members.iter().filter(|m| m.connected).count());
            icon(ui, icon!("partyf"), pos2(l + 9., y + 11.), 18., t.tx);
            trunc(&p, pos2(l + 26., y + 11.), w - 26. - 90., "Listening party", font_w(15., Weight::Bold), t.tx);
            // Head count at the right of the title.
            if let Some(n) = live {
                let n = n.to_string();
                let nx = l + w - text_w(&p, &n, font_w(12., Weight::Semi));
                p.text(pos2(l + w, y + 11.), Align2::RIGHT_CENTER, &n, font_w(12., Weight::Semi), t.tx);
                icon(ui, icon!("group"), pos2(nx - 11., y + 11.), 15., t.tm);
            }
            y += 22. + 12.;
            if let Some(s) = &self.party.status {
                let g = p.layout(s.clone(), font(12.5), t.tm, w);
                let gh = g.size().y;
                p.galley(pos2(l, y), g, t.tm);
                y += gh + 12.;
            }

            match &self.party.room {
                None => {
                    let g = p.layout("Listen in sync with friends. Everyone hears the same song at the same moment.".into(), font(13.), t.tm, w);
                    let gh = g.size().y;
                    p.galley(pos2(l, y), g, t.tm);
                    y += gh + 12.;
                    let connecting = self.party.active();
                    let start = Rect::from_min_size(pos2(l, y), vec2(w, 36.));
                    if btn(ui, &t, "start", start, if connecting { "Cancel" } else { "Start a party" }, !connecting) {
                        act = Some(if connecting { Box::new(|a: &mut App| a.party.leave()) } else { let (n, c) = (name.clone(), ctx.clone()); Box::new(move |a: &mut App| a.party.create(&c, &n)) });
                    }
                    y += 36. + 12.;
                    let jw = text_w(&p, "Join", font_w(13., Weight::Semi)) + 28.;
                    let field = Rect::from_min_size(pos2(l, y), vec2(w - jw - 6., 36.));
                    let fid = ui.scope_id().with("party-code");
                    p.rect_filled(field, rad(10.), t.s1);
                    if ui.memory(|m| m.has_focus(fid)) {
                        p.rect_stroke(field, rad(10.), egui::Stroke::new(2., t.ax), egui::StrokeKind::Inside);
                    }
                    let edit = field.shrink2(vec2(12., 0.));
                    let te = ui.put(edit, egui::TextEdit::singleline(&mut self.party_code).id(fid).frame(egui::Frame::NONE).font(font(13.)).align(Align2::LEFT_CENTER)
                        .hint_text(egui::RichText::new("Enter a party code").color(t.tm)).desired_width(edit.width()));
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if (btn(ui, &t, "join", Rect::from_min_size(pos2(field.max.x + 6., y), vec2(jw, 36.)), "Join", false) || enter) && !self.party_code.trim().is_empty() {
                        let (n, code, c) = (name.clone(), self.party_code.clone(), ctx.clone());
                        act = Some(Box::new(move |a: &mut App| a.party.join(&c, &code, &n)));
                    }
                    y += 36.;
                }
                Some(room) => {
                    let host = room.is_host();
                    // Code, and the link Metrolist opens.
                    let code_r = Rect::from_min_size(pos2(l, y), vec2(w, 56.));
                    p.rect_filled(code_r, rad(12.), t.s1);
                    label(&p, &t, pos2(l + 14., y + 17.), w - 120., "Party code");
                    trunc(&p, pos2(l + 14., y + 37.), w - 120., &room.code, egui::FontId::monospace(16.), t.tx);
                    let cw = text_w(&p, "Copy link", font_w(13., Weight::Semi)) + 24.;
                    if btn(ui, &t, "copy", Rect::from_min_size(pos2(code_r.max.x - 10. - cw, y + 12.), vec2(cw, 32.)), "Copy link", false) {
                        ctx.copy_text(format!("{LINK}{}", room.code));
                        crate::toast::info("Link copied");
                    }
                    y += 56. + 14.;

                    label(&p, &t, pos2(l, y + 6.), w, "Listeners");
                    y += 14.;
                    for m in &room.members {
                        let cy = y + 22.;
                        let you = m.id == room.me;
                        // The protocol carries no pictures, so only yours is known; everyone else gets their initials.
                        p.circle_filled(pos2(l + 16., cy), 16., t.axc);
                        p.text(pos2(l + 16., cy), Align2::CENTER_CENTER, initials(&m.name), font_w(11., Weight::Semi), t.tx);
                        if let Some(u) = my_pic.as_deref().filter(|_| you) {
                            paint_cover(ui, u, Rect::from_center_size(pos2(l + 16., cy), vec2(32., 32.)), rad(16.), egui::Color32::WHITE);
                        }
                        let kick = host && !you;
                        let state = if !m.connected { "Offline" } else if room.waiting.contains(&m.id) { "Syncing…" } else { "Synced" };
                        let sw = text_w(&p, state, font(12.));
                        // Every row keeps the trailing icon slot (host's crown, a guest's remove), so the states line up.
                        let right = l + w - 34.;
                        let nw = right - sw - 10. - (l + 42.);
                        trunc(&p, pos2(l + 42., cy - 8.), nw, &if you { format!("{} (you)", m.name) } else { m.name.clone() }, font_w(13., Weight::Semi), t.tx);
                        trunc(&p, pos2(l + 42., cy + 9.), nw, if m.id == room.host_id { "Host" } else { "Guest" }, font(11.5), t.tm);
                        p.text(pos2(right, cy), Align2::RIGHT_CENTER, state, font(12.), t.tm);
                        if m.id == room.host_id {
                            icon(ui, icon!("crown"), pos2(l + w - 14., cy), 15., t.ax);
                        } else if kick && icon_button(ui, &t, ("party-kick", &m.id), pos2(l + w - 14., cy), 28., icon!("x"), 14., t.tm).clicked() {
                            let id = m.id.clone();
                            act = Some(Box::new(move |a: &mut App| a.party.kick(&id)));
                        }
                        y += 44.;
                    }

                    // Host only: join requests (initials) and suggestions (the song's cover), each a card with accept / decline.
                    let rows: Vec<(String, String, String, Option<String>, bool)> = if host {
                        room.requests.iter().map(|(id, n)| (id.clone(), n.clone(), "Wants to join".into(), None, true))
                            .chain(room.suggestions.iter().map(|s| (s.id.clone(), s.track.title.clone(), format!("Suggested by {}", s.from), Some(s.track.thumbnail.clone()), false)))
                            .collect()
                    } else {
                        vec![]
                    };
                    if !rows.is_empty() {
                        y += 8.;
                        label(&p, &t, pos2(l, y + 6.), w, "Requests");
                        y += 14.;
                    }
                    for (i, (id, title, sub, cover, join)) in rows.into_iter().enumerate() {
                        let card = Rect::from_min_size(pos2(l, y + 2.), vec2(w, 56.));
                        p.rect_filled(card, rad(12.), t.s1);
                        let cy = card.center().y;
                        let pic = pos2(card.min.x + 10. + 16., cy);
                        match cover.filter(|u| !u.is_empty()) {
                            Some(u) => self.thumb(ui, Some(&u), Rect::from_center_size(pic, vec2(32., 32.)), 6.),
                            None => {
                                p.circle_filled(pic, 16., t.axc);
                                p.text(pic, Align2::CENTER_CENTER, initials(&title), font_w(11., Weight::Semi), t.tx);
                            }
                        }
                        let yes_label = if join { "Accept" } else { "Add" };
                        let yw = text_w(&p, yes_label, font_w(12.5, Weight::Semi)) + 24.;
                        let yes = Rect::from_min_size(pos2(card.max.x - 10. - yw, cy - 14.), vec2(yw, 28.));
                        let no_c = pos2(yes.min.x - 6. - 14., cy);
                        let tx = card.min.x + 10. + 32. + 10.;
                        let nw = no_c.x - 14. - 8. - tx;
                        trunc(&p, pos2(tx, cy - 8.), nw, &title, font_w(13., Weight::Semi), t.tx);
                        trunc(&p, pos2(tx, cy + 9.), nw, &sub, font(11.5), t.tm);
                        let no_hit = press(ui, ("party-no", i), Rect::from_center_size(no_c, vec2(28., 28.)));
                        p.circle_filled(no_c, 14., if no_hit.hovered() { t.s2 } else { t.s3 });
                        icon(ui, icon!("x"), no_c, 13., t.tm);
                        let yes_hit = press(ui, ("party-yes", i), yes);
                        p.rect_filled(yes, rad(14.), if yes_hit.hovered() { t.tx.gamma_multiply(0.88) } else { t.tx });
                        p.text(yes.center(), Align2::CENTER_CENTER, yes_label, font_w(12.5, Weight::Semi), t.s0);
                        if let Some(ok) = [(yes_hit.clicked(), true), (no_hit.clicked(), false)].into_iter().find(|c| c.0).map(|c| c.1) {
                            let id = id.clone();
                            act = Some(Box::new(move |a: &mut App| if join { a.party.answer_join(&id, ok) } else { a.party.answer_suggestion(&id, ok) }));
                        }
                        y += 56. + 6.;
                    }

                    if host {
                        y += 8.;
                        label(&p, &t, pos2(l, y + 6.), w, "Guests can");
                        y += 14.;
                        let cy = y + 20.;
                        trunc(&p, pos2(l, cy), w - 56., "Add to the queue", font_w(13., Weight::Semi), t.tx);
                        let mut on = self.party.auto_add;
                        if crate::settings::toggle(ui, &t, "party-auto", l + w, cy, &mut on) {
                            act = Some(Box::new(move |a: &mut App| a.party.auto_add = on));
                        }
                        y += 40.;
                    }
                    y += 12.;
                    if btn(ui, &t, "end", Rect::from_min_size(pos2(l, y), vec2(w, 36.)), if host { "End party" } else { "Leave party" }, false) {
                        act = Some(Box::new(|a: &mut App| a.party.leave()));
                    }
                    y += 36.;
                }
            }
            let nh = y + pad - rect.min.y;
            if (nh - h).abs() > 0.5 {
                ctx.data_mut(|d| d.insert_temp(id, nh));
                ctx.request_repaint();
            }
        });
        if let Some(f) = act {
            f(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Conn {
        out: Sender<Vec<u8>>,
        inbox: Receiver<In>,
        clock: Arc<Mutex<Clock>>,
    }

    fn connect(first: Vec<u8>) -> Conn {
        let (out, out_rx) = channel();
        let (in_tx, inbox) = channel();
        out.send(first).unwrap();
        let clock = Arc::new(Mutex::new(Clock::default()));
        let c = clock.clone();
        std::thread::spawn(move || worker(Context::default(), out_rx, in_tx, c, Default::default()));
        Conn { out, inbox, clock }
    }

    /// The next message of type `kind`, skipping others; panics after 15s.
    fn wait(c: &Conn, kind: &str) -> Vec<u8> {
        let end = Instant::now() + Duration::from_secs(15);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match c.inbox.recv_timeout(left) {
                Ok(In::Msg(k, b)) if k == kind => return b,
                Ok(In::Msg(..)) | Ok(In::Lost) => {}
                Ok(In::Failed(e)) => panic!("{e}"),
                Err(_) => break,
            }
        }
        panic!("no {kind} within 15s");
    }

    #[test]
    fn clock_extrapolates_only_while_playing() {
        let mut c = Clock::default();
        assert_eq!(c.position_at(1000, 5, true), 1000, "no offset yet");
        c.offset = Some(10_000.);
        let at = c.now().unwrap() - 500;
        assert!((1500..1600).contains(&c.position_at(1000, at, true)));
        assert_eq!(c.position_at(1000, at, false), 1000);
    }

    /// Joins the room in PARTY_CODE as a guest and prints what the host sends for 5 minutes:
    /// `PARTY_CODE=ABC123 cargo test -p serafin listen_to_party -- --ignored --nocapture`
    #[test]
    #[ignore = "joins a live room; needs PARTY_CODE"]
    fn listen_to_party() {
        let code = std::env::var("PARTY_CODE").expect("PARTY_CODE");
        let g = connect(encode("join_room", Some(&pb::Join { room_code: code.trim().to_uppercase(), username: "Serafin test guest".into() })));
        let end = Instant::now() + Duration::from_secs(300);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let Ok(In::Msg(k, b)) = g.inbox.recv_timeout(left) else { continue };
            match k.as_str() {
                "sync_playback" => {
                    let a = pb::PlaybackAction::decode(&b[..]).unwrap();
                    println!("{k}: {} track={} pos={}ms server_time={} info={:?} queue={}", a.action, a.track_id, a.position, a.server_time, a.track_info.map(|t| t.title), a.queue.len());
                }
                "join_approved" => {
                    // A suggestion, so the host sees that card too.
                    g.out.send(encode("suggest_track", Some(&pb::SuggestTrack { track_info: Some(pb::TrackInfo { id: "fJ9rUzIMcZQ".into(), title: "Bohemian Rhapsody".into(), artist: "Queen".into(), duration: 354_000, thumbnail: "https://i.ytimg.com/vi/fJ9rUzIMcZQ/hqdefault.jpg".into(), ..Default::default() }) }))).unwrap();
                    let s = pb::Session::decode(&b[..]).unwrap().state.unwrap_or_default();
                    println!("{k}: host={} users={:?} track={:?} playing={} pos={}ms", s.host_id, s.users.iter().map(|u| &u.username).collect::<Vec<_>>(), s.current_track.map(|t| t.title), s.is_playing, s.position);
                }
                _ => println!("{k}"),
            }
        }
    }

    /// Hosts a scripted room for 10 minutes: prints the code, lets everyone in, approves suggestions, and drives playback
    /// (song A from 0, a seek, a pause and resume, then song B). `cargo test -p serafin host_party -- --ignored --nocapture`
    #[test]
    #[ignore = "hosts a live room for 10 minutes"]
    fn host_party() {
        let h = connect(encode("create_room", Some(&pb::Name { username: "Serafin test host".into() })));
        let room = pb::Session::decode(&wait(&h, "room_created")[..]).unwrap();
        println!("PARTY CODE: {}", room.room_code);
        let songs = [
            pb::TrackInfo { id: "dQw4w9WgXcQ".into(), title: "Never Gonna Give You Up".into(), artist: "Rick Astley".into(), duration: 213_000, ..Default::default() },
            pb::TrackInfo { id: "fJ9rUzIMcZQ".into(), title: "Bohemian Rhapsody".into(), artist: "Queen".into(), duration: 354_000, ..Default::default() },
        ];
        let t0 = Instant::now();
        let log = |m: String| println!("[{:5.1}s] {m}", t0.elapsed().as_secs_f32());
        let send = |action: &str, track: &pb::TrackInfo, pos: i64, info: bool| {
            let captured = h.clock.lock().unwrap().now().unwrap_or(0);
            h.out.send(encode("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: track.id.clone(), position: pos, track_info: info.then(|| track.clone()), volume: 1., captured_at_server_time: captured, ..Default::default() }))).unwrap();
        };
        // (song, position it was at when `since` was taken, playing)
        let (mut cur, mut base, mut since, mut playing) = (0usize, 0i64, Instant::now(), true);
        let pos = |base: i64, since: Instant, playing: bool| if playing { base + since.elapsed().as_millis() as i64 } else { base };
        // Waits for someone to join before the song starts, so the guest sees it from the top.
        let mut started = false;
        let (mut beat, mut step) = (Instant::now(), 0);
        while t0.elapsed() < Duration::from_secs(600) {
            if let Ok(In::Msg(k, b)) = h.inbox.recv_timeout(Duration::from_millis(200)) {
                match k.as_str() {
                    "join_request" => {
                        let u = pb::User::decode(&b[..]).unwrap();
                        log(format!("join request from {} — approving", u.username));
                        h.out.send(encode("approve_join", Some(&pb::Id { id: u.user_id, reason: String::new() }))).unwrap();
                    }
                    "suggestion_received" => {
                        let s = pb::Suggestion::decode(&b[..]).unwrap();
                        log(format!("suggestion from {}: {:?} — approving", s.from_username, s.track_info.map(|t| t.title)));
                        h.out.send(encode("approve_suggestion", Some(&pb::Id { id: s.suggestion_id, reason: String::new() }))).unwrap();
                    }
                    "sync_playback" => {
                        let a = pb::PlaybackAction::decode(&b[..]).unwrap();
                        log(format!("{k}: {} {:?}", a.action, a.track_info.map(|t| t.title)));
                    }
                    "buffer_wait" => log(format!("buffer_wait: {:?}", pb::BufferWait::decode(&b[..]).unwrap().waiting_for)),
                    "user_joined" | "user_left" | "user_disconnected" | "user_reconnected" => log(format!("{k}: {}", pb::User::decode(&b[..]).unwrap().username)),
                    _ => log(k.clone()),
                }
                if k == "user_joined" && !started {
                    started = true;
                    (base, since, playing) = (0, Instant::now(), true);
                    send("change_track", &songs[cur], 0, true);
                    send("play", &songs[cur], 0, false);
                    log(format!("playing {} from 0:00", songs[cur].title));
                    (beat, step) = (Instant::now(), 0);
                }
            }
            if !started {
                continue;
            }
            let at = beat.elapsed().as_secs();
            let script = [(40, "seek"), (60, "pause"), (70, "play"), (95, "next")];
            if let Some(&(_, what)) = script.get(step).filter(|(s, _)| at >= *s) {
                step += 1;
                match what {
                    "seek" => {
                        (base, since) = (90_000, Instant::now());
                        send("seek", &songs[cur], base, false);
                        log("seek to 1:30".into());
                    }
                    "pause" => {
                        (base, playing) = (pos(base, since, playing), false);
                        send("pause", &songs[cur], base, false);
                        log(format!("pause at {}s", base / 1000));
                    }
                    "play" => {
                        (since, playing) = (Instant::now(), true);
                        send("play", &songs[cur], base, false);
                        log(format!("resume at {}s", base / 1000));
                    }
                    _ => {
                        cur = 1;
                        (base, since, playing) = (0, Instant::now(), true);
                        send("change_track", &songs[cur], 0, true);
                        send("play", &songs[cur], 0, false);
                        log(format!("next song: {}", songs[cur].title));
                    }
                }
            }
            // Heartbeat, as the app does.
            if playing && since.elapsed().as_millis() % 8000 < 200 && since.elapsed() > Duration::from_secs(1) {
                send("play", &songs[cur], pos(base, since, playing), false);
            }
        }
        h.out.send(encode("leave_room", None)).unwrap();
    }

    #[test]
    #[ignore = "talks to Metrolist's party server; run with --ignored"]
    fn live_room_round_trip() {
        let host = connect(encode("create_room", Some(&pb::Name { username: "serafin-test-host".into() })));
        let created = pb::Session::decode(&wait(&host, "room_created")[..]).unwrap();
        assert!(!created.room_code.is_empty() && !created.session_token.is_empty());

        let guest = connect(encode("join_room", Some(&pb::Join { room_code: created.room_code.clone(), username: "serafin-test-guest".into() })));
        let req = pb::User::decode(&wait(&host, "join_request")[..]).unwrap();
        assert_eq!(req.username, "serafin-test-guest");
        host.out.send(encode("approve_join", Some(&pb::Id { id: req.user_id.clone(), reason: String::new() }))).unwrap();
        let joined = pb::Session::decode(&wait(&guest, "join_approved")[..]).unwrap();
        assert_eq!(joined.room_code, created.room_code);
        let state = joined.state.unwrap();
        assert_eq!(state.host_id, created.user_id);
        assert_eq!(state.users.len(), 2);

        let track = pb::TrackInfo { id: "dQw4w9WgXcQ".into(), title: "Never Gonna Give You Up".into(), artist: "Rick Astley".into(), duration: 213_000, ..Default::default() };
        let act = |action: &str, pos: i64, info: Option<pb::TrackInfo>| {
            let captured = host.clock.lock().unwrap().now().unwrap_or(0);
            encode("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: track.id.clone(), position: pos, track_info: info, volume: 1., captured_at_server_time: captured, ..Default::default() }))
        };
        host.out.send(act("change_track", 0, Some(track.clone()))).unwrap();
        let change = pb::PlaybackAction::decode(&wait(&guest, "sync_playback")[..]).unwrap();
        assert_eq!(change.action, "change_track");
        assert_eq!(change.track_info.unwrap().id, track.id);

        guest.out.send(encode("buffer_ready", Some(&pb::Id { id: track.id.clone(), reason: String::new() }))).unwrap();
        host.out.send(act("play", 12_000, None)).unwrap();
        let play = loop {
            let a = pb::PlaybackAction::decode(&wait(&guest, "sync_playback")[..]).unwrap();
            if a.action == "play" {
                break a;
            }
        };
        // The server moves the position on from the host's capture time to its own `server_time`.
        assert!((12_000..12_500).contains(&play.position), "position {}", play.position);
        assert!(play.server_time > 0, "play carries the server time it is valid at");
        assert!(guest.clock.lock().unwrap().now().is_some(), "pongs set the guest's clock");

        guest.out.send(encode("suggest_track", Some(&pb::SuggestTrack { track_info: Some(pb::TrackInfo { id: "fJ9rUzIMcZQ".into(), title: "Bohemian Rhapsody".into(), artist: "Queen".into(), duration: 354_000, ..Default::default() }) }))).unwrap();
        let sug = pb::Suggestion::decode(&wait(&host, "suggestion_received")[..]).unwrap();
        assert_eq!(sug.from_username, "serafin-test-guest");
        assert_eq!(sug.track_info.unwrap().id, "fJ9rUzIMcZQ");
        host.out.send(encode("approve_suggestion", Some(&pb::Id { id: sug.suggestion_id, reason: String::new() }))).unwrap();
        let add = loop {
            let a = pb::PlaybackAction::decode(&wait(&host, "sync_playback")[..]).unwrap();
            if a.action == "queue_add" {
                break a;
            }
        };
        assert_eq!(add.track_info.unwrap().id, "fJ9rUzIMcZQ");

        host.out.send(encode("kick_user", Some(&pb::Id { id: req.user_id, reason: String::new() }))).unwrap();
        wait(&guest, "kicked");
        host.out.send(encode("leave_room", None)).unwrap();
    }
}
