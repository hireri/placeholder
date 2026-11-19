//! Music videos: an `ffmpeg` child process decodes the daemon's video-only stream (`/video/{id}`, WebM/MP4 from YouTube's SABR) into RGBA
//! frames. A frame is shown once the audio engine's playhead reaches its timestamp, so audio stays the master clock and
//! video needs no sync of its own; while audio is paused nothing is pulled and backpressure stalls the stream.

use crate::app::App;
use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, sync_channel, Receiver, TryRecvError};
use std::time::Duration;

/// How far behind the playhead a frame may be and still be shown; older ones are dropped.
const MAX_LATE: f32 = 0.15;
/// Seconds a stream may go without its first frame before it is reopened; a first open (fresh extraction) can take ~10.
const STALL: f64 = 20.;
/// How far past the newest decoded frame a seek may land and still be caught up by draining instead of reopening.
const AHEAD: f32 = 10.;
/// Seconds a forward seek may take to drain before the stream is reopened instead.
const CATCH_UP: f64 = 0.4;

/// A decoded RGBA frame and its presentation time.
struct Raw {
    pts: f32,
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

/// The ffmpeg process and its frame queue; the queue is bounded, so a full one stalls ffmpeg and with it the HTTP stream.
struct Stream {
    child: Child,
    rx: Receiver<Raw>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct Video {
    pub id: String,
    height: u32,
    stream: Option<Stream>,
    /// Set when the stream failed (no ffmpeg, no decoder, network); the thumbnail shows instead and it is not retried.
    error: Option<String>,
    /// The stream could not even be started (no ffmpeg, no URL); retrying is pointless.
    fatal: bool,
    /// `ctx` time the pipeline was opened; restarts wait for it to warm up.
    opened: f64,
    ended: bool,
    /// Next decoded frame, waiting for the playhead: (pts seconds, image).
    next: Option<(f32, ColorImage)>,
    /// Timestamp of the frame on screen.
    shown: Option<f32>,
    tex: Option<TextureHandle>,
    pub aspect: Option<f32>,
    /// The player's seek count this stream was opened for.
    seeks: u32,
    /// Drain deadline (`ctx` time) for a forward seek; past it the stream is reopened at the playhead.
    catch_up: Option<f64>,
}

impl Video {
    fn open(id: &str, at: f32, now: f64, height: u32, seeks: u32) -> Video {
        let mut v = Video { id: id.to_string(), height, stream: None, error: None, fatal: false, opened: now, ended: false, next: None, shown: None, tex: None, aspect: None, seeks, catch_up: None };
        match spawn(id, at, height) {
            Ok(st) => v.stream = Some(st),
            Err(e) => {
                eprintln!("video: {e}");
                v.error = Some(e);
                v.fatal = true;
            }
        }
        v
    }

    /// Pulls frames due at `pos` and uploads the latest; returns the texture (once one was shown) and seconds until the next frame.
    fn frame(&mut self, ctx: &Context, pos: f32) -> (Option<TextureHandle>, Option<f32>) {
        let Some(st) = &self.stream else { return (self.tex.clone(), None) };
        if self.error.is_some() {
            self.stream = None;
            return (self.tex.clone(), None);
        }
        let mut due = None;
        let mut gone = false;
        // Bounded so catching up after a stall can't stall the UI.
        for _ in 0..256 {
            let (pts, img) = match self.next.take() {
                Some(n) => n,
                None => match st.rx.try_recv() {
                    // Stale frames (a reopened stream decodes from the keyframe before the playhead) are skipped unconverted.
                    Ok(r) if r.pts < pos - MAX_LATE => continue,
                    Ok(r) => (r.pts, ColorImage::from_rgba_unmultiplied([r.w, r.h], &r.rgba)),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        gone = true;
                        break;
                    }
                },
            };
            if pts > pos + 0.01 {
                self.next = Some((pts, img));
                break;
            }
            if pts >= pos - MAX_LATE {
                due = Some((pts, img));
            }
        }
        if let Some((pts, img)) = due {
            self.catch_up = None;
            self.aspect = Some(img.width() as f32 / img.height().max(1) as f32);
            self.shown = Some(pts);
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("video", img, TextureOptions::LINEAR)),
            }
        }
        if gone {
            if self.shown.is_none() && self.next.is_none() {
                eprintln!("video {}: ffmpeg exited without a frame", self.id);
                self.error = Some("ffmpeg exited without a frame".into());
            } else {
                self.ended = true;
            }
        }
        (self.tex.clone(), self.next.as_ref().map(|(pts, _)| pts - pos))
    }

    /// Takes a seek just past the newest decoded frame by draining the stream, which is cheaper than reopening it.
    fn forward(&mut self, pos: f32, now: f64, seeks: u32) -> bool {
        let latest = self.next.as_ref().map(|n| n.0).or(self.shown);
        let ok = self.stream.is_some() && self.error.is_none() && !self.ended && latest.is_some_and(|t| pos > t && pos <= t + AHEAD);
        if ok {
            self.seeks = seeks;
            self.catch_up = Some(now + CATCH_UP);
        }
        ok
    }

    /// No frame is available for the playhead: the stream is opening, catching up after a seek, starved, or about to be reopened.
    pub fn waiting(&self, pos: f32, dur: f32) -> bool {
        !self.fatal && pos < dur - 1.5 && self.next.is_none() && self.shown.is_none_or(|s| pos - s > 0.3)
    }

    /// The stream no longer covers the playhead, or never delivered a frame (the daemon can hang a stream after its header);
    /// reopening at `pos` is cheaper than seeking a live HTTP stream.
    fn lost(&self, pos: f32, now: f64, dur: f32) -> bool {
        if self.error.is_some() || self.ended {
            // A stream that died before the song's end (network, daemon) is reopened at the playhead, after a pause so a persistent failure doesn't hammer the daemon.
            return !self.fatal && pos < dur - 1.5 && now - self.opened > if self.shown.is_some() { 2. } else { 5. };
        }
        if self.catch_up.is_some() || now - self.opened < 4. {
            return false;
        }
        if self.shown.is_none() && self.next.is_none() {
            return now - self.opened > STALL;
        }
        // Newest decoded timestamp; far behind the playhead means a forward seek, which decoding would only crawl through.
        let latest = self.next.as_ref().map(|n| n.0).or(self.shown);
        let behind = latest.is_some_and(|t| pos > t + 3.);
        let ahead = self.shown.is_some_and(|s| pos < s - 1.) || self.next.as_ref().is_some_and(|(pts, _)| *pts > pos + 3.);
        behind || ahead
    }
}

/// Starts ffmpeg on the daemon's stream. `-seekable 0`: the daemon has no Range support, and a demuxer that finds the source
/// seekable may try to seek it and kill the stream. `-copyts` keeps the stream's absolute timestamps. `showinfo` reports each
/// frame's pts and size on stderr, in the order the raw frames follow on stdout.
fn spawn(id: &str, at: f32, height: u32) -> Result<Stream, String> {
    let (url, token) = crate::backend::video_url(id, (at.max(0.) * 1000.) as u64, height)?;
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "info", "-fflags", "nobuffer", "-probesize", "65536", "-analyzeduration", "0", "-seekable", "0", "-copyts", "-headers"])
        .arg(format!("X-Metrolist-Token: {token}\r\n"))
        .arg("-i")
        .arg(url)
        .args(["-an", "-vf", "showinfo,format=rgba", "-fps_mode", "passthrough", "-f", "rawvideo", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start ffmpeg: {e}"))?;
    let (mut out, err) = (child.stdout.take().ok_or("no stdout")?, child.stderr.take().ok_or("no stderr")?);
    let (info_tx, info_rx) = channel::<(f32, usize, usize)>();
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if line.contains("showinfo") && line.contains("pts_time:") {
                let field = |k: &str| line.split_whitespace().find_map(|t| t.strip_prefix(k));
                let size = field("s:").and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                let pts = field("pts_time:").and_then(|t| t.parse().ok()).unwrap_or(f32::NAN);
                if let Some((w, h)) = size {
                    let _ = info_tx.send((pts, w, h));
                }
            } else if line.contains("rror") {
                eprintln!("video ffmpeg: {line}");
            }
        }
    });
    let (tx, rx) = sync_channel(3);
    std::thread::spawn(move || {
        while let Ok((pts, w, h)) = info_rx.recv() {
            let mut rgba = vec![0u8; w * h * 4];
            if out.read_exact(&mut rgba).is_err() || (pts.is_finite() && tx.send(Raw { pts, w, h, rgba }).is_err()) {
                break;
            }
        }
    });
    Ok(Stream { child, rx })
}

impl App {
    /// The video frame for YouTube id `want` at the current playhead, with its aspect; `None` (and the pipeline torn down)
    /// when no video is wanted, while the first frame decodes, or when the stream failed.
    pub fn video_frame(&mut self, ctx: &Context, want: Option<&str>, height: u32) -> Option<(TextureHandle, f32)> {
        let Some(id) = want else {
            self.video = None;
            return None;
        };
        let (pos, now, seeks, dur) = (self.pl.heard(), ctx.input(|i| i.time), self.pl.seeks, self.pl.dur());
        let drain = self.video.as_mut().is_some_and(|v| v.id == id && v.height == height && v.seeks != seeks && v.forward(pos, now, seeks));
        let reopen = !drain && self.video.as_ref().is_none_or(|v| v.id != id || v.height != height || v.seeks != seeks || v.lost(pos, now, dur) || v.catch_up.is_some_and(|d| now > d));
        if reopen {
            let mut v = Video::open(id, pos, now, height, seeks);
            // Keep the last frame on screen until the new stream catches up.
            if let Some(old) = self.video.as_ref().filter(|old| old.id == id) {
                v.tex = old.tex.clone();
                v.aspect = old.aspect;
            }
            self.video = Some(v);
        }
        let v = self.video.as_mut()?;
        let (tex, wait) = v.frame(ctx, pos);
        if v.catch_up.is_some() {
            ctx.request_repaint();
        } else if self.pl.playing && v.error.is_none() {
            ctx.request_repaint_after(Duration::from_secs_f32(wait.unwrap_or(0.04).clamp(0.005, 0.1)));
        }
        tex.zip(v.aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the daemon and network; run with --ignored"]
    fn live_video_decodes_from_start_point() {
        let st = spawn("dQw4w9WgXcQ", 30., 480).expect("spawn");
        let r = st.rx.recv_timeout(Duration::from_secs(40)).expect("no frame within 40s");
        drop(st);
        crate::backend::stop();
        // Streams start at the keyframe before the requested time, with absolute timestamps.
        assert!((20. ..=30.5).contains(&r.pts), "pts {}", r.pts);
        assert!(r.w > r.h && r.h >= 240, "{}x{}", r.w, r.h);
    }
}
