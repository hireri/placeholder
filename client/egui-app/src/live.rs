//! Pages backed by the daemon: home feed, album/artist/playlist detail and the account settings.

use crate::app::{App, Route};
use crate::backend;
use crate::data::{Item, Section, State, Track};
use crate::gfx::*;
use crate::menu::Target;
use crate::page::{Look, HEAD, HERO_H, HERO_MIN};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, ScrollArea};
use std::borrow::Cow;

const GAP: f32 = 8.;
/// Seconds a carousel arrow takes to slide the row.
const CAR_SLIDE: f64 = 0.45;
const CHIPS: [(&str, &str); 5] = [("All", ""), ("Songs", "song"), ("Albums", "album"), ("Playlists", "playlist"), ("Artists", "artist")];
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
        ui.painter().rect_filled(rect, rad(r), self.t.s3);
        if let Some(u) = url {
            paint_cover(ui, u, rect, rad(r), Color32::WHITE);
        }
    }

    /// Opens an item's page: albums, playlists and artists their own, a song the album it sits on (or its single).
    pub(crate) fn open_item(&mut self, _ctx: &egui::Context, it: &Item) {
        if it.r#type == "song" {
            return self.open_song(it.track());
        }
        let Some((kind, route)) = target(it) else { return };
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
        let a = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("play-badge-a", &key)), over, 0.16, crate::gfx::ease_out);
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

    fn item_card(&mut self, ui: &mut egui::Ui, cell: Rect, it: &Item, queue: &[&Item], key: (usize, usize)) {
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
        let p = ui.painter();
        trunc(p, pos2(art.min.x, art.max.y + 16.), art.width(), &it.title, font_w(13., Weight::Medium), t.tx);
        trunc(p, pos2(art.min.x, art.max.y + 33.), art.width(), it.subtitle.as_deref().unwrap_or(""), font(12.), t.tm);
    }

    /// One titled, horizontally scrolling row of cards; arrows and edge fades show when more cards are off-screen.
    /// `more` adds a "Show all" button that opens the shelf as a wrapped page.
    pub(crate) fn carousel<'a>(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32), title: &str, items: impl Iterator<Item = &'a Item>, si: usize, more: bool) {
        let t = self.t;
        let all: Vec<&Item> = items.collect();
        // fixed card width: a narrower window clips cards instead of resizing them
        let cw = 160.;
        let cols = (((w + GAP) / (cw + GAP)) as usize).max(1);
        let step = cw + GAP;
        let max_off = (all.len() as f32 * step - GAP - w).max(0.);
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
                self.open(Route::Shelf, Some(("shelf", title.to_string())));
            }
            rx -= bw + 4.;
        }
        trunc(&p, pos2(l + 8., cy), (rx - l - 16.).max(0.), title, font_w(17., Weight::Bold), t.tx);

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
            let (_, r) = ui.allocate_space(vec2((all.len() as f32 * step - GAP).max(w), view.height()));
            let vis = ui.clip_rect();
            for (c, &it) in all.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(r.min.x + c as f32 * step, r.min.y), vec2(cw, cw + 40.));
                if cell.intersects(vis) {
                    self.item_card(ui, cell, it, &all, (si, c));
                }
            }
        });
        let off = out.state.clamped_offset().x;
        ui.ctx().data_mut(|d| d.insert_temp(id, off));
        let (left, right) = (ui.ctx().animate_bool_with_time(id.with("fade-l"), off > 1., 0.15), ui.ctx().animate_bool_with_time(id.with("fade-r"), off < max_off - 1., 0.15));
        if left > 0. {
            fade_h(&p, Rect::from_min_size(view.min, vec2(48., view.height())), t.s1.gamma_multiply(left), false);
        }
        if right > 0. {
            fade_h(&p, Rect::from_min_max(pos2(view.max.x - 48., view.min.y), view.max), t.s1.gamma_multiply(right), true);
        }
    }

    /// Every item of a shelf in a wrapped grid, virtualised by row.
    pub fn wall(&mut self, ui: &mut egui::Ui, card: Rect, top: f32, items: &[&Item], salt: &str) {
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let (cols, cw) = grid(w, 160., GAP);
        let rh = cw + 48.;
        let rows = items.len().div_ceil(cols);
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
                    let cell = Rect::from_min_size(pos2(l + c as f32 * (cw + GAP), row.min.y), vec2(cw, cw + 40.));
                    self.item_card(ui, cell, it, items, (0, r * cols + c));
                }
            }
            ui.add_space((rows - last) as f32 * rh + 70.);
        });
        scroll_fades(&lui, &out, self.t.s1);
    }

    /// A home shelf listed in full, honouring the home page's type chip.
    pub fn shelf_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let Some((_, title)) = self.sel.clone() else { return };
        let d = self.data.clone();
        let g = d.home.lock();
        let State::Ready(secs) = &*g else { return self.note(ui, card, top, "Loading…") };
        let kind = CHIPS[self.home_chip].1;
        let items: Vec<&Item> = secs.iter().filter(|s| s.title == title).flat_map(|s| &s.items).filter(|i| kind.is_empty() || i.r#type == kind).collect();
        self.wall(ui, card, top, &items, "shelf");
    }

    /// Titled carousels, one per section, under the collapsible hero; returns the scroll offset.
    pub fn sections(&mut self, ui: &mut egui::Ui, card: Rect, secs: &[Section]) -> f32 {
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let area = Rect::from_min_max(pos2(card.min.x, card.min.y + HERO_MIN), card.max);
        let mut lui = App::region(ui, area);
        let out = ScrollArea::vertical().id_salt(("sections", &self.sel)).auto_shrink(false).show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            ui.add_space(HERO_H - HERO_MIN);
            for (si, s) in secs.iter().enumerate() {
                self.carousel(ui, (l, w, area.width()), &s.title, s.items.iter(), si, false);
            }
            ui.add_space(70.);
        });
        scroll_fades(&lui, &out, self.t.s1);
        out.state.clamped_offset().y
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
    fn feature_card(&mut self, ui: &mut egui::Ui, cell: Rect, label: &str, queue: &[&Item], key: usize) {
        let t = self.t;
        let Some(&it) = queue.first() else { return };
        let names: Vec<&str> = queue.iter().skip(1).take(3).map(|i| i.title.as_str()).collect();
        let with = if names.is_empty() { String::new() } else { format!("With {} and more", names.join(", ")) };
        let r = ui.interact(cell, ui.scope_id().with(("feat", key)), egui::Sense::click());
        let p = ui.painter().clone();
        trunc(&p, pos2(cell.min.x, cell.min.y - 12.), cell.width(), label, font(12.5), t.tm);
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
        trunc(&p, pos2(tx, art.min.y + 46.), tw, &[Some(kind), it.subtitle.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" · "), font(12.5), t.tm);
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
            x += w + GAP;
        }
        sel
    }

    pub fn home_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let labels: Vec<&str> = CHIPS.iter().map(|c| c.0).collect();
        self.home_chip = self.chips(ui, card.min.x + 22., top, &labels, self.home_chip);
        let top = top + 44.;
        let d = self.data.clone();
        d.status.ensure(ui.ctx(), || backend::get("/auth/status"));
        let user = match &*d.status.lock() {
            State::Ready(s) => s.youtube.user.clone().filter(|_| s.youtube.logged_in),
            _ => None,
        };
        d.home.ensure(ui.ctx(), || backend::get("/youtube/home"));
        let g = d.home.lock();
        match &*g {
            State::Ready(s) => self.home_feed(ui, card, top, s, user.as_ref()),
            State::Failed(e) => self.note(ui, card, top, e),
            _ => self.note(ui, card, top, "Loading…"),
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
            if let Some(u) = user {
                let (_, r) = ui.allocate_space(vec2(area.width(), 52.));
                let av = Rect::from_min_size(pos2(card.min.x + 22., r.min.y + 4.), vec2(40., 40.));
                self.thumb(ui, u.avatar_url.as_deref(), av, 20.);
                trunc(ui.painter(), pos2(av.max.x + 12., av.center().y), card.width() - 100., &u.name, font_w(15., Weight::Semi), self.t.tx);
            }
            if let Some(s) = quick {
                let cols = (((w + GAP) / 240.) as usize).clamp(2, 4);
                let cw = (w - 16. - GAP * (cols - 1) as f32) / cols as f32;
                let (_, r) = ui.allocate_space(vec2(area.width(), 56. * 2. + GAP + 16.));
                let all: Vec<&Item> = s.items.iter().collect();
                for (n, it) in s.items.iter().take(cols * 2).enumerate() {
                    let cell = Rect::from_min_size(pos2(l + 8. + (n % cols) as f32 * (cw + GAP), r.min.y + (n / cols) as f32 * (56. + GAP)), vec2(cw, 56.));
                    self.quick_tile(ui, cell, it, &all, n);
                }
            }
            let rest: Vec<&Section> = shelves.collect();
            for (si, s) in rest.iter().take(3).enumerate() {
                self.carousel(ui, (l, w, area.width()), &s.title, s.items.iter().filter(ok), si, true);
            }
            let feats = &rest[rest.len().min(3)..];
            let fcols = (((w + GAP) / 300.) as usize).clamp(1, 4);
            let cw = (w - 16. - GAP * (fcols - 1) as f32) / fcols as f32;
            let ch = (cw * 1.2).clamp(260., 420.);
            for (row, chunk) in feats.chunks(fcols).enumerate() {
                let (_, r) = ui.allocate_space(vec2(area.width(), 28. + ch + 14.));
                for (c, s) in chunk.iter().enumerate() {
                    let shelf: Vec<&Item> = s.items.iter().filter(ok).collect();
                    let cell = Rect::from_min_size(pos2(l + 8. + c as f32 * (cw + GAP), r.min.y + 28.), vec2(cw, ch));
                    self.feature_card(ui, cell, &s.title, &shelf, row * 4 + c);
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
                let year = p.item.subtitle.as_deref().and_then(|s| s.rsplit(" · ").next()).filter(|y| y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit())).map(String::from);
                let note = year.as_deref().map(|y| format!("Released {y}")).unwrap_or_default();
                let sub = if album {
                    [year, Some(summary(&p.tracks))].into_iter().flatten().collect::<Vec<_>>().join(" · ")
                } else {
                    [p.item.subtitle.clone(), (!p.tracks.is_empty()).then(|| summary(&p.tracks))].into_iter().flatten().collect::<Vec<_>>().join(" · ")
                };
                let first = p.tracks.first();
                let by = first.filter(|_| album).and_then(|t| t.artists.first()).map(|n| (n.as_str(), first.and_then(|t| t.artist_id.as_deref())));
                // Only YouTube albums and playlists can be saved; the saved state is read off the sidebar's library list.
                let save = match kind {
                    "albums" | "playlists" => {
                        let lib = if kind == "albums" { 2 } else { 1 };
                        Some((kind, id, self.lib_items.iter().any(|i| i.kind == lib && i.id == id)))
                    }
                    _ => None,
                };
                // The list scrolls under the hero, which shrinks as it does, so the list is drawn first to know the offset.
                let off = if p.tracks.is_empty() { self.sections(ui, card, &p.sections) } else { self.tracks_table(ui, card, &p.tracks, album.then(|| (by.and_then(|b| b.1).unwrap_or(""), p.sections.as_slice(), note.as_str()))) };
                // While a Spotify import resolves, only its matched rows can play.
                let playable: Cow<'_, [Track]> = if kind == "spotify" {
                    Cow::Owned(p.tracks.iter().filter(|t| !t.resolving && !t.unavailable).cloned().collect())
                } else {
                    Cow::Borrowed(&p.tracks)
                };
                self.hero(ui, card, round, head, &p.item.title, by, &sub, p.item.thumbnail.as_deref(), &playable, save, off);
            }
            Err(Some(e)) => self.note(ui, card, card.min.y + 40., &e),
            Err(None) => self.note(ui, card, card.min.y + 40., "Loading…"),
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
            let sub = if q.is_empty() { "Press Ctrl K to search" } else { "Songs, videos, albums and artists on YouTube Music" };
            trunc(&p, pos2(l, hr.min.y + 66.), full - 44., sub, font(13.), t.tm);
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
                heading(ui, if side { l + TOP_W + 20. } else { l }, "Songs", hd);
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
                    heading(ui, l, "Videos", hd);
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
        let (cols, cw) = grid(w, 160., GAP);
        for r in 0..items.len().div_ceil(cols) {
            let (_, row) = ui.allocate_space(vec2(card.width(), cw + 48.));
            for c in 0..cols {
                let Some(&it) = items.get(r * cols + c) else { break };
                let cell = Rect::from_min_size(pos2(l + c as f32 * (cw + GAP), row.min.y), vec2(cw, cw + 40.));
                self.item_card(ui, cell, it, items, (si, r * cols + c));
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
        let sub = [Some(kind), it.subtitle.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        trunc(&p, pos2(rect.min.x + 18., art.max.y + 50.), tw, &sub, font(13.), t.tm);
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
        trunc(&p, pos2(tx, rect.center().y + 10.), tw, "Artist", font(12.), t.tm);
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
            Err(None) => self.note(ui, card, top, "Loading…"),
        }
    }

    /// Sign-in rows for each service, drawn inside the Settings page's Accounts panel (`card`).
    pub fn accounts_panel(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let (t, d) = (self.t, self.data.clone());
        d.status.ensure(ui.ctx(), || backend::get("/auth/status"));
        let g = d.status.lock();
        let st = match &*g {
            State::Ready(s) => s.clone(),
            State::Failed(e) => return self.note(ui, card, top, e),
            _ => return self.note(ui, card, top, "Connecting to the daemon…"),
        };
        drop(g);
        let (busy, err) = d.busy.lock().unwrap().clone();
        for (i, (svc, label, s)) in [("youtube", "YouTube Music", &st.youtube), ("spotify", "Spotify", &st.spotify)].into_iter().enumerate() {
            let row = Rect::from_min_size(pos2(card.min.x + 16., top + 16. + i as f32 * 84.), vec2(card.width() - 32., 72.));
            let p = ui.painter().clone();
            p.rect_filled(row, rad(14.), t.s1);
            let cy = row.center().y;
            p.circle_filled(pos2(row.min.x + 36., cy), 20., t.axc);
            let who = s.user.as_ref().map(|u| u.name.as_str());
            p.text(pos2(row.min.x + 36., cy), Align2::CENTER_CENTER, who.and_then(|n| n.chars().next()).unwrap_or('?').to_uppercase().to_string(), font_w(15., Weight::Semi), t.tx);
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
        if let Some(e) = err {
            self.note(ui, card, top + 16. + 2. * 84. - 20., &e);
        }
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
                _ => return self.note(ui, card, card.min.y + 40., "Loading…"),
            }
        }
        let one = std::slice::from_ref(&track);
        let by = track.artists.first().map(|n| (n.as_str(), track.artist_id.as_deref()));
        let off = self.tracks_table(ui, card, one, Some((track.artist_id.as_deref().unwrap_or(""), &[], "")));
        self.hero(ui, card, false, "Single", &track.title, by, &summary(one), track.thumbnail.as_deref(), one, None, off);
    }
}
