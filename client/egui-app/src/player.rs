//! Right sidebar: blurred-cover backdrop, cover/lyrics/queue switcher and transport.
//! Laid out at a fixed 308px from the slot's left edge, so closing just slides it out behind the clip.

use crate::app::{App, PlayerView};
use crate::audio;
use crate::backend::{self, Lyrics};
use crate::data::Track;
use crate::gfx::*;
use crate::menu::Target;
use crate::theme::{mix, oklch, Theme};
use eframe::egui::{self, lerp, pos2, vec2, Align2, Color32, CornerRadius, Rect, Sense};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Song {
    /// Track id as the daemon reports it (Spotify tracks carry a Spotify id here).
    id: String,
    /// YouTube id the stream and lyrics come from; empty for a Spotify song until its first search.
    vid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    dur: u32,
    pub thumbnail: Option<String>,
    pub artist_id: Option<String>,
    pub album_id: Option<String>,
    source: String,
    /// Appended by endless mode rather than queued by the user.
    auto: bool,
}

/// Seconds the queue takes to slide after a skip.
const QSLIDE: f64 = 0.45;

/// Top of queue row `j` below the "NOW PLAYING" slot while `c` plays: rows before it stack up out of sight, the first upcoming
/// row sits 80 down (room for its heading), and endless mode's own heading opens a gap before its first row.
fn qpos(queue: &[Song], j: usize, c: usize) -> f32 {
    if j <= c {
        return -((c - j) as f32) * 54.;
    }
    let gap = (c + 1..queue.len()).find(|&i| queue[i].auto).is_some_and(|a| a > c + 1 && j >= a);
    80. + (j - c - 1) as f32 * 54. + if gap { 26. } else { 0. }
}

fn song_of(t: &Track) -> Song {
    let vid = if t.source == "spotify" { String::new() } else { t.id.clone() };
    Song {
        id: t.id.clone(),
        vid,
        title: t.title.clone(),
        artist: t.artists.join(", "),
        album: t.album.clone().unwrap_or_default(),
        dur: t.duration_sec.unwrap_or(0),
        thumbnail: t.thumbnail.clone(),
        artist_id: t.artist_id.clone(),
        album_id: t.album_id.clone(),
        source: t.source.clone(),
        auto: false,
    }
}

impl Song {
    /// Back to the track it came from, for menus and the song page.
    pub fn track(&self) -> Track {
        Track {
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.artist.split(", ").filter(|a| !a.is_empty()).map(String::from).collect(),
            album: (!self.album.is_empty()).then(|| self.album.clone()),
            duration_sec: (self.dur > 0).then_some(self.dur),
            thumbnail: self.thumbnail.clone(),
            source: self.source.clone(),
            artist_id: self.artist_id.clone(),
            album_id: self.album_id.clone(),
            ..Default::default()
        }
    }
}

enum Lyr {
    Idle,
    Loading(Receiver<Result<Option<Lyrics>, String>>),
    Done(Option<Lyrics>),
    Failed(String),
}

/// (queue index, request counter, resolved YouTube id and stream) from a worker.
type Resolved = (usize, u64, Result<(String, audio::StreamInfo), String>);

/// Player state. `engine` decodes and outputs audio; the queue, transport and lyrics live here.
pub struct Player {
    pub engine: audio::Engine,
    /// Codec and bitrate of the loaded stream, e.g. ("OPUS", 160) kbps.
    pub quality: Option<(String, u32)>,
    queue: Vec<Song>,
    cur: usize,
    pub playing: bool,
    /// Playhead in seconds. Read from the engine while a stream is loaded; while resolving, where playback will start.
    pub pos: f32,
    /// True when the engine holds the current song's stream.
    loaded: bool,
    /// The scrubber is being dragged, so the engine position is not read back over the pointer.
    pub scrub: bool,
    /// Bumped by every seek, so the video reopens at the new spot.
    pub seeks: u32,
    /// Stream lookup in flight for the current song.
    pending: Option<Receiver<Resolved>>,
    req: u64,
    /// The current song has already been re-resolved once after an engine error.
    retried: bool,
    /// Last stream or like failure, shown on the player's muted line.
    pub error: Option<String>,
    pub shuffle: bool,
    pub repeat: bool,
    /// Endless mode: when the queue is about to run out, YouTube Music's radio for its last song is appended.
    pub endless: bool,
    /// Queue row being dragged by its handle (queue index).
    qdrag: Option<usize>,
    /// Bumped on every reorder so the rows' slide animations restart from their new places instead of replaying.
    qgen: u32,
    /// Where on the dragged row the pointer took hold (px from its top), so it follows the pointer from there.
    qgrab: f32,
    /// The song that was playing before the last jump (skip, back, or a click in the queue) and when, so the queue can slide.
    qjump: Option<(usize, f64)>,
    /// Song just dropped by hand and when, for a fading highlight.
    moved: Option<(String, f64)>,
    /// Radio lookup in flight for a seed song, and the last seed asked about (so an empty or failed one is not retried in a loop).
    more: Option<Receiver<Result<Vec<Track>, String>>>,
    seeded: String,
    /// The current song's like as the star shows it; not read back from YouTube, so it starts unfilled.
    pub liked: bool,
    /// Like request in flight: the value it asked for, and the daemon's answer.
    like: Option<(bool, Track, Receiver<Result<(), String>>)>,
    /// A like change for `App` to mirror into the cached Liked songs page (applied the frame it happens).
    pub like_event: Option<(Track, bool)>,
    scroll: f32,
    lyr_anim: LyrAnim,
    card_scroll: f32,
    lyr: Lyr,
    shown: (PlayerView, f64),
    /// Stream of the next song, looked up while this one plays: (song id, lookup).
    ahead: Option<(String, Receiver<Result<(String, audio::StreamInfo), String>>)>,
    /// The upcoming song whose stream the engine holds to continue into, and what to show once it is playing.
    queued: Option<Queued>,
}

fn clock(s: f32) -> String {
    format!("{}:{:02}", s as u32 / 60, s as u32 % 60)
}

/// Fisher-Yates driven by a time-seeded xorshift64; no RNG crate needed for a queue order.
struct Queued {
    id: String,
    vid: String,
    quality: (String, u32),
    dur: u32,
}

fn shuffle_in_place(songs: &mut [Song]) {
    let mut x = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
    for i in (1..songs.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        songs.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

/// Blocking: the stream for a song. A Spotify song has no YouTube id yet, so the first search hit stands in for it.
fn stream(vid: &str, title: &str, artist: &str) -> Result<(String, audio::StreamInfo), String> {
    let vid = if vid.is_empty() {
        let hits: Vec<Track> = backend::get(&format!("/youtube/search?q={}", backend::enc(&format!("{title} {artist}"))))?;
        hits.into_iter().next().map(|t| t.id).ok_or("no YouTube match")?
    } else {
        vid.to_string()
    };
    let info = backend::get(&format!("/stream/{vid}"))?;
    Ok((vid, info))
}

impl Player {
    pub fn new(engine: audio::Engine) -> Self {
        Player { engine, quality: None, queue: vec![], cur: 0, playing: false, pos: 0., loaded: false, scrub: false, seeks: 0, pending: None, req: 0, retried: false, error: None, shuffle: false, repeat: false, endless: false, qdrag: None, qgen: 0, qgrab: 26., qjump: None, moved: None, more: None, seeded: String::new(), liked: false, like: None, like_event: None, scroll: 0., lyr_anim: LyrAnim::default(), card_scroll: 0., lyr: Lyr::Idle, shown: (PlayerView::Art, f64::NEG_INFINITY), ahead: None, queued: None }
    }

    /// Drops every stored egui timestamp. A new window's `Context` restarts its clock near zero, so times taken from the
    /// old one would read as in the future and freeze the animations they drive (the queue body stuck invisible).
    pub fn reset_clock(&mut self) {
        self.shown.1 = f64::NEG_INFINITY;
        self.qjump = None;
        self.moved = None;
        self.lyr_anim = LyrAnim::default();
    }

    pub fn song(&self) -> Option<&Song> {
        self.queue.get(self.cur)
    }

    pub fn next_song(&self) -> Option<&Song> {
        self.queue.get(self.cur + 1)
    }

    pub fn track_at(&self, i: usize) -> Option<Track> {
        self.queue.get(i).map(Song::track)
    }

    /// Adds `tracks` right after the current song (`next`) or at the end of the queued songs; with an empty queue they start playing.
    pub fn enqueue(&mut self, ctx: &egui::Context, tracks: &[Track], next: bool) {
        if self.queue.is_empty() {
            return self.play_tracks(ctx, tracks, 0, false);
        }
        // "Add to queue" goes ahead of endless mode's songs, which stay last.
        let at = if next { self.cur + 1 } else { (self.cur + 1..self.queue.len()).find(|&i| self.queue[i].auto).unwrap_or(self.queue.len()) };
        self.queue.splice(at..at, tracks.iter().map(song_of));
    }

    /// Drops queue item `i`; the current song is not removable.
    pub fn remove_at(&mut self, i: usize) {
        if i != self.cur && i < self.queue.len() {
            self.qjump = None;
            self.queue.remove(i);
            if i < self.cur {
                self.cur -= 1;
            }
        }
    }

    /// Moves upcoming song `from` to index `to`. A song placed by hand counts as queued, so it stays ahead of endless mode's.
    pub fn move_to(&mut self, from: usize, to: usize) {
        let n = self.queue.len();
        if from != to && from > self.cur && to > self.cur && from < n && to < n {
            let mut s = self.queue.remove(from);
            s.auto = false;
            self.queue.insert(to, s);
            self.queue[self.cur + 1..].sort_by_key(|s| s.auto);
            self.qgen += 1;
        }
    }

    /// Drops every song after the current one.
    pub fn clear_upcoming(&mut self) {
        self.queue.truncate(self.cur + 1);
    }

    /// Moves queue item `i` to play right after the current song.
    pub fn move_next(&mut self, i: usize) {
        if i > self.cur + 1 && i < self.queue.len() {
            let s = self.queue.remove(i);
            self.queue.insert(self.cur + 1, s);
        }
    }

    pub fn play_index(&mut self, ctx: &egui::Context, i: usize) {
        if i < self.queue.len() {
            self.play_at(ctx, i, 0.);
        }
    }

    /// Length in seconds: the engine's once a stream is loaded, else the track's own.
    pub fn dur(&self) -> f32 {
        let meta = self.song().map_or(0, |s| s.dur) as f32;
        if self.loaded { self.engine.duration().filter(|d| *d > 0.).unwrap_or(meta) } else { meta }
    }

    pub fn frac(&self) -> f32 {
        let d = self.dur();
        if d > 0. { (self.pos / d).clamp(0., 1.) } else { 0. }
    }

    /// Whether `t` is the song loaded now.
    pub fn is_current(&self, t: &Track) -> bool {
        !t.id.is_empty() && self.song().is_some_and(|s| s.id == t.id)
    }

    /// Replaces the queue with `tracks` and starts it at `start`; `shuffle` shuffles it and starts from the top.
    pub fn play_tracks(&mut self, ctx: &egui::Context, tracks: &[Track], start: usize, shuffle: bool) {
        let mut songs: Vec<Song> = tracks.iter().map(song_of).collect();
        if songs.is_empty() {
            return;
        }
        let start = if shuffle {
            shuffle_in_place(&mut songs);
            0
        } else {
            start
        };
        self.queue = songs;
        self.play_at(ctx, start, 0.);
        self.qjump = None;
    }

    /// Plays queue item `idx` from `at` seconds. The stream is looked up on a worker; `tick` loads it when it arrives.
    fn play_at(&mut self, ctx: &egui::Context, idx: usize, at: f32) {
        self.enter(ctx, idx);
        self.resolve(ctx, at);
    }

    /// Makes queue item `idx` the current song and resets what belongs to the previous one.
    fn enter(&mut self, ctx: &egui::Context, idx: usize) {
        if self.queue.get(idx).map(|s| &s.id) != self.song().map(|s| &s.id) {
            self.liked = false;
            self.like = None;
        }
        self.qjump = Some((self.cur, ctx.input(|i| i.time)));
        self.cur = idx;
        self.scroll = 0.;
        self.lyr_anim = LyrAnim::default();
        self.lyr = Lyr::Idle;
        self.retried = false;
        self.error = None;
    }

    fn resolve(&mut self, ctx: &egui::Context, at: f32) {
        let Some(s) = self.song() else { return };
        let (idx, id, vid, title, artist) = (self.cur, s.id.clone(), s.vid.clone(), s.title.clone(), s.artist.clone());
        let ahead = match &self.ahead {
            Some((a, _)) if *a == id => self.ahead.take().map(|(_, rx)| rx),
            _ => None,
        };
        self.req += 1;
        let req = self.req;
        self.queued = None;
        self.engine.stop();
        self.loaded = false;
        self.playing = true;
        self.pos = at;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // A failed look-ahead gets a fresh try.
            let r = ahead.and_then(|rx| rx.recv().ok()).filter(Result::is_ok).unwrap_or_else(|| stream(&vid, &title, &artist));
            let _ = tx.send((idx, req, r));
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    /// Starts looking up the next song's stream so skipping to it does not wait on the daemon.
    fn look_ahead(&mut self) {
        let Some(s) = self.next_song() else { return };
        if self.ahead.as_ref().is_some_and(|(id, _)| *id == s.id) {
            return;
        }
        let (id, vid, title, artist) = (s.id.clone(), s.vid.clone(), s.title.clone(), s.artist.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(stream(&vid, &title, &artist));
        });
        self.ahead = Some((id, rx));
    }

    /// Hands the engine the next song's stream once the look-ahead has it, so playback continues into it without a gap,
    /// and takes it back when the queue (or repeat) no longer agrees.
    fn chain_next(&mut self) {
        let want = self.next_song().filter(|_| !self.repeat).map(|s| s.id.clone());
        if self.queued.as_ref().map(|q| &q.id) != want.as_ref() && self.queued.take().is_some() {
            self.engine.clear_next();
        }
        let Some(id) = want.filter(|_| self.queued.is_none()) else { return };
        let Some(res) = self.ahead.as_ref().filter(|(a, _)| *a == id).and_then(|(_, rx)| rx.try_recv().ok()) else { return };
        if let Ok((vid, info)) = &res {
            self.engine.queue_next(info.clone());
            self.queued = Some(Queued { id: id.clone(), vid: vid.clone(), quality: (codec_name(&info.mime_type), info.bitrate.map_or(0, |b| b / 1000)), dur: info.duration_sec.unwrap_or(0) });
        }
        // Put the result back for `resolve`, in case the song is skipped to by hand before it plays on its own.
        let (tx, rx) = channel();
        let _ = tx.send(res);
        self.ahead = Some((id, rx));
    }

    pub fn next(&mut self, ctx: &egui::Context) {
        if self.cur + 1 < self.queue.len() {
            self.play_at(ctx, self.cur + 1, 0.);
        } else {
            self.halt();
        }
    }

    pub fn prev(&mut self, ctx: &egui::Context) {
        if self.pos > 3. || self.cur == 0 {
            self.seek(0.);
        } else {
            self.play_at(ctx, self.cur - 1, 0.);
        }
    }

    /// Stops at the end of the queue.
    fn halt(&mut self) {
        self.engine.stop();
        self.pending = None;
        self.loaded = false;
        self.playing = false;
        self.pos = 0.;
    }

    fn fail(&mut self, e: String) {
        self.playing = false;
        self.loaded = false;
        crate::toast::error(format!("Can't play this track: {e}\nDetails: {}", backend::log_path().display()));
        self.error = Some(e);
    }

    /// Where playback actually is: while scrubbing, `pos` follows the pointer instead.
    /// Playback is wanted but no sound is coming: the stream is resolving or the output ran dry.
    pub fn buffering(&self) -> bool {
        self.playing && (self.pending.is_some() || (self.loaded && self.engine.starved()))
    }

    pub fn heard(&self) -> f32 {
        if self.scrub && self.loaded { self.engine.position() } else { self.pos }
    }

    /// Moves the playhead. Without a loaded stream the next load starts from here.
    pub fn seek(&mut self, secs: f32) {
        self.pos = secs;
        self.seeks = self.seeks.wrapping_add(1);
        if self.loaded {
            self.engine.seek(secs);
        }
    }

    /// Turning shuffle on reorders the songs after the current one; the current song keeps its place.
    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle && self.cur + 1 < self.queue.len() {
            shuffle_in_place(&mut self.queue[self.cur + 1..]);
        }
    }

    pub fn toggle_repeat(&mut self) {
        self.repeat = !self.repeat;
    }

    pub fn toggle_endless(&mut self) {
        self.endless = !self.endless;
    }

    /// Endless mode: once the last song is the one playing, asks the daemon for its radio and appends what is new.
    fn extend_queue(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.more {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => return,
                Ok(Ok(tracks)) => {
                    let new: Vec<Song> = tracks.iter().filter(|t| !self.queue.iter().any(|s| s.id == t.id)).map(|t| Song { auto: true, ..song_of(t) }).collect();
                    self.queue.extend(new);
                }
                Ok(Err(e)) => crate::toast::error(format!("Can't load more songs: {e}")),
                Err(TryRecvError::Disconnected) => {}
            }
            self.more = None;
        }
        // Fetch while the last song (or the one before it) plays, so skipping or a slow lookup doesn't end the queue.
        let near_end = self.cur + 2 >= self.queue.len();
        if !near_end {
            self.seeded.clear();
        }
        let Some(vid) = self.queue.last().map(|s| s.vid.clone()).filter(|v| !v.is_empty() && near_end && self.endless && !self.repeat) else { return };
        if self.seeded == vid {
            return;
        }
        self.seeded.clone_from(&vid);
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(backend::get(&format!("/youtube/radio/{vid}")));
            ctx.request_repaint();
        });
        self.more = Some(rx);
    }

    /// Likes or unlikes the current song on YouTube. The star flips at once; `tick` reverts it if the daemon refuses.
    pub fn set_like(&mut self, ctx: &egui::Context) {
        let Some(s) = self.song().filter(|s| !s.vid.is_empty()) else { return };
        let vid = s.vid.clone();
        let track = Track {
            id: vid.clone(),
            title: s.title.clone(),
            artists: s.artist.split(", ").filter(|a| !a.is_empty()).map(String::from).collect(),
            album: Some(s.album.clone()).filter(|a| !a.is_empty()),
            duration_sec: Some(s.dur),
            thumbnail: s.thumbnail.clone(),
            source: "youtube".into(),
            artist_id: s.artist_id.clone(),
            album_id: s.album_id.clone(),
            ..Default::default()
        };
        let on = !self.liked;
        self.liked = on;
        crate::toast::info(if on { "Added to liked songs" } else { "Removed from liked songs" });
        self.like_event = Some((track.clone(), on));
        ctx.request_repaint();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(backend::post(&format!("/youtube/like/{vid}?on={on}")));
            ctx.request_repaint();
        });
        self.like = Some((on, track, rx));
    }

    pub fn toggle(&mut self, ctx: &egui::Context) {
        if self.playing {
            self.playing = false;
            self.engine.pause();
        } else if self.loaded {
            self.playing = true;
            self.engine.play();
        } else if self.pending.is_some() {
            self.playing = true;
        } else if self.song().is_some() {
            self.play_at(ctx, self.cur, self.pos);
        }
    }

    /// Applies a finished stream lookup, handles engine events and refreshes the playhead; called every frame (also while the sidebar is collapsed).
    pub fn tick(&mut self, ctx: &egui::Context) {
        for ev in self.engine.poll_events() {
            // Events from a stream that was replaced or stopped are stale.
            if !self.loaded {
                break;
            }
            match ev {
                audio::Event::Advanced => match self.queued.take().and_then(|q| Some((self.queue.iter().position(|s| s.id == q.id)?, q))) {
                    Some((i, q)) => {
                        self.enter(ctx, i);
                        self.quality = Some(q.quality);
                        if let Some(s) = self.queue.get_mut(i) {
                            if s.dur == 0 {
                                s.dur = q.dur;
                            }
                            s.vid = q.vid;
                        }
                    }
                    // The song left the queue after the engine had already moved on: reload what should play.
                    None => self.next(ctx),
                },
                audio::Event::Ended if self.repeat => self.resolve(ctx, 0.),
                audio::Event::Ended => self.next(ctx),
                // Stream URLs expire mid-track; a fresh lookup of the same song usually fixes it.
                audio::Event::Error(_) if !self.retried => {
                    self.retried = true;
                    self.resolve(ctx, self.pos);
                }
                audio::Event::Error(e) => self.fail(e),
            }
        }
        if let Some((idx, req, res)) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            if (idx, req) == (self.cur, self.req) {
                match res {
                    Ok((vid, info)) => {
                        if let Some(s) = self.queue.get_mut(idx) {
                            if s.dur == 0 {
                                s.dur = info.duration_sec.unwrap_or(0);
                            }
                            if s.vid != vid {
                                s.vid = vid;
                                self.lyr = Lyr::Idle;
                            }
                        }
                        self.quality = Some((codec_name(&info.mime_type), info.bitrate.map_or(0, |b| b / 1000)));
                        self.engine.load(info, self.pos);
                        self.loaded = true;
                        if !self.playing {
                            self.engine.pause();
                        }
                    }
                    Err(e) => self.fail(e),
                }
            }
        }
        if let Some((on, track, rx)) = self.like.take() {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => self.like = Some((on, track, rx)),
                Ok(Err(e)) => {
                    self.liked = !on;
                    self.like_event = Some((track, !on));
                    crate::toast::error(e.clone());
                    self.error = Some(e);
                }
                _ => {}
            }
        }
        if self.loaded && !self.scrub {
            self.pos = self.engine.position();
        }
        self.extend_queue(ctx);
        if self.loaded {
            self.look_ahead();
            self.chain_next();
        }
        if self.playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    pub fn poll_lyrics(&mut self, ctx: &egui::Context) {
        if let Lyr::Idle = self.lyr {
            if let Some(s) = self.song() {
                let (id, title, artist, dur, album) = (s.vid.clone(), s.title.clone(), s.artist.clone(), s.dur, s.album.clone());
                let (tx, rx) = channel();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(backend::lyrics(&id, &title, &artist, dur, &album));
                    ctx.request_repaint();
                });
                self.lyr = Lyr::Loading(rx);
            } else {
                self.lyr = Lyr::Done(None);
            }
        }
        if let Lyr::Loading(rx) = &self.lyr {
            match rx.try_recv() {
                Ok(Ok(l)) => self.lyr = Lyr::Done(l),
                Ok(Err(e)) => self.lyr = Lyr::Failed(e),
                Err(_) => {}
            }
        }
    }
}

/// Thin progress/volume bar; grows on hover like the scrubber in the mock. Returns the pointer fraction while pressed.
fn bar(ui: &egui::Ui, key: &str, rect: Rect, track: Color32, fill: Color32, frac: f32, grow: bool, cached: &[(f32, f32)]) -> Option<f32> {
    let resp = ui.interact(rect.expand2(vec2(0., 6.)), ui.scope_id().with(key), Sense::click_and_drag());
    let h = if grow { 4. + 2. * ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with((key, "h")), resp.hovered(), 0.12, ease_out) } else { 4. };
    let r = Rect::from_center_size(rect.center(), vec2(rect.width(), h));
    ui.painter().rect_filled(r, rad(h / 2.), track);
    for &(a, b) in cached {
        let seg = Rect::from_min_max(pos2(r.left() + r.width() * a, r.top()), pos2(r.left() + r.width() * b, r.bottom()));
        ui.painter().rect_filled(seg, rad(h / 2.), fill.gamma_multiply(0.35));
    }
    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * frac, h)), rad(h / 2.), fill);
    resp.is_pointer_button_down_on().then(|| resp.interact_pointer_pos().map(|p| ((p.x - rect.min.x) / rect.width()).clamp(0., 1.))).flatten()
}

/// Hover affordance for a text link: pointing hand and a 1px underline in the text colour.
fn link_hover(ctx: &egui::Context, p: &egui::Painter, resp: &egui::Response, r: Rect, col: Color32) {
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        p.line_segment([r.left_bottom() + vec2(0., 1.), r.right_bottom() + vec2(0., 1.)], (1., col));
    }
}

/// Synced-lyrics scroll: every row has its own offset, so a line change can cascade down the list (BitChord's `ScrollRun`).
#[derive(Default)]
struct LyrAnim {
    cur: Option<usize>,
    back: bool,
    /// When the current line changed, and the scroll target it changed to.
    t0: f64,
    to: f32,
    /// Per-row offsets when the change began, and as drawn last frame (the start of an interrupting change).
    from: Vec<f32>,
    shown: Vec<f32>,
    /// The user scrolled away: where, and when they last touched the wheel. Following resumes after `BROWSE_HOLD`.
    browse: Option<(f32, f64)>,
    /// Last frame drawn, so reopening the view snaps instead of replaying a stale cascade.
    seen: f64,
}

/// BitChord's lyric motion constants (PlayerLyrics.kt): easing, scroll run length, per-row stagger, falloff by distance from the line.
fn lyr_ease(t: f32) -> f32 {
    bezier(0.41, 0., 0.12, 0.99, t)
}
const LYR_DUR: f32 = 0.35;
const LYR_STAGGER: f32 = 0.06 * LYR_DUR;
const LINE_ALPHA: [f32; 5] = [1., 0.8, 0.7, 0.58, 0.46];
const LINE_BLUR: [f32; 5] = [0., 1., 1., 1.7, 2.4];
const GLOW_ROOM: f32 = 10.;
const GAP_ROW: f32 = 40. + 16.;
const BROWSE_HOLD: f64 = 5.;

/// Cover, backdrop and video crossfade time.
const ART_FADE: f32 = 0.25;

/// Cubic ease-in-out, so a crossfade starts and ends gently instead of ramping linearly.
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 { 4. * t * t * t } else { 1. - (-2. * t + 2.).powi(3) / 2. }
}

/// What the player art shows, kept in egui memory between frames.
#[derive(Clone, Default)]
struct ArtFade {
    /// The cover url being faded in (or fully shown), and the one it replaced while fading.
    shown: Option<String>,
    prev: Option<String>,
    /// Bumped on each switch; `animate_value` eases toward it so the fade runs from `gen - 1` to `gen`.
    gen: u32,
    /// Last video frame, kept (and its texture alive) so the video can fade out after video.rs drops it.
    last: Option<(egui::TextureHandle, f32)>,
}

/// Opacity of the button backgrounds over the backdrop, low enough for the cover to show through.
const GLASS: f32 = 0.6;

/// The blur's dimming at alpha `a`, premultiplied so a layer fades over whatever lies beneath it.
fn dim_alpha(a: f32) -> Color32 {
    let g = (DIM as f32 * a) as u8;
    Color32::from_rgba_premultiplied(g, g, g, (a * 255.) as u8)
}

impl App {
    /// `open` is 0..1 (0 = collapsed to the peek sliver); the card keeps its full size and is just slid by the caller.
    pub fn player(&mut self, ui: &mut egui::Ui, slot: Rect, open: f32) {
        let t = self.t;
        let mut owned = App::region(ui, slot);
        let view = self.view;
        // Rounded card inset from the slot like the page card; fixed width so closing just slides it out behind the clip.
        let c = Rect::from_min_max(pos2(slot.min.x + 10., slot.min.y), pos2(slot.min.x + self.player_w - 10., slot.max.y - 10.));
        let (x0, x1) = (c.min.x + 18., c.max.x - 18.);
        let w = x1 - x0;
        // Content scrolls under the card's rounded corners, so the corners are masked afterwards with whatever lies behind the card.
        let outer = owned.painter().clone();
        let corners = |bg: Color32| {
            round_top_corners(&outer, c, 18., bg);
            round_bottom_corners(&outer, c, 18., bg);
        };
        let behind = t.s0;

        let empty = self.pl.song().is_none();
        let url = self.pl.song().and_then(|s| s.thumbnail.clone());
        let blur = url.as_deref().and_then(|u| self.g.backdrop(owned.ctx(), u));
        let ui = &mut App::region(&mut owned, c);
        let ui_id = ui.scope_id();

        // Ink: text and controls over the backdrop switch to dark tones when the cover is bright, so they stay readable.
        // 0.575 is where white and dark ink have equal contrast; the faint tiers keep a lightness gap from mid-tone backdrops.
        // The controls sit on the cover's lower 40%; the lyrics/queue body sits on a different band, so it gets its own ink.
        let tones = |ctx: &egui::Context, key: &str, l: f32| {
            let dark = ctx.animate_value_with_time(ui_id.with(key), if l > 0.575 { 1. } else { 0. }, 0.4);
            let ink = |light: f32, gap: f32, dark_l: f32| mix(oklch(light.max((l + gap).min(0.9)), 0.02, t.hue), oklch(dark_l, 0.02, t.hue), dark);
            let theme = Theme {
                tx: mix(t.tx, oklch(0.18, 0.02, t.hue), dark),
                tm: ink(0.72, 0.30, 0.28),
                tf: ink(0.56, 0.20, 0.36),
                ax: mix(t.ax, oklch(0.36, 0.12, t.hue), dark),
                s2: t.s2.lerp_to_gamma(Color32::from_black_alpha(30), dark),
                s3: t.s3.lerp_to_gamma(Color32::from_black_alpha(46), dark),
                ..t
            };
            (theme, dark)
        };
        let l = url.as_deref().and_then(|u| self.g.luma(u, 0.3, 0.5)).unwrap_or(0.);
        let (it, dark) = tones(ui.ctx(), "pl-ink", l);

        // The whole card scrolls: cover/lyrics/queue slot, then the controls (CTRL_H), then the info cards.
        // `va` morphs the art view (0) into the compact header (1) so the cover and title travel between them.
        const CTRL_H: f32 = 248.;
        let art = view == PlayerView::Art;
        let va = crate::app::anim(ui.ctx(), "pl-view", !art);
        let cards_h = if empty { 0. } else { self.info_cards(ui, c, 0., false) };
        // Lyrics/queue body: shrinks (down to 240) so the controls stay on screen in short windows, caps at 380 so tall ones show the cards.
        let body_h = (c.height() - 118. - CTRL_H - 6.).clamp(240., 380.);
        let top_h = lerp(c.width()..=118. + body_h, va).max(c.height() - CTRL_H - 6. - cards_h);
        let max_sc = (top_h + CTRL_H + 6. + cards_h - c.height()).max(0.);
        let sc = self.pl.card_scroll.clamp(0., max_sc);
        let body_hit = Rect::from_min_max(pos2(x0, c.min.y - sc + 118.), pos2(x1, c.min.y - sc + top_h));
        let wheel = ui.input(|i| i.pointer.hover_pos().filter(|p| c.contains(*p) && (art || !body_hit.contains(*p))).map_or(0., |_| i.smooth_scroll_delta.y));
        let sc = if open > 0.9 { (sc - wheel).clamp(0., max_sc) } else { sc };
        self.pl.card_scroll = sc;
        let base = c.min.y - sc;
        let yb = base + top_h + CTRL_H;

        let square = Rect::from_min_size(pos2(c.min.x, base), vec2(c.width(), c.width()));

        // Bottom stack, built upward: error line, mode row, volume, transport, scrubber (14px gaps).
        let mut y = yb - 12.;
        y -= 14. + 14.;
        let modes_y = y - 21.;
        y -= 42. + 14.;
        let vol_y = y - 8.;
        let q_y = vol_y - 8. - 4. - 7.;
        y -= 16. + 14. + 18.;
        let tr_y = y - 26.;
        y -= 52. + 14.;
        let scrub = Rect::from_min_size(pos2(x0, y - 24.), vec2(w, 24.));
        let body_bottom = scrub.min.y - 14.;

        // Cover: a square at the top of the art view (a wide video thumbnail floats instead), a 46px thumbnail in the compact header.
        let ident_top = base + 12. + 32. + 14.;
        let aspect = url.as_deref().and_then(|u| cover_aspect(ui.ctx(), u, square.size()));
        let wide = aspect.filter(|a| *a > 1.2);
        // A wide cover is a video's: with videos on, the video itself plays in the floating rect (paced by the audio playhead).
        let want = (self.video_on && wide.is_some() && open > 0.5).then(|| self.pl.song().map(|s| s.vid.clone())).flatten().filter(|v| !v.is_empty());
        #[cfg(feature = "video")]
        let frame = self.video_frame(ui.ctx(), want.as_deref(), self.video_h);
        #[cfg(not(feature = "video"))]
        let frame: Option<(egui::TextureHandle, f32)> = want.and(None);

        // Crossfades: a new cover only replaces the shown one once its cover and blur have loaded; the outgoing layer stays
        // under the incoming one, so the stack is always opaque. The video fades over the cover by `vf`.
        let (ctx, id) = (ui.ctx().clone(), ui.scope_id().with("pl-art"));
        let mut st = ctx.data(|d| d.get_temp::<ArtFade>(id)).unwrap_or_default();
        if url != st.shown && (url.is_none() || (blur.is_some() && aspect.is_some())) {
            st.prev = if url.is_some() { st.shown.take() } else { None };
            st.shown = url.clone();
            st.gen += 1;
        }
        let f = ease_in_out((ctx.animate_value_with_time(id.with("gen"), st.gen as f32, ART_FADE) - st.gen.saturating_sub(1) as f32).clamp(0., 1.));
        let vf = ctx.animate_bool_with_time_and_easing(id.with("vid"), frame.is_some(), ART_FADE, ease_in_out);
        if frame.is_some() {
            st.last = frame.clone();
        } else if vf == 0. {
            st.last = None;
        }
        let vtex = frame.clone().or(st.last.clone());
        let old_blur = st.prev.as_deref().and_then(|u| self.g.backdrop(&ctx, u));
        let cur_blur = st.shown.as_deref().and_then(|u| self.g.backdrop(&ctx, u));
        ctx.data_mut(|d| d.insert_temp(id, st.clone()));

        // Backdrop: the blurred, dimmed cover lined up with the art view's square cover and reflected below it. Scrolls with the card.
        let layers = [old_blur.filter(|_| f < 1.).map(|o| (o, 1.)), cur_blur.map(|b| (b, f))];
        for (tex, a) in layers.into_iter().flatten() {
            egui::Image::new(sized(tex, 1.)).fit_to_exact_size(c.size()).uv(backdrop_uv(square, c)).corner_radius(rad(18.)).tint(dim_alpha(a)).paint_at(ui, c);
        }
        if cur_blur.is_none() {
            ui.painter().rect_filled(c, rad(18.), t.s1);
        }

        let wide_a = wide.map(|a| frame.as_ref().map_or(a, |f| f.1));
        let start = wide_a.map_or(square, |a| {
            let slot = Rect::from_min_max(pos2(x0, base + 56.), pos2(x1, body_bottom - 62.));
            let h = (w / a).min(slot.height().max(0.));
            Rect::from_center_size(slot.center(), vec2(h * a, h))
        });
        // A video keeps its shape in the compact header too.
        let small = Rect::from_min_size(pos2(x0, ident_top), vec2(46. * wide_a.unwrap_or(1.), 46.));
        let cr = Rect::from_min_max(start.min.lerp(small.min, va), start.max.lerp(small.max, va));
        let (r0, b0): (f32, f32) = if wide.is_some() { (12., 12.) } else { (18., 0.) };
        let (top_r, bot_r) = (lerp(r0..=6., va) as u8, lerp(b0..=6., va) as u8);
        let shown = lerp(1.0..=open, va);
        let cr_corners = CornerRadius { nw: top_r, ne: top_r, sw: bot_r, se: bot_r };
        let tint = Color32::WHITE.gamma_multiply(shown);
        if wide.is_some() {
            shadow(ui.painter(), cr, 12., 8, 28, (90. * (1. - va)) as u8);
        }
        if let Some(p) = st.prev.as_deref().filter(|_| f < 1.) {
            paint_cover(ui, p, cr, cr_corners, tint);
        }
        match st.shown.as_deref() {
            Some(u) => paint_cover(ui, u, cr, cr_corners, tint.gamma_multiply(f)),
            None if url.is_none() && empty => icon(ui, icon!("song"), square.center(), 72., it.tf.gamma_multiply(1. - va)),
            None if url.is_none() => {
                ui.painter().rect_filled(cr, cr_corners, it.s3.gamma_multiply(shown));
            }
            None => {}
        }
        if let Some((tex, a)) = vtex.filter(|_| vf > 0.) {
            egui::Image::new(sized(tex.id(), 1.)).fit_to_exact_size(cr.size()).uv(crop_uv(a, cr.width() / cr.height())).corner_radius(cr_corners).tint(tint.gamma_multiply(vf)).paint_at(ui, cr);
        }
        #[cfg(feature = "video")]
        if want.is_some() && self.video.as_ref().is_some_and(|v| v.waiting(self.pl.heard(), self.pl.dur())) {
            let time = ctx.input(|i| i.time) as f32;
            ui.painter().rect_filled(cr, cr_corners, Color32::BLACK.gamma_multiply(0.35 * shown));
            crate::page::spinner(ui.painter(), cr.center(), (cr.height() * 0.1).clamp(8., 16.), time, Color32::WHITE);
            ctx.request_repaint();
        }
        if let Some(u) = self.pl.next_song().and_then(|s| s.thumbnail.as_deref()) {
            preload_cover(ui.ctx(), u, square.size());
        }
        // Dissolves the cover's lower half into the backdrop. It follows the cover (not the art-view square) and is gone
        // by mid-morph, so it never sits over the cover as it travels to the header.
        let dis = (1. - va * 2.).max(0.);
        if wide.is_none() && dis > 0. {
            let fp = ui.painter().with_clip_rect(cr);
            if let Some(o) = old_blur.filter(|_| f < 1.) {
                fade_blur(&fp, o, c, square, cr.center().y, cr.max.y, dis);
            }
            if let Some(b) = cur_blur {
                fade_blur(&fp, b, c, square, cr.center().y, cr.max.y, dis * f);
            }
        }
        if open < 0.02 {
            corners(behind);
            return;
        }
        ui.set_opacity(open);
        let p = ui.painter().clone();

        // Right-click on the cover in the art view opens the track menu; applied once the drawing borrows end.
        let mut pending_menu: Option<(egui::Pos2, Target)> = None;
        let mut menu_btn = None;
        if va < 0.5 {
            let hit = ui.interact(cr, ui.scope_id().with("pl-cover"), Sense::click());
            if let (true, Some(pos), Some(s)) = (hit.secondary_clicked(), hit.interact_pointer_pos(), self.pl.song()) {
                pending_menu = Some((pos, Target::Track(s.track())));
            }
        }
        // Video / thumbnail toggle and quality in the floating rect's corner; they fade in while the pointer is over it.
        if wide.is_some() && va < 0.5 && cfg!(feature = "video") {
            let over = ui.rect_contains_pointer(cr);
            let a = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with("pl-vctl"), over, 0.15, ease_in_out);
            if a > 0. {
                let at = pos2(cr.max.x - 24., cr.max.y - 24.);
                p.circle_filled(at, 16., Color32::from_black_alpha(140).gamma_multiply(a));
                let (ic, tip) = if self.video_on { (icon!("vidoff"), "Hide the video") } else { (icon!("vid"), "Play the video") };
                if glyph_button(ui, "pl-video", at, 32., ic, 16., Color32::WHITE.gamma_multiply(a)).on_hover_text(tip).clicked() {
                    self.video_on = !self.video_on;
                }
                if self.video_on {
                    // Quality: a pill left of the toggle; click steps up through the offered heights, right-click steps down.
                    let label = format!("{}p", self.video_h);
                    let w = text_w(&p, &label, font_w(11., Weight::Semi)) + 16.;
                    let pill = Rect::from_center_size(pos2(at.x - 16. - 6. - w / 2., at.y), vec2(w, 24.));
                    p.rect_filled(pill, rad(12.), Color32::from_black_alpha(140).gamma_multiply(a));
                    p.text(pill.center(), Align2::CENTER_CENTER, &label, font_w(11., Weight::Semi), Color32::WHITE.gamma_multiply(a));
                    let r = press(ui, "pl-vq", pill).on_hover_text("Video quality (right-click for lower)");
                    let hs = crate::app::VIDEO_HEIGHTS;
                    let i = hs.iter().position(|h| *h == self.video_h).unwrap_or(0);
                    if r.clicked() {
                        self.video_h = hs[(i + 1) % hs.len()];
                    } else if r.secondary_clicked() {
                        self.video_h = hs[(i + hs.len() - 1) % hs.len()];
                    }
                }
            }
        }

        let pl = &mut self.pl;
        let dur = pl.dur();
        let (title, artist, album) = pl.song().map_or_else(|| ("Nothing playing".to_string(), "Pick a song to start listening".to_string(), String::new()), |s| (s.title.clone(), s.artist.clone(), s.album.clone()));
        let artist_id = pl.song().and_then(|s| s.artist_id.clone());

        // The pointer only moves the shown playhead; playback carries on and the new spot is committed on release.
        if let Some(f) = bar(ui, "scrub", Rect::from_min_size(scrub.min, vec2(w, 4.)), it.s3, it.tx, pl.frac(), true, &pl.engine.cached(dur)) {
            pl.pos = f * dur;
            pl.scrub = true;
        } else if pl.scrub {
            pl.scrub = false;
            pl.seek(pl.pos);
        }
        p.text(pos2(x0, scrub.min.y + 17.), Align2::LEFT_CENTER, clock(pl.pos), font(11.), it.tf);
        p.text(pos2(x1, scrub.min.y + 17.), Align2::RIGHT_CENTER, format!("−{}", clock((dur - pl.pos).max(0.))), font(11.), it.tf);

        // Spread across the width like the mode row below, so resizing the player keeps them evenly spaced.
        let tr_x = [x0 + w * 0.15, x0 + w * 0.5, x0 + w * 0.85];
        let main_icon = if pl.playing { icon!("pause") } else { icon!("play") };
        let buffering = pl.buffering();
        for (i, (ic, s)) in [(icon!("rw"), 34.), (main_icon, 46.), (icon!("ff"), 34.)].into_iter().enumerate() {
            let col = if i == 1 && buffering { Color32::TRANSPARENT } else if empty { it.tf } else { it.tx };
            if i == 1 && buffering {
                crate::page::spinner(ui.painter(), pos2(tr_x[i], tr_y), 14., ui.input(|i| i.time) as f32, it.tx);
                ui.ctx().request_repaint();
            }
            if glyph_button(ui, ("tr", i), pos2(tr_x[i], tr_y), 52., ic, s, col).clicked() {
                match i {
                    0 => pl.prev(ui.ctx()),
                    1 => pl.toggle(ui.ctx()),
                    _ => pl.next(ui.ctx()),
                }
            }
        }

        if let Some((codec, kbps)) = pl.quality.as_ref().filter(|_| !empty) {
            let (ic, tier) = if *kbps >= 256 { (icon!("aqh"), "High") } else { (icon!("aqs"), "Standard") };
            let label = if *kbps == 0 { codec.clone() } else { format!("{tier} · {codec} · {kbps} kbps") };
            let font = font(11.);
            let tw = text_w(ui.painter(), &label, font.clone());
            let x = (x0 + x1) / 2. - (tw + 18.) / 2.;
            icon(ui, ic, pos2(x + 6., q_y), 12., it.tm);
            ui.painter().text(pos2(x + 18., q_y), Align2::LEFT_CENTER, label, font, it.tm);
        }
        icon(ui, icon!("vlo"), pos2(x0 + 8., vol_y), 16., it.tm);
        icon(ui, icon!("vhi"), pos2(x1 - 8., vol_y), 16., it.tm);
        if let Some(v) = bar(ui, "vol", Rect::from_min_max(pos2(x0 + 26., vol_y - 2.), pos2(x1 - 26., vol_y + 2.)), it.s3, it.tm, self.volume, false, &[]) {
            self.volume = v;
            pl.engine.set_volume(v);
        }

        // Mode row: lyrics | shuffle, repeat pill | queue.
        for (i, (v, ic, cx)) in [(PlayerView::Lyrics, icon!("lyr"), tr_x[0]), (PlayerView::Queue, icon!("que"), tr_x[2])].into_iter().enumerate() {
            let r = Rect::from_center_size(pos2(cx, modes_y), vec2(44., 38.));
            let on = view == v;
            if on {
                p.rect_filled(r, rad(11.), it.s3.gamma_multiply(GLASS));
            }
            if glyph_button(ui, ("mode", i), r.center(), 44., ic, 18., if on { it.tx } else { it.tm }).clicked() {
                self.view = if on { PlayerView::Art } else { v };
                pl.scroll = 0.;
            }
        }
        let pw = (tr_x[2] - tr_x[0] - 44. - 24.).min(150.);
        let pill = Rect::from_center_size(pos2(tr_x[1], modes_y), vec2(pw, 42.));
        let seg_w = pill.width() / 3.;
        p.rect_filled(pill, rad(21.), it.s2.gamma_multiply(GLASS));
        let modes = [(icon!("shuf"), pl.shuffle), (icon!("rep"), pl.repeat), (icon!("inf"), pl.endless)];
        for (i, (ic, on)) in modes.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(pill.min.x + i as f32 * seg_w, pill.min.y), vec2(seg_w, pill.height()));
            let cr = CornerRadius { nw: if i == 0 { 21 } else { 0 }, sw: if i == 0 { 21 } else { 0 }, ne: if i == 2 { 21 } else { 0 }, se: if i == 2 { 21 } else { 0 } };
            if on {
                p.rect_filled(r, cr, it.tx.gamma_multiply(0.24));
            }
            if i > 0 {
                p.rect_filled(Rect::from_min_size(r.min, vec2(1., r.height())), 0., it.s0);
            }
            if glyph_button(ui, ("pm", i), r.center(), r.height().min(seg_w), ic, 20., if on { it.tx } else { it.tm }).clicked() {
                match i {
                    0 => pl.toggle_shuffle(),
                    1 => pl.toggle_repeat(),
                    _ => pl.toggle_endless(),
                }
            }
        }

        // Header: album name (compact views) + hide button.
        let top = base + 12.;
        if va < 0.99 {
            let disc = Color32::from_black_alpha(90).lerp_to_gamma(Color32::from_white_alpha(70), dark).gamma_multiply(1. - va);
            p.circle_filled(pos2(x1 - 16., top + 16.), 16., disc);
        }
        if va > 0.01 {
            trunc(&p, pos2(x0 + 4., top + 16.), w - 44., if album.is_empty() { &title } else { &album }, font_w(13., Weight::Semi), it.tx.gamma_multiply(va));
        }
        let hide = glyph_button(ui, "hide", pos2(x1 - 16., top + 16.), 32., icon!("colr"), 16., if va < 0.5 { it.tx } else { it.tm }).clicked();
        // Identity row; in the art view it sits on the cover's lower edge. Title and artist are links.
        let cy = lerp(body_bottom - 50. + 23.0..=ident_top + 23., va);
        let tx = lerp(x0..=small.max.x + 10., va);
        let text_w = x1 - 72. - tx;
        let title_r = trunc(&p, pos2(tx, cy - 9.), text_w, &title, font_w(15., Weight::Bold), it.tx);
        let artist_r = trunc(&p, pos2(tx, cy + 10.), text_w, &artist, font(13.), it.tm);
        let (mut title_click, mut artist_click) = (false, false);
        if !empty {
            let resp = ui.interact(title_r, ui.scope_id().with("pl-title"), Sense::click());
            link_hover(ui.ctx(), &p, &resp, title_r, it.tx);
            title_click = resp.clicked();
        }
        if artist_id.is_some() {
            let resp = ui.interact(artist_r, ui.scope_id().with("pl-artist"), Sense::click());
            link_hover(ui.ctx(), &p, &resp, artist_r, it.tm);
            artist_click = resp.clicked();
        }
        if !empty && glyph_button(ui, "fav", pos2(x1 - 52., cy), 32., if pl.liked { icon!("starf") } else { icon!("star") }, 16., if pl.liked { it.ax } else { it.tm }).clicked() {
            pl.set_like(ui.ctx());
        }
        let more = Rect::from_center_size(pos2(x1 - 16., cy), vec2(32., 32.));
        if !empty && glyph_button(ui, "pl-more", more.center(), 32., icon!("more"), 16., it.tm).clicked() {
            if let Some(tr) = pl.song().map(Song::track) {
                pending_menu = Some((pos2(more.min.x - 200., more.max.y + 6.), Target::Track(tr)));
                menu_btn = Some(more);
            }
        }
        let top = ident_top + 46. + 14.;

        // Body (lyrics / queue) fills what is left.
        let v = |y: f32| (y - square.min.y) / square.height() * 0.5;
        let lb = url.as_deref().and_then(|u| self.g.luma(u, v(top), v(body_bottom))).unwrap_or(0.);
        let (mut it, _) = tones(ui.ctx(), "pl-ink-body", lb);
        // Small text over a cover-tinted backdrop needs more than the faint tier, and an accent of the cover's own hue vanishes into it.
        (it.tf, it.ax) = (it.tm, it.tx);
        // Enter animation on view change (mock: 180ms ease-out, fade + 6px rise); restarts from wherever it is, so it is interruptible.
        let now = ui.input(|i| i.time);
        if self.pl.shown.0 != view {
            self.pl.shown = (view, now);
        }
        let k = ((now - self.pl.shown.1) / 0.18).clamp(0., 1.) as f32;
        let k = 1. - (1. - k) * (1. - k);
        if k < 1. {
            ui.ctx().request_repaint();
        }
        let op = k;
        let body = Rect::from_min_max(pos2(x0, top), pos2(x1, body_bottom)).translate(vec2(0., 6. * (1. - k)));
        let pl = &mut self.pl;
        let mut seek_to = None;
        // While the cover travels to the header it still overlaps the body, which must not paint (text, edge fades) over it.
        let below = Rect::everything_below(cr.max.y);
        match view {
            PlayerView::Art => {}
            PlayerView::Lyrics => {
                // Wider than `body` so the hover highlight's padding isn't clipped flat at the text edge.
                let mut b = App::region(ui, body.expand2(vec2(GLOW_ROOM, 0.)));
                b.set_clip_rect(b.clip_rect().intersect(below));
                b.set_opacity(op);
                let bp = b.painter().clone();
                let foot_h = 28.;
                let area = Rect::from_min_max(body.min, pos2(body.max.x, body.max.y - foot_h - 4.));
                let msg = match &pl.lyr {
                    Lyr::Done(Some(l)) if !l.lines.is_empty() => None,
                    Lyr::Done(_) => Some("No lyrics for this song".to_string()),
                    Lyr::Failed(e) => Some(format!("Lyrics unavailable: {e}")),
                    _ => Some("Loading lyrics…".to_string()),
                };
                if let Some(m) = msg {
                    bp.text(pos2(x0 + 2., area.min.y + 20.), Align2::LEFT_CENTER, m, font(14.), it.tm);
                } else if let Lyr::Done(Some(l)) = &pl.lyr {
                    let cur = if l.synced { l.lines.iter().rposition(|(s, _)| *s <= pl.pos) } else { None };
                    let n = l.lines.len();
                    let now = ui.input(|i| i.time);
                    let ctx = ui.ctx().clone();
                    let gs: Vec<_> = l.lines.iter().map(|(_, tx)| bp.layout(tx.clone(), font_w(30., Weight::Extra), Color32::WHITE, w)).collect();
                    // Instrumental breaks (empty lines) swell open while they play: a row of three dots that fill with the break.
                    let until = |i: usize| l.lines.get(i + 1).map_or(f32::INFINITY, |n| n.0);
                    let swell: Vec<f32> = (0..n).map(|i| {
                        if !l.lines[i].1.is_empty() { return 1. }
                        let on = l.synced && pl.pos >= l.lines[i].0 && pl.pos < until(i);
                        tween(&ctx, ui.scope_id().with(("lyr-gap", i)), on as u8 as f32, if on { 0.4 } else { 0.35 }, lyr_ease)
                    }).collect();
                    // Row tops from the animated heights; `ys_end` is the same with every break at its target, so the follow target
                    // does not drift while one opens.
                    let (mut ys, mut ys_end) = (vec![], vec![]);
                    let (mut ly, mut ly_end) = (30., 30.);
                    for (i, g) in gs.iter().enumerate() {
                        let gap = l.lines[i].1.is_empty();
                        ys.push(ly);
                        ys_end.push(ly_end);
                        ly += if gap { GAP_ROW * swell[i] } else { g.size().y + 2. * GLOW_ROOM };
                        ly_end += if gap { if Some(i) == cur { GAP_ROW } else { 0. } } else { g.size().y + 2. * GLOW_ROOM };
                    }
                    let max_scroll = (ly + area.height() * 0.8 - area.height()).max(0.);
                    let wheel = if b.rect_contains_pointer(area) { ui.input(|i| i.smooth_scroll_delta.y) } else { 0. };
                    let mut offs = vec![pl.scroll; n];
                    let mut browsing = false;
                    let mut follow = 0.;
                    if l.synced {
                        let want = cur.map_or(0., |i| ys_end[i] - 30.).clamp(0., max_scroll);
                        let an = &mut pl.lyr_anim;
                        if an.from.len() != n || now - an.seen > 0.3 {
                            *an = LyrAnim { cur, t0: f64::NEG_INFINITY, to: want, from: vec![want; n], shown: vec![want; n], seen: now, ..Default::default() };
                        }
                        an.seen = now;
                        if wheel != 0. {
                            let at = an.browse.map_or_else(|| an.shown.get(cur.unwrap_or(0)).copied().unwrap_or(want), |b| b.0);
                            an.browse = Some(((at - wheel).clamp(0., max_scroll), now));
                        }
                        follow = want;
                        // Only an idle view with the playing line on screen resumes by itself; otherwise the resync button does it.
                        let cur_visible = |at: f32| cur.is_none_or(|c| {
                            let rh = if l.lines[c].1.is_empty() { GAP_ROW } else { gs[c].size().y + 2. * GLOW_ROOM };
                            ys[c] - at + rh > 0. && ys[c] - at < area.height()
                        });
                        if an.browse.is_some_and(|b| now - b.1 > BROWSE_HOLD && cur_visible(b.0)) {
                            // Hand back to following: every row animates from where the user left it.
                            let at = an.browse.take().map_or(want, |b| b.0);
                            (an.from, an.cur, an.t0, an.to, an.back) = (vec![at; n], cur, now, want, want < at);
                        }
                        if let Some((at, _)) = an.browse {
                            browsing = true;
                            (an.cur, an.to) = (cur, want);
                            offs = vec![at; n];
                            ctx.request_repaint_after(std::time::Duration::from_millis(250));
                        } else {
                            if an.cur != cur || (an.to - want).abs() > 0.5 {
                                // Interrupting: every row restarts from where it is drawn now.
                                an.back = want < an.to;
                                an.from = an.shown.clone();
                                (an.cur, an.t0, an.to) = (cur, now, want);
                            }
                            let c0 = cur.unwrap_or(0) as isize;
                            let mut moving = false;
                            for i in 0..n {
                                let d = if an.back { c0 - i as isize } else { i as isize - c0 }.clamp(0, 3) as f32;
                                let k = ((now - an.t0) as f32 - d * LYR_STAGGER) / LYR_DUR;
                                moving |= k < 1.;
                                offs[i] = lerp(an.from[i]..=an.to, lyr_ease(k.clamp(0., 1.)));
                            }
                            if moving {
                                ctx.request_repaint();
                            }
                        }
                        an.shown.clone_from(&offs);
                        pl.scroll = want;
                    } else if wheel != 0. {
                        pl.scroll = (pl.scroll - wheel).clamp(0., max_scroll);
                        offs = vec![pl.scroll; n];
                    }
                    let lp = bp.with_clip_rect(area.expand2(vec2(GLOW_ROOM, 0.)));
                    let btn = browsing.then(|| {
                        let bw = crate::gfx::text_w(&bp, "Resync", font_w(12., Weight::Semi)) + 28.;
                        Rect::from_min_size(pos2(x1 - bw, area.max.y - 44.), vec2(bw, 30.))
                    });
                    let hover_ok = l.synced && b.rect_contains_pointer(area) && !btn.is_some_and(|r| ui.rect_contains_pointer(r));
                    let scroll_line = cur.unwrap_or(0);
                    for (i, g) in gs.into_iter().enumerate() {
                        let gap = l.lines[i].1.is_empty();
                        let rh = if gap { GAP_ROW * swell[i] } else { g.size().y + 2. * GLOW_ROOM };
                        let top = area.min.y + ys[i] - offs[i];
                        if rh <= 0.5 || top + rh < area.min.y - 24. || top > area.max.y + 24. {
                            continue;
                        }
                        let row = Rect::from_min_size(pos2(x0 - GLOW_ROOM, top), vec2(w + 2. * GLOW_ROOM, rh));
                        let id = ("lyr-line", i);
                        let hit = hover_ok && ui.rect_contains_pointer(row.intersect(area));
                        let ha = ctx.animate_bool_with_time_and_easing(ui.scope_id().with(id), hit, 0.18, ease_out);
                        if ha > 0. {
                            lp.rect_filled(row, rad(10.), it.tx.gamma_multiply(0.10 * ha));
                        }
                        if hit && press(ui, id, row.intersect(area)).clicked() {
                            seek_to = Some(l.lines[i].0);
                            pl.lyr_anim.browse = None;
                        }
                        // Falloff by distance from the line: dimmer and blurrier the further away; browsing or the line itself keeps it sharp.
                        let active = l.synced && Some(i) == cur;
                        let di = i.abs_diff(scroll_line).min(4);
                        let sid = ui.scope_id().with(("lyr-fx", i));
                        let ta = if !l.synced { 0.95 } else if active { 1. } else if browsing { 0.5 } else { LINE_ALPHA[di] };
                        let tb = if !l.synced || browsing || active { 0. } else { LINE_BLUR[di] };
                        let a = tween(&ctx, sid.with("a"), ta, 0.4, lyr_ease);
                        let r = tween(&ctx, sid.with("r"), tb, 0.4, lyr_ease);
                        if gap {
                            let p = ((pl.pos - l.lines[i].0) / (until(i) - l.lines[i].0)).clamp(0., 1.);
                            for k in 0..3 {
                                let f = (p * 3. - k as f32).clamp(0., 1.);
                                let at = pos2(x0 + 6.5 + k as f32 * 18., top + rh / 2.);
                                lp.circle_filled(at, 6.5 * swell[i], it.tx.gamma_multiply(lerp(0.3..=1., f) * a * swell[i]));
                            }
                        } else {
                            blur_galley(&lp, &g, pos2(x0, top + GLOW_ROOM), it.tx, a, r);
                        }
                    }
                    if let Some(b) = blur {
                        let fp = p.with_clip_rect(below);
                        fade_blur(&fp, b, c, square, area.min.y + 28., area.min.y, 1.);
                        fade_blur(&fp, b, c, square, area.max.y - 28., area.max.y, 1.);
                    }
                    if let Some(r) = btn {
                        let hit = press(ui, "lyr-sync", r);
                        bp.rect_filled(r, rad(15.), it.s3.gamma_multiply(if hit.hovered() { 1. } else { 0.9 }));
                        bp.text(r.center(), Align2::CENTER_CENTER, "Resync", font_w(12., Weight::Semi), it.tx);
                        if hit.clicked() {
                            // Animate every row from where the user left it back to the playing line.
                            let an = &mut pl.lyr_anim;
                            let at = an.shown.first().copied().unwrap_or(follow);
                            (an.from, an.t0, an.to, an.back, an.browse) = (vec![at; n], now, follow, follow < at, None);
                        }
                    }
                    p.text(pos2(x0, body.max.y - foot_h / 2.), Align2::LEFT_CENTER, format!("Lyrics by {}", l.provider), font(12.), it.tm);
                }
            }
            PlayerView::Queue if pl.queue.is_empty() => {
                let mut b = App::region(ui, body);
                b.set_clip_rect(b.clip_rect().intersect(below));
                b.set_opacity(op);
                b.painter().text(pos2(x0, body.min.y + 12.), Align2::LEFT_CENTER, "Nothing in the queue", font(14.), it.tm);
            }
            PlayerView::Queue => {
                let mut b = App::region(ui, body);
                b.set_clip_rect(b.clip_rect().intersect(below));
                b.set_opacity(op);
                let bp = b.painter().clone();
                bp.text(pos2(x0, body.min.y + 12.), Align2::LEFT_CENTER, "Queue", font_w(20., Weight::Bold), it.tx);
                let list = Rect::from_min_max(pos2(x0, body.min.y + 30.), body.max);
                let upcoming = pl.queue.len() - pl.cur - 1;
                // Endless mode's songs sit under their own heading, which replaces "NEXT UP" when no queued song is left.
                let auto_at = (pl.cur + 1..pl.queue.len()).find(|&i| pl.queue[i].auto);
                let content = 20. + 52. + 14. + 14. + upcoming as f32 * 54. + if auto_at.is_some_and(|a| a > pl.cur + 1) { 26. } else { 0. };
                let max_scroll = (content - list.height()).max(0.);
                if b.rect_contains_pointer(list) {
                    pl.scroll = (pl.scroll - ui.input(|i| i.smooth_scroll_delta.y)).clamp(0., max_scroll);
                }
                // Dragging near (or past) the list's top or bottom edge scrolls it, faster the further out.
                if let (Some(_), Some(py)) = (pl.qdrag, ui.input(|i| i.pointer.interact_pos()).map(|p| p.y)) {
                    const EDGE: f32 = 56.;
                    let k = ((list.min.y + EDGE - py) / EDGE).clamp(0., 2.) - ((py - (list.max.y - EDGE)) / EDGE).clamp(0., 2.);
                    if k != 0. && (k < 0. && pl.scroll < max_scroll || k > 0. && pl.scroll > 0.) {
                        pl.scroll = (pl.scroll - k * 600. * ui.input(|i| i.stable_dt)).clamp(0., max_scroll);
                        ui.ctx().request_repaint();
                    }
                }
                let mut clear = false;
                if upcoming > 0 {
                    let f = font_w(12., Weight::Semi);
                    let cw = crate::gfx::text_w(&bp, "Clear", f.clone()) + 20.;
                    let r = Rect::from_center_size(pos2(x1 - cw / 2., body.min.y + 12.), vec2(cw, 26.));
                    let hit = press(&b, "q-clear", r);
                    if hit.hovered() {
                        bp.rect_filled(r, rad(13.), it.tx.gamma_multiply(0.12));
                    }
                    bp.text(r.center(), Align2::CENTER_CENTER, "Clear", f, if hit.hovered() { it.tx } else { it.tm });
                    clear = hit.clicked();
                }
                let lp = bp.with_clip_rect(list);
                let label = |y: f32, s: &str| { lp.text(pos2(x0, y), Align2::LEFT_CENTER, s, font_w(10.5, Weight::Semi), it.tf); };
                let slot = list.min.y - pl.scroll + 20.;
                label(slot - 12., "NOW PLAYING");
                // Skipping slides the rows from where they were (`from` playing) to where they are now: the old song leaves up
                // under the heading, the next one takes its slot; going back does the reverse.
                let now = ui.input(|i| i.time);
                let (cur, n) = (pl.cur, pl.queue.len());
                let (from, e) = match pl.qjump {
                    Some((f, t0)) if f != cur && f < n && f.abs_diff(cur) <= 8 && now - t0 < QSLIDE => (f, ease_out_soft(((now - t0) / QSLIDE) as f32)),
                    _ => (cur, 1.),
                };
                if e < 1. {
                    ui.ctx().request_repaint();
                }
                let shift = |j: usize| (qpos(&pl.queue, j, from) - qpos(&pl.queue, j, cur)) * (1. - e);
                // Rows are cut off under the "NOW PLAYING" heading, so a leaving or arriving row vanishes there instead of overlapping it.
                let clip = Rect::from_min_max(pos2(list.min.x, slot.max(list.min.y)), list.max);
                let vp = bp.with_clip_rect(clip);
                // Returns (clicked, removed, right-click position, drag handle). Only upcoming rows have a remove button and a handle.
                let row = |b: &mut egui::Ui, id: usize, s: &Song, qy: f32, on: bool, removable: bool, pad: f32, alpha: f32| -> (bool, bool, Option<egui::Pos2>, Option<egui::Response>) {
                    let r = Rect::from_min_size(pos2(x0, qy), vec2(w, 52.));
                    let resp = press(b, ("q", id), r);
                    let play = !on && resp.clicked();
                    let menu = resp.interact_pointer_pos().filter(|_| resp.secondary_clicked());
                    let cy = r.center().y;
                    let mut gone = false;
                    if removable {
                        gone = glyph_button(b, ("qx", id), pos2(r.max.x - 16., cy), 24., icon!("x"), 14., it.tf).clicked();
                    }
                    let handle = removable.then(|| {
                        let at = pos2(r.min.x + 12., cy);
                        let h = b.interact(Rect::from_center_size(at, vec2(22., 44.)), b.scope_id().with(("qh", id)), Sense::drag());
                        if h.hovered() || h.dragged() {
                            b.ctx().set_cursor_icon(if h.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
                        }
                        (h, at)
                    });
                    b.set_opacity(op * alpha);
                    if let Some((h, at)) = &handle {
                        icon(b, icon!("grip"), *at, 16., if h.hovered() || h.dragged() { it.tx } else { it.tf });
                    }
                    // Every thumbnail is 38 wide so the text lines up; a wide video thumbnail gets shorter instead of wider.
                    let aspect = s.thumbnail.as_deref().and_then(|u| cover_aspect(b.ctx(), u, vec2(38., 38.))).filter(|&a| a > 1.3).unwrap_or(1.);
                    let art = Rect::from_center_size(pos2(r.min.x + pad + 19., cy), vec2(38., 38. / aspect));
                    match &s.thumbnail {
                        Some(u) => paint_cover(b, u, art, rad(6.), Color32::WHITE),
                        None => {
                            b.painter().rect_filled(art, rad(6.), it.s3);
                        }
                    }
                    let tx = art.max.x + 10.;
                    let tw = (r.max.x - 30. - tx).max(0.);
                    let lp = b.painter().with_clip_rect(clip);
                    trunc(&lp, pos2(tx, cy - 8.), tw, &s.title, font_w(13., Weight::Medium), if on { it.ax } else { it.tx });
                    trunc(&lp, pos2(tx, cy + 9.), tw, &s.artist, font(12.), it.tm);
                    (play, gone, menu, handle.map(|h| h.0))
                };
                let mut b = App::region(&mut b, list);
                b.set_clip_rect(clip);
                // Headings travel with the row below them.
                let first = cur + 1;
                if first < n {
                    let text = if auto_at == Some(first) {
                        "AUTOPLAY".to_string()
                    } else if album.is_empty() {
                        "NEXT UP".to_string()
                    } else {
                        format!("NEXT FROM {}", album.to_uppercase())
                    };
                    // Fixed in place: the rows slide over the headings instead of carrying them along.
                    let y = slot + qpos(&pl.queue, first, cur) - 14.;
                    vp.text(pos2(x0, y), Align2::LEFT_CENTER, text, font_w(10.5, Weight::Semi), it.tf);
                    if let Some(a) = auto_at.filter(|&a| a > first) {
                        let y = slot + qpos(&pl.queue, a, cur) - 14.;
                        vp.text(pos2(x0, y), Align2::LEFT_CENTER, "AUTOPLAY", font_w(10.5, Weight::Semi), it.tf);
                    }
                }
                let (mut play, mut gone) = (None, None);
                let ptr_y = ui.input(|i| i.pointer.interact_pos()).map(|p| p.y);
                // Natural row centres, to find where a dragged row would land; the dragged row is drawn last, under the pointer.
                let mut dragged = None;
                let tops: Vec<f32> = (first..n).map(|j| slot + qpos(&pl.queue, j, cur) + 26.).collect();
                // The rows the dragged one passes slide a row's height out of its way, opening a gap where it will land.
                let drag_to = pl.qdrag.zip(ptr_y).map(|(from, py)| {
                    let mid = (py - pl.qgrab).clamp(list.min.y, list.max.y - 52.) + 26.;
                    let k = tops.iter().enumerate().min_by(|a, b| (a.1 - mid).abs().total_cmp(&(b.1 - mid).abs())).map_or(0, |(k, _)| k);
                    (from, first + k)
                });
                for j in from.min(cur)..n {
                    if pl.qdrag == Some(j) {
                        dragged = Some(j);
                        continue;
                    }
                    let y = slot + qpos(&pl.queue, j, cur) + shift(j);
                    if y > list.max.y || y + 52. < clip.min.y {
                        continue;
                    }
                    let (on, removable, pad, alpha) = if j > cur {
                        (false, true, if j == from { 24. * e } else { 24. }, 1.)
                    } else if j == cur {
                        (true, false, if from < cur { 24. * (1. - e) } else { 0. }, 1.)
                    } else {
                        (j == from, false, if j == from { 0. } else { 24. }, 1. - e)
                    };
                    let want = match drag_to {
                        Some((from, to)) if from < to && j > from && j <= to => -54.,
                        Some((from, to)) if to < from && j >= to && j < from => 54.,
                        _ => 0.,
                    };
                    let off = if removable { tween(ui.ctx(), ui.scope_id().with(("qoff", pl.qgen, j)), want, 0.16, ease_out) } else { 0. };
                    if let Some((id, t0)) = &pl.moved {
                        let a = 1. - ((now - t0) / 1.4) as f32;
                        if *id == pl.queue[j].id && a > 0. {
                            vp.rect_filled(Rect::from_min_size(pos2(x0, y + off), vec2(w, 52.)), rad(10.), it.ax.gamma_multiply(0.28 * a * a));
                            ui.ctx().request_repaint();
                        }
                    }
                    let (p_, g_, menu, handle) = row(&mut b, j, &pl.queue[j], y + off, on, removable, pad, alpha);
                    if j >= cur {
                        if let Some(at) = menu {
                            pending_menu = Some((at, Target::Queue(j)));
                        }
                    }
                    if j < cur {
                        continue;
                    }
                    if handle.is_some_and(|h| h.drag_started()) {
                        pl.qdrag = Some(j);
                        pl.qgrab = ptr_y.map_or(26., |py| (py - (y + off)).clamp(0., 52.));
                    }
                    if p_ { play = Some(j); }
                    if g_ { gone = Some(j); }
                }
                // Equaliser bars on the current row.
                let (ex, ey) = (list.max.x - 12., slot + shift(cur) + 26.);
                let time = now as f32;
                for k in 0..3 {
                    let h = if pl.playing { 4. + 8. * (0.5 + 0.5 * (time * (5. + k as f32 * 1.7)).sin()) } else { 6. };
                    vp.rect_filled(Rect::from_min_max(pos2(ex - 10. + k as f32 * 5., ey + 6.), pos2(ex - 7. + k as f32 * 5., ey + 6. - h)), rad(1.), it.ax);
                }
                if pl.playing {
                    ui.ctx().request_repaint_after(Duration::from_millis(60));
                }
                let mut drop_to = None;
                if let (Some(i), Some(py)) = (dragged, ptr_y) {
                    let y = (py - pl.qgrab).clamp(list.min.y, list.max.y - 52.);
                    let target = drag_to.map_or(i, |d| d.1);
                    lp.rect_filled(Rect::from_min_size(pos2(x0, y), vec2(w, 52.)), rad(10.), it.tx.gamma_multiply(0.12));
                    if row(&mut b, i, &pl.queue[i], y, false, true, 24., 1.).3.is_some_and(|h| h.drag_stopped()) {
                        drop_to = Some((i, target));
                    }
                }
                if !ui.input(|i| i.pointer.any_down()) {
                    pl.qdrag = None;
                }
                if let Some(tex) = blur {
                    if pl.scroll > 0.5 {
                        fade_blur(&lp, tex, c, square, list.min.y + 32., list.min.y, op);
                    }
                    if pl.scroll < max_scroll - 0.5 {
                        fade_blur(&lp, tex, c, square, list.max.y - 40., list.max.y, op);
                    }
                }
                if clear {
                    pl.clear_upcoming();
                } else if let Some((from, to)) = drop_to {
                    let id = pl.queue.get(from).map(|s| s.id.clone());
                    pl.move_to(from, to);
                    if from != to {
                        pl.moved = id.map(|id| (id, now));
                    }
                } else if let Some(i) = gone {
                    pl.remove_at(i);
                } else if let Some(i) = play {
                    pl.play_at(ui.ctx(), i, 0.);
                }
            }
        }
        if let Some(s) = seek_to {
            self.pl.seek(s);
        }
        if !empty {
            self.info_cards(ui, c, yb + 6., true);
        }
        corners(behind);
        if hide {
            self.set_player(None);
        }
        if title_click {
            if let Some(tr) = self.pl.song().map(Song::track) {
                self.open_song(tr);
            }
        }
        if artist_click {
            if let Some(id) = &artist_id {
                self.open_artist(id);
            }
        }
        if let Some(m) = pending_menu {
            self.toggle_menu(menu_btn, m);
        }
    }
}

fn codec_name(mime: &str) -> String {
    if mime.contains("opus") { "OPUS" } else if mime.contains("mp4a") { "AAC" } else { "AUDIO" }.into()
}
