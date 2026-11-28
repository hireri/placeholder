//! Jellyfin SyncPlay: one group on a Jellyfin server that listens together. The daemon talks to the server; here a worker
//! long-polls the daemon's event feed and the UI thread applies what arrives once per frame, like the YouTube party does.
//! Positions are milliseconds, `at` times are local epoch milliseconds.

use crate::app::App;
use crate::backend;
use crate::data::Track;
use eframe::egui::Context;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HARD_SYNC_MS: i64 = 2000;
const DRIFT_CHECK: Duration = Duration::from_secs(1);
const LIST_EVERY: Duration = Duration::from_secs(4);

fn epoch_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub participants: Vec<String>,
}

enum Ev {
    Groups(Result<Vec<GroupInfo>, String>),
    Joined(String),
    Failed(String),
    Group(Vec<String>, String),
    Queue { tracks: Vec<Track>, pids: Vec<String>, index: usize, pos: i64, playing: bool },
    Command { cmd: String, pos: i64, at: i64, pid: Option<String> },
    Left,
}

/// Where the group says playback should be.
struct Target {
    id: String,
    playing: bool,
    pos: i64,
    at: i64,
}

/// What the group last heard from us (or what we last took from it): changes to the player beyond this are ours to announce.
#[derive(Clone, Default, PartialEq)]
struct Sent {
    id: String,
    playing: bool,
    seeks: u32,
    queue: Vec<String>,
}

pub struct Group {
    pub name: String,
    pub state: String,
    pub participants: Vec<String>,
}

pub struct SyncPlay {
    tx: Sender<Ev>,
    rx: Receiver<Ev>,
    stop: Arc<AtomicBool>,
    cmds: Option<Sender<Value>>,
    /// The Jellyfin source the group lives on.
    pub src: String,
    pub group: Option<Group>,
    pub groups: Option<Result<Vec<GroupInfo>, String>>,
    listed: Option<Instant>,
    pub busy: bool,
    target: Option<Target>,
    fresh: bool,
    checked: Option<Instant>,
    sent: Option<Sent>,
    ready: Option<(String, bool)>,
    /// The group's queue as (song id, PlaylistItemId): Jellyfin wants the latter in `ready`, `buffering` and `next`.
    items: Vec<(String, String)>,
}

impl Default for SyncPlay {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, stop: Default::default(), cmds: None, src: String::new(), group: None, groups: None, listed: None, busy: false, target: None, fresh: false, checked: None, sent: None, ready: None, items: vec![] }
    }
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The queued songs in order, from one daemon lookup; ids it cannot find become bare tracks that play but show only their id.
fn resolve(src: &str, ids: &[String]) -> Vec<Track> {
    let found: Vec<Track> = ids.chunks(100).flat_map(|c| backend::get::<Vec<Track>>(&format!("/music/syncplay/{}/tracks?ids={}", backend::enc(src), backend::enc(&c.join(",")))).unwrap_or_default()).collect();
    ids.iter()
        .map(|i| found.iter().find(|t| &t.id == i).cloned().unwrap_or_else(|| Track { id: i.clone(), source: i.split(':').next().unwrap_or_default().into(), title: i.clone(), ..Default::default() }))
        .collect()
}

fn parse(e: &Value, src: &str) -> Option<Ev> {
    let n = |k: &str| e[k].as_i64().unwrap_or(0);
    Some(match e["type"].as_str()? {
        "group" => Ev::Group(strs(&e["participants"]), e["state"].as_str().unwrap_or_default().into()),
        "queue" => Ev::Queue { tracks: resolve(src, &strs(&e["ids"])), pids: strs(&e["playlistItemIds"]), index: n("index") as usize, pos: n("positionMs"), playing: e["playing"].as_bool().unwrap_or(false) },
        "command" => Ev::Command { cmd: e["command"].as_str().unwrap_or_default().into(), pos: n("positionMs"), at: n("atMs"), pid: e["playlistItemId"].as_str().map(String::from) },
        "left" => Ev::Left,
        _ => return None,
    })
}

fn poll(ctx: Context, src: String, stop: Arc<AtomicBool>, tx: Sender<Ev>) {
    let (mut seq, mut fails) = (0u64, 0);
    while !stop.load(Ordering::Relaxed) {
        let r = backend::get::<Value>(&format!("/music/syncplay/{}/events?after={seq}&wait=25", backend::enc(&src)));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match r {
            Ok(v) => {
                fails = 0;
                seq = v["seq"].as_u64().unwrap_or(seq);
                for e in v["events"].as_array().into_iter().flatten() {
                    if let Some(ev) = parse(e, &src) {
                        let _ = tx.send(ev);
                    }
                }
            }
            Err(e) => {
                fails += 1;
                if fails >= 5 {
                    let _ = tx.send(Ev::Failed(e));
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        ctx.request_repaint();
    }
}

impl SyncPlay {
    fn path(&self, tail: &str) -> String {
        format!("/music/syncplay/{}/{tail}", backend::enc(&self.src))
    }

    /// Asks for the groups on the server.
    pub fn refresh(&mut self, ctx: &Context) {
        self.listed = Some(Instant::now());
        let (path, tx, ctx) = (self.path("groups"), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Ev::Groups(backend::get(&path)));
            ctx.request_repaint();
        });
    }

    pub fn create(&mut self, ctx: &Context, name: String) {
        let path = format!("{}?name={}", self.path("new"), backend::enc(&name));
        self.busy = true;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(match backend::post_json::<_, Value>(&path, &json!({})) {
                Ok(_) => Ev::Joined(name),
                Err(e) => Ev::Failed(e),
            });
            ctx.request_repaint();
        });
    }

    pub fn join(&mut self, ctx: &Context, g: &GroupInfo) {
        let (path, tx, ctx, name) = (self.path(&format!("join/{}", backend::enc(&g.id))), self.tx.clone(), ctx.clone(), g.name.clone());
        self.busy = true;
        std::thread::spawn(move || {
            let _ = tx.send(match backend::post(&path) {
                Ok(()) => Ev::Joined(name),
                Err(e) => Ev::Failed(e),
            });
            ctx.request_repaint();
        });
    }

    fn begin(&mut self, ctx: &Context, name: String) {
        self.reset();
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let (ctx, src, tx) = (ctx.clone(), self.src.clone(), self.tx.clone());
        std::thread::spawn(move || poll(ctx, src, stop, tx));
        let (ctx_tx, ctx_rx) = channel::<Value>();
        let path = self.path("cmd");
        // One thread sends the commands, so they reach the daemon in the order they were made.
        std::thread::spawn(move || {
            while let Ok(body) = ctx_rx.recv() {
                if let Err(e) = backend::send_json_ok("POST", &path, &body) {
                    crate::toast::error(format!("SyncPlay: {e}"));
                }
            }
        });
        self.cmds = Some(ctx_tx);
        self.group = Some(Group { name, state: String::new(), participants: vec![] });
        self.busy = false;
    }

    /// Forgets the group and stops listening for its events.
    fn reset(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        (self.group, self.cmds, self.target, self.sent, self.ready, self.checked) = (None, None, None, None, None, None);
        self.items.clear();
    }

    pub fn leave(&mut self) {
        if self.group.is_some() {
            let path = self.path("leave");
            std::thread::spawn(move || {
                let _ = backend::post(&path);
            });
        }
        self.reset();
    }

    /// Leaves before the app exits, so the server does not keep a ghost listener.
    pub fn close(&mut self) {
        if self.group.is_some() {
            let _ = backend::post(&self.path("leave"));
        }
        self.reset();
    }

    fn cmd(&self, body: Value) {
        if let Some(c) = &self.cmds {
            let _ = c.send(body);
        }
    }

    /// Sends `body` with the PlaylistItemId of song `id` when the group's queue has it.
    fn cmd_for(&self, mut body: Value, id: &str) {
        if let Some((_, p)) = self.items.iter().find(|(s, _)| s == id) {
            body["itemId"] = json!(p);
        }
        self.cmd(body);
    }

    fn ours(&self, id: &str) -> bool {
        id.strip_prefix(self.src.as_str()).is_some_and(|r| r.starts_with(':'))
    }
}

impl App {
    /// Drains the group's events and keeps the player and the group in step; called every frame.
    pub fn syncplay_tick(&mut self, ctx: &Context) {
        while let Ok(ev) = self.sp.rx.try_recv() {
            self.sp_event(ctx, ev);
        }
        // The party popover lists the server's groups while it is open and we are in neither kind of session.
        if self.party_open.is_some() && self.sp.group.is_none() && !self.sp.busy && self.party.idle() {
            if let Some(src) = self.sources.with_cap("syncplay").into_iter().next() {
                self.sp.src = src;
                if self.sp.listed.is_none_or(|l| l.elapsed() >= LIST_EVERY) {
                    self.sp.refresh(ctx);
                }
                ctx.request_repaint_after(LIST_EVERY);
            }
        }
        if self.sp.group.is_none() {
            return;
        }
        if !self.sources.can(&self.sp.src, "syncplay") {
            crate::toast::info("SyncPlay ended: its source is gone");
            return self.sp.leave();
        }
        if std::mem::take(&mut self.pl.endless) {
            crate::toast::info("Endless mode is off while you are in a SyncPlay group");
        }
        self.sp_apply(ctx);
        self.sp_announce();
    }

    fn sp_event(&mut self, ctx: &Context, ev: Ev) {
        match ev {
            Ev::Groups(r) => self.sp.groups = Some(r),
            Ev::Joined(name) => self.sp.begin(ctx, name),
            Ev::Failed(e) => {
                crate::toast::error(format!("SyncPlay: {e}"));
                self.sp.busy = false;
                self.sp.leave();
            }
            Ev::Group(participants, state) => {
                if let Some(g) = &mut self.sp.group {
                    (g.participants, g.state) = (participants, state);
                }
            }
            Ev::Left => {
                crate::toast::info("SyncPlay group ended");
                self.sp.reset();
            }
            Ev::Queue { tracks, pids, index, pos, playing } if self.sp.group.is_some() => {
                self.sp.items = tracks.iter().map(|t| t.id.clone()).zip(pids).collect();
                self.sp_follow(ctx, tracks, index, pos, playing)
            }
            Ev::Command { cmd, pos, at, pid } if self.sp.group.is_some() => {
                let Some(id) = self.pl.song().map(|s| s.vid().to_string()).filter(|id| self.sp.ours(id)) else { return };
                let mine = || self.sp.items.iter().filter(|(s, _)| *s == id).map(|(_, p)| p);
                if pid.as_ref().is_some_and(|p| mine().next().is_some() && mine().all(|m| m != p)) {
                    return;
                }
                let playing = match cmd.as_str() {
                    "unpause" => true,
                    "pause" | "stop" => false,
                    _ => self.sp.target.as_ref().map_or(self.pl.playing, |t| t.playing),
                };
                let at = if at > 0 { at } else { epoch_ms() };
                (self.sp.target, self.sp.fresh) = (Some(Target { id, playing, pos, at }), true);
            }
            _ => {}
        }
    }

    fn sp_upcoming(&self) -> Vec<String> {
        (1..).map_while(|n| self.pl.upcoming(n)).map(|t| t.id).filter(|id| self.sp.ours(id)).collect()
    }

    fn sp_local(&self) -> Sent {
        Sent {
            id: self.pl.song().map(|s| s.vid().to_string()).filter(|id| self.sp.ours(id)).unwrap_or_default(),
            playing: self.pl.playing,
            seeks: self.pl.seeks,
            queue: self.sp_upcoming(),
        }
    }

    /// The group's queue replaces ours: load its current song (paused) unless it is already playing, then aim for the group's playback.
    fn sp_follow(&mut self, ctx: &Context, tracks: Vec<Track>, index: usize, pos: i64, playing: bool) {
        let Some(id) = tracks.get(index).map(|t| t.id.clone()) else { return };
        if self.pl.song().map(|s| s.vid()) != Some(id.as_str()) {
            self.pl.play_tracks(ctx, &tracks, index, false);
            self.pl.playing = false;
            self.sp.ready = None;
        } else if tracks[index + 1..].iter().map(|t| &t.id).ne(self.sp_upcoming().iter()) {
            self.pl.set_upcoming(&tracks[index + 1..]);
        }
        self.sp.sent = Some(self.sp_local());
        (self.sp.target, self.sp.fresh) = (Some(Target { id, playing, pos, at: epoch_ms() }), true);
    }

    /// Brings the player to where the group is: exactly on a pause, jump or new song, and only past `HARD_SYNC_MS` of drift while playing.
    fn sp_apply(&mut self, ctx: &Context) {
        let Some(id) = self.pl.song().map(|s| s.vid().to_string()).filter(|id| self.sp.ours(id)) else { return };
        if !self.pl.loaded() {
            return;
        }
        let ready = !self.pl.buffering();
        if self.sp.ready.as_ref() != Some(&(id.clone(), ready)) {
            let pos = (self.pl.pos * 1000.) as i64;
            self.sp.cmd_for(json!({"type": if ready { "ready" } else { "buffering" }, "positionMs": pos, "playing": self.pl.playing}), &id);
            self.sp.ready = Some((id.clone(), ready));
        }
        let sp = &mut self.sp;
        let Some(t) = sp.target.as_ref().filter(|t| t.id == id) else { return };
        let steady = t.playing && self.pl.playing;
        if !(sp.fresh || (steady && sp.checked.is_none_or(|c| c.elapsed() >= DRIFT_CHECK))) {
            return;
        }
        let now = epoch_ms();
        if now < t.at {
            return ctx.request_repaint_after(Duration::from_millis((t.at - now) as u64));
        }
        (sp.fresh, sp.checked) = (false, Some(Instant::now()));
        let want = if t.playing { t.pos + now - t.at } else { t.pos };
        let (playing, have) = (t.playing, (self.pl.pos * 1000.) as i64);
        if !steady || (want - have).abs() > HARD_SYNC_MS {
            self.pl.seek(want as f32 / 1000.);
        }
        if playing != self.pl.playing {
            self.pl.toggle(ctx);
        }
        if let Some(s) = &mut sp.sent {
            (s.playing, s.seeks) = (self.pl.playing, self.pl.seeks);
        }
    }

    /// Tells the group what changed in our player: another song, play or pause, a seek, or the queue.
    fn sp_announce(&mut self) {
        let now = self.sp_local();
        let Some(last) = self.sp.sent.clone() else { return self.sp.sent = Some(now) };
        if now == last {
            return;
        }
        let pos = (self.pl.pos * 1000.) as i64;
        let ids = || std::iter::once(now.id.clone()).chain(now.queue.clone()).collect::<Vec<_>>();
        if now.id != last.id {
            if !now.id.is_empty() {
                // ponytail: no "prev" detection, going back sends the queue from the song we landed on.
                if last.queue.first() == Some(&now.id) {
                    self.sp.cmd_for(json!({"type": "next"}), &last.id);
                } else {
                    self.sp.cmd(json!({"type": "queue", "ids": ids(), "index": 0, "positionMs": 0, "playing": now.playing}));
                }
            }
        } else if !now.id.is_empty() {
            if now.playing != last.playing {
                self.sp.cmd_for(json!({"type": if now.playing { "unpause" } else { "pause" }, "positionMs": pos}), &now.id);
            }
            if now.seeks != last.seeks {
                self.sp.cmd_for(json!({"type": "seek", "positionMs": pos}), &now.id);
            }
            if now.queue != last.queue {
                self.sp.cmd(json!({"type": "queue", "ids": ids(), "index": 0, "positionMs": pos, "playing": now.playing}));
            }
        }
        self.sp.sent = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_event_keeps_playlist_item_ids() {
        let e = json!({"type": "queue", "ids": [], "playlistItemIds": ["g1", "g2"], "index": 1, "positionMs": 5, "playing": true});
        let Some(Ev::Queue { pids, index, pos, playing, .. }) = parse(&e, "jf") else { panic!() };
        assert_eq!((pids, index, pos, playing), (vec!["g1".to_string(), "g2".into()], 1, 5, true));
    }
}
