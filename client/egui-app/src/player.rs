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
    queue: Vec<Song>,
    cur: usize,
    pub playing: bool,
    /// Playhead in seconds. Read from the engine while a stream is loaded; while resolving, where playback will start.
    pub pos: f32,
    /// True when the engine holds the current song's stream.
    loaded: bool,
    /// The scrubber is being dragged, so the engine position is not read back over the pointer.
    scrub: bool,
    /// Stream lookup in flight for the current song.
    pending: Option<Receiver<Resolved>>,
    req: u64,
    /// The current song has already been re-resolved once after an engine error.
    retried: bool,
    /// Last stream or like failure, shown on the player's muted line.
    pub error: Option<String>,
    pub shuffle: bool,
    pub repeat: bool,
    /// The current song's like as the star shows it; not read back from YouTube, so it starts unfilled.
    pub liked: bool,
    /// Like request in flight: the value it asked for, and the daemon's answer.
    like: Option<(bool, Receiver<Result<(), String>>)>,
    scroll: f32,
    card_scroll: f32,
    lyr: Lyr,
    shown: (PlayerView, f64),
    /// Stream of the next song, looked up while this one plays: (song id, lookup).
    ahead: Option<(String, Receiver<Result<(String, audio::StreamInfo), String>>)>,
}

fn clock(s: f32) -> String {
    format!("{}:{:02}", s as u32 / 60, s as u32 % 60)
}

/// Fisher-Yates driven by a time-seeded xorshift64; no RNG crate needed for a queue order.
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
        Player { engine, queue: vec![], cur: 0, playing: false, pos: 0., loaded: false, scrub: false, pending: None, req: 0, retried: false, error: None, shuffle: false, repeat: false, liked: false, like: None, scroll: 0., card_scroll: 0., lyr: Lyr::Idle, shown: (PlayerView::Art, f64::NEG_INFINITY), ahead: None }
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

    /// Adds `tracks` right after the current song (`next`) or at the end; with an empty queue they start playing.
    pub fn enqueue(&mut self, ctx: &egui::Context, tracks: &[Track], next: bool) {
        if self.queue.is_empty() {
            return self.play_tracks(ctx, tracks, 0, false);
        }
        let at = if next { self.cur + 1 } else { self.queue.len() };
        self.queue.splice(at..at, tracks.iter().map(song_of));
    }

    /// Drops queue item `i`; the current song is not removable.
    pub fn remove_at(&mut self, i: usize) {
        if i != self.cur && i < self.queue.len() {
            self.queue.remove(i);
            if i < self.cur {
                self.cur -= 1;
            }
        }
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
    }

    /// Plays queue item `idx` from `at` seconds. The stream is looked up on a worker; `tick` loads it when it arrives.
    fn play_at(&mut self, ctx: &egui::Context, idx: usize, at: f32) {
        if self.queue.get(idx).map(|s| &s.id) != self.song().map(|s| &s.id) {
            self.liked = false;
            self.like = None;
        }
        self.cur = idx;
        self.scroll = 0.;
        self.lyr = Lyr::Idle;
        self.retried = false;
        self.error = None;
        self.resolve(ctx, at);
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
        self.error = Some(e);
    }

    /// Moves the playhead. Without a loaded stream the next load starts from here.
    pub fn seek(&mut self, secs: f32) {
        self.pos = secs;
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

    /// Likes or unlikes the current song on YouTube. The star flips at once; `tick` reverts it if the daemon refuses.
    pub fn set_like(&mut self, ctx: &egui::Context) {
        let Some(vid) = self.song().map(|s| s.vid.clone()).filter(|v| !v.is_empty()) else { return };
        let on = !self.liked;
        self.liked = on;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(backend::post(&format!("/youtube/like/{vid}?on={on}")));
            ctx.request_repaint();
        });
        self.like = Some((on, rx));
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
        if let Some((on, rx)) = self.like.take() {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => self.like = Some((on, rx)),
                Ok(Err(e)) => {
                    self.liked = !on;
                    self.error = Some(e);
                }
                _ => {}
            }
        }
        if self.loaded && !self.scrub {
            self.pos = self.engine.position();
        }
        if self.loaded {
            self.look_ahead();
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
fn bar(ui: &egui::Ui, key: &str, rect: Rect, track: Color32, fill: Color32, frac: f32, grow: bool) -> Option<f32> {
    let resp = ui.interact(rect.expand2(vec2(0., 6.)), ui.scope_id().with(key), Sense::click_and_drag());
    let h = if grow { 4. + 2. * ui.ctx().animate_bool_with_time(ui.scope_id().with((key, "h")), resp.hovered(), 0.12) } else { 4. };
    let r = Rect::from_center_size(rect.center(), vec2(rect.width(), h));
    ui.painter().rect_filled(r, rad(h / 2.), track);
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

/// Cover, backdrop and video crossfade time.
const ART_FADE: f32 = 0.25;

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
        let behind = t.s0.lerp_to_gamma(t.s1, 1. - open);

        let empty = self.pl.song().is_none();
        let url = self.pl.song().and_then(|s| s.thumbnail.clone());
        let blur = url.as_deref().and_then(|u| self.g.backdrop(owned.ctx(), u));
        let ui = &mut App::region(&mut owned, c);

        // Ink: text and controls over the backdrop switch to dark tones when the cover is bright, so they stay readable.
        let luma = url.as_deref().and_then(|u| self.g.luma(u));
        let dark = ui.ctx().animate_value_with_time(ui.scope_id().with("pl-ink"), if luma.is_some_and(|l| l > 0.62) { 1. } else { 0. }, 0.4);
        let it = Theme {
            tx: mix(t.tx, oklch(0.18, 0.02, t.hue), dark),
            tm: mix(t.tm, oklch(0.30, 0.02, t.hue), dark),
            tf: mix(t.tf, oklch(0.40, 0.02, t.hue), dark),
            s2: t.s2.lerp_to_gamma(Color32::from_black_alpha(30), dark),
            s3: t.s3.lerp_to_gamma(Color32::from_black_alpha(46), dark),
            ..t
        };

        // The whole card scrolls: cover/lyrics/queue slot, then the controls (CTRL_H), then the info cards.
        // `va` morphs the art view (0) into the compact header (1) so the cover and title travel between them.
        const CTRL_H: f32 = 230.;
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
        let device_y = y - 7.;
        y -= 14. + 14.;
        let modes_y = y - 21.;
        y -= 42. + 14.;
        let vol_y = y - 8.;
        y -= 16. + 14.;
        let tr_y = y - 26.;
        y -= 52. + 14.;
        let scrub = Rect::from_min_size(pos2(x0, y - 24.), vec2(w, 24.));
        let body_bottom = scrub.min.y - 14.;

        // Cover: a square at the top of the art view (a wide video thumbnail floats instead), a 46px thumbnail in the compact header.
        let ident_top = base + 12. + 32. + 14.;
        let aspect = url.as_deref().and_then(|u| cover_aspect(ui.ctx(), u, square.size()));
        let wide = aspect.filter(|a| *a > 1.2);
        // A wide cover is a video's: with videos on, the video itself plays in the floating rect (paced by the audio playhead).
        let want = (self.video_on && wide.is_some() && open > 0.5 && va < 0.99).then(|| self.pl.song().map(|s| s.vid.clone())).flatten().filter(|v| !v.is_empty());
        #[cfg(feature = "video")]
        let frame = self.video_frame(ui.ctx(), want.as_deref());
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
        let f = (ctx.animate_value_with_time(id.with("gen"), st.gen as f32, ART_FADE) - st.gen.saturating_sub(1) as f32).clamp(0., 1.);
        let vf = ctx.animate_bool_with_time(id.with("vid"), frame.is_some(), ART_FADE);
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

        let start = wide.map(|a| frame.as_ref().map_or(a, |f| f.1)).map_or(square, |a| {
            let slot = Rect::from_min_max(pos2(x0, base + 56.), pos2(x1, body_bottom - 62.));
            let h = (w / a).min(slot.height().max(0.));
            Rect::from_center_size(slot.center(), vec2(h * a, h))
        });
        let small = Rect::from_min_size(pos2(x0, ident_top), vec2(46., 46.));
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
        if let Some(u) = self.pl.next_song().and_then(|s| s.thumbnail.as_deref()) {
            preload_cover(ui.ctx(), u, square.size());
        }
        if wide.is_none() {
            if let Some(o) = old_blur.filter(|_| f < 1.) {
                fade_blur(ui.painter(), o, c, square, square.center().y, square.max.y, 1. - va);
            }
            if let Some(b) = cur_blur {
                fade_blur(ui.painter(), b, c, square, square.center().y, square.max.y, (1. - va) * f);
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
        // Video / thumbnail toggle in the floating rect's corner.
        if wide.is_some() && va < 0.5 && cfg!(feature = "video") {
            let at = pos2(cr.max.x - 24., cr.max.y - 24.);
            p.circle_filled(at, 16., Color32::from_black_alpha(140));
            let (ic, tip) = if self.video_on { (icon!("vidoff"), "Show the thumbnail") } else { (icon!("vid"), "Play the video") };
            if glyph_button(ui, "pl-video", at, 32., ic, 16., Color32::WHITE).on_hover_text(tip).clicked() {
                self.video_on = !self.video_on;
            }
        }

        let pl = &mut self.pl;
        let dur = pl.dur();
        let (title, artist, album) = pl.song().map_or_else(|| ("Nothing playing".to_string(), "Pick a song to start listening".to_string(), String::new()), |s| (s.title.clone(), s.artist.clone(), s.album.clone()));
        let artist_id = pl.song().and_then(|s| s.artist_id.clone());

        if let Some(e) = &pl.error {
            trunc(&p, pos2(x0, device_y), w, e, font(11.), it.tm);
        }
        if let Some(f) = bar(ui, "scrub", Rect::from_min_size(scrub.min, vec2(w, 4.)), it.s3, it.tx, pl.frac(), true) {
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
        for (i, (ic, s)) in [(icon!("rw"), 34.), (main_icon, 46.), (icon!("ff"), 34.)].into_iter().enumerate() {
            if glyph_button(ui, ("tr", i), pos2(tr_x[i], tr_y), 52., ic, s, if empty { it.tf } else { it.tx }).clicked() {
                match i {
                    0 => pl.prev(ui.ctx()),
                    1 => pl.toggle(ui.ctx()),
                    _ => pl.next(ui.ctx()),
                }
            }
        }

        icon(ui, icon!("vlo"), pos2(x0 + 8., vol_y), 16., it.tm);
        icon(ui, icon!("vhi"), pos2(x1 - 8., vol_y), 16., it.tm);
        if let Some(v) = bar(ui, "vol", Rect::from_min_max(pos2(x0 + 26., vol_y - 2.), pos2(x1 - 26., vol_y + 2.)), it.s3, it.tm, self.volume, false) {
            self.volume = v;
            pl.engine.set_volume(v);
        }

        // Mode row: lyrics | shuffle, repeat pill | queue.
        for (i, (v, ic, cx)) in [(PlayerView::Lyrics, icon!("lyr"), tr_x[0]), (PlayerView::Queue, icon!("que"), tr_x[2])].into_iter().enumerate() {
            let r = Rect::from_center_size(pos2(cx, modes_y), vec2(44., 38.));
            let on = view == v;
            if on {
                p.rect_filled(r, rad(11.), it.s3);
            }
            if glyph_button(ui, ("mode", i), r.center(), 44., ic, 18., if on { it.tx } else { it.tm }).clicked() {
                self.view = if on { PlayerView::Art } else { v };
                pl.scroll = 0.;
            }
        }
        let pw = (tr_x[2] - tr_x[0] - 44. - 24.).min(132.);
        let pill = Rect::from_center_size(pos2(tr_x[1], modes_y), vec2(pw, 42.));
        let seg_w = pill.width() / 2.;
        p.rect_filled(pill, rad(21.), it.s2);
        let modes = [(icon!("shuf"), pl.shuffle), (icon!("rep"), pl.repeat)];
        for (i, (ic, on)) in modes.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(pill.min.x + i as f32 * seg_w, pill.min.y), vec2(seg_w, pill.height()));
            let cr = CornerRadius { nw: if i == 0 { 21 } else { 0 }, sw: if i == 0 { 21 } else { 0 }, ne: if i == 1 { 21 } else { 0 }, se: if i == 1 { 21 } else { 0 } };
            if on {
                p.rect_filled(r, cr, it.tx.gamma_multiply(0.24));
            }
            if i > 0 {
                p.rect_filled(Rect::from_min_size(r.min, vec2(1., r.height())), 0., it.s0);
            }
            if glyph_button(ui, ("pm", i), r.center(), r.height(), ic, 20., if on { it.tx } else { it.tm }).clicked() {
                if i == 0 { pl.toggle_shuffle() } else { pl.toggle_repeat() }
            }
        }

        // Header: album name (compact views) + hide button.
        let top = base + 12.;
        if va < 0.99 {
            let disc = Color32::from_black_alpha(90).lerp_to_gamma(Color32::from_white_alpha(70), dark).gamma_multiply(1. - va);
            p.circle_filled(pos2(x1 - 16., top + 16.), 16., disc);
        }
        if va > 0.01 {
            trunc(&p, pos2(x0 + 4., top + 16.), w - 44., &album, font_w(13., Weight::Semi), it.tx.gamma_multiply(va));
        }
        let hide = glyph_button(ui, "hide", pos2(x1 - 16., top + 16.), 32., icon!("colr"), 16., if va < 0.5 { it.tx } else { it.tm }).clicked();
        // Identity row; in the art view it sits on the cover's lower edge. Title and artist are links.
        let cy = lerp(body_bottom - 50. + 23.0..=ident_top + 23., va);
        let tx = lerp(x0..=x0 + 56., va);
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
        if !empty && glyph_button(ui, "fav", pos2(x1 - 52., cy), 32., if pl.liked { icon!("starf") } else { icon!("star") }, 16., if pl.liked { t.ax } else { it.tm }).clicked() {
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
        match view {
            PlayerView::Art => {}
            PlayerView::Lyrics => {
                let mut b = App::region(ui, body);
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
                    let gs: Vec<_> = l.lines.iter().enumerate().map(|(i, (_, tx))| {
                        bp.layout(tx.clone(), font_w(21., Weight::Bold), if !l.synced || Some(i) == cur { it.tx } else { it.tf }, w - 4.)
                    }).collect();
                    let mut ys = vec![];
                    let mut ly = 10.;
                    for g in &gs {
                        ys.push(ly);
                        ly += g.size().y + 14.;
                    }
                    let max_scroll = (ly - area.height()).max(0.);
                    if l.synced {
                        // Follow the current line, a third of the way down; the animation also keeps repainting while it moves.
                        let want = cur.map_or(0., |i| ys[i] - area.height() * 0.33).clamp(0., max_scroll);
                        pl.scroll = ui.ctx().animate_value_with_time(ui.scope_id().with("lyr-scroll"), want, 0.35);
                    } else if b.rect_contains_pointer(body) {
                        pl.scroll = (pl.scroll - ui.input(|i| i.smooth_scroll_delta.y)).clamp(0., max_scroll);
                    }
                    let lp = bp.with_clip_rect(area);
                    for (g, y) in gs.into_iter().zip(ys) {
                        lp.galley(pos2(x0 + 2., area.min.y + y - pl.scroll), g, Color32::WHITE);
                    }
                    if let Some(b) = blur {
                        fade_blur(&p, b, c, square, area.min.y + area.height() * 0.12, area.min.y, 1.);
                        fade_blur(&p, b, c, square, area.min.y + area.height() * 0.8, area.max.y, 1.);
                    }
                    p.text(pos2(x0, body.max.y - foot_h / 2.), Align2::LEFT_CENTER, format!("Lyrics by {}", l.provider), font(12.), it.tm);
                }
            }
            PlayerView::Queue if pl.queue.is_empty() => {
                let mut b = App::region(ui, body);
                b.set_opacity(op);
                b.painter().text(pos2(x0, body.min.y + 12.), Align2::LEFT_CENTER, "Nothing in the queue", font(14.), it.tm);
            }
            PlayerView::Queue => {
                let mut b = App::region(ui, body);
                b.set_opacity(op);
                let bp = b.painter().clone();
                bp.text(pos2(x0, body.min.y + 12.), Align2::LEFT_CENTER, "Queue", font_w(20., Weight::Bold), it.tx);
                let list = Rect::from_min_max(pos2(x0, body.min.y + 30.), body.max);
                let upcoming = pl.queue.len() - pl.cur - 1;
                let content = 20. + 52. + 14. + 14. + upcoming as f32 * 54.;
                if b.rect_contains_pointer(list) {
                    pl.scroll = (pl.scroll - ui.input(|i| i.smooth_scroll_delta.y)).clamp(0., (content - list.height()).max(0.));
                }
                let lp = bp.with_clip_rect(list);
                let label = |y: f32, s: &str| { lp.text(pos2(x0, y), Align2::LEFT_CENTER, s, font_w(10.5, Weight::Semi), it.tf); };
                let mut qy = list.min.y - pl.scroll;
                label(qy + 8., "NOW PLAYING");
                qy += 20.;
                // Returns (clicked, removed, right-click position). Only upcoming rows have a remove button.
                let row = |b: &mut egui::Ui, id: usize, s: &Song, qy: f32, on: bool, removable: bool| -> (bool, bool, Option<egui::Pos2>) {
                    let r = Rect::from_min_size(pos2(x0, qy), vec2(w, 52.));
                    let resp = press(b, ("q", id), r);
                    let play = !on && resp.clicked();
                    let menu = resp.interact_pointer_pos().filter(|_| resp.secondary_clicked());
                    let cy = r.center().y;
                    let mut gone = false;
                    if removable {
                        gone = glyph_button(b, ("qx", id), pos2(r.max.x - 16., cy), 24., icon!("x"), 14., it.tf).clicked();
                    }
                    b.set_opacity(op);
                    let art = Rect::from_center_size(pos2(r.min.x + 14. + 10. + 19., cy), vec2(38., 38.));
                    match &s.thumbnail {
                        Some(u) => paint_cover(b, u, art, rad(6.), Color32::WHITE),
                        None => {
                            b.painter().rect_filled(art, rad(6.), it.s3);
                        }
                    }
                    let (tx, tw) = (r.min.x + 14. + 10. + 38. + 10., r.width() - 14. - 10. - 38. - 10. - 30.);
                    let lp = b.painter().with_clip_rect(list);
                    trunc(&lp, pos2(tx, cy - 8.), tw, &s.title, font_w(13., Weight::Medium), if on { t.ax } else { it.tx });
                    trunc(&lp, pos2(tx, cy + 9.), tw, &s.artist, font(12.), it.tm);
                    (play, gone, menu)
                };
                let mut b = App::region(&mut b, list);
                if let Some(s) = pl.song() {
                    if let Some(at) = row(&mut b, usize::MAX, s, qy, true, false).2 {
                        pending_menu = Some((at, Target::Queue(pl.cur)));
                    }
                }
                if view == PlayerView::Queue {
                    // Equaliser bars on the current row.
                    let (ex, ey) = (list.max.x - 12., qy + 26.);
                    let time = ui.input(|i| i.time) as f32;
                    for k in 0..3 {
                        let h = if pl.playing { 4. + 8. * (0.5 + 0.5 * (time * (5. + k as f32 * 1.7)).sin()) } else { 6. };
                        lp.rect_filled(Rect::from_min_max(pos2(ex - 10. + k as f32 * 5., ey + 6.), pos2(ex - 7. + k as f32 * 5., ey + 6. - h)), rad(1.), t.ax);
                    }
                    if pl.playing {
                        ui.ctx().request_repaint_after(Duration::from_millis(60));
                    }
                }
                qy += 52. + 2. + 12.;
                label(qy, &if album.is_empty() { "NEXT UP".to_string() } else { format!("NEXT FROM {}", album.to_uppercase()) });
                qy += 14.;
                let (mut play, mut gone) = (None, None);
                for i in pl.cur + 1..pl.queue.len() {
                    let (p_, g_, menu) = row(&mut b, i, &pl.queue[i], qy, false, true);
                    if p_ { play = Some(i); }
                    if g_ { gone = Some(i); }
                    if let Some(at) = menu {
                        pending_menu = Some((at, Target::Queue(i)));
                    }
                    qy += 54.;
                }
                if let Some(i) = gone {
                    pl.remove_at(i);
                } else if let Some(i) = play {
                    pl.play_at(ui.ctx(), i, 0.);
                }
            }
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
