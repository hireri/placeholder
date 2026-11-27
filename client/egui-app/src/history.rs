//! Reports plays to YouTube so they land in the account history, and to provider servers (Jellyfin, Navidrome) as now playing, stopped or finished. The daemon checks the sign-in and the setting.
//! ponytail: a repeat of the same song is not reported again, and a failed report is not retried.

use crate::backend;
use std::time::Instant;

/// Seconds of actual play before a YouTube song counts (upstream's default history delay).
const AFTER: f32 = 30.;
/// Last.fm's rule for a provider play: half the track or four minutes, whichever is first.
const PROVIDER_CAP: f32 = 240.;

#[derive(Default)]
pub struct History {
    vid: String,
    provider: bool,
    played: f32,
    pos: f32,
    sent: bool,
    started: bool,
    last: Option<Instant>,
}

impl History {
    /// Called every frame with the id of the current song (and whether it is from a provider server), its position and length in seconds.
    pub fn update(&mut self, song: Option<(&str, bool)>, playing: bool, pos: f32, dur: f32) {
        let now = Instant::now();
        let dt = self.last.replace(now).map_or(0., |l| (now - l).as_secs_f32().min(2.));
        let Some((vid, provider)) = song.filter(|(v, _)| !v.is_empty()) else {
            self.leave();
            return;
        };
        if self.vid != vid {
            self.leave();
            (self.vid, self.provider, self.played, self.sent) = (vid.into(), provider, 0., false);
        }
        self.pos = pos;
        if playing {
            self.played += dt;
        }
        if !self.provider {
            if !self.sent && self.played >= AFTER {
                self.sent = true;
                send(format!("/music/played/{vid}?finished=true"));
            }
            return;
        }
        if playing && !self.started {
            self.started = true;
            send(format!("/music/played/{vid}?state=start"));
        }
        let need = if dur > 0. { (dur / 2.).min(PROVIDER_CAP) } else { PROVIDER_CAP };
        if !self.sent && self.played >= need {
            self.sent = true;
            send(format!("/music/played/{vid}?state=finish&pos={:.1}", self.pos));
        }
    }

    /// The song ended or changed. A provider song that started but never finished is reported stopped at its position.
    fn leave(&mut self) {
        if self.provider && self.started && !self.sent {
            send(format!("/music/played/{}?state=stop&pos={:.1}", self.vid, self.pos));
        }
        self.started = false;
        self.vid.clear();
    }
}

fn send(path: String) {
    std::thread::spawn(move || {
        if let Err(e) = backend::post(&path) {
            eprintln!("history {path}: {e}");
        }
    });
}
