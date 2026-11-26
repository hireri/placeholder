//! Reports plays to YouTube so they land in the account history. The daemon checks the sign-in and the setting.
//! ponytail: a repeat of the same song is not reported again, and a failed report is not retried.

use crate::backend;
use std::time::Instant;

/// Seconds of actual play before a song counts (upstream's default history delay).
const AFTER: f32 = 30.;

#[derive(Default)]
pub struct History {
    vid: String,
    played: f32,
    sent: bool,
    last: Option<Instant>,
}

impl History {
    /// Called every frame with the YouTube id of the current song.
    pub fn update(&mut self, vid: Option<&str>, playing: bool) {
        let now = Instant::now();
        let dt = self.last.replace(now).map_or(0., |l| (now - l).as_secs_f32().min(2.));
        let Some(vid) = vid.filter(|v| !v.is_empty()) else {
            self.vid.clear();
            return;
        };
        if self.vid != vid {
            (self.vid, self.played, self.sent) = (vid.into(), 0., false);
        }
        if playing {
            self.played += dt;
        }
        if !self.sent && self.played >= AFTER {
            self.sent = true;
            let path = format!("/youtube/playback/{vid}");
            std::thread::spawn(move || {
                if let Err(e) = backend::post(&path) {
                    eprintln!("history {path}: {e}");
                }
            });
        }
    }
}
