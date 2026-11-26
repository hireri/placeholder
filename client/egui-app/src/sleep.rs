//! Sleep timer: pauses playback after a delay or when the current song ends. Client-only.

use std::time::{Duration, Instant};

#[derive(Clone, Default, PartialEq)]
pub enum Sleep {
    #[default]
    Off,
    At(Instant),
    /// Pause once the song with this (title, artist) is over.
    EndOfSong(String, String),
}

impl Sleep {
    pub fn minutes(m: u64) -> Sleep {
        Sleep::At(Instant::now() + Duration::from_secs(m * 60))
    }

    /// True once playback should stop now; the timer then turns itself off. `song` is the current (title, artist).
    pub fn due(&mut self, song: Option<(&str, &str)>) -> bool {
        let hit = match self {
            Sleep::Off => false,
            Sleep::At(t) => Instant::now() >= *t,
            Sleep::EndOfSong(a, b) => song.is_none_or(|(x, y)| x != a || y != b),
        };
        if hit {
            *self = Sleep::Off;
        }
        hit
    }

    /// Index into the settings control: Off, 15, 30, 60 minutes, end of song.
    pub fn slot(&self) -> usize {
        match self {
            Sleep::Off => 0,
            Sleep::At(_) => usize::MAX,
            Sleep::EndOfSong(..) => 4,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Sleep::Off => "Pause playback after a while or when the song ends".into(),
            Sleep::At(t) => format!("Pauses in {} min", t.saturating_duration_since(Instant::now()).as_secs().div_ceil(60)),
            Sleep::EndOfSong(..) => "Pauses when this song ends".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once() {
        let mut s = Sleep::At(Instant::now() - Duration::from_secs(1));
        assert!(s.due(None));
        assert!(!s.due(None));
        let mut s = Sleep::EndOfSong("a".into(), "b".into());
        assert!(!s.due(Some(("a", "b"))));
        assert!(s.due(Some(("c", "b"))));
        assert!(Sleep::minutes(1).slot() == usize::MAX);
    }
}
