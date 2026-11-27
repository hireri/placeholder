// Discord Rich Presence over the local Discord client's IPC socket (no OAuth: the desktop client is already signed in).

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{
    activity::{Activity, ActivityType, Assets, Button, Party, StatusDisplayType, Timestamps},
    DiscordIpc, DiscordIpcClient,
};

const APP_ID: &str = "1572980024091345097";
/// How long to wait before trying to reach Discord again after it was unavailable.
const RETRY: Duration = Duration::from_secs(15);
/// Drift beyond this (vs. extrapolated position) counts as a seek.
const JUMP_SECS: f32 = 1.5;

#[derive(PartialEq)]
struct Shown {
    cover: Option<String>,
    title: String,
    artist: String,
    playing: bool,
    dur: u32,
    /// Party code and how many listeners are connected, while sharing a party.
    party: Option<(String, i32)>,
    links: Vec<(&'static str, String)>,
}

pub struct Discord {
    client: DiscordIpcClient,
    connected: bool,
    next_try: Instant,
    shown: Option<Shown>,
    /// Position and time of the last push, to detect seeks.
    last: (f32, Instant),
    covers: HashMap<String, Option<String>>,
    lookup: Option<(String, Receiver<Option<Option<String>>>)>,
    /// No new lookup before this (after a failed request).
    lookup_after: Instant,
}

#[derive(serde::Deserialize)]
struct CoverReply {
    url: Option<String>,
}

/// Discord rejects text under 2 or over 128 characters: cut long text, pad short with trailing spaces.
fn field(s: &str) -> String {
    let mut t: String = s.chars().take(128).collect();
    while t.chars().count() < 2 {
        t.push(' ');
    }
    t
}

/// First artist of a joined credit.
fn first_artist(a: &str) -> &str {
    // ponytail: splits on common separators, so a band named "A, B" loses its tail; harmless for a search.
    a.split([',', '&', ';']).next().unwrap_or(a).trim()
}

/// Public search/page link for a server song, built from metadata only. `kind`: 1 Last.fm, 2 MusicBrainz.
pub fn meta_link(kind: u8, artist: &str, title: &str) -> Option<(&'static str, String)> {
    use crate::backend::enc;
    let artist = first_artist(artist);
    match kind {
        1 => Some(("View on Last.fm", format!("https://www.last.fm/music/{}/_/{}", enc(artist), enc(title)))),
        2 => Some(("Look up on MusicBrainz", format!("https://musicbrainz.org/search?query={}&type=recording", enc(&format!("{artist} {title}"))))),
        _ => None,
    }
}

impl Discord {
    pub fn new() -> Self {
        Discord {
            client: DiscordIpcClient::new(APP_ID),
            connected: false,
            next_try: Instant::now(),
            shown: None,
            last: (0., Instant::now()),
            covers: HashMap::new(),
            lookup: None,
            lookup_after: Instant::now(),
        }
    }

    /// Cover for a server song from the daemon's MusicBrainz lookup: the cached answer, or None while it is fetched (one request at a time; the repaint on arrival refreshes the presence).
    pub fn lookup_cover(&mut self, ctx: &eframe::egui::Context, artist: &str, album: &str, title: &str) -> Option<String> {
        let artist = first_artist(artist);
        // Without an album the title is the key, so singles don't share one answer.
        let key = format!("{artist}\0{album}\0{}", if album.is_empty() { title } else { "" });
        if let Some((k, rx)) = &self.lookup {
            match rx.try_recv() {
                Ok(r) => {
                    match r {
                        Some(url) => drop(self.covers.insert(k.clone(), url)),
                        None => self.lookup_after = Instant::now() + RETRY * 4,
                    }
                    self.lookup = None;
                }
                Err(TryRecvError::Disconnected) => self.lookup = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(c) = self.covers.get(&key) {
            return c.clone();
        }
        if self.lookup.is_none() && !artist.is_empty() && Instant::now() >= self.lookup_after {
            let (tx, rx) = channel();
            let path = format!("/cover?artist={}&album={}&title={}", crate::backend::enc(artist), crate::backend::enc(album), crate::backend::enc(title));
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                // Err (daemon unreachable) is reported as None so the miss isn't cached.
                let _ = tx.send(crate::backend::get::<CoverReply>(&path).ok().map(|r| r.url.filter(|u| u.starts_with("https://"))));
                ctx.request_repaint();
            });
            self.lookup = Some((key, rx));
        }
        None
    }

    fn ensure(&mut self) -> bool {
        if !self.connected {
            if Instant::now() < self.next_try {
                return false;
            }
            self.connected = match self.client.connect() {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("discord: connect failed: {e}");
                    false
                }
            };
            if !self.connected {
                self.next_try = Instant::now() + RETRY;
                return false;
            }
            self.shown = None;
        }
        true
    }

    fn drop_conn(&mut self) {
        let _ = self.client.close();
        self.connected = false;
        self.shown = None;
        self.next_try = Instant::now() + RETRY;
    }

    /// `song` is (title, artist, cover url); `party` the code and listener count to show instead of the progress bar.
    /// Pushes only when something Discord displays changed. Paused drops the presence unless `keep_paused`.
    pub fn update(&mut self, song: Option<(&str, &str, Option<&str>)>, playing: bool, pos: f32, dur: f32, keep_paused: bool, party: Option<(&str, i32)>, links: Vec<(&'static str, String)>) {
        let Some((title, artist, cover)) = song else {
            return self.clear();
        };
        if !playing && !keep_paused {
            return self.clear();
        }
        let now = Instant::now();
        let expected = self.last.0 + (now - self.last.1).as_secs_f32();
        let cover = cover.map(str::to_owned);
        let want = Shown { cover, title: title.into(), artist: artist.into(), playing, dur: dur as u32, party: party.map(|(c, n)| (c.into(), n)), links };
        let jumped = playing && (pos - expected).abs() > JUMP_SECS;
        if self.shown.as_ref() == Some(&want) && !jumped {
            return;
        }
        if !self.ensure() {
            return;
        }
        let mut act = Activity::new()
            .activity_type(ActivityType::Listening)
            .name("Serafin")
            .status_display_type(StatusDisplayType::Details)
            .details(field(title));
        if playing {
            let start = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) - (pos * 1000.) as i64;
            let mut ts = Timestamps::new().start(start);
            // In a party everyone's playhead is a little different, so a progress bar would only mislead: elapsed time only.
            if dur > 0. && party.is_none() {
                ts = ts.end(start + (dur * 1000.) as i64);
            }
            act = act.timestamps(ts);
            if !artist.is_empty() {
                act = act.state(field(artist));
            }
        } else {
            // No timestamps, so nothing ticks while paused.
            act = act.state(field(&if artist.is_empty() { "Paused".to_string() } else { format!("{artist} · Paused") }));
        }
        if let Some((code, n)) = &want.party {
            act = act.party(Party::new().id(code).size([(*n).clamp(1, crate::party::MAX), crate::party::MAX]));
        }
        if !want.links.is_empty() {
            act = act.buttons(want.links.iter().map(|(label, url)| Button::new(*label, url.as_str())).collect());
        }
        let mut assets = Assets::new();
        if let Some(c) = want.cover.as_deref() {
            assets = assets.large_image(c);
        }
        if let Some((_, n)) = &want.party {
            // On a listening card this is the line under the artist, where Spotify shows the album.
            assets = assets.large_text(format!("In a party ({n} of {})", crate::party::MAX));
        }
        if want.cover.is_some() || want.party.is_some() {
            act = act.assets(assets);
        }
        match self.client.set_activity(act) {
            Ok(()) => {
                self.shown = Some(want);
                self.last = (pos, now);
            }
            Err(e) => {
                eprintln!("discord: set_activity failed: {e}");
                self.drop_conn();
            }
        }
    }

    pub fn clear(&mut self) {
        if self.shown.take().is_some() && self.client.clear_activity().is_err() {
            self.drop_conn();
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_limits() {
        assert_eq!(field("A"), "A ");
        assert_eq!(field(""), "  ");
        assert_eq!(field("ab"), "ab");
        assert_eq!(field(&"é".repeat(200)).chars().count(), 128);
    }

    #[test]
    fn links() {
        assert_eq!(meta_link(1, "A & B", "x y").unwrap().1, "https://www.last.fm/music/A/_/x%20y");
        assert_eq!(meta_link(2, "A", "x").unwrap().1, "https://musicbrainz.org/search?query=A%20x&type=recording");
        assert!(meta_link(0, "A", "x").is_none());
    }
}
