// Discord Rich Presence over the local Discord client's IPC socket (no OAuth: the desktop client is already signed in).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{
    activity::{Activity, ActivityType, Assets, StatusDisplayType, Timestamps},
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
}

pub struct Discord {
    client: DiscordIpcClient,
    connected: bool,
    next_try: Instant,
    shown: Option<Shown>,
    /// Position and time of the last push, to detect seeks.
    last: (f32, Instant),
}

impl Discord {
    pub fn new() -> Self {
        Discord {
            client: DiscordIpcClient::new(APP_ID),
            connected: false,
            next_try: Instant::now(),
            shown: None,
            last: (0., Instant::now()),
        }
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

    /// `song` is (title, artist, cover url). Pushes only when something Discord displays changed.
    pub fn update(&mut self, song: Option<(&str, &str, Option<&str>)>, playing: bool, pos: f32, dur: f32) {
        let Some((title, artist, cover)) = song else {
            return self.clear();
        };
        // Paused: Discord has no paused state for listening, so drop the presence rather than show a ticking bar.
        if !playing {
            return self.clear();
        }
        let now = Instant::now();
        let expected = self.last.0 + (now - self.last.1).as_secs_f32();
        let cover = cover.map(str::to_owned);
        let want = Shown { cover, title: title.into(), artist: artist.into(), playing, dur: dur as u32 };
        let jumped = (pos - expected).abs() > JUMP_SECS;
        if self.shown.as_ref() == Some(&want) && !jumped {
            return;
        }
        if !self.ensure() {
            return;
        }
        let start = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) - (pos * 1000.) as i64;
        let mut ts = Timestamps::new().start(start);
        if dur > 0. {
            ts = ts.end(start + (dur * 1000.) as i64);
        }
        let mut act = Activity::new()
            .activity_type(ActivityType::Listening)
            .status_display_type(StatusDisplayType::Details)
            .details(title)
            .state(artist)
            .timestamps(ts);
        if let Some(c) = want.cover.as_deref() {
            act = act.assets(Assets::new().large_image(c));
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

