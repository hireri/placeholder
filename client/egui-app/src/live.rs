//! Pages backed by the daemon: home feed, album/artist/playlist detail and the account settings.

use crate::app::{App, Route};
use crate::backend;
use crate::data::{Item, Section, State, Track};
use crate::gfx::*;
use crate::menu::Target;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, ScrollArea};

const GAP: f32 = 8.;
const CHIPS: [(&str, &str); 5] = [("All", ""), ("Songs", "song"), ("Albums", "album"), ("Playlists", "playlist"), ("Artists", "artist")];
const SEARCH_CHIPS: [&str; 4] = ["All", "Songs", "Albums", "Artists"];
/// Search table row height (page.rs COVER_ROW_H), used to bound the top-song block.
const SONG_ROW: f32 = 60.;
const TOP_W: f32 = 240.;
const TOP_H: f32 = 200.;

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

    /// Opens an album/playlist/artist. A song starts playing: the queue is the song items of `queue` (the shelf it is in), from itself.
    fn open_item(&mut self, ctx: &egui::Context, it: &Item, queue: &[&Item]) {
        if it.r#type == "song" {
            let songs: Vec<Track> = queue.iter().filter(|i| i.r#type == "song").map(|i| i.track()).collect();
            let start = songs.iter().position(|t| t.id == it.id).unwrap_or(0);
            self.pl.play_tracks(ctx, &songs, start, false);
            return;
        }
        let Some((kind, route)) = target(it) else { return };
        self.open(route, Some((kind, it.id.clone())));
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
            self.open_item(ui.ctx(), it, queue);
        }
        self.card_menu(&r, cell, it);
        let art = Rect::from_min_size(cell.min + vec2(8., 8.), vec2(cell.width() - 16., cell.width() - 16.));
        self.thumb(ui, it.thumbnail.as_deref(), art, if it.r#type == "artist" { art.width() / 2. } else { 6. });
        let p = ui.painter();
        trunc(p, pos2(art.min.x, art.max.y + 16.), art.width(), &it.title, font_w(13., Weight::Medium), t.tx);
        trunc(p, pos2(art.min.x, art.max.y + 33.), art.width(), it.subtitle.as_deref().unwrap_or(""), font(12.), t.tm);
    }

    /// One titled row of cards (as many as fit).
    fn carousel<'a>(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32), title: &str, items: impl Iterator<Item = &'a Item>, si: usize) {
        let t = self.t;
        let all: Vec<&Item> = items.collect();
        let (cols, cw) = grid(w, 160., GAP);
        let (_, hd) = ui.allocate_space(vec2(full, 44.));
        trunc(ui.painter(), pos2(l + 8., hd.center().y + 6.), w - 16., title, font_w(17., Weight::Bold), t.tx);
        let (_, row) = ui.allocate_space(vec2(full, cw + 8. + 40.));
        for (c, &it) in all.iter().take(cols).enumerate() {
            let cell = Rect::from_min_size(pos2(l + c as f32 * (cw + GAP), row.min.y), vec2(cw, cw + 40.));
            self.item_card(ui, cell, it, &all, (si, c));
        }
    }

    /// Titled carousels, one per section.
    pub fn sections(&mut self, ui: &mut egui::Ui, card: Rect, top: f32, secs: &[Section], salt: &str) {
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let area = Rect::from_min_max(pos2(card.min.x, top), card.max);
        let mut lui = App::region(ui, area);
        ScrollArea::vertical().id_salt(("sections", salt)).auto_shrink(false).show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            for (si, s) in secs.iter().enumerate() {
                self.carousel(ui, (l, w, area.width()), &s.title, s.items.iter(), si);
            }
            ui.add_space(70.);
        });
    }

    /// Compact quick-pick tile: cover flush left, title beside it.
    fn quick_tile(&mut self, ui: &mut egui::Ui, cell: Rect, it: &Item, queue: &[&Item], key: usize) {
        let t = self.t;
        let r = tonal(ui, &t, ("quick", key), cell, 8., Some(t.s2));
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it, queue);
        }
        self.card_menu(&r, cell, it);
        let art = Rect::from_min_size(cell.min, vec2(cell.height(), cell.height()));
        self.thumb(ui, it.thumbnail.as_deref(), art, 8.);
        trunc(ui.painter(), pos2(art.max.x + 12., cell.center().y), cell.width() - art.width() - 24., &it.title, font_w(13., Weight::Medium), t.tx);
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
            let g = wrapped(&p, &with, font(12.5), t.tm, cell.width() - 32., 2);
            p.galley(pos2(cell.min.x + 16., cell.max.y - 16. - g.size().y), g, t.tm);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it, queue);
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
            if tonal(ui, &t, ("chip", i), b, 16., Some(if on { t.tx } else { t.s2 })).clicked() {
                sel = i;
            }
            ui.painter().text(b.center(), Align2::CENTER_CENTER, label, if on { font_w(13., Weight::Medium) } else { font(13.) }, if on { t.s0 } else { t.tm });
            x += w + GAP;
        }
        sel
    }

    pub fn home_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let labels: Vec<&str> = CHIPS.iter().map(|c| c.0).collect();
        self.home_chip = self.chips(ui, card.min.x + 22., top, &labels, self.home_chip);
        let top = top + 44.;
        let d = self.data.clone();
        d.home.ensure(ui.ctx(), || backend::get("/youtube/home"));
        let g = d.home.lock();
        match &*g {
            State::Ready(s) => self.home_feed(ui, card, top, s),
            State::Failed(e) => self.note(ui, card, top, e),
            _ => self.note(ui, card, top, "Loading…"),
        }
    }

    /// Quick picks from the first shelf, three carousels, then one large card per remaining shelf (shelf title as its label).
    fn home_feed(&mut self, ui: &mut egui::Ui, card: Rect, top: f32, secs: &[Section]) {
        let kind = CHIPS[self.home_chip].1;
        let ok = move |i: &&Item| kind.is_empty() || i.r#type == kind;
        let mut shelves = secs.iter().filter(|s| s.items.iter().any(|i| ok(&i)));
        let quick = if kind.is_empty() { shelves.next() } else { None };
        let (l, w) = (card.min.x + 14., card.width() - 28.);
        let area = Rect::from_min_max(pos2(card.min.x, top), card.max);
        let mut lui = App::region(ui, area);
        ScrollArea::vertical().id_salt(("home", self.home_chip)).auto_shrink(false).show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
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
                self.carousel(ui, (l, w, area.width()), &s.title, s.items.iter().filter(ok), si);
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
    }

    /// Album, playlist or artist selected from a card.
    pub fn live_detail(&mut self, ui: &mut egui::Ui, card: Rect, kind: &'static str, id: &str) {
        let d = self.data.detail(ui.ctx(), kind, id);
        let g = d.lock();
        match &*g {
            State::Ready(p) => {
                let (head, round) = match kind { "artists" => ("ARTIST", true), "playlists" | "spotify" => ("PLAYLIST", false), _ => ("ALBUM", false) };
                let sub = [p.item.subtitle.clone(), (!p.tracks.is_empty()).then(|| format!("{} songs", p.tracks.len()))].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                // Only YouTube albums and playlists can be saved; the saved state is read off the sidebar's library list.
                let save = match kind {
                    "albums" | "playlists" => {
                        let lib = if kind == "albums" { 2 } else { 1 };
                        Some((kind, id, self.lib_items.iter().any(|i| i.kind == lib && i.id == id)))
                    }
                    _ => None,
                };
                let y = self.hero(ui, card, round, head, &p.item.title, &sub, p.item.thumbnail.as_deref(), &p.tracks, save);
                if p.tracks.is_empty() { self.sections(ui, card, y, &p.sections, id) } else { self.tracks_table(ui, card, y, &p.tracks) }
            }
            State::Failed(e) => self.note(ui, card, card.min.y + 40., e),
            _ => self.note(ui, card, card.min.y + 40., "Loading…"),
        }
    }

    /// Type chips, then the top result beside the first songs (wide cards) and album and artist shelves.
    /// Only All shows the top result and five songs; the other chips show one kind in full.
    pub fn search_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        if self.submitted.is_empty() {
            return;
        }
        let (d, q) = (self.data.clone(), self.submitted.clone());
        let r = d.search(ui.ctx(), &q);
        let (albums, artists) = (d.search_items(ui.ctx(), &q, "album"), d.search_items(ui.ctx(), &q, "artist"));
        let (albums_v, artists_v) = (
            match &*albums.lock() {
                State::Ready(v) => v.clone(),
                _ => vec![],
            },
            match &*artists.lock() {
                State::Ready(v) => v.clone(),
                _ => vec![],
            },
        );

        let id = ui.scope_id().with(("search-chip", &q));
        let cur = ui.ctx().data(|m| m.get_temp::<usize>(id)).unwrap_or(0);
        let chip = self.chips(ui, card.min.x + 22., top, &SEARCH_CHIPS, cur);
        if chip != cur {
            ui.ctx().data_mut(|m| m.insert_temp(id, chip));
        }
        let all = chip == 0;
        let (show_songs, show_albums, show_artists) = (chip <= 1, all || chip == 2, all || chip == 3);

        let by = top + 44.;
        let (l, w) = (card.min.x + 22., card.width() - 44.);
        let top_item = if all { albums_v.first().or(artists_v.first()) } else { None };
        let side = top_item.is_some() && card.width() >= 640.;
        let (sx, sw) = if side { (l + TOP_W + 20., w - TOP_W - 20.) } else { (l, w) };
        let sy = if top_item.is_some() && !side { by + TOP_H + 12. } else { by };

        let mut block = 0.;
        let g = r.lock();
        if show_songs {
            let songs: &[Track] = match &*g {
                State::Ready(v) => v,
                _ => &[],
            };
            let msg = match &*g {
                State::Ready(v) if v.is_empty() => Some("No songs found.".to_string()),
                State::Ready(_) => None,
                State::Failed(e) => Some(e.clone()),
                _ => Some("Searching…".to_string()),
            };
            match msg {
                Some(m) => {
                    trunc(ui.painter(), pos2(sx, sy + 20.), sw, &m, font(13.), self.t.tf);
                    block = 40.;
                }
                None => {
                    let n = if all { songs.len().min(5) } else { songs.len() };
                    block = 28. + n as f32 * SONG_ROW;
                    let table = Rect::from_min_max(pos2(sx - 6., sy), pos2(sx + sw + 6., sy + block));
                    self.tracks_table(ui, table, sy, &songs[..n]);
                }
            }
        }
        drop(g);

        let end = if side { by + block.max(TOP_H) } else { sy + block };
        if let Some(it) = top_item {
            let h = if side { block.max(TOP_H) } else { TOP_H };
            self.top_card(ui, Rect::from_min_size(pos2(l, by), vec2(if side { TOP_W } else { w }, h)), it);
        }

        let mut secs = vec![];
        for (title, v, on) in [("Albums", albums_v, show_albums), ("Artists", artists_v, show_artists)] {
            if on && !v.is_empty() {
                secs.push(Section { title: title.into(), items: v });
            }
        }
        // Each carousel is a title row plus one row of covers.
        let shelf_h: f32 = secs.iter().map(|_| 44. + grid(card.width() - 28., 160., GAP).1 + 48.).sum();
        let y = end + 16.;
        if !secs.is_empty() {
            self.sections(ui, Rect::from_min_max(card.min, pos2(card.max.x, y + shelf_h)), y, &secs, "search");
        } else if !show_songs {
            self.note(ui, card, y, "No results.");
        }
    }

    /// Mockup "Top result": cover, kind and title; the card opens it.
    fn top_card(&mut self, ui: &mut egui::Ui, rect: Rect, it: &Item) {
        let t = self.t;
        let r = tonal(ui, &t, "top-result", rect, 14., Some(t.s2));
        if r.hovered() {
            self.prefetch(ui.ctx(), it);
        }
        if r.clicked() {
            self.open_item(ui.ctx(), it, &[]);
        }
        self.card_menu(&r, rect, it);
        let art = Rect::from_min_size(rect.min + vec2(18., 40.), vec2(96., 96.));
        let p = ui.painter().clone();
        shadow(&p, art, 6., 6, 18, 66);
        self.thumb(ui, it.thumbnail.as_deref(), art, if it.r#type == "artist" { 48. } else { 6. });
        let tw = rect.width() - 36.;
        trunc(&p, pos2(rect.min.x + 18., rect.min.y + 20.), tw, "TOP RESULT", font_w(10.5, Weight::Semi), t.tm);
        trunc(&p, pos2(rect.min.x + 18., art.max.y + 26.), tw, &it.title, font_w(24., Weight::Bold), t.tx);
        let kind = match it.r#type.as_str() { "album" => "Album", "playlist" => "Playlist", "artist" => "Artist", _ => "Song" };
        let sub = [Some(kind), it.subtitle.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        trunc(&p, pos2(rect.min.x + 18., art.max.y + 50.), tw, &sub, font(13.), t.tm);
    }

    pub fn import_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let d = self.data.clone();
        d.load_spotify(ui.ctx());
        let g = d.spotify.lock();
        match &*g {
            State::Ready(v) if v.is_empty() => self.note(ui, card, top, "No Spotify playlists."),
            State::Ready(v) => self.sections(ui, card, top, &[Section { title: "Playlists".into(), items: v.clone() }], "import"),
            State::Failed(e) => self.note(ui, card, top, &format!("{e} — sign in to Spotify under Settings.")),
            _ => self.note(ui, card, top, "Loading…"),
        }
    }

    pub fn settings_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
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
        trunc(ui.painter(), pos2(card.min.x + 22., top + 12.), 300., "ACCOUNTS", font_w(10.5, Weight::Semi), t.tf);
        for (i, (svc, label, s)) in [("youtube", "YouTube Music", &st.youtube), ("spotify", "Spotify", &st.spotify)].into_iter().enumerate() {
            let row = Rect::from_min_size(pos2(card.min.x + 22., top + 28. + i as f32 * 84.), vec2(card.width() - 44., 72.));
            let p = ui.painter().clone();
            p.rect_filled(row, rad(14.), t.s2);
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
                p.rect_filled(ib, rad(18.), if ir.hovered() { t.s3 } else { t.s1 });
                p.text(ib.center(), Align2::CENTER_CENTER, "Import playlists", font_w(13., Weight::Semi), t.tx);
                if ir.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if ir.clicked() {
                    self.go(Route::Import);
                }
            }
            let primary = !s.logged_in;
            p.rect_filled(b, rad(18.), if primary { if r.hovered() { t.ax.gamma_multiply(0.9) } else { t.ax } } else if r.hovered() { t.s3 } else { t.s1 });
            p.text(b.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), if primary { t.on_ax } else { t.tx });
            if r.hovered() && !working {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() && busy.is_none() {
                d.account(ui.ctx(), svc, !s.logged_in);
            }
        }
        if let Some(e) = err {
            self.note(ui, card, top + 28. + 2. * 84., &e);
        }
    }
}

impl App {
    /// Song route: the song's hero, then its album's tracks (or the artist's shelves when it has no album).
    pub fn song_page(&mut self, ui: &mut egui::Ui, card: Rect) {
        let Some(track) = self.song.clone() else {
            return self.note(ui, card, card.min.y + 40., "Pick a song");
        };
        let sub = [Some(track.artists.join(", ")), track.album.clone(), Some(track.dur())].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let y = self.hero(ui, card, false, "SONG", &track.title, &sub, track.thumbnail.as_deref(), std::slice::from_ref(&track), None);
        if let Some(id) = &track.album_id {
            let art = Rect::from_min_size(pos2(card.min.x + 22., card.min.y + 24.), vec2(148., 148.));
            let r = ui.interact(art, ui.scope_id().with("song-cover"), egui::Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(art, rad(6.), Color32::from_black_alpha(110));
                icon(ui, icon!("alb"), art.center(), 28., Color32::WHITE);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() {
                self.open_album(id);
            }
            if r.secondary_clicked() {
                self.ctx_menu = Some((r.interact_pointer_pos().unwrap_or(art.center()), Target::Track(track.clone())));
            }
            let d = self.data.detail(ui.ctx(), "albums", id);
            let g = d.lock();
            match &*g {
                State::Ready(p) => self.tracks_table(ui, card, y, &p.tracks),
                State::Failed(e) => self.note(ui, card, y, e),
                _ => self.note(ui, card, y, "Loading…"),
            }
        } else if let Some(id) = &track.artist_id {
            let d = self.data.detail(ui.ctx(), "artists", id);
            let g = d.lock();
            match &*g {
                State::Ready(p) => self.sections(ui, card, y, &p.sections, "song-artist"),
                State::Failed(e) => self.note(ui, card, y, e),
                _ => self.note(ui, card, y, "Loading…"),
            }
        }
    }
}
