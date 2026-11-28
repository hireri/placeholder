//! Textures (placeholder art, header wash, blurred backdrop), icons, fonts and small paint helpers.

use crate::theme::{mix, oklab, Theme};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::mpsc::{channel, Receiver, Sender};
use eframe::egui::epaint::text::VariationCoords;
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
    ("bytes://icons/mplay.svg", include_bytes!("../assets/icons/mplay.svg")),
    ("bytes://icons/mnext.svg", include_bytes!("../assets/icons/mnext.svg")),
    ("bytes://icons/mnote.svg", include_bytes!("../assets/icons/mnote.svg")),
    ("bytes://icons/martist.svg", include_bytes!("../assets/icons/martist.svg")),
    ("bytes://icons/mopen.svg", include_bytes!("../assets/icons/mopen.svg")),
    ("bytes://icons/msave.svg", include_bytes!("../assets/icons/msave.svg")),
    ("bytes://icons/munsave.svg", include_bytes!("../assets/icons/munsave.svg")),
    ("bytes://icons/mtrash.svg", include_bytes!("../assets/icons/mtrash.svg")),
    ("bytes://icons/mlink.svg", include_bytes!("../assets/icons/mlink.svg")),
    ("bytes://icons/mgear.svg", include_bytes!("../assets/icons/mgear.svg")),
    ("bytes://icons/macct.svg", include_bytes!("../assets/icons/macct.svg")),
    ("bytes://icons/mkeys.svg", include_bytes!("../assets/icons/mkeys.svg")),
    ("bytes://icons/mspot.svg", include_bytes!("../assets/icons/mspot.svg")),
    ("bytes://icons/mpriv.svg", include_bytes!("../assets/icons/mpriv.svg")),
    ("bytes://icons/homef.svg", include_bytes!("../assets/icons/homef.svg")),
    ("bytes://icons/impf.svg", include_bytes!("../assets/icons/impf.svg")),
    ("bytes://icons/dirf.svg", include_bytes!("../assets/icons/dirf.svg")),
    ("bytes://icons/histf.svg", include_bytes!("../assets/icons/histf.svg")),
    ("bytes://icons/lyrf.svg", include_bytes!("../assets/icons/lyrf.svg")),
    ("bytes://icons/quef.svg", include_bytes!("../assets/icons/quef.svg")),
    ("bytes://icons/partyf.svg", include_bytes!("../assets/icons/partyf.svg")),
    ("bytes://icons/alb.svg", include_bytes!("../assets/icons/alb.svg")),
    ("bytes://icons/aqh.svg", include_bytes!("../assets/icons/aqh.svg")),
    ("bytes://icons/aqs.svg", include_bytes!("../assets/icons/aqs.svg")),
    ("bytes://icons/art.svg", include_bytes!("../assets/icons/art.svg")),
    ("bytes://icons/back.svg", include_bytes!("../assets/icons/back.svg")),
    ("bytes://icons/bell.svg", include_bytes!("../assets/icons/bell.svg")),
    ("bytes://icons/explicit.svg", include_bytes!("../assets/icons/explicit.svg")),
    ("bytes://icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    ("bytes://icons/party.svg", include_bytes!("../assets/icons/party.svg")),
    ("bytes://icons/crown.svg", include_bytes!("../assets/icons/crown.svg")),
    ("bytes://icons/group.svg", include_bytes!("../assets/icons/group.svg")),
    ("bytes://icons/gear.svg", include_bytes!("../assets/icons/gear.svg")),
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
    ("bytes://icons/pin.svg", include_bytes!("../assets/icons/pin.svg")),
    ("bytes://icons/pl.svg", include_bytes!("../assets/icons/pl.svg")),
    ("bytes://icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("bytes://icons/que.svg", include_bytes!("../assets/icons/que.svg")),
    ("bytes://icons/rep.svg", include_bytes!("../assets/icons/rep.svg")),
    ("bytes://icons/rep1.svg", include_bytes!("../assets/icons/rep1.svg")),
    ("bytes://icons/rw.svg", include_bytes!("../assets/icons/rw.svg")),
    ("bytes://icons/serafin.svg", include_bytes!("../assets/logo/serafin.svg")),
    ("bytes://icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("bytes://icons/shuf.svg", include_bytes!("../assets/icons/shuf.svg")),
    ("bytes://icons/viz.svg", include_bytes!("../assets/icons/viz.svg")),
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
    ("bytes://icons/src-youtube.svg", include_bytes!("../assets/sources/youtube.svg")),
    ("bytes://icons/src-jellyfin.svg", include_bytes!("../assets/sources/jellyfin.svg")),
    ("bytes://icons/src-soundcloud.svg", include_bytes!("../assets/sources/soundcloud.svg")),
    ("bytes://icons/src-bandcamp.svg", include_bytes!("../assets/sources/bandcamp.svg")),
    ("bytes://icons/src-generic.svg", include_bytes!("../assets/sources/generic.svg")),
    ("bytes://icons/src-youtube-color.svg", include_bytes!("../assets/sources/youtube-color.svg")),
    ("bytes://icons/src-jellyfin-color.svg", include_bytes!("../assets/sources/jellyfin-color.svg")),
    ("bytes://icons/src-soundcloud-color.svg", include_bytes!("../assets/sources/soundcloud-color.svg")),
    ("bytes://icons/src-bandcamp-color.svg", include_bytes!("../assets/sources/bandcamp-color.svg")),
    ("bytes://icons/src-spotify-color.svg", include_bytes!("../assets/sources/spotify-color.svg")),
    ("bytes://icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("bytes://icons/vid.svg", include_bytes!("../assets/icons/vid.svg")),
    ("bytes://icons/vidoff.svg", include_bytes!("../assets/icons/vidoff.svg"))
];

/// Allo axes besides weight (wght 100..900 is set per entry below). opsz 9..32; egui can't auto-size it, so pick one.
const ALLO_OPSZ: f32 = 9.;

pub fn install(ctx: &Context) {
    egui_extras::install_image_loaders(ctx);
    for (uri, bytes) in ICONS {
        ctx.include_bytes(*uri, *bytes);
    }
    set_fonts(ctx, "");
}

/// Allo (bundled default) in every weight, with the installed font called `user` (if any) ahead of it; the default fonts stay as glyph fallback.
/// Returns false when no installed font has that family name (the UI then keeps Allo).
pub fn set_fonts(ctx: &Context, user: &str) -> bool {
    use fontique::{Collection, CollectionOptions, QueryFamily, QueryStatus, SourceCache};
    let mut defs = egui::FontDefinitions::default();
    let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // One font entry per weight so FontId can pick weight through its family.
    let weights = [("400", 400., FontFamily::Proportional), ("500", 500., w(Weight::Medium)), ("600", 600., w(Weight::Semi)), ("650", 650., w(Weight::Bold)), ("800", 800., w(Weight::Extra))];
    let user = user.trim();
    let mut coll = Collection::new(CollectionOptions { shared: false, system_fonts: !user.is_empty() });
    let mut cache = SourceCache::new(Default::default());
    let mut ok = true;
    // Japanese glyphs Allo lacks, in a rounded face that suits it.
    defs.font_data.insert("kosugi".into(), FontData::from_static(include_bytes!("../assets/fonts/KosugiMaru-Regular.ttf")).into());
    for (tag, wght, fam) in weights {
        // The installed face closest to this weight (a variable font is also set to it); the same family for every weight of the UI.
        let mine = (!user.is_empty()).then(|| {
            let mut q = coll.query(&mut cache);
            q.set_families([QueryFamily::Named(user)]);
            q.set_attributes(fontique::Attributes::new(fontique::FontWidth::NORMAL, fontique::FontStyle::Normal, fontique::FontWeight::new(wght)));
            let mut out = None;
            q.matches_with(|f| {
                let d = FontData { index: f.index, ..FontData::from_owned(f.blob.as_ref().to_vec()) };
                out = Some(d.tweak(egui::FontTweak { coords: VariationCoords::new([(b"wght", wght)]), ..Default::default() }));
                QueryStatus::Stop
            });
            out
        }).flatten();
        ok &= mine.is_some() || user.is_empty();
        let data = FontData::from_static(include_bytes!("../assets/fonts/AlloVariable-Round60.ttf"))
            .tweak(egui::FontTweak { coords: VariationCoords::new([(b"wght", wght), (b"opsz", ALLO_OPSZ)]), ..Default::default() });
        defs.font_data.insert(format!("allo-{tag}"), data.into());
        let mut list = vec![];
        if let Some(d) = mine {
            defs.font_data.insert(format!("user-{tag}"), d.into());
            list.push(format!("user-{tag}"));
        }
        list.push(format!("allo-{tag}"));
        list.push("kosugi".into());
        list.extend(fallback.iter().cloned());
        defs.families.insert(fam, list);
    }
    ctx.set_fonts(defs);
    ok
}

#[derive(Clone, Copy)]
pub enum Weight {
    Medium,
    Semi,
    Bold,
    Extra,
}

fn w(weight: Weight) -> FontFamily {
    FontFamily::Name(match weight { Weight::Medium => "w500", Weight::Semi => "w600", Weight::Bold => "w650", Weight::Extra => "w800" }.into())
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn font_w(size: f32, weight: Weight) -> FontId {
    FontId::new(size, w(weight))
}

/// Standard ease-out for hover/fade transitions (egui's default animate_* is linear).
pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

static REDUCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The "Reduce motion" preference; `METROLIST_REDUCE_MOTION` forces it on.
pub fn set_reduced_motion(on: bool) {
    REDUCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// True when motion should be skipped: the preference, or `METROLIST_REDUCE_MOTION` set.
pub fn reduced_motion() -> bool {
    static ENV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    REDUCED.load(std::sync::atomic::Ordering::Relaxed) || *ENV.get_or_init(|| std::env::var_os("METROLIST_REDUCE_MOTION").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Gentle ease-out (`cubic-bezier(0.22, 1, 0.36, 1)`) for slides that should settle softly.
pub fn ease_out_soft(t: f32) -> f32 {
    bezier(0.22, 1., 0.36, 1., t)
}

/// CSS-style cubic-bezier easing (Compose `CubicBezierEasing`): finds the curve parameter whose x is `t`, returns its y.
pub fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let at = |a: f32, b: f32, s: f32| 3. * a * s * (1. - s).powi(2) + 3. * b * s * s * (1. - s) + s * s * s;
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let m = (lo + hi) / 2.;
        if at(x1, x2, m) < t { lo = m } else { hi = m }
    }
    at(y1, y2, (lo + hi) / 2.)
}

/// Tween toward `target` over `secs` with `ease`; a new target starts from the value on screen, so it is interruptible.
pub fn tween(ctx: &Context, id: egui::Id, target: f32, secs: f32, ease: impl Fn(f32) -> f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let (from, to, t0) = ctx.data(|d| d.get_temp::<(f32, f32, f64)>(id)).unwrap_or((target, target, f64::NEG_INFINITY));
    let k = |t0: f64| ((now - t0) / secs as f64).clamp(0., 1.) as f32;
    let (from, t0) = if to != target { (egui::lerp(from..=to, ease(k(t0))), now) } else { (from, t0) };
    ctx.data_mut(|d| d.insert_temp(id, (from, target, t0)));
    if k(t0) < 1. {
        ctx.request_repaint();
    }
    egui::lerp(from..=target, ease(k(t0)))
}

/// Gaussian blur of a galley (`sigma` in points) as copies at whole physical-pixel offsets: egui snaps text to the pixel grid, so
/// sub-pixel taps would collapse onto each other. Each tap's alpha is `1-(1-a)^w`, so a stroke's interior composites to exactly `a`.
pub fn blur_galley(p: &Painter, g: &std::sync::Arc<egui::Galley>, pos: Pos2, rgb: Color32, a: f32, sigma: f32) {
    let ppp = p.ctx().pixels_per_point();
    let s = sigma * ppp;
    let a = a.min(0.98);
    if s < 0.35 {
        p.galley_with_override_text_color(pos, g.clone(), rgb.gamma_multiply(a));
        return;
    }
    let step = (s / 1.2).ceil().max(1.);
    let n = (2. * s / step).ceil() as i32;
    let mut taps = vec![];
    let mut sum = 0.;
    for j in -n..=n {
        for i in -n..=n {
            let (dx, dy) = (i as f32 * step, j as f32 * step);
            let w = (-(dx * dx + dy * dy) / (2. * s * s)).exp();
            if w > 0.1 {
                taps.push((dx / ppp, dy / ppp, w));
                sum += w;
            }
        }
    }
    for (dx, dy, w) in taps {
        p.galley_with_override_text_color(pos + egui::vec2(dx, dy), g.clone(), rgb.gamma_multiply(1. - (1. - a).powf(w / sum)));
    }
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
    corner_wedges(painter, rect, r, bg, true, None);
}

/// Bottom corners again, for a card that casts `shadow(.., dy, blur, alpha)`: the wedges carry the shadow's falloff
/// so they don't show up as flat squares of `bg` inside the shadow.
pub fn round_bottom_corners_shadowed(painter: &Painter, rect: Rect, r: f32, bg: Color32, dy: i8, blur: u8, alpha: u8) {
    corner_wedges(painter, rect, r, bg, true, Some((rect, r, dy as f32, blur as f32 / 2., shadow_alpha(painter, alpha) as f32 / 255.)));
}

/// Same for the top corners.
pub fn round_top_corners(painter: &Painter, rect: Rect, r: f32, bg: Color32) {
    corner_wedges(painter, rect, r, bg, false, None);
}

/// Signed distance from `p` to a rounded rect (negative inside).
fn rrect_sd(p: egui::Pos2, rect: Rect, r: f32) -> f32 {
    let q = (p - rect.center()).abs() - rect.size() / 2. + egui::vec2(r, r);
    q.max(egui::Vec2::ZERO).length() + q.x.max(q.y).min(0.) - r
}

fn corner_wedges(painter: &Painter, rect: Rect, r: f32, bg: Color32, bottom: bool, lit: Option<(Rect, f32, f32, f32, f32)>) {
    const N: usize = 12;
    // Colour of `bg` under the shadow at `p`; the logistic stands in for the Gaussian falloff.
    let shade = |p: egui::Pos2| match lit {
        Some((card, cr, dy, sigma, a)) => {
            let d = rrect_sd(p - egui::vec2(0., dy), card, cr);
            bg.lerp_to_gamma(Color32::BLACK, a / (1. + (1.7 * d / sigma).exp()))
        }
        None => bg,
    };
    let (ey, sy) = if bottom { (rect.max.y, 1.) } else { (rect.min.y, -1.) };
    let mut mesh = egui::Mesh::default();
    for (cx, sx) in [(rect.min.x + r, -1.), (rect.max.x - r, 1.)] {
        let centre = egui::pos2(cx, ey - sy * r);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(cx + sx * r, ey), shade(egui::pos2(cx + sx * r, ey)));
        for i in 0..=N {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / N as f32;
            let at = centre + egui::vec2(sx * r * a.sin(), sy * r * a.cos());
            mesh.colored_vertex(at, shade(at));
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

/// Dark-theme alphas read as dirty smudges on light surfaces, so they are softened there.
fn shadow_alpha(painter: &Painter, alpha: u8) -> u8 {
    if painter.ctx().global_style().visuals.dark_mode { alpha } else { (alpha as f32 * 0.4) as u8 }
}

pub fn shadow(painter: &Painter, rect: Rect, r: f32, dy: i8, blur: u8, alpha: u8) {
    let alpha = shadow_alpha(painter, alpha);
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

/// [`trunc`] for a track title; an explicit track gets the "E" mark right after the text. Returns the text's rect.
pub fn title_trunc(ui: &Ui, painter: &Painter, pos: Pos2, max_w: f32, text: &str, font: FontId, color: Color32, explicit: Option<(f32, Color32)>) -> Rect {
    let Some((size, tint)) = explicit else { return trunc(painter, pos, max_w, text, font, color) };
    let r = trunc(painter, pos, max_w - size - 5., text, font, color);
    icon(ui, icon!("explicit"), egui::pos2(r.max.x + 5. + size / 2., pos.y), size, tint);
    r
}

pub fn text_w(painter: &Painter, text: &str, font: FontId) -> f32 {
    painter.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x
}

/// Like [`icon_button`], but hover and press are a translucent wash of the text colour, so what lies beneath shows through.
pub fn glass_button(ui: &Ui, t: &Theme, key: impl std::hash::Hash + std::fmt::Debug, center: Pos2, size: f32, uri: &'static str, glyph: f32, color: Color32) -> Response {
    let resp = press(ui, key, Rect::from_center_size(center, egui::vec2(size, size)));
    let a = if resp.is_pointer_button_down_on() { 0.18 } else if resp.hovered() { 0.1 } else { 0. };
    if a > 0. {
        ui.painter().circle_filled(center, size / 2., t.tx.gamma_multiply(a));
    }
    icon(ui, uri, center, glyph, color);
    resp
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

/// egui's image loaders keep bytes, decoded pixels and a GPU texture per URI forever; track last use so `trim_covers` can bound that.
static COVER_KEEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(96);

pub fn set_cover_limit(n: u16) {
    COVER_KEEP.store(n as usize, std::sync::atomic::Ordering::Relaxed);
}

/// Covers held in memory and what the image loaders use in total (icons included, a few KB).
pub fn cover_usage(ctx: &Context) -> (usize, usize) {
    let l = ctx.loaders();
    let bytes = l.bytes.lock().iter().map(|x| x.byte_size()).sum::<usize>() + l.image.lock().iter().map(|x| x.byte_size()).sum::<usize>() + l.texture.lock().iter().map(|x| x.byte_size()).sum::<usize>();
    (ctx.data_mut(|d| d.get_temp_mut_or_default::<HashMap<String, u64>>(egui::Id::NULL).len()), bytes)
}

/// Forgets every cover (not the bundled icons); they reload when drawn again.
pub fn clear_covers(ctx: &Context) {
    let all: Vec<String> = ctx.data_mut(|d| d.get_temp_mut_or_default::<HashMap<String, u64>>(egui::Id::NULL).drain().map(|(k, _)| k).collect());
    for uri in all {
        ctx.forget_image(&uri);
    }
}

pub fn touch_cover(ctx: &Context, uri: &str) {
    let frame = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.get_temp_mut_or_default::<HashMap<String, u64>>(egui::Id::NULL).insert(uri.to_owned(), frame));
}

/// Forgets the least recently drawn covers beyond `COVER_KEEP`; call once per frame.
pub fn trim_covers(ctx: &Context) {
    let keep = COVER_KEEP.load(std::sync::atomic::Ordering::Relaxed);
    let stale: Vec<String> = ctx.data_mut(|d| {
        let m = d.get_temp_mut_or_default::<HashMap<String, u64>>(egui::Id::NULL);
        if m.len() <= keep {
            return vec![];
        }
        let mut by_age: Vec<_> = m.iter().map(|(k, v)| (*v, k.clone())).collect();
        by_age.sort_unstable();
        let drop: Vec<String> = by_age.into_iter().take(m.len() - keep).map(|(_, k)| k).collect();
        for k in &drop { m.remove(k); }
        drop
    });
    for uri in stale {
        ctx.forget_image(&uri);
    }
}

/// The cover at `url` for a `size` slot, `None` until one has loaded. The CDN bucket is roughly the pixels drawn, in 64px steps,
/// and never below the largest bucket already loaded for this URL, so a resize keeps the sharp image up while a bigger one loads.
pub fn cover_texture(ctx: &Context, url: &str, size: egui::Vec2) -> Option<SizedTexture> {
    cover_loaded(ctx, url, size).map(|(t, _)| t)
}

/// [`cover_texture`] plus the uri that was loaded, so a variant of the same pixels can be derived from its cached bytes.
fn cover_loaded(ctx: &Context, url: &str, size: egui::Vec2) -> Option<(SizedTexture, String)> {
    let key = egui::Id::unique(("cover-px", url));
    let shown = ctx.data(|d| d.get_temp::<u32>(key)).unwrap_or(0);
    let want = ((size.x.max(size.y) * ctx.pixels_per_point() / 64.).ceil() * 64.).clamp(64., 800.) as u32;
    let load = |px: u32| {
        let uri = sized_url(url, px);
        touch_cover(ctx, &uri);
        match Image::new(uri.clone()).texture_options(egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR }).load_for_size(ctx, size) {
            Ok(TexturePoll::Ready { texture }) => Some((texture, uri)),
            _ => None,
        }
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

/// True once the cover at `url` has definitely failed to load (a 404, or bytes that are not an image). Only ask while it has not
/// loaded: it requests the same size bucket `cover_texture` would, so asking about a loaded cover could fetch another size.
pub fn cover_broken(ctx: &Context, url: &str, size: egui::Vec2) -> bool {
    let want = ((size.x.max(size.y) * ctx.pixels_per_point() / 64.).ceil() * 64.).clamp(64., 800.) as u32;
    Image::new(sized_url(url, want)).load_for_size(ctx, size).is_err()
}

/// Paints the cover at `url` centre-cropped to fill `rect` (never stretched). Nothing paints until a cover for it has loaded.
pub fn paint_cover(ui: &Ui, url: &str, rect: Rect, corners: CornerRadius, tint: Color32) {
    let Some(texture) = cover_texture(ui.ctx(), url, rect.size()) else { return };
    let uv = crop_uv(texture.size.x / texture.size.y, rect.width() / rect.height());
    Image::new(texture).fit_to_exact_size(rect.size()).uv(uv).corner_radius(corners).tint(tint).paint_at(ui, rect);
}

/// A greyscale copy of the cover at `uri`, decoded once from the bytes the colour version already loaded; `None` until they are there.
fn gray_texture(ctx: &Context, uri: &str) -> Option<egui::TextureHandle> {
    let cache = egui::Id::NULL.with("gray-covers");
    if let Some(t) = ctx.data(|d| d.get_temp::<HashMap<String, egui::TextureHandle>>(cache)).and_then(|m| m.get(uri).cloned()) {
        return Some(t);
    }
    let Ok(egui::load::BytesPoll::Ready { bytes, .. }) = ctx.try_load_bytes(uri) else { return None };
    let img = image::load_from_memory(&bytes).ok()?.grayscale().to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let tex = ctx.load_texture(format!("gray:{uri}"), egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| {
        let m = d.get_temp_mut_or_default::<HashMap<String, egui::TextureHandle>>(cache);
        if m.len() >= 64 {
            m.clear(); // ponytail: wholesale reset past 64 covers; few items are ever greyed at once
        }
        m.insert(uri.to_owned(), tex.clone());
    });
    Some(tex)
}

/// [`paint_cover`] in greyscale (the colour cover shows for the frame or two until the grey copy is decoded).
pub fn paint_cover_gray(ui: &Ui, url: &str, rect: Rect, corners: CornerRadius, tint: Color32) {
    let Some((color, uri)) = cover_loaded(ui.ctx(), url, rect.size()) else { return };
    let uv = crop_uv(color.size.x / color.size.y, rect.width() / rect.height());
    let texture = gray_texture(ui.ctx(), &uri).map_or(color, |h| SizedTexture::from_handle(&h));
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

/// Rewrites a YouTube `=wN-hN` size suffix so its longer side is `px`, keeping the aspect ratio (video thumbnails are 16:9); other URLs pass through.
pub fn sized_url(url: &str, px: u32) -> String {
    let Some(at) = url.find("=w") else { return url.to_string() };
    let rest = &url[at + 2..];
    let w = rest.bytes().take_while(u8::is_ascii_digit).count();
    let Some(h) = rest[w..].strip_prefix("-h") else { return url.to_string() };
    let n = h.bytes().take_while(u8::is_ascii_digit).count();
    if w == 0 || n == 0 {
        return url.to_string();
    }
    let (sw, sh): (f32, f32) = (rest[..w].parse().unwrap_or(1.), h[..n].parse().unwrap_or(1.));
    let k = px as f32 / sw.max(sh);
    format!("{}{}-h{}{}", &url[..at + 2], (sw * k).round() as u32, (sh * k).round() as u32, &h[n..])
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

/// Vertical fade from `to` at the top to transparent at the bottom.
pub fn fade_up(painter: &Painter, rect: Rect, to: Color32) {
    let mut m = egui::Mesh::default();
    let (a, b) = (to, to.gamma_multiply(0.));
    for (p, c) in [(rect.left_top(), a), (rect.right_top(), a), (rect.right_bottom(), b), (rect.left_bottom(), b)] {
        m.vertices.push(egui::epaint::Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: c });
    }
    m.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(m);
}

/// Fades a scroll area's top and bottom edges into `bg` while more content lies past them. Call right after `show`, with the
/// ui the area was shown in, so the fades land over the content.
pub fn scroll_fades<R>(ui: &egui::Ui, out: &egui::scroll_area::ScrollAreaOutput<R>, bg: Color32) {
    let (r, y) = (out.inner_rect, out.state.clamped_offset().y);
    let ctx = ui.ctx();
    let top = ctx.calm_bool(out.id.with("fade-top"), y > 0.5, 0.15);
    let bottom = ctx.calm_bool(out.id.with("fade-bottom"), y + r.height() < out.content_size.y - 0.5, 0.15);
    let p = ui.painter();
    if top > 0. {
        fade_up(p, Rect::from_min_size(r.min, egui::vec2(r.width(), 32.)), bg.gamma_multiply(top));
    }
    if bottom > 0. {
        fade_down(p, Rect::from_min_max(egui::pos2(r.min.x, r.max.y - 40.), r.max), bg.gamma_multiply(bottom));
    }
}

/// Scrollbar for a page whose tall header overlaps the top of the scroll area: the track starts at `top` (the header's
/// bottom) instead of behind it. The area itself must hide its own bar. Hover/drag only; hidden when nothing scrolls.
pub fn page_bar<R>(ui: &egui::Ui, id: egui::Id, out: &egui::scroll_area::ScrollAreaOutput<R>, top: f32, color: Color32) {
    let (r, content) = (out.inner_rect, out.content_size.y);
    let view = r.height();
    if content <= view + 0.5 {
        return;
    }
    let track = Rect::from_min_max(egui::pos2(r.max.x - 10., top.max(r.min.y) + 4.), egui::pos2(r.max.x - 2., r.max.y - 4.));
    let th = (track.height() * view / content).max(28.).min(track.height());
    let off = out.state.clamped_offset().y;
    let free = (track.height() - th).max(1.);
    let y = track.min.y + free * off / (content - view);
    let thumb = Rect::from_min_size(egui::pos2(track.min.x, y), egui::vec2(track.width(), th));
    let resp = ui.interact(thumb.expand2(egui::vec2(4., 0.)), id.with("page-bar"), Sense::drag());
    if resp.dragged() {
        let to = (off + resp.drag_delta().y * (content - view) / free).clamp(0., content - view);
        ui.ctx().data_mut(|d| d.insert_temp(ui.scope_id().with("page-bar-set"), to));
        ui.ctx().request_repaint();
    }
    let a = if resp.hovered() || resp.dragged() { 1. } else { 0.6 };
    ui.painter().rect_filled(thumb, 4., color.gamma_multiply(a));
}

/// Offset a [`page_bar`] drag asked for last frame; apply it with `ScrollArea::vertical_scroll_offset` before `show`.
pub fn page_bar_set(ui: &egui::Ui) -> Option<f32> {
    ui.ctx().data_mut(|d| d.remove_temp(ui.scope_id().with("page-bar-set")))
}

/// Horizontal fade into `to`: transparent at one edge, solid at the right edge (`solid_right`) or the left one.
pub fn fade_h(painter: &Painter, rect: Rect, to: Color32, solid_right: bool) {
    let mut m = egui::Mesh::default();
    let (a, b) = if solid_right { (to.gamma_multiply(0.), to) } else { (to, to.gamma_multiply(0.)) };
    for (p, c) in [(rect.left_top(), a), (rect.right_top(), b), (rect.right_bottom(), b), (rect.left_bottom(), a)] {
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
    fade_blur_under(painter, tex, card, anchor, y0, y1, alpha, 0.);
}

/// [`fade_blur`] for a layer that a second fade of share `over` is painted on top of: its alpha is cut so the two stacked fades
/// cover the cover as strongly as one (`1 - (1 - a1)(1 - a2) = a`) instead of compounding.
pub fn fade_blur_under(painter: &Painter, tex: TextureId, card: Rect, anchor: Rect, y0: f32, y1: f32, alpha: f32, over: f32) {
    const ROWS: usize = 6;
    let uv = backdrop_uv(anchor, Rect::from_x_y_ranges(card.x_range(), y0.min(y1)..=y0.max(y1)));
    let (vy0, vy1) = if y0 <= y1 { (uv.min.y, uv.max.y) } else { (uv.max.y, uv.min.y) };
    let mut m = egui::Mesh::with_texture(tex);
    for i in 0..ROWS {
        let f = i as f32 / (ROWS - 1) as f32;
        let a = f * f * (3. - 2. * f) * alpha;
        let a = a * (1. - over) / (1. - a * over);
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
    /// Mean OKLab lightness of each row of the backdrop as drawn (after `DIM`), so any band of it can be read off.
    luma: Vec<f32>,
}

/// What a backdrop worker sends back: (url, (blurred image, tone, luma)), `None` when the cover could not be fetched.
type Fetched = (String, Option<(ColorImage, Option<(f32, f32)>, Vec<f32>)>);

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

    /// Header wash tinted by the cover at `url` (the theme's accent until its tone arrives or when it is grey), redrawn as the
    /// tint or the theme's surface moves. Pages don't transition, so the tint doesn't either.
    pub fn wash(&mut self, ctx: &Context, t: &Theme, url: Option<&str>) -> TextureId {
        if let Some(u) = url {
            self.fetch(ctx, u);
        }
        let want = match url.and_then(|u| self.tone(u)) {
            Some(Some(tone)) => Theme::tinted(tone, t.dark).ax,
            _ => t.ax,
        };
        let ax = want;
        let img = || gradient(256, 96, 135., mix(t.s1, ax, 0.24), t.s1, 0.64);
        match &mut self.wash {
            Some((s1, a, tex)) if (*s1, *a) != (t.s1, ax) => {
                tex.set(img(), TextureOptions::LINEAR);
                (*s1, *a) = (t.s1, ax);
            }
            Some(_) => {}
            None => self.wash = Some((t.s1, ax, ctx.load_texture("wash", img(), TextureOptions::LINEAR))),
        }
        self.wash.as_ref().unwrap().2.id()
    }

    /// Fetches the blurred backdrop and tone of the cover at `url` on a worker, once per URL.
    pub fn fetch(&mut self, ctx: &Context, url: &str) {
        if self.asked.insert(url.to_owned()) {
            let (tx, u, ctx) = (self.tx.clone(), url.to_owned(), ctx.clone());
            std::thread::spawn(move || {
                // A transient network error must not leave this song unthemed for the whole session (`asked` never retries).
                let r = (0..3).find_map(|i| {
                    if i > 0 {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                    fetch_backdrop(&u)
                });
                let _ = tx.send((u, r));
                ctx.request_repaint();
            });
        }
        while let Ok((k, r)) = self.rx.try_recv() {
            match r {
                Some((img, tone, luma)) => {
                    self.arts.insert(k, Art { tex: ctx.load_texture("backdrop", img, TextureOptions { wrap_mode: egui::TextureWrapMode::MirroredRepeat, ..TextureOptions::LINEAR }), tone, luma });
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

    /// Lightness (OKLab L, 0..1) of the dimmed backdrop over texture rows `v0..v1` (0..1 of the whole texture, the cover being 0..0.5);
    /// `None` until it arrives.
    pub fn luma(&self, url: &str, v0: f32, v1: f32) -> Option<f32> {
        let rows = &self.arts.get(url)?.luma;
        let n = rows.len() as f32;
        let (a, b) = (((v0.clamp(0., 1.) * n) as usize).min(rows.len() - 1), (v1.clamp(0., 1.) * n).ceil() as usize);
        let band = &rows[a..b.clamp(a + 1, rows.len())];
        Some(band.iter().sum::<f32>() / band.len() as f32)
    }
}

/// Blocking: the cover at `url`, centre-cropped square, with its vertical flip below it, blurred as one 96x192 backdrop, plus its tone.
/// Blurring across the seam hides the crease a mirrored wrap used to show.
fn fetch_backdrop(url: &str) -> Option<(ColorImage, Option<(f32, f32)>, Vec<f32>)> {
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
    let l = row_luma(&out);
    Some((out, tone, l))
}

/// Mean OKLab lightness of every row of `img` once dimmed by `DIM`.
fn row_luma(img: &ColorImage) -> Vec<f32> {
    let dim = |v: u8| (v as u32 * DIM as u32 / 255) as u8;
    img.pixels.chunks_exact(img.size[0]).map(|row| row.iter().map(|p| oklab([dim(p.r()), dim(p.g()), dim(p.b())]).0).sum::<f32>() / row.len() as f32).collect()
}

/// Dominant hue of the pixels in OKLab, weighted by chroma so vivid pixels win: a mostly grey cover with one red stripe reads red.
/// Averaging a/b over the whole cover would let opposite hues (red and teal) cancel to grey, so the winning 90° sector of the hue wheel is
/// averaged alone. `None` when the result is too grey to theme around.
fn tone(px: impl Iterator<Item = [u8; 3]>) -> Option<(f32, f32)> {
    const BINS: usize = 12;
    let (mut w, mut sums) = ([0f32; BINS], [(0f32, 0f32, 0f32); BINS]);
    for p in px {
        let (_, a, b) = oklab(p);
        let c = (a * a + b * b).sqrt();
        let bin = ((b.atan2(a).to_degrees().rem_euclid(360.) / 360. * BINS as f32) as usize).min(BINS - 1);
        w[bin] += c;
        let s = &mut sums[bin];
        (s.0, s.1, s.2) = (s.0 + a * c, s.1 + b * c, s.2 + c * c);
    }
    // A sector is its bin plus both neighbours, so a hue straddling a bin edge is not split in half.
    let best = (0..BINS).max_by(|&i, &j| {
        let sum = |k: usize| w[(k + BINS - 1) % BINS] + w[k] + w[(k + 1) % BINS];
        sum(i).total_cmp(&sum(j))
    })?;
    let (mut a, mut b, mut cc, mut weight) = (0., 0., 0., 0.);
    for k in [(best + BINS - 1) % BINS, best, (best + 1) % BINS] {
        a += sums[k].0;
        b += sums[k].1;
        cc += sums[k].2;
        weight += w[k];
    }
    let weight = weight.max(1e-6);
    // Chroma-weighted mean chroma of the sector (sum c² / sum c), the same quantity the old vector mean gave for a single hue.
    let c = cc / weight;
    (c > 0.025).then(|| ((b / weight).atan2(a / weight).to_degrees().rem_euclid(360.), c))
}

pub fn sized(id: TextureId, size: f32) -> SizedTexture {
    SizedTexture::new(id, egui::vec2(size, size))
}

/// The Liked songs / favourites cover: `base` as a diagonal gradient, rebuilt only when the colour changes.
pub fn liked_cover(ctx: &Context, base: Color32) -> SizedTexture {
    let key = egui::Id::NULL.with("liked-cover");
    let img = || gradient(64, 64, 135., mix(base, Color32::WHITE, 0.3), mix(base, Color32::BLACK, 0.25), 1.);
    // The texture calls take the context lock themselves, so they must not run inside `data_mut`.
    let tex = match ctx.data(|d| d.get_temp::<(Color32, egui::TextureHandle)>(key)) {
        Some((c, tex)) if c == base => tex,
        found => {
            let tex = match found {
                Some((_, mut tex)) => {
                    tex.set(img(), egui::TextureOptions::LINEAR);
                    tex
                }
                None => ctx.load_texture("liked-cover", img(), egui::TextureOptions::LINEAR),
            };
            ctx.data_mut(|d| d.insert_temp(key, (base, tex.clone())));
            tex
        }
    };
    SizedTexture::from_handle(&tex)
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
    fn tone_keeps_a_hue_when_opposite_hues_would_cancel() {
        let red = [220, 40, 40];
        let teal = [40, 200, 200];
        let (hue, c) = tone(std::iter::repeat(red).take(60).chain(std::iter::repeat(teal).take(40))).expect("two vivid hues still theme");
        assert!(c > 0.1 && !(60. ..300.).contains(&hue), "hue {hue} should be the red side, not the cancelled mean");
        assert!(tone(std::iter::repeat([120, 120, 120]).take(100)).is_none());
    }

    #[test]
    fn backdrop_uv_top_half_is_cover_bottom_half_reflection() {
        let anchor = Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(100., 100.));
        let cover = backdrop_uv(anchor, anchor);
        assert!(cover.min.y.abs() < 1e-6 && (cover.max.y - 0.5).abs() < 1e-6);
        let below = backdrop_uv(anchor, anchor.translate(egui::vec2(0., 100.)));
        assert!((below.min.y - 0.5).abs() < 1e-6 && (below.max.y - 1.).abs() < 1e-6);
    }
}

/// egui's `animate_*` with the duration forced to zero under "Reduce motion", so every transition snaps.
pub trait Calm {
    fn calm_bool(&self, id: egui::Id, on: bool, secs: f32) -> f32;
    fn calm_bool_ease(&self, id: egui::Id, on: bool, secs: f32, ease: fn(f32) -> f32) -> f32;
    fn calm_value(&self, id: egui::Id, to: f32, secs: f32) -> f32;
}

impl Calm for egui::Context {
    fn calm_bool(&self, id: egui::Id, on: bool, secs: f32) -> f32 {
        self.animate_bool_with_time(id, on, if reduced_motion() { 0. } else { secs })
    }
    fn calm_bool_ease(&self, id: egui::Id, on: bool, secs: f32, ease: fn(f32) -> f32) -> f32 {
        self.animate_bool_with_time_and_easing(id, on, if reduced_motion() { 0. } else { secs }, ease)
    }
    fn calm_value(&self, id: egui::Id, to: f32, secs: f32) -> f32 {
        self.animate_value_with_time(id, to, if reduced_motion() { 0. } else { secs })
    }
}
