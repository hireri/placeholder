//! The app logo as a chrome model: drag it to turn it, let go and it springs back to face-on.
//!
//! Drawn on the CPU (a z-buffered rasteriser shading each pixel from a matcap rendered from the same studio HDRI), so it
//! needs no GL state, works on any backend, and costs nothing while it rests: the image is only redrawn while it moves.

use eframe::egui::{self, pos2, vec2, Color32, ColorImage, CursorIcon, Id, Rect, Sense, TextureHandle, TextureOptions, Ui};
use std::sync::OnceLock;

struct Model {
    v: Vec<[f32; 3]>,
    n: Vec<[f32; 3]>,
    t: Vec<[u32; 3]>,
}

/// A shade map, not colours: 0 is the deepest shadow, 128 the surface's own colour, 255 the brightest highlight.
struct Matcap {
    px: Vec<u8>,
    w: usize,
}

/// `serafin-3d.bin`: u32 vertex count, u32 triangle count, then per vertex position and normal (6 × f32), then u32 triangle indices.
fn model() -> &'static Model {
    static M: OnceLock<Model> = OnceLock::new();
    M.get_or_init(|| {
        let b = include_bytes!("../assets/logo/serafin-3d.bin");
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let (nv, nt) = (u(0) as usize, u(4) as usize);
        let (mut v, mut n) = (Vec::with_capacity(nv), Vec::with_capacity(nv));
        for i in 0..nv {
            let o = 8 + i * 24;
            v.push([f(o), f(o + 4), f(o + 8)]);
            n.push([f(o + 12), f(o + 16), f(o + 20)]);
        }
        let base = 8 + nv * 24;
        let t = (0..nt).map(|i| [u(base + i * 12), u(base + i * 12 + 4), u(base + i * 12 + 8)]).collect();
        Model { v, n, t }
    })
}

fn matcap() -> &'static Matcap {
    static M: OnceLock<Matcap> = OnceLock::new();
    M.get_or_init(|| {
        let img = image::load_from_memory(include_bytes!("../assets/logo/serafin-matcap.png")).expect("bundled matcap").into_rgb8();
        Matcap { w: img.width() as usize, px: img.pixels().map(|p| p.0[0]).collect() }
    })
}

/// The model turned by `yaw` about the vertical axis then `pitch` about the horizontal one, `px` square.
fn render(px: usize, yaw: f32, pitch: f32, accent: [u8; 3]) -> ColorImage {
    render_culled(px, yaw, pitch, accent, true)
}

/// Shade -> colour: shadows are a dark cut of `accent`, midtones are `accent`, highlights run toward white.
fn ramp(accent: [u8; 3]) -> [[u8; 3]; 256] {
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    let base = accent.map(f32::from);
    let (dark, light) = (base.map(|c| c * 0.16), mix(base, [255.; 3], 0.85));
    std::array::from_fn(|i| {
        let s = i as f32 / 255.;
        let c = if s < 0.5 { mix(dark, base, s * 2.) } else { mix(base, light, (s - 0.5) * 2.) };
        c.map(|v| v.round().clamp(0., 255.) as u8)
    })
}

fn render_culled(px: usize, yaw: f32, pitch: f32, accent: [u8; 3], cull: bool) -> ColorImage {
    const SS: usize = 2;
    let (m, mc, lut) = (model(), matcap(), ramp(accent));
    let w = px * SS;
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let rot = |a: [f32; 3]| {
        let (x, z) = (a[0] * cy + a[2] * sy, -a[0] * sy + a[2] * cy);
        [x, a[1] * cp - z * sp, a[1] * sp + z * cp]
    };
    // The model's farthest point from its centre is ~1.17, so any turn stays inside the square.
    let s = w as f32 * 0.5 / 1.2;
    let c = w as f32 * 0.5;
    let sv: Vec<[f32; 3]> = m.v.iter().map(|&p| { let r = rot(p); [c + r[0] * s, c - r[1] * s, r[2]] }).collect();
    let sn: Vec<[f32; 3]> = m.n.iter().map(|&n| rot(n)).collect();

    let mut z = vec![f32::NEG_INFINITY; w * w];
    let mut col = vec![0u8; w * w];
    let edge = |a: [f32; 3], b: [f32; 3], x: f32, y: f32| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
    for t in &m.t {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (a, b, d) = (sv[i0], sv[i1], sv[i2]);
        let area = edge(a, b, d[0], d[1]);
        // Screen y points down, so front faces (counter-clockwise from the camera) have a negative area here.
        if area.abs() < 1e-9 || (cull && area > 0.) {
            continue;
        }
        let x0 = a[0].min(b[0]).min(d[0]).floor().max(0.) as usize;
        let x1 = (a[0].max(b[0]).max(d[0]).ceil() as usize).min(w - 1);
        let y0 = a[1].min(b[1]).min(d[1]).floor().max(0.) as usize;
        let y1 = (a[1].max(b[1]).max(d[1]).ceil() as usize).min(w - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let (w0, w1, w2) = (edge(b, d, px, py) / area, edge(d, a, px, py) / area, edge(a, b, px, py) / area);
                if w0 < 0. || w1 < 0. || w2 < 0. {
                    continue;
                }
                let depth = w0 * a[2] + w1 * b[2] + w2 * d[2];
                let k = y * w + x;
                if depth <= z[k] {
                    continue;
                }
                z[k] = depth;
                let (n0, n1, n2) = (sn[i0], sn[i1], sn[i2]);
                let mut n = [w0 * n0[0] + w1 * n1[0] + w2 * n2[0], w0 * n0[1] + w1 * n1[1] + w2 * n2[1]];
                let len = (n[0] * n[0] + n[1] * n[1] + (w0 * n0[2] + w1 * n1[2] + w2 * n2[2]).powi(2)).sqrt().max(1e-6);
                n = [n[0] / len, n[1] / len];
                // Grazing normals sit on the matcap's rim, which is transparent there; stay just inside it.
                let r = (n[0] * n[0] + n[1] * n[1]).sqrt();
                if r > 0.97 {
                    n = [n[0] * 0.97 / r, n[1] * 0.97 / r];
                }
                let u = ((n[0] * 0.5 + 0.5) * (mc.w - 1) as f32) as usize;
                let v = ((0.5 - n[1] * 0.5) * (mc.w - 1) as f32) as usize;
                col[k] = mc.px[v * mc.w + u];
            }
        }
    }
    let mut out = vec![0u8; px * px * 4];
    for y in 0..px {
        for x in 0..px {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..SS {
                for dx in 0..SS {
                    let k = (y * SS + dy) * w + x * SS + dx;
                    if z[k] > f32::NEG_INFINITY {
                        let c = lut[col[k] as usize];
                        (r, g, b, n) = (r + c[0] as u32, g + c[1] as u32, b + c[2] as u32, n + 1);
                    }
                }
            }
            // Premultiplied: colour summed over covered samples, alpha is the covered share.
            let o = (y * px + x) * 4;
            let q = (SS * SS) as u32;
            out[o..o + 4].copy_from_slice(&[(r / q) as u8, (g / q) as u8, (b / q) as u8, (n * 255 / q) as u8]);
        }
    }
    ColorImage::from_rgba_premultiplied([px, px], &out)
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Spin {
    yaw: f32,
    pitch: f32,
    vyaw: f32,
    vpitch: f32,
}

/// Draws the logo `size` points square at `center`; dragging it turns it, releasing springs it back. `scale` (the beat pulse) only
/// stretches the drawn image, so pulsing never forces a redraw of the texture. The metal takes its colour from `accent`.
pub fn show(ui: &mut Ui, center: egui::Pos2, size: f32, scale: f32, accent: Color32) {
    let ctx = ui.ctx().clone();
    let id = Id::unique("logo3d");
    let rect = Rect::from_center_size(center, vec2(size, size));
    let resp = ui.interact(rect, id, Sense::drag());
    let mut s: Spin = ctx.data(|d| d.get_temp(id)).unwrap_or_default();

    if resp.dragged() {
        let d = resp.drag_delta();
        s.yaw += d.x * 0.02;
        s.pitch = (s.pitch + d.y * 0.02).clamp(-1.3, 1.3);
        s.vyaw = 0.;
        s.vpitch = 0.;
        ctx.set_cursor_icon(CursorIcon::Grabbing);
    } else if crate::gfx::reduced_motion() {
        s = Spin::default();
    } else {
        if resp.hovered() {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        // Slightly underdamped, so it overshoots a little on its way home. Taking the shortest way round first.
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        s.yaw = (s.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        const K: f32 = 90.;
        const C: f32 = 9.;
        s.vyaw += (-K * s.yaw - C * s.vyaw) * dt;
        s.vpitch += (-K * s.pitch - C * s.vpitch) * dt;
        s.yaw += s.vyaw * dt;
        s.pitch += s.vpitch * dt;
        if s.yaw.abs() < 1e-3 && s.pitch.abs() < 1e-3 && s.vyaw.abs() < 1e-2 && s.vpitch.abs() < 1e-2 {
            s = Spin::default();
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, s));
    if resp.dragged() || s != Spin::default() {
        ctx.request_repaint();
    }

    // Redrawn only when the pose, the pixel size or the accent changed since the texture was made.
    let accent = [accent.r(), accent.g(), accent.b()];
    let px = ((size * ctx.pixels_per_point()).round() as usize).clamp(24, 256);
    let tex_id = id.with("tex");
    let drawn: Option<(TextureHandle, Spin, usize, [u8; 3])> = ctx.data(|d| d.get_temp(tex_id));
    let tex = match drawn {
        Some((t, last, lpx, lac)) if last == s && lpx == px && lac == accent => t,
        Some((mut t, ..)) => {
            t.set(render(px, s.yaw, s.pitch, accent), TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(tex_id, (t.clone(), s, px, accent)));
            t
        }
        None => {
            let t = ctx.load_texture("logo3d", render(px, s.yaw, s.pitch, accent), TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(tex_id, (t.clone(), s, px, accent)));
            t
        }
    };
    let drawn = Rect::from_center_size(center, vec2(size, size) * scale);
    ui.painter().image(tex.id(), drawn, Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)), Color32::WHITE);
}

/// The smallest rectangle `(x0, y0, x1, y1)` (end-exclusive) holding every pixel with any alpha, if there is one.
fn alpha_bbox(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for (x, y, _) in img.enumerate_pixels().filter(|(_, _, p)| p.0[3] > 0) {
        b = Some(b.map_or((x, y, x + 1, y + 1), |(a, c, d, e)| (a.min(x), c.min(y), d.max(x + 1), e.max(y + 1))));
    }
    b
}

/// The fuzzy render cropped to its visible glyph and premultiplied, so resizing can't bleed dark colour into the edges.
fn fuzzy() -> &'static image::RgbaImage {
    static F: OnceLock<image::RgbaImage> = OnceLock::new();
    F.get_or_init(|| {
        let img = image::load_from_memory(include_bytes!("../assets/logo/serafin-3d-fuzzy.png")).expect("bundled fuzzy logo").into_rgba8();
        let (x0, y0, x1, y1) = alpha_bbox(&img).unwrap_or((0, 0, img.width(), img.height()));
        let mut c = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
        for p in c.pixels_mut() {
            let a = p.0[3] as f32 / 255.;
            for k in &mut p.0[..3] {
                *k = (*k as f32 * a).round() as u8;
            }
        }
        c
    })
}

/// Visible glyph width as a fraction of the slot, matching the metal model.
const FUZZY_FILL: f32 = 0.82;

/// The still fuzzy logo in the same `size` slot as [`show`], pulsed by `scale`. Not draggable, and its colour is baked in.
pub fn show_fuzzy(ui: &mut Ui, center: egui::Pos2, size: f32, scale: f32) {
    let ctx = ui.ctx().clone();
    let src = fuzzy();
    let w = size * FUZZY_FILL;
    let (pw, ph) = (w * ctx.pixels_per_point(), w * src.height() as f32 / src.width() as f32 * ctx.pixels_per_point());
    let (pw, ph) = ((pw.round() as u32).max(1), (ph.round() as u32).max(1));
    let id = Id::unique("logo-fuzzy");
    let tex = match ctx.data(|d| d.get_temp::<(TextureHandle, u32)>(id)) {
        Some((t, lw)) if lw == pw => t,
        _ => {
            let r = image::imageops::resize(src, pw, ph, image::imageops::FilterType::Lanczos3);
            let px = r.pixels().map(|p| {
                let a = p.0[3];
                Color32::from_rgba_premultiplied(p.0[0].min(a), p.0[1].min(a), p.0[2].min(a), a)
            });
            let t = ctx.load_texture("logo-fuzzy", ColorImage { size: [pw as usize, ph as usize], pixels: px.collect(), source_size: vec2(pw as f32, ph as f32) }, TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, (t.clone(), pw)));
            t
        }
    };
    let r = Rect::from_center_size(center, vec2(w, w * ph as f32 / pw as f32) * scale);
    ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)), Color32::WHITE);
}

/// Window icon for logo choice `n` (0 flat, 1 metal, 2 fuzzy), decoded once.
pub fn window_icon(n: u8) -> std::sync::Arc<egui::IconData> {
    static I: [OnceLock<std::sync::Arc<egui::IconData>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let n = (n as usize).min(2);
    I[n].get_or_init(|| {
        let b: &[u8] = match n {
            0 => include_bytes!("../assets/logo/serafin-app-icon.png"),
            1 => include_bytes!("../assets/logo/serafin-app-icon-metal-256.png"),
            _ => include_bytes!("../assets/logo/serafin-app-icon-fuzzy-256.png"),
        };
        let i = image::load_from_memory(b).expect("bundled app icon").into_rgba8();
        std::sync::Arc::new(egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() })
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: [u8; 3] = [0xA2, 0x4B, 0xFF];

    #[test]
    fn fuzzy_crop_fits() {
        let mut i = image::RgbaImage::new(10, 10);
        assert_eq!(alpha_bbox(&i), None);
        i.put_pixel(2, 3, image::Rgba([1, 1, 1, 9]));
        i.put_pixel(6, 4, image::Rgba([1, 1, 1, 9]));
        assert_eq!(alpha_bbox(&i), Some((2, 3, 7, 5)));
        let f = fuzzy();
        assert!(f.width() < 1024 && f.width() > f.height());
    }

    #[test]
    fn renders_and_turns() {
        let rest = render(128, 0., 0., A);
        let alpha = |i: &ColorImage, x: usize, y: usize| i.pixels[y * 128 + x].a();
        assert_eq!(alpha(&rest, 0, 0), 0, "corner is empty");
        assert!(alpha(&rest, 64, 64) > 200, "centre is covered");
        let turned = render(128, 1.0, 0.3, A);
        assert_ne!(rest.pixels, turned.pixels);
        for (y, p) in [(0., 0.), (1.0, 0.3), (-2.0, -0.6), (3.0, 0.9)] {
            assert_eq!(render_culled(96, y, p, A, true).pixels, render_culled(96, y, p, A, false).pixels, "culling must not change the picture");
        }
        // `SERAFIN_DUMP=dir cargo test -p serafin logo3d` writes the poses as PNGs to look at.
        if let Some(dir) = std::env::var_os("SERAFIN_DUMP") {
            for (name, (yaw, pitch, ac)) in [("rest", (0., 0., A)), ("yaw", (0.9, 0., A)), ("green", (0.5, 0.2, [0x30, 0xC0, 0x70])), ("orange", (-0.7, 0.3, [0xF0, 0x80, 0x30]))] {
                let img = render(256, yaw, pitch, ac);
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::RgbaImage::from_raw(256, 256, bytes).unwrap().save(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore]
    fn render_time() {
        for px in [64, 96, 128, 192] {
            let t = std::time::Instant::now();
            for i in 0..20 {
                super::render(px, i as f32 * 0.1, 0.2, [0xA2, 0x4B, 0xFF]);
            }
            println!("{px}px: {:?} per frame", t.elapsed() / 20);
        }
    }
}
