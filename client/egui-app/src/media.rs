#![allow(dead_code)]
// OS media controls (MPRIS on Linux, SMTC on Windows, Now Playing on macOS) via souvlaki.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

/// Used when the OS asks for a seek without saying how far.
const DEFAULT_SEEK_SECS: f32 = 5.0;
/// Drift beyond this (vs. extrapolated position) counts as a seek. MPRIS clients extrapolate
/// position themselves, so smaller drift is not pushed.
const JUMP_TOLERANCE_SECS: f32 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek(f32),
    SeekBy(f32),
    SetVolume(f32),
}

#[derive(Debug, Clone, PartialEq)]
struct Meta {
    title: String,
    artist: String,
    album: String,
    cover_url: Option<String>,
    duration_secs: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct Progress {
    playing: bool,
    pos_secs: f32,
    at: Instant,
}

pub struct Media {
    controls: MediaControls,
    rx: Receiver<Cmd>,
    meta: Option<Meta>,
    /// Last state pushed to the OS. `None` means stopped (or never set).
    progress: Option<Progress>,
    /// Last volume pushed; MPRIS clients step from the published value, so it must match the app's.
    volume: Option<f32>,
}

impl Media {
    /// `wake` is called from souvlaki's thread when a command arrives.
    /// `hwnd` is required on Windows and ignored elsewhere.
    pub fn new(wake: impl Fn() + Send + Sync + 'static, hwnd: Option<*mut std::ffi::c_void>) -> Option<Media> {
        let config = PlatformConfig { dbus_name: "serafin", display_name: "Serafin", hwnd };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("media: OS controls unavailable: {e:?}");
                return None;
            }
        };
        let (tx, rx) = mpsc::channel();
        let attached = controls.attach(move |ev| {
            if let Some(cmd) = map_event(ev) {
                if tx.send(cmd).is_ok() {
                    wake();
                }
            }
        });
        if let Err(e) = attached {
            eprintln!("media: failed to attach OS controls: {e:?}");
            return None;
        }
        Some(Media { controls, rx, meta: None, progress: None, volume: None })
    }

    pub fn set_metadata(&mut self, title: &str, artist: &str, album: &str, cover_url: Option<&str>, duration_secs: Option<f32>) {
        let unchanged = self.meta.as_ref().is_some_and(|m| {
            m.title == title
                && m.artist == artist
                && m.album == album
                && m.cover_url.as_deref() == cover_url
                && m.duration_secs == duration_secs
        });
        if unchanged {
            return;
        }
        let duration = duration_secs.map(|d| Duration::from_secs_f32(d.max(0.0)));
        let meta = MediaMetadata {
            title: Some(title),
            artist: Some(artist),
            album: Some(album),
            cover_url,
            duration,
        };
        if let Err(e) = self.controls.set_metadata(meta) {
            eprintln!("media: set_metadata failed: {e:?}");
        }
        self.meta = Some(Meta {
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: album.to_owned(),
            cover_url: cover_url.map(str::to_owned),
            duration_secs,
        });
    }

    pub fn set_playback(&mut self, playing: bool, position_secs: f32) {
        let now = Instant::now();
        let pos = position_secs.max(0.0);
        let expected = match self.progress {
            Some(p) if p.playing => p.pos_secs + (now - p.at).as_secs_f32(),
            Some(p) => p.pos_secs,
            None => pos,
        };
        let state_changed = self.progress.is_none_or(|p| p.playing != playing);
        let jumped = (pos - expected).abs() > JUMP_TOLERANCE_SECS;
        if !state_changed && !jumped {
            return;
        }
        let progress = Some(MediaPosition(Duration::from_secs_f32(pos)));
        let playback = if playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        if let Err(e) = self.controls.set_playback(playback) {
            eprintln!("media: set_playback failed: {e:?}");
        }
        self.progress = Some(Progress { playing, pos_secs: pos, at: now });
    }

    /// Publishes the volume (0..1). Only MPRIS exposes one; elsewhere this is a no-op.
    pub fn set_volume(&mut self, v: f32) {
        if self.volume == Some(v) {
            return;
        }
        self.volume = Some(v);
        #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
        if let Err(e) = self.controls.set_volume(v as f64) {
            eprintln!("media: set_volume failed: {e:?}");
        }
    }

    pub fn set_stopped(&mut self) {
        if self.progress.take().is_some() {
            if let Err(e) = self.controls.set_playback(MediaPlayback::Stopped) {
                eprintln!("media: set_playback(stopped) failed: {e:?}");
            }
        }
    }

    /// Drains commands received from the OS since the last call.
    pub fn poll(&self) -> Vec<Cmd> {
        self.rx.try_iter().collect()
    }
}

fn map_event(ev: MediaControlEvent) -> Option<Cmd> {
    use MediaControlEvent as E;
    Some(match ev {
        E::Play => Cmd::Play,
        E::Pause | E::Stop => Cmd::Pause,
        E::Toggle => Cmd::Toggle,
        E::Next => Cmd::Next,
        E::Previous => Cmd::Prev,
        E::Seek(dir) => Cmd::SeekBy(sign(dir) * DEFAULT_SEEK_SECS),
        E::SeekBy(dir, by) => Cmd::SeekBy(sign(dir) * by.as_secs_f32()),
        E::SetPosition(MediaPosition(at)) => Cmd::Seek(at.as_secs_f32()),
        E::SetVolume(v) => Cmd::SetVolume(v as f32),
        _ => return None,
    })
}

fn sign(dir: SeekDirection) -> f32 {
    match dir {
        SeekDirection::Forward => 1.0,
        SeekDirection::Backward => -1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a session D-Bus; run with --ignored"]
    fn mpris_publishes_metadata() {
        let mut m = Media::new(|| {}, None).expect("MPRIS backend failed to start");
        m.set_metadata("Test", "Artist", "Album", None, Some(200.0));
        m.set_playback(true, 10.0);
        std::thread::sleep(Duration::from_secs(1));
        let out = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--dest=org.mpris.MediaPlayer2.serafin",
                "/org/mpris/MediaPlayer2",
                "org.freedesktop.DBus.Properties.Get",
                "string:org.mpris.MediaPlayer2.Player",
                "string:Metadata",
            ])
            .output()
            .expect("dbus-send not available");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("\"Test\""), "title not found in Metadata:\n{text}");
    }
}
