//! Audio visualizer: spectrum levels analysed from the output tap, and the styles that draw them.

use crate::audio::{self, TAP};
use eframe::egui::{self, pos2, vec2, Color32, Context, Mesh, Painter, Rect, Shape, Stroke};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::OnceLock;

/// Style names for the settings picker; `Opts::viz_style` indexes this (0 = off).
pub const STYLES: [&str; 7] = ["Off", "Bars", "Mirror", "Wave", "Dots", "Ring", "MilkDrop"];

/// Index of the MilkDrop style, drawn by projectM instead of `draw`.
pub const MILKDROP: u8 = 6;

pub const BANDS: usize = 48;
const LO_HZ: f32 = 40.;
const HI_HZ: f32 = 16_000.;
const DB_FLOOR: f32 = -66.;
/// Treble carries less energy than bass; this much boost across the range keeps the right side of the display alive.
const TILT_DB: f32 = 15.;

/// Log-spaced band levels (0..1), slow-falling peak caps, the smoothed low end, and beat onsets for the pulsing effects.
#[derive(Clone, Copy)]
pub struct Levels {
    pub bands: [f32; BANDS],
    pub peaks: [f32; BANDS],
    pub bass: f32,
    /// How far the low end is above its own slow average (0..1): follows every hit, where `bass` barely moves in dense music.
    pub punch: f32,
    /// 1 on a beat, decaying in ~0.15s.
    pub kick: f32,
    last_beat: f64,
    /// When these levels were last analysed.
    t: f64,
    avg: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Levels { bands: [0.; BANDS], peaks: [0.; BANDS], bass: 0., punch: 0., kick: 0., last_beat: f64::NEG_INFINITY, t: f64::NEG_INFINITY, avg: 0. }
    }
}

impl Levels {
    /// Band level at `f` in 0..=1 across the range, interpolated.
    fn at(&self, f: f32) -> f32 {
        let p = f.clamp(0., 1.) * (BANDS - 1) as f32;
        let i = p as usize;
        let j = (i + 1).min(BANDS - 1);
        self.bands[i] + (self.bands[j] - self.bands[i]) * (p - i as f32)
    }

    fn peak_at(&self, f: f32) -> f32 {
        self.peaks[(f.clamp(0., 1.) * (BANDS - 1) as f32).round() as usize]
    }

    /// 0 in silence, rising to 1 once there is a little signal; light effects fade with it so they never sit there idle.
    pub fn gate(&self) -> f32 {
        let e = self.bands.iter().sum::<f32>() / BANDS as f32;
        (e * 6.).min(1.)
    }

    /// Still moving, so callers keep repainting.
    pub fn active(&self) -> bool {
        self.kick > 0.02 || self.bands.iter().any(|v| *v > 0.02)
    }
}

/// Hann window and FFT twiddles, built once.
fn tables() -> &'static (Vec<f32>, Vec<(f32, f32)>) {
    static T: OnceLock<(Vec<f32>, Vec<(f32, f32)>)> = OnceLock::new();
    T.get_or_init(|| {
        let win = (0..TAP).map(|i| 0.5 - 0.5 * (TAU * i as f32 / TAP as f32).cos()).collect();
        let tw = (0..TAP / 2).map(|k| { let a = -TAU * k as f32 / TAP as f32; (a.cos(), a.sin()) }).collect();
        (win, tw)
    })
}

/// In-place radix-2 FFT of `TAP` points.
fn fft(re: &mut [f32; TAP], im: &mut [f32; TAP]) {
    let tw = &tables().1;
    let bits = TAP.trailing_zeros();
    for i in 0..TAP {
        let j = (i.reverse_bits() >> (usize::BITS - bits)) as usize;
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= TAP {
        let (half, step) = (len / 2, TAP / len);
        for start in (0..TAP).step_by(len) {
            for k in 0..half {
                let (wr, wi) = tw[k * step];
                let (a, b) = (start + k, start + k + half);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len *= 2;
    }
}

/// Spectrum of the latest output samples as `BANDS` levels.
fn analyse(s: &[f32; TAP], rate: f32) -> [f32; BANDS] {
    let win = &tables().0;
    let (mut re, mut im) = ([0.; TAP], [0.; TAP]);
    for i in 0..TAP {
        re[i] = s[i] * win[i];
    }
    fft(&mut re, &mut im);
    let mag: Vec<f32> = (0..TAP / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() / (TAP as f32 / 4.)).collect();
    let hz = rate / TAP as f32;
    let hi = HI_HZ.min(rate / 2. - hz);
    let edge = |i: usize| LO_HZ * (hi / LO_HZ).powf(i as f32 / BANDS as f32) / hz;
    let mut out = [0.; BANDS];
    for (i, o) in out.iter_mut().enumerate() {
        let (a, b) = (edge(i), edge(i + 1));
        // Bass bands are narrower than one bin: interpolate between bins instead of repeating one.
        let m = if b - a < 1. {
            let p = (a + b) / 2.;
            let k = (p as usize).min(mag.len() - 2);
            mag[k] + (mag[k + 1] - mag[k]) * (p - k as f32)
        } else {
            mag[a as usize..(b as usize + 1).min(mag.len())].iter().fold(0., |m: f32, v| m.max(*v))
        };
        let db = 20. * (m + 1e-9).log10() + TILT_DB * i as f32 / BANDS as f32;
        *o = ((db - DB_FLOOR) / -DB_FLOOR).clamp(0., 1.);
    }
    out
}

/// The levels for this frame. Analysed once per frame however many things draw them, with a fast attack and a slow
/// release; with nothing playing they decay to zero. Callers keep repainting (see [`repaint`]) while `active()`.
pub fn levels(ctx: &Context, engine: &audio::Engine) -> Levels {
    let id = egui::Id::unique("viz-levels");
    let (mut lv, pass) = ctx.data(|d| d.get_temp::<(Levels, u64)>(id)).unwrap_or_default();
    let n = ctx.cumulative_pass_nr();
    if pass == n {
        return lv;
    }
    let now = ctx.input(|i| i.time);
    // Extra repaints (mouse moves, hovers) reuse the last analysis, so nothing runs faster than the cap.
    if now - lv.t < interval(ctx) as f64 * 0.9 {
        ctx.data_mut(|d| d.insert_temp(id, (lv, n)));
        return lv;
    }
    let mut buf = [0.; TAP];
    let raw = if engine.tap(&mut buf) {
        analyse(&buf, engine.out_rate() as f32)
    } else {
        [0.; BANDS]
    };
    let dt = ((now - lv.t) as f32).min(0.1);
    lv.t = now;
    let (up, down) = (1. - (-dt / 0.04).exp(), 1. - (-dt / 0.18).exp());
    for i in 0..BANDS {
        lv.bands[i] += (raw[i] - lv.bands[i]) * if raw[i] > lv.bands[i] { up } else { down };
        lv.peaks[i] = (lv.peaks[i] - dt * 0.55).max(lv.bands[i]);
    }
    let bass = lv.bands[..4].iter().sum::<f32>() / 4.;
    lv.bass += ((bass * 1.3).min(1.) - lv.bass) * if bass > lv.bass { 0.5 } else { 0.2 };
    let hit = ((bass - lv.avg) * 4.5).clamp(0., 1.);
    lv.punch += (hit - lv.punch) * if hit > lv.punch { 0.7 } else { 0.18 };
    // A beat is the low end jumping clear of its own slow average.
    if bass > lv.avg * 1.25 + 0.05 && bass > 0.3 && now - lv.last_beat > 0.24 {
        lv.kick = 1.;
        lv.last_beat = now;
    }
    lv.avg += (bass - lv.avg) * (1. - (-dt / 0.6).exp());
    lv.kick *= (-dt / 0.15).exp();
    ctx.data_mut(|d| d.insert_temp(id, (lv, n)));
    lv
}

/// Frame rate cap for the visualizers (Settings), and what applies while the window isn't focused.
static FPS: AtomicU32 = AtomicU32::new(60);

pub fn set_fps(fps: u8) {
    // 0 = uncapped: 240 is above any display, so the window's own vsync is the real limit
    FPS.store(if fps == 0 { 240 } else { fps.clamp(15, 120) as u32 }, Relaxed);
}

/// Frames per second to run at: the cap, and a much lower rate when nobody is looking at the window.
pub fn fps(ctx: &Context, unfocused: u32) -> u32 {
    let cap = FPS.load(Relaxed);
    if ctx.input(|i| i.focused) { cap } else { cap.min(unfocused) }
}

fn interval(ctx: &Context) -> f32 {
    1. / fps(ctx, 15) as f32
}

/// Frame pacing for anything animated by the levels: repaints at the capped rate while they move.
pub fn repaint(ctx: &Context, lv: &Levels) {
    if lv.active() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(interval(ctx)));
    }
}

fn falloff(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    1. - t * t * (3. - 2. * t)
}

/// Vertical gradient bar from `top` to `bot`, eased across a few segments instead of one linear ramp.
fn vbar(m: &mut Mesh, r: Rect, top: Color32, bot: Color32) {
    const SEG: usize = 4;
    let base = m.vertices.len() as u32;
    for s in 0..=SEG {
        let f = s as f32 / SEG as f32;
        let k = f * f * (3. - 2. * f);
        let c = top.lerp_to_gamma(bot, k);
        let y = r.min.y + r.height() * f;
        m.colored_vertex(pos2(r.min.x, y), c);
        m.colored_vertex(pos2(r.max.x, y), c);
    }
    for s in 0..SEG as u32 {
        let i = base + s * 2;
        m.indices.extend([i, i + 1, i + 3, i, i + 3, i + 2]);
    }
}

/// Soft radial light: `col` at the centre easing to nothing at `r`.
fn glow(p: &Painter, c: egui::Pos2, (rx, ry): (f32, f32), col: Color32, a: f32) {
    const N: usize = 32;
    const RINGS: usize = 6;
    let mut m = Mesh::default();
    m.colored_vertex(c, col.gamma_multiply(a));
    for ring in 1..=RINGS {
        let t = ring as f32 / RINGS as f32;
        for i in 0..N {
            let ang = i as f32 / N as f32 * TAU;
            m.colored_vertex(c + vec2(ang.cos() * rx, ang.sin() * ry) * t, col.gamma_multiply(a * falloff(t)));
        }
    }
    let n = N as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        m.indices.extend([0, 1 + i, 1 + j]);
        for ring in 0..RINGS as u32 - 1 {
            let (a0, b0) = (1 + ring * n + i, 1 + ring * n + j);
            m.indices.extend([a0, a0 + n, b0 + n, a0, b0 + n, b0]);
        }
    }
    p.add(m);
}

/// Soft glow behind the app logo that follows the beat. Returns the logo's scale and how far to push its colour
/// towards white. 
pub fn pulse(p: &Painter, c: egui::Pos2, col: Color32, lv: &Levels) -> (f32, f32) {
    let e = (0.1 + 0.3 * lv.bass + 0.7 * lv.punch + 0.4 * lv.kick).min(1.);
    // The logo is wider than tall, so the glow is an oval: 26px at most sideways (the logo sits 30px from the window edge, which clips anything wider), 20px up and down (the titlebar is 52px).
    let (rx, ry, a) = (18. + 8. * e, 12. + 8. * e, lv.gate() * (0.08 + 0.4 * e));
    if crate::viz_gl::usable(p.ctx()) {
        if a > 0.003 {
            crate::viz_gl::paint(p, Rect::from_center_size(c, vec2(2. * rx, 2. * ry)), gl_params(crate::viz_gl::GLOW, lv, col.gamma_multiply(a)));
        }
    } else {
        glow(p, c, (rx, ry), col, a);
    }
    (1. + 0.06 * lv.bass + 0.1 * lv.punch + 0.16 * lv.kick, 0.5 * lv.kick.max(lv.punch * 0.6))
}

fn gl_params(mode: i32, lv: &Levels, col: Color32) -> crate::viz_gl::Params {
    crate::viz_gl::Params { mode, bands: lv.bands, peaks: lv.peaks, col, p: [lv.bass, lv.kick, lv.gate()], raw_tex: None }
}

/// Draws `style` (1..) filling `area`, on the GPU when the shader is available. `col` carries the opacity.
pub fn draw(p: &Painter, style: u8, area: Rect, lv: &Levels, col: Color32) {
    if !lv.active() {
        return;
    }
    if crate::viz_gl::usable(p.ctx()) {
        crate::viz_gl::paint(p, area, gl_params(style as i32, lv, col));
        return;
    }
    let (x0, w, base, sh) = (area.min.x, area.width(), area.max.y, area.height());
    let n = ((w / 9.) as usize).clamp(18, 40);
    if !lv.active() {
        return;
    }
    match style {
        1 | 2 => {
            let gap = (w / n as f32 * 0.22).max(2.);
            let bw = (w - gap * (n - 1) as f32) / n as f32;
            let mut m = Mesh::default();
            for i in 0..n {
                let f = i as f32 / (n - 1) as f32;
                let v = lv.at(f).max(0.05 * lv.gate());
                let x = x0 + i as f32 * (bw + gap);
                if style == 1 {
                    vbar(&mut m, Rect::from_min_max(pos2(x, base - v * sh), pos2(x + bw, base)), col, col.gamma_multiply(0.62));
                    // Peak cap, falling slowly behind the bar.
                    let c = base - lv.peak_at(f).max(0.05) * sh - 4.;
                    vbar(&mut m, Rect::from_min_max(pos2(x, c - 2.), pos2(x + bw, c)), col.gamma_multiply(0.8), col.gamma_multiply(0.8));
                } else {
                    let mid = base - sh / 2.;
                    vbar(&mut m, Rect::from_min_max(pos2(x, mid - v * sh / 2.), pos2(x + bw, mid)), col, col.gamma_multiply(0.75));
                    vbar(&mut m, Rect::from_min_max(pos2(x, mid), pos2(x + bw, mid + v * sh / 2.)), col.gamma_multiply(0.75), col.gamma_multiply(0.4));
                }
            }
            p.add(m);
        }
        3 => {
            let pts: Vec<_> = (0..=n * 2).map(|i| { let f = i as f32 / (n * 2) as f32; pos2(x0 + w * f, base - lv.at(f) * sh) }).collect();
            let mut m = Mesh::default();
            for (i, pt) in pts.iter().enumerate() {
                m.colored_vertex(*pt, col.gamma_multiply(0.4));
                m.colored_vertex(pos2(pt.x, base), Color32::TRANSPARENT);
                if i > 0 {
                    let b = (i * 2) as u32;
                    m.indices.extend([b - 2, b - 1, b + 1, b - 2, b + 1, b]);
                }
            }
            p.add(m);
            p.add(Shape::line(pts, Stroke::new(2.2, col)));
        }
        4 => {
            let cols = n / 2;
            let rows = ((sh / (w / cols as f32)).round() as usize).clamp(5, 14);
            let d = (w / cols as f32).min(sh / rows as f32);
            for i in 0..cols {
                let lit = (lv.at(i as f32 / (cols - 1) as f32) * rows as f32).round() as usize;
                for j in 0..rows {
                    let c = if j < lit { col.gamma_multiply(0.55 + 0.45 * j as f32 / rows as f32) } else { col.gamma_multiply(0.13) };
                    p.circle_filled(pos2(x0 + (i as f32 + 0.5) * w / cols as f32, base - (j as f32 + 0.5) * d), d * 0.31, c);
                }
            }
        }
        5 => {
            let (c, m) = (area.center(), area.width().min(area.height()));
            let (r0, len) = (m * 0.26 + lv.bass * m * 0.03, m * 0.22);
            glow(p, c, (r0 * 1.2, r0 * 1.2), col, 0.12 + 0.25 * lv.bass);
            for i in 0..90 {
                let a = i as f32 / 90. * TAU - PI / 2.;
                let l = (0.07 + lv.at((i as f32 / 90. * 2. - 1.).abs())) * len;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * r0, c + d * (r0 + l)], Stroke::new(2.6, col));
            }
        }
        _ => {}
    }
}

pub const WAVE_N: usize = 96;

/// A waveform drawn as the progress bar: `env` is the loudness heard so far per slot (0 = not heard yet, shown as a
/// seeded stand-in shape since a stream's full waveform isn't known up front), the played part is solid, and the bars
/// at the playhead jump with the live bass.
pub fn waveform(p: &Painter, r: Rect, frac: f32, lv: &Levels, env: &[f32], seed: u64, played: Color32, rest: Color32) {
    let n = WAVE_N;
    let (step, mid) = (r.width() / n as f32, r.center().y);
    let mut s = seed | 1;
    for i in 0..n {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let guess = 0.22 + 0.45 * ((s % 1000) as f32 / 1000.).powf(0.8);
        let known = env[i] > 0.01;
        let f = (i as f32 + 0.5) / n as f32;
        let base = if known { 0.18 + 0.82 * env[i] } else { guess };
        let near = (1. - ((f - frac).abs() * n as f32 / 4.)).max(0.);
        let h = (base * r.height() * (0.9 + 0.35 * lv.bass * near * near)).clamp(3., r.height());
        let col = if f <= frac { played } else if known { rest.gamma_multiply(1.4) } else { rest };
        let x = r.min.x + i as f32 * step;
        let bar = Rect::from_min_max(pos2(x + step * 0.2, mid - h / 2.), pos2(x + step * 0.8, mid + h / 2.));
        p.rect_filled(bar, (bar.width() / 2.).max(1.), col);
    }
}


/// The latest mono output samples for projectM; zeros when nothing is playing.
pub fn pcm(engine: &audio::Engine) -> Vec<f32> {
    let mut buf = [0.; TAP];
    engine.tap(&mut buf);
    buf[TAP - 1024..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lights_the_band_at_its_pitch() {
        let rate = 48_000.;
        let mut s = [0.; TAP];
        for (i, v) in s.iter_mut().enumerate() {
            *v = 0.5 * (TAU * 1000. * i as f32 / rate).sin();
        }
        let b = analyse(&s, rate);
        let peak = (0..BANDS).max_by(|a, c| b[*a].total_cmp(&b[*c])).unwrap();
        let hz = LO_HZ * (HI_HZ / LO_HZ).powf((peak as f32 + 0.5) / BANDS as f32);
        assert!((hz / 1000.).log2().abs() < 0.3, "peaked at {hz} Hz");
        assert!(b[peak] > 0.6 && b[0] < b[peak] * 0.5);
    }
}
