//! Tonal theme derived from one hue, mirroring the CSS tokens in design/desktop-design.html.

use gpui::{linear_color_stop, linear_gradient, px, Background, Global, Hsla, Pixels, Rgba};

#[derive(Clone, Copy)]
pub struct Theme {
    pub hue: f32,
    /// window, sidebar, panel, hover
    pub s0: Hsla,
    pub s1: Hsla,
    pub s2: Hsla,
    pub s3: Hsla,
    /// text: primary, muted, faint
    pub tx: Hsla,
    pub tm: Hsla,
    pub tf: Hsla,
    /// accent, text on accent, accent container
    pub ax: Hsla,
    pub on_ax: Hsla,
    pub axc: Hsla,
    pub art_r: Pixels,
    pub ctl_r: Pixels,
}

impl Global for Theme {}

impl Theme {
    pub fn new(hue: f32, ac: f32, nc: f32, dark: bool) -> Self {
        let c = |l, ch| oklch(l, ch, hue);
        let (s, tx, tm, tf, ax, on_ax, axc) = if dark {
            ([0.17, 0.205, 0.25, 0.30], c(0.96, 0.008), c(0.72, 0.02), c(0.56, 0.02), c(0.78, ac), c(0.2, 0.03), c(0.32, ac * 0.55))
        } else {
            ([0.975, 0.95, 0.925, 0.89], c(0.22, 0.02), c(0.46, 0.02), c(0.6, 0.015), c(0.5, ac), c(0.98, 0.01), c(0.9, ac * 0.5))
        };
        Theme {
            hue, tx, tm, tf, ax, on_ax, axc,
            s0: c(s[0], nc), s1: c(s[1], nc), s2: c(s[2], nc), s3: c(s[3], nc),
            art_r: px(6.), ctl_r: px(8.),
        }
    }

    /// Placeholder artwork: a two-stop gradient whose hue is offset per item.
    pub fn art(&self, offset: f32) -> Background {
        let h = self.hue + offset;
        linear_gradient(140., linear_color_stop(oklch(0.66, 0.16, h), 0.), linear_color_stop(oklch(0.34, 0.1, h - 90.), 1.))
    }

    /// Header wash: accent faintly mixed into the panel colour, fading out by 64%.
    pub fn wash(&self) -> Background {
        linear_gradient(135., linear_color_stop(self.s1.blend(self.ax.opacity(0.24)), 0.), linear_color_stop(self.s1, 0.64))
    }

    /// Vertical fade from transparent to `to`, for list edges (gpui has no mask-image).
    pub fn fade(to: Hsla) -> Background {
        linear_gradient(180., linear_color_stop(to.opacity(0.), 0.), linear_color_stop(to, 1.))
    }
}

/// OKLCH (hue in degrees) to sRGB, clamped to gamut.
pub fn oklch(l: f32, c: f32, h: f32) -> Hsla {
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
        if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1. / 2.4) - 0.055 }
    };
    Rgba { r: enc(lin[0]), g: enc(lin[1]), b: enc(lin[2]), a: 1. }.into()
}
