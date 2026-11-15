//! Shell: titlebar, collapsible library (left), page (centre), collapsible player (right), mini pill.
//!
//! egui is immediate mode, so the "layout" is plain rect arithmetic: the two sidebar widths come from
//! `animate_bool` (0..1) and every region is drawn into its own clipped child `Ui`. Contents of a
//! collapsing sidebar keep their size and are simply clipped, as in the design mock.

use crate::gfx::*;
use crate::library::{LibItem, LibSort, LibView};
use crate::data::{Data, State};
use std::sync::Arc;
use crate::player::Player;
use crate::theme::Theme;
use eframe::egui::{self, lerp, Color32, pos2, vec2, Context, Id, Key, Modifiers, Rect, Ui, UiBuilder};

pub const TB_H: f32 = 52.;
pub const LIB_OPEN: f32 = 264.;
pub const LIB_MIN: f32 = 200.;
pub const LIB_MAX: f32 = 360.;
pub const LIB_SHUT: f32 = 68.;
pub const PLAYER_W: f32 = 340.;
const PLAYER_MIN: f32 = 270.;
const PLAYER_MAX: f32 = 410.;
const MAIN_MIN: f32 = 380.;
const PEEK_W: f32 = 56.;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Home,
    Search,
    Album,
    Artist,
    Downloads,
    Local,
    History,
    Settings,
    Import,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PlayerView {
    Art,
    Lyrics,
    Queue,
}

pub struct App {
    pub t: Theme,
    pub g: Gfx,
    pub lib_open: bool,
    pub player_open: bool,
    pub player_w: f32,
    pub view: PlayerView,
    pub route: Route,
    pub back: Vec<Route>,
    pub fwd: Vec<Route>,
    pub lib_sort: LibSort,
    pub lib_view: LibView,
    /// Open width of the library and the view currently drawn (lags `lib_view` during the cross-fade).
    pub lib_w: f32,
    pub lib_shown: LibView,
    pub lib_menu: bool,
    pub lib_filter: usize,
    pub lib_current: usize,
    pub lib_search: String,
    pub focus_lib_search: bool,
    pub global_search: String,
    pub lib_items: Vec<LibItem>,
    pub lib_loaded: bool,
    /// Query the Search page shows (`global_search` is the text box, committed on Enter).
    pub submitted: String,
    pub data: Arc<Data>,
    /// Album/artist/playlist shown by the Album and Artist routes: (endpoint segment, browse id).
    pub sel: Option<(&'static str, String)>,
    pub home_chip: usize,
    pub playing: usize,
    pub pl: Player,
    pub volume: f32,
    pub vol_open: bool,
    started: bool,
}

fn quint_out(t: f32) -> f32 {
    1. - (1. - t).powi(5)
}

/// 0..1 progress that eases toward `on`, reversing smoothly if toggled mid-flight.
pub fn anim(ctx: &Context, id: &str, on: bool) -> f32 {
    ctx.animate_bool_with_time_and_easing(Id::unique(id), on, 0.32, quint_out)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install(&cc.egui_ctx);
        let t = Theme::new(265., 0.14, 0.014, true);
        let mut v = egui::Visuals::dark();
        v.override_text_color = Some(t.tx);
        v.selection.bg_fill = t.ax.gamma_multiply(0.35);
        v.selection.stroke = egui::Stroke::NONE;
        v.text_cursor.stroke.color = t.ax;
        v.window_fill = t.s0;
        v.panel_fill = t.s0;
        cc.egui_ctx.set_visuals(v);
        App {
            t,
            g: Gfx::new(),
            lib_open: true,
            player_open: true,
            player_w: PLAYER_W,
            view: PlayerView::Art,
            route: Route::Album,
            back: Vec::new(),
            fwd: Vec::new(),
            lib_sort: LibSort::Recents,
            lib_view: LibView::List,
            lib_w: LIB_OPEN,
            lib_shown: LibView::List,
            lib_menu: false,
            lib_filter: 0,
            lib_current: usize::MAX,
            lib_search: String::new(),
            focus_lib_search: false,
            global_search: String::new(),
            lib_items: Vec::new(),
            lib_loaded: false,
            submitted: String::new(),
            data: Arc::new(Data::new()),
            sel: None,
            home_chip: 0,
            playing: 1,
            pl: Player::default(),
            volume: 0.7,
            vol_open: false,
            started: false,
        }
    }

    /// A clipped child `Ui` covering `rect`.
    pub fn region(ui: &mut Ui, rect: Rect) -> Ui {
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child
    }

    fn titlebar(&mut self, ui: &mut Ui, tb: Rect, lw: f32) {
        let t = self.t;
        let mut owned = App::region(ui, tb);
        let ui = &mut owned;
        let p = ui.painter().clone();
        let cy = tb.center().y;

        // Brand sits over the library and is clipped by its width.
        let bp = p.with_clip_rect(Rect::from_min_max(tb.min, pos2(tb.min.x + lw, tb.max.y)));
        bp.rect_filled(Rect::from_center_size(pos2(tb.min.x + 34., cy), vec2(16., 16.)), rad(5.), t.ax);
        trunc(&bp, pos2(tb.min.x + 54., cy), 200., "Metrolist", font_w(14., Weight::Semi), t.tx);

        // Right cluster, laid out from the right edge.
        let mut x = tb.max.x - 12.;
        self.data.status.ensure(ui.ctx(), || crate::backend::get("/auth/status"));
        let name = match &*self.data.status.lock() {
            State::Ready(s) => s.youtube.user.as_ref().map(|u| u.name.clone()),
            _ => None,
        }
        .unwrap_or_else(|| "Sign in".into());
        let initial = name.chars().next().unwrap_or('?').to_uppercase().to_string();
        let me_w = 3. + 26. + 8. + text_w(&p, &name, font(13.)) + 10.;
        let me = Rect::from_min_size(pos2(x - me_w, cy - 16.), vec2(me_w, 32.));
        if tonal(ui, &t, "me", me, 16., None).clicked() {
            self.go(Route::Settings);
        }
        p.circle_filled(pos2(me.min.x + 16., cy), 13., t.axc);
        p.text(pos2(me.min.x + 16., cy), egui::Align2::CENTER_CENTER, initial, font_w(11., Weight::Semi), t.tx);
        trunc(&p, pos2(me.min.x + 37., cy), 100., &name, font(13.), t.tx);
        x = me.min.x - 6.;
        icon_button(ui, &t, "bell", pos2(x - 16., cy), 32., icon!("bell"), 16., t.tm);
        x -= 38.;
        let imp_txt = "Importing 412 / 640";
        let imp_w = 10. + 16. + 8. + text_w(&p, imp_txt, font(12.)) + 8. + 44. + 10.;
        let imp = Rect::from_min_size(pos2(x - imp_w, cy - 14.), vec2(imp_w, 28.));
        p.rect_filled(imp, rad(14.), t.s1);
        if tonal(ui, &t, "imp-chip", imp, 14., Some(t.s1)).clicked() {
            self.go(Route::Import);
        }
        icon(ui, icon!("imp"), pos2(imp.min.x + 18., cy), 16., t.tm);
        let tx_r = trunc(&p, pos2(imp.min.x + 34., cy), 200., imp_txt, font(12.), t.tm);
        let bar = Rect::from_min_size(pos2(tx_r.max.x + 8., cy - 1.5), vec2(44., 3.));
        p.rect_filled(bar, rad(2.), t.s3);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(44. * 0.64, 3.)), rad(2.), t.ax);

        // Middle: back/forward + global search, centred in the space between brand and right cluster.
        let (lo, hi) = (tb.min.x + lw, imp.min.x - 12.);
        let w = (hi - lo).min(560.).max(0.);
        let mx = (lo + hi) / 2. - w / 2.;
        if icon_button(ui, &t, "back", pos2(mx + 16., cy), 32., icon!("back"), 16., if self.back.is_empty() { t.tf } else { t.tm }).clicked() {
            self.back();
        }
        if icon_button(ui, &t, "fwd", pos2(mx + 54., cy), 32., icon!("fwd"), 16., if self.fwd.is_empty() { t.tf } else { t.tm }).clicked() {
            self.forward();
        }
        let sr = Rect::from_min_max(pos2(mx + 70., cy - 17.), pos2(mx + w, cy + 17.));
        let id = Id::unique("global-search");
        let focused = ui.memory(|m| m.has_focus(id));
        p.rect_filled(sr, rad(17.), if focused { t.s3 } else { t.s2 });
        if focused {
            p.rect_stroke(sr, rad(17.), egui::Stroke::new(2., t.ax), egui::StrokeKind::Inside);
        }
        icon(ui, icon!("search"), pos2(sr.min.x + 22., cy), 16., t.tm);
        let edit = Rect::from_min_max(pos2(sr.min.x + 38., sr.min.y), pos2(sr.max.x - 62., sr.max.y));
        let te = ui.put(edit, egui::TextEdit::singleline(&mut self.global_search).id(id).frame(egui::Frame::NONE).align(egui::Align2::LEFT_CENTER)
            .hint_text(egui::RichText::new("Search songs, albums, artists").color(t.tm)).desired_width(edit.width()));
        if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.global_search.trim().is_empty() {
            self.submitted = self.global_search.trim().to_string();
            self.go(Route::Search);
        }
        if !focused {
            p.text(pos2(sr.max.x - 14., cy), egui::Align2::RIGHT_CENTER, "Ctrl K", font(10.5), t.tf);
        }
        if ui.ctx().input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            ui.memory_mut(|m| m.request_focus(id));
        }
    }

    /// Floating pill, centred on the window, shown while the player is collapsed.
    fn mini(&mut self, ui: &mut Ui, full: Rect, shown: f32) {
        let t = self.t;
        let (w, h) = ((full.width() - 32.).min(760.), 64.);
        let rise = (1. - shown) * (h + 24.);
        let rect = Rect::from_min_size(pos2(full.center().x - w / 2., full.max.y - 14. - h + rise), vec2(w, h));
        let ui = &mut Self::region(ui, full);
        ui.set_opacity(shown);
        let p = ui.painter().clone();
        shadow(&p, rect, 32., 10, 28, 82);
        p.rect_filled(rect, rad(32.), t.s2);
        let tc = rect.center().y;

        // Transport: shuffle, prev, play, next, repeat.
        let x0 = rect.min.x + 40.;
        for (i, (ic, s)) in [(icon!("shuf"), 16.), (icon!("rw"), 18.), (icon!("play"), 24.), (icon!("ff"), 18.), (icon!("rep"), 16.)].into_iter().enumerate() {
            icon_button(ui, &t, ("mini", i), pos2(x0 + i as f32 * 40., tc), 36., ic, s, if i % 4 == 0 { t.tm } else { t.tx });
        }
        let ldiv = x0 + 160. + 32.;
        p.rect_filled(Rect::from_center_size(pos2(ldiv, tc), vec2(1., 28.)), 0., t.s3);

        // Right cluster: favourite, lyrics, queue, volume, open sidebar.
        let rx = rect.max.x - 40.;
        let at = |i: usize| pos2(rx - (4 - i) as f32 * 40., tc);
        let rdiv = at(0).x - 32.;
        p.rect_filled(Rect::from_center_size(pos2(rdiv, tc), vec2(1., 28.)), 0., t.s3);
        icon_button(ui, &t, "mini-fav", at(0), 36., icon!("star"), 18., t.tx);
        if icon_button(ui, &t, "mini-lyr", at(1), 36., icon!("lyr"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Lyrics));
        }
        if icon_button(ui, &t, "mini-que", at(2), 36., icon!("que"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Queue));
        }
        let vol_btn = Rect::from_center_size(at(3), vec2(36., 36.));
        icon_button(ui, &t, "mini-vol", at(3), 36., if self.volume < 0.35 { icon!("vlo") } else { icon!("vhi") }, 18., t.tx);
        if icon_button(ui, &t, "mini-open", at(4), 36., icon!("colr"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Art));
        }

        // Track: cover + title/artist over a thin progress line; opens the sidebar.
        let sec = Rect::from_min_max(pos2(ldiv + 10., rect.min.y + 6.), pos2(rdiv - 10., rect.max.y - 6.));
        if tonal(ui, &t, "mini-who", sec, 14., None).clicked() {
            self.set_player(Some(PlayerView::Art));
        }
        let cy = tc - 4.;
        let cover = self.g.cover(ui.ctx(), &t);
        egui::Image::new(sized(cover, 36.)).corner_radius(rad(6.)).paint_at(ui, Rect::from_center_size(pos2(sec.min.x + 8. + 18., cy), vec2(36., 36.)));
        let (tx, tw) = (sec.min.x + 8. + 36. + 10., sec.width() - 8. - 36. - 10. - 8.);
        trunc(&p, pos2(tx, cy - 8.), tw, "Window Seat, Rain", font_w(13., Weight::Semi), t.tx);
        trunc(&p, pos2(tx, cy + 9.), tw, "Hoshi Yume", font(12.), t.tm);
        let bar = Rect::from_min_max(pos2(sec.min.x + 8., rect.max.y - 13.), pos2(sec.max.x - 8., rect.max.y - 11.));
        p.rect_filled(bar, rad(1.), t.s3);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * 0.38, 2.)), rad(1.), t.tx);

        // Volume popup: vertical slider above the button, open while hovered or being dragged.
        let pop = Rect::from_center_size(pos2(vol_btn.center().x, rect.min.y - 8. - 70.), vec2(44., 140.));
        let hover = ui.input(|i| i.pointer.hover_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        let zone = if self.vol_open { vol_btn.union(pop).expand(4.) } else { vol_btn };
        self.vol_open = shown > 0.9 && (hover.is_some_and(|h| zone.contains(h)) || (self.vol_open && down));
        if self.vol_open {
            shadow(&p, pop, 22., 6, 20, 70);
            p.rect_filled(pop, rad(22.), t.s2);
            let track = Rect::from_x_y_ranges(pop.center().x - 2.0..=pop.center().x + 2.0, pop.min.y + 16.0..=pop.max.y - 16.0);
            let r = ui.interact(track.expand2(vec2(14., 8.)), ui.scope_id().with("mini-vol-slider"), egui::Sense::click_and_drag());
            if let (true, Some(pt)) = (r.is_pointer_button_down_on(), r.interact_pointer_pos()) {
                self.volume = ((track.max.y - pt.y) / track.height()).clamp(0., 1.);
            }
            p.rect_filled(track, rad(2.), t.s3);
            p.rect_filled(Rect::from_min_max(pos2(track.min.x, track.max.y - track.height() * self.volume), track.max), rad(2.), t.tx);
        }
    }

    /// Fade over the collapsed card's sliver (solid at the window edge, 62% opaque toward the page) with a chevron; click reopens.
    fn peek(&mut self, ui: &mut Ui, pl_r: Rect, shown: f32) {
        let t = self.t;
        let c = Rect::from_min_max(pos2(pl_r.min.x + 10., pl_r.min.y), pos2(pl_r.min.x + self.player_w - 10., pl_r.max.y - 10.));
        let sliver = Rect::from_min_max(c.min, pos2(c.min.x + PEEK_W, c.max.y));
        let ui = &mut Self::region(ui, sliver.intersect(ui.max_rect()));
        let hot = ui.interact(sliver, ui.scope_id().with("peek"), egui::Sense::click());
        if hot.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let h = ui.ctx().animate_bool_with_time(ui.scope_id().with("peek-h"), hot.hovered(), 0.12);
        let (a_l, a_r) = (1., lerp(0.62..=0.78, h));
        // Left corners are rounded like the card: inset each strip by the circle's offset at that row.
        let r = 18.;
        let mut m = egui::Mesh::default();
        let (n, step) = (24, 1.);
        let col = |x: f32| t.s0.gamma_multiply(lerp(a_l..=a_r, ((x - sliver.min.x) / sliver.width()).clamp(0., 1.)) * shown);
        let mut strip = |y0: f32, y1: f32, inset: f32| {
            let b = m.vertices.len() as u32;
            let (xl, xr) = (sliver.min.x + inset, sliver.max.x);
            for (x, y) in [(xl, y0), (xr, y0), (xr, y1), (xl, y1)] {
                m.colored_vertex(pos2(x, y), col(x));
            }
            m.add_triangle(b, b + 1, b + 2);
            m.add_triangle(b, b + 2, b + 3);
        };
        for i in 0..n {
            let dy = r - i as f32 * step;
            let inset = r - (r * r - dy * dy).max(0.).sqrt();
            strip(sliver.min.y + i as f32 * step, sliver.min.y + (i + 1) as f32 * step, inset);
            strip(sliver.max.y - (i + 1) as f32 * step, sliver.max.y - i as f32 * step, inset);
        }
        strip(sliver.min.y + n as f32 * step, sliver.max.y - n as f32 * step, 0.);
        ui.painter().add(m);
        let nudge = -2. * h;
        let mut g = ui.new_child(UiBuilder::new().max_rect(sliver));
        g.set_opacity(shown);
        icon(&g, icon!("back"), pos2(sliver.center().x + nudge, sliver.center().y), 22., Color32::from_gray(250));
        if hot.clicked() {
            self.set_player(Some(self.view));
        }
    }

    pub fn go(&mut self, r: Route) {
        if r != self.route {
            self.back.push(std::mem::replace(&mut self.route, r));
            self.fwd.clear();
        }
    }

    fn back(&mut self) {
        if let Some(r) = self.back.pop() {
            self.fwd.push(std::mem::replace(&mut self.route, r));
        }
    }

    fn forward(&mut self) {
        if let Some(r) = self.fwd.pop() {
            self.back.push(std::mem::replace(&mut self.route, r));
        }
    }

    /// `Some(view)` opens the player on that view; `None` hides it.
    pub fn set_player(&mut self, open_on: Option<PlayerView>) {
        if let Some(v) = open_on {
            self.view = v;
        }
        self.player_open = open_on.is_some();
    }
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        self.t.s0.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self) {
        crate::backend::stop();
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let t = self.t;
        ui.painter().rect_filled(full, 0., t.s0);
        if !self.started {
            // egui focuses the first text edit it sees; start with nothing focused.
            self.started = true;
            ctx.memory_mut(|m| m.stop_text_input());
        }

        self.pl.tick(&ctx);
        let lib_t = anim(&ctx, "lib", self.lib_open);
        let pl_t = anim(&ctx, "player", self.player_open);
        self.lib_w = self.lib_w.clamp(LIB_MIN, LIB_MAX.min(full.width() - self.player_w - MAIN_MIN).max(LIB_MIN));
        let lw = lerp(LIB_SHUT..=self.lib_w, lib_t);
        self.player_w = self.player_w.clamp(PLAYER_MIN, PLAYER_MAX.min(full.width() - lw - MAIN_MIN).max(PLAYER_MIN));

        let tb = Rect::from_min_size(full.min, vec2(full.width(), TB_H));
        let body = Rect::from_min_max(pos2(full.min.x, tb.max.y), full.max);
        let lib_r = Rect::from_min_max(body.min, pos2(body.min.x + lw, body.max.y));
        // The card keeps its width and slides right until a PEEK_W sliver is left at the window edge.
        let pl_r = Rect::from_min_size(pos2(body.max.x - self.player_w + (1. - pl_t) * (self.player_w - PEEK_W - 10.), body.min.y), vec2(self.player_w, body.height()));
        let page_r = Rect::from_min_max(pos2(lib_r.max.x, body.min.y), pos2(pl_r.min.x, body.max.y - 10.));

        self.library(ui, lib_r, lib_t);
        self.page(ui, page_r);
        if self.lib_open && lib_t > 0.99 {
            // Grab zone straddling the library's right edge; double-click resets.
            let grab = Rect::from_min_max(pos2(lib_r.max.x - 8., lib_r.min.y + 6.), pos2(lib_r.max.x + 6., lib_r.max.y - 10.));
            let r = ui.interact(grab, ui.scope_id().with("lib-resize"), egui::Sense::click_and_drag());
            if r.hovered() || r.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if let (true, Some(p)) = (r.dragged(), r.interact_pointer_pos()) {
                self.lib_w = p.x - body.min.x;
            } else if r.double_clicked() {
                self.lib_w = LIB_OPEN;
            }
        }
        self.player(ui, pl_r, pl_t);
        if self.player_open && pl_t > 0.99 {
            // Grab zone straddling the card's left edge; double-click resets.
            let grab = Rect::from_min_max(pos2(pl_r.min.x - 6., pl_r.min.y + 6.), pos2(pl_r.min.x + 8., pl_r.max.y - 10.));
            let r = ui.interact(grab, ui.scope_id().with("player-resize"), egui::Sense::click_and_drag());
            if r.hovered() || r.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if let (true, Some(p)) = (r.dragged(), r.interact_pointer_pos()) {
                self.player_w = body.max.x - p.x;
            } else if r.double_clicked() {
                self.player_w = PLAYER_W;
            }
        }
        self.titlebar(ui, tb, lw);
        if pl_t < 1. {
            self.peek(ui, pl_r, 1. - pl_t);
            self.mini(ui, full, 1. - pl_t);
        }
    }
}
