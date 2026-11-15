//! Tonal theme derived from one hue; same OKLCH tokens as design/desktop-design.html.

use eframe::egui::Color32;

#[derive(Clone, Copy, PartialEq)]
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

    /// Nothing playing: near-grey surfaces with a blue accent.
    pub fn idle() -> Self {
        Theme::new(265., 0.12, 0.003, true)
    }

    /// Built around a cover's (hue, chroma): vivid covers get a stronger accent and tinted surfaces.
    pub fn tinted((hue, chroma): (f32, f32)) -> Self {
        Theme::new(hue, (chroma * 1.4).clamp(0.06, 0.16), (chroma * 0.2).clamp(0.006, 0.026), true)
    }

    /// Colour-wise blend `t` of the way from `self` to `to`.
    pub fn lerp(&self, to: &Theme, t: f32) -> Theme {
        let m = |a, b| mix(a, b, t);
        Theme {
            hue: to.hue,
            s0: m(self.s0, to.s0),
            s1: m(self.s1, to.s1),
            s2: m(self.s2, to.s2),
            s3: m(self.s3, to.s3),
            tx: m(self.tx, to.tx),
            tm: m(self.tm, to.tm),
            tf: m(self.tf, to.tf),
            ax: m(self.ax, to.ax),
            on_ax: m(self.on_ax, to.on_ax),
            axc: m(self.axc, to.axc),
        }
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

/// sRGB to OKLab (L, a, b).
pub fn oklab([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let lin = |v: u8| {
        let v = v as f32 / 255.;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t + 0.5) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips_through_oklch() {
        for rgb in [[200u8, 40, 60], [30, 120, 220], [90, 200, 110]] {
            let (l, a, b) = oklab(rgb);
            let back = oklch(l, (a * a + b * b).sqrt(), b.atan2(a).to_degrees());
            let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
            assert!(d(back.r(), rgb[0]) <= 1 && d(back.g(), rgb[1]) <= 1 && d(back.b(), rgb[2]) <= 1, "{rgb:?} -> {back:?}");
        }
    }
}
