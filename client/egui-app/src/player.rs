//! Right sidebar: blurred-cover backdrop, cover/lyrics/queue switcher and transport.
//! Laid out at a fixed 308px from the slot's left edge, so closing just slides it out behind the clip.

use crate::app::{App, PlayerView};
use crate::backend::{self, Lyrics};
use crate::gfx::*;
use eframe::egui::{self, lerp, pos2, vec2, Align2, Color32, CornerRadius, Rect, Sense};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub struct Song {
    id: String,
    title: String,
    artist: String,
    album: String,
    dur: u32,
}

/// SEAM: the daemon has no player/queue API yet (only `/stream/{id}` and `/lyrics`), and the client has no audio
/// engine. Queue, now-playing and "more like this" come from here; replace with daemon data when it exists.
fn sample_queue() -> (Vec<Song>, Vec<Song>) {
    let s = |title: &str, artist: &str, dur| Song { id: String::new(), title: title.into(), artist: artist.into(), album: "Night Bus Home".into(), dur };
    (
        vec![
            s("Window Seat, Rain", "Hoshi Yume", 248),
            s("Transfer at Midnight", "Hoshi Yume, Aoi Tsuru", 195),
            s("Fare Card Blue", "Hoshi Yume", 177),
            s("Depot", "Hoshi Yume", 321),
            s("Everyone Gets Off Here, Eventually", "Hoshi Yume", 219),
        ],
        vec![s("Platform Nine", "Aoi Tsuru", 201), s("Slow Ferry", "Mirrorlake", 233)],
    )
}

enum Lyr {
    Idle,
    Loading(Receiver<Result<Option<Lyrics>, String>>),
    Done(Option<Lyrics>),
    Failed(String),
}

/// Player state. Position is a wall-clock timer (SEAM: no audio engine, so nothing is actually audible).
pub struct Player {
    queue: Vec<Song>,
    more: Vec<Song>,
    cur: usize,
    pub playing: bool,
    pos: f32,
    last: f64,
    shuffle: bool,
    repeat: bool,
    endless: bool,
    following: bool,
    scroll: f32,
    card_scroll: f32,
    lyr: Lyr,
    shown: (PlayerView, f64),
}

impl Default for Player {
    fn default() -> Self {
        let (queue, more) = sample_queue();
        Player { queue, more, cur: 0, playing: false, pos: 0., last: 0., shuffle: false, repeat: false, endless: true, following: false, scroll: 0., card_scroll: 0., lyr: Lyr::Idle, shown: (PlayerView::Art, f64::NEG_INFINITY) }
    }
}

fn clock(s: f32) -> String {
    format!("{}:{:02}", s as u32 / 60, s as u32 % 60)
}

impl Player {
    fn song(&self) -> &Song {
        &self.queue[self.cur]
    }

    fn jump(&mut self, i: usize) {
        self.cur = i;
        self.pos = 0.;
        self.scroll = 0.;
        self.lyr = Lyr::Idle;
    }

    fn next(&mut self) {
        if self.cur + 1 == self.queue.len() && self.endless && !self.more.is_empty() {
            self.queue.push(self.more.remove(0));
        }
        if self.cur + 1 < self.queue.len() {
            self.jump(self.cur + 1);
        } else {
            self.playing = false;
            self.pos = self.song().dur as f32;
        }
    }

    fn prev(&mut self) {
        if self.pos > 3. || self.cur == 0 { self.pos = 0.; } else { self.jump(self.cur - 1); }
    }

    /// Advances the position clock; called every frame (also while the sidebar is collapsed).
    pub fn tick(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.playing {
            self.pos += (now - self.last) as f32;
            if self.pos >= self.song().dur as f32 {
                if self.repeat { self.pos = 0.; } else { self.next(); }
            }
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.last = now;
    }

    fn poll_lyrics(&mut self, ctx: &egui::Context) {
        if let Lyr::Idle = self.lyr {
            let s = self.song();
            let (id, title, artist, dur, album) = (s.id.clone(), s.title.clone(), s.artist.clone(), s.dur, s.album.clone());
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(backend::lyrics(&id, &title, &artist, dur, &album));
                ctx.request_repaint();
            });
            self.lyr = Lyr::Loading(rx);
        }
        if let Lyr::Loading(rx) = &self.lyr {
            match rx.try_recv() {
                Ok(Ok(l)) => self.lyr = Lyr::Done(l),
                Ok(Err(e)) => self.lyr = Lyr::Failed(e),
                Err(_) => {}
            }
        }
    }
}

/// Thin progress/volume bar; grows on hover like the scrubber in the mock. Returns the pointer fraction while pressed.
fn bar(ui: &egui::Ui, key: &str, rect: Rect, track: Color32, fill: Color32, frac: f32, grow: bool) -> Option<f32> {
    let resp = ui.interact(rect.expand2(vec2(0., 6.)), ui.scope_id().with(key), Sense::click_and_drag());
    let h = if grow { 4. + 2. * ui.ctx().animate_bool_with_time(ui.scope_id().with((key, "h")), resp.hovered(), 0.12) } else { 4. };
    let r = Rect::from_center_size(rect.center(), vec2(rect.width(), h));
    ui.painter().rect_filled(r, rad(h / 2.), track);
    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * frac, h)), rad(h / 2.), fill);
    resp.is_pointer_button_down_on().then(|| resp.interact_pointer_pos().map(|p| ((p.x - rect.min.x) / rect.width()).clamp(0., 1.))).flatten()
}

/// SEAM: songs here carry no artist id, so the "About" and "Credits" cards use canned text. The daemon's
/// `/youtube/artists/{id}` already returns a description and subscriber count to replace it with.
const ABOUT: (&str, &str) = ("167,418 monthly listeners", "Writes and produces late-night city pop from a bedroom studio. Mostly songs about trains, rain and the last stop on the line.");

impl App {
    /// "About the artist" and "Credits" cards starting at `y0`; returns their height (`paint == false` only measures).
    fn info_cards(&mut self, ui: &mut egui::Ui, c: Rect, y0: f32, paint: bool) -> f32 {
        let t = self.t;
        let artist = self.pl.song().artist.clone();
        let (bx, bw) = (c.min.x + 12., c.width() - 24.);
        let bio = wrapped(ui.painter(), ABOUT.1, font(12.5), t.tm, bw - 32., 4);
        let about_h = 214. + bio.size().y + 16.;
        let mut credits = vec![];
        for (i, a) in artist.split(", ").enumerate() {
            credits.push((a.to_string(), if i == 0 { "Main Artist · Producer" } else { "Featured Artist" }));
        }
        credits.push(("Mika Sorano".into(), "Lyricist · Composer"));
        let credits_h = 44. + credits.len() as f32 * 52. + 12.;
        let h = about_h + 12. + credits_h + 24.;
        if !paint {
            return h;
        }
        let p = ui.painter().clone();
        let avatar = self.g.art(ui.ctx(), &t, 120.);
        let pl = &mut self.pl;
        let fill = t.s1.gamma_multiply(0.7);

        let a = Rect::from_min_size(pos2(bx, y0), vec2(bw, about_h));
        p.rect_filled(a, rad(14.), fill);
        trunc(&p, pos2(a.min.x + 16., a.min.y + 26.), bw - 32., "About the artist", font_w(14., Weight::Bold), t.tx);
        egui::Image::new(sized(avatar, 80.)).corner_radius(rad(8.)).paint_at(ui, Rect::from_min_size(a.min + vec2(16., 46.), vec2(80., 80.)));
        trunc(&p, pos2(a.min.x + 16., a.min.y + 149.), bw - 32., artist.split(", ").next().unwrap_or(""), font_w(15., Weight::Bold), t.tx);
        let follow = Rect::from_min_size(pos2(a.max.x - 16. - 84., a.min.y + 170. + 2.), vec2(84., 32.));
        trunc(&p, pos2(a.min.x + 16., a.min.y + 188.), bw - 32. - 92., ABOUT.0, font(12.5), t.tm);
        if pill(ui, &t, "follow-about", follow, if pl.following { "Following" } else { "Follow" }).clicked() {
            pl.following = !pl.following;
        }
        p.galley(pos2(a.min.x + 16., a.min.y + 214.), bio, t.tm);

        let k = Rect::from_min_size(pos2(bx, a.max.y + 12.), vec2(bw, credits_h));
        p.rect_filled(k, rad(14.), fill);
        trunc(&p, pos2(k.min.x + 16., k.min.y + 26.), bw - 100., "Credits", font_w(14., Weight::Bold), t.tx);
        p.text(pos2(k.max.x - 16., k.min.y + 26.), Align2::RIGHT_CENTER, "Show all", font_w(12., Weight::Semi), t.tm);
        for (i, (name, role)) in credits.iter().enumerate() {
            let cy = k.min.y + 44. + i as f32 * 52. + 26.;
            let tw = bw - 32. - if i == 0 { 96. } else { 0. };
            trunc(&p, pos2(k.min.x + 16., cy - 9.), tw, name, font_w(13.5, Weight::Medium), t.tx);
            trunc(&p, pos2(k.min.x + 16., cy + 10.), tw, role, font(12.), t.tm);
            if i == 0 && pill(ui, &t, "follow-credit", Rect::from_center_size(pos2(k.max.x - 16. - 42., cy), vec2(84., 32.)), if pl.following { "Following" } else { "Follow" }).clicked() {
                pl.following = !pl.following;
            }
        }
        h
    }

    /// `open` is 0..1 (0 = collapsed to the peek sliver); the card keeps its full size and is just slid by the caller.
    pub fn player(&mut self, ui: &mut egui::Ui, slot: Rect, open: f32) {
        let t = self.t;
        let mut owned = App::region(ui, slot);
        let view = self.view;
        if view == PlayerView::Lyrics {
            self.pl.poll_lyrics(owned.ctx());
        }
        // Rounded card inset from the slot like the page card; fixed width so closing just slides it out behind the clip.
        let c = Rect::from_min_max(pos2(slot.min.x + 10., slot.min.y), pos2(slot.min.x + self.player_w - 10., slot.max.y - 10.));
        let (x0, x1) = (c.min.x + 18., c.max.x - 18.);
        let w = x1 - x0;
        // Content scrolls under the card's rounded corners, so the corners are masked afterwards with whatever lies behind the card.
        let outer = owned.painter().clone();
        let corners = |bg: Color32| {
            round_top_corners(&outer, c, 18., bg);
            round_bottom_corners(&outer, c, 18., bg);
        };
        let behind = t.s0.lerp_to_gamma(t.s1, 1. - open);

        // Backdrop: the whole card is the blurred, dimmed cover.
        let blur = self.g.backdrop(owned.ctx(), &t);
        egui::Image::new(sized(blur, 1.)).fit_to_exact_size(c.size()).corner_radius(rad(18.)).tint(Color32::from_gray(DIM)).paint_at(&owned, c);
        let ui = &mut App::region(&mut owned, c);

        // The whole card scrolls: cover/lyrics/queue slot, then the controls (CTRL_H), then the info cards.
        // `va` morphs the art view (0) into the compact header (1) so the cover and title travel between them.
        const CTRL_H: f32 = 230.;
        let art = view == PlayerView::Art;
        let va = crate::app::anim(ui.ctx(), "pl-view", !art);
        let cards_h = self.info_cards(ui, c, 0., false);
        // Lyrics/queue body: shrinks (down to 240) so the controls stay on screen in short windows, caps at 380 so tall ones show the cards.
        let body_h = (c.height() - 118. - CTRL_H - 6.).clamp(240., 380.);
        let top_h = lerp(c.width()..=118. + body_h, va).max(c.height() - CTRL_H - 6. - cards_h);
        let max_sc = (top_h + CTRL_H + 6. + cards_h - c.height()).max(0.);
        let sc = self.pl.card_scroll.clamp(0., max_sc);
        let body_hit = Rect::from_min_max(pos2(x0, c.min.y - sc + 118.), pos2(x1, c.min.y - sc + top_h));
        let wheel = ui.input(|i| i.pointer.hover_pos().filter(|p| c.contains(*p) && (art || !body_hit.contains(*p))).map_or(0., |_| i.smooth_scroll_delta.y));
        let sc = if open > 0.9 { (sc - wheel).clamp(0., max_sc) } else { sc };
        self.pl.card_scroll = sc;
        let base = c.min.y - sc;
        let yb = base + top_h + CTRL_H;

        // Bottom stack, built upward: device label, mode row, volume, transport, scrubber (14px gaps).
        let mut y = yb - 12.;
        let device_y = y - 7.;
        y -= 14. + 14.;
        let modes_y = y - 21.;
        y -= 42. + 14.;
        let vol_y = y - 8.;
        y -= 16. + 14.;
        let tr_y = y - 26.;
        y -= 52. + 14.;
        let scrub = Rect::from_min_size(pos2(x0, y - 24.), vec2(w, 24.));
        let body_bottom = scrub.min.y - 14.;

        // Cover: full-width at the top of the art view, a 46px thumbnail in the compact header.
        let cover = self.g.cover(ui.ctx(), &t);
        let ident_top = base + 12. + 32. + 14.;
        let big = Rect::from_min_max(pos2(c.min.x, base), pos2(c.max.x, body_bottom));
        let small = Rect::from_min_size(pos2(x0, ident_top), vec2(46., 46.));
        let cr = Rect::from_min_max(big.min.lerp(small.min, va), big.max.lerp(small.max, va));
        let asp = cr.width() / cr.height();
        let uv = Rect::from_center_size(pos2(0.5, 0.5), if asp < 1. { vec2(asp, 1.) } else { vec2(1., 1. / asp) });
        let (top_r, bot_r) = (lerp(18.0..=6., va) as u8, lerp(0.0..=6., va) as u8);
        let shown = lerp(1.0..=open, va);
        egui::Image::new(sized(cover, 1.)).fit_to_exact_size(cr.size()).uv(uv)
            .corner_radius(CornerRadius { nw: top_r, ne: top_r, sw: bot_r, se: bot_r }).tint(Color32::WHITE.gamma_multiply(shown)).paint_at(ui, cr);
        fade_blur(ui.painter(), blur, c, big.min.y + big.height() * 0.5, big.max.y, 1. - va);
        if open < 0.02 {
            corners(behind);
            return;
        }
        ui.set_opacity(open);
        let p = ui.painter().clone();

        let pl = &mut self.pl;
        let (dur, title, artist, album) = (pl.song().dur as f32, pl.song().title.clone(), pl.song().artist.clone(), pl.song().album.clone());

        p.text(pos2(c.center().x, device_y), Align2::CENTER_CENTER, "This computer", font(11.), t.tf);
        let frac = (pl.pos / dur).clamp(0., 1.);
        if let Some(f) = bar(ui, "scrub", Rect::from_min_size(scrub.min, vec2(w, 4.)), t.s3, t.tx, frac, true) {
            pl.pos = f * dur;
        }
        p.text(pos2(x0, scrub.min.y + 17.), Align2::LEFT_CENTER, clock(pl.pos), font(11.), t.tf);
        p.text(pos2(x1, scrub.min.y + 17.), Align2::RIGHT_CENTER, format!("−{}", clock((dur - pl.pos).max(0.))), font(11.), t.tf);

        let tr_x = [x0 + 14. + 26., (x0 + x1) / 2., x1 - 14. - 26.];
        let main_icon = if pl.playing { icon!("pause") } else { icon!("play") };
        for (i, (ic, s)) in [(icon!("rw"), 26.), (main_icon, 34.), (icon!("ff"), 26.)].into_iter().enumerate() {
            if icon_button(ui, &t, ("tr", i), pos2(tr_x[i], tr_y), 52., ic, s, t.tx).clicked() {
                match i {
                    0 => pl.prev(),
                    1 => {
                        pl.playing = !pl.playing;
                        if pl.playing && pl.pos >= dur { pl.pos = 0.; }
                    }
                    _ => pl.next(),
                }
            }
        }

        icon(ui, icon!("vlo"), pos2(x0 + 8., vol_y), 16., t.tm);
        icon(ui, icon!("vhi"), pos2(x1 - 8., vol_y), 16., t.tm);
        if let Some(v) = bar(ui, "vol", Rect::from_min_max(pos2(x0 + 26., vol_y - 2.), pos2(x1 - 26., vol_y + 2.)), t.s3, t.tm, self.volume, false) {
            self.volume = v;
        }

        // Mode row: lyrics | shuffle, repeat, endless pill | queue.
        for (i, (v, ic, cx)) in [(PlayerView::Lyrics, icon!("lyr"), x0 + 22.), (PlayerView::Queue, icon!("que"), x1 - 22.)].into_iter().enumerate() {
            let r = Rect::from_center_size(pos2(cx, modes_y), vec2(44., 38.));
            let on = view == v;
            if tonal(ui, &t, ("mode", i), r, 11., on.then_some(t.s3)).clicked() {
                self.view = if on { PlayerView::Art } else { v };
                pl.scroll = 0.;
            }
            icon(ui, ic, r.center(), 16., if on { t.tx } else { t.tm });
        }
        let pill = Rect::from_min_max(pos2(x0 + 44. + 12., modes_y - 21.), pos2(x1 - 44. - 12., modes_y + 21.));
        let seg_w = pill.width() / 3.;
        p.rect_filled(pill, rad(21.), t.s2);
        let modes = [(icon!("shuf"), &mut pl.shuffle), (icon!("rep"), &mut pl.repeat), (icon!("inf"), &mut pl.endless)];
        for (i, (ic, on)) in modes.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(pill.min.x + i as f32 * seg_w, pill.min.y), vec2(seg_w, pill.height()));
            let resp = ui.interact(r, ui.scope_id().with(("pm", i)), Sense::click());
            let cr = CornerRadius { nw: if i == 0 { 21 } else { 0 }, sw: if i == 0 { 21 } else { 0 }, ne: if i == 2 { 21 } else { 0 }, se: if i == 2 { 21 } else { 0 } };
            if resp.hovered() || resp.is_pointer_button_down_on() {
                p.rect_filled(r, cr, t.s3);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if *on {
                p.rect_filled(r, cr, t.tx.gamma_multiply(0.24));
            }
            if i > 0 {
                p.rect_filled(Rect::from_min_size(r.min, vec2(1., r.height())), 0., t.s0);
            }
            icon(ui, ic, r.center(), 20., if *on || resp.hovered() { t.tx } else { t.tm });
            if resp.clicked() {
                *on = !*on;
            }
        }

        // Header: album name (compact views) + hide button.
        let top = base + 12.;
        if va < 0.99 {
            p.circle_filled(pos2(x1 - 16., top + 16.), 16., Color32::from_black_alpha((90. * (1. - va)) as u8));
        }
        if va > 0.01 {
            trunc(&p, pos2(x0 + 4., top + 16.), w - 44., &album, font_w(13., Weight::Semi), t.tx.gamma_multiply(va));
        }
        let hide = icon_button(ui, &t, "hide", pos2(x1 - 16., top + 16.), 32., icon!("colr"), 16., if va < 0.5 { t.tx } else { t.tm }).clicked();
        // Identity row; in the art view it sits on the cover's lower edge.
        let cy = lerp(body_bottom - 50. + 23.0..=ident_top + 23., va);
        let tx = lerp(x0..=x0 + 56., va);
        let text_w = x1 - 72. - tx;
        trunc(&p, pos2(tx, cy - 9.), text_w, &title, font_w(15., Weight::Bold), t.tx);
        trunc(&p, pos2(tx, cy + 10.), text_w, &artist, font(13.), t.tm);
        icon_button(ui, &t, "fav", pos2(x1 - 52., cy), 32., icon!("star"), 16., t.tm);
        icon_button(ui, &t, "more", pos2(x1 - 16., cy), 32., icon!("more"), 16., t.tm);
        let top = ident_top + 46. + 14.;

        // Body (lyrics / queue) fills what is left.
        // Enter animation on view change (mock: 180ms ease-out, fade + 6px rise); restarts from wherever it is, so it is interruptible.
        let now = ui.input(|i| i.time);
        if self.pl.shown.0 != view {
            self.pl.shown = (view, now);
        }
        let k = ((now - self.pl.shown.1) / 0.18).clamp(0., 1.) as f32;
        let k = 1. - (1. - k) * (1. - k);
        if k < 1. {
            ui.ctx().request_repaint();
        }
        let op = k;
        let body = Rect::from_min_max(pos2(x0, top), pos2(x1, body_bottom)).translate(vec2(0., 6. * (1. - k)));
        let pl = &mut self.pl;
        match view {
            PlayerView::Art => {}
            PlayerView::Lyrics => {
                let mut b = App::region(ui, body);
                b.set_opacity(op);
                let bp = b.painter().clone();
                let foot_h = 28.;
                let area = Rect::from_min_max(body.min, pos2(body.max.x, body.max.y - foot_h - 4.));
                let msg = match &pl.lyr {
                    Lyr::Done(Some(l)) if !l.lines.is_empty() => None,
                    Lyr::Done(_) => Some("No lyrics for this song".to_string()),
                    Lyr::Failed(e) => Some(format!("Lyrics unavailable: {e}")),
                    _ => Some("Loading lyrics…".to_string()),
                };
                if let Some(m) = msg {
                    bp.text(pos2(x0 + 2., area.min.y + 20.), Align2::LEFT_CENTER, m, font(14.), t.tm);
                } else if let Lyr::Done(Some(l)) = &pl.lyr {
                    let cur = if l.synced { l.lines.iter().rposition(|(s, _)| *s <= pl.pos) } else { None };
                    let gs: Vec<_> = l.lines.iter().enumerate().map(|(i, (_, tx))| {
                        bp.layout(tx.clone(), font_w(21., Weight::Bold), if !l.synced || Some(i) == cur { t.tx } else { t.tf }, w - 4.)
                    }).collect();
                    let mut ys = vec![];
                    let mut ly = 10.;
                    for g in &gs {
                        ys.push(ly);
                        ly += g.size().y + 14.;
                    }
                    let max_scroll = (ly - area.height()).max(0.);
                    if l.synced {
                        // Follow the current line, a third of the way down; the animation also keeps repainting while it moves.
                        let want = cur.map_or(0., |i| ys[i] - area.height() * 0.33).clamp(0., max_scroll);
                        pl.scroll = ui.ctx().animate_value_with_time(ui.scope_id().with("lyr-scroll"), want, 0.35);
                    } else if b.rect_contains_pointer(body) {
                        pl.scroll = (pl.scroll - ui.input(|i| i.smooth_scroll_delta.y)).clamp(0., max_scroll);
                    }
                    let lp = bp.with_clip_rect(area);
                    for (g, y) in gs.into_iter().zip(ys) {
                        lp.galley(pos2(x0 + 2., area.min.y + y - pl.scroll), g, Color32::WHITE);
                    }
                    fade_blur(&p, blur, c, area.min.y + area.height() * 0.12, area.min.y, 1.);
                    fade_blur(&p, blur, c, area.min.y + area.height() * 0.8, area.max.y, 1.);
                    p.text(pos2(x0, body.max.y - foot_h / 2.), Align2::LEFT_CENTER, format!("Lyrics by {}", l.provider), font(12.), t.tm);
                    // Stub: no translation endpoint in the daemon yet.
                    let tr = Rect::from_min_size(pos2(x1 - 70., body.max.y - foot_h), vec2(70., foot_h));
                    p.rect_filled(tr, rad(14.), t.s3);
                    p.text(tr.center(), Align2::CENTER_CENTER, "Translate", font_w(11., Weight::Semi), t.tm);
                }
            }
            PlayerView::Queue => {
                let mut b = App::region(ui, body);
                b.set_opacity(op);
                let bp = b.painter().clone();
                bp.text(pos2(x0, body.min.y + 12.), Align2::LEFT_CENTER, "Queue", font_w(20., Weight::Bold), t.tx);
                let list = Rect::from_min_max(pos2(x0, body.min.y + 30.), body.max);
                let upcoming = pl.queue.len() - pl.cur - 1;
                let endless = if pl.endless { pl.more.len() } else { 0 };
                let content = 20. + 52. + 14. + 14. + upcoming as f32 * 54. + if endless > 0 { 26. + endless as f32 * 54. } else { 0. };
                if b.rect_contains_pointer(list) {
                    pl.scroll = (pl.scroll - ui.input(|i| i.smooth_scroll_delta.y)).clamp(0., (content - list.height()).max(0.));
                }
                let lp = bp.with_clip_rect(list);
                let label = |y: f32, s: &str| { lp.text(pos2(x0, y), Align2::LEFT_CENTER, s, font_w(10.5, Weight::Semi), t.tf); };
                let mut qy = list.min.y - pl.scroll;
                label(qy + 8., "NOW PLAYING");
                qy += 20.;
                // `i` is None for rows that are not removable (current song, endless suggestions); `fade` dims suggestions.
                let row = |b: &mut egui::Ui, id: usize, s: &Song, qy: f32, on: bool, grip: bool, fade: bool| -> (bool, bool) {
                    let r = Rect::from_min_size(pos2(x0, qy), vec2(w, 52.));
                    let play = !on && tonal(b, &t, ("q", id), r, 8., None).clicked();
                    let cy = r.center().y;
                    let mut gone = false;
                    if grip {
                        icon(b, icon!("grip"), pos2(r.min.x + 11., cy), 14., t.tf);
                        gone = icon_button(b, &t, ("qx", id), pos2(r.max.x - 16., cy), 24., icon!("x"), 14., t.tf).clicked();
                    }
                    b.set_opacity(op * if fade { 0.65 } else { 1. });
                    egui::Image::new(sized(cover, 38.)).corner_radius(rad(6.)).paint_at(b, Rect::from_center_size(pos2(r.min.x + 14. + 10. + 19., cy), vec2(38., 38.)));
                    let (tx, tw) = (r.min.x + 14. + 10. + 38. + 10., r.width() - 14. - 10. - 38. - 10. - 30.);
                    let lp = b.painter().with_clip_rect(list);
                    trunc(&lp, pos2(tx, cy - 8.), tw, &s.title, font_w(13., Weight::Medium), if on { t.ax } else { t.tx });
                    trunc(&lp, pos2(tx, cy + 9.), tw, &s.artist, font(12.), t.tm);
                    b.set_opacity(op);
                    (play, gone)
                };
                let mut b = App::region(&mut b, list);
                row(&mut b, usize::MAX, pl.song(), qy, true, false, false);
                if view == PlayerView::Queue {
                    // Equaliser bars on the current row.
                    let (ex, ey) = (list.max.x - 12., qy + 26.);
                    let time = ui.input(|i| i.time) as f32;
                    for k in 0..3 {
                        let h = if pl.playing { 4. + 8. * (0.5 + 0.5 * (time * (5. + k as f32 * 1.7)).sin()) } else { 6. };
                        lp.rect_filled(Rect::from_min_max(pos2(ex - 10. + k as f32 * 5., ey + 6.), pos2(ex - 7. + k as f32 * 5., ey + 6. - h)), rad(1.), t.ax);
                    }
                    if pl.playing {
                        ui.ctx().request_repaint_after(Duration::from_millis(60));
                    }
                }
                qy += 52. + 2. + 12.;
                label(qy, &format!("NEXT FROM {}", album.to_uppercase()));
                qy += 14.;
                let (mut play, mut gone) = (None, None);
                for i in pl.cur + 1..pl.queue.len() {
                    let (p_, g_) = row(&mut b, i, &pl.queue[i], qy, false, true, false);
                    if p_ { play = Some(i); }
                    if g_ { gone = Some(i); }
                    qy += 54.;
                }
                if endless > 0 {
                    label(qy + 8., "ENDLESS · MORE LIKE THIS");
                    qy += 26.;
                    for (i, s) in pl.more.iter().enumerate() {
                        row(&mut b, 1000 + i, s, qy, false, false, true);
                        qy += 54.;
                    }
                }
                if let Some(i) = gone {
                    pl.queue.remove(i);
                } else if let Some(i) = play {
                    pl.jump(i);
                    pl.playing = true;
                }
            }
        }
        self.info_cards(ui, c, yb + 6., true);
        corners(behind);
        if hide {
            self.set_player(None);
        }
    }
}
