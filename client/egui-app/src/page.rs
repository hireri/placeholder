//! Centre panel: album page (header wash, actions, virtualized track list).

use crate::app::{App, Route};
use crate::data::Track;
use crate::gfx::*;
use eframe::egui::{self, pos2, vec2, Align2, Rect, ScrollArea};
use std::time::Duration;

pub fn sample_tracks() -> Vec<Track> {
    let t = |title: &str, artist: &str, dur: u32| Track { title: title.into(), artists: vec![artist.into()], duration_sec: Some(dur), ..Default::default() };
    vec![
        t("Last Stop Is Mine", "Hoshi Yume", 222),
        t("Window Seat, Rain", "Hoshi Yume", 248),
        t("Transfer at Midnight", "Hoshi Yume, Aoi Tsuru", 195),
        t("Fare Card Blue", "Hoshi Yume", 177),
        t("Depot", "Hoshi Yume", 321),
        t("Everyone Gets Off Here, Eventually", "Hoshi Yume", 219),
        t("Terminus", "Hoshi Yume", 284),
        t("Paper Lantern", "Hoshi Yume", 200),
    ]
}

const ROW_H: f32 = 44.;

/// Column x-ranges inside a row of width `w`: # | title (1.6) | artist (1) | time.
fn columns(left: f32, w: f32) -> [(f32, f32); 4] {
    let flex = w - 24. - 36. - 52. - 3. * 8.;
    let (tw, aw) = (flex * 1.6 / 2.6, flex / 2.6);
    let n = left + 12.;
    let title = n + 36. + 8.;
    let artist = title + tw + 8.;
    let dur = artist + aw + 8.;
    [(n, 36.), (title, tw), (artist, aw), (dur, 52.)]
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
            Route::Album => {
                let y = self.hero(ui, card, false, "ALBUM · 2024", "Night Bus Home", "Hoshi Yume · 11 songs · 41 min", None);
                self.tracks_table(ui, card, y, &sample_tracks());
            }
            Route::Artist => {
                let y = self.hero(ui, card, true, "ARTIST", "Hoshi Yume", "184,203 monthly listeners · 3 albums", None);
                self.tracks_table(ui, card, y, &sample_tracks());
            }
            Route::History => self.stats_page(ui, card),
            r => self.plain(ui, card, r),
        }
        fade_down(&p, Rect::from_min_max(pos2(card.min.x, card.max.y - 56.), card.max), t.s1);
        round_bottom_corners(&p, card, 18., t.s0);
    }

    /// Plain header (title + subtitle) over a track table, a card grid, or a placeholder.
    fn plain(&mut self, ui: &mut egui::Ui, card: Rect, r: Route) {
        let t = self.t;
        let p = ui.painter().clone();
        let (title, sub) = match r {
            Route::Home => ("Good evening", "Picked from what you played this week"),
            Route::Search if self.submitted.is_empty() => ("Search", "Type in the box above and press Enter"),
            Route::Search => ("Search", "Songs on YouTube Music"),
            Route::Downloads => ("Downloads", "96 songs · 2.4 GB on this device"),
            Route::Local => ("Local Music", "1,284 files · 3 folders"),
            Route::Settings => ("Settings", ""),
            _ => ("Spotify import", "Your Spotify playlists"),
        };
        trunc(&p, pos2(card.min.x + 22., card.min.y + 38.), card.width() - 44., title, font_w(26., Weight::Bold), t.tx);
        trunc(&p, pos2(card.min.x + 22., card.min.y + 66.), card.width() - 44., sub, font(13.), t.tm);
        let top = card.min.y + 90.;
        match r {
            Route::Search => self.search_page(ui, card, top),
            Route::Import => self.import_page(ui, card, top),
            Route::Downloads | Route::Local => self.tracks_table(ui, card, top, &sample_tracks()),
            Route::Home => self.home_page(ui, card, top),
            Route::Settings => self.settings_page(ui, card, top),
            _ => {
                trunc(&p, pos2(card.min.x + 22., top + 20.), card.width() - 44., "Not designed yet.", font(13.), t.tf);
            }
        }
    }

    /// Wash header with cover (round for artists), title block and action buttons, then the track table.
    pub fn hero(&mut self, ui: &mut egui::Ui, card: Rect, round: bool, kicker: &str, title: &str, sub: &str, cover_url: Option<&str>) -> f32 {
        let t = self.t;
        let p = ui.painter().clone();

        let wash_h = 24. + 148. + 20.;
        let wash = Rect::from_min_size(card.min, vec2(card.width(), wash_h));
        let wash_tex = self.g.wash(ui.ctx(), &t);
        egui::Image::new(sized(wash_tex, 1.)).fit_to_exact_size(wash.size()).corner_radius(rad(18.)).paint_at(ui, wash);
        let art = Rect::from_min_size(pos2(card.min.x + 22., card.min.y + 24.), vec2(148., 148.));
        let ar = if round { 74. } else { 6. };
        shadow(&p, art, ar, 6, 22, 71);
        match cover_url {
            Some(_) => self.thumb(ui, cover_url, art, ar),
            None => {
                let cover = self.g.cover(ui.ctx(), &t);
                egui::Image::new(sized(cover, 148.)).corner_radius(rad(ar)).paint_at(ui, art);
            }
        }

        // Meta block, bottom-aligned with the art.
        let mx = art.max.x + 22.;
        let mw = (card.max.x - 22. - mx).max(0.);
        let mut y = art.max.y;
        trunc(&p, pos2(mx, y - 9.5), mw, sub, font(13.), t.tm);
        y -= 19. + 6.;
        trunc(&p, pos2(mx, y - 15.75), mw, title, font_w(30., Weight::Bold), t.tx);
        y -= 31.5 + 6.;
        trunc(&p, pos2(mx, y - 7.5), mw, kicker, font_w(10.5, Weight::Semi), t.tm);

        // Actions.
        let ay = wash.max.y + 14.;
        let play = Rect::from_min_size(pos2(card.min.x + 22., ay), vec2(14. + 16. + 8. + text_w(&p, "Play", font_w(13., Weight::Semi)) + 18., 35.));
        let r = ui.interact(play, ui.scope_id().with("play"), egui::Sense::click());
        p.rect_filled(play, rad(17.5), if r.is_pointer_button_down_on() { t.ax.gamma_multiply(0.85) } else { t.ax });
        icon(ui, icon!("play"), pos2(play.min.x + 22., play.center().y), 16., t.on_ax);
        p.text(pos2(play.min.x + 14. + 16. + 8., play.center().y), Align2::LEFT_CENTER, "Play", font_w(13., Weight::Semi), t.on_ax);
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let mut x = play.max.x + 8.;
        for label in ["Shuffle", "Save", "Download"] {
            let w = text_w(&p, label, font(13.)) + 24.;
            let b = Rect::from_min_size(pos2(x, ay), vec2(w, 35.));
            let hot = tonal(ui, &t, label, b, 8., None);
            p.text(b.center(), Align2::CENTER_CENTER, label, font(13.), if hot.hovered() { t.tx } else { t.tm });
            x += w + 8.;
        }

        ay + 35. + 8.
    }

    /// Table header + virtualized rows from `ty` down to the card bottom.
    pub fn tracks_table(&mut self, ui: &mut egui::Ui, card: Rect, ty: f32, tracks: &[Track]) {
        let t = self.t;
        let p = ui.painter().clone();
        let (tl, tr) = (card.min.x + 6., card.max.x - 6.);
        let cols = columns(tl, tr - tl);
        let hy = ty + 14.;
        let head = font_w(10.5, Weight::Semi);
        for (i, label) in ["#", "TITLE", "ARTIST", "TIME"].iter().enumerate() {
            let (cx, cw) = cols[i];
            if i == 3 {
                p.text(pos2(cx + cw, hy), Align2::RIGHT_CENTER, label, head.clone(), t.tf);
            } else {
                p.text(pos2(cx, hy), Align2::LEFT_CENTER, label, head.clone(), t.tf);
            }
        }
        let list = Rect::from_min_max(pos2(card.min.x, ty + 28.), card.max);
        let mut lui = App::region(ui, list);
        lui.spacing_mut().item_spacing = vec2(0., 0.);
        let time = ui.ctx().input(|i| i.time) as f32;
        let mut any_on = false;
        ScrollArea::vertical().id_salt(("tracks", self.route)).auto_shrink(false).show_rows(&mut lui, ROW_H, tracks.len(), |ui, range| {
            for ix in range {
                let (_, r) = ui.allocate_space(vec2(list.width(), ROW_H));
                let row = Rect::from_min_max(pos2(tl, r.min.y), pos2(tr, r.max.y));
                tonal(ui, &t, ("row", ix), row, 8., None);
                let tk = &tracks[ix];
                let on = ix == self.playing && self.pl.playing;
                any_on |= on;
                let (cy, p) = (row.center().y, ui.painter());
                if on {
                    // Equaliser bars; the only continuous animation, repainted at ~30fps.
                    for b in 0..3 {
                        let h = 3. + 9. * (0.5 + 0.5 * (time * 6. + b as f32 * 2.1).sin());
                        let bx = cols[0].0 + b as f32 * 4.;
                        p.rect_filled(Rect::from_min_max(pos2(bx, cy + 6. - h), pos2(bx + 2., cy + 6.)), rad(1.), t.ax);
                    }
                } else {
                    p.text(pos2(cols[0].0, cy), Align2::LEFT_CENTER, ix + 1, font(13.), t.tm);
                }
                trunc(p, pos2(cols[1].0, cy), cols[1].1, &tk.title, if on { font_w(13., Weight::Medium) } else { font(13.) }, if on { t.ax } else { t.tx });
                trunc(p, pos2(cols[2].0, cy), cols[2].1, &tk.artists.join(", "), font(13.), t.tm);
                p.text(pos2(cols[3].0 + cols[3].1, cy), Align2::RIGHT_CENTER, tk.dur(), font(13.), t.tm);
            }
        });
        if any_on {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }
}
