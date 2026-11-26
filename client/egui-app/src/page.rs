//! Centre panel: album page (header wash, actions, virtualized track list).

use crate::app::{App, Route};
use crate::data::{Item, Section, State, Track};
use crate::gfx::*;
use crate::menu::Target;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Painter, Rect, ScrollArea};
use std::time::Duration;

const ROW_H: f32 = 44.;
const COVER_ROW_H: f32 = 60.;
const COVER_ROW_COMPACT: f32 = 48.;
/// Cover rows are 14px taller than their cover.
const COVER_PAD: f32 = 14.;
/// Hero height at the top of the page and once collapsed into a single row by scrolling.
pub const HERO_H: f32 = 249.;
pub const HERO_MIN: f32 = 72.;
pub const HEAD: f32 = 32.;

/// How a table's rows look: width of the cover beside each row (0 for a number column) and whether a click opens the song's album instead of playing it.
#[derive(Clone, Copy)]
pub struct Look {
    pub cover_w: f32,
    pub open: bool,
    /// Adds an ALBUM column (playlists, whose songs come from many albums).
    pub album: bool,
}

impl Look {
    fn num(self) -> f32 {
        if self.cover_w > 0. { 30. + self.cover_w } else { 36. }
    }
}

/// What the pointer did in a batch of rows, applied once drawing is done.
#[derive(Default)]
pub struct Acts {
    clicked: Option<usize>,
    play: Option<usize>,
    menu: Option<(egui::Pos2, usize)>,
    artist: Option<String>,
    playing: bool,
    like: Option<usize>,
}

/// Centre of the number slot from the column's left edge; the number, play icon and equaliser all sit on it.
const NUM_X: f32 = 9.;

/// Width reserved right of the time column for the fav and "more" buttons.
const ACT_W: f32 = 64.;

/// Column x-ranges inside a row of width `w`: # (`num` wide) | title (1.6) | artist (1) | album (1, `album` only; else zero wide) | time | buttons.
fn columns(left: f32, w: f32, num: f32, album: bool) -> [(f32, f32); 6] {
    let gaps = if album { 40. } else { 32. };
    let flex = (w - 24. - num - 52. - ACT_W - gaps).max(0.);
    let units = if album { 3.6 } else { 2.6 };
    let (tw, aw) = (flex * 1.6 / units, flex / units);
    let alw = if album { aw } else { 0. };
    let n = left + 12.;
    let title = n + num + 8.;
    let artist = title + tw + 8.;
    let alb = artist + aw + 8.;
    let dur = alb + if album { alw + 8. } else { 0. };
    [(n, num), (title, tw), (artist, aw), (alb, alw), (dur, 52.), (dur + 52. + 8., ACT_W)]
}

/// Three bars with their left edge at `x`, bottoms at `cy + 6`; bouncing while `playing`, otherwise static.
fn equaliser(p: &Painter, x: f32, cy: f32, time: f32, playing: bool, col: Color32) {
    for b in 0..3 {
        let h = if playing { 3. + 9. * (0.5 + 0.5 * (time * 6. + b as f32 * 2.1).sin()) } else { [5., 9., 6.][b] };
        let bx = x + b as f32 * 4.;
        p.rect_filled(Rect::from_min_max(pos2(bx, cy + 6. - h), pos2(bx + 2., cy + 6.)), rad(1.), col);
    }
}

/// Buffering indicator: a steady dot with two sonar ripples that swell and fade, half a cycle apart.
pub fn spinner(p: &Painter, center: egui::Pos2, r: f32, time: f32, col: Color32) {
    let ease = |t: f32| 1. - (1. - t).powi(3);
    for k in 0..2 {
        let t = (time / 1.4 + k as f32 * 0.5).fract();
        p.circle_stroke(center, r * (0.3 + 0.7 * ease(t)), egui::Stroke::new(1.6, col.gamma_multiply((1. - t).powf(1.5))));
    }
    p.circle_filled(center, r * 0.2, col.gamma_multiply(0.8 + 0.2 * (time * 4.5).sin()));
}

/// `tracks` ordered by column `col` (1 title, 2 artist, 3 album, 4 time), case-insensitively; blanks first when ascending.
fn sort_tracks(tracks: &[Track], col: usize, asc: bool) -> Vec<Track> {
    let mut v = tracks.to_vec();
    match col {
        1 => v.sort_by_cached_key(|t| t.title.to_lowercase()),
        2 => v.sort_by_cached_key(|t| t.artists.join(", ").to_lowercase()),
        3 => v.sort_by_cached_key(|t| t.album.as_deref().unwrap_or("").to_lowercase()),
        _ => v.sort_by_key(|t| t.duration_sec),
    }
    if !asc {
        v.reverse();
    }
    v
}

impl App {
    /// Shared card shell for every route: shadow, rounded panel, routed content, bottom fade and corner fix.
    pub fn page(&mut self, ui: &mut egui::Ui, card: Rect) {
        let t = self.t;
        let mut owned = App::region(ui, card.expand(40.));
        let ui = &mut owned;
        let p = ui.painter().clone();

        // Card: shadow outside, panel fill and rounded clip inside.
        shadow(&p, card, 18., 10, 32, 61);
        p.rect_filled(card, rad(18.), t.s1);
        let mut ui = App::region(ui, card);
        let ui = &mut ui;
        let p = ui.painter().clone();

        match self.route {
            Route::Album | Route::Artist if self.sel.is_some() => {
                let (kind, id) = self.sel.clone().unwrap();
                self.live_detail(ui, card, kind, &id);
            }
            Route::History => self.stats_page(ui, card),
            r => self.plain(ui, card, r),
        }
        round_bottom_corners(&p, card, 18., t.s0);
    }

    /// Plain header (title + subtitle) over a track table, a card grid, or a placeholder.
    fn plain(&mut self, ui: &mut egui::Ui, card: Rect, r: Route) {
        if r == Route::Search {
            return self.search_page(ui, card);
        }
        let t = self.t;
        let p = ui.painter().clone();
        let shelf = self.sel.as_ref().map(|s| s.1.clone()).unwrap_or_default();
        let (title, sub) = match r {
            Route::Home => ("Home", "From YouTube Music"),
            Route::Shelf => (shelf.as_str(), "From YouTube Music"),
            Route::Downloads => ("Downloads", "Songs saved on this device"),
            Route::Local => ("Local Music", ""),
            Route::Album => ("Album", ""),
            Route::Artist => ("Artist", ""),
            Route::Settings => ("Settings", ""),
            _ => ("Spotify import", "Your Spotify playlists"),
        };
        trunc(&p, pos2(card.min.x + 22., card.min.y + 38.), card.width() - 44., title, font_w(26., Weight::Bold), t.tx);
        trunc(&p, pos2(card.min.x + 22., card.min.y + 66.), card.width() - 44., sub, font(13.), t.tm);
        let top = card.min.y + 90.;
        match r {
            Route::Import => self.import_page(ui, card, top),
            Route::Downloads => self.note(ui, card, top, "No downloads yet."),
            Route::Local => self.note(ui, card, top, "No local music folders yet."),
            Route::Album => self.note(ui, card, top, "Pick an album from Home, Search or your library."),
            Route::Artist => self.note(ui, card, top, "Pick an artist from Home, Search or your library."),
            Route::Home => self.home_page(ui, card, top),
            Route::Shelf => self.shelf_page(ui, card, top),
            Route::Settings => self.settings_page(ui, card, top),
            _ => {}
        }
    }

    /// Wash header with cover (round for artists), title block and action buttons.
    /// `save` is (kind, id, saved) when the page can be added to the library. `off` is how far the page below has
    /// scrolled: the wash shrinks with it while the tall layout fades out and a one-row layout fades in.
    pub fn hero(&mut self, ui: &mut egui::Ui, card: Rect, round: bool, kicker: &str, title: &str, by: Option<(&str, Option<&str>)>, sub: &str, cover_url: Option<&str>, tracks: &[Track], save: Option<(&'static str, &str, bool)>, off: f32) {
        let t = self.t;
        let k = (off / (HERO_H - HERO_MIN)).clamp(0., 1.);
        let wash = Rect::from_min_size(card.min, vec2(card.width(), 192. + (HERO_MIN - 192.) * k));
        // Album and artist pages can be right-clicked and opened from the menu; `sel` is only set on those.
        let page_item = match (&self.sel, self.route) {
            (Some((kind, id)), Route::Album | Route::Artist) if *kind != "song" => Some(Item {
                r#type: match *kind { "albums" => "album", "playlists" => "playlist", "artists" => "artist", _ => "spotify" }.into(),
                id: id.clone(),
                title: title.into(),
                thumbnail: cover_url.map(String::from),
                ..Default::default()
            }),
            _ => None,
        };
        let wash_hit = ui.interact(wash, ui.scope_id().with("wash"), egui::Sense::click());
        if let (true, Some(it), Some(pos)) = (wash_hit.secondary_clicked(), &page_item, wash_hit.interact_pointer_pos()) {
            self.ctx_menu = Some((pos, Target::Item(it.clone())));
        }
        let wash_tex = self.g.wash(ui.ctx(), &t, cover_url);
        if self.opts.tint {
            egui::Image::new(sized(wash_tex, 1.)).fit_to_exact_size(wash.size()).corner_radius(rad(18.)).paint_at(ui, wash);
        } else {
            ui.painter().rect_filled(wash, rad(18.), t.s2);
        }
        // The layout swaps once the page has scrolled past a point, as a quick crossfade (never both at once,
        // so the layers' widgets never share an id or overlap); the wash itself still shrinks with the scroll.
        let a = ui.ctx().animate_bool_with_time(ui.scope_id().with("hero-swap"), off > 40., 0.16);
        for (collapsed, alpha) in [(false, 1. - 2. * a), (true, 2. * a - 1.)] {
            if alpha <= 0. {
                continue;
            }
            let r = Rect::from_min_size(card.min, vec2(card.width(), HERO_H + (HERO_MIN - HERO_H) * k));
            let mut lui = ui.new_child(egui::UiBuilder::new().id_salt(("hero", collapsed)).max_rect(r));
            lui.set_clip_rect(r.intersect(ui.clip_rect()));
            lui.set_opacity(alpha.min(1.));
            self.hero_layer(&mut lui, card, collapsed, round, kicker, title, by, sub, cover_url, tracks, save, &page_item);
        }
    }

    fn hero_layer(&mut self, ui: &mut egui::Ui, card: Rect, collapsed: bool, round: bool, kicker: &str, title: &str, by: Option<(&str, Option<&str>)>, sub: &str, cover_url: Option<&str>, tracks: &[Track], save: Option<(&'static str, &str, bool)>, page_item: &Option<Item>) {
        let t = self.t;
        let p = ui.painter().clone();
        let k = if collapsed { 1. } else { 0. };
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        let side = lerp(148., 52.).round();
        // A video's wide thumbnail keeps its shape instead of being cropped square.
        let wide = cover_url.filter(|_| !round).and_then(|u| cover_aspect(ui.ctx(), u, vec2(side, side))).filter(|a| *a > 1.3);
        let art = Rect::from_min_size(pos2(card.min.x + 22., card.min.y + lerp(24., 10.)), vec2(side * wide.unwrap_or(1.), side));
        let ar = if round { side / 2. } else { lerp(6., 4.) };

        // Action buttons: below the wash at the top, right-aligned in the row once collapsed.
        let play_w = 14. + 16. + 8. + text_w(&p, "Play", font_w(13., Weight::Semi)) + 18.;
        let mut labels = vec!["Shuffle"];
        if let Some((kind, id, saved)) = save {
            let owned = saved && kind == "playlists" && self.data.library_item(id).is_some_and(|i| i.editable);
            labels.push(if owned { "Delete" } else if saved { "Saved" } else { "Save" });
        }
        if matches!(self.sel.as_ref().map(|s| s.0), Some("spotify")) {
            labels.push("Export");
        }
        let acts_w = play_w + labels.iter().map(|l| text_w(&p, l, font(13.)) + 32.).sum::<f32>() + if page_item.is_some() { 43. } else { 0. };
        let ay = card.min.y + lerp(HERO_H - 43., (HERO_MIN - 35.) / 2.);
        let ax = lerp(card.min.x + 22., card.max.x - 22. - acts_w);
        shadow(&p, art, ar, 6, 22, 71);
        self.thumb(ui, cover_url, art, ar);

        // Meta block, bottom-aligned with the art: kicker, big title, then "Artist · details" with the artist a link.
        let mx = art.max.x + 22.;
        let mw = (card.max.x - 22. - mx - k * (acts_w + 16.)).max(0.);
        let by_y = art.max.y - lerp(10., 6.);
        let (name, aid) = by.unzip();
        let aid = aid.flatten();
        let mut x = mx;
        if let Some(n) = name {
            let nf = font_w(14., Weight::Semi);
            let r = trunc(&p, pos2(x, by_y), mw, n, nf.clone(), t.tx);
            if let Some(id) = aid {
                let hit = ui.interact(r.expand2(vec2(2., 4.)), ui.scope_id().with("hero-artist"), egui::Sense::click());
                if hit.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    p.line_segment([r.left_bottom() + vec2(0., 1.), r.right_bottom() + vec2(0., 1.)], (1., t.tx));
                }
                if hit.clicked() {
                    self.open_artist(id);
                }
            }
            x = r.max.x + 6.;
            if !sub.is_empty() {
                p.text(pos2(x, by_y), Align2::LEFT_CENTER, "·", font(14.), t.tm);
                x += 14.;
            }
        }
        trunc(&p, pos2(x, by_y), (card.max.x - 22. - x).max(0.), sub, font(13.5), t.tm);
        // Titles are huge and tight, like the mock; long ones ellipsise rather than wrap.
        let title_f = font_w(lerp(if title.chars().count() > 28 { 34. } else { 44. }, 20.).round(), Weight::Bold);
        let title_cy = by_y - 14. - title_f.size * 0.6;
        trunc(&p, pos2(mx, title_cy), mw, title, title_f.clone(), t.tx);
        if !collapsed {
            trunc(&p, pos2(mx, title_cy - title_f.size * 0.6 - 12.), mw, kicker, font_w(12., Weight::Medium), t.tx);
        }

        let play = Rect::from_min_size(pos2(ax, ay), vec2(play_w, 35.));
        let r = ui.interact(play, ui.scope_id().with("play"), egui::Sense::click());
        p.rect_filled(play, rad(17.5), if r.is_pointer_button_down_on() { t.ax.gamma_multiply(0.85) } else { t.ax });
        icon(ui, icon!("play"), pos2(play.min.x + 22., play.center().y), 16., t.on_ax);
        p.text(pos2(play.min.x + 14. + 16. + 8., play.center().y), Align2::LEFT_CENTER, "Play", font_w(13., Weight::Semi), t.on_ax);
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked() {
            self.pl.play_tracks(ui.ctx(), tracks, 0, false);
        }
        let mut x = play.max.x + 8.;
        for label in labels {
            let w = text_w(&p, label, font(13.)) + 24.;
            let b = Rect::from_min_size(pos2(x, ay), vec2(w, 35.));
            let hot = tonal(ui, &t, label, b, 8., None);
            if hot.clicked() {
                if label == "Export" {
                    self.data.export_spotify(ui.ctx(), title, tracks);
                } else {
                    match save {
                        Some((_, id, _)) if label == "Delete" => self.confirm_delete(ui.ctx(), id),
                        Some((kind, id, saved)) if label != "Shuffle" => self.data.save(ui.ctx(), kind, id, !saved),
                        _ => self.pl.play_tracks(ui.ctx(), tracks, 0, true),
                    }
                }
            }
            p.text(b.center(), Align2::CENTER_CENTER, label, font(13.), if hot.hovered() { t.tx } else { t.tm });
            x += w + 8.;
        }
        if let Some(it) = page_item {
            let more = Rect::from_min_size(pos2(x, ay), vec2(35., 35.));
            if icon_button(ui, &t, "page-more", more.center(), 35., icon!("more"), 18., t.tm).clicked() {
                self.toggle_menu(Some(more), (pos2(more.min.x, more.max.y + 6.), Target::Item(it.clone())));
            }
        }
    }

    /// Song table filling the card below the collapsed hero, in one scroll area: the tall hero's space, the column
    /// header, virtualized rows, then (with `footer`: artist id, own shelves, note) the page's shelves.
    /// Returns the scroll offset, which collapses the hero.
    pub fn tracks_table(&mut self, ui: &mut egui::Ui, card: Rect, tracks: &[Track], footer: Option<(&str, &[Section], &str)>) -> f32 {
        // Songs with their own covers (playlists) get a cover column in place of the number column.
        let covers = self.route == Route::Album && matches!(self.sel.as_ref().map(|s| s.0), Some("playlists" | "spotify"));
        let look = if covers { Look { cover_w: self.cover_px(), open: true, album: true } } else { Look { cover_w: 0., open: false, album: false } };
        let rh = if covers { self.cover_row_h() } else if self.opts.compact { 34. } else { ROW_H };
        let (tl, tr) = (card.min.x + 6., card.max.x - 6.);
        let list = Rect::from_min_max(pos2(card.min.x, card.min.y + HERO_MIN), card.max);
        let mut lui = App::region(ui, list);
        lui.spacing_mut().item_spacing = vec2(0., 0.);
        // Header clicks sort the list: (column, ascending); kept per page so it survives a visit elsewhere.
        let sort_id = ui.scope_id().with(("sort", self.route, &self.sel));
        let mut sort: Option<(usize, bool)> = ui.ctx().data(|d| d.get_temp(sort_id)).flatten();
        // ponytail: re-sorted and cloned every frame while a sort is active; cache by (page, sort) if huge playlists lag.
        let sorted;
        let tracks = match sort {
            Some((col, asc)) => {
                sorted = sort_tracks(tracks, col, asc);
                &sorted[..]
            }
            None => tracks,
        };
        let shown_sort = sort;
        let n = tracks.len();
        let base = HERO_H - HERO_MIN + HEAD;
        let out = ScrollArea::vertical().id_salt(("tracks", self.route, &self.sel)).auto_shrink(false).show_viewport(&mut lui, |ui, vp| {
            ui.add_space(HERO_H - HERO_MIN);
            let (_, head) = ui.allocate_space(vec2(list.width(), HEAD));
            self.table_head(ui, head.min.y + 14., tl, tr, look, Some(&mut sort));
            // Rows are virtualized by hand (not `show_rows`) so the footer can follow the last one.
            let (first, last) = ((((vp.min.y - base).max(0.) / rh) as usize).min(n), ((((vp.max.y - base).max(0.) / rh).ceil() as usize) + 1).min(n));
            if let Some(("spotify", id)) = self.sel.as_ref().map(|s| (s.0, s.1.as_str())) {
                let pad = 16;
                let start = first.saturating_sub(pad);
                let end = (last + pad).min(n);
                self.data.want_spotify(ui.ctx(), id, start..end);
            }
            ui.add_space(first as f32 * rh);
            let acts = self.track_rows(ui, "table", tracks, first..last, (tl, tr), rh, look);
            ui.add_space((n - last) as f32 * rh);
            if let Some((artist_id, own, note)) = footer {
                self.track_footer(ui, (card.min.x + 14., card.width() - 28., list.width()), tracks, artist_id, own, note);
            }
            acts
        });
        scroll_fades(&lui, &out, self.t.s1);
        self.apply_acts(ui, tracks, out.inner, look);
        if sort != shown_sort {
            ui.ctx().data_mut(|d| d.insert_temp(sort_id, sort));
            ui.ctx().request_repaint();
        }
        out.state.clamped_offset().y
    }

    /// "# TITLE ARTIST TIME" with its rule, centred on `hy`. With `sort`, the labels are buttons: a click on a column
    /// cycles ascending, descending, off; "#" turns sorting off.
    pub fn table_head(&self, ui: &egui::Ui, hy: f32, tl: f32, tr: f32, look: Look, mut sort: Option<&mut Option<(usize, bool)>>) {
        let (t, p) = (self.t, ui.painter());
        let cols = columns(tl, tr - tl, look.num(), look.album);
        let head = font_w(11., Weight::Semi);
        let cur = sort.as_deref().copied().flatten();
        for (i, label) in ["#", "TITLE", "ARTIST", "ALBUM", "TIME"].iter().enumerate() {
            let (cx, cw) = cols[i];
            if i == 3 && !look.album {
                continue;
            }
            let hit = sort.is_some().then(|| press(ui, ("head", i), Rect::from_min_size(pos2(cx - 4., hy - 12.), vec2(cw + 8., 24.))));
            let active = cur.filter(|c| c.0 == i);
            let col = if active.is_some() || hit.as_ref().is_some_and(|h| h.hovered()) { t.tx } else { t.tf };
            if i == 0 {
                p.text(pos2(cx + NUM_X, hy), Align2::CENTER_CENTER, label, head.clone(), col);
            } else if i == 4 {
                p.text(pos2(cx + cw, hy), Align2::RIGHT_CENTER, label, head.clone(), col);
            } else {
                p.text(pos2(cx, hy), Align2::LEFT_CENTER, label, head.clone(), col);
            }
            if let Some((_, asc)) = active {
                // Caret beside the label (left of it for the right-aligned TIME).
                let w = text_w(p, label, head.clone());
                let x = if i == 4 { cx + cw - w - 9. } else { cx + w + 9. };
                let d = if asc { -2. } else { 2. };
                p.add(egui::Shape::line(vec![pos2(x - 3.5, hy - d), pos2(x, hy + d), pos2(x + 3.5, hy - d)], (1.5, t.tx)));
            }
            if let (Some(s), Some(h)) = (sort.as_deref_mut(), hit) {
                if h.clicked() {
                    *s = match (i, *s) {
                        (0, _) => None,
                        (_, Some((c, true))) if c == i => Some((i, false)),
                        (_, Some((c, false))) if c == i => None,
                        _ => Some((i, true)),
                    };
                }
            }
        }
        p.line_segment([pos2(tl + 12., hy + 14.), pos2(tr - 12., hy + 14.)], (1., t.s2));
    }

    /// Rows `range` of `tracks`, stacked down `ui` between x `tl..tr`; `sid` keeps two tables on a page apart.
    /// Row height of lists with covers (search songs, playlists), following the density setting.
    pub fn cover_row_h(&self) -> f32 {
        if self.opts.compact { COVER_ROW_COMPACT } else { COVER_ROW_H }
    }

    pub fn cover_px(&self) -> f32 {
        self.cover_row_h() - COVER_PAD
    }

    pub fn track_rows(&mut self, ui: &mut egui::Ui, sid: &'static str, tracks: &[Track], range: std::ops::Range<usize>, (tl, tr): (f32, f32), rh: f32, look: Look) -> Acts {
        let t = self.t;
        let cols = columns(tl, tr - tl, look.num(), look.album);
        let time = ui.ctx().input(|i| i.time) as f32;
        let mut a = Acts::default();
        let mut spinning = false;
        self.data.detail(ui.ctx(), "playlists", "LM"); // starts loading Liked songs so the stars know what is liked
        for ix in range {
            let (_, r) = ui.allocate_space(vec2(tr - tl + 12., rh));
            let row = Rect::from_min_max(pos2(tl, r.min.y), pos2(tr, r.max.y));
            // Even-numbered rows (2nd, 4th, ...) sit on a brighter band. Hover is read off the pointer position, not the
            // row's own response, so the row stays lit while the pointer is over a button or link inside it.
            let hot = ui.rect_contains_pointer(row);
            let band = if hot { t.s2 } else { t.s1.lerp_to_gamma(t.s2, 0.5) };
            let hit = tonal(ui, &t, (sid, "row", ix), row, 8., (hot || ix % 2 == 1).then_some(band));
            if hit.clicked() {
                a.clicked = Some(ix);
            }
            if let (true, Some(pos)) = (hit.secondary_clicked(), hit.interact_pointer_pos()) {
                a.menu = Some((pos, ix));
            }
            let tk = &tracks[ix];
            let pending = tk.resolving;
            let dim = pending || tk.unavailable;
            spinning |= pending;
            let cur = self.pl.is_current(tk);
            let playing = cur && self.pl.playing;
            // The equaliser is the only continuous animation, repainted at ~30fps.
            a.playing |= playing;
            let (cy, p) = (row.center().y, ui.painter());
            if look.cover_w > 0. {
                let mut cover = Rect::from_min_size(pos2(cols[0].0 + 26., cy - (rh - COVER_PAD) / 2.), vec2(look.cover_w, rh - COVER_PAD));
                p.text(pos2(cols[0].0 + NUM_X, cy), Align2::CENTER_CENTER, ix + 1, font(13.), if dim { t.tf } else { t.tm });
                // A wide thumbnail (a video) is shown whole, letterboxed in its slot, instead of cropped to fill it.
                let wide = tk.thumbnail.as_deref().and_then(|u| cover_aspect(ui.ctx(), u, cover.size())).filter(|a| *a > 1.3 && look.cover_w <= rh - COVER_PAD);
                if let Some(a) = wide {
                    cover = Rect::from_center_size(cover.center(), vec2(cover.width(), cover.width() / a));
                }
                self.thumb(ui, tk.thumbnail.as_deref(), cover, 6.);
                if dim {
                    p.rect_filled(cover, rad(6.), Color32::from_black_alpha(150));
                    if pending {
                        spinner(p, cover.center(), (rh - COVER_PAD) * 0.22, time, Color32::WHITE);
                    }
                } else if cur {
                    p.rect_filled(cover, rad(6.), Color32::from_black_alpha(120));
                    equaliser(p, cover.center().x - 5., cy, time, playing, Color32::WHITE);
                } else if hot {
                    p.rect_filled(cover, rad(6.), Color32::from_black_alpha(120));
                    icon(ui, icon!("play"), cover.center(), 18., Color32::WHITE);
                }
                // Rows with their own cover open the song's album; the cover plays it.
                if hot && !dim && press(ui, (sid, "row-play", ix), cover).clicked() {
                    a.play = Some(ix);
                }
            } else if cur {
                equaliser(p, cols[0].0 + NUM_X - 5., cy, time, playing, t.ax);
            } else if pending {
                spinner(p, pos2(cols[0].0 + NUM_X, cy), 7., time, Color32::WHITE);
            } else if hot {
                icon(ui, icon!("play"), pos2(cols[0].0 + NUM_X, cy), 14., t.tx);
            } else {
                p.text(pos2(cols[0].0 + NUM_X, cy), Align2::CENTER_CENTER, ix + 1, font(13.), t.tm);
            }
            let title_col = if dim { t.tf } else if cur { t.ax } else { t.tx };
            trunc(p, pos2(cols[1].0, cy), cols[1].1, &tk.title, if cur && !dim { font_w(14., Weight::Semi) } else { font_w(14., Weight::Medium) }, title_col);
            // The first artist links to its page; the rest of the credit follows in plain text.
            let first_artist = tk.artists.first().map_or("", String::as_str);
            let rest = if tk.artists.len() > 1 { format!(", {}", tk.artists[1..].join(", ")) } else { String::new() };
            let badge = if tk.explicit { explicit_badge(p, pos2(cols[2].0, cy), if hot || ix % 2 == 1 { band } else { t.s1 }, if dim { t.tf } else { t.tm }, 14.) } else { 0. };
            let (ax0, aw) = (cols[2].0 + badge, (cols[2].1 - badge).max(0.));
            let fw = text_w(p, first_artist, font(13.)).min(aw);
            let ar = Rect::from_min_size(pos2(ax0, cy - 9.), vec2(fw, 18.));
            let link = !dim && tk.artist_id.is_some() && !first_artist.is_empty();
            let over = link && ui.rect_contains_pointer(ar);
            let p = ui.painter();
            trunc(p, pos2(ax0, cy), aw, first_artist, font(13.), if over { t.tx } else if dim { t.tf } else { t.tm });
            if over {
                p.line_segment([ar.left_bottom() + vec2(0., -1.), ar.right_bottom() + vec2(0., -1.)], (1., t.tx));
            }
            if fw < aw - 8. {
                trunc(p, pos2(ax0 + fw, cy), aw - fw, &rest, font(13.), if dim { t.tf } else { t.tm });
            }
            if link && press(ui, (sid, "row-artist", ix), ar).clicked() {
                a.artist = tk.artist_id.clone();
            }
            if look.album {
                trunc(ui.painter(), pos2(cols[3].0, cy), cols[3].1, tk.album.as_deref().unwrap_or(""), font(13.), if dim { t.tf } else { t.tm });
            }
            ui.painter().text(pos2(cols[4].0 + cols[4].1, cy), Align2::RIGHT_CENTER, tk.dur(), font(13.), if dim { t.tf } else { t.tm });
            // Fav and more: shown on hover (the fav star stays while liked).
            let liked = tk.source == "youtube" && self.data.is_liked(&tk.id);
            if hot || liked {
                if tk.source == "youtube" {
                    let col = if liked { t.ax } else { t.tm };
                    if icon_button(ui, &t, (sid, "row-fav", ix), pos2(cols[5].0 + 14., cy), 28., if liked { icon!("starf") } else { icon!("star") }, 16., col).clicked() {
                        a.like = Some(ix);
                    }
                }
                let more = pos2(cols[5].0 + 46., cy);
                if icon_button(ui, &t, (sid, "row-more", ix), more, 28., icon!("more"), 16., t.tm).clicked() {
                    a.menu = Some((pos2(more.x - 14., more.y + 16.), ix));
                }
            }
        }
        if spinning {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }
        a
    }

    pub fn apply_acts(&mut self, ui: &egui::Ui, tracks: &[Track], a: Acts, look: Look) {
        if let Some(id) = a.artist {
            return self.open_artist(&id);
        }
        // Rows still resolving or with no YouTube counterpart have nothing to play yet.
        let usable = |ix: usize| !tracks[ix].resolving && !tracks[ix].unavailable;
        match (a.clicked, a.play) {
            (_, Some(ix)) if usable(ix) => self.pl.play_tracks(ui.ctx(), tracks, ix, false),
            (Some(ix), _) if usable(ix) && look.open => self.open_song(tracks[ix].clone()),
            (Some(ix), _) if usable(ix) => self.pl.play_tracks(ui.ctx(), tracks, ix, false),
            _ => {}
        }
        if let Some(ix) = a.like {
            let on = !self.data.is_liked(&tracks[ix].id);
            self.data.like(ui.ctx(), tracks[ix].clone(), on);
        }
        if let Some((pos, ix)) = a.menu {
            self.ctx_menu = Some((pos, Target::Track(tracks[ix].clone())));
        }
        if a.playing {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }

    /// Under the rows: your play count, the page's own shelves (other versions), the artist's other albums, an artist card and similar artists.
    fn track_footer(&mut self, ui: &mut egui::Ui, dims: (f32, f32, f32), tracks: &[Track], artist_id: &str, own: &[Section], note: &str) {
        let t = self.t;
        let (l, w, full) = dims;
        let mut si = 0;
        self.play_count(ui, dims, tracks);
        if !note.is_empty() {
            let (_, r) = ui.allocate_space(vec2(full, 36.));
            trunc(ui.painter(), pos2(l + 8., r.center().y), w - 16., note, font(12.), t.tf);
        }
        for s in own {
            self.carousel(ui, dims, &s.title, s.items.iter(), si, false);
            si += 1;
        }
        if artist_id.is_empty() {
            return ui.add_space(70.);
        }
        let d = self.data.detail(ui.ctx(), "artists", artist_id);
        let ready = d.ready();
        let (mut open, mut sub) = (false, None);
        if let Some(a) = &ready {
            let skip: Vec<&str> = tracks.iter().filter_map(|t| t.album_id.as_deref()).collect();
            let more: Vec<&Item> = a.sections.iter().flat_map(|s| &s.items).filter(|i| i.r#type == "album" && !skip.contains(&i.id.as_str())).collect();
            let by = tracks.first().and_then(|t| t.artists.first()).map_or(a.item.title.as_str(), String::as_str);
            if !more.is_empty() {
                self.carousel(ui, dims, &format!("More by {by}"), more.into_iter(), si, false);
                si += 1;
            }

            // Artist card: round photo, name, audience and the start of the bio; the card opens the artist.
            let (_, hd) = ui.allocate_space(vec2(full, 44.));
            let p = ui.painter().clone();
            trunc(&p, pos2(l + 8., hd.center().y + 6.), w - 16., "About the artist", font_w(17., Weight::Bold), t.tx);
            const PHOTO: f32 = 132.;
            let tw = (w - 16. - PHOTO - 20. - 20. - 16.).max(0.);
            let bio = a.description.as_deref().filter(|_| self.opts.show_bio).map(|b| wrapped(&p, b, font(13.), t.tm, tw, 4));
            let text_h = 24. + 24. + 8. + bio.as_ref().map_or(0., |g| g.size().y + 10.) + 35.;
            let h = text_h.max(PHOTO) + 40.;
            let (_, row) = ui.allocate_space(vec2(full, h + 24.));
            let card = Rect::from_min_size(pos2(l + 8., row.min.y), vec2(w - 16., h));
            if tonal(ui, &t, "about-card", card, 14., Some(t.s2)).clicked() {
                open = true;
            }
            let photo = Rect::from_min_size(card.min + vec2(20., 20.), vec2(PHOTO, PHOTO));
            self.thumb(ui, a.item.thumbnail.as_deref(), photo, PHOTO / 2.);
            let tx = photo.max.x + 20.;
            let mut y = card.min.y + 20.;
            trunc(&p, pos2(tx, y + 12.), tw, &a.item.title, font_w(24., Weight::Bold), t.tx);
            y += 30.;
            if let Some(m) = a.monthly_listeners.as_deref().filter(|_| self.opts.show_monthly) {
                trunc(&p, pos2(tx, y + 8.), tw, m, font(13.), t.tm);
            }
            y += 24.;
            if let Some(g) = bio {
                let gh = g.size().y;
                p.galley(pos2(tx, y), g, t.tm);
                y += gh + 10.;
            }
            if let Some(ch) = a.channel_id.as_deref() {
                let label = if a.subscribed { "Subscribed" } else { "Subscribe" };
                let b = Rect::from_min_size(pos2(tx, y), vec2(text_w(&p, label, font_w(13., Weight::Semi)) + 28., 35.));
                let r = press(ui, "about-sub", b);
                let (fill, ink) = if a.subscribed { (if r.hovered() { Color32::from_gray(225) } else { Color32::WHITE }, Color32::BLACK) } else { (if r.hovered() { t.ax.gamma_multiply(0.9) } else { t.ax }, t.on_ax) };
                p.rect_filled(b, rad(17.5), fill);
                p.text(b.center(), Align2::CENTER_CENTER, label, font_w(13., Weight::Semi), ink);
                if r.clicked() {
                    sub = Some((ch.to_string(), !a.subscribed));
                    open = false;
                }
            }

            let similar: Vec<&Item> = a.sections.iter().flat_map(|s| &s.items).filter(|i| i.r#type == "artist").collect();
            if !similar.is_empty() {
                self.carousel(ui, dims, "Fans also like", similar.into_iter(), si, false);
            }
        }
        ui.add_space(70.);
        if let Some((ch, on)) = sub {
            self.data.subscribe(ui.ctx(), artist_id, &ch, on);
        } else if open {
            self.open_artist(artist_id);
        }
    }

    /// "In your recent history: N plays" for the page's tracks, from the YouTube Music history (loaded on first use).
    fn play_count(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32), tracks: &[Track]) {
        let d = self.data.clone();
        d.history.ensure(ui.ctx(), || Ok(crate::data::Stats::new(crate::backend::get("/youtube/history")?)));
        let g = d.history.lock();
        let State::Ready(st) = &*g else { return };
        let n = st.sections.iter().flat_map(|s| &s.items).filter(|i| i.r#type == "song" && tracks.iter().any(|t| t.id == i.id)).count();
        if n == 0 {
            return;
        }
        let (_, r) = ui.allocate_space(vec2(full, 40.));
        let what = if tracks.len() == 1 { "this song" } else { "songs from this album" };
        let text = format!("You've played {what} {n} time{} recently", if n == 1 { "" } else { "s" });
        trunc(ui.painter(), pos2(l + 8., r.center().y + 8.), w - 16., &text, font(13.), self.t.tm);
    }
}
