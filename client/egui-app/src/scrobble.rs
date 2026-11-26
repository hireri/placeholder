//! Last.fm scrobbling driven from playback state. The daemon owns the login and the API calls (`/lastfm/*`);
//! this is upstream's ScrobbleManager timer: a song counts after min(percent of its length, max seconds) of actual play,
//! and only when it is longer than the minimum duration.
//! ponytail: no retry queue; a scrobble sent while offline is lost.

use crate::backend;
use crate::data::Track;
use eframe::egui::Context;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Cfg {
    pub configured: bool,
    pub custom_key: bool,
    pub logged_in: bool,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub enabled: bool,
    pub now_playing: bool,
    pub send_likes: bool,
    pub delay_percent: f32,
    pub min_duration: u32,
    pub delay_seconds: u32,
}

#[derive(Default)]
pub struct Scrobbler {
    /// `None` until the daemon has answered.
    pub cfg: Arc<Mutex<Option<Cfg>>>,
    /// Set while the user is approving the sign-in in the browser.
    pending: Arc<Mutex<Option<Instant>>>,
    polled: Option<Instant>,
    asked: bool,
    key: (String, String),
    started: u64,
    played: f32,
    last_pos: f32,
    last: Option<Instant>,
    announced: bool,
    sent: bool,
}

fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn spawn(ctx: &Context, path: &'static str, body: Value) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if let Err(e) = backend::post_json::<_, Value>(path, &body) {
            eprintln!("last.fm {path}: {e}");
        }
        ctx.request_repaint();
    });
}

impl Scrobbler {
    pub fn cfg(&self) -> Option<Cfg> {
        self.cfg.lock().unwrap().clone()
    }

    pub fn refresh(&self, ctx: &Context) {
        let (cfg, ctx) = (self.cfg.clone(), ctx.clone());
        std::thread::spawn(move || {
            if let Ok(c) = backend::get::<Cfg>("/lastfm") {
                *cfg.lock().unwrap() = Some(c);
            }
            ctx.request_repaint();
        });
    }

    /// Sends `body` to the daemon (a `PUT /lastfm`) after applying `edit` locally; the reply replaces the settings.
    pub fn set(&self, ctx: &Context, body: Value, edit: impl FnOnce(&mut Cfg)) {
        if let Some(c) = &mut *self.cfg.lock().unwrap() {
            edit(c);
        }
        self.call(ctx, "PUT", "/lastfm", body);
    }

    /// Web sign-in: the daemon gets a token, the user approves it on last.fm, then `finish` (or the poll in `update_pending`) completes it.
    pub fn start_login(&mut self, ctx: &Context) {
        let (pending, ctx) = (self.pending.clone(), ctx.clone());
        std::thread::spawn(move || {
            match backend::post_json::<_, Value>("/lastfm/auth/start", &json!({})) {
                Ok(v) => {
                    if let Some(u) = v["url"].as_str() {
                        ctx.open_url(eframe::egui::OpenUrl::new_tab(u));
                        *pending.lock().unwrap() = Some(Instant::now());
                    }
                }
                Err(e) => crate::toast::error(format!("Last.fm: {e}")),
            }
            ctx.request_repaint();
        });
    }

    pub fn waiting(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }

    pub fn cancel_login(&self) {
        *self.pending.lock().unwrap() = None;
    }

    /// While waiting for the browser approval, tries to finish every 3 s for up to 5 min. `loud` toasts the reply (the button).
    pub fn poll_login(&mut self, ctx: &Context, loud: bool) {
        let Some(since) = *self.pending.lock().unwrap() else { return };
        if since.elapsed().as_secs() > 300 {
            return self.cancel_login();
        }
        if !loud && self.polled.is_some_and(|t| t.elapsed().as_secs() < 3) {
            return;
        }
        self.polled = Some(Instant::now());
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
        let (cfg, pending, ctx) = (self.cfg.clone(), self.pending.clone(), ctx.clone());
        std::thread::spawn(move || {
            match backend::post_json::<_, Cfg>("/lastfm/auth/finish", &json!({})) {
                Ok(c) => {
                    *cfg.lock().unwrap() = Some(c);
                    *pending.lock().unwrap() = None;
                }
                Err(e) if loud => crate::toast::error(format!("Last.fm: {e}")),
                Err(_) => {}
            }
            ctx.request_repaint();
        });
    }

    /// Both empty removes the custom key. Changing the key signs the account out.
    pub fn set_key(&self, ctx: &Context, key: &str, secret: &str) {
        self.call(ctx, "PUT", "/lastfm/key", json!({ "apiKey": key, "secret": secret }));
    }

    pub fn logout(&self, ctx: &Context) {
        let (cfg, ctx) = (self.cfg.clone(), ctx.clone());
        std::thread::spawn(move || {
            match backend::delete_json::<Cfg>("/lastfm/login") {
                Ok(c) => *cfg.lock().unwrap() = Some(c),
                Err(e) => crate::toast::error(format!("Couldn't sign out of Last.fm: {e}")),
            }
            ctx.request_repaint();
        });
    }

    fn call(&self, ctx: &Context, method: &'static str, path: &'static str, body: Value) {
        let (cfg, ctx) = (self.cfg.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = if method == "PUT" { backend::put_json::<_, Cfg>(path, &body) } else { backend::post_json::<_, Cfg>(path, &body) };
            match r {
                Ok(c) => *cfg.lock().unwrap() = Some(c),
                Err(e) => {
                    crate::toast::error(format!("Last.fm: {e}"));
                    if let Ok(c) = backend::get::<Cfg>("/lastfm") {
                        *cfg.lock().unwrap() = Some(c);
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Mirrors a like to Last.fm's loved tracks.
    pub fn love(&self, ctx: &Context, t: &Track, on: bool) {
        if self.cfg().is_some_and(|c| c.logged_in && c.enabled && c.send_likes) {
            spawn(ctx, "/lastfm/love", json!({ "artist": t.artists.join(", "), "track": t.title, "album": t.album, "love": on }));
        }
    }

    /// Called every frame with the current song as (title, artist, album) and the playhead.
    pub fn update(&mut self, ctx: &Context, song: Option<(&str, &str, &str)>, playing: bool, pos: f32, dur: f32) {
        if !self.asked {
            self.asked = true;
            self.refresh(ctx);
        }
        let now = Instant::now();
        let dt = self.last.replace(now).map_or(0., |l| (now - l).as_secs_f32().min(2.));
        let Some((title, artist, album)) = song else {
            self.key = Default::default();
            return;
        };
        let Some(cfg) = self.cfg().filter(|c| c.logged_in && c.enabled) else { return };
        // A new song, or the same one started over after it was counted (repeat).
        if self.key.0 != title || self.key.1 != artist || (self.sent && pos < 2. && self.last_pos > 5.) {
            self.key = (title.into(), artist.into());
            (self.started, self.played, self.announced, self.sent) = (unix(), 0., false, false);
        }
        self.last_pos = pos;
        let info = json!({ "artist": artist, "track": title, "album": (!album.is_empty()).then_some(album), "duration": (dur > 0.).then_some(dur as u32) });
        if playing {
            self.played += dt;
            if !self.announced {
                self.announced = true;
                if cfg.now_playing {
                    spawn(ctx, "/lastfm/now-playing", info.clone());
                }
            }
        }
        let due = (dur * cfg.delay_percent).min(cfg.delay_seconds as f32);
        if !self.sent && dur > cfg.min_duration as f32 && self.played >= due {
            self.sent = true;
            let mut body = info;
            body["timestamp"] = self.started.into();
            spawn(ctx, "/lastfm/scrobble", body);
        }
    }
}
