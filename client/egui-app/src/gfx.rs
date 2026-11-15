//! Textures (placeholder art, header wash, blurred backdrop), icons, fonts and small paint helpers.

use crate::theme::{mix, oklab, Theme};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::mpsc::{channel, Receiver, Sender};
use eframe::egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily, VariationCoords};
use eframe::egui::{
    self, load::{SizedTexture, TexturePoll}, text::LayoutJob, Align2, Color32, ColorImage, Context, CornerRadius, FontData, FontFamily, FontId, Image, Painter,
    Pos2, Rect, Response, Sense, TextFormat, TextureHandle, TextureId, TextureOptions, Ui,
};

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
    ("bytes://icons/col.svg", include_bytes!("../assets/icons/col.svg")),
    ("bytes://icons/colr.svg", include_bytes!("../assets/icons/colr.svg")),
    ("bytes://icons/dir.svg", include_bytes!("../assets/icons/dir.svg")),
    ("bytes://icons/ff.svg", include_bytes!("../assets/icons/ff.svg")),
    ("bytes://icons/fwd.svg", include_bytes!("../assets/icons/fwd.svg")),
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
    ("bytes://icons/starf.svg", include_bytes!("../assets/icons/starf.svg")),
    ("bytes://icons/vhi.svg", include_bytes!("../assets/icons/vhi.svg")),
    ("bytes://icons/vlo.svg", include_bytes!("../assets/icons/vlo.svg")),
    ("bytes://icons/check.svg", include_bytes!("../assets/icons/check.svg")),
    ("bytes://icons/grid.svg", include_bytes!("../assets/icons/grid.svg")),
    ("bytes://icons/vc.svg", include_bytes!("../assets/icons/vc.svg")),
    ("bytes://icons/vg.svg", include_bytes!("../assets/icons/vg.svg")),
    ("bytes://icons/vl.svg", include_bytes!("../assets/icons/vl.svg")),
    ("bytes://icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("bytes://icons/vid.svg", include_bytes!("../assets/icons/vid.svg")),
    ("bytes://icons/vidoff.svg", include_bytes!("../assets/icons/vidoff.svg"))
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
        // Meshes get no anti-aliasing: fade `bg` out over 1px inside the arc so the curve isn't a staircase.
        let inner = mesh.vertices.len() as u32;
        for i in 0..=N {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / N as f32;
            mesh.colored_vertex(centre + egui::vec2(sx * (r - 1.) * a.sin(), sy * (r - 1.) * a.cos()), Color32::TRANSPARENT);
        }
        for i in 0..N as u32 {
            let (o, n) = (base + 1 + i, inner + i);
            mesh.add_triangle(o, o + 1, n + 1);
            mesh.add_triangle(o, n + 1, n);
        }
    }
    painter.add(mesh);
}

pub fn shadow(painter: &Painter, rect: Rect, r: f32, dy: i8, blur: u8, alpha: u8) {
    painter.add(egui::Shadow { offset: [0, dy], blur, spread: 0, color: Color32::from_black_alpha(alpha) }.as_shape(rect, rad(r)));
}

/// Single-line text, left/centre anchored at `pos`, ellipsised at `max_w`.
pub fn trunc(painter: &Painter, pos: Pos2, max_w: f32, text: &str, font: FontId, color: Color32) -> Rect {
    // A column narrower than a glyph would show only the ellipsis.
    if max_w < 8. {
        return Rect::from_min_size(pos, egui::Vec2::ZERO);
    }
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

/// Hit area with a pointer cursor and no fill; for controls drawn over the player's artwork.
pub fn press(ui: &Ui, key: impl std::hash::Hash + std::fmt::Debug, rect: Rect) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(key), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Icon button without a background: the glyph brightens on hover and shrinks a little while pressed.
pub fn glyph_button(ui: &Ui, key: impl std::hash::Hash + std::fmt::Debug, center: Pos2, size: f32, uri: &'static str, glyph: f32, color: Color32) -> Response {
    let resp = press(ui, key, Rect::from_center_size(center, egui::vec2(size, size)));
    let perceived = 0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32;
    let col = match (resp.hovered(), perceived < 110.) {
        (false, _) => color,
        (true, true) => color.lerp_to_gamma(Color32::BLACK, 0.45),
        (true, false) => color.lerp_to_gamma(Color32::WHITE, 0.45),
    };
    icon(ui, uri, center, if resp.is_pointer_button_down_on() { glyph * 0.9 } else { glyph }, col);
    resp
}

/// 32px (default) circular icon button centred on `center`.
pub fn icon_button(ui: &Ui, t: &Theme, key: impl std::hash::Hash + std::fmt::Debug, center: Pos2, size: f32, uri: &'static str, glyph: f32, color: Color32) -> Response {
    let rect = Rect::from_center_size(center, egui::vec2(size, size));
    let resp = tonal(ui, t, key, rect, size / 2., None);
    icon(ui, uri, center, glyph, color);
    resp
}

/// UV window that centre-crops a source of aspect `src` (w/h) to fill a target of aspect `dst`.
pub fn crop_uv(src: f32, dst: f32) -> Rect {
    // Keep the longer side's centre: trim the sides when the source is wider than the target, else the top and bottom.
    let size = if src > dst { egui::vec2(dst / src, 1.) } else { egui::vec2(1., src / dst) };
    Rect::from_center_size(egui::pos2(0.5, 0.5), size)
}

/// The cover at `url` for a `size` slot, `None` until one has loaded. The CDN bucket is roughly the pixels drawn, in 64px steps,
/// and never below the largest bucket already loaded for this URL, so a resize keeps the sharp image up while a bigger one loads.
fn cover_texture(ctx: &Context, url: &str, size: egui::Vec2) -> Option<SizedTexture> {
    let key = egui::Id::unique(("cover-px", url));
    let shown = ctx.data(|d| d.get_temp::<u32>(key)).unwrap_or(0);
    let want = ((size.x.max(size.y) * ctx.pixels_per_point() / 64.).ceil() * 64.).clamp(64., 800.) as u32;
    let load = |px: u32| match Image::new(sized_url(url, px)).load_for_size(ctx, size) {
        Ok(TexturePoll::Ready { texture }) => Some(texture),
        _ => None,
    };
    let px = want.max(shown);
    if let Some(texture) = load(px) {
        if px > shown {
            ctx.data_mut(|d| d.insert_temp(key, px));
        }
        return Some(texture);
    }
    (shown > 0).then(|| load(shown)).flatten()
}

/// Paints the cover at `url` centre-cropped to fill `rect` (never stretched). Nothing paints until a cover for it has loaded.
pub fn paint_cover(ui: &Ui, url: &str, rect: Rect, corners: CornerRadius, tint: Color32) {
    let Some(texture) = cover_texture(ui.ctx(), url, rect.size()) else { return };
    let uv = crop_uv(texture.size.x / texture.size.y, rect.width() / rect.height());
    Image::new(texture).fit_to_exact_size(rect.size()).uv(uv).corner_radius(corners).tint(tint).paint_at(ui, rect);
}

/// Width / height of the cover at `url` once it has loaded for a `size` slot (video thumbnails are ~16:9, songs 1:1).
pub fn cover_aspect(ctx: &Context, url: &str, size: egui::Vec2) -> Option<f32> {
    cover_texture(ctx, url, size).map(|t| t.size.x / t.size.y)
}

/// The cover centre-cropped onto `rect` as ONE textured mesh shaded by vertical colour stops `(fraction 0..1 of the height, tint)`,
/// then rounded by masking the corners with `bg`. One layer, so no overlay edge can leave a light seam along the card's sides.
/// Returns false (painting nothing) until the cover has loaded.
pub fn paint_cover_shaded(ui: &Ui, url: &str, rect: Rect, r: f32, stops: &[(f32, Color32)], bg: Color32) -> bool {
    let Some(texture) = cover_texture(ui.ctx(), url, rect.size()) else { return false };
    let uv = crop_uv(texture.size.x / texture.size.y, rect.width() / rect.height());
    let mut m = egui::Mesh::with_texture(texture.id);
    for (i, (f, c)) in stops.iter().enumerate() {
        let (y, v) = (rect.min.y + rect.height() * f, uv.min.y + uv.height() * f);
        for (x, u) in [(rect.min.x, uv.min.x), (rect.max.x, uv.max.x)] {
            m.vertices.push(egui::epaint::Vertex { pos: egui::pos2(x, y), uv: egui::pos2(u, v), color: *c });
        }
        if i > 0 {
            let b = (i * 2) as u32;
            m.indices.extend([b - 2, b - 1, b + 1, b - 2, b + 1, b]);
        }
    }
    ui.painter().add(m);
    round_top_corners(ui.painter(), rect, r, bg);
    round_bottom_corners(ui.painter(), rect, r, bg);
    true
}

/// Starts loading the cover `paint_cover` would draw for a `size` slot, so it is ready when it is shown.
pub fn preload_cover(ctx: &Context, url: &str, size: egui::Vec2) {
    let _ = cover_texture(ctx, url, size);
}

/// Rewrites a YouTube `=wN-hN` size suffix to `px`; other URLs pass through.
pub fn sized_url(url: &str, px: u32) -> String {
    let Some(at) = url.find("=w") else { return url.to_string() };
    let rest = &url[at + 2..];
    let w = rest.bytes().take_while(u8::is_ascii_digit).count();
    let Some(h) = rest[w..].strip_prefix("-h") else { return url.to_string() };
    let n = h.bytes().take_while(u8::is_ascii_digit).count();
    if w == 0 || n == 0 {
        return url.to_string();
    }
    format!("{}{px}-h{px}{}", &url[..at + 2], &h[n..])
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

/// UVs of `r` when the backdrop is laid over `anchor`, the rect the sharp cover fills. The texture's top half is the cover
/// and its bottom half the reflection, so below `anchor` (v past 0.5) the reflection shows; beyond the texture it clamps.
pub fn backdrop_uv(anchor: Rect, r: Rect) -> Rect {
    let u = |x: f32| (x - anchor.min.x) / anchor.width();
    let v = |y: f32| (y - anchor.min.y) / anchor.height() * 0.5;
    Rect::from_min_max(egui::pos2(u(r.min.x), v(r.min.y)), egui::pos2(u(r.max.x), v(r.max.y)))
}

/// Paints the blurred backdrop texture over `y0..y1` of `card` with alpha rising 0 -> 1, so whatever is beneath dissolves into it.
/// UVs match [`backdrop_uv`] for the same `anchor`.
pub fn fade_blur(painter: &Painter, tex: TextureId, card: Rect, anchor: Rect, y0: f32, y1: f32, alpha: f32) {
    const ROWS: usize = 6;
    let uv = backdrop_uv(anchor, Rect::from_x_y_ranges(card.x_range(), y0.min(y1)..=y0.max(y1)));
    let (vy0, vy1) = if y0 <= y1 { (uv.min.y, uv.max.y) } else { (uv.max.y, uv.min.y) };
    let mut m = egui::Mesh::with_texture(tex);
    for i in 0..ROWS {
        let f = i as f32 / (ROWS - 1) as f32;
        let a = f * f * (3. - 2. * f) * alpha;
        let (y, al) = (y0 + (y1 - y0) * f, (a * 255.) as u8);
        let g = (DIM as f32 * a) as u8;
        let c = Color32::from_rgba_premultiplied(g, g, g, al);
        let v = vy0 + (vy1 - vy0) * f;
        for (x, u) in [(card.min.x, uv.min.x), (card.max.x, uv.max.x)] {
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

/// A cover fetched for the player: its blurred backdrop and dominant tone (hue degrees, chroma), `None` for a grey cover.
struct Art {
    tex: TextureHandle,
    tone: Option<(f32, f32)>,
    /// OKLab lightness of the backdrop's lower part as drawn (after `DIM`), where the player's controls sit.
    luma: f32,
}

/// What a backdrop worker sends back: (url, (blurred image, tone, luma)), `None` when the cover could not be fetched.
type Fetched = (String, Option<(ColorImage, Option<(f32, f32)>, f32)>);

pub struct Gfx {
    /// Header wash and the (s1, accent) it was drawn for.
    wash: Option<(Color32, Color32, TextureHandle)>,
    /// Cover URLs asked for (also the ones that failed, so they are not refetched) and the ones that arrived.
    asked: HashSet<String>,
    // ponytail: never evicted; each entry is a 96px texture, add an LRU if sessions get very long.
    arts: HashMap<String, Art>,
    /// Covers whose fetch failed; they theme like a grey cover instead of leaving the last song's colours up.
    failed: HashSet<String>,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
}

impl Gfx {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Gfx { wash: None, asked: HashSet::new(), arts: HashMap::new(), failed: HashSet::new(), tx, rx }
    }

    /// Redrawn when the theme moves, so it follows the track colours.
    pub fn wash(&mut self, ctx: &Context, t: &Theme) -> TextureId {
        let img = || gradient(256, 96, 135., mix(t.s1, t.ax, 0.24), t.s1, 0.64);
        match &mut self.wash {
            Some((s1, ax, tex)) if (*s1, *ax) != (t.s1, t.ax) => {
                tex.set(img(), TextureOptions::LINEAR);
                (*s1, *ax) = (t.s1, t.ax);
            }
            Some(_) => {}
            None => self.wash = Some((t.s1, t.ax, ctx.load_texture("wash", img(), TextureOptions::LINEAR))),
        }
        self.wash.as_ref().unwrap().2.id()
    }

    /// Fetches the blurred backdrop and tone of the cover at `url` on a worker, once per URL.
    pub fn fetch(&mut self, ctx: &Context, url: &str) {
        if self.asked.insert(url.to_owned()) {
            let (tx, u, ctx) = (self.tx.clone(), url.to_owned(), ctx.clone());
            std::thread::spawn(move || {
                let r = fetch_backdrop(&u);
                let _ = tx.send((u, r));
                ctx.request_repaint();
            });
        }
        while let Ok((k, r)) = self.rx.try_recv() {
            match r {
                Some((img, tone, luma)) => {
                    self.arts.insert(k, Art { tex: ctx.load_texture("backdrop", img, TextureOptions::LINEAR), tone, luma });
                }
                None => {
                    self.failed.insert(k);
                }
            }
        }
    }

    /// Blurred cover for the player backdrop; `None` until it has loaded.
    pub fn backdrop(&mut self, ctx: &Context, url: &str) -> Option<TextureId> {
        self.fetch(ctx, url);
        self.arts.get(url).map(|a| a.tex.id())
    }

    /// `None` until the cover has been fetched (or has failed); then its dominant (hue, chroma), `Some(None)` when grey or failed.
    pub fn tone(&self, url: &str) -> Option<Option<(f32, f32)>> {
        if self.failed.contains(url) {
            return Some(None);
        }
        self.arts.get(url).map(|a| a.tone)
    }

    /// Lightness (OKLab L, 0..1) of the dimmed backdrop under the player's controls; `None` until it arrives.
    pub fn luma(&self, url: &str) -> Option<f32> {
        self.arts.get(url).map(|a| a.luma)
    }
}

/// Blocking: the cover at `url`, centre-cropped square, with its vertical flip below it, blurred as one 96x192 backdrop, plus its tone.
/// Blurring across the seam hides the crease a mirrored wrap used to show.
fn fetch_backdrop(url: &str) -> Option<(ColorImage, Option<(f32, f32)>, f32)> {
    let mut bytes = vec![];
    ureq::get(&sized_url(url, 128)).call().ok()?.into_reader().read_to_end(&mut bytes).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let side = img.width().min(img.height());
    let sq = image::imageops::crop_imm(&img, (img.width() - side) / 2, (img.height() - side) / 2, side, side).to_image();
    let tone = tone(sq.pixels().map(|p| [p[0], p[1], p[2]]));
    let row = side as usize * 4;
    let mut both = sq.as_raw().clone();
    both.extend(sq.as_raw().chunks_exact(row).rev().flatten());
    // 96px with a sigma of 4 keeps the cover's shapes readable; 48px at sigma 6 smeared it into grey clouds once scaled up 7x.
    let out = blur(&ColorImage::from_rgba_unmultiplied([side as usize, 2 * side as usize], &both), 96, 192, 4.);
    let l = luma(&out);
    Some((out, tone, l))
}

/// Mean OKLab lightness of the lower 40% of the cover half of `img` once dimmed by `DIM`: the controls sit there.
fn luma(img: &ColorImage) -> f32 {
    let [w, h] = img.size;
    let dim = |v: u8| (v as u32 * DIM as u32 / 255) as u8;
    let rows = &img.pixels[w * (h / 2 * 6 / 10)..w * (h / 2)];
    rows.iter().map(|p| oklab([dim(p.r()), dim(p.g()), dim(p.b())]).0).sum::<f32>() / rows.len().max(1) as f32
}

/// Chroma-weighted mean of the pixels in OKLab: vivid pixels set the hue, so a mostly grey cover with one red stripe reads red.
/// `None` when the result is too grey to theme around.
fn tone(px: impl Iterator<Item = [u8; 3]>) -> Option<(f32, f32)> {
    let (mut a, mut b, mut w) = (0., 0., 0.);
    for p in px {
        let (_, pa, pb) = oklab(p);
        let c = (pa * pa + pb * pb).sqrt();
        a += pa * c;
        b += pb * c;
        w += c;
    }
    let (a, b) = (a / w.max(1e-6), b / w.max(1e-6));
    let c = (a * a + b * b).sqrt();
    (c > 0.025).then(|| (b.atan2(a).to_degrees().rem_euclid(360.), c))
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

fn blur(src: &ColorImage, w: u32, h: u32, sigma: f32) -> ColorImage {
    let raw: Vec<u8> = src.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), 255]).collect();
    let img = image::RgbaImage::from_raw(src.width() as u32, src.height() as u32, raw).expect("size matches");
    let small = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    let out = image::imageops::blur(&small, sigma);
    ColorImage::from_rgba_unmultiplied([w as usize, h as usize], out.as_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrop_uv_top_half_is_cover_bottom_half_reflection() {
        let anchor = Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(100., 100.));
        let cover = backdrop_uv(anchor, anchor);
        assert!(cover.min.y.abs() < 1e-6 && (cover.max.y - 0.5).abs() < 1e-6);
        let below = backdrop_uv(anchor, anchor.translate(egui::vec2(0., 100.)));
        assert!((below.min.y - 0.5).abs() < 1e-6 && (below.max.y - 1.).abs() < 1e-6);
    }
}
