//! Music videos: GStreamer decodes the daemon's video-only stream (`/video/{id}`, WebM/MP4 from YouTube's SABR) into RGBA
//! frames. A frame is shown once the audio engine's playhead reaches its timestamp, so audio stays the master clock and
//! video needs no sync of its own; while audio is paused nothing is pulled and backpressure stalls the stream.

use crate::app::App;
use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use std::time::Duration;

/// Decoded height asked of the daemon; the player's floating rect is at most ~720px wide.
const MAX_HEIGHT: u32 = 480;
/// How far behind the playhead a frame may be and still be shown; older ones are dropped.
const MAX_LATE: f32 = 0.15;

pub struct Video {
    pub id: String,
    pipe: Option<(gst::Pipeline, AppSink)>,
    /// Set when the stream failed (no decoder, network); the thumbnail shows instead and it is not retried.
    error: Option<String>,
    /// `ctx` time the pipeline was opened; restarts wait for it to warm up.
    opened: f64,
    ended: bool,
    /// Next decoded frame, waiting for the playhead: (pts seconds, image).
    next: Option<(f32, ColorImage)>,
    /// Timestamp of the frame on screen.
    shown: Option<f32>,
    tex: Option<TextureHandle>,
    pub aspect: Option<f32>,
    /// Playhead and `ctx` time seen by the last `video_frame`, to spot seeks by how far the playhead jumped.
    last_pos: f32,
    last_ui: f64,
}

impl Video {
    fn open(id: &str, at: f32, now: f64) -> Video {
        let mut v = Video { id: id.to_string(), pipe: None, error: None, opened: now, ended: false, next: None, shown: None, tex: None, aspect: None, last_pos: at, last_ui: now };
        match pipeline(id, at) {
            Ok(p) => v.pipe = Some(p),
            Err(e) => {
                eprintln!("video: {e}");
                v.error = Some(e);
            }
        }
        v
    }

    /// Pulls frames due at `pos` and uploads the latest; returns the texture (once one was shown) and seconds until the next frame.
    fn frame(&mut self, ctx: &Context, pos: f32) -> (Option<TextureHandle>, Option<f32>) {
        let Some((pipe, sink)) = &self.pipe else { return (None, None) };
        if let Some(bus) = pipe.bus() {
            while let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos]) {
                match msg.view() {
                    gst::MessageView::Error(e) => {
                        let e = e.error().to_string();
                        eprintln!("video {}: {e}", self.id);
                        self.error = Some(e);
                    }
                    _ => self.ended = true,
                }
            }
        }
        if self.error.is_some() {
            self.pipe = None;
            return (None, None);
        }
        let mut due = None;
        // Bounded so catching up after a stall can't stall the UI.
        for _ in 0..64 {
            let (pts, img) = match self.next.take() {
                Some(n) => n,
                None => {
                    let Some(s) = sink.try_pull_sample(gst::ClockTime::ZERO) else { break };
                    match sample_pts(&s) {
                        // Stale frames (a reopened stream decodes from the keyframe before the playhead) are skipped unconverted.
                        Some(pts) if pts < pos - MAX_LATE => continue,
                        Some(pts) => match to_image(&s) {
                            Some(img) => (pts, img),
                            None => continue,
                        },
                        None => continue,
                    }
                }
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
            self.aspect = Some(img.width() as f32 / img.height().max(1) as f32);
            self.shown = Some(pts);
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("video", img, TextureOptions::LINEAR)),
            }
        }
        (self.tex.clone(), self.next.as_ref().map(|(pts, _)| pts - pos))
    }

    /// The stream no longer covers the playhead (a seek either way); reopening at `pos` is cheaper than seeking a live HTTP stream.
    fn lost(&self, pos: f32, now: f64) -> bool {
        if self.error.is_some() || self.ended || now - self.opened < 4. {
            return false;
        }
        // Newest decoded timestamp; far behind the playhead means a forward seek, which decoding would only crawl through.
        let latest = self.next.as_ref().map(|n| n.0).or(self.shown);
        let behind = latest.is_some_and(|t| pos > t + 3.);
        let ahead = self.shown.is_some_and(|s| pos < s - 1.) || self.next.as_ref().is_some_and(|(pts, _)| *pts > pos + 3.);
        behind || ahead
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        if let Some((p, _)) = &self.pipe {
            let _ = p.set_state(gst::State::Null);
        }
    }
}

fn pipeline(id: &str, at: f32) -> Result<(gst::Pipeline, AppSink), String> {
    gst::init().map_err(|e| e.to_string())?;
    let (url, token) = crate::backend::video_url(id, (at.max(0.) * 1000.) as u64, MAX_HEIGHT)?;
    let desc = "souphttpsrc name=src ! decodebin ! videoconvert ! appsink name=sink caps=video/x-raw,format=RGBA max-buffers=3 drop=false sync=false";
    let pipe = gst::parse::launch(desc).map_err(|e| e.to_string())?.downcast::<gst::Pipeline>().map_err(|_| "not a pipeline")?;
    let src = pipe.by_name("src").ok_or("no source")?;
    src.set_property("location", url);
    src.set_property("extra-headers", gst::Structure::builder("headers").field("X-Metrolist-Token", token).build());
    let sink = pipe.by_name("sink").ok_or("no sink")?.downcast::<AppSink>().map_err(|_| "not an appsink")?;
    pipe.set_state(gst::State::Playing).map_err(|e| format!("cannot start video: {e}"))?;
    Ok((pipe, sink))
}

/// Presentation timestamp in seconds, read without touching the pixels.
fn sample_pts(s: &gst::Sample) -> Option<f32> {
    Some((s.buffer()?.pts()?.nseconds() as f64 / 1e9) as f32)
}

/// RGBA sample to an egui image, dropping the row padding GStreamer may add.
fn to_image(s: &gst::Sample) -> Option<ColorImage> {
    let info = gstreamer_video::VideoInfo::from_caps(s.caps()?).ok()?;
    let map = s.buffer()?.map_readable().ok()?;
    let (w, h, stride) = (info.width() as usize, info.height() as usize, info.stride()[0] as usize);
    let mut px = Vec::with_capacity(w * h * 4);
    for row in map.as_slice().chunks(stride).take(h) {
        px.extend_from_slice(&row[..w * 4]);
    }
    Some(ColorImage::from_rgba_unmultiplied([w, h], &px))
}

impl App {
    /// The video frame for YouTube id `want` at the current playhead, with its aspect; `None` (and the pipeline torn down)
    /// when no video is wanted, while the first frame decodes, or when the stream failed.
    pub fn video_frame(&mut self, ctx: &Context, want: Option<&str>) -> Option<(TextureHandle, f32)> {
        let Some(id) = want else {
            self.video = None;
            return None;
        };
        let (pos, now) = (self.pl.pos, ctx.input(|i| i.time));
        // A seek shows as a playhead that moved more than the time elapsed since the last call.
        let seeked = self.video.as_ref().is_some_and(|v| {
            let expected = v.last_pos + if self.pl.playing { (now - v.last_ui) as f32 } else { 0. };
            (pos - expected).abs() > 1.
        });
        let reopen = self.video.as_ref().is_none_or(|v| v.id != id || seeked || v.lost(pos, now));
        if reopen {
            let mut v = Video::open(id, pos, now);
            // Keep the last frame on screen until the new stream catches up.
            if let Some(old) = self.video.as_ref().filter(|old| old.id == id) {
                v.tex = old.tex.clone();
                v.aspect = old.aspect;
            }
            self.video = Some(v);
        }
        let v = self.video.as_mut()?;
        v.last_pos = pos;
        v.last_ui = now;
        let (tex, wait) = v.frame(ctx, pos);
        if self.pl.playing && v.error.is_none() {
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
        let (pipe, sink) = pipeline("dQw4w9WgXcQ", 30.).expect("pipeline");
        let s = sink.try_pull_sample(gst::ClockTime::from_seconds(40)).expect("no frame within 40s");
        let pts = sample_pts(&s).expect("pts");
        let img = to_image(&s).expect("frame");
        let _ = pipe.set_state(gst::State::Null);
        crate::backend::stop();
        // Streams start at the keyframe before the requested time, with absolute timestamps.
        assert!((20. ..=30.5).contains(&pts), "pts {pts}");
        assert!(img.width() > img.height() && img.height() >= 240, "{:?}", img.size);
    }
}
