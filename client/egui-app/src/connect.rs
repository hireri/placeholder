//! The app's side of Spotify Connect: tells the helper what is playing and does what other Spotify apps ask.

use crate::app::App;
use crate::backend;
use crate::data::{Detail, Track};
use crate::player::Song;
use eframe::egui::Context;
use serde_json::{json, Value};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Upcoming songs other apps are shown (and so can reorder).
const NEXT: usize = 20;
const PREV: usize = 5;

#[derive(Default)]
pub struct Connect {
    /// What the helper was last told: a hash of everything but the moving playhead.
    sent: u64,
    sent_at: Option<Instant>,
    /// The Spotify album, playlist or artist the queue was loaded from, while it still holds that song.
    context: Option<(String, String)>,
    /// A queue being fetched for another app.
    load: Option<Receiver<Load>>,
    warned: bool,
}

struct Load {
    context: String,
    tracks: Vec<Track>,
    start: usize,
    queued: Vec<Track>,
    at: f32,
    paused: bool,
    shuffle: Option<bool>,
}

/// A song the helper looked up, as a queue track.
fn track_of(v: &Value) -> Option<Track> {
    let id = v["uri"].as_str()?.strip_prefix("spotify:track:")?;
    Some(Track {
        id: format!("spotify:{id}"),
        title: v["title"].as_str()?.into(),
        artists: v["artists"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)).collect(),
        album: v["album"].as_str().map(String::from),
        duration_sec: v["dur"].as_u64().map(|ms| (ms / 1000) as u32),
        thumbnail: v["art"].as_str().map(String::from),
        source: "spotify".into(),
        ..Default::default()
    })
}

/// The daemon route that lists a Spotify context's songs; `None` for contexts it has none for (radio, search results).
fn route(context: &str) -> Option<String> {
    let parts: Vec<&str> = context.split(':').collect();
    Some(match parts[..] {
        ["spotify", "album", id] => format!("/music/album/spotify:{id}"),
        ["spotify", "playlist", id] | ["spotify", "user", _, "playlist", id] => format!("/music/playlist/spotify:{id}"),
        ["spotify", "user", _, "collection"] => "/music/playlist/spotify:liked".into(),
        ["spotify", "artist", id] => format!("/music/artist/spotify:{id}"),
        _ => return None,
    })
}

/// Where to start in `tracks`: the song asked for, else the position asked for. A song the list lacks goes in front.
fn start_at(tracks: &mut Vec<Track>, wanted: Option<Track>, index: Option<usize>) -> usize {
    match wanted {
        Some(w) => tracks.iter().position(|t| t.id == w.id).unwrap_or_else(|| {
            tracks.insert(0, w);
            0
        }),
        None => index.filter(|i| *i < tracks.len()).unwrap_or(0),
    }
}

/// Blocking: the queue for a `load` command.
fn fetch(command: Value) -> Load {
    let context = command["context"].as_str().unwrap_or_default().to_string();
    let mut tracks = route(&context)
        .and_then(|path| backend::get::<Detail>(&path).map_err(|e| eprintln!("spotify connect: {e}")).ok())
        .map_or_else(Vec::new, |page| page.songs());
    let start = start_at(&mut tracks, track_of(&command["track"]), command["index"].as_u64().map(|i| i as usize));
    Load {
        context,
        tracks,
        start,
        queued: command["queue"].as_array().into_iter().flatten().filter_map(track_of).collect(),
        at: command["pos"].as_f64().unwrap_or(0.) as f32 / 1000.,
        paused: command["paused"].as_bool().unwrap_or(false),
        shuffle: command["shuffle"].as_bool(),
    }
}

fn row(s: &Song) -> Value {
    json!({
        "uid": s.id(),
        // A song from elsewhere has no Spotify URI, even one matched to Spotify by name.
        "uri": s.id().strip_prefix("spotify:").map(|id| format!("spotify:track:{id}")),
        "title": s.title, "artist": s.artist, "album": s.album, "dur": u64::from(s.secs()) * 1000,
        "art": s.thumbnail.as_deref().filter(|u| s.public_cover() && u.starts_with("https://")),
    })
}

impl App {
    pub fn connect_tick(&mut self, ctx: &Context) {
        self.connect_commands(ctx);
        self.connect_loaded(ctx);
        self.connect_publish(ctx);
    }

    /// Off during a shared session (something else is steering) and a private one (titles go to Spotify).
    fn connect_on(&self) -> bool {
        self.opts.spotify_connect && self.private.is_none() && self.party.room.is_none() && self.sp.group.is_none()
    }

    fn connect_publish(&mut self, ctx: &Context) {
        let on = self.connect_on();
        let helper = crate::spotify::generation();
        // Nothing to take back when it was never on.
        if !on && self.connect.sent == 0 {
            return;
        }
        let (prev, cur, next) = self.pl.window(PREV, NEXT);
        let volume = (self.volume * f32::from(u16::MAX)).round() as u64;
        let mut h = DefaultHasher::new();
        (on, helper, self.pl.playing, self.pl.buffering(), self.pl.seeks, volume, self.pl.shuffle, self.pl.repeat).hash(&mut h);
        for s in prev.iter().chain(cur).chain(next) {
            (s.id(), s.secs()).hash(&mut h);
        }
        let key = h.finish().max(1);
        if key == self.connect.sent {
            return;
        }
        // A dragged volume or seek bar changes every frame.
        const GAP: Duration = Duration::from_millis(250);
        if let Some(left) = self.connect.sent_at.and_then(|t| GAP.checked_sub(t.elapsed())) {
            return ctx.request_repaint_after(left);
        }
        let state = if on {
            let context = self.connect.context.as_ref().filter(|(_, id)| prev.iter().chain(cur).chain(next).any(|s| s.id() == id)).map(|c| &c.0);
            json!({
                "on": true, "playing": self.pl.playing, "buffering": self.pl.buffering(), "pos": (self.pl.pos * 1000.) as u64,
                "volume": volume, "shuffle": self.pl.shuffle, "repeat": self.pl.repeat, "context": context,
                "cur": cur.map(row), "prev": prev.iter().map(row).collect::<Vec<_>>(), "next": next.iter().map(row).collect::<Vec<_>>(),
            })
        } else {
            json!({ "on": false })
        };
        if crate::spotify::publish(&state.to_string(), on) {
            self.connect.sent = if on { key } else { 0 };
            self.connect.sent_at = Some(Instant::now());
        } else if !on {
            // No helper, so no device to take down.
            self.connect.sent = 0;
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn connect_commands(&mut self, ctx: &Context) {
        for c in crate::spotify::take_commands() {
            if !self.connect_on() {
                continue;
            }
            let pl = &mut self.pl;
            match c["cmd"].as_str().unwrap_or_default() {
                "load" => {
                    let (tx, rx) = channel();
                    let wake = ctx.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(fetch(c));
                        wake.request_repaint();
                    });
                    self.connect.load = Some(rx);
                }
                "pause" if pl.playing => pl.toggle(ctx),
                "resume" if !pl.playing => pl.toggle(ctx),
                // A song tapped in the other app's queue arrives as a skip to it.
                "next" => match c["uid"].as_str().and_then(|id| pl.upcoming_index(id)) {
                    Some(i) => pl.play_index(ctx, i),
                    None => pl.next(ctx),
                },
                "prev" => pl.prev(ctx),
                "seek" => pl.seek(c["pos"].as_f64().unwrap_or(0.) as f32 / 1000.),
                "volume" => {
                    self.volume = (c["v"].as_f64().unwrap_or(0.) as f32 / f32::from(u16::MAX)).clamp(0., 1.);
                    self.push_volume();
                }
                "options" => {
                    if c["shuffle"].as_bool().is_some_and(|on| on != pl.shuffle) {
                        pl.toggle_shuffle();
                    }
                    match (c["repeat_track"].as_bool(), c["repeat_context"].as_bool()) {
                        (Some(true), _) => pl.repeat = 2,
                        (_, Some(true)) => pl.repeat = 1,
                        (Some(false), _) if pl.repeat == 2 => pl.repeat = 0,
                        (_, Some(false)) if pl.repeat == 1 => pl.repeat = 0,
                        _ => {}
                    }
                }
                "queue_add" => {
                    if let Some(t) = track_of(&c["track"]) {
                        pl.enqueue(ctx, &[t], false);
                    }
                }
                "queue_set" => {
                    let ids: Vec<String> = c["uids"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(String::from)).collect();
                    pl.reorder_upcoming(&ids, NEXT);
                }
                "error" if !self.connect.warned => {
                    self.connect.warned = true;
                    crate::toast::error(format!("Spotify Connect: {}", c["message"].as_str().unwrap_or("unavailable")));
                }
                _ => {}
            }
        }
    }

    fn connect_loaded(&mut self, ctx: &Context) {
        let Some(rx) = &self.connect.load else { return };
        let load = match rx.try_recv() {
            Err(TryRecvError::Empty) => return,
            other => other.ok(),
        };
        self.connect.load = None;
        let Some(load) = load.filter(|l| !l.tracks.is_empty()) else {
            return crate::toast::error("Spotify Connect: couldn't load what was sent");
        };
        if let Some(on) = load.shuffle {
            // Set before loading: the list is shuffled behind the song asked for, not over it.
            self.pl.shuffle = false;
            self.pl.play_from(ctx, &load.tracks, load.start, load.at, load.paused);
            if on {
                self.pl.toggle_shuffle();
            }
        } else {
            self.pl.play_from(ctx, &load.tracks, load.start, load.at, load.paused);
        }
        self.pl.enqueue(ctx, &load.queued, true);
        self.connect.context = Some((load.context, load.tracks[load.start].id.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_map_to_pages_and_the_asked_song_starts() {
        assert_eq!(route("spotify:user:me:collection").as_deref(), Some("/music/playlist/spotify:liked"));
        assert_eq!(route("spotify:album:x").as_deref(), Some("/music/album/spotify:x"));
        assert_eq!(route("spotify:station:album:x"), None);
        let t = |id: &str| Track { id: id.into(), ..Default::default() };
        let mut list = vec![t("a"), t("b")];
        assert_eq!(start_at(&mut list, Some(t("b")), None), 1);
        assert_eq!(start_at(&mut list, None, Some(9)), 0);
        assert_eq!((start_at(&mut list, Some(t("z")), None), list[0].id.as_str()), (0, "z"));
    }
}
