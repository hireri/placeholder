//! Left sidebar: page nav, library header/search/filters and a virtualized item list.

use crate::app::{App, Route};
use crate::gfx::*;
use crate::live::grid;
use crate::data::State;
use crate::menu::{self, Item, Target};
use eframe::egui::{self, pos2, vec2, Align2, Rect, ScrollArea, Sense};

pub struct LibItem {
    pub id: String,
    pub thumb: Option<String>,
    pub title: String,
    pub sub: String,
    pub by: String,
    /// Index into `FILTERS`: 1 playlist, 2 album, 3 artist.
    pub kind: usize,
    pub editable: bool,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LibSort {
    Recents,
    Alpha,
    Creator,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LibView {
    Compact,
    List,
    Grid,
    Grid2,
}

const SORTS: [(LibSort, &str); 3] = [(LibSort::Recents, "Recents"), (LibSort::Alpha, "Alphabetical"), (LibSort::Creator, "Creator")];
const VIEWS: [(LibView, &str, &str); 4] =
    [(LibView::Compact, icon!("vc"), "Compact list"), (LibView::List, icon!("vl"), "List"), (LibView::Grid, icon!("vg"), "Grid of covers"), (LibView::Grid2, icon!("grid"), "Grid with titles")];

const NAV: [(&str, &str, Route); 4] = [
    ("Home", icon!("home"), Route::Home),
    ("Downloads", icon!("imp"), Route::Downloads),
    ("Local Music", icon!("dir"), Route::Local),
    ("History & stats", icon!("hist"), Route::History),
];
const FILTERS: [&str; 4] = ["All", "Playlists", "Albums", "Artists"];

impl LibItem {
    pub fn round(&self) -> bool {
        self.kind == 3
    }

    pub fn route(&self) -> Route {
        if self.round() { Route::Artist } else { Route::Album }
    }

    pub fn from_item(kind: usize, i: crate::data::Item) -> LibItem {
        let label = ["", "Playlist", "Album", "Artist"][kind];
        let by = i.subtitle.clone().unwrap_or_default();
        let sub = if kind == 3 || by.is_empty() { label.to_string() } else { format!("{label} · {by}") };
        LibItem { id: i.id, thumb: i.thumbnail, title: i.title, sub, by, kind, editable: i.editable }
    }

    /// As a generic item, for the right-click menu.
    pub fn item(&self) -> crate::data::Item {
        let r#type = ["", "playlist", "album", "artist"][self.kind].to_string();
        crate::data::Item { r#type, id: self.id.clone(), title: self.title.clone(), thumbnail: self.thumb.clone(), editable: self.editable, ..Default::default() }
    }

    pub fn endpoint(&self) -> &'static str {
        ["", "playlists", "albums", "artists"][self.kind]
    }
}

impl App {
    /// `open_t` is the 0..1 expansion progress; text fades with it and the filter row grows with it.
    pub fn library(&mut self, ui: &mut egui::Ui, rect: Rect, open_t: f32) {
        let t = self.t;
        let mut owned = App::region(ui, rect);
        let ui = &mut owned;
        let p = ui.painter().clone();
        let d = self.data.clone();
        d.load_library(ui.ctx());
        match &*d.library.lock() {
            State::Ready(v) if !self.lib_loaded || self.lib_rev != d.lib_rev.load(std::sync::atomic::Ordering::Relaxed) => {
                self.lib_items = v.iter().map(|(k, i)| LibItem::from_item(*k, i.clone())).collect();
                self.lib_rev = d.lib_rev.load(std::sync::atomic::Ordering::Relaxed);
                self.lib_loaded = true;
            }
            State::Loading | State::Idle => self.lib_loaded = false,
            _ => {}
        }
        let ta = ((open_t - 0.35) / 0.65).clamp(0., 1.); // text opacity
        let (x0, x1) = (rect.min.x + 8., rect.max.x - 8.);
        let mut y = rect.min.y + 12.;

        // Page nav: 4 rows, icon slot fixed 26px from the row's left edge.
        for (i, (label, ic, route)) in NAV.iter().enumerate() {
            let row = Rect::from_min_size(pos2(x0, y), vec2(x1 - x0, 38.));
            let on = self.route == *route;
            let col = if on { t.tx } else { t.tm };
            if tonal(ui, &t, ("nav", i), row, 10., on.then_some(t.axc)).clicked() {
                self.go(*route);
            }
            icon(ui, ic, pos2(row.min.x + 26., row.center().y), 18., col);
            trunc(&p, pos2(row.min.x + 56., row.center().y), 160., label, if on { font_w(13., Weight::Medium) } else { font(13.) }, col.gamma_multiply(ta));
            y += 39.;
        }
        y += 5. + 8.;

        // Header: collapse button keeps its position; title fades.
        let hdr_c = pos2(x0 + 26., y + 16.);
        if icon_button(ui, &t, "lib-toggle", hdr_c, 32., icon!("col"), 16., t.tm).clicked() {
            self.lib_open = !self.lib_open;
        }
        trunc(&p, pos2(x0 + 58., y + 16.), 180., "Your library", font_w(14., Weight::Semi), t.tx.gamma_multiply(ta));
        y += 32. + 8.;

        // Search: real text input when open, plain icon button when collapsed.
        let sr = Rect::from_min_size(pos2(x0, y), vec2(x1 - x0 - 36. * open_t, 32.));
        let search_id = egui::Id::unique("lib-search");
        p.rect_filled(sr, rad(8.), t.s2.gamma_multiply(open_t));
        if !self.lib_open {
            if tonal(ui, &t, "lib-search-btn", sr, 8., None).clicked() {
                self.lib_open = true;
                self.focus_lib_search = true;
            }
        } else if open_t > 0.5 {
            let edit = Rect::from_min_size(pos2(sr.min.x + 48., sr.min.y), vec2((sr.max.x - sr.min.x - 56.).max(0.), 32.));
            if self.focus_lib_search {
                ui.memory_mut(|m| m.request_focus(search_id));
                self.focus_lib_search = false;
            }
            if ui.memory(|m| m.has_focus(search_id)) {
                p.rect_stroke(sr, rad(8.), egui::Stroke::new(2., t.ax), egui::StrokeKind::Inside);
            }
            ui.put(edit, egui::TextEdit::singleline(&mut self.lib_search).id(search_id).frame(egui::Frame::NONE).align(egui::Align2::LEFT_CENTER)
                .font(font(12.5)).hint_text(egui::RichText::new("Search your library").color(t.tm)).desired_width(edit.width()));
        }
        icon(ui, icon!("search"), pos2(sr.min.x + 26., sr.center().y), 18., t.tm);

        // Sort / view button (hidden when collapsed); its icon shows the current view.
        let vb = Rect::from_min_size(pos2(x1 - 32., y), vec2(32., 32.));
        if !self.lib_open {
            self.lib_menu = false;
        } else if open_t > 0.5 {
            let r = ui.interact(vb, ui.scope_id().with("lib-view"), Sense::click());
            p.rect_filled(vb, rad(8.), if r.is_pointer_button_down_on() || r.hovered() { t.s3 } else { t.s2 });
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() {
                self.lib_menu = !self.lib_menu;
            }
            let cur = VIEWS.iter().find(|v| v.0 == self.lib_view).map_or(icon!("vl"), |v| v.1);
            icon(ui, cur, vb.center(), 16., t.tm);
        }
        let segs: Vec<_> = VIEWS.iter().enumerate().map(|(i, (v, ic, tip))| (10 + i, *ic, *tip, *v == self.lib_view)).collect();
        let mut items = vec![Item::Heading("Sort by")];
        items.extend(SORTS.iter().enumerate().map(|(i, (s, l))| Item::Check(i, l, *s == self.lib_sort)));
        items.extend([Item::Heading("View as"), Item::Seg(&segs)]);
        match menu::show(ui.ctx(), &t, "lib-menu", &mut self.lib_menu, pos2(x0, vb.max.y + 6.), vb, &items) {
            Some(i) if i < 10 => self.lib_sort = SORTS[i].0,
            Some(i) => self.lib_view = VIEWS[i - 10].0,
            None => {}
        }
        y += 32. + 8.;

        // Filters: height eases to 0 when collapsed, pulling the list up.
        let fh = 30. * open_t;
        if open_t > 0.02 {
            let mut x = x0 + 6.;
            let fp = p.with_clip_rect(Rect::from_min_size(pos2(rect.min.x, y), vec2(rect.width(), fh)));
            for (i, f) in FILTERS.iter().enumerate() {
                let on = i == self.lib_filter;
                let w = text_w(&p, f, font(12.)) + 20.;
                let r = Rect::from_min_size(pos2(x, y + 2.), vec2(w, 26.));
                let resp = ui.interact(r, ui.scope_id().with(("filter", i)), Sense::click());
                if resp.clicked() {
                    self.lib_filter = i;
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                fp.rect_filled(r, rad(13.), if on { t.tx } else { t.s2 }.gamma_multiply(open_t));
                fp.text(r.center(), Align2::CENTER_CENTER, f, if on { font_w(12., Weight::Medium) } else { font(12.) },
                    if on { t.s0 } else if resp.hovered() { t.tx } else { t.tm }.gamma_multiply(open_t));
                x += w + 4.;
            }
        }
        y += fh + 8. * open_t;

        // Visible items (filter pills + search) in sort order. ponytail: rebuilt per frame, cache if the list gets large.
        let q = self.lib_search.to_lowercase();
        let mut order: Vec<usize> = (0..self.lib_items.len())
            .filter(|&i| {
                let it = &self.lib_items[i];
                (self.lib_filter == 0 || it.kind == self.lib_filter) && (q.is_empty() || it.title.to_lowercase().contains(&q))
            })
            .collect();
        let items = &self.lib_items;
        match self.lib_sort {
            LibSort::Recents => {}
            LibSort::Alpha => order.sort_by_cached_key(|&i| items[i].title.to_lowercase()),
            LibSort::Creator => order.sort_by_cached_key(|&i| (items[i].by.to_lowercase(), items[i].title.to_lowercase())),
        }

        // List: virtualized by row; grids pack several items per row. Collapsed always shows the icon list.
        // View switches fade the list out, swap, then fade it in; reversing mid-fade just reverses the fade.
        let target = if self.lib_open { self.lib_view } else { LibView::List };
        let settled = self.lib_shown == target;
        let va = ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique("lib-view-fade"), settled, 0.15, crate::gfx::ease_out);
        if !settled && va <= 0. {
            self.lib_shown = target;
        }
        let view = self.lib_shown;
        // Grids lay out against the open width so they don't reflow while the sidebar animates.
        let inner = if view == LibView::List || view == LibView::Compact { x1 - x0 } else { self.lib_w - 16. };
        let ((cols, cw), gap) = match view {
            LibView::Grid => (grid(inner - 6., 72., 8.), 8.),
            LibView::Grid2 => (grid(inner, 110., 4.), 4.),
            _ => ((1, inner), 0.),
        };
        let row_h = match view {
            LibView::Compact => 30.,
            LibView::List => 50.,
            LibView::Grid => cw + gap,
            LibView::Grid2 => cw - 16. + 58.,
        };
        let cur = self.lib_current;
        let on_item = matches!(self.route, Route::Album | Route::Artist);
        let mut open_item: Option<(Route, &'static str, String)> = None;
        let mut play_req: Option<crate::data::Item> = None;
        let mut menu_at: Option<(egui::Pos2, Target)> = None;
        let list = Rect::from_min_max(pos2(rect.min.x, y), rect.max);
        let mut lui = App::region(ui, list);
        lui.spacing_mut().item_spacing = vec2(0., 0.);
        lui.set_opacity(va);
        let bar = if self.lib_open { egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded } else { egui::scroll_area::ScrollBarVisibility::AlwaysHidden };
        let out = ScrollArea::vertical().id_salt("lib-list").auto_shrink(false).scroll_bar_visibility(bar)
            .show_rows(&mut lui, row_h, order.len().div_ceil(cols), |ui, range| {
                for r in range {
                    let (_, row) = ui.allocate_space(vec2(list.width(), row_h));
                    for c in 0..cols {
                        let Some(&ix) = order.get(r * cols + c) else { break };
                        let it = &self.lib_items[ix];
                        let on = ix == cur && on_item;
                        let ax = x0 + if view == LibView::Grid { 3. } else { 0. } + c as f32 * (cw + gap);
                        let cell = Rect::from_min_size(pos2(ax, row.min.y), vec2(cw, if view == LibView::Grid { cw } else { row_h }));
                        let hit = tonal(ui, &t, ("li", ix), cell, if view == LibView::Grid { 6. } else { 10. }, (on && view != LibView::Grid).then_some(t.axc));
                        if hit.hovered() {
                            self.data.detail(ui.ctx(), it.endpoint(), &it.id);
                        }
                        if hit.clicked() {
                            self.lib_current = ix;
                            open_item = Some((it.route(), it.endpoint(), it.id.clone()));
                        }
                        if let (true, Some(pos)) = (hit.secondary_clicked(), hit.interact_pointer_pos()) {
                            menu_at = Some((pos, Target::Item(it.item())));
                        }
                        let tp = ui.painter().clone();
                        let badge = match view {
                            LibView::List => Some((pos2(cell.min.x + 26., cell.center().y), 28.)),
                            LibView::Grid => Some((cell.center(), 30.)),
                            LibView::Grid2 => Some((pos2(cell.max.x - 8. - 12. - 16., cell.min.y + 8. + (cw - 16.) - 12. - 16.), 32.)),
                            LibView::Compact => None,
                        };
                        let (tx, ty, tw) = match view {
                            LibView::Compact => (cell.min.x + 10., cell.center().y, cell.width() - 20.),
                            LibView::List => {
                                let art = Rect::from_center_size(pos2(cell.min.x + 26., cell.center().y), vec2(40., 40.));
                                self.thumb(ui, it.thumb.as_deref(), art, if it.round() { 20. } else { 6. });
                                (cell.min.x + 56., cell.center().y, cell.max.x - cell.min.x - 62.)
                            }
                            LibView::Grid => {
                                self.thumb(ui, it.thumb.as_deref(), cell, if it.round() { cw / 2. } else { 6. });
                                if on {
                                    tp.rect_stroke(cell.expand(2.), rad(8.), egui::Stroke::new(2., t.tx), egui::StrokeKind::Outside);
                                }
                                if let Some((at, size)) = badge {
                                    if self.play_badge(ui, cell, at, size, ("lib-play", ix)) {
                                        play_req = Some(it.item());
                                    }
                                }
                                continue;
                            }
                            LibView::Grid2 => {
                                let art = Rect::from_min_size(cell.min + vec2(8., 8.), vec2(cw - 16., cw - 16.));
                                self.thumb(ui, it.thumb.as_deref(), art, if it.round() { (cw - 16.) / 2. } else { 6. });
                                (art.min.x, art.max.y + 8. + 8., cw - 16.)
                            }
                        };
                        if let Some((at, size)) = badge.filter(|_| view != LibView::Grid) {
                            if self.play_badge(ui, cell, at, size, ("lib-play", ix)) {
                                play_req = Some(it.item());
                            }
                        }
                        if view == LibView::Compact {
                            let tr = trunc(&tp, pos2(tx, ty), tw, &it.title, font_w(13., Weight::Medium), t.tx);
                            let sx = tr.max.x + 6.;
                            trunc(&tp, pos2(sx, ty), (tx + tw - sx).max(0.), &format!("· {}", it.sub), font(12.), t.tm);
                        } else {
                            let dy = if view == LibView::List { 8. } else { 9. };
                            let fade = if view == LibView::List { ta } else { 1. };
                            trunc(&tp, pos2(tx, ty - if view == LibView::List { dy } else { 0. }), tw, &it.title, font_w(13., Weight::Medium), t.tx.gamma_multiply(fade));
                            trunc(&tp, pos2(tx, ty + if view == LibView::List { 9. } else { 17. }), tw, &it.sub, font(12.), t.tm.gamma_multiply(fade));
                        }
                    }
                }
            });
        scroll_fades(&lui, &out, t.s0);
        if let Some((r, kind, id)) = open_item {
            self.open(r, Some((kind, id)));
        }
        if let Some(it) = play_req {
            self.play_item(ui.ctx(), &it, &[]);
        }
        if let Some(m) = menu_at {
            self.ctx_menu = Some(m);
        }
    }
}
