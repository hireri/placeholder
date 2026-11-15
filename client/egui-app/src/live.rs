//! Pages backed by the daemon: home feed, album/artist/playlist detail and the account settings.

use crate::app::{App, Route};
use crate::backend;
use crate::data::{Item, Section, State};
use crate::gfx::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, ScrollArea};

const GAP: f32 = 8.;
const CHIPS: [(&str, &str); 5] = [("All", ""), ("Songs", "song"), ("Albums", "album"), ("Playlists", "playlist"), ("Artists", "artist")];

/// Columns and card width for a card grid across `w` pixels.
fn grid(w: f32) -> (usize, f32) {
    let cols = (((w + GAP) / 168.) as usize).max(1);
    (cols, (w - GAP * (cols - 1) as f32) / cols as f32)
}

impl App {
    pub fn note(&self, ui: &egui::Ui, card: Rect, top: f32, text: &str) {
        trunc(ui.painter(), pos2(card.min.x + 22., top + 20.), card.width() - 44., text, font(13.), self.t.tf);
    }

    /// Thumbnail from a URL (loaded and cached by egui), over a placeholder fill.
    pub fn thumb(&self, ui: &egui::Ui, url: Option<&str>, rect: Rect, r: f32) {
        ui.painter().rect_filled(rect, rad(r), self.t.s3);
        if let Some(u) = url {
            // Ask the CDN for roughly the pixels drawn (64px steps) instead of decoding and uploading 400px for every card.
            let px = ((rect.width() * ui.ctx().pixels_per_point() / 64.).ceil() * 64.).clamp(64., 800.) as u32;
            egui::Image::new(u.replace("w120-h120", &format!("w{px}-h{px}"))).fit_to_exact_size(rect.size()).corner_radius(rad(r)).show_loading_spinner(false).paint_at(ui, rect);
        }
    }

    /// Opens an album/playlist/artist; songs have nowhere to go until playback exists.
    fn open_item(&mut self, it: &Item) {
        let (kind, route) = match it.r#type.as_str() {
            "album" => ("albums", Route::Album),
            "playlist" => ("playlists", Route::Album),
            "artist" => ("artists", Route::Artist),
            "spotify" => ("spotify", Route::Album),
            _ => return,
        };
        self.sel = Some((kind, it.id.clone()));
        self.go(route);
    }

    fn item_card(&mut self, ui: &mut egui::Ui, cell: Rect, it: &Item, key: (usize, usize)) {
        let t = self.t;
        if tonal(ui, &t, ("card", key), cell, 10., None).clicked() {
            self.open_item(it);
        }
        let art = Rect::from_min_size(cell.min + vec2(8., 8.), vec2(cell.width() - 16., cell.width() - 16.));
        self.thumb(ui, it.thumbnail.as_deref(), art, if it.r#type == "artist" { art.width() / 2. } else { 6. });
        let p = ui.painter();
        trunc(p, pos2(art.min.x, art.max.y + 16.), art.width(), &it.title, font_w(13., Weight::Medium), t.tx);
        trunc(p, pos2(art.min.x, art.max.y + 33.), art.width(), it.subtitle.as_deref().unwrap_or(""), font(12.), t.tm);
    }

    /// One titled row of cards (as many as fit).
    fn carousel<'a>(&mut self, ui: &mut egui::Ui, (l, w, full): (f32, f32, f32), title: &str, items: impl Iterator<Item = &'a Item>, si: usize) {
        let t = self.t;
        let (cols, cw) = grid(w);
        let (_, hd) = ui.allocate_space(vec2(full, 44.));
        trunc(ui.painter(), pos2(l + 8., hd.center().y + 6.), w - 16., title, font_w(17., Weight::Bold), t.tx);
        let (_, row) = ui.allocate_space(vec2(full, cw + 8. + 40.));
        for (c, it) in items.take(cols).enumerate() {
            let cell = Rect::from_min_size(pos2(l + c as f32 * (cw + GAP), row.min.y), vec2(cw, cw + 40.));
            self.item_card(ui, cell, it, (si, c));
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
    fn quick_tile(&mut self, ui: &mut egui::Ui, cell: Rect, it: &Item, key: usize) {
        let t = self.t;
        if tonal(ui, &t, ("quick", key), cell, 8., Some(t.s2)).clicked() {
            self.open_item(it);
        }
        let art = Rect::from_min_size(cell.min, vec2(cell.height(), cell.height()));
        self.thumb(ui, it.thumbnail.as_deref(), art, 8.);
        trunc(ui.painter(), pos2(art.max.x + 12., cell.center().y), cell.width() - art.width() - 24., &it.title, font_w(13., Weight::Medium), t.tx);
    }

    /// Large portrait card: cover-filled backdrop under a dark wash, small cover + title on top, "With A, B and more" at the bottom.
    /// `label` sits above the card and says why it is shown (the section title).
    fn feature_card(&mut self, ui: &mut egui::Ui, cell: Rect, label: &str, it: &Item, with: &str, key: usize) {
        let t = self.t;
        let r = ui.interact(cell, ui.scope_id().with(("feat", key)), egui::Sense::click());
        let p = ui.painter().clone();
        trunc(&p, pos2(cell.min.x, cell.min.y - 12.), cell.width(), label, font(12.5), t.tm);
        p.rect_filled(cell, rad(14.), t.s3);
        if let Some(u) = &it.thumbnail {
            let px = ((cell.height() * ui.ctx().pixels_per_point() / 64.).ceil() * 64.).clamp(64., 800.) as u32;
            // ponytail: assumes square source art; crops it to the card's aspect around the centre.
            let a = cell.width() / cell.height();
            let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(a.min(1.), (1. / a).min(1.)));
            egui::Image::new(u.replace("w120-h120", &format!("w{px}-h{px}"))).fit_to_exact_size(cell.size()).uv(uv).corner_radius(rad(14.)).show_loading_spinner(false).paint_at(ui, cell);
        }
        p.rect_filled(cell, rad(14.), Color32::from_black_alpha(110));
        fade_down(&p, Rect::from_min_max(pos2(cell.min.x, cell.center().y), cell.max), Color32::from_black_alpha(200));
        round_bottom_corners(&p, cell, 14., t.s1);
        if r.hovered() {
            p.rect_filled(cell, rad(14.), Color32::from_white_alpha(12));
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
            let g = wrapped(&p, with, font(12.5), t.tm, cell.width() - 32., 2);
            p.galley(pos2(cell.min.x + 16., cell.max.y - 16. - g.size().y), g, t.tm);
        }
        if r.clicked() {
            self.open_item(it);
        }
    }

    pub fn home_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let t = self.t;
        let mut x = card.min.x + 22.;
        for (i, (label, _)) in CHIPS.iter().enumerate() {
            let w = text_w(ui.painter(), label, font(13.)) + 28.;
            let b = Rect::from_min_size(pos2(x, top), vec2(w, 32.));
            let on = self.home_chip == i;
            if tonal(ui, &t, ("chip", i), b, 16., Some(if on { t.tx } else { t.s2 })).clicked() {
                self.home_chip = i;
            }
            ui.painter().text(b.center(), Align2::CENTER_CENTER, label, if on { font_w(13., Weight::Medium) } else { font(13.) }, if on { t.s0 } else { t.tm });
            x += w + GAP;
        }
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
                for (n, it) in s.items.iter().take(cols * 2).enumerate() {
                    let cell = Rect::from_min_size(pos2(l + 8. + (n % cols) as f32 * (cw + GAP), r.min.y + (n / cols) as f32 * (56. + GAP)), vec2(cw, 56.));
                    self.quick_tile(ui, cell, it, n);
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
                    let mut it = s.items.iter().filter(ok);
                    let Some(first) = it.next() else { continue };
                    let names: Vec<&str> = it.take(3).map(|i| i.title.as_str()).collect();
                    let with = if names.is_empty() { String::new() } else { format!("With {} and more", names.join(", ")) };
                    let cell = Rect::from_min_size(pos2(l + 8. + c as f32 * (cw + GAP), r.min.y + 28.), vec2(cw, ch));
                    self.feature_card(ui, cell, &s.title, first, &with, row * 4 + c);
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
                let y = self.hero(ui, card, round, head, &p.item.title, &sub, p.item.thumbnail.as_deref());
                if p.tracks.is_empty() { self.sections(ui, card, y, &p.sections, id) } else { self.tracks_table(ui, card, y, &p.tracks) }
            }
            State::Failed(e) => self.note(ui, card, card.min.y + 40., e),
            _ => self.note(ui, card, card.min.y + 40., "Loading…"),
        }
    }

    pub fn search_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        if self.submitted.is_empty() {
            return;
        }
        let r = self.data.search(ui.ctx(), &self.submitted);
        let g = r.lock();
        match &*g {
            State::Ready(v) if v.is_empty() => self.note(ui, card, top, "No songs found."),
            State::Ready(v) => self.tracks_table(ui, card, top, v),
            State::Failed(e) => self.note(ui, card, top, e),
            _ => self.note(ui, card, top, "Searching…"),
        }
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
            let tx = row.min.x + 68.;
            trunc(&p, pos2(tx, cy - 9.), row.width() - 220., label, font_w(14., Weight::Semi), t.tx);
            let line = match (who, &s.error) {
                (Some(n), _) if s.logged_in => format!("Signed in as {n}"),
                (_, Some(e)) => e.clone(),
                _ => "Not signed in".into(),
            };
            trunc(&p, pos2(tx, cy + 10.), row.width() - 220., &line, font(12.), t.tm);
            let working = busy == Some(svc);
            let text = if working { "Waiting…" } else if s.logged_in { "Sign out" } else { "Sign in" };
            let bw = text_w(&p, text, font_w(13., Weight::Semi)) + 32.;
            let b = Rect::from_center_size(pos2(row.max.x - 16. - bw / 2., cy), vec2(bw, 36.));
            let r = ui.interact(b, ui.scope_id().with(("acct", svc)), egui::Sense::click());
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
