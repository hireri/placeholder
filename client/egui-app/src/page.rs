//! Centre panel: album page (header wash, actions, virtualized track list).

use crate::app::{App, Route};
use crate::data::{Item, Track};
use crate::gfx::*;
use crate::menu::Target;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Painter, Rect, ScrollArea};
use std::time::Duration;

const ROW_H: f32 = 44.;
const COVER_ROW_H: f32 = 60.;
const COVER: f32 = 46.;

/// Column x-ranges inside a row of width `w`: # (`num` wide) | title (1.6) | artist (1) | time.
fn columns(left: f32, w: f32, num: f32) -> [(f32, f32); 4] {
    let flex = w - num - 100.;
    let (tw, aw) = (flex * 1.6 / 2.6, flex / 2.6);
    let n = left + 12.;
    let title = n + num + 8.;
    let artist = title + tw + 8.;
    let dur = artist + aw + 8.;
    [(n, num), (title, tw), (artist, aw), (dur, 52.)]
}

/// Three bars with their left edge at `x`, bottoms at `cy + 6`; bouncing while `playing`, otherwise static.
fn equaliser(p: &Painter, x: f32, cy: f32, time: f32, playing: bool, col: Color32) {
    for b in 0..3 {
        let h = if playing { 3. + 9. * (0.5 + 0.5 * (time * 6. + b as f32 * 2.1).sin()) } else { [5., 9., 6.][b] };
        let bx = x + b as f32 * 4.;
        p.rect_filled(Rect::from_min_max(pos2(bx, cy + 6. - h), pos2(bx + 2., cy + 6.)), rad(1.), col);
    }
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
            Route::Song => self.song_page(ui, card),
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
            Route::Home => ("Home", "From YouTube Music"),
            Route::Search if self.submitted.is_empty() => ("Search", "Type in the box above and press Enter"),
            Route::Search => ("Search", "Songs, albums and artists on YouTube Music"),
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
            Route::Search => self.search_page(ui, card, top),
            Route::Import => self.import_page(ui, card, top),
            Route::Downloads => self.note(ui, card, top, "No downloads yet."),
            Route::Local => self.note(ui, card, top, "No local music folders yet."),
            Route::Album => self.note(ui, card, top, "Pick an album from Home, Search or your library."),
            Route::Artist => self.note(ui, card, top, "Pick an artist from Home, Search or your library."),
            Route::Home => self.home_page(ui, card, top),
            Route::Settings => self.settings_page(ui, card, top),
            _ => {}
        }
    }

    /// Wash header with cover (round for artists), title block and action buttons, then the track table.
    /// `save` is (kind, id, saved) when the page can be added to the library.
    pub fn hero(&mut self, ui: &mut egui::Ui, card: Rect, round: bool, kicker: &str, title: &str, sub: &str, cover_url: Option<&str>, tracks: &[Track], save: Option<(&'static str, &str, bool)>) -> f32 {
        let t = self.t;
        let p = ui.painter().clone();

        let wash_h = 24. + 148. + 20.;
        let wash = Rect::from_min_size(card.min, vec2(card.width(), wash_h));
        // Album and artist pages can be right-clicked and opened from the menu; `sel` is only set on those.
        let page_item = match (&self.sel, self.route) {
            (Some((kind, id)), Route::Album | Route::Artist) => Some(Item {
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
        let wash_tex = self.g.wash(ui.ctx(), &t);
        egui::Image::new(sized(wash_tex, 1.)).fit_to_exact_size(wash.size()).corner_radius(rad(18.)).paint_at(ui, wash);
        let art = Rect::from_min_size(pos2(card.min.x + 22., card.min.y + 24.), vec2(148., 148.));
        let ar = if round { 74. } else { 6. };
        shadow(&p, art, ar, 6, 22, 71);
        self.thumb(ui, cover_url, art, ar);

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
        if r.clicked() {
            self.pl.play_tracks(ui.ctx(), tracks, 0, false);
        }
        let mut x = play.max.x + 8.;
        let mut labels = vec!["Shuffle"];
        if let Some((_, _, saved)) = save {
            labels.push(if saved { "Saved" } else { "Save" });
        }
        for label in labels {
            let w = text_w(&p, label, font(13.)) + 24.;
            let b = Rect::from_min_size(pos2(x, ay), vec2(w, 35.));
            let hot = tonal(ui, &t, label, b, 8., None);
            if hot.clicked() {
                match save {
                    Some((kind, id, saved)) if label != "Shuffle" => self.data.save(ui.ctx(), kind, id, !saved),
                    _ => self.pl.play_tracks(ui.ctx(), tracks, 0, true),
                }
            }
            p.text(b.center(), Align2::CENTER_CENTER, label, font(13.), if hot.hovered() { t.tx } else { t.tm });
            x += w + 8.;
        }
        if let Some(it) = &page_item {
            let more = Rect::from_min_size(pos2(x, ay), vec2(35., 35.));
            if icon_button(ui, &t, "page-more", more.center(), 35., icon!("more"), 18., t.tm).clicked() {
                self.toggle_menu(Some(more), (pos2(more.min.x, more.max.y + 6.), Target::Item(it.clone())));
            }
            x += 35. + 8.;
        }
        if let Some(e) = self.data.notice.lock().unwrap().clone() {
            trunc(&p, pos2(x + 4., ay + 17.), (card.max.x - x - 26.).max(0.), &e, font(12.), t.tm);
        }

        ay + 35. + 8.
    }

    /// Table header + virtualized rows from `ty` down to the card bottom.
    pub fn tracks_table(&mut self, ui: &mut egui::Ui, card: Rect, ty: f32, tracks: &[Track]) {
        let t = self.t;
        let p = ui.painter().clone();
        let (tl, tr) = (card.min.x + 6., card.max.x - 6.);
        // Songs with their own covers (search results, playlists) get a cover column in place of the number column.
        let covers = self.route == Route::Search || (self.route == Route::Album && matches!(self.sel.as_ref().map(|s| s.0), Some("playlists" | "spotify")));
        let (rh, num) = if covers { (COVER_ROW_H, 76.) } else { (ROW_H, 36.) };
        let cols = columns(tl, tr - tl, num);
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
        let mut clicked = None;
        let mut menu = None;
        ScrollArea::vertical().id_salt(("tracks", self.route)).auto_shrink(false).show_rows(&mut lui, rh, tracks.len(), |ui, range| {
            for ix in range {
                let (_, r) = ui.allocate_space(vec2(list.width(), rh));
                let row = Rect::from_min_max(pos2(tl, r.min.y), pos2(tr, r.max.y));
                let hit = tonal(ui, &t, ("row", ix), row, 8., None);
                if hit.clicked() {
                    clicked = Some(ix);
                }
                if let (true, Some(pos)) = (hit.secondary_clicked(), hit.interact_pointer_pos()) {
                    menu = Some((pos, ix));
                }
                let tk = &tracks[ix];
                let cur = self.pl.is_current(tk);
                let playing = cur && self.pl.playing;
                // The equaliser is the only continuous animation, repainted at ~30fps.
                any_on |= playing;
                let (cy, p) = (row.center().y, ui.painter());
                if covers {
                    let cover = Rect::from_min_size(pos2(cols[0].0 + 26., cy - COVER / 2.), vec2(COVER, COVER));
                    p.text(pos2(cols[0].0, cy), Align2::LEFT_CENTER, ix + 1, font(13.), t.tm);
                    self.thumb(ui, tk.thumbnail.as_deref(), cover, 6.);
                    if cur {
                        p.rect_filled(cover, rad(6.), Color32::from_black_alpha(120));
                        equaliser(p, cover.center().x - 5., cy, time, playing, Color32::WHITE);
                    } else if hit.hovered() {
                        p.rect_filled(cover, rad(6.), Color32::from_black_alpha(120));
                        icon(ui, icon!("play"), cover.center(), 18., Color32::WHITE);
                    }
                } else if cur {
                    equaliser(p, cols[0].0, cy, time, playing, t.ax);
                } else {
                    p.text(pos2(cols[0].0, cy), Align2::LEFT_CENTER, ix + 1, font(13.), t.tm);
                }
                trunc(p, pos2(cols[1].0, cy), cols[1].1, &tk.title, if cur { font_w(13., Weight::Medium) } else { font(13.) }, if cur { t.ax } else { t.tx });
                trunc(p, pos2(cols[2].0, cy), cols[2].1, &tk.artists.join(", "), font(13.), t.tm);
                p.text(pos2(cols[3].0 + cols[3].1, cy), Align2::RIGHT_CENTER, tk.dur(), font(13.), t.tm);
            }
        });
        if let Some(ix) = clicked {
            self.pl.play_tracks(ui.ctx(), tracks, ix, false);
        }
        if let Some((pos, ix)) = menu {
            self.ctx_menu = Some((pos, Target::Track(tracks[ix].clone())));
        }
        if any_on {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }
}
