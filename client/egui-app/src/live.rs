//! Pages backed by the daemon: home feed, album/artist/playlist detail and the account settings.

use crate::gfx::Calm;
use crate::app::{App, Route};
use crate::backend;
use crate::data::{Item, Section, State, Track};
use crate::gfx::*;
use crate::menu::Target;
use crate::page::{Look, HEAD, HERO_H, HERO_MIN};
use eframe::egui::{self, pos2, Image, vec2, Align2, Color32, Rect, ScrollArea};
use std::borrow::Cow;

/// Seconds a carousel arrow takes to slide the row.
const CAR_SLIDE: f64 = 0.45;
pub const CHIPS: [(&str, &str); 5] = [("All", ""), ("Songs", "song"), ("Albums", "album"), ("Playlists", "playlist"), ("Artists", "artist")];
const SEARCH_CHIPS: [&str; 5] = ["All", "Songs", "Videos", "Albums", "Artists"];
const TOP_W: f32 = 340.;
const TOP_H: f32 = 196.;
const STRIP_H: f32 = 72.;

/// Detail endpoint and route an item opens; `None` for songs.
pub fn target(it: &Item) -> Option<(&'static str, Route)> {
    Some(match it.r#type.as_str() {
        "album" => ("albums", Route::Album),
        "playlist" => ("playlists", Route::Album),
        "artist" => ("artists", Route::Artist),
        "spotify" => ("spotify", Route::Album),
        _ => return None,
    })
}

/// The Liked songs playlist (`LM`) or a source's favourites (`<source>:favorites`).
pub fn is_liked_list(id: &str) -> bool {
    id == "LM" || id.ends_with(":favorites")
}

/// Columns and card width for a grid across `w` pixels whose cards are at least `min` wide, `gap` apart.
pub fn grid(w: f32, min: f32, gap: f32) -> (usize, f32) {
    let cols = (((w + gap) / (min + gap)) as usize).max(1);
    (cols, (w - gap * (cols - 1) as f32) / cols as f32)
}

impl App {
    pub fn note(&self, ui: &egui::Ui, card: Rect, top: f32, text: &str) {
        trunc(ui.painter(), pos2(card.min.x + 22., top + 20.), card.width() - 44., text, font(13.), self.t.tf);
    }

    /// Thumbnail from a URL (loaded and cached by egui), over a placeholder fill.
    pub fn thumb(&self, ui: &egui::Ui, url: Option<&str>, rect: Rect, r: f32) {
        self.thumb_g(ui, url, rect, r, false);
    }

    /// [`thumb`](Self::thumb), in greyscale when `gray` (an item whose source is off).
    pub fn thumb_g(&self, ui: &egui::Ui, url: Option<&str>, rect: Rect, r: f32, gray: bool) {
        self.thumb_l(ui, url, rect, r, gray, false);
    }

    /// [`thumb_g`](Self::thumb_g); `liked` marks the Liked songs playlist, whose placeholder is a filled star.
    pub fn thumb_l(&self, ui: &egui::Ui, url: Option<&str>, rect: Rect, r: f32, gray: bool, liked: bool) {
        let side = rect.width().min(rect.height());
        if liked {
            // The source's own cover paints over this once it has loaded; without one this is the cover.
            Image::new(liked_cover(ui.ctx(), self.t.on_ax)).fit_to_exact_size(rect.size()).corner_radius(rad(r)).paint_at(ui, rect);
            icon(ui, "bytes://icons/starf.svg", rect.center(), side * 0.5, self.t.ax);
        } else {
            ui.painter().rect_filled(rect, rad(r), self.t.s3);
            // Round slots are artists, square ones tracks.
            let round = r * 2. >= side - 1.;
            icon(ui, if round { "bytes://icons/art.svg" } else { "bytes://icons/song.svg" }, rect.center(), side * 0.45, self.t.tf);
        }
        if let Some(u) = url {
            if gray { paint_cover_gray(ui, u, rect, rad(r), Color32::WHITE) } else { paint_cover(ui, u, rect, rad(r), Color32::WHITE) }
        }
    }

    /// Round profile picture; the name's initial on the accent colour while there is none (or it is still loading).
    pub fn avatar(&self, ui: &egui::Ui, url: Option<&str>, c: egui::Pos2, r: f32, name: Option<&str>) {
        let p = ui.painter();
        p.circle_filled(c, r, self.t.axc);
        p.text(c, Align2::CENTER_CENTER, name.and_then(|n| n.chars().next()).unwrap_or('?').to_uppercase().to_string(), font_w(r * 0.75, Weight::Semi), self.t.tx);
        if let Some(u) = url {
            paint_cover(ui, u, Rect::from_center_size(c, vec2(r * 2., r * 2.)), rad(r), Color32::WHITE);
        }
    }

    /// Opens an item's page: albums, playlists and artists their own, a song the album it sits on (or its single).
    pub(crate) fn open_item(&mut self, _ctx: &egui::Context, it: &Item) {
        if it.r#type == "song" {
            return self.open_song(it.track());
        }
        let Some((kind, route)) = target(it) else { return };
        self.peek = Some(it.clone());
        self.open(route, Some((kind, it.id.clone())));
    }

    /// Plays an item. A song plays from itself through the song items of `queue` (the shelf it is in); the rest play once their page has loaded.
    pub fn play_item(&mut self, ctx: &egui::Context, it: &Item, queue: &[&Item]) {
        if it.r#type == "song" {
            let mut songs: Vec<Track> = queue.iter().filter(|i| i.r#type == "song").map(|i| i.track()).collect();
            if songs.is_empty() {
                songs.push(it.track());
            }
            let start = songs.iter().position(|t| t.id == it.id).unwrap_or(0);
            return self.pl.play_tracks(ctx, &songs, start, false);
        }
        if let Some((kind, _)) = target(it) {
            self.data.detail(ctx, kind, &it.id);
            self.want_play = Some((kind, it.id.clone()));
        }
    }

    /// Round play button centred on `at` that eases in (fade + 6px rise) while the pointer is over `cell`; true when clicked.
    /// Cards open on click, this plays.
    pub fn play_badge(&self, ui: &egui::Ui, cell: Rect, at: egui::Pos2, size: f32, key: impl std::hash::Hash + std::fmt::Debug) -> bool {
        let over = ui.rect_contains_pointer(cell);
        let a = ui.ctx().calm_bool_ease(ui.scope_id().with(("play-badge-a", &key)), over, 0.16, crate::gfx::ease_out);
        if a <= 0. {
            return false;
        }
        let t = self.t;
        let at = at + vec2(0., (1. - a) * 6.);
        let b = Rect::from_center_size(at, vec2(size, size));
        let r = ui.interact(b, ui.scope_id().with(("play-badge", key)), egui::Sense::click());
        let fill = if r.is_pointer_button_down_on() { t.ax.gamma_multiply(0.8) } else if r.hovered() { t.ax } else { t.ax.gamma_multiply(0.94) };
        // Hovering the button itself grows it a touch; the triangle is nudged right so it looks optically centred.
        let grow = if r.hovered() { 1.06 } else { 1. };
        let size = size * grow;
        shadow(ui.painter(), Rect::from_center_size(at, vec2(size, size)), size / 2., 4, 12, (70. * a) as u8);
        ui.painter().circle_filled(at, size / 2., fill.gamma_multiply(a));
        // The glyph fills under half of its icon box and sits right of it, so the box is big and nudged left.
        icon(ui, icon!("play"), at - vec2(size * 0.02, 0.), size * 0.8, t.on_ax.gamma_multiply(a));
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        over && r.clicked()
    }

    /// Right-click on a card opens its context menu; a song card's menu acts on the song.
    fn card_menu(&mut self, r: &egui::Response, cell: Rect, it: &Item) {
        if r.secondary_clicked() {
            let target = if it.r#type == "song" { Target::Track(it.track()) } else { Target::Item(it.clone()) };
            self.ctx_menu = Some((r.interact_pointer_pos().unwrap_or(cell.center()), target));
        }
    }

    /// Hovering a card starts loading the page it opens.
    fn prefetch(&self, ctx: &egui::Context, it: &Item) {
        if let Some((kind, _)) = target(it) {
            self.data.detail(ctx, kind, &it.id);
        }
    }

    /// `badge`: the list this card is in mixes sources, so the card names its own.
    fn item_card(&mut self, ui: &mut egui::Ui, cell: Rect, it: &Item, queue: &[&Item], key: (usize, usize), badge: bool) {
        let t = self.t;
        let r = tonal(ui, &t, ("card", key), cell, 10., None);
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it);
        }
        self.card_menu(&r, cell, it);
        let art = Rect::from_min_size(cell.min + vec2(8., 8.), vec2(cell.width() - 16., cell.width() - 16.));
        self.thumb(ui, it.thumbnail.as_deref(), art, if it.r#type == "artist" { art.width() / 2. } else { 6. });
        if self.play_badge(ui, cell, pos2(art.max.x - 10. - 24., art.max.y - 10. - 24.), 48., ("card-play", key)) {
            self.play_item(ui.ctx(), it, queue);
        }
        let dx = self.source_mark_if(ui, (badge || it.unavailable).then_some(it.src()), it.unavailable, art.min.x, art.max.y + 33., t.tm);
        let p = ui.painter();
        title_trunc(ui, p, pos2(art.min.x, art.max.y + 16.), art.width(), &it.title, font_w(13., Weight::Medium), t.tx, (it.explicit && it.r#type == "song").then_some((14., t.tm)));
        trunc(p, pos2(art.min.x + dx, art.max.y + 33.), art.width() - dx, it.sub(), font(12.), t.tm);
    }

    /// One titled, horizontally scrolling row of cards; arrows and edge fades show when more cards are off-screen.
    /// `more` adds a "Show all" button that opens the shelf as a wrapped page.
    pub(crate) fn carousel<'a>(&mut self, ui: &mut egui::Ui, dims: (f32, f32, f32), title: &str, items: impl Iterator<Item = &'a Item>, si: usize, more: bool) {
        self.carousel_from(ui, dims, title, None, items, si, more)
    }

    /// `carousel` whose title is preceded by the logo of the shelf's `src` (when 2+ sources are enabled).
    #[allow(clippy::too_many_arguments)]
    fn carousel_from<'a>(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32), title: &str, src: Option<&str>, items: impl Iterator<Item = &'a Item>, si: usize, more: bool) {
        let t = self.t;
        let all: Vec<&Item> = items.collect();
        // A mixed shelf badges its cards; otherwise the shelf names its source once, on its title (detail pages say it in their hero).
        let badge = src.is_none() && crate::sources::mixed(all.iter().map(|i| i.src()));
        let one = src.or_else(|| all.first().map(|i| i.src())).filter(|_| !badge && !matches!(self.route, Route::Album | Route::Artist));
        // fixed card width: a narrower window clips cards instead of resizing them
        let cw = 160.;
        let cols = (((w + self.gap()) / (cw + self.gap())) as usize).max(1);
        let step = cw + self.gap();
        let max_off = (all.len() as f32 * step - self.gap() - w).max(0.);
        let id = ui.scope_id().with(("carousel", title));
        let off: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.);
        // Arrow slide (from, to, start time). Clicks while it runs extend its target, so repeated clicks keep moving forward.
        let aid = id.with("slide");
        let now = ui.ctx().input(|i| i.time);
        let mut slide = ui.ctx().data(|d| d.get_temp::<(f32, f32, f64)>(aid)).unwrap_or((0., 0., f64::NEG_INFINITY));
        let k = ((now - slide.2) / CAR_SLIDE) as f32;
        // A wheel or trackpad swipe over the row takes over from the slide.
        if k < 1. && ui.input(|i| i.smooth_scroll_delta.x != 0.) {
            slide.2 = f64::NEG_INFINITY;
        }
        let k = ((now - slide.2) / CAR_SLIDE) as f32;
        let sliding = k < 1.;
        let shown = if sliding { egui::lerp(slide.0..=slide.1, ease_out_soft(k)) } else { off };
        let base = if sliding { slide.1 } else { off };

        let (_, hd) = ui.allocate_space(vec2(full, 44.));
        let (p, cy) = (ui.painter().clone(), hd.center().y + 6.);
        let mut rx = l + w - 8.;
        let mut go = None;
        if max_off > 0. {
            for (dir, ic, can) in [(1., icon!("fwd"), base < max_off - 1.), (-1., icon!("back"), base > 1.)] {
                if icon_button(ui, &t, ("car-arrow", title, dir as i32), pos2(rx - 16., cy), 32., ic, 18., if can { t.tx } else { t.tf }).clicked() && can {
                    go = Some(dir);
                }
                rx -= 36.;
            }
        }
        if more && all.len() > cols {
            let f = font_w(13., Weight::Semi);
            let bw = text_w(&p, "Show all", f.clone()) + 16.;
            let b = Rect::from_center_size(pos2(rx - bw / 2., cy), vec2(bw, 28.));
            let r = press(ui, ("show-all", title), b);
            p.text(b.center(), Align2::CENTER_CENTER, "Show all", f, if r.hovered() { t.tx } else { t.tm });
            if r.clicked() {
                let sub = match src.or_else(|| all.first().map(|i| i.src())).filter(|_| !badge && self.route == Route::Home) {
                    Some(id) => format!("From {}", self.sources.name(id)),
                    None => String::new(),
                };
                self.shelf = Some((title.to_string(), sub, all.iter().map(|i| (*i).clone()).collect()));
                self.open(Route::Shelf, Some(("shelf", title.to_string())));
            }
            rx -= bw + 4.;
        }
        let dx = self.source_mark(ui, one, l + 8., cy, t.tm);
        trunc(&p, pos2(l + 8. + dx, cy), (rx - l - 16. - dx).max(0.), title, font_w(17., Weight::Bold), t.tx);

        let (_, row) = ui.allocate_space(vec2(full, cw + 8. + 40.));
        let view = Rect::from_min_size(pos2(l, row.min.y), vec2(w, row.height()));
        let mut rui = App::region(ui, view);
        if let Some(dir) = go {
            let to = (((base / step).round() + dir * cols.saturating_sub(1).max(1) as f32) * step).clamp(0., max_off);
            slide = (shown, to, now);
            ui.ctx().data_mut(|d| d.insert_temp(aid, slide));
        }
        let sliding = sliding || go.is_some();
        let mut area = ScrollArea::horizontal().id_salt(id).auto_shrink(false).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if sliding {
            area = area.horizontal_scroll_offset(shown);
            ui.ctx().request_repaint();
        }
        let out = area.show(&mut rui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let (_, r) = ui.allocate_space(vec2((all.len() as f32 * step - self.gap()).max(w), view.height()));
            let vis = ui.clip_rect();
            for (c, &it) in all.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(r.min.x + c as f32 * step, r.min.y), vec2(cw, cw + 40.));
                if cell.intersects(vis) {
                    self.item_card(ui, cell, it, &all, (si, c), badge);
                }
            }
        });
        let off = out.state.clamped_offset().x;
        ui.ctx().data_mut(|d| d.insert_temp(id, off));
        let (left, right) = (ui.ctx().calm_bool(id.with("fade-l"), off > 1., 0.15), ui.ctx().calm_bool(id.with("fade-r"), off < max_off - 1., 0.15));
        if left > 0. {
            // The fades run a pixel past the row: at a fractional edge the clip keeps a last column of the card, which would show as a line.
            fade_h(&p, Rect::from_min_size(view.min - vec2(1., 0.), vec2(49., view.height())), t.s1.gamma_multiply(left), false);
        }
        if right > 0. {
            fade_h(&p, Rect::from_min_max(pos2(view.max.x - 48., view.min.y), view.max + vec2(1., 0.)), t.s1.gamma_multiply(right), true);
        }
    }

    /// Every item of a shelf in a wrapped grid, virtualised by row.
    pub fn wall(&mut self, ui: &mut egui::Ui, card: Rect, top: f32, items: &[&Item], salt: &str) {
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let (cols, cw) = grid(w, 160., self.gap());
        let rh = cw + 48.;
        let rows = items.len().div_ceil(cols);
        let badge = crate::sources::mixed(items.iter().map(|i| i.src()));
        let area = Rect::from_min_max(pos2(card.min.x, top), card.max);
        let mut lui = App::region(ui, area);
        let out = ScrollArea::vertical().id_salt(("wall", salt)).auto_shrink(false).show_viewport(&mut lui, |ui, vp| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let (first, last) = (((vp.min.y / rh) as usize).min(rows), (((vp.max.y / rh).ceil() as usize) + 1).min(rows));
            ui.add_space(first as f32 * rh);
            for r in first..last {
                let (_, row) = ui.allocate_space(vec2(area.width(), rh));
                for c in 0..cols {
                    let Some(&it) = items.get(r * cols + c) else { break };
                    let cell = Rect::from_min_size(pos2(l + c as f32 * (cw + self.gap()), row.min.y), vec2(cw, cw + 40.));
                    self.item_card(ui, cell, it, items, (0, r * cols + c), badge);
                }
            }
            ui.add_space((rows - last) as f32 * rh + 70.);
        });
        scroll_fades(&lui, &out, self.t.s1);
    }

    /// A home shelf listed in full, honouring the home page's type chip.
    pub fn shelf_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let Some((_, title)) = self.sel.clone() else { return };
        if let Some((_, _, items)) = self.shelf.clone().filter(|s| s.0 == title) {
            let items: Vec<&Item> = items.iter().collect();
            return self.wall(ui, card, top, &items, "shelf");
        }
        let d = self.data.clone();
        let g = d.home.lock();
        let State::Ready(secs) = &*g else { return self.wall_skeleton(ui, card, top) };
        let kind = CHIPS[self.home_chip].1;
        let items: Vec<&Item> = secs.iter().filter(|s| s.title == title).flat_map(|s| &s.items).filter(|i| kind.is_empty() || i.r#type == kind).collect();
        self.wall(ui, card, top, &items, "shelf");
    }

    /// Titled carousels, one per section, under the collapsible hero; returns the scroll offset.
    pub fn sections(&mut self, ui: &mut egui::Ui, card: Rect, secs: &[Section]) -> f32 {
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let area = Rect::from_min_max(pos2(card.min.x, card.min.y + HERO_MIN), card.max);
        let mut lui = App::region(ui, area);
        let sa = ScrollArea::vertical().id_salt(("sections", &self.sel)).auto_shrink(false).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        let sa = match crate::gfx::page_bar_set(&lui) {
            Some(o) => sa.vertical_scroll_offset(o),
            None => sa,
        };
        let out = sa.show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            ui.add_space(HERO_H - HERO_MIN);
            for (si, s) in secs.iter().enumerate() {
                if self.route == Route::Artist && s.title == "Top songs" && s.items.iter().all(|i| i.r#type == "song") {
                    self.top_songs(ui, (l, w, area.width()), card, &s.items);
                    self.library_card(ui, (l, w, area.width()));
                } else {
                    self.carousel_from(ui, (l, w, area.width()), &s.title, s.source.as_deref(), s.items.iter(), si, true);
                }
            }
            ui.add_space(70.);
        });
        scroll_fades(&lui, &out, self.t.s1);
        let off = out.state.clamped_offset().y;
        crate::gfx::page_bar(&lui, out.id, &out, card.min.y + HERO_H - off.min(HERO_H - HERO_MIN), self.t.tf);
        off
    }

    /// An artist's Top songs as a vertical track list.
    fn top_songs(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32), card: Rect, items: &[Item]) {
        let tracks: Vec<Track> = items.iter().map(Item::track).collect();
        let (_, hd) = ui.allocate_space(vec2(full, 44.));
        trunc(ui.painter(), pos2(l + 8., hd.center().y + 6.), w - 16., "Top songs", font_w(17., Weight::Bold), self.t.tx);
        let (tl, tr) = (card.min.x + 16., card.max.x - 16.);
        let look = Look { cover_w: self.cover_px(), open: false, album: false };
        let (_, head) = ui.allocate_space(vec2(full, HEAD));
        self.table_head(ui, head.min.y + 14., tl, tr, look, None);
        let acts = self.track_rows(ui, "top-songs", &tracks, 0..tracks.len(), (tl, tr), self.cover_row_h(), look);
        self.apply_acts(ui, &tracks, acts, look);
        ui.add_space(16.);
    }

    /// The artist's songs among the user's Liked songs (empty until that page loads).
    fn liked_by(&mut self, ctx: &egui::Context, artist_id: &str) -> Vec<Track> {
        let name = self.data.detail(ctx, "artists", artist_id).ready().map(|d| d.item.title.clone()).unwrap_or_default();
        match self.data.detail(ctx, "playlists", "LM").ready() {
            Some(d) => d.tracks.iter().filter(|t| t.artist_id.as_deref() == Some(artist_id) || t.artists.contains(&name)).cloned().collect(),
            None => vec![],
        }
    }

    /// "From your library" card under Top songs; opens the liked songs by this artist. Hidden when there are none.
    fn library_card(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32)) {
        let Some(("artists", id)) = self.sel.clone() else { return };
        let liked = self.liked_by(ui.ctx(), &id);
        let Some(first) = liked.first() else { return };
        let t = self.t;
        let (_, r) = ui.allocate_space(vec2(full, 88. + 16.));
        let card = Rect::from_min_size(pos2(l + 8., r.min.y + 8.), vec2(w - 16., 80.));
        let hit = tonal(ui, &t, "library-card", card, 14., Some(t.s2));
        let art = Rect::from_min_size(card.min + vec2(12., 12.), vec2(56., 56.));
        self.thumb(ui, first.thumbnail.as_deref(), art, 8.);
        let p = ui.painter();
        let tx = art.max.x + 16.;
        trunc(p, pos2(tx, card.center().y - 11.), card.max.x - tx - 48., "From your library", font_w(15., Weight::Bold), t.tx);
        trunc(p, pos2(tx, card.center().y + 11.), card.max.x - tx - 48., &summary(&liked), font(13.), t.tm);
        icon(ui, icon!("fwd"), pos2(card.max.x - 28., card.center().y), 18., t.tm);
        if hit.clicked() {
            self.open(Route::LibraryTracks, Some(("library", id)));
        }
    }

    /// The user's liked songs by one artist, as a cover track list.
    pub fn library_tracks_page(&mut self, ui: &mut egui::Ui, card: Rect, artist_id: &str) {
        let t = self.t;
        let name = self.data.detail(ui.ctx(), "artists", artist_id).ready().map(|d| d.item.title.clone()).unwrap_or_default();
        let p = ui.painter().clone();
        trunc(&p, pos2(card.min.x + 22., card.min.y + 38.), card.width() - 44., "From your library", font_w(26., Weight::Bold), t.tx);
        trunc(&p, pos2(card.min.x + 22., card.min.y + 66.), card.width() - 44., &name, font(13.), t.tm);
        let tracks = self.liked_by(ui.ctx(), artist_id);
        if tracks.is_empty() {
            return self.note(ui, card, card.min.y + 90., "No liked songs by this artist.");
        }
        let list = Rect::from_min_max(pos2(card.min.x, card.min.y + 90.), card.max);
        let mut lui = App::region(ui, list);
        let (tl, tr) = (card.min.x + 16., card.max.x - 16.);
        let look = Look { cover_w: self.cover_px(), open: false, album: false };
        let out = ScrollArea::vertical().id_salt(("library-tracks", artist_id)).auto_shrink(false).show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let (_, head) = ui.allocate_space(vec2(list.width(), HEAD));
            self.table_head(ui, head.min.y + 14., tl, tr, look, None);
            let acts = self.track_rows(ui, "library-tracks", &tracks, 0..tracks.len(), (tl, tr), self.cover_row_h(), look);
            self.apply_acts(ui, &tracks, acts, look);
            ui.add_space(70.);
        });
        scroll_fades(&lui, &out, t.s1);
    }

    /// Compact quick-pick tile: cover flush left, title beside it.
    fn quick_tile(&mut self, ui: &mut egui::Ui, cell: Rect, it: &Item, queue: &[&Item], key: usize) {
        let t = self.t;
        let r = tonal(ui, &t, ("quick", key), cell, 8., Some(t.s2));
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it);
        }
        self.card_menu(&r, cell, it);
        let art = Rect::from_min_size(cell.min, vec2(cell.height(), cell.height()));
        self.thumb(ui, it.thumbnail.as_deref(), art, 8.);
        trunc(ui.painter(), pos2(art.max.x + 12., cell.center().y), cell.width() - art.width() - 24. - 36., &it.title, font_w(13., Weight::Medium), t.tx);
        if self.play_badge(ui, cell, pos2(cell.max.x - 12. - 16., cell.center().y), 32., ("quick-play", key)) {
            self.play_item(ui.ctx(), it, queue);
        }
    }

    /// Large portrait card: cover-filled backdrop under a dark wash, small cover + title on top, "With A, B and more" at the bottom.
    /// `label` sits above the card and says why it is shown (the section title); `queue` is the shelf, its first item is the card.
    /// `one` is the source the whole shelf comes from, shown beside `label`; without it the shelf mixes sources and the card badges its own.
    fn feature_card(&mut self, ui: &mut egui::Ui, cell: Rect, label: &str, one: Option<&str>, queue: &[&Item], key: usize) {
        let t = self.t;
        let Some(&it) = queue.first() else { return };
        let names: Vec<&str> = queue.iter().skip(1).take(3).map(|i| i.title.as_str()).collect();
        let with = if names.is_empty() { String::new() } else { format!("With {} and more", names.join(", ")) };
        let r = ui.interact(cell, ui.scope_id().with(("feat", key)), egui::Sense::click());
        let p = ui.painter().clone();
        let ld = self.source_mark(ui, one, cell.min.x, cell.min.y - 12., t.tm);
        trunc(&p, pos2(cell.min.x + ld, cell.min.y - 12.), cell.width() - ld, label, font(12.5), t.tm);
        // Grey tints multiply the cover: 145 is the 110-alpha black wash, fading to 31 at the bottom; hover brightens it.
        let g = Color32::from_gray;
        let stops = if r.hovered() { [(0., g(160)), (0.5, g(160)), (1., g(40))] } else { [(0., g(145)), (0.5, g(145)), (1., g(31))] };
        let shaded = it.thumbnail.as_deref().is_some_and(|u| paint_cover_shaded(ui, u, cell, 14., &stops, t.s1));
        if !shaded {
            p.rect_filled(cell, rad(14.), t.s3);
        }
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let art = Rect::from_min_size(cell.min + vec2(16., 16.), vec2(64., 64.));
        shadow(&p, art, 8., 4, 14, 90);
        self.thumb(ui, it.thumbnail.as_deref(), art, 8.);
        let tx = art.max.x + 12.;
        let tw = (cell.max.x - 16. - tx).max(0.);
        trunc(&p, pos2(tx, art.min.y + 20.), tw, &it.title, font_w(19., Weight::Bold), t.tx);
        let kind = match it.r#type.as_str() { "album" => "Album", "playlist" => "Playlist", "artist" => "Artist", _ => "Song" };
        let dx = self.source_mark(ui, one.is_none().then_some(it.src()), tx, art.min.y + 46., t.tm);
        trunc(&p, pos2(tx + dx, art.min.y + 46.), tw - dx, &kind_line(kind, it.sub()), font(12.5), t.tm);
        if !with.is_empty() {
            let g = wrapped(&p, &with, font(12.5), t.tm, cell.width() - 32. - 48., 2);
            p.galley(pos2(cell.min.x + 16., cell.max.y - 16. - g.size().y), g, t.tm);
        }
        if self.play_badge(ui, cell, pos2(cell.max.x - 16. - 24., cell.max.y - 16. - 24.), 48., ("feat-play", key)) {
            self.play_item(ui.ctx(), it, queue);
        } else if r.clicked() {
            self.open_item(ui.ctx(), it);
        }
        self.card_menu(&r, cell, it);
    }

    /// Pill chips from `x0`; returns the selected index, which a click changes.
    fn chips(&self, ui: &mut egui::Ui, x0: f32, top: f32, labels: &[&str], current: usize) -> usize {
        let t = self.t;
        let (mut sel, mut x) = (current, x0);
        for (i, label) in labels.iter().enumerate() {
            let w = text_w(ui.painter(), label, font(13.)) + 28.;
            let b = Rect::from_min_size(pos2(x, top), vec2(w, 32.));
            let on = current == i;
            let r = press(ui, ("chip", i), b);
            let fill = if on { t.tx } else if r.is_pointer_button_down_on() { t.s2 } else if r.hovered() { t.s3 } else { t.s2 };
            ui.painter().rect_filled(b, rad(16.), fill);
            if r.clicked() {
                sel = i;
            }
            ui.painter().text(b.center(), Align2::CENTER_CENTER, label, if on { font_w(13., Weight::Medium) } else { font(13.) }, if on { t.s0 } else if r.hovered() { t.tx } else { t.tm });
            x += w + self.gap();
        }
        sel
    }

    /// Spacing between cards and tiles: roomier in Comfortable, tight in Compact.
    pub(crate) fn gap(&self) -> f32 {
        if self.opts.compact { 8. } else { 14. }
    }

    /// "Good morning" etc. from the local hour.
    fn greeting() -> &'static str {
        use chrono::Timelike;
        match chrono::Local::now().hour() {
            5..=11 => "Good morning",
            12..=16 => "Good afternoon",
            17..=21 => "Good evening",
            _ => "Good night",
        }
    }

    /// Height of `home_head`: greeting row, "From …" line, then the chips with room around them.
    fn home_head_h(&self) -> f32 {
        100. + 32. + if self.opts.compact { 12. } else { 20. }
    }

    /// Greeting row, the sources line and the chips, starting at `y`; returns the y below them. Scrolls with the feed.
    fn home_head(&mut self, ui: &mut egui::Ui, card: Rect, y: f32, user: Option<&backend::User>) -> f32 {
        let t = self.t;
        let x = card.min.x + 22.;
        let (w, f) = (card.width() - 44., font_w(26., Weight::Bold));
        match user {
            Some(u) => {
                let hi = format!("{},", Self::greeting());
                let hw = text_w(ui.painter(), &hi, f.clone());
                trunc(ui.painter(), pos2(x, y + 38.), w, &hi, f.clone(), t.tx);
                let av = Rect::from_center_size(pos2(x + hw + 12. + 16., y + 38.), vec2(32., 32.));
                self.thumb(ui, u.avatar_url.as_deref(), av, 16.);
                trunc(ui.painter(), pos2(av.max.x + 10., y + 38.), (w - hw - 12. - 32. - 10.).max(0.), &u.name, f, t.tx);
            }
            None => { trunc(ui.painter(), pos2(x, y + 38.), w, Self::greeting(), f, t.tx); }
        }
        let names = self.sources.names_for("home");
        let from = if names.is_empty() { String::new() } else { format!("From {}", names.join(", ")) };
        trunc(ui.painter(), pos2(x, y + 70.), (card.width() - 44.).max(0.), &from, font(13.), t.tm);
        let labels: Vec<&str> = CHIPS.iter().map(|c| c.0).collect();
        self.home_chip = self.chips(ui, x, y + 100., &labels, self.home_chip);
        y + self.home_head_h()
    }

    /// The source's logo just left of a line of subtext, its name on hover. Returns the width taken (0 unless more than one source is enabled).
    pub fn source_mark(&self, ui: &egui::Ui, src: Option<&str>, x: f32, cy: f32, col: Color32) -> f32 {
        self.source_mark_if(ui, src, false, x, cy, col)
    }

    /// `source_mark`, drawn even with a single source enabled when `force` is set (items from a source that is off).
    pub fn source_mark_if(&self, ui: &egui::Ui, src: Option<&str>, force: bool, x: f32, cy: f32, col: Color32) -> f32 {
        let Some(src) = src.filter(|_| force || self.sources.multi()) else { return 0. };
        source_mark_at(ui, &self.t, src, &self.sources.name(src), x, cy, col)
    }

    /// The blocking fetch behind Home.
    pub fn home_loader(&self) -> impl FnOnce() -> Result<Vec<Section>, String> + Send + 'static {
        let stamp = self.data.clone();
        let path = if self.opts.home_layout == 1 { "/mix/home?layout=separate" } else { "/mix/home?layout=mixed" };
        move || backend::get::<Vec<Section>>(path).map(|mut v| {
            crate::data::clean_sections(&mut v);
            crate::data::clean_home(&mut v);
            stamp.home_at.store(crate::data::now_secs(), std::sync::atomic::Ordering::Relaxed);
            v
        })
    }

    pub fn home_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let top = top;
        let d = self.data.clone();
        d.status.ensure(ui.ctx(), || backend::get("/auth/status"));
        // The greeting names the account of the topmost enabled source.
        let user = match self.sources.top() {
            Some(s) if !s.builtin => {
                // Name and picture come from a connection test, which the daemon forgets on restart and on sign-in.
                if s.secret_set && s.status == "unchecked" && !self.sources.checking(&s.id) {
                    self.sources.test(ui.ctx(), &s.id);
                }
                s.detail.as_deref().and_then(|d| d.strip_prefix("Signed in as ")).map(str::to_string).or(s.username.clone().filter(|_| s.secret_set))
                    .map(|name| backend::User { name, subtitle: None, avatar_url: s.avatar_url.clone() })
            }
            _ => match &*d.status.lock() {
                State::Ready(s) => s.youtube.user.clone().filter(|_| s.youtube.logged_in),
                _ => None,
            },
        };
        // Home refetches when a source changes (see `library`), and whenever it is older than ten minutes when the page is drawn.
        if d.home_age() > 600 {
            d.home.reset();
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(60));
        d.home.ensure(ui.ctx(), self.home_loader());
        let g = d.home.lock();
        match &*g {
            State::Ready(s) => self.home_feed(ui, card, top, s, user.as_ref()),
            State::Failed(e) => {
                let y = self.home_head(ui, card, top, user.as_ref());
                self.note(ui, card, y, e)
            }
            _ => {
                let y = self.home_head(ui, card, top, user.as_ref());
                self.home_skeleton(ui, card, y)
            }
        }
    }

    /// Quick picks from the first shelf, three carousels, then one large card per remaining shelf (shelf title as its label).
    fn home_feed(&mut self, ui: &mut egui::Ui, card: Rect, top: f32, secs: &[Section], user: Option<&backend::User>) {
        let kind = CHIPS[self.home_chip].1;
        let ok = move |i: &&Item| kind.is_empty() || i.r#type == kind;
        let mut shelves = secs.iter().filter(|s| s.items.iter().any(|i| ok(&i)));
        let quick = if kind.is_empty() { shelves.next() } else { None };
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let area = Rect::from_min_max(pos2(card.min.x, top), card.max);
        let mut lui = App::region(ui, area);
        let out = ScrollArea::vertical().id_salt(("home", self.home_chip)).auto_shrink(false).show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let (_, hr) = ui.allocate_space(vec2(area.width(), self.home_head_h()));
            self.home_head(ui, card, hr.min.y, user);
            if let Some(s) = quick {
                let cols = (((w + self.gap()) / 240.) as usize).clamp(2, 4);
                let cw = (w - 16. - self.gap() * (cols - 1) as f32) / cols as f32;
                let (_, r) = ui.allocate_space(vec2(area.width(), 56. * 2. + self.gap() + 16.));
                let all: Vec<&Item> = s.items.iter().collect();
                for (n, it) in s.items.iter().take(cols * 2).enumerate() {
                    let cell = Rect::from_min_size(pos2(l + 8. + (n % cols) as f32 * (cw + self.gap()), r.min.y + (n / cols) as f32 * (56. + self.gap())), vec2(cw, 56.));
                    self.quick_tile(ui, cell, it, &all, n);
                }
            }
            let rest: Vec<&Section> = shelves.collect();
            for (si, s) in rest.iter().take(3).enumerate() {
                self.carousel_from(ui, (l, w, area.width()), &s.title, s.source.as_deref(), s.items.iter().filter(ok), si, true);
            }
            let feats = &rest[rest.len().min(3)..];
            let fcols = (((w + self.gap()) / 300.) as usize).clamp(1, 4);
            let cw = (w - 16. - self.gap() * (fcols - 1) as f32) / fcols as f32;
            let ch = (cw * 1.2).clamp(260., 420.);
            for (row, chunk) in feats.chunks(fcols).enumerate() {
                let (_, r) = ui.allocate_space(vec2(area.width(), 28. + ch + 14.));
                for (c, s) in chunk.iter().enumerate() {
                    let shelf: Vec<&Item> = s.items.iter().filter(ok).collect();
                    let cell = Rect::from_min_size(pos2(l + 8. + c as f32 * (cw + self.gap()), r.min.y + 28.), vec2(cw, ch));
                    let one = s.source.as_deref().or_else(|| shared(shelf.iter().map(|i| i.src())));
                    self.feature_card(ui, cell, &s.title, one, &shelf, row * 4 + c);
                }
            }
            ui.add_space(70.);
        });
        scroll_fades(&lui, &out, self.t.s1);
    }

    /// Album, playlist or artist selected from a card.
    pub fn live_detail(&mut self, ui: &mut egui::Ui, card: Rect, kind: &'static str, id: &str) {
        if kind == "song" {
            return self.single_page(ui, card);
        }
        let d = self.data.detail(ui.ctx(), kind, id);
        // Cloned out so no lock is held while drawing: rows read the Liked songs page, which may be this very one.
        let state = match &*d.lock() {
            State::Ready(p) => Ok(p.clone()),
            State::Failed(e) => Err(Some(e.clone())),
            _ => Err(None),
        };
        match state {
            Ok(p) => {
                let (head, round) = match kind { "artists" => ("Artist", true), "playlists" | "spotify" => ("Playlist", false), _ => ("Album", false) };
                let album = kind == "albums" && !p.tracks.is_empty();
                // Albums read "Artist · 2026 · 12 songs, 41 min"; the artist is a link, so it leaves the subtitle.
                let year = p.item.subtitle.as_deref().and_then(|s| s.rsplit(" · ").next()).filter(|y| is_year(y)).map(String::from);
                let note = year.as_deref().map(|y| format!("Released {y}")).unwrap_or_default();
                let sub = if album {
                    [year, Some(summary(&p.tracks))].into_iter().flatten().collect::<Vec<_>>().join(" · ")
                } else {
                    [p.item.subtitle.clone().filter(|_| p.item.author.is_none() && (kind != "artists" || self.opts.show_subs)), (!p.tracks.is_empty()).then(|| summary(&p.tracks))].into_iter().flatten().collect::<Vec<_>>().join(" · ")
                };
                let first = p.tracks.first();
                // The album's own artist (its subtitle before the year), not the first track's, which may be a featured guest.
                let album_artist = p.item.subtitle.as_deref().map(|s| match s.rsplit_once(" · ") {
                    Some((a, y)) if is_year(y) => a,
                    _ if is_year(s) => "",
                    _ => s,
                }).filter(|a| !a.is_empty());
                let by = match album_artist {
                    Some(a) if album => Some((a, p.item.artist_id.as_deref().or(first.and_then(|t| t.artist_id.as_deref())))),
                    _ if p.item.author.is_some() => p.item.author.as_deref().map(|a| (a, p.item.artist_id.as_deref())),
                    _ => first.filter(|_| album).map(|t| t.artists.first().map_or((crate::data::UNKNOWN_ARTIST, None), |n| (n.as_str(), t.artist_id.as_deref()))),
                };
                // Only YouTube albums and playlists can be saved; the saved state is read off the sidebar's library list.
                let save = match kind {
                    // Saving writes to YouTube Music, which has no Spotify ids.
                    "albums" | "playlists" if !id.starts_with("spotify:") => {
                        let lib = if kind == "albums" { 2 } else { 1 };
                        Some((kind, id, self.lib_items.iter().any(|i| i.kind == lib && i.id == id)))
                    }
                    _ => None,
                };
                // The list scrolls under the hero, which shrinks as it does, so the list is drawn first to know the offset.
                // Rows of a playlist the user can edit get "Remove from this playlist" in their menu.
                self.pl_ctx = (kind == "playlists" && p.item.editable).then(|| id.to_string());
                let off = if p.tracks.is_empty() { self.sections(ui, card, &p.sections) } else { self.tracks_table(ui, card, &p.tracks, album.then(|| (by.and_then(|b| b.1).unwrap_or(""), p.sections.as_slice(), note.as_str()))) };
                // While a Spotify import resolves, only its matched rows can play (they are YouTube songs by then).
                let playable: Cow<'_, [Track]> = if kind == "spotify" {
                    Cow::Owned(p.tracks.iter().filter(|t| !t.resolving && !t.unavailable).cloned().collect())
                } else if id.starts_with("spotify:") {
                    // The whole list queues; songs are matched as they come up.
                    Cow::Owned(p.tracks.iter().filter(|t| !t.unavailable).cloned().collect())
                } else {
                    Cow::Borrowed(&p.tracks)
                };
                self.hero(ui, card, round, head, &p.item.title, by, &sub, p.item.thumbnail.as_deref(), &playable, save, off);
            }
            Err(Some(e)) => self.note(ui, card, card.min.y + 40., &e),
            Err(None) => {
                let it = self.peek.clone().filter(|i| i.id == id);
                self.detail_skeleton(ui, card, kind, it.as_ref().map(|i| i.title.as_str()).unwrap_or(""), it.as_ref().and_then(|i| i.thumbnail.as_deref()).filter(|_| !(kind == "artists" && id.starts_with("UC"))), it.as_ref().and_then(|i| i.subtitle.as_deref()).unwrap_or(""));
            }
        }
    }

    /// The whole search page is one scroll area: title, type chips, then a section per kind of result.
    /// All shows the top result beside the first songs and a short run of each kind; a chip shows one kind in full.
    pub fn search_page(&mut self, ui: &mut egui::Ui, card: Rect) {
        let t = self.t;
        let (d, q) = (self.data.clone(), self.submitted.clone());
        let (l, full) = (card.min.x + 22., card.width());
        let (songs_r, albums_r, artists_r, videos_r) = (d.search(ui.ctx(), &q), d.search_items(ui.ctx(), &q, "album"), d.search_items(ui.ctx(), &q, "artist"), d.search_items(ui.ctx(), &q, "video"));
        // `None` while loading or failed, with the message to show instead.
        fn ready<T: Clone + Send + Sync + 'static>(r: &crate::data::Remote<Vec<T>>, what: &str) -> Result<Vec<T>, String> {
            match &*r.lock() {
                State::Ready(v) if v.is_empty() => Err(format!("No {what} found.")),
                State::Ready(v) => Ok(v.to_vec()),
                State::Failed(e) => Err(e.clone()),
                _ => Err("Searching…".into()),
            }
        }
        let songs = ready(&songs_r, "songs");
        let (albums, artists, videos) = (ready(&albums_r, "albums"), ready(&artists_r, "artists"), ready(&videos_r, "videos"));

        let chip_id = ui.scope_id().with(("search-chip", &q));
        let cur = ui.ctx().data(|m| m.get_temp::<usize>(chip_id)).unwrap_or(0);
        let mut area = App::region(ui, card);
        let out = ScrollArea::vertical().id_salt(("search", &q)).auto_shrink(false).show(&mut area, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let (_, hr) = ui.allocate_space(vec2(full, 90.));
            let p = ui.painter().clone();
            trunc(&p, pos2(l, hr.min.y + 38.), full - 44., "Search", font_w(26., Weight::Bold), t.tx);
            let names = self.sources.names_for("search");
            let sub = match (q.is_empty(), names.is_empty()) {
                (true, _) => "Press Ctrl K to search".to_string(),
                (false, true) => String::new(),
                (false, false) => format!("Results from {}", names.join(", ")),
            };
            trunc(&p, pos2(l, hr.min.y + 66.), full - 44., &sub, font(13.), t.tm);
            if q.is_empty() {
                return;
            }

            let (_, cr) = ui.allocate_space(vec2(full, 44.));
            let chip = self.chips(ui, l, cr.min.y, &SEARCH_CHIPS, cur);
            if chip != cur {
                ui.ctx().data_mut(|m| m.insert_temp(chip_id, chip));
            }
            let all = chip == 0;
            let heading = |ui: &mut egui::Ui, x: f32, text: &str, hd: Rect| trunc(ui.painter(), pos2(x, hd.center().y + 6.), full - 44., text, font_w(17., Weight::Bold), t.tx);
            let msg = |ui: &mut egui::Ui, m: &str| {
                let (_, r) = ui.allocate_space(vec2(full, 40.));
                trunc(ui.painter(), pos2(l, r.center().y), full - 44., m, font(13.), t.tf);
            };

            // Songs, with the top result beside them when the card is wide enough and stacked above otherwise.
            if chip <= 1 {
                let (top_album, top_artist) = if all { (albums.as_ref().ok().and_then(|v| v.first()), artists.as_ref().ok().and_then(|v| v.first())) } else { (None, None) };
                let col_h = top_album.map_or(0., |_| TOP_H) + top_album.and(top_artist).map_or(0., |_| 8.) + top_artist.map_or(0., |_| STRIP_H);
                let side = col_h > 0. && full >= 640.;
                if col_h > 0. && !side {
                    let (_, hd) = ui.allocate_space(vec2(full, 44.));
                    heading(ui, l, "Top result", hd);
                    let (_, r) = ui.allocate_space(vec2(full, col_h + 16.));
                    self.top_column(ui, r.min + vec2(22., 0.), full - 44., top_album, top_artist);
                }
                let (_, hd) = ui.allocate_space(vec2(full, 44.));
                let hx = if side { l + TOP_W + 20. } else { l };
                let dx = self.source_mark(ui, songs.as_ref().ok().and_then(|v| shared(v.iter().map(|t| t.source.as_str()))), hx, hd.center().y + 6., t.tm);
                heading(ui, hx + dx, "Songs", hd);
                if side {
                    heading(ui, l, "Top result", hd);
                }
                match &songs {
                    Ok(v) => {
                        let n = if all { v.len().min(5) } else { v.len() };
                        let block = HEAD + n as f32 * self.cover_row_h();
                        let (_, br) = ui.allocate_space(vec2(full, if side { block.max(col_h) } else { block }));
                        let (tl, tr) = if side { (l + TOP_W + 20. - 6., card.max.x - 16.) } else { (card.min.x + 16., card.max.x - 16.) };
                        if side {
                            self.top_column(ui, pos2(l, br.min.y), TOP_W, top_album, top_artist);
                        }
                        let mut tui = App::region(ui, Rect::from_min_max(pos2(tl - 6., br.min.y), pos2(tr + 6., br.max.y)));
                        tui.spacing_mut().item_spacing = vec2(0., 0.);
                        let look = Look { cover_w: self.cover_px(), open: true, album: false };
                        let (_, head) = tui.allocate_space(vec2(tr - tl, HEAD));
                        self.table_head(&tui, head.min.y + 14., tl, tr, look, None);
                        let acts = self.track_rows(&mut tui, "songs", v, 0..n, (tl, tr), self.cover_row_h(), look);
                        self.apply_acts(ui, v, acts, look);
                    }
                    Err(m) => msg(ui, m),
                }
                ui.add_space(16.);
            }

            // Videos: rows with a 16:9 thumbnail; clicking one plays it.
            if all || chip == 2 {
                if let Ok(v) = &videos {
                    let tracks: Vec<Track> = v.iter().map(Item::track).collect();
                    let n = if all { tracks.len().min(4) } else { tracks.len() };
                    let (_, hd) = ui.allocate_space(vec2(full, 44.));
                    let dx = self.source_mark(ui, shared(tracks.iter().map(|t| t.source.as_str())), l, hd.center().y + 6., t.tm);
                    heading(ui, l + dx, "Videos", hd);
                    let (tl, tr) = (card.min.x + 16., card.max.x - 16.);
                    let look = Look { cover_w: self.cover_px() * 16. / 9., open: false, album: false };
                    let (_, head) = ui.allocate_space(vec2(full, HEAD));
                    self.table_head(ui, head.min.y + 14., tl, tr, look, None);
                    let acts = self.track_rows(ui, "videos", &tracks, 0..n, (tl, tr), self.cover_row_h(), look);
                    self.apply_acts(ui, &tracks, acts, look);
                    ui.add_space(16.);
                } else if chip == 2 {
                    msg(ui, videos.as_ref().err().map_or("", String::as_str));
                }
            }

            // Albums and artists: one row of cards under All, the full wrapped grid under their own chip.
            for (title, kind, v, si) in [("Albums", 3, &albums, 0), ("Artists", 4, &artists, 1)] {
                if !(all || chip == kind) {
                    continue;
                }
                match v {
                    Ok(v) if all => self.carousel(ui, (card.min.x + 14., card.width() - 28., full), title, v.iter(), si, false),
                    Ok(v) => {
                        let items: Vec<&Item> = v.iter().collect();
                        let (_, hd) = ui.allocate_space(vec2(full, 44.));
                        let dx = self.source_mark(ui, shared(items.iter().map(|i| i.src())), l, hd.center().y + 6., t.tm);
                        heading(ui, l + dx, title, hd);
                        self.item_grid(ui, card, &items, si);
                    }
                    Err(m) if chip == kind => msg(ui, m),
                    Err(_) => {}
                }
            }
            ui.add_space(70.);
        });
        scroll_fades(&area, &out, self.t.s1);
    }

    /// Every item in a wrapped grid inside the current scroll area.
    fn item_grid(&mut self, ui: &mut egui::Ui, card: Rect, items: &[&Item], si: usize) {
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let (cols, cw) = grid(w, 160., self.gap());
        let badge = crate::sources::mixed(items.iter().map(|i| i.src()));
        for r in 0..items.len().div_ceil(cols) {
            let (_, row) = ui.allocate_space(vec2(card.width(), cw + 48.));
            for c in 0..cols {
                let Some(&it) = items.get(r * cols + c) else { break };
                let cell = Rect::from_min_size(pos2(l + c as f32 * (cw + self.gap()), row.min.y), vec2(cw, cw + 40.));
                self.item_card(ui, cell, it, items, (si, r * cols + c), badge);
            }
        }
    }

    /// The top album card with the top artist's strip under it, from `at` down, `w` wide.
    fn top_column(&mut self, ui: &mut egui::Ui, at: egui::Pos2, w: f32, album: Option<&Item>, artist: Option<&Item>) {
        let mut y = at.y;
        if let Some(it) = album {
            self.top_card(ui, Rect::from_min_size(pos2(at.x, y), vec2(w, TOP_H)), it);
            y += TOP_H + 8.;
        }
        if let Some(it) = artist {
            self.artist_strip(ui, Rect::from_min_size(pos2(at.x, y), vec2(w, STRIP_H)), it);
        }
    }

    /// Top result: cover above kind and title; the card opens it.
    fn top_card(&mut self, ui: &mut egui::Ui, rect: Rect, it: &Item) {
        let t = self.t;
        let r = tonal(ui, &t, "top-result", rect, 14., Some(t.s2));
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it);
        }
        self.card_menu(&r, rect, it);
        let art = Rect::from_min_size(rect.min + vec2(18., 18.), vec2(96., 96.));
        let p = ui.painter().clone();
        shadow(&p, art, 6., 6, 18, 66);
        self.thumb(ui, it.thumbnail.as_deref(), art, 6.);
        let tw = rect.width() - 36. - 56.;
        trunc(&p, pos2(rect.min.x + 18., art.max.y + 26.), tw, &it.title, font_w(24., Weight::Bold), t.tx);
        let kind = match it.r#type.as_str() { "album" => "Album", "playlist" => "Playlist", _ => "Song" };
        let dx = self.source_mark(ui, Some(it.src()), rect.min.x + 18., art.max.y + 50., t.tm);
        trunc(&p, pos2(rect.min.x + 18. + dx, art.max.y + 50.), tw - dx, &kind_line(kind, it.sub()), font(13.), t.tm);
        if self.play_badge(ui, rect, pos2(rect.max.x - 18. - 22., rect.max.y - 18. - 22.), 44., "top-play") {
            self.play_item(ui.ctx(), it, &[]);
        }
    }

    /// Horizontal artist row: round photo, name and kind, play button at the end; the strip opens the artist.
    fn artist_strip(&mut self, ui: &mut egui::Ui, rect: Rect, it: &Item) {
        let t = self.t;
        let r = tonal(ui, &t, "top-artist", rect, 14., Some(t.s2));
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it);
        }
        self.card_menu(&r, rect, it);
        let photo = Rect::from_center_size(pos2(rect.min.x + 12. + 24., rect.center().y), vec2(48., 48.));
        self.thumb(ui, it.thumbnail.as_deref(), photo, 24.);
        let (p, tx) = (ui.painter().clone(), photo.max.x + 12.);
        let tw = rect.max.x - 12. - 40. - 8. - tx;
        trunc(&p, pos2(tx, rect.center().y - 9.), tw, &it.title, font_w(15., Weight::Semi), t.tx);
        let dx = self.source_mark(ui, Some(it.src()), tx, rect.center().y + 10., t.tm);
        trunc(&p, pos2(tx + dx, rect.center().y + 10.), tw - dx, "Artist", font(12.), t.tm);
        if self.play_badge(ui, rect, pos2(rect.max.x - 12. - 20., rect.center().y), 40., "top-artist-play") {
            self.play_item(ui.ctx(), it, &[]);
        }
    }

    pub fn import_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let d = self.data.clone();
        d.load_spotify(ui.ctx());
        let state = {
            let g = d.spotify.lock();
            match &*g {
                State::Ready(v) => Ok(v.clone()),
                State::Failed(e) => Err(Some(e.clone())),
                _ => Err(None),
            }
        };
        match state {
            Ok(v) if v.is_empty() => self.note(ui, card, top, "No Spotify playlists."),
            Ok(v) => self.wall(ui, card, top, &v.iter().collect::<Vec<_>>(), "import"),
            Err(Some(e)) => self.note(ui, card, top, &format!("{e} — sign in to Spotify under Settings.")),
            Err(None) => self.wall_skeleton(ui, card, top),
        }
    }

    /// Sign-in rows for each service, drawn inside the Settings page's Accounts panel (`card`).
    /// Returns the height it used.
    pub fn accounts_panel(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) -> f32 {
        let (t, d) = (self.t, self.data.clone());
        if d.account_changed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.sources.refetch();
        }
        d.status.ensure(ui.ctx(), || backend::get("/auth/status"));
        let g = d.status.lock();
        let st = match &*g {
            State::Ready(s) => s.clone(),
            State::Failed(e) => return self.note_h(ui, card, top, e),
            _ => return self.note_h(ui, card, top, "Connecting to the daemon…"),
        };
        drop(g);
        let (busy, err) = d.busy.lock().unwrap().clone();
        for (i, (svc, label, s)) in [("youtube", "YouTube Music", &st.youtube), ("spotify", "Spotify", &st.spotify)].into_iter().enumerate() {
            let row = Rect::from_min_size(pos2(card.min.x + 16., top + 16. + i as f32 * 84.), vec2(card.width() - 32., 72.));
            let p = ui.painter().clone();
            p.rect_filled(row, rad(14.), t.s1);
            let cy = row.center().y;
            let who = s.user.as_ref().map(|u| u.name.as_str());
            self.avatar(ui, s.user.as_ref().and_then(|u| u.avatar_url.as_deref()).filter(|_| s.logged_in), pos2(row.min.x + 36., cy), 20., who);
            let import = svc == "spotify" && s.logged_in;
            let tx = row.min.x + 68.;
            let tw = row.width() - if import { 380. } else { 220. };
            trunc(&p, pos2(tx, cy - 9.), tw, label, font_w(14., Weight::Semi), t.tx);
            let line = match (who, &s.error) {
                (Some(n), _) if s.logged_in => format!("Signed in as {n}"),
                (_, Some(e)) => e.clone(),
                _ => "Not signed in".into(),
            };
            trunc(&p, pos2(tx, cy + 10.), tw, &line, font(12.), t.tm);
            let working = busy == Some(svc);
            let text = if working { "Waiting…" } else if s.logged_in { "Sign out" } else { "Sign in" };
            let bw = text_w(&p, text, font_w(13., Weight::Semi)) + 32.;
            let b = Rect::from_center_size(pos2(row.max.x - 16. - bw / 2., cy), vec2(bw, 36.));
            let r = ui.interact(b, ui.scope_id().with(("acct", svc)), egui::Sense::click());
            if import {
                let iw = text_w(&p, "Import playlists", font_w(13., Weight::Semi)) + 32.;
                let ib = Rect::from_center_size(pos2(b.min.x - 8. - iw / 2., cy), vec2(iw, 36.));
                let ir = ui.interact(ib, ui.scope_id().with("import"), egui::Sense::click());
                p.rect_filled(ib, rad(18.), if ir.hovered() { t.s3.lerp_to_gamma(t.tx, 0.1) } else { t.s3 });
                p.text(ib.center(), Align2::CENTER_CENTER, "Import playlists", font_w(13., Weight::Semi), t.tx);
                if ir.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if ir.clicked() {
                    self.go(Route::Import);
                }
            }
            let primary = !s.logged_in;
            p.rect_filled(b, rad(18.), if primary { if r.hovered() { t.ax.gamma_multiply(0.9) } else { t.ax } } else if r.hovered() { t.s3.lerp_to_gamma(t.tx, 0.1) } else { t.s3 });
            p.text(b.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), if primary { t.on_ax } else { t.tx });
            if r.hovered() && !working {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() && busy.is_none() {
                d.account(ui.ctx(), svc, !s.logged_in);
            }
        }
        self.soundcloud_card(ui, Rect::from_min_size(pos2(card.min.x + 16., top + 16. + 2. * 84.), vec2(card.width() - 32., 72.)));
        self.lastfm_card(ui, Rect::from_min_size(pos2(card.min.x + 16., top + 16. + 3. * 84.), vec2(card.width() - 32., 72.)));
        if let Some(e) = err {
            self.note(ui, card, top + 16. + 4. * 84. - 20., &e);
        }
        4. * 84. + 12.
    }

    /// `note` for an offline panel; returns the height used.
    fn note_h(&self, ui: &egui::Ui, card: Rect, top: f32, text: &str) -> f32 {
        self.note(ui, card, top, text);
        44.
    }

    /// SoundCloud's browser sign-in (its `oauth_token` cookie); turning the source on or off is in Sources.
    fn soundcloud_card(&mut self, ui: &mut egui::Ui, row: Rect) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        p.rect_filled(row, rad(14.), t.s1);
        let cy = row.center().y;
        let src = self.sources.list(&ctx).and_then(|l| l.ok()).and_then(|l| l.into_iter().find(|s| s.r#type == "soundcloud"));
        let signed = src.as_ref().is_some_and(|s| s.secret_set);
        // The name and picture come from a connection test, which the daemon forgets on restart and on sign-in.
        if let Some(s) = src.as_ref().filter(|s| signed && s.status == "unchecked" && !self.sources.checking(&s.id)) {
            self.sources.test(&ctx, &s.id);
        }
        let who = src.as_ref().filter(|_| signed).and_then(|s| s.detail.as_deref()?.strip_prefix("Signed in as ")).map(str::to_string);
        let pic = src.as_ref().filter(|_| signed).and_then(|s| s.avatar_url.clone());
        if signed {
            self.avatar(ui, pic.as_deref(), pos2(row.min.x + 36., cy), 20., who.as_deref());
        } else {
            p.circle_filled(pos2(row.min.x + 36., cy), 20., t.s2);
            crate::gfx::icon(ui, icon!("src-soundcloud"), pos2(row.min.x + 36., cy), 22., t.tx);
        }
        let (tx, tw) = (row.min.x + 68., row.width() - 220.);
        trunc(&p, pos2(tx, cy - 9.), tw, "SoundCloud", font_w(14., Weight::Semi), t.tx);
        let line = match &src {
            None => "Connecting to the daemon…".to_string(),
            Some(s) if signed => s.detail.clone().filter(|d| d.starts_with("Signed in")).unwrap_or_else(|| "Signed in".into()),
            Some(_) => "Not signed in. Sign in to use your likes and library".into(),
        };
        trunc(&p, pos2(tx, cy + 10.), tw, &line, font(12.), t.tm);
        let working = self.sources.signing();
        let text = if working { "Waiting…" } else if signed { "Sign out" } else { "Sign in" };
        let bw = text_w(&p, text, font_w(13., Weight::Semi)) + 32.;
        let b = Rect::from_center_size(pos2(row.max.x - 16. - bw / 2., cy), vec2(bw, 36.));
        let r = ui.interact(b, ui.scope_id().with("acct-soundcloud"), egui::Sense::click());
        let primary = !signed;
        p.rect_filled(b, rad(18.), if primary { if r.hovered() { t.ax.gamma_multiply(0.9) } else { t.ax } } else if r.hovered() { t.s3.lerp_to_gamma(t.tx, 0.1) } else { t.s3 });
        p.text(b.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), if primary { t.on_ax } else { t.tx });
        if r.hovered() && !working {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked() && !working && src.is_some() {
            self.sources.soundcloud_auth(&ctx, !signed);
        }
    }

    /// Last.fm as a third connection beside YouTube Music and Spotify; its scrobbling options sit under the cards.
    fn lastfm_card(&mut self, ui: &mut egui::Ui, row: Rect) {
        let t = self.t;
        let p = ui.painter().clone();
        p.rect_filled(row, rad(14.), t.s1);
        let cy = row.center().y;
        let c = self.scrobbler.cfg().unwrap_or_default();
        let who = c.username.as_deref().filter(|_| c.logged_in);
        self.avatar(ui, c.avatar_url.as_deref().filter(|_| c.logged_in), pos2(row.min.x + 36., cy), 20., who);
        let (tx, tw) = (row.min.x + 68., row.width() - 220.);
        trunc(&p, pos2(tx, cy - 9.), tw, "Last.fm", font_w(14., Weight::Semi), t.tx);
        let line = match who {
            Some(n) => format!("Signed in as {n}"),
            None if !c.configured => "Add an API key below to connect".into(),
            None => "Not signed in".into(),
        };
        trunc(&p, pos2(tx, cy + 10.), tw, &line, font(12.), t.tm);
        let waiting = self.scrobbler.waiting();
        let text = if waiting { "Waiting…" } else if c.logged_in { "Sign out" } else { "Sign in" };
        let bw = text_w(&p, text, font_w(13., Weight::Semi)) + 32.;
        let b = Rect::from_center_size(pos2(row.max.x - 16. - bw / 2., cy), vec2(bw, 36.));
        let r = ui.interact(b, ui.scope_id().with("acct-lastfm"), egui::Sense::click());
        let primary = !c.logged_in;
        p.rect_filled(b, rad(18.), if primary { if r.hovered() { t.ax.gamma_multiply(0.9) } else { t.ax } } else if r.hovered() { t.s3.lerp_to_gamma(t.tx, 0.1) } else { t.s3 });
        p.text(b.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), if primary { t.on_ax } else { t.tx });
        if r.hovered() && !waiting {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked() && !waiting {
            let ctx = ui.ctx().clone();
            if c.logged_in {
                self.scrobbler.logout(&ctx);
            } else if c.configured {
                self.scrobbler.start_login(&ctx);
            } else {
                crate::toast::info("Add a Last.fm API key below first");
            }
        }
    }
}

/// The one source all of `ids` name, if they do not mix.
fn shared<'a>(ids: impl Iterator<Item = &'a str> + Clone) -> Option<&'a str> {
    ids.clone().next().filter(|_| !crate::sources::mixed(ids))
}

/// "Album · Artist", or just the kind when there is nothing after it.
fn kind_line(kind: &str, sub: &str) -> String {
    if sub.is_empty() { kind.into() } else { format!("{kind} · {sub}") }
}

fn is_year(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Spotify's rule of thumb: up to 3 songs under 30 minutes is a single, up to 6 an EP.
fn release_kind(tracks: &[Track]) -> &'static str {
    let mins = tracks.iter().filter_map(|t| t.duration_sec).sum::<u32>() / 60;
    match tracks.len() {
        n if n <= 3 && mins < 30 => "Single",
        n if n <= 6 && mins < 30 => "EP",
        _ => "Album",
    }
}

const DISCO_CHIPS: [&str; 3] = ["All", "Albums", "Singles and EPs"];
/// A release's header row: cover, title, details and play button.
const DISCO_HEAD: f32 = 124.;

impl App {
    /// Every release of an artist, newest first, each with its track list under it (rows are only drawn while on screen).
    pub fn discography_page(&mut self, ui: &mut egui::Ui, card: Rect, artist_id: &str) {
        let t = self.t;
        let name = self.data.detail(ui.ctx(), "artists", artist_id).ready().map(|d| d.item.title.clone()).unwrap_or_default();
        let p = ui.painter().clone();
        trunc(&p, pos2(card.min.x + 22., card.min.y + 38.), card.width() - 44., "Discography", font_w(26., Weight::Bold), t.tx);
        trunc(&p, pos2(card.min.x + 22., card.min.y + 66.), card.width() - 44., &name, font(13.), t.tm);
        let top = card.min.y + 90.;
        let all = match &*self.data.discography(ui.ctx(), artist_id).lock() {
            State::Ready(v) => v.clone(),
            State::Failed(e) => return self.note(ui, card, top, e),
            _ => return self.note(ui, card, top, "Loading releases…"),
        };
        let chip_id = ui.scope_id().with(("disco-chip", artist_id));
        let cur = ui.ctx().data(|m| m.get_temp::<usize>(chip_id)).unwrap_or(0);
        let chip = self.chips(ui, card.min.x + 22., top, &DISCO_CHIPS, cur);
        if chip != cur {
            ui.ctx().data_mut(|m| m.insert_temp(chip_id, chip));
        }
        let shown: Vec<&crate::data::Detail> = all.iter().filter(|d| match chip {
            1 => release_kind(&d.tracks) == "Album",
            2 => release_kind(&d.tracks) != "Album",
            _ => true,
        }).collect();
        if shown.is_empty() {
            return self.note(ui, card, top + 44., "No releases here.");
        }
        let list = Rect::from_min_max(pos2(card.min.x, top + 48.), card.max);
        let mut lui = App::region(ui, list);
        let (tl, tr) = (card.min.x + 6., card.max.x - 6.);
        let rh = if self.opts.compact { 34. } else { crate::page::ROW_H };
        let look = Look { cover_w: 0., open: false, album: false };
        let out = ScrollArea::vertical().id_salt(("disco", artist_id, chip)).auto_shrink(false).show_viewport(&mut lui, |ui, vp| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let mut y = 0.;
            for d in &shown {
                let h = DISCO_HEAD + HEAD + d.tracks.len() as f32 * rh + 28.;
                let visible = y + h >= vp.min.y && y <= vp.max.y;
                y += h;
                if !visible {
                    ui.add_space(h);
                    continue;
                }
                ui.push_id(("disco-release", &d.item.id), |ui| {
                    let (_, hr) = ui.allocate_space(vec2(list.width(), DISCO_HEAD));
                    let p = ui.painter().clone();
                    let art = Rect::from_min_size(pos2(card.min.x + 22., hr.min.y + 14.), vec2(96., 96.));
                    shadow(&p, art, 6., 4, 14, 50);
                    self.thumb(ui, d.item.thumbnail.as_deref(), art, 6.);
                    let tx = art.max.x + 18.;
                    let title_r = trunc(&p, pos2(tx, art.min.y + 26.), (card.max.x - 22. - tx).max(0.), &d.item.title, font_w(20., Weight::Bold), t.tx);
                    let open = ui.interact(art.union(title_r), ui.scope_id().with("open"), egui::Sense::click());
                    if open.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        p.line_segment([title_r.left_bottom() + vec2(0., 1.), title_r.right_bottom() + vec2(0., 1.)], (1., t.tx));
                    }
                    if open.clicked() {
                        self.open_album(&d.item.id);
                    }
                    let year = d.item.subtitle.as_deref().and_then(|s| s.rsplit(" · ").next()).filter(|y| is_year(y));
                    let meta = [Some(release_kind(&d.tracks)), year].into_iter().flatten().map(String::from).chain([summary(&d.tracks)]).collect::<Vec<_>>().join(" · ");
                    trunc(&p, pos2(tx, art.min.y + 54.), (card.max.x - 22. - tx).max(0.), &meta, font(13.), t.tm);
                    let pb = Rect::from_center_size(pos2(tx + 16., art.max.y - 16.), vec2(32., 32.));
                    let pr = ui.interact(pb, ui.scope_id().with("play"), egui::Sense::click());
                    p.circle_filled(pb.center(), 16., if pr.is_pointer_button_down_on() { t.ax.gamma_multiply(0.85) } else { t.ax });
                    icon(ui, icon!("play"), pb.center(), 18., t.on_ax);
                    if pr.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if pr.clicked() {
                        self.pl.play_tracks(ui.ctx(), &d.tracks, 0, false);
                    }
                    let (_, head) = ui.allocate_space(vec2(list.width(), HEAD));
                    self.table_head(ui, head.min.y + 14., tl, tr, look, None);
                    let acts = self.track_rows(ui, "disco", &d.tracks, 0..d.tracks.len(), (tl, tr), rh, look);
                    self.apply_acts(ui, &d.tracks, acts, look);
                    ui.add_space(28.);
                });
            }
            ui.add_space(70.);
        });
        scroll_fades(&lui, &out, t.s1);
    }
}

/// "12 songs, 41 min 5 sec" over the tracks' durations.
fn summary(tracks: &[Track]) -> String {
    let secs: u32 = tracks.iter().filter_map(|t| t.duration_sec).sum();
    let n = tracks.len();
    let songs = format!("{n} song{}", if n == 1 { "" } else { "s" });
    match secs {
        0 => songs,
        s if s >= 3600 => format!("{songs}, {} hr {} min", s / 3600, s % 3600 / 60),
        s if s >= 600 => format!("{songs}, {} min", s / 60),
        s => format!("{songs}, {} min {} sec", s / 60, s % 60),
    }
}

impl App {
    /// A song with no album, shown as a one-track playlist.
    fn single_page(&mut self, ui: &mut egui::Ui, card: Rect) {
        let Some(track) = self.song.clone() else {
            return self.note(ui, card, card.min.y + 40., "Pick a song");
        };
        // Feed items rarely say which album they are on; look it up and, if there is one, show that album instead.
        if track.source == "youtube" {
            let info = self.data.song_info(ui.ctx(), &track.id);
            let g = info.lock();
            match &*g {
                State::Ready(t) if t.album_id.is_some() => {
                    self.sel = Some(("albums", t.album_id.clone().unwrap_or_default()));
                    self.song = None;
                    return;
                }
                State::Ready(_) | State::Failed(_) => {}
                _ => return self.detail_skeleton(ui, card, "albums", &track.title, track.thumbnail.as_deref(), &track.artists.join(", ")),
            }
        }
        let one = std::slice::from_ref(&track);
        let by = Some(track.artists.first().map_or((crate::data::UNKNOWN_ARTIST, None), |n| (n.as_str(), track.artist_id.as_deref())));
        let off = self.tracks_table(ui, card, one, Some((track.artist_id.as_deref().unwrap_or(""), &[], "")));
        self.hero(ui, card, false, "Single", &track.title, by, &summary(one), track.thumbnail.as_deref(), one, None, off);
    }
}

/// The logo for a source id (`yt`/`youtube`, or a configured source's id whose prefix is its type).
pub fn source_icon(src: &str) -> Option<&'static str> {
    match src.split('-').next()? {
        "yt" | "youtube" => Some(icon!("src-youtube")),
        "" => None,
        "spotify" => Some(icon!("mspot")),
        "jellyfin" => Some(icon!("src-jellyfin")),
        "soundcloud" => Some(icon!("src-soundcloud")),
        "bandcamp" => Some(icon!("src-bandcamp")),
        _ => Some(icon!("src-generic")),
    }
}

/// Draws `src`'s logo at `x` (left edge) centred on `cy` with `name` as its tooltip; returns the width it takes.
pub fn source_mark_at(ui: &egui::Ui, t: &crate::theme::Theme, src: &str, name: &str, x: f32, cy: f32, col: Color32) -> f32 {
    let Some(uri) = source_icon(src) else { return 0. };
    let c = pos2(x + 6., cy);
    icon(ui, uri, c, 12., col);
    let hit = ui.interact(Rect::from_center_size(c, vec2(16., 16.)), ui.scope_id().with(("src-mark", x as i32, cy as i32)), egui::Sense::hover());
    crate::tip::tip(&hit, t, name, false);
    18.
}
