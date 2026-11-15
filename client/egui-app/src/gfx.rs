//! Textures (placeholder art, header wash, blurred backdrop), icons, fonts and small paint helpers.

use crate::theme::{mix, oklch, Theme};
use eframe::egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily, VariationCoords};
use eframe::egui::{
    self, load::SizedTexture, text::LayoutJob, Align2, Color32, ColorImage, Context, CornerRadius, FontData, FontFamily, FontId, Image, Painter, Pos2, Rect,
    Response, Sense, TextFormat, TextureHandle, TextureId, TextureOptions, Ui,
};
use std::collections::HashMap;

// ---------- icons ----------

/// `icon!("play")` is a static URI into the compiled-in SVG set; draw with [`icon`].
macro_rules! icon {
    ($n:literal) => {
        concat!("bytes://icons/", $n, ".svg")
    };
}

const ICONS: &[(&str, &[u8])] = &[
    ("bytes://icons/alb.svg", include_bytes!("../assets/icons/alb.svg")),
    ("bytes://icons/art.svg", include_bytes!("../assets/icons/art.svg")),
    ("bytes://icons/back.svg", include_bytes!("../assets/icons/back.svg")),
    ("bytes://icons/bell.svg", include_bytes!("../assets/icons/bell.svg")),
    ("bytes://icons/cast.svg", include_bytes!("../assets/icons/cast.svg")),
    ("bytes://icons/col.svg", include_bytes!("../assets/icons/col.svg")),
    ("bytes://icons/colr.svg", include_bytes!("../assets/icons/colr.svg")),
    ("bytes://icons/dir.svg", include_bytes!("../assets/icons/dir.svg")),
    ("bytes://icons/ff.svg", include_bytes!("../assets/icons/ff.svg")),
    ("bytes://icons/fwd.svg", include_bytes!("../assets/icons/fwd.svg")),
    ("bytes://icons/grip.svg", include_bytes!("../assets/icons/grip.svg")),
    ("bytes://icons/hist.svg", include_bytes!("../assets/icons/hist.svg")),
    ("bytes://icons/home.svg", include_bytes!("../assets/icons/home.svg")),
    ("bytes://icons/imp.svg", include_bytes!("../assets/icons/imp.svg")),
    ("bytes://icons/inf.svg", include_bytes!("../assets/icons/inf.svg")),
    ("bytes://icons/pause.svg", include_bytes!("../assets/icons/pause.svg")),
    ("bytes://icons/lib.svg", include_bytes!("../assets/icons/lib.svg")),
    ("bytes://icons/lyr.svg", include_bytes!("../assets/icons/lyr.svg")),
    ("bytes://icons/more.svg", include_bytes!("../assets/icons/more.svg")),
    ("bytes://icons/pl.svg", include_bytes!("../assets/icons/pl.svg")),
    ("bytes://icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("bytes://icons/que.svg", include_bytes!("../assets/icons/que.svg")),
    ("bytes://icons/rep.svg", include_bytes!("../assets/icons/rep.svg")),
    ("bytes://icons/rw.svg", include_bytes!("../assets/icons/rw.svg")),
    ("bytes://icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("bytes://icons/shuf.svg", include_bytes!("../assets/icons/shuf.svg")),
    ("bytes://icons/song.svg", include_bytes!("../assets/icons/song.svg")),
    ("bytes://icons/star.svg", include_bytes!("../assets/icons/star.svg")),
    ("bytes://icons/vhi.svg", include_bytes!("../assets/icons/vhi.svg")),
    ("bytes://icons/vlo.svg", include_bytes!("../assets/icons/vlo.svg")),
    ("bytes://icons/check.svg", include_bytes!("../assets/icons/check.svg")),
    ("bytes://icons/grid.svg", include_bytes!("../assets/icons/grid.svg")),
    ("bytes://icons/vc.svg", include_bytes!("../assets/icons/vc.svg")),
    ("bytes://icons/vg.svg", include_bytes!("../assets/icons/vg.svg")),
    ("bytes://icons/vl.svg", include_bytes!("../assets/icons/vl.svg")),
    ("bytes://icons/x.svg", include_bytes!("../assets/icons/x.svg"))
];

pub fn install(ctx: &Context) {
    egui_extras::install_image_loaders(ctx);
    for (uri, bytes) in ICONS {
        ctx.include_bytes(*uri, *bytes);
    }
    // Inter Variable, one font entry per weight so FontId can pick weight through its family.
    let weights = [("inter-400", 400., FontFamily::Proportional), ("inter-500", 500., w(Weight::Medium)), ("inter-600", 600., w(Weight::Semi)), ("inter-650", 650., w(Weight::Bold))];
    for (name, wght, fam) in weights {
        let data = FontData::from_static(include_bytes!("../assets/fonts/InterVariable.ttf"))
            .tweak(egui::FontTweak { coords: VariationCoords::new([(b"wght", wght)]), ..Default::default() });
        // Lowest priority would let the default font win; Highest puts Inter first and keeps the defaults as glyph fallback.
        ctx.add_font(FontInsert::new(name, data, vec![InsertFontFamily { family: fam, priority: FontPriority::Highest }]));
    }
}

#[derive(Clone, Copy)]
pub enum Weight {
    Medium,
    Semi,
    Bold,
}

fn w(weight: Weight) -> FontFamily {
    FontFamily::Name(match weight { Weight::Medium => "w500", Weight::Semi => "w600", Weight::Bold => "w650" }.into())
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn font_w(size: f32, weight: Weight) -> FontId {
    FontId::new(size, w(weight))
}

pub fn icon(ui: &Ui, uri: &'static str, center: Pos2, size: f32, color: Color32) {
    Image::new(uri).fit_to_exact_size(egui::vec2(size, size)).tint(color).paint_at(ui, Rect::from_center_size(center, egui::vec2(size, size)));
}

// ---------- shapes & text ----------

pub fn rad(r: f32) -> CornerRadius {
    CornerRadius::same(r.min(255.) as u8)
}

/// Paints the bottom corners' outside-the-curve wedges in `bg`, so rectangular-clipped content looks rounded.
pub fn round_bottom_corners(painter: &Painter, rect: Rect, r: f32, bg: Color32) {
    corner_wedges(painter, rect, r, bg, true);
}

/// Same for the top corners.
pub fn round_top_corners(painter: &Painter, rect: Rect, r: f32, bg: Color32) {
    corner_wedges(painter, rect, r, bg, false);
}

fn corner_wedges(painter: &Painter, rect: Rect, r: f32, bg: Color32, bottom: bool) {
    const N: usize = 12;
    let (ey, sy) = if bottom { (rect.max.y, 1.) } else { (rect.min.y, -1.) };
    let mut mesh = egui::Mesh::default();
    for (cx, sx) in [(rect.min.x + r, -1.), (rect.max.x - r, 1.)] {
        let centre = egui::pos2(cx, ey - sy * r);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(cx + sx * r, ey), bg);
        for i in 0..=N {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / N as f32;
            mesh.colored_vertex(centre + egui::vec2(sx * r * a.sin(), sy * r * a.cos()), bg);
        }
        for i in 0..N as u32 {
            mesh.add_triangle(base, base + 1 + i, base + 2 + i);
        }
    }
    painter.add(mesh);
}

pub fn shadow(painter: &Painter, rect: Rect, r: f32, dy: i8, blur: u8, alpha: u8) {
    painter.add(egui::Shadow { offset: [0, dy], blur, spread: 0, color: Color32::from_black_alpha(alpha) }.as_shape(rect, rad(r)));
}

/// Single-line text, left/centre anchored at `pos`, ellipsised at `max_w`.
pub fn trunc(painter: &Painter, pos: Pos2, max_w: f32, text: &str, font: FontId, color: Color32) -> Rect {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = max_w;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    let rect = Align2::LEFT_CENTER.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// Word-wrapped text ellipsised after `rows` lines; paint with `painter.galley(pos, g, color)`.
pub fn wrapped(painter: &Painter, text: &str, font: FontId, color: Color32, max_w: f32, rows: usize) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = max_w;
    job.wrap.max_rows = rows;
    job.wrap.overflow_character = Some('…');
    painter.layout_job(job)
}

/// Outlined pill button with a centred label.
pub fn pill(ui: &Ui, t: &Theme, key: impl std::hash::Hash + std::fmt::Debug, rect: Rect, label: &str) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(key), Sense::click());
    let col = if resp.hovered() { t.tx } else { t.tm };
    ui.painter().rect_stroke(rect, rad(rect.height() / 2.), egui::Stroke::new(1., col.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font_w(12.5, Weight::Semi), col);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn text_w(painter: &Painter, text: &str, font: FontId) -> f32 {
    painter.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x
}

/// Hit-tested tonal button: press shows `s3`, hover `s2`, otherwise `base`. Commits on release.
pub fn tonal(ui: &Ui, t: &Theme, key: impl std::hash::Hash + std::fmt::Debug, rect: Rect, r: f32, base: Option<Color32>) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(key), Sense::click());
    let fill = if resp.is_pointer_button_down_on() { Some(t.s3) } else if resp.hovered() { Some(t.s2) } else { base };
    if let Some(c) = fill {
        ui.painter().rect_filled(rect, rad(r), c);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 32px (default) circular icon button centred on `center`.
pub fn icon_button(ui: &Ui, t: &Theme, key: impl std::hash::Hash + std::fmt::Debug, center: Pos2, size: f32, uri: &'static str, glyph: f32, color: Color32) -> Response {
    let rect = Rect::from_center_size(center, egui::vec2(size, size));
    let resp = tonal(ui, t, key, rect, size / 2., None);
    icon(ui, uri, center, glyph, color);
    resp
}

/// Vertical fade from transparent to `to` over `rect` (egui has no gradient fill).
pub fn fade_down(painter: &Painter, rect: Rect, to: Color32) {
    let mut m = egui::Mesh::default();
    let (a, b) = (to.gamma_multiply(0.), to);
    for (p, c) in [(rect.left_top(), a), (rect.right_top(), a), (rect.right_bottom(), b), (rect.left_bottom(), b)] {
        m.vertices.push(egui::epaint::Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: c });
    }
    m.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(m);
}

/// Backdrop dimming (0..255 grey multiplier) so text stays readable over the blurred cover.
pub const DIM: u8 = 150;

/// Paints the blurred backdrop texture over `y0..y1` with alpha rising 0 -> 1, so whatever is beneath dissolves into it.
/// `card` is the rect the backdrop texture is stretched over; uvs match it.
pub fn fade_blur(painter: &Painter, tex: TextureId, card: Rect, y0: f32, y1: f32, alpha: f32) {
    const ROWS: usize = 6;
    let mut m = egui::Mesh::with_texture(tex);
    for i in 0..ROWS {
        let f = i as f32 / (ROWS - 1) as f32;
        let a = f * f * (3. - 2. * f) * alpha;
        let (y, al) = (y0 + (y1 - y0) * f, (a * 255.) as u8);
        let g = (DIM as f32 * a) as u8;
        let c = Color32::from_rgba_premultiplied(g, g, g, al);
        let v = (y - card.min.y) / card.height();
        for (x, u) in [(card.min.x, 0.), (card.max.x, 1.)] {
            m.vertices.push(egui::epaint::Vertex { pos: egui::pos2(x, y), uv: egui::pos2(u, v), color: c });
        }
        if i > 0 {
            let b = (i * 2) as u32;
            m.indices.extend([b - 2, b - 1, b + 1, b - 2, b + 1, b]);
        }
    }
    painter.add(m);
}

// ---------- textures ----------

pub struct Gfx {
    arts: HashMap<i32, TextureHandle>,
    cover: Option<TextureHandle>,
    wash: Option<TextureHandle>,
    backdrop: Option<TextureHandle>,
}

impl Gfx {
    pub fn new() -> Self {
        Gfx { arts: HashMap::new(), cover: None, wash: None, backdrop: None }
    }

    /// Small placeholder gradient used for library items; cached per hue offset.
    pub fn art(&mut self, ctx: &Context, t: &Theme, offset: f32) -> TextureId {
        self.arts.entry(offset as i32).or_insert_with(|| {
            let h = t.hue + offset;
            ctx.load_texture(format!("art{offset}"), gradient(48, 48, 140., oklch(0.66, 0.16, h), oklch(0.34, 0.1, h - 90.), 1.), TextureOptions::LINEAR)
        }).id()
    }

    /// Stand-in for real cover art: the gradient plus two soft colour blobs, so the blurred backdrop has something to show.
    pub fn cover(&mut self, ctx: &Context, t: &Theme) -> TextureId {
        self.cover.get_or_insert_with(|| ctx.load_texture("cover", cover_image(t, 256), TextureOptions::LINEAR)).id()
    }

    pub fn wash(&mut self, ctx: &Context, t: &Theme) -> TextureId {
        self.wash.get_or_insert_with(|| {
            let from = mix(t.s1, t.ax, 0.24);
            ctx.load_texture("wash", gradient(256, 96, 135., from, t.s1, 0.64), TextureOptions::LINEAR)
        }).id()
    }

    /// Blurred cover for the player backdrop. Real art takes the same path: downscale, then blur on the CPU once per track.
    pub fn backdrop(&mut self, ctx: &Context, t: &Theme) -> TextureId {
        self.backdrop.get_or_insert_with(|| ctx.load_texture("backdrop", blur(&cover_image(t, 256), 48, 5.), TextureOptions::LINEAR)).id()
    }
}

pub fn sized(id: TextureId, size: f32) -> SizedTexture {
    SizedTexture::new(id, egui::vec2(size, size))
}

/// CSS-style linear gradient (`angle` 0 = up, clockwise); `end` is where `to` is reached (0..1).
fn gradient(w: usize, h: usize, angle: f32, from: Color32, to: Color32, end: f32) -> ColorImage {
    let (s, c) = angle.to_radians().sin_cos();
    let (dx, dy) = (s, -c);
    let len = s.abs() + c.abs();
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / (w - 1) as f32 - 0.5, y as f32 / (h - 1) as f32 - 0.5);
            let t = ((u * dx + v * dy) / len + 0.5) / end;
            px.push(mix(from, to, t.clamp(0., 1.)));
        }
    }
    ColorImage::new([w, h], px)
}

fn cover_image(t: &Theme, size: usize) -> ColorImage {
    let h = t.hue + 50.;
    let mut img = gradient(size, size, 140., oklch(0.66, 0.16, h), oklch(0.34, 0.1, h - 90.), 1.);
    let blobs = [((0.28, 0.3), 0.32, oklch(0.78, 0.15, h + 120.)), ((0.75, 0.72), 0.38, oklch(0.55, 0.14, h - 70.))];
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
            let px = &mut img.pixels[y * size + x];
            for ((bx, by), r, col) in blobs {
                let d = ((u - bx).powi(2) + (v - by).powi(2)).sqrt() / r;
                *px = mix(*px, col, (1. - d).clamp(0., 1.).powi(2) * 0.8);
            }
        }
    }
    img
}

fn blur(src: &ColorImage, side: u32, sigma: f32) -> ColorImage {
    let raw: Vec<u8> = src.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), 255]).collect();
    let img = image::RgbaImage::from_raw(src.width() as u32, src.height() as u32, raw).expect("size matches");
    let small = image::imageops::resize(&img, side, side, image::imageops::FilterType::Triangle);
    let out = image::imageops::blur(&small, sigma);
    ColorImage::from_rgba_unmultiplied([side as usize, side as usize], out.as_raw())
}
