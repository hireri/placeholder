//! Player sidebar's info cards below the controls: "About the artist" and "Credits".

use crate::app::App;
use crate::data::{State, Track};
use crate::gfx::*;
use eframe::egui::{self, pos2, vec2, Align2, Id, Rect, Sense};

impl App {
    /// "About the artist" and "Credits" cards starting at `y0`; returns their height (`paint == false` only measures).
    pub fn info_cards(&mut self, ui: &mut egui::Ui, c: Rect, y0: f32, paint: bool) -> f32 {
        let t = self.t;
        let (artist, artist_id, album, album_id) = self.pl.song().map_or((String::new(), None, String::new(), None), |s| (s.artist.clone(), s.artist_id.clone(), s.album.clone(), s.album_id.clone()));
        let (bx, bw) = (c.min.x + 12., c.width() - 24.);
        let p = ui.painter().clone();
        let fill = t.s1.gamma_multiply(0.7);

        // The artist loads once; until it is ready the card shows only the name.
        let detail = artist_id.as_deref().map(|id| self.data.detail(ui.ctx(), "artists", id));
        let guard = detail.as_ref().map(|d| d.lock());
        let info = match guard.as_deref() {
            Some(State::Ready(d)) => Some(d),
            _ => None,
        };
        let name = info.map_or(artist.split(", ").next().unwrap_or(""), |d| d.item.title.as_str());

        // Bio shows four rows until expanded; "Show more" only when the full text is longer than that.
        let expanded_key = Id::unique(("bio", artist_id.as_deref().unwrap_or("")));
        let expanded = ui.ctx().data_mut(|m| *m.get_temp_mut_or(expanded_key, false));
        let (bio, more) = match info.and_then(|d| d.description.as_deref()) {
            Some(text) => {
                let full = wrapped(&p, text, font(12.), t.tm, bw - 32., usize::MAX);
                let more = full.rows.len() > 4;
                let g = if more && !expanded { wrapped(&p, text, font(12.), t.tm, bw - 32., 4) } else { full };
                (Some(g), more)
            }
            None => (None, false),
        };
        // Header (avatar, name, lines) ends at 110 and the bio starts at 124; the actions row and optional error line follow.
        let notice = self.data.notice.lock().unwrap().clone();
        let bio_end = 124. + bio.as_ref().map_or(0., |g| g.size().y) + if more { 24. } else { 0. };
        let actions_y = bio_end + 12.;
        let about_h = actions_y + 35. + if notice.is_some() { 22. } else { 0. } + 16.;

        let mut credits: Vec<(String, &str, Option<(&'static str, String)>)> = vec![];
        for (i, a) in artist.split(", ").filter(|a| !a.is_empty()).enumerate() {
            let target = if i == 0 { artist_id.clone().map(|id| ("artists", id)) } else { None };
            credits.push((a.to_string(), if i == 0 { "Artist" } else { "Featured artist" }, target));
        }
        if !album.is_empty() {
            credits.push((album.clone(), "Album", album_id.clone().map(|id| ("albums", id))));
        }
        let credits_h = 44. + credits.len() as f32 * 52. + 12.;
        let h = about_h + 12. + credits_h + 24.;
        if !paint {
            return h;
        }

        let a = Rect::from_min_size(pos2(bx, y0), vec2(bw, about_h));
        p.rect_filled(a, rad(14.), fill);
        trunc(&p, pos2(a.min.x + 16., a.min.y + 26.), bw - 32., "About the artist", font_w(14., Weight::Bold), t.tx);
        let av = Rect::from_min_size(a.min + vec2(16., 46.), vec2(64., 64.));
        self.thumb(ui, info.and_then(|d| d.item.thumbnail.as_deref()), av, 32.);
        let tx = av.max.x + 12.;
        let tw = a.max.x - 16. - tx;
        trunc(&p, pos2(tx, av.min.y + 14.), tw, name, font_w(15., Weight::Bold), t.tx);
        if let Some(d) = info {
            if let Some(sub) = d.item.subtitle.as_deref() {
                trunc(&p, pos2(tx, av.min.y + 36.), tw, sub, font(12.), t.tm);
            }
            if let Some(m) = d.monthly_listeners.as_deref() {
                trunc(&p, pos2(tx, av.min.y + 54.), tw, m, font(12.), t.tm);
            }
        }
        if let Some(g) = bio {
            p.galley(pos2(a.min.x + 16., a.min.y + 124.), g, t.tm);
        }
        let mut toggle = None;
        if more {
            let label = if expanded { "Show less" } else { "Show more" };
            let r = Rect::from_min_size(pos2(a.min.x + 12., a.min.y + bio_end - 22.), vec2(text_w(&p, label, font(12.)) + 8., 22.));
            let hot = tonal(ui, &t, "bio-toggle", r, 6., None);
            p.text(pos2(r.min.x + 4., r.center().y), Align2::LEFT_CENTER, label, font_w(12., Weight::Semi), if hot.hovered() { t.tx } else { t.tm });
            if hot.clicked() {
                toggle = Some(!expanded);
            }
        }

        // Header: click opens the artist, right-click opens the item menu (copied out so the detail lock is released before acting).
        let (mut artist_click, mut menu) = (false, None);
        if artist_id.is_some() {
            let hit = ui.interact(Rect::from_min_max(av.min, pos2(a.max.x - 16., av.max.y)), ui.scope_id().with("about-artist"), Sense::click());
            if hit.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            artist_click = hit.clicked();
            if hit.secondary_clicked() {
                menu = info.map(|d| (hit.interact_pointer_pos().unwrap_or(av.center()), d.item.clone()));
            }
        }

        // Actions: subscribe (when the channel is known), shuffle (when the page has songs), view artist.
        let subscribed = info.is_some_and(|d| d.subscribed);
        let channel = info.and_then(|d| d.channel_id.clone());
        let song_items = || info.into_iter().flat_map(|d| &d.sections).flat_map(|s| &s.items).filter(|i| i.r#type == "song");
        let songs = song_items().count();
        let ay = a.min.y + actions_y;
        let mut btns: Vec<(&str, &str, bool)> = vec![];
        if channel.is_some() {
            btns.push(if subscribed { ("sub", "Subscribed", false) } else { ("sub", "Subscribe", true) });
        }
        if songs > 0 {
            btns.push(("shuffle", "Shuffle", false));
        }
        if artist_id.is_some() {
            btns.push(("view", "View artist", false));
        }
        let mut clicked = None;
        let mut x = a.min.x + 16.;
        for (key, label, primary) in btns {
            let w = text_w(&p, label, font(13.)) + 24.;
            let r = Rect::from_min_size(pos2(x, ay), vec2(w, 35.));
            let hot = if primary {
                let hot = ui.interact(r, ui.scope_id().with(key), Sense::click());
                if hot.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                p.rect_filled(r, rad(17.5), if hot.is_pointer_button_down_on() { t.ax.gamma_multiply(0.85) } else { t.ax });
                p.text(r.center(), Align2::CENTER_CENTER, label, font(13.), t.on_ax);
                hot
            } else {
                let hot = tonal(ui, &t, key, r, 17.5, Some(t.s2));
                p.text(r.center(), Align2::CENTER_CENTER, label, font(13.), if hot.hovered() { t.tx } else { t.tm });
                hot
            };
            if hot.clicked() {
                clicked = Some(key);
            }
            x += w + 8.;
        }
        let shuffle: Vec<Track> = if clicked == Some("shuffle") { song_items().map(|i| i.track()).collect() } else { vec![] };
        if let Some(e) = &notice {
            trunc(&p, pos2(a.min.x + 16., ay + 41.), bw - 32., e, font(12.), t.tm);
        }

        let k = Rect::from_min_size(pos2(bx, a.max.y + 12.), vec2(bw, credits_h));
        p.rect_filled(k, rad(14.), fill);
        trunc(&p, pos2(k.min.x + 16., k.min.y + 26.), bw - 32., "Credits", font_w(14., Weight::Bold), t.tx);
        let mut credit_click = None;
        for (i, (who, role, target)) in credits.iter().enumerate() {
            let top = k.min.y + 44. + i as f32 * 52.;
            let cy = top + 26.;
            if let Some(tg) = target {
                let r = Rect::from_min_size(pos2(k.min.x + 6., top), vec2(bw - 12., 52.));
                if tonal(ui, &t, ("credit", i), r, 10., None).clicked() {
                    credit_click = Some(tg.clone());
                }
            }
            trunc(&p, pos2(k.min.x + 16., cy - 9.), bw - 32., who, font_w(13.5, Weight::Medium), t.tx);
            trunc(&p, pos2(k.min.x + 16., cy + 10.), bw - 32., role, font(12.), t.tm);
        }

        drop(guard);
        if let Some(v) = toggle {
            ui.ctx().data_mut(|m| *m.get_temp_mut_or(expanded_key, false) = v);
        }
        if let Some((pos, item)) = menu {
            self.ctx_menu = Some((pos, crate::menu::Target::Item(item)));
        }
        if artist_click {
            if let Some(id) = artist_id.as_deref() {
                self.open_artist(id);
            }
        }
        match clicked {
            Some("sub") => {
                if let (Some(ar), Some(ch)) = (artist_id.as_deref(), channel.as_deref()) {
                    self.data.subscribe(ui.ctx(), ar, ch, !subscribed);
                }
            }
            Some("shuffle") => self.pl.play_tracks(ui.ctx(), &shuffle, 0, true),
            Some("view") => {
                if let Some(id) = artist_id.as_deref() {
                    self.open_artist(id);
                }
            }
            _ => {}
        }
        if let Some((kind, id)) = credit_click {
            if kind == "albums" {
                self.open_album(&id);
            } else {
                self.open_artist(&id);
            }
        }
        h
    }
}
