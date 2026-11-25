//! History & stats: totals, top artists/songs and sources built from the daemon's YouTube Music history and Spotify library, then recent plays.

use crate::app::App;
use crate::backend;
use crate::data::{Item, State, Track};
use crate::gfx::*;
use crate::menu::Target;
use eframe::egui::{self, pos2, vec2, Align2, Rect, ScrollArea};

const GAP: f32 = 8.;
const PANEL_H: f32 = 52. + 5. * 48. + 12.;
const RECENT_PER_DAY: usize = 6;
const RECENT_MAX: usize = 30;

fn duration(secs: u32) -> String {
    let m = secs / 60;
    if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{m}m") }
}

impl App {
    pub fn stats_page(&mut self, ui: &mut egui::Ui, card: Rect) {
        let (t, d) = (self.t, self.data.clone());
        let p = ui.painter().clone();
        trunc(&p, pos2(card.min.x + 22., card.min.y + 38.), card.width() - 44., "History & stats", font_w(26., Weight::Bold), t.tx);
        trunc(&p, pos2(card.min.x + 22., card.min.y + 66.), card.width() - 44., "What you've played on YouTube Music, plus what Spotify shares", font(13.), t.tm);
        let top = card.min.y + 90.;
        d.history.ensure(ui.ctx(), || Ok(crate::data::Stats::new(backend::get("/youtube/history")?)));
        d.load_spotify(ui.ctx());
        let g = d.history.lock();
        let st = match &*g {
            State::Ready(s) => s,
            State::Failed(e) => return self.note(ui, card, top, &format!("{e} — sign in under Settings.")),
            _ => return self.note(ui, card, top, "Loading…"),
        };
        let spotify = match &*d.spotify.lock() {
            State::Ready(n) => format!("{} Spotify playlists · no play history", n.len()),
            State::Failed(e) => e.clone(),
            _ => "Loading…".into(),
        };

        let area = Rect::from_min_max(pos2(card.min.x, top), card.max);
        let mut lui = App::region(ui, area);
        // A row click queues that list from the clicked song, once the scroll area is done with `self`.
        let mut play: Option<(Vec<Track>, usize)> = None;
        let mut menu: Option<(egui::Pos2, Target)> = None;
        let out = ScrollArea::vertical().id_salt("stats").auto_shrink(false).show(&mut lui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0., 0.);
            let p = ui.painter().clone();
            let (l, w) = (card.min.x + 22., card.width() - 44.);
            ui.add_space(8.);

            // Totals.
            let tiles = [("Plays", st.plays.to_string()), ("Songs", st.songs.to_string()), ("Artists", st.artists.to_string()), ("≈ Listening time", duration(st.secs))];
            let cols = if w >= 560. { 4 } else { 2 };
            let tw = (w - GAP * (cols - 1) as f32) / cols as f32;
            let (_, r) = ui.allocate_space(vec2(area.width(), (4 / cols) as f32 * (84. + GAP)));
            for (i, (label, val)) in tiles.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(l + (i % cols) as f32 * (tw + GAP), r.min.y + (i / cols) as f32 * (84. + GAP)), vec2(tw, 84.));
                p.rect_filled(cell, rad(14.), t.s2);
                trunc(&p, pos2(cell.min.x + 16., cell.min.y + 30.), tw - 32., val, font_w(26., Weight::Bold), t.tx);
                trunc(&p, pos2(cell.min.x + 16., cell.min.y + 62.), tw - 32., label, font(12.5), t.tm);
            }

            // Top artists / top songs, side by side when there is room.
            let two = w >= 640.;
            let pw = if two { (w - GAP) / 2. } else { w };
            let (_, r) = ui.allocate_space(vec2(area.width(), if two { PANEL_H } else { 2. * PANEL_H + GAP } + GAP));
            let a = Rect::from_min_size(pos2(l, r.min.y), vec2(pw, PANEL_H));
            let b = Rect::from_min_size(if two { pos2(l + pw + GAP, r.min.y) } else { pos2(l, r.min.y + PANEL_H + GAP) }, vec2(pw, PANEL_H));
            let rows = |panel: Rect, title: &str, empty: bool| {
                p.rect_filled(panel, rad(16.), t.s2);
                trunc(&p, pos2(panel.min.x + 16., panel.min.y + 26.), panel.width() - 32., title, font_w(15., Weight::Bold), t.tx);
                if empty {
                    trunc(&p, pos2(panel.min.x + 16., panel.min.y + 76.), panel.width() - 32., "No plays yet", font(13.), t.tf);
                }
                (0..5).map(move |i| Rect::from_min_size(pos2(panel.min.x, panel.min.y + 52. + i as f32 * 48.), vec2(panel.width(), 48.)))
            };
            let most = st.top_artists.first().map_or(1, |a| a.1) as f32;
            for (i, (row, (name, n))) in rows(a, "Top artists", st.top_artists.is_empty()).zip(&st.top_artists).enumerate() {
                let cy = row.center().y;
                p.text(pos2(row.min.x + 22., cy), Align2::CENTER_CENTER, i + 1, font(13.), t.tf);
                p.circle_filled(pos2(row.min.x + 56., cy), 16., t.axc);
                p.text(pos2(row.min.x + 56., cy), Align2::CENTER_CENTER, name.chars().next().unwrap_or('?').to_uppercase().to_string(), font_w(12., Weight::Semi), t.tx);
                let (tx, tw) = (row.min.x + 80., row.width() - 80. - 80.);
                trunc(&p, pos2(tx, cy - 7.), tw, name, font_w(13., Weight::Medium), t.tx);
                let bar = Rect::from_min_size(pos2(tx, cy + 8.), vec2(tw, 3.));
                p.rect_filled(bar, rad(2.), t.s3);
                p.rect_filled(Rect::from_min_size(bar.min, vec2(tw * *n as f32 / most, 3.)), rad(2.), t.ax);
                p.text(pos2(row.max.x - 16., cy), Align2::RIGHT_CENTER, format!("{n} plays"), font(12.), t.tm);
            }
            let top_list: Vec<Track> = st.top_songs.iter().map(|(i, _)| i.track()).collect();
            for (i, (row, (it, n))) in rows(b, "Top songs", st.top_songs.is_empty()).zip(&st.top_songs).enumerate() {
                let cy = row.center().y;
                let hit = tonal(ui, &t, ("top-song", i), row, 0., None);
                if hit.clicked() {
                    play = Some((top_list.clone(), i));
                }
                if let (true, Some(pos)) = (hit.secondary_clicked(), hit.interact_pointer_pos()) {
                    menu = Some((pos, Target::Track(it.track())));
                }
                p.text(pos2(row.min.x + 22., cy), Align2::CENTER_CENTER, i + 1, font(13.), t.tf);
                self.thumb(ui, it.thumbnail.as_deref(), Rect::from_center_size(pos2(row.min.x + 62., cy), vec2(36., 36.)), 6.);
                let (tx, tw) = (row.min.x + 90., row.width() - 90. - 60.);
                trunc(&p, pos2(tx, cy - 8.), tw, &it.title, font_w(13., Weight::Medium), t.tx);
                trunc(&p, pos2(tx, cy + 9.), tw, it.subtitle.as_deref().unwrap_or(""), font(12.), t.tm);
                p.text(pos2(row.max.x - 16., cy), Align2::RIGHT_CENTER, format!("×{n}"), font(12.), t.tm);
            }

            // Sources.
            let (_, r) = ui.allocate_space(vec2(area.width(), 52. + 2. * 52. + 12. + GAP));
            let panel = Rect::from_min_size(pos2(l, r.min.y), vec2(w, r.height() - GAP));
            p.rect_filled(panel, rad(16.), t.s2);
            trunc(&p, pos2(panel.min.x + 16., panel.min.y + 26.), w - 32., "Sources", font_w(15., Weight::Bold), t.tx);
            let sources = [("YouTube Music", format!("{} plays in your history", st.plays)), ("Spotify", spotify)];
            for (i, (name, line)) in sources.iter().enumerate() {
                let cy = panel.min.y + 52. + i as f32 * 52. + 26.;
                p.circle_filled(pos2(panel.min.x + 36., cy), 18., t.s3);
                p.text(pos2(panel.min.x + 36., cy), Align2::CENTER_CENTER, &name[..1], font_w(13., Weight::Semi), t.tm);
                trunc(&p, pos2(panel.min.x + 68., cy - 9.), w - 84., name, font_w(13., Weight::Medium), t.tx);
                trunc(&p, pos2(panel.min.x + 68., cy + 10.), w - 84., line, font(12.), t.tm);
            }

            // Recent plays, grouped by the day label YouTube Music gives each shelf.
            let (_, hd) = ui.allocate_space(vec2(area.width(), 44.));
            trunc(&p, pos2(l + 8., hd.center().y + 6.), w - 16., "Recently played", font_w(17., Weight::Bold), t.tx);
            let mut left = RECENT_MAX;
            let mut shelves: Vec<(&str, Vec<&Item>)> = vec![];
            for s in st.sections.iter().filter(|s| !s.items.is_empty()) {
                if left == 0 {
                    break;
                }
                let items: Vec<&Item> = s.items.iter().take(RECENT_PER_DAY.min(left)).collect();
                left -= items.len();
                shelves.push((s.title.as_str(), items));
            }
            let recent: Vec<Track> = shelves.iter().flat_map(|(_, items)| items.iter().map(|i| i.track())).collect();
            let mut n = 0;
            for (title, items) in &shelves {
                let (_, h) = ui.allocate_space(vec2(area.width(), 28.));
                trunc(&p, pos2(l + 8., h.center().y + 4.), w - 16., &title.to_uppercase(), font_w(10.5, Weight::Semi), t.tf);
                for it in items {
                    let (_, row) = ui.allocate_space(vec2(area.width(), 44.));
                    let row = Rect::from_min_size(pos2(l, row.min.y), vec2(w, 44.));
                    let hit = tonal(ui, &t, ("recent", row.min.y as i32), row, 8., None);
                    if hit.clicked() {
                        play = Some((recent.clone(), n));
                    }
                    if let (true, Some(pos)) = (hit.secondary_clicked(), hit.interact_pointer_pos()) {
                        menu = Some((pos, Target::Track(it.track())));
                    }
                    n += 1;
                    let cy = row.center().y;
                    self.thumb(ui, it.thumbnail.as_deref(), Rect::from_center_size(pos2(row.min.x + 26., cy), vec2(34., 34.)), 6.);
                    let avail = w - 56. - 64.;
                    trunc(&p, pos2(row.min.x + 52., cy), avail * 0.6, &it.title, font(13.), t.tx);
                    trunc(&p, pos2(row.min.x + 52. + avail * 0.6 + 8., cy), avail * 0.4 - 8., it.subtitle.as_deref().unwrap_or(""), font(13.), t.tm);
                    let dur = it.duration_sec.map_or(String::new(), |s| format!("{}:{:02}", s / 60, s % 60));
                    p.text(pos2(row.max.x - 12., cy), Align2::RIGHT_CENTER, dur, font(13.), t.tm);
                }
            }
            ui.add_space(70.);
        });
        scroll_fades(&lui, &out, self.t.s1);
        if let Some((list, i)) = play {
            self.pl.play_tracks(ui.ctx(), &list, i, false);
        }
        if let Some(m) = menu {
            self.ctx_menu = Some(m);
        }
    }
}
