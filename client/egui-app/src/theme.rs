//! Tonal theme derived from one hue; same OKLCH tokens as design/desktop-design.html.

use eframe::egui::Color32;

#[derive(Clone, Copy)]
pub struct Theme {
    pub hue: f32,
    /// window, sidebar, panel, hover
    pub s0: Color32,
    pub s1: Color32,
    pub s2: Color32,
    pub s3: Color32,
    /// text: primary, muted, faint
    pub tx: Color32,
    pub tm: Color32,
    pub tf: Color32,
    /// accent, text on accent, accent container
    pub ax: Color32,
    pub on_ax: Color32,
    pub axc: Color32,
}

impl Theme {
    pub fn new(hue: f32, ac: f32, nc: f32, dark: bool) -> Self {
        let c = |l, ch| oklch(l, ch, hue);
        let (s, tx, tm, tf, ax, on_ax, axc) = if dark {
            ([0.17, 0.205, 0.25, 0.30], c(0.96, 0.008), c(0.72, 0.02), c(0.56, 0.02), c(0.78, ac), c(0.2, 0.03), c(0.32, ac * 0.55))
        } else {
            ([0.975, 0.95, 0.925, 0.89], c(0.22, 0.02), c(0.46, 0.02), c(0.6, 0.015), c(0.5, ac), c(0.98, 0.01), c(0.9, ac * 0.5))
        };
        Theme { hue, tx, tm, tf, ax, on_ax, axc, s0: c(s[0], nc), s1: c(s[1], nc), s2: c(s[2], nc), s3: c(s[3], nc) }
    }
}

/// OKLCH (hue in degrees) to sRGB, clamped to gamut.
pub fn oklch(l: f32, c: f32, h: f32) -> Color32 {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    let lin = [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    ];
    let enc = |v: f32| {
        let v = v.clamp(0., 1.);
        let v = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1. / 2.4) - 0.055 };
        (v * 255. + 0.5) as u8
    };
    Color32::from_rgb(enc(lin[0]), enc(lin[1]), enc(lin[2]))
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t + 0.5) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
