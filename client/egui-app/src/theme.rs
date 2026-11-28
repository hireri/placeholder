//! Tonal theme derived from one hue; same OKLCH tokens as design/desktop-design.html.

use eframe::egui::Color32;
use std::collections::HashMap;

/// User colours from `theme.toml`: `[dark]` and `[light]` tables of `name = "#rrggbb"`.
#[derive(serde::Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
pub struct Custom {
    dark: HashMap<String, String>,
    light: HashMap<String, String>,
}

const TEMPLATE: &str = "# Serafin custom theme. Only uncommented lines override the generated (artwork or accent) colours.
# Names: s0 s1 s2 s3 (window, sidebar, panel, hover), tx tm tf (text), ax on_ax axc (accent, text on accent, accent container)
# With Pure black on, dark s0 and s1 are pulled towards black after these apply.
[dark]
# s0 = \"#121214\"
# ax = \"#8ab4ff\"

[light]
# s0 = \"#fafafa\"
# ax = \"#1a56db\"
";

pub fn theme_path() -> std::path::PathBuf {
    crate::backend::data_dir().join("theme.toml")
}

pub fn mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(theme_path()).and_then(|m| m.modified()).ok()
}

/// Reads `theme.toml`, writing a starter file first if there is none. Errors are toasted and give `None`.
pub fn load_custom() -> Option<Custom> {
    let p = theme_path();
    if !p.exists() {
        let _ = std::fs::create_dir_all(p.parent()?);
        let _ = std::fs::write(&p, TEMPLATE);
    }
    match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| toml::from_str(&s).map_err(|e| e.message().to_string())) {
        Ok(c) => Some(c),
        Err(e) => {
            crate::toast::error(format!("theme.toml: {e}"));
            None
        }
    }
}

fn hex(s: &str) -> Option<Color32> {
    let s = s.trim().strip_prefix('#')?;
    let n = u32::from_str_radix(s, 16).ok().filter(|_| s.len() == 6)?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Hand-tuned accent presets: (name, hue, accent chroma, surface chroma). Chroma is tuned per hue because
/// yellows and cyans clip in sRGB sooner than blues and reds, and warm surfaces need less tint to read as neutral.
pub const PRESETS: [(&str, f32, f32, f32); 10] = [
    ("Blue", 265., 0.13, 0.012),
    ("Violet", 295., 0.14, 0.014),
    ("Pink", 350., 0.14, 0.012),
    ("Red", 25., 0.15, 0.012),
    ("Orange", 55., 0.14, 0.011),
    ("Amber", 85., 0.13, 0.010),
    ("Green", 150., 0.13, 0.010),
    ("Teal", 190., 0.10, 0.009),
    ("Sky", 235., 0.12, 0.011),
    ("Monochrome", 0., 0., 0.),
];

#[derive(Clone, Copy, PartialEq)]
pub struct Theme {
    pub hue: f32,
    pub dark: bool,
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
        // Monochrome (ac == 0): the fixed text chroma would otherwise tint everything red at hue 0.
        let k = if ac == 0. { 0. } else { 1. };
        let c = |l, ch| oklch(l, ch, hue);
        let t = |l, ch: f32| c(l, ch * k);
        let (s, tx, tm, tf, ax, on_ax, axc) = if dark {
            ([0.17, 0.205, 0.25, 0.30], t(0.96, 0.008), t(0.72, 0.02), t(0.56, 0.02), c(0.78, ac), t(0.2, 0.03), c(0.32, ac * 0.55))
        } else {
            ([0.935, 0.99, 0.95, 0.91], t(0.22, 0.02), t(0.46, 0.02), t(0.6, 0.015), c(0.5, ac), t(0.98, 0.01), c(0.9, ac * 0.5))
        };
        Theme { hue, dark, tx, tm, tf, ax, on_ax, axc, s0: c(s[0], nc), s1: c(s[1], nc), s2: c(s[2], nc), s3: c(s[3], nc) }
    }

    /// Nothing playing: near-grey surfaces with a blue accent.
    pub fn idle(dark: bool) -> Self {
        Theme::new(265., 0.12, 0.003, dark)
    }

    /// Accent preset `i` of `PRESETS`.
    pub fn preset(i: u8, dark: bool) -> Self {
        let (_, hue, ac, nc) = PRESETS[(i as usize).min(PRESETS.len() - 1)];
        Theme::new(hue, ac, nc, dark)
    }

    /// Built around a cover's (hue, chroma): vivid covers get a stronger accent and tinted surfaces.
    pub fn tinted((hue, chroma): (f32, f32), dark: bool) -> Self {
        Theme::new(hue, (chroma * 1.4).clamp(0.06, 0.16), (chroma * 0.2).clamp(0.006, 0.026), dark)
    }

    /// Keeps `self`'s surfaces and text but takes `p`'s accent colours.
    pub fn accent_of(mut self, p: Theme) -> Self {
        (self.ax, self.on_ax, self.axc) = (p.ax, p.on_ax, p.axc);
        self
    }

    /// Window and sidebar go true black (OLED); panels and hover keep their tonal steps.
    pub fn pure_black(mut self) -> Self {
        self.s0 = Color32::BLACK;
        self.s1 = mix(Color32::BLACK, self.s1, 0.5);
        self
    }

    /// Overrides colours with the matching mode's table from `c`.
    pub fn custom(mut self, c: &Custom) -> Self {
        for (k, v) in if self.dark { &c.dark } else { &c.light } {
            let Some(col) = hex(v) else { continue };
            match k.as_str() {
                "s0" => self.s0 = col,
                "s1" => self.s1 = col,
                "s2" => self.s2 = col,
                "s3" => self.s3 = col,
                "tx" => self.tx = col,
                "tm" => self.tm = col,
                "tf" => self.tf = col,
                "ax" => self.ax = col,
                "on_ax" => self.on_ax = col,
                "axc" => self.axc = col,
                _ => {}
            }
        }
        self
    }

    /// Colour-wise blend `t` of the way from `self` to `to`.
    pub fn lerp(&self, to: &Theme, t: f32) -> Theme {
        let m = |a, b| mix(a, b, t);
        Theme {
            hue: to.hue,
            dark: to.dark,
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
