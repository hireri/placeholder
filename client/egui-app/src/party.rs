//! Listening parties: Metrolist's Listen Together rooms, on its server and protocol, so a room can mix Serafin and
//! Metrolist listeners. A worker thread owns the WebSocket (protobuf envelopes, gzip when flagged) and the server clock;
//! the UI thread sends messages and applies what arrives once per frame. The host's player goes out as playback actions,
//! a guest's player follows them. Positions and times on the wire are milliseconds.

use crate::gfx::Calm;
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

/// The party server: Serafin's own fork of metroserver (desktop/party-server), where every listener controls playback.
/// SERAFIN_PARTY_SERVER overrides it for a local or test server.
const SERVER: &str = "wss://party.aveline.fish/ws";

fn server() -> String {
    std::env::var("SERAFIN_PARTY_SERVER").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| SERVER.into())
}
/// How many can be in a party; the server enforces the same limit.
pub const MAX: i32 = 8;
/// Pings carry our playhead, so the server can tell the room when we drift out of step.
const PING_EVERY: Duration = Duration::from_secs(5);
/// A playing listener only seeks when this far off; smaller drift is left alone.
const HARD_SYNC_MS: i64 = 2000;
/// How often a playing listener compares itself with the room.
const DRIFT_CHECK: Duration = Duration::from_secs(1);
/// Upcoming songs shared with the room.
const QUEUE_MAX: usize = 50;

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
        #[prost(string, tag = "5")]
        pub avatar_url: String,
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
        #[prost(string, tag = "3")]
        pub avatar_url: String,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Name {
        #[prost(string, tag = "1")]
        pub username: String,
        #[prost(string, tag = "2")]
        pub avatar_url: String,
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
        /// change_track: replace the upcoming queue even when it is empty.
        #[prost(bool, tag = "12")]
        pub replace_queue: bool,
        /// Who caused a broadcast; our own come back too and are already applied.
        #[prost(string, tag = "13")]
        pub origin_user_id: String,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Ping {
        #[prost(int64, tag = "1")]
        pub client_time: i64,
        #[prost(uint64, tag = "2")]
        pub sequence: u64,
        #[prost(bool, tag = "3")]
        pub has_position: bool,
        #[prost(string, tag = "4")]
        pub track_id: String,
        #[prost(int64, tag = "5")]
        pub position: i64,
        #[prost(int64, tag = "6")]
        pub position_captured_at_server_time: i64,
    }

    /// Sent while the host has yet to answer a join request.
    #[derive(Clone, PartialEq, Message)]
    pub struct JoinPending {
        #[prost(string, tag = "1")]
        pub host_name: String,
        #[prost(uint32, tag = "2")]
        pub position: u32,
        #[prost(bool, tag = "3")]
        pub host_away: bool,
    }

    /// A listener fell out of step with the room, or got back in.
    #[derive(Clone, PartialEq, Message)]
    pub struct MemberSync {
        #[prost(string, tag = "1")]
        pub user_id: String,
        #[prost(bool, tag = "2")]
        pub in_sync: bool,
        #[prost(int64, tag = "3")]
        pub drift_ms: i64,
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

    /// join_request, user_joined/left/reconnected/disconnected, host_changed: (user id, name); user_joined adds a picture.
    #[derive(Clone, PartialEq, Message)]
    pub struct User {
        #[prost(string, tag = "1")]
        pub user_id: String,
        #[prost(string, tag = "2")]
        pub username: String,
        #[prost(string, tag = "3")]
        pub avatar_url: String,
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

/// Our playhead as the UI last saw it, sent with each ping.
#[derive(Clone, Default)]
struct Report {
    track: String,
    pos_ms: i64,
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
fn worker(ctx: Context, out: Receiver<Vec<u8>>, inbox: Sender<In>, clock: Arc<Mutex<Clock>>, token: Arc<Mutex<String>>, report: Arc<Mutex<Report>>) {
    let mut attempt = 0u32;
    loop {
        let (mut ws, _) = match tungstenite::connect(server()) {
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
                let r = report.lock().unwrap().clone();
                let at = clock.lock().unwrap().now().unwrap_or(0);
                let ping = pb::Ping { client_time: mono_ms(), sequence: seq, has_position: !r.track.is_empty() && at > 0, track_id: r.track, position: r.pos_ms, position_captured_at_server_time: at };
                let _ = ws.send(tungstenite::Message::Binary(encode("ping", Some(&ping)).into()));
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
    pub avatar: String,
    pub host: bool,
    pub connected: bool,
}

pub struct Room {
    pub code: String,
    pub me: String,
    /// Keeps the room: approves joins and removes listeners. Everyone controls playback.
    pub host_id: String,
    pub members: Vec<Member>,
    /// Join requests waiting on the host: (user id, name).
    pub requests: Vec<(String, String)>,
    /// Listeners still loading the current song.
    pub waiting: Vec<String>,
    /// Listeners the server says are out of step: (user id, milliseconds ahead, negative when behind).
    pub drifting: Vec<(String, i64)>,
}

impl Room {
    pub fn is_host(&self) -> bool {
        !self.me.is_empty() && self.me == self.host_id
    }

    fn from_state(code: String, me: String, s: Option<&pb::RoomState>) -> Room {
        let (host_id, members) = s.map_or_else(Default::default, |s| {
            (s.host_id.clone(), s.users.iter().map(|u| Member { id: u.user_id.clone(), name: u.username.clone(), avatar: u.avatar_url.clone(), host: u.is_host, connected: u.is_connected }).collect())
        });
        Room { code, me, host_id, members, requests: vec![], waiting: vec![], drifting: vec![] }
    }
}

/// Where the room says playback should be, for us to reach once the stream loads.
struct Target {
    id: String,
    playing: bool,
    pos: i64,
    at: i64,
}

/// What the room last heard from us (or what we last took from it): changes to the player beyond this are ours to announce.
#[derive(Clone, Default, PartialEq)]
struct Sent {
    vid: String,
    playing: bool,
    seeks: u32,
    queue: Vec<String>,
}

/// Why there is no party: what the panel says after one ends, is refused or cannot start.
#[derive(Clone, PartialEq)]
pub enum Closed {
    /// The host's name.
    Refused(String),
    Full,
    Unreachable,
    Removed,
    Ended,
    Left,
    Other(String),
}

#[derive(Default)]
pub struct Party {
    out: Option<Sender<Vec<u8>>>,
    inbox: Option<Receiver<In>>,
    clock: Arc<Mutex<Clock>>,
    token: Arc<Mutex<String>>,
    report: Arc<Mutex<Report>>,
    pub room: Option<Room>,
    pub closed: Option<Closed>,
    /// Asked to join and waiting on the host: (their name, our place in line).
    pub pending: Option<(String, u32)>,
    /// Setting up, until the room or the host's answer arrives: true when joining, false when starting.
    busy: Option<bool>,
    pub reconnecting: bool,
    /// Host: asking whether to end the party for everyone.
    pub confirm_end: bool,
    sent: Option<Sent>,
    /// The room's playback, applied once the song is loaded and re-checked while playing.
    target: Option<Target>,
    fresh: bool,
    /// Seek to the room's position even when close: set by the Sync button.
    exact: bool,
    checked: Option<Instant>,
    ready_sent: String,
    /// When this connection was started, for the timing lines in the terminal.
    t0: Option<Instant>,
    /// Listeners shown stacked on the titlebar button, including ones still shrinking away: (id, name, picture).
    faces: Vec<(String, String, String)>,
    /// The song whose full details (cover, album, artist links) were looked up.
    filled: String,
}

impl Party {
    fn start(&mut self, ctx: &Context, first: Vec<u8>, joining: bool) {
        self.leave();
        let (out_tx, out_rx) = channel();
        let (in_tx, in_rx) = channel();
        let _ = out_tx.send(first);
        (self.clock, self.token, self.report) = Default::default();
        let (ctx, clock, token, report) = (ctx.clone(), self.clock.clone(), self.token.clone(), self.report.clone());
        std::thread::spawn(move || worker(ctx, out_rx, in_tx, clock, token, report));
        (self.out, self.inbox, self.busy) = (Some(out_tx), Some(in_rx), Some(joining));
        self.t0 = Some(Instant::now());
    }

    fn lap(&self, what: &str) {
        if let Some(t0) = self.t0 {
            eprintln!("party: {what} after {} ms", t0.elapsed().as_millis());
        }
    }

    pub fn create(&mut self, ctx: &Context, name: &str, avatar: &str) {
        self.start(ctx, encode("create_room", Some(&pb::Name { username: name.into(), avatar_url: avatar.into() })), false);
    }

    /// Takes a bare code or a pasted share link.
    pub fn join(&mut self, ctx: &Context, code: &str, name: &str, avatar: &str) {
        let code = code.trim().rsplit("code=").next().unwrap_or("").trim().to_uppercase();
        if !code.is_empty() {
            self.start(ctx, encode("join_room", Some(&pb::Join { room_code: code, username: name.into(), avatar_url: avatar.into() })), true);
        }
    }

    pub fn leave(&mut self) {
        if self.room.is_some() {
            self.send("leave_room", None);
        }
        // Dropping the sender ends the worker once it has flushed what is queued.
        *self = Party::default();
    }

    /// Leaves, and the panel says why there is no party.
    pub fn close(&mut self, why: Closed) {
        self.leave();
        self.closed = Some(why);
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

    /// Asks the room for where it is now and goes there, for when we have drifted.
    pub fn resync(&mut self) {
        self.send("request_sync", None);
        self.exact = true;
        if let Some(r) = &mut self.room {
            let me = r.me.clone();
            r.drifting.retain(|d| d.0 != me);
        }
    }

    pub fn kick(&self, user: &str) {
        self.id("kick_user", user);
    }

    fn action(&self, action: &str, vid: &str, pos_s: f32, info: Option<pb::TrackInfo>, queue: Vec<pb::TrackInfo>) {
        let pos = (pos_s.max(0.) * 1000.) as i64;
        let captured = if matches!(action, "play" | "pause" | "seek") { self.clock.lock().unwrap().now().unwrap_or(0) } else { 0 };
        let replace_queue = action == "change_track";
        self.send("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: vid.into(), position: pos, track_info: info, queue, volume: 1., captured_at_server_time: captured, replace_queue, ..Default::default() }));
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
                In::Lost if self.party.room.is_some() => self.party.reconnecting = true,
                In::Lost => {}
                In::Failed(e) => {
                    crate::toast::error(e.clone());
                    self.party.close(if e.contains("can't reach") { Closed::Unreachable } else { Closed::Other(e) });
                }
            }
        }
        if self.party.room.is_none() {
            return;
        }
        if self.opts.party_auto_accept && self.party.room.as_ref().is_some_and(|r| r.is_host() && !r.requests.is_empty()) {
            let waiting: Vec<String> = self.party.room.as_ref().map_or(vec![], |r| r.requests.iter().map(|q| q.0.clone()).collect());
            for id in waiting {
                self.party.answer_join(&id, true);
            }
        }
        // ponytail: radio songs would differ per listener and each would announce its own; no endless mode in a party.
        self.pl.endless = false;
        self.party_follow_tick(ctx);
        self.party_announce();
        let (track, pos_ms) = match self.pl.song() {
            Some(s) if self.pl.loaded() && matches!(s.source(), "yt" | "spotify") => (s.vid().to_string(), (self.pl.pos * 1000.) as i64),
            _ => Default::default(),
        };
        *self.party.report.lock().unwrap() = Report { track, pos_ms };
    }

    /// Who we are in a party: the signed-in account's name (the last known one while it loads), else the stored "Guest 1234".
    fn party_name(&self) -> String {
        self.user_name().or_else(|| Some(self.me.0.clone()).filter(|n| !n.is_empty())).unwrap_or_else(|| self.guest.clone())
    }

    /// A system notification, if that kind is on and (by default) the window is not in front.
    fn party_alert(&self, ctx: &Context, on: bool, title: &str, body: &str) {
        if on && !(self.opts.notify_background && ctx.input(|i| i.viewport().focused.unwrap_or(true))) {
            crate::notify::send(title, body);
        }
    }

    fn my_avatar(&self) -> String {
        self.youtube_user().and_then(|u| u.0).unwrap_or_else(|| self.me.1.clone())
    }

    /// The player as the room would see it.
    fn local_sent(&self) -> Sent {
        Sent {
            vid: self.pl.song().filter(|s| matches!(s.source(), "yt" | "spotify")).map(|s| s.vid().to_string()).unwrap_or_default(),
            playing: self.pl.playing,
            seeks: self.pl.seeks,
            queue: self.shared_upcoming().iter().map(|t| t.id.clone()).collect(),
        }
    }

    /// The songs after the current one that can be shared (YouTube Music ones).
    fn shared_upcoming(&self) -> Vec<Track> {
        (1..=QUEUE_MAX).map_while(|i| self.pl.upcoming(i)).filter(|t| t.source == "youtube").collect()
    }

    fn party_msg(&mut self, ctx: &Context, kind: &str, b: &[u8]) {
        let user = || pb::User::decode(b).unwrap_or_default();
        match kind {
            "room_created" => {
                let s = pb::Session::decode(b).unwrap_or_default();
                *self.party.token.lock().unwrap() = s.session_token;
                let name = self.party_name();
                let mut room = Room::from_state(s.room_code, s.user_id.clone(), None);
                room.host_id = s.user_id.clone();
                room.members.push(Member { id: s.user_id, name, avatar: self.my_avatar(), host: true, connected: true });
                // An empty baseline, so whatever is playing is announced to the room.
                (self.party.room, self.party.busy, self.party.sent) = (Some(room), None, Some(Sent::default()));
            }
            "join_approved" => {
                let s = pb::Session::decode(b).unwrap_or_default();
                *self.party.token.lock().unwrap() = s.session_token;
                self.party.room = Some(Room::from_state(s.room_code, s.user_id, s.state.as_ref()));
                (self.party.busy, self.party.pending) = (None, None);
                self.party.lap("approved");
                self.party_alert(ctx, self.opts.notify_requests, "You're in the party", "The host let you in");
                self.party_join_state(ctx, s.state);
            }
            "reconnected" => {
                let s = pb::Reconnected::decode(b).unwrap_or_default();
                let me = s.user_id.clone();
                let keep = self.party.room.take();
                let mut room = Room::from_state(s.room_code, me, s.state.as_ref());
                if let Some(old) = keep {
                    room.requests = old.requests;
                }
                if s.is_host {
                    room.host_id = room.me.clone();
                }
                (self.party.room, self.party.reconnecting) = (Some(room), false);
                self.party_join_state(ctx, s.state);
            }
            "join_request" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                crate::toast::info(format!("{} wants to join the party", u.username));
                let name = u.username.clone();
                room.requests.push((u.user_id, u.username));
                self.party_alert(ctx, self.opts.notify_requests && !self.opts.party_auto_accept, "Join request", &format!("{name} wants to join your party"));
            }
            // Someone who was waiting gave up or dropped: take their request off the host's list.
            "join_cancelled" => {
                let u = user();
                if let Some(room) = &mut self.party.room {
                    room.requests.retain(|q| q.0 != u.user_id);
                }
            }
            "join_rejected" | "kicked" => {
                // The panel says it (and a system notification if you are elsewhere), so no toast on top.
                let what = if kind == "kicked" { "You were removed from the party" } else { "The host turned down your request" };
                let on = if kind == "kicked" { self.opts.notify_ended } else { self.opts.notify_requests };
                self.party_alert(ctx, on, "Listening party", what);
                let host = self.party.pending.take().map(|p| p.0).unwrap_or_default();
                self.party.close(if kind == "kicked" { Closed::Removed } else { Closed::Refused(host) });
            }
            "user_joined" | "user_reconnected" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                room.requests.retain(|q| q.0 != u.user_id);
                let name = u.username.clone();
                match room.members.iter_mut().find(|m| m.id == u.user_id) {
                    Some(m) => m.connected = true,
                    None => room.members.push(Member { id: u.user_id, name: u.username, avatar: u.avatar_url, host: false, connected: true }),
                }
                if kind == "user_joined" {
                    self.party_alert(ctx, self.opts.notify_people, "Listening party", &format!("{name} joined"));
                }
            }
            "user_left" | "user_disconnected" => {
                let u = user();
                let Some(room) = &mut self.party.room else { return };
                if kind == "user_left" {
                    room.members.retain(|m| m.id != u.user_id);
                    room.waiting.retain(|w| *w != u.user_id);
                    room.drifting.retain(|d| d.0 != u.user_id);
                    self.party_alert(ctx, self.opts.notify_people, "Listening party", &format!("{} left", u.username));
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
                if room.is_host() {
                    crate::toast::info("You are now the party host");
                }
            }
            "join_pending" => {
                let p = pb::JoinPending::decode(b).unwrap_or_default();
                let host = if p.host_name.is_empty() { "the host".to_string() } else { p.host_name };
                (self.party.pending, self.party.busy) = (Some((host, p.position)), None);
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
            "member_sync" => {
                let m = pb::MemberSync::decode(b).unwrap_or_default();
                if let Some(room) = &mut self.party.room {
                    room.drifting.retain(|d| d.0 != m.user_id);
                    if !m.in_sync {
                        room.drifting.push((m.user_id, m.drift_ms));
                    }
                }
            }
            "sync_state" => {
                let s = pb::SyncState::decode(b).unwrap_or_default();
                self.party_follow(ctx, s.current_track, s.queue, s.is_playing, s.position, s.last_update);
            }
            "sync_playback" => {
                let a = pb::PlaybackAction::decode(b).unwrap_or_default();
                self.party_action(ctx, a);
            }
            "error" => {
                let e = pb::Error::decode(b).unwrap_or_default();
                if e.code == "session_not_found" || (e.code == "not_in_room" && self.party.room.is_some()) {
                    *self.party.token.lock().unwrap() = String::new();
                    self.party.close(Closed::Ended);
                    self.party_alert(ctx, self.opts.notify_ended, "Listening party", "The party has ended");
                } else if e.code == "stale_track" || e.code == "stale_action" {
                    // Someone changed the song a moment before us; their change reaches us next.
                } else {
                    let msg = if e.message.is_empty() { e.code.clone() } else { e.message };
                    if self.party.room.is_none() {
                        self.party.close(if e.code == "room_full" { Closed::Full } else { Closed::Other(msg.clone()) });
                    }
                    crate::toast::error(msg);
                }
            }
            _ => {}
        }
    }

    /// Joining or resuming: take the room's song if it has one, else keep what is playing without announcing it.
    fn party_join_state(&mut self, ctx: &Context, st: Option<pb::RoomState>) {
        match st.filter(|s| s.current_track.is_some()) {
            Some(st) => self.party_follow(ctx, st.current_track, st.queue, st.is_playing, st.position, st.last_update),
            None => self.party.sent = Some(self.local_sent()),
        }
    }

    /// A playback action from another listener (ours come back too, and are already in the player).
    fn party_action(&mut self, ctx: &Context, a: pb::PlaybackAction) {
        if self.party.room.as_ref().is_some_and(|r| r.me == a.origin_user_id) {
            return;
        }
        let at = if a.server_time > 0 { a.server_time } else { a.captured_at_server_time };
        match a.action.as_str() {
            "queue_add" | "queue_remove" | "queue_clear" | "sync_queue" => self.party_set_queue(&a.queue),
            // Everyone's player moves on at the end of a song, so the same change arrives from several; the first one counts.
            "change_track" if a.track_info.as_ref().map(|t| t.id.as_str()) == self.pl.song().map(|s| s.vid()) => {}
            "change_track" => self.party_follow(ctx, a.track_info, a.queue, true, 0, at),
            "play" | "pause" | "seek" => {
                let cur = self.pl.song().map(|s| s.vid().to_string()).unwrap_or_default();
                let playing = match a.action.as_str() {
                    "play" => true,
                    "pause" => false,
                    _ => self.party.target.as_ref().map_or(self.pl.playing, |t| t.playing),
                };
                let id = if a.track_id.is_empty() { cur.clone() } else { a.track_id.clone() };
                if id != cur {
                    // An action for a song we don't have (we joined mid-change, or missed it): ask for the full state.
                    self.party.send("request_sync", None);
                    return;
                }
                (self.party.target, self.party.fresh) = (Some(Target { id, playing, pos: a.position, at }), true);
            }
            _ => {}
        }
    }

    /// Replaces the songs after the current one with the room's.
    fn party_set_queue(&mut self, queue: &[pb::TrackInfo]) {
        if self.pl.song().is_none() {
            return;
        }
        let ids: Vec<String> = queue.iter().map(|t| t.id.clone()).collect();
        if self.shared_upcoming().iter().map(|t| &t.id).ne(ids.iter()) {
            let tracks: Vec<Track> = queue.iter().map(pb::to_track).collect();
            self.pl.set_upcoming(&tracks);
        }
        if let Some(s) = &mut self.party.sent {
            s.queue = ids;
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
        self.party.sent = Some(Sent { vid: t.id.clone(), playing: self.pl.playing, seeks: self.pl.seeks, queue: vec![] });
        self.party_set_queue(&queue);
        (self.party.target, self.party.fresh) = (Some(Target { id: t.id, playing, pos, at }), true);
    }

    fn party_follow_tick(&mut self, ctx: &Context) {
        let Some(vid) = self.pl.song().filter(|s| matches!(s.source(), "yt" | "spotify")).map(|s| s.vid().to_string()) else { return };
        // The room only carries title, artist and a thumbnail; the rest comes from looking the song up, as a normal play has it.
        if self.party.filled != vid {
            match &*self.data.song_info(ctx, &vid).lock() {
                crate::data::State::Ready(t) => self.pl.fill_current(t),
                crate::data::State::Failed(_) => {}
                _ => return self.party_apply(ctx, vid),
            }
            self.party.filled = vid.clone();
        }
        self.party_apply(ctx, vid)
    }

    /// Brings the player to where the room is: exactly on a pause, jump or new song, and only past `HARD_SYNC_MS` of drift while playing.
    fn party_apply(&mut self, ctx: &Context, vid: String) {
        if !self.pl.loaded() {
            return;
        }
        if self.party.ready_sent != vid {
            self.party.lap("song loaded");
            self.party.id("buffer_ready", &vid);
            self.party.ready_sent = vid.clone();
        }
        let party = &mut self.party;
        let Some(t) = party.target.as_ref().filter(|t| t.id == vid) else { return };
        let steady = t.playing && self.pl.playing;
        if !(party.fresh || (steady && party.checked.is_none_or(|c| c.elapsed() >= DRIFT_CHECK))) {
            return;
        }
        let exact = std::mem::take(&mut party.exact);
        (party.fresh, party.checked) = (false, Some(Instant::now()));
        let want = party.clock.lock().unwrap().position_at(t.pos, t.at, t.playing);
        let have = (self.pl.pos * 1000.) as i64;
        let playing = t.playing;
        if !steady || exact || (want - have).abs() > HARD_SYNC_MS {
            self.pl.seek(want as f32 / 1000.);
        }
        if playing != self.pl.playing {
            self.pl.toggle(ctx);
        }
        // What the room told us is not ours to announce back.
        if let Some(s) = &mut party.sent {
            (s.playing, s.seeks) = (self.pl.playing, self.pl.seeks);
        }
    }

    /// Tells the room what changed in our player: a new song (with the queue), play or pause, a seek, or the queue.
    fn party_announce(&mut self) {
        let Some(song) = self.pl.song() else { return };
        // A Spotify song has no YouTube id until it is matched; a server's song cannot be streamed by the room.
        if song.vid().is_empty() || !matches!(song.source(), "yt" | "spotify") {
            return;
        }
        let now = self.local_sent();
        let Some(last) = self.party.sent.clone() else { return self.party.sent = Some(now) };
        if now == last {
            return;
        }
        let (vid, pos) = (now.vid.clone(), self.pl.pos);
        let party = &self.party;
        if now.vid != last.vid {
            let info = pb::track(&song.track(), &vid);
            let queue = self.shared_upcoming().iter().map(|t| pb::track(t, &t.id)).collect();
            party.action("change_track", &vid, 0., Some(info), queue);
            // The server starts a new song playing.
            if !now.playing {
                party.action("pause", &vid, pos, None, vec![]);
            }
        } else {
            if now.playing != last.playing {
                party.action(if now.playing { "play" } else { "pause" }, &vid, pos, None, vec![]);
            }
            if now.seeks != last.seeks {
                party.action("seek", &vid, pos, None, vec![]);
            }
            if now.queue != last.queue {
                let queue = self.shared_upcoming().iter().map(|t| pb::track(t, &t.id)).collect();
                party.action("sync_queue", "", 0., None, queue);
            }
        }
        self.party.sent = Some(now);
    }
}

const POP_W: f32 = 320.;

/// Small caps label over a section, as in the mockup.
pub(crate) fn label(p: &egui::Painter, t: &crate::theme::Theme, at: egui::Pos2, w: f32, text: &str) {
    trunc(p, at, w, &text.to_uppercase(), font_w(10.5, Weight::Semi), t.tf);
}

/// Rounded button; `strong` is the filled primary style.
pub(crate) fn btn(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, r: Rect, text: &str, strong: bool) -> bool {
    let hit = press(ui, ("party", key), r);
    let (bg, fg) = if strong { (t.tx, t.s0) } else { (if hit.hovered() { t.s1 } else { t.s3 }, t.tx) };
    ui.painter().rect_filled(r, rad(10.), bg);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), fg);
    hit.clicked()
}


impl App {
    /// Titlebar button beside the account pill: the party icon, or while a party is live the listeners' pictures stacked
    /// (they grow in as people join and shrink out as they leave). `right` is its right edge; returns its width.
    pub fn party_button(&mut self, ui: &mut egui::Ui, right: f32, cy: f32) -> f32 {
        const D: f32 = 22.;
        const STEP: f32 = 15.;
        const SHOWN: usize = 3;
        let (t, ctx) = (self.t, ui.ctx().clone());
        let open = self.party_open.is_some();
        let me_pic = self.my_avatar();
        // Everyone in the room, then those still fading out.
        let live: Vec<(String, String, String)> = self.party.room.as_ref().map_or(vec![], |r| {
            r.members.iter().filter(|m| m.connected).map(|m| (m.id.clone(), m.name.clone(), if m.avatar.is_empty() && m.id == r.me { me_pic.clone() } else { m.avatar.clone() })).collect()
        });
        for (id, name, av) in &live {
            match self.party.faces.iter_mut().find(|f| f.0 == *id) {
                Some(f) => (f.1, f.2) = (name.clone(), av.clone()),
                None => self.party.faces.push((id.clone(), name.clone(), av.clone())),
            }
        }
        let motion = !reduced_motion();
        let ks: Vec<f32> = self.party.faces.iter().map(|f| {
            let on = if live.iter().any(|l| l.0 == f.0) { 1. } else { 0. };
            if motion { ctx.calm_value(Id::unique("party-face").with(&f.0), on, 0.28) } else { on }
        }).collect();
        let keep: Vec<bool> = self.party.faces.iter().zip(&ks).map(|(f, k)| *k > 0.001 || live.iter().any(|l| l.0 == f.0)).collect();
        let mut it = keep.iter();
        self.party.faces.retain(|_| *it.next().unwrap());
        let faces: Vec<(usize, f32)> = ks.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(v, _)| v).enumerate().collect();

        let extra = live.len().saturating_sub(SHOWN);
        let shown: Vec<&(usize, f32)> = faces.iter().take(SHOWN).collect();
        // Alone in the party, the party icon sits behind your picture; it tucks away when someone else arrives.
        let alone = if live.len() <= 1 { 1. } else { 0. };
        let ik = if motion { ctx.calm_value(Id::unique("party-alone"), alone, 0.28) } else { alone };
        let stack = if shown.is_empty() { 0. } else { D + STEP * (ik + shown.iter().map(|f| f.1).sum::<f32>() - 1.).max(0.) };
        let chip = if extra > 0 { STEP } else { 0. };
        let w = if self.party.room.is_some() { (stack + chip + 14.).max(32.) } else { 32. };
        let rect = Rect::from_min_max(pos2(right - w, cy - 16.), pos2(right, cy + 16.));
        self.party_btn = rect;
        let resp = press(ui, "party-btn", rect);
        if open || resp.hovered() {
            ui.painter().rect_filled(rect, rad(16.), if open { t.s3 } else { t.s2 });
        }
        if self.party.room.is_none() {
            icon(ui, icon!("party"), rect.center(), 16., if open { t.tx } else { t.tm });
        } else {
            let mut x = rect.center().x - (stack + chip) / 2. + D / 2.;
            let mut last = x;
            if ik > 0.001 {
                ui.scope(|ui| {
                    ui.set_opacity(ik.clamp(0., 1.));
                    let c = pos2(x, cy);
                    icon(ui, icon!("partyf"), c, 22. * ik, t.tx);
                });
                x += STEP * ik;
            }
            for (i, k) in shown.iter().map(|f| (f.0, f.1)) {
                last = x;
                let f = &self.party.faces[i];
                let c = pos2(x, cy);
                let r = D / 2. * (0.55 + 0.45 * k);
                ui.scope(|ui| {
                    ui.set_opacity(k.clamp(0., 1.));
                    ui.painter().circle_filled(c, r + 1.5, t.s0);
                    self.avatar(ui, Some(f.2.as_str()).filter(|u| !u.is_empty()), c, r, Some(&f.1));
                });
                x += STEP * k;
            }
            if extra > 0 {
                let c = pos2(last + STEP + 2., cy);
                ui.painter().circle_filled(c, D / 2. + 1.5, t.s0);
                ui.painter().circle_filled(c, D / 2., t.s3);
                ui.painter().text(c, Align2::CENTER_CENTER, format!("+{extra}"), font_w(10., Weight::Semi), t.tx);
            }
        }
        let clicked = resp.clicked();
        crate::tip::Tip::tip(resp, &t, "Listening party");
        if clicked {
            if open {
                self.party_open = None;
            } else if self.sp.group.is_some() {
                crate::toast::info("Leave your SyncPlay group to start a listening party");
            } else {
                (self.palette, self.ctx_menu, self.lib_menu, self.vol_open, self.me_menu) = (None, None, false, false, None);
                self.party_open = Some(ui.input(|i| i.time));
            }
        }
        w
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
        let name = self.party_name();
        let my_pic = self.my_avatar();
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
            if self.party.reconnecting && live.is_some() {
                p.text(pos2(l + w, y + 11.), Align2::RIGHT_CENTER, "Reconnecting…", font_w(12., Weight::Semi), t.tm);
            } else if let Some(n) = live {
                let n = format!("{n} of {MAX}");
                let nx = l + w - text_w(&p, &n, font_w(12., Weight::Semi));
                p.text(pos2(l + w, y + 11.), Align2::RIGHT_CENTER, &n, font_w(12., Weight::Semi), t.tx);
                icon(ui, icon!("group"), pos2(nx - 11., y + 11.), 15., t.tm);
            }
            y += 22. + 12.;
            match &self.party.room {
                Some(room) => {
                    let host = room.is_host();
                    let others = room.members.iter().filter(|m| m.connected && m.id != room.me).count();
                    let left = (MAX as usize).saturating_sub(room.members.len());
                    // The code to share, first: it is what you open this for.
                    let code_r = Rect::from_min_size(pos2(l, y), vec2(w, 56.));
                    p.rect_filled(code_r, rad(12.), t.s1);
                    label(&p, &t, pos2(l + 14., y + 17.), w - 120., &if left == 0 { "Party code · full".to_string() } else { format!("Party code · {left} left") });
                    trunc(&p, pos2(l + 14., y + 37.), w - 120., &room.code, egui::FontId::monospace(16.), t.tx);
                    let cw = text_w(&p, "Copy code", font_w(13., Weight::Semi)) + 24.;
                    if btn(ui, &t, "copy", Rect::from_min_size(pos2(code_r.max.x - 10. - cw, y + 12.), vec2(cw, 32.)), "Copy code", false) {
                        ctx.copy_text(room.code.clone());
                        crate::toast::info("Code copied");
                    }
                    y += 56. + 10.;

                    label(&p, &t, pos2(l, y + 6.), w, "Listeners");
                    y += 14.;
                    for m in &room.members {
                        let cy = y + 22.;
                        let you = m.id == room.me;
                        let row = Rect::from_min_size(pos2(l, y), vec2(w, 44.));
                        if !m.connected {
                            // Offline: a flat grey disc instead of the picture.
                            p.circle_filled(pos2(l + 16., cy), 16., t.s3);
                            p.text(pos2(l + 16., cy), Align2::CENTER_CENTER, m.name.chars().next().unwrap_or('?').to_uppercase().to_string(), font_w(11., Weight::Semi), t.tf);
                        } else {
                            let pic = Some(if m.avatar.is_empty() && you { my_pic.as_str() } else { m.avatar.as_str() }).filter(|u| !u.is_empty());
                            self.avatar(ui, pic, pos2(l + 16., cy), 16., Some(&m.name));
                        }
                        let state = if !m.connected {
                            "Offline".to_string()
                        } else if room.waiting.contains(&m.id) {
                            "Syncing…".to_string()
                        } else if let Some((_, ms)) = room.drifting.iter().find(|d| d.0 == m.id) {
                            format!("Out of sync ({:+.1}s)", *ms as f32 / 1000.)
                        } else {
                            "Synced".to_string()
                        };
                        // Every row keeps the trailing slot (the host's crown, or a remove button on hover), so the states line up.
                        let mut right = l + w - 34.;
                        let mut state = state;
                        // Out of step: the status becomes a button that brings you back.
                        if let Some((_, ms)) = room.drifting.iter().find(|d| d.0 == m.id).filter(|_| you && m.connected) {
                            let bw = text_w(&p, "Sync", font_w(12.5, Weight::Semi)) + 24.;
                            if btn(ui, &t, "resync", Rect::from_min_size(pos2(right - bw, cy - 13.), vec2(bw, 26.)), "Sync", true) {
                                act = Some(Box::new(|a: &mut App| a.party.resync()));
                            }
                            right -= bw + 8.;
                            state = format!("{:.1}s {}", ms.abs() as f32 / 1000., if *ms > 0 { "ahead" } else { "behind" });
                        }
                        let sw = text_w(&p, &state, font(12.));
                        let nw = right - sw - 10. - (l + 42.);
                        let dim = if m.connected { t.tx } else { t.tm };
                        trunc(&p, pos2(l + 42., cy - 8.), nw, &if you { format!("{} (you)", m.name) } else { m.name.clone() }, font_w(13., Weight::Semi), dim);
                        trunc(&p, pos2(l + 42., cy + 9.), nw, if m.id == room.host_id { "Host" } else { "Guest" }, font(11.5), t.tm);
                        p.text(pos2(right, cy), Align2::RIGHT_CENTER, &state, font(12.), t.tm);
                        if m.id == room.host_id {
                            icon(ui, icon!("crown"), pos2(l + w - 14., cy), 15., t.ax);
                        } else if host && !you && ui.rect_contains_pointer(row) && icon_button(ui, &t, ("party-kick", &m.id), pos2(l + w - 14., cy), 28., icon!("x"), 14., t.tm).clicked() {
                            let id = m.id.clone();
                            act = Some(Box::new(move |a: &mut App| a.party.kick(&id)));
                        }
                        y += 44.;
                    }
                    // People at the door follow the listeners (three at a time), so everyone not yet in is in one place.
                    if host && !room.requests.is_empty() {
                        label(&p, &t, pos2(l, y + 6.), w - 30., "Wants to join");
                        p.text(pos2(l + w, y + 6.), Align2::RIGHT_CENTER, room.requests.len().to_string(), font_w(10.5, Weight::Semi), t.tf);
                        y += 14.;
                        for (i, (id, title)) in room.requests.iter().take(3).enumerate() {
                            let cy = y + 22.;
                            self.avatar(ui, None, pos2(l + 16., cy), 16., Some(title));
                            let yw = text_w(&p, "Accept", font_w(12.5, Weight::Semi)) + 24.;
                            let yes = Rect::from_min_size(pos2(l + w - yw, cy - 13.), vec2(yw, 26.));
                            let no_c = pos2(yes.min.x - 6. - 13., cy);
                            trunc(&p, pos2(l + 42., cy), no_c.x - 13. - 8. - (l + 42.), title, font_w(13., Weight::Semi), t.tx);
                            let no_hit = press(ui, ("party-no", i), Rect::from_center_size(no_c, vec2(26., 26.)));
                            p.circle_filled(no_c, 13., if no_hit.hovered() { t.s2 } else { t.s3 });
                            icon(ui, icon!("x"), no_c, 12., t.tm);
                            let yes_hit = press(ui, ("party-yes", i), yes);
                            p.rect_filled(yes, rad(13.), if yes_hit.hovered() { t.tx.gamma_multiply(0.88) } else { t.tx });
                            p.text(yes.center(), Align2::CENTER_CENTER, "Accept", font_w(12.5, Weight::Semi), t.s0);
                            if let Some(ok) = [(yes_hit.clicked(), true), (no_hit.clicked(), false)].into_iter().find(|c| c.0).map(|c| c.1) {
                                let id = id.clone();
                                act = Some(Box::new(move |a: &mut App| a.party.answer_join(&id, ok)));
                            }
                            y += 44.;
                        }
                        if room.requests.len() > 3 {
                            p.text(pos2(l + 42., y + 4.), Align2::LEFT_CENTER, format!("+{} more waiting", room.requests.len() - 3), font(12.), t.tm);
                            y += 16.;
                        }
                        y += 4.;
                    }

                    if others == 0 && room.requests.is_empty() {
                        let c = pos2(l + 16., y + 22.);
                        p.circle_stroke(c, 15., egui::Stroke::new(1.5, t.s3));
                        p.text(c, Align2::CENTER_CENTER, "+", font_w(14., Weight::Semi), t.tf);
                        trunc(&p, pos2(l + 42., y + 22.), w - 42., "Waiting for friends…", font(13.), t.tm);
                        y += 44.;
                    }

                    if host {
                        y += 6.;
                        label(&p, &t, pos2(l, y + 6.), w, "Host options");
                        y += 14.;
                        let cy = y + 20.;
                        trunc(&p, pos2(l, cy), w - 56., "Let people in automatically", font_w(13., Weight::Semi), t.tx);
                        let mut on = self.opts.party_auto_accept;
                        if crate::settings::toggle(ui, &t, "party-auto", l + w, cy, &mut on) {
                            act = Some(Box::new(move |a: &mut App| a.opts.party_auto_accept = on));
                        }
                        y += 40.;
                    }
                    y += 10.;
                    if host && self.party.confirm_end {
                        let g = p.layout(format!("This ends the party for {others} other{}.", if others == 1 { "" } else { "s" }), font(12.5), t.tm, w);
                        let gh = g.size().y;
                        p.galley(pos2(l, y), g, t.tm);
                        y += gh + 6.;
                        let half = (w - 6.) / 2.;
                        if btn(ui, &t, "keep", Rect::from_min_size(pos2(l, y), vec2(half, 36.)), "Keep going", false) {
                            act = Some(Box::new(|a: &mut App| a.party.confirm_end = false));
                        }
                        if btn(ui, &t, "end", Rect::from_min_size(pos2(l + half + 6., y), vec2(half, 36.)), "End party", false) {
                            act = Some(Box::new(|a: &mut App| a.party.leave()));
                        }
                    } else if btn(ui, &t, "end", Rect::from_min_size(pos2(l, y), vec2(w, 36.)), if host { "End party" } else { "Leave party" }, false) {
                        act = Some(if host && others > 0 { Box::new(|a: &mut App| a.party.confirm_end = true) } else if host { Box::new(|a: &mut App| a.party.leave()) } else { Box::new(|a: &mut App| a.party.close(Closed::Left)) });
                    }
                    y += 36.;
                }
                // Asked to join: the host's card, with where we stand.
                None if self.party.pending.is_some() => {
                    let (host, pos) = self.party.pending.clone().unwrap_or_default();
                    let card = Rect::from_min_size(pos2(l, y), vec2(w, 72.));
                    p.rect_filled(card, rad(12.), t.s1);
                    self.avatar(ui, None, pos2(card.min.x + 14. + 26., card.center().y), 26., Some(&host));
                    let tx = card.min.x + 14. + 52. + 12.;
                    trunc(&p, pos2(tx, card.center().y - 9.), card.max.x - 12. - tx, &host, font_w(14., Weight::Bold), t.tx);
                    let sub = if pos > 1 { format!("Host · number {pos} in line") } else { "Host · hasn't answered yet".to_string() };
                    trunc(&p, pos2(tx, card.center().y + 10.), card.max.x - 12. - tx, &sub, font(12.), t.tm);
                    y += 72. + 12.;
                    if btn(ui, &t, "cancel", Rect::from_min_size(pos2(l, y), vec2(w, 36.)), "Cancel request", false) {
                        act = Some(Box::new(|a: &mut App| a.party.leave()));
                    }
                    y += 36.;
                }
                // A party that did not happen or has ended: what happened, and what to do next.
                None if self.party.closed.is_some() => {
                    let (glyph, title, body, retry) = match self.party.closed.clone().unwrap_or(Closed::Left) {
                        Closed::Refused(_) => (icon!("x"), "Request declined".to_string(), String::new(), false),
                        Closed::Full => (icon!("group"), format!("This party is full ({MAX} of {MAX})"), String::new(), false),
                        Closed::Unreachable => (icon!("x"), "Can't reach the party server".into(), String::new(), true),
                        Closed::Removed => (icon!("x"), "You were removed".into(), String::new(), false),
                        Closed::Ended => (icon!("party"), "The party has ended".into(), String::new(), false),
                        Closed::Left => (icon!("check"), "You left the party".into(), String::new(), false),
                        Closed::Other(m) => (icon!("x"), "Something went wrong".into(), m, true),
                    };
                    let c = pos2(l + w / 2., y + 24.);
                    p.circle_filled(c, 22., t.s1);
                    icon(ui, glyph, c, 22., t.tm);
                    y += 56.;
                    p.text(pos2(l + w / 2., y), Align2::CENTER_TOP, &title, font_w(15., Weight::Bold), t.tx);
                    y += 24.;
                    if body.is_empty() {
                        y += 6.;
                    } else {
                        let g = p.layout(body, font(13.), t.tm, w);
                        let gh = g.size().y;
                        p.galley(pos2(l + (w - g.size().x) / 2., y), g, t.tm);
                        y += gh + 14.;
                    }
                    if btn(ui, &t, "again", Rect::from_min_size(pos2(l, y), vec2(w, 36.)), if retry { "Try again" } else { "Join another party" }, true) {
                        act = Some(Box::new(|a: &mut App| a.party.closed = None));
                    }
                    y += 36.;
                    if !retry {
                        y += 6.;
                        if btn(ui, &t, "again-start", Rect::from_min_size(pos2(l, y), vec2(w, 36.)), "Start your own", false) {
                            let (n, c, av) = (name.clone(), ctx.clone(), my_pic.clone());
                            act = Some(Box::new(move |a: &mut App| a.party.create(&c, &n, &av)));
                        }
                        y += 36.;
                    }
                }
                // Not in a party: start one or join with a code. While one is being set up, the buttons say so.
                None => {
                    let busy = self.party.busy;
                    let start = Rect::from_min_size(pos2(l, y), vec2(w, 36.));
                    if btn(ui, &t, "start", start, if busy == Some(false) { "Starting…" } else { "Start a party" }, busy.is_none()) && busy.is_none() {
                        let (n, c, av) = (name.clone(), ctx.clone(), my_pic.clone());
                        act = Some(Box::new(move |a: &mut App| a.party.create(&c, &n, &av)));
                    }
                    y += 36. + 12.;
                    let join_label = if busy == Some(true) { "Joining…" } else { "Join" };
                    let jw = text_w(&p, "Joining…", font_w(13., Weight::Semi)) + 28.;
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
                    if (btn(ui, &t, "join", Rect::from_min_size(pos2(field.max.x + 6., y), vec2(jw, 36.)), join_label, false) || enter) && !self.party_code.trim().is_empty() && busy.is_none() {
                        let (n, code, c, av) = (name.clone(), self.party_code.clone(), ctx.clone(), my_pic.clone());
                        act = Some(Box::new(move |a: &mut App| a.party.join(&c, &code, &n, &av)));
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
        std::thread::spawn(move || worker(Context::default(), out_rx, in_tx, c, Default::default(), Default::default()));
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

    /// Two listeners in one room on a local server (`cd desktop/party-server && PORT=8099 go run ./cmd/metroserver`):
    /// `SERAFIN_PARTY_SERVER=ws://127.0.0.1:8099/ws cargo test -p serafin shared_control -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the party server running locally"]
    fn shared_control() {
        let avatar = "https://yt3.ggpht.com/host=s88";
        let host = connect(encode("create_room", Some(&pb::Name { username: "host".into(), avatar_url: avatar.into() })));
        let created = pb::Session::decode(&wait(&host, "room_created")[..]).unwrap();
        let guest = connect(encode("join_room", Some(&pb::Join { room_code: created.room_code.clone(), username: "guest".into(), avatar_url: "https://evil.example/a".into() })));
        let pending = pb::JoinPending::decode(&wait(&guest, "join_pending")[..]).unwrap();
        assert_eq!((pending.host_name.as_str(), pending.position, pending.host_away), ("host", 1, false), "the joiner is told who they wait on");
        let req = pb::User::decode(&wait(&host, "join_request")[..]).unwrap();
        host.out.send(encode("approve_join", Some(&pb::Id { id: req.user_id.clone(), reason: String::new() }))).unwrap();
        let state = pb::Session::decode(&wait(&guest, "join_approved")[..]).unwrap().state.unwrap();
        assert_eq!(state.users.iter().find(|u| u.is_host).unwrap().avatar_url, avatar, "the host's picture reaches the guest");
        assert_eq!(state.users.iter().find(|u| !u.is_host).unwrap().avatar_url, "", "pictures off Google's hosts are dropped");

        let song = |id: &str| pb::TrackInfo { id: id.into(), title: id.into(), artist: "a".into(), duration: 200_000, ..Default::default() };
        let act = |c: &Conn, action: &str, track: &str, info: Option<pb::TrackInfo>, queue: Vec<pb::TrackInfo>| {
            let captured = c.clock.lock().unwrap().now().unwrap_or(0);
            c.out.send(encode("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: track.into(), track_info: info, queue, replace_queue: true, volume: 1., captured_at_server_time: captured, ..Default::default() }))).unwrap();
        };
        let next = |c: &Conn, action: &str| loop {
            let a = pb::PlaybackAction::decode(&wait(c, "sync_playback")[..]).unwrap();
            if a.action == action {
                break a;
            }
        };
        // The guest starts a song, and the host (who did nothing) pauses it.
        act(&guest, "change_track", "", Some(song("one")), vec![song("two")]);
        let change = next(&host, "change_track");
        assert_eq!((change.track_info.unwrap().id, change.queue.len(), change.origin_user_id), ("one".into(), 1, state.users.iter().find(|u| !u.is_host).unwrap().user_id.clone()));
        act(&host, "pause", "one", None, vec![]);
        assert_eq!(next(&guest, "pause").track_id, "one");
        // The guest edits the queue.
        act(&guest, "queue_add", "", Some(song("three")), vec![]);
        assert_eq!(next(&host, "queue_add").queue.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["two", "three"]);
        guest.out.send(encode("leave_room", None)).unwrap();
        assert_eq!(pb::User::decode(&wait(&host, "user_left")[..]).unwrap().username, "guest");
        host.out.send(encode("leave_room", None)).unwrap();
    }

    /// A scripted listener for trying the app against a live room: creates a party, lets everyone in, starts a song when
    /// the first person joins, then seeks, pauses, resumes, queues a song and skips, and logs what others do.
    /// `cargo test -p serafin host_party -- --ignored --nocapture` (SERAFIN_PARTY_SERVER overrides the server)
    #[test]
    #[ignore = "hosts a live room for 15 minutes"]
    fn host_party() {
        let h = connect(encode("create_room", Some(&pb::Name { username: "Claude (test bot)".into(), avatar_url: String::new() })));
        let room = pb::Session::decode(&wait(&h, "room_created")[..]).unwrap();
        println!("PARTY CODE: {}", room.room_code);
        let song = |id: &str, title: &str, artist: &str, secs: i64| pb::TrackInfo { id: id.into(), title: title.into(), artist: artist.into(), duration: secs * 1000, thumbnail: format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"), ..Default::default() };
        let (a, b, c) = (song("dQw4w9WgXcQ", "Never Gonna Give You Up", "Rick Astley", 213), song("fJ9rUzIMcZQ", "Bohemian Rhapsody", "Queen", 354), song("kJQP7kiw5Fk", "Despacito", "Luis Fonsi", 229));
        let t0 = Instant::now();
        let log = |m: String| println!("[{:5.1}s] {m}", t0.elapsed().as_secs_f32());
        let send = |action: &str, track: &str, pos: i64, info: Option<pb::TrackInfo>, queue: Vec<pb::TrackInfo>| {
            let captured = h.clock.lock().unwrap().now().unwrap_or(0);
            h.out.send(encode("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: track.into(), position: pos, track_info: info, queue, replace_queue: action == "change_track", volume: 1., captured_at_server_time: captured, ..Default::default() }))).unwrap();
        };
        let mut cur = a.id.clone();
        let (mut started, mut step, mut beat) = (false, 0, Instant::now());
        while t0.elapsed() < Duration::from_secs(900) {
            if let Ok(In::Msg(k, body)) = h.inbox.recv_timeout(Duration::from_millis(200)) {
                match k.as_str() {
                    "join_request" => {
                        let u = pb::User::decode(&body[..]).unwrap();
                        log(format!("join request from {} — approving", u.username));
                        h.out.send(encode("approve_join", Some(&pb::Id { id: u.user_id, reason: String::new() }))).unwrap();
                    }
                    "sync_playback" => {
                        let p = pb::PlaybackAction::decode(&body[..]).unwrap();
                        let who = if p.origin_user_id == room.user_id { "bot" } else if p.origin_user_id.is_empty() { "server" } else { "someone" };
                        log(format!("{who}: {} track={} pos={}ms queue={:?}", p.action, p.track_id, p.position, p.queue.iter().map(|t| t.title.as_str()).collect::<Vec<_>>()));
                        if p.action == "change_track" {
                            cur = p.track_id.clone();
                        }
                    }
                    "member_sync" => {
                        let m = pb::MemberSync::decode(&body[..]).unwrap();
                        log(format!("member_sync: in_sync={} drift={}ms", m.in_sync, m.drift_ms));
                    }
                    "user_joined" | "user_left" | "user_disconnected" | "user_reconnected" => {
                        let u = pb::User::decode(&body[..]).unwrap();
                        log(format!("{k}: {} avatar={:?}", u.username, u.avatar_url));
                    }
                    _ => log(k.clone()),
                }
                if k == "user_joined" && !started {
                    started = true;
                    send("change_track", &a.id, 0, Some(a.clone()), vec![b.clone()]);
                    log(format!("starting {}", a.title));
                    (beat, step) = (Instant::now(), 0);
                }
            }
            if !started {
                continue;
            }
            // Positions are only approximate: the bot has no audio and just says where the song should be.
            let at = beat.elapsed().as_secs();
            let script = [(30, "seek"), (50, "pause"), (60, "play"), (80, "queue"), (100, "skip")];
            if let Some(&(_, what)) = script.get(step).filter(|(s, _)| at >= *s) {
                step += 1;
                match what {
                    "seek" => send("seek", &cur, 120_000, None, vec![]),
                    "pause" => send("pause", &cur, 140_000, None, vec![]),
                    "play" => send("play", &cur, 140_000, None, vec![]),
                    "queue" => send("queue_add", "", 0, Some(c.clone()), vec![]),
                    _ => send("skip_next", &cur, 0, None, vec![]),
                }
                log(format!("bot sent {what}"));
            }
        }
        h.out.send(encode("leave_room", None)).unwrap();
    }

    /// Joins the party in PARTY_CODE as a listener with no audio and acts on commands written (one per line) to the file
    /// in PARTY_CMDS: `pause`, `play`, `seek <secs>`, `next`, `queue <youtube id>`, `leave`. Logs what the room does.
    /// `PARTY_CODE=ABC123 PARTY_CMDS=/tmp/cmds cargo test -p serafin join_party -- --ignored --nocapture`
    #[test]
    #[ignore = "joins a live room; needs PARTY_CODE and PARTY_CMDS"]
    fn join_party() {
        let code = std::env::var("PARTY_CODE").expect("PARTY_CODE").trim().to_uppercase();
        let cmds = std::env::var("PARTY_CMDS").expect("PARTY_CMDS");
        let g = connect(encode("join_room", Some(&pb::Join { room_code: code, username: "Claude (test bot)".into(), avatar_url: String::new() })));
        let t0 = Instant::now();
        let log = |m: String| println!("[{:5.1}s] {m}", t0.elapsed().as_secs_f32());
        // What the room is doing, as far as the bot has heard.
        let (mut cur, mut playing, mut pos, mut at) = (String::new(), false, 0i64, 0i64);
        let now_pos = |g: &Conn, pos: i64, at: i64, playing: bool| g.clock.lock().unwrap().position_at(pos, at, playing);
        let act = |g: &Conn, action: &str, track: &str, pos: i64, info: Option<pb::TrackInfo>| {
            let captured = g.clock.lock().unwrap().now().unwrap_or(0);
            g.out.send(encode("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: track.into(), position: pos, track_info: info, volume: 1., captured_at_server_time: captured, ..Default::default() }))).unwrap();
        };
        while t0.elapsed() < Duration::from_secs(1200) {
            if let Ok(In::Msg(k, b)) = g.inbox.recv_timeout(Duration::from_millis(200)) {
                match k.as_str() {
                    "join_pending" => {
                        let p = pb::JoinPending::decode(&b[..]).unwrap();
                        log(format!("waiting for {} (number {} in line)", p.host_name, p.position));
                    }
                    "join_approved" => {
                        let s = pb::Session::decode(&b[..]).unwrap().state.unwrap_or_default();
                        log(format!("approved: users={:?} track={:?} playing={} pos={}ms queue={}", s.users.iter().map(|u| (&u.username, u.avatar_url.len())).collect::<Vec<_>>(), s.current_track.as_ref().map(|t| &t.title), s.is_playing, s.position, s.queue.len()));
                        if let Some(t) = &s.current_track {
                            (cur, playing, pos, at) = (t.id.clone(), s.is_playing, s.position, s.last_update);
                            g.out.send(encode("buffer_ready", Some(&pb::Id { id: cur.clone(), reason: String::new() }))).unwrap();
                        }
                    }
                    "sync_playback" => {
                        let a = pb::PlaybackAction::decode(&b[..]).unwrap();
                        log(format!("{}: {} track={} pos={}ms queue={:?}", if a.origin_user_id.is_empty() { "server" } else { "someone" }, a.action, a.track_id, a.position, a.queue.iter().map(|t| t.title.as_str()).collect::<Vec<_>>()));
                        let when = if a.server_time > 0 { a.server_time } else { 0 };
                        match a.action.as_str() {
                            "change_track" => {
                                if let Some(t) = &a.track_info {
                                    (cur, playing, pos, at) = (t.id.clone(), true, 0, when);
                                    g.out.send(encode("buffer_ready", Some(&pb::Id { id: cur.clone(), reason: String::new() }))).unwrap();
                                }
                            }
                            "play" => (playing, pos, at) = (true, a.position, when),
                            "pause" => (playing, pos, at) = (false, a.position, when),
                            "seek" => (pos, at) = (a.position, when),
                            _ => {}
                        }
                    }
                    "sync_state" => {
                        let s = pb::SyncState::decode(&b[..]).unwrap();
                        if let Some(t) = &s.current_track {
                            (cur, playing, pos, at) = (t.id.clone(), s.is_playing, s.position, s.last_update);
                        }
                        log(format!("sync_state: track={:?} playing={} pos={}ms queue={}", s.current_track.map(|t| t.title), s.is_playing, s.position, s.queue.len()));
                    }
                    "member_sync" => {
                        let m = pb::MemberSync::decode(&b[..]).unwrap();
                        log(format!("member_sync: in_sync={} drift={}ms", m.in_sync, m.drift_ms));
                    }
                    "user_joined" | "user_left" | "user_disconnected" | "user_reconnected" | "join_rejected" | "kicked" => {
                        let u = pb::User::decode(&b[..]).unwrap_or_default();
                        log(format!("{k}: {} avatar={}", u.username, !u.avatar_url.is_empty()));
                    }
                    "error" => log(format!("error: {:?}", pb::Error::decode(&b[..]).unwrap_or_default())),
                    _ => log(k.clone()),
                }
            }
            let Ok(text) = std::fs::read_to_string(&cmds) else { continue };
            if text.trim().is_empty() {
                continue;
            }
            let _ = std::fs::write(&cmds, "");
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
                let here = now_pos(&g, pos, at, playing);
                log(format!("bot: {line}"));
                match cmd {
                    "pause" => act(&g, "pause", &cur, here, None),
                    "play" => act(&g, "play", &cur, here, None),
                    "seek" => act(&g, "seek", &cur, arg.parse::<i64>().unwrap_or(0) * 1000, None),
                    "next" => act(&g, "skip_next", &cur, 0, None),
                    "queue" => act(&g, "queue_add", "", 0, Some(pb::TrackInfo { id: arg.into(), title: arg.into(), artist: "bot".into(), duration: 200_000, thumbnail: format!("https://i.ytimg.com/vi/{arg}/hqdefault.jpg"), ..Default::default() })),
                    "leave" => {
                        g.out.send(encode("leave_room", None)).unwrap();
                        std::thread::sleep(Duration::from_millis(500));
                        return;
                    }
                    _ => log("unknown command".into()),
                }
            }
        }
    }

    /// Hosts a room with no audio and acts on commands written (one per line) to the file in PARTY_CMDS: `approve`, `decline`,
    /// `kick`, `song`, `pause`, `play`, `seek <secs>`, `fill` (seven more listeners join, to fill the party), `end`.
    /// Join requests wait for `approve` or `decline`, so the joiner can see the waiting state.
    /// `PARTY_CMDS=/tmp/cmds cargo test -p serafin host_room -- --ignored --nocapture`
    #[test]
    #[ignore = "hosts a live room; needs PARTY_CMDS"]
    fn host_room() {
        let cmds = std::env::var("PARTY_CMDS").expect("PARTY_CMDS");
        let h = connect(encode("create_room", Some(&pb::Name { username: "Claude (test bot)".into(), avatar_url: String::new() })));
        let room = pb::Session::decode(&wait(&h, "room_created")[..]).unwrap();
        println!("PARTY CODE: {}", room.room_code);
        let t0 = Instant::now();
        let log = |m: String| println!("[{:5.1}s] {m}", t0.elapsed().as_secs_f32());
        let (mut asking, mut members): (Vec<(String, String)>, Vec<(String, String)>) = (vec![], vec![]);
        let mut fillers: Vec<Conn> = vec![];
        let (mut cur, mut playing, mut pos, mut at) = (String::new(), false, 0i64, 0i64);
        let rick = pb::TrackInfo { id: "dQw4w9WgXcQ".into(), title: "Never Gonna Give You Up".into(), artist: "Rick Astley".into(), duration: 213_000, thumbnail: "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg".into(), ..Default::default() };
        let act = |action: &str, track: &str, pos: i64, info: Option<pb::TrackInfo>| {
            let captured = h.clock.lock().unwrap().now().unwrap_or(0);
            h.out.send(encode("playback_action", Some(&pb::PlaybackAction { action: action.into(), track_id: track.into(), position: pos, track_info: info, replace_queue: action == "change_track", volume: 1., captured_at_server_time: captured, ..Default::default() }))).unwrap();
        };
        while t0.elapsed() < Duration::from_secs(1500) {
            if let Ok(In::Msg(k, b)) = h.inbox.recv_timeout(Duration::from_millis(200)) {
                match k.as_str() {
                    "join_request" => {
                        let u = pb::User::decode(&b[..]).unwrap();
                        if u.username.starts_with("Filler") {
                            h.out.send(encode("approve_join", Some(&pb::Id { id: u.user_id, reason: String::new() }))).unwrap();
                        } else {
                            log(format!("join request from {} (waiting for approve or decline)", u.username));
                            asking.push((u.user_id, u.username));
                        }
                    }
                    "join_cancelled" => {
                        let u = pb::User::decode(&b[..]).unwrap();
                        asking.retain(|q| q.0 != u.user_id);
                        log(format!("{} gave up waiting", u.username));
                    }
                    "user_joined" | "user_left" | "user_disconnected" | "user_reconnected" => {
                        let u = pb::User::decode(&b[..]).unwrap();
                        if !u.username.starts_with("Filler") {
                            log(format!("{k}: {} avatar={}", u.username, !u.avatar_url.is_empty()));
                        }
                        if k == "user_joined" && !u.username.starts_with("Filler") {
                            members.push((u.user_id.clone(), u.username));
                        }
                        if k == "user_left" {
                            members.retain(|m| m.0 != u.user_id);
                        }
                    }
                    "sync_playback" => {
                        let a = pb::PlaybackAction::decode(&b[..]).unwrap();
                        log(format!("{}: {} track={} pos={}ms queue={:?}", if a.origin_user_id == room.user_id { "bot" } else { "someone" }, a.action, a.track_id, a.position, a.queue.iter().map(|t| t.title.as_str()).collect::<Vec<_>>()));
                        match a.action.as_str() {
                            "change_track" => (cur, playing, pos, at) = (a.track_id.clone(), true, 0, a.server_time),
                            "play" => (playing, pos, at) = (true, a.position, a.server_time),
                            "pause" => (playing, pos, at) = (false, a.position, a.server_time),
                            "seek" => (pos, at) = (a.position, a.server_time),
                            _ => {}
                        }
                    }
                    "member_sync" => {
                        let m = pb::MemberSync::decode(&b[..]).unwrap();
                        log(format!("member_sync: in_sync={} drift={}ms", m.in_sync, m.drift_ms));
                    }
                    "error" => log(format!("error: {:?}", pb::Error::decode(&b[..]).unwrap_or_default())),
                    _ => {}
                }
            }
            for f in &fillers {
                while f.inbox.try_recv().is_ok() {}
            }
            let Ok(text) = std::fs::read_to_string(&cmds) else { continue };
            if text.trim().is_empty() {
                continue;
            }
            let _ = std::fs::write(&cmds, "");
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
                let here = h.clock.lock().unwrap().position_at(pos, at, playing);
                log(format!("bot: {line}"));
                match cmd {
                    "approve" if !asking.is_empty() => {
                        let (id, _) = asking.remove(0);
                        h.out.send(encode("approve_join", Some(&pb::Id { id, reason: String::new() }))).unwrap();
                    }
                    "decline" if !asking.is_empty() => {
                        let (id, _) = asking.remove(0);
                        h.out.send(encode("reject_join", Some(&pb::Id { id, reason: String::new() }))).unwrap();
                    }
                    "kick" if !members.is_empty() => {
                        let (id, _) = members.remove(0);
                        h.out.send(encode("kick_user", Some(&pb::Id { id, reason: String::new() }))).unwrap();
                    }
                    "song" => act("change_track", "", 0, Some(rick.clone())),
                    "pause" => act("pause", &cur, here, None),
                    "play" => act("play", &cur, here, None),
                    "seek" => act("seek", &cur, arg.parse::<i64>().unwrap_or(0) * 1000, None),
                    "fill" => {
                        for i in 1..=7 {
                            fillers.push(connect(encode("join_room", Some(&pb::Join { room_code: room.room_code.clone(), username: format!("Filler {i}"), avatar_url: String::new() }))));
                        }
                    }
                    "end" => {
                        h.out.send(encode("leave_room", None)).unwrap();
                        std::thread::sleep(Duration::from_millis(500));
                        return;
                    }
                    _ => log("nothing to do".into()),
                }
            }
        }
    }

    /// Asks to join the room in PARTY_CODE and gives up, as Cancel does: the host should be told. `PARTY_CODE=ABC123 cargo test -p serafin cancel_join -- --ignored --nocapture`
    #[test]
    #[ignore = "joins a live room; needs PARTY_CODE"]
    fn cancel_join() {
        let code = std::env::var("PARTY_CODE").expect("PARTY_CODE").trim().to_uppercase();
        let g = connect(encode("join_room", Some(&pb::Join { room_code: code, username: "Cancelling bot".into(), avatar_url: String::new() })));
        wait(&g, "join_pending");
        // Dropping the sender ends the worker, which closes the socket.
        drop(g.out);
        std::thread::sleep(Duration::from_secs(2));
    }
}
