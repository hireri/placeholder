//! Shell: titlebar, collapsible library (left), page (centre), collapsible player (right), mini pill.
//!
//! egui is immediate mode, so the "layout" is plain rect arithmetic: the two sidebar widths come from
//! `animate_bool` (0..1) and every region is drawn into its own clipped child `Ui`. Contents of a
//! collapsing sidebar keep their size and are simply clipped, as in the design mock.

use crate::audio;
use crate::gfx::*;
use crate::library::{LibItem, LibSort, LibView};
use crate::data::{Data, State, Track};
use std::sync::Arc;
use crate::media::{Cmd, Media};
use crate::player::Player;
use crate::theme::Theme;
use eframe::egui::{self, lerp, Color32, pos2, vec2, Context, Id, Key, Modifiers, PointerButton, Rect, Ui, UiBuilder};
use serde::{Deserialize, Serialize};

pub const TB_H: f32 = 52.;
pub const LIB_OPEN: f32 = 264.;
pub const LIB_MIN: f32 = 200.;
pub const LIB_MAX: f32 = 360.;
pub const LIB_SHUT: f32 = 68.;
pub const PLAYER_W: f32 = 340.;
const PLAYER_MIN: f32 = 270.;
/// Wide enough for a 16:9 video thumbnail to read at a useful size; MAIN_MIN still caps it on small windows.
const PLAYER_MAX: f32 = 760.;
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
    /// One song (`App::song`): cover, details and the album it is on.
    Song,
}

/// A page in the back/forward history: the route plus what it shows.
#[derive(Clone)]
pub struct Loc {
    route: Route,
    sel: Option<(&'static str, String)>,
    song: Option<Track>,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlayerView {
    Art,
    Lyrics,
    Queue,
}

pub struct App {
    pub t: Theme,
    /// Theme blend in flight: from, to, start time. `t` is drawn from it every frame.
    theme_anim: (Theme, Theme, f64),
    pub g: Gfx,
    pub lib_open: bool,
    pub player_open: bool,
    pub player_w: f32,
    pub view: PlayerView,
    pub route: Route,
    pub back: Vec<Loc>,
    pub fwd: Vec<Loc>,
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
    /// Song shown by the Song route.
    pub song: Option<Track>,
    /// Right-click menu: where it opened and what it is for. Drawn by `menu::context_menu`.
    pub ctx_menu: Option<(egui::Pos2, crate::menu::Target)>,
    /// Button that opened `ctx_menu`: presses on it toggle the menu instead of counting as "outside".
    pub ctx_keep: egui::Rect,
    pub home_chip: usize,
    pub pl: Player,
    pub volume: f32,
    pub vol_open: bool,
    /// Music videos play in the player's floating rect (`true`) or show their thumbnail.
    pub video_on: bool,
    #[cfg(feature = "video")]
    pub video: Option<crate::video::Video>,
    /// OS media controls (MPRIS/SMTC); `None` when the platform backend is unavailable.
    media: Option<Media>,
    started: bool,
}

/// UI state kept between runs (eframe storage).
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    lib_open: bool,
    player_open: bool,
    lib_w: f32,
    player_w: f32,
    lib_sort: LibSort,
    lib_view: LibView,
    view: PlayerView,
    volume: f32,
    video_on: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { lib_open: true, player_open: true, lib_w: LIB_OPEN, player_w: PLAYER_W, lib_sort: LibSort::Recents, lib_view: LibView::List, view: PlayerView::Art, volume: 0.7, video_on: true }
    }
}

const PREFS: &str = "prefs";

/// Win32 window handle, which SMTC needs; other platforms take none.
#[cfg(target_os = "windows")]
fn os_window(cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn os_window(_: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    None
}

fn visuals(ctx: &Context, t: &Theme) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(t.tx);
    v.selection.bg_fill = t.ax.gamma_multiply(0.35);
    v.selection.stroke = egui::Stroke::NONE;
    v.text_cursor.stroke.color = t.ax;
    v.window_fill = t.s0;
    v.panel_fill = t.s0;
    ctx.set_visuals(v);
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
        let prefs: Prefs = cc.storage.and_then(|s| eframe::get_value(s, PREFS)).unwrap_or_default();
        let volume = prefs.volume.clamp(0., 1.);
        let repaint = cc.egui_ctx.clone();
        let pl = Player::new(audio::Engine::new(move || repaint.request_repaint()));
        let wake = cc.egui_ctx.clone();
        let media = Media::new(move || wake.request_repaint(), os_window(cc));
        pl.engine.set_volume(volume);
        let t = Theme::idle();
        visuals(&cc.egui_ctx, &t);
        App {
            t,
            theme_anim: (t, t, 0.),
            g: Gfx::new(),
            lib_open: prefs.lib_open,
            player_open: prefs.player_open,
            player_w: prefs.player_w,
            view: prefs.view,
            route: Route::Home,
            back: Vec::new(),
            fwd: Vec::new(),
            lib_sort: prefs.lib_sort,
            lib_view: prefs.lib_view,
            lib_w: prefs.lib_w,
            lib_shown: if prefs.lib_open { prefs.lib_view } else { LibView::List },
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
            song: None,
            ctx_menu: None,
            ctx_keep: egui::Rect::NOTHING,
            home_chip: 0,
            pl,
            volume,
            vol_open: false,
            video_on: prefs.video_on,
            #[cfg(feature = "video")]
            video: None,
            media,
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
        let user = match &*self.data.status.lock() {
            State::Ready(s) => [&s.youtube, &s.spotify].into_iter().filter(|s| s.logged_in).find_map(|s| s.user.as_ref().map(|u| u.name.clone())),
            _ => None,
        };
        let initial = user.as_deref().and_then(|n| n.chars().next()).unwrap_or('?').to_uppercase().to_string();
        let name = user.unwrap_or_else(|| "Sign in".into());
        let me_w = 3. + 26. + 8. + text_w(&p, &name, font(13.)) + 10.;
        let me = Rect::from_min_size(pos2(x - me_w, cy - 16.), vec2(me_w, 32.));
        if tonal(ui, &t, "me", me, 16., None).clicked() {
            self.go(Route::Settings);
        }
        p.circle_filled(pos2(me.min.x + 16., cy), 13., t.axc);
        p.text(pos2(me.min.x + 16., cy), egui::Align2::CENTER_CENTER, initial, font_w(11., Weight::Semi), t.tx);
        trunc(&p, pos2(me.min.x + 37., cy), 100., &name, font(13.), t.tx);
        x = me.min.x - 6.;

        // Middle: back/forward + global search, centred in the space between brand and right cluster.
        let (lo, hi) = (tb.min.x + lw, x - 12.);
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
        let play = if self.pl.playing { icon!("pause") } else { icon!("play") };
        for (i, (ic, s)) in [(icon!("shuf"), 16.), (icon!("rw"), 18.), (play, 24.), (icon!("ff"), 18.), (icon!("rep"), 16.)].into_iter().enumerate() {
            let c = pos2(x0 + i as f32 * 40., tc);
            let on = match i {
                0 => self.pl.shuffle,
                4 => self.pl.repeat,
                _ => true,
            };
            if i % 4 == 0 && on {
                p.circle_filled(c, 18., t.tx.gamma_multiply(0.24));
            }
            if icon_button(ui, &t, ("mini", i), c, 36., ic, s, if on { t.tx } else { t.tm }).clicked() {
                match i {
                    0 => self.pl.toggle_shuffle(),
                    1 => self.pl.prev(ui.ctx()),
                    2 => self.pl.toggle(ui.ctx()),
                    3 => self.pl.next(ui.ctx()),
                    _ => self.pl.toggle_repeat(),
                }
            }
        }
        let ldiv = x0 + 160. + 32.;
        p.rect_filled(Rect::from_center_size(pos2(ldiv, tc), vec2(1., 28.)), 0., t.s3);

        // Right cluster: favourite, lyrics, queue, volume, open sidebar.
        let rx = rect.max.x - 40.;
        let at = |i: usize| pos2(rx - (4 - i) as f32 * 40., tc);
        let rdiv = at(0).x - 32.;
        p.rect_filled(Rect::from_center_size(pos2(rdiv, tc), vec2(1., 28.)), 0., t.s3);
        if icon_button(ui, &t, "mini-fav", at(0), 36., if self.pl.liked { icon!("starf") } else { icon!("star") }, 18., if self.pl.liked { t.ax } else { t.tx }).clicked() {
            self.pl.set_like(ui.ctx());
        }
        if icon_button(ui, &t, "mini-lyr", at(1), 36., icon!("lyr"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Lyrics));
        }
        if icon_button(ui, &t, "mini-que", at(2), 36., icon!("que"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Queue));
        }
        let vol_btn = Rect::from_center_size(at(3), vec2(36., 36.));
        if icon_button(ui, &t, "mini-vol", at(3), 36., if self.volume < 0.35 { icon!("vlo") } else { icon!("vhi") }, 18., t.tx).clicked() {
            self.vol_open = !self.vol_open;
        }
        if icon_button(ui, &t, "mini-open", at(4), 36., icon!("colr"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Art));
        }

        // Track: cover + title/artist over a thin progress line; opens the sidebar.
        let sec = Rect::from_min_max(pos2(ldiv + 10., rect.min.y + 6.), pos2(rdiv - 10., rect.max.y - 6.));
        if tonal(ui, &t, "mini-who", sec, 14., None).clicked() {
            self.set_player(Some(PlayerView::Art));
        }
        let cy = tc - 4.;
        self.thumb(ui, self.pl.song().and_then(|s| s.thumbnail.as_deref()), Rect::from_center_size(pos2(sec.min.x + 8. + 18., cy), vec2(36., 36.)), 6.);
        let (tx, tw) = (sec.min.x + 8. + 36. + 10., sec.width() - 8. - 36. - 10. - 8.);
        let (title, artist) = self.pl.song().map_or(("Nothing playing".to_string(), String::new()), |s| (s.title.clone(), s.artist.clone()));
        trunc(&p, pos2(tx, cy - 8.), tw, &title, font_w(13., Weight::Semi), t.tx);
        trunc(&p, pos2(tx, cy + 9.), tw, &artist, font(12.), t.tm);
        let bar = Rect::from_min_max(pos2(sec.min.x + 8., rect.max.y - 13.), pos2(sec.max.x - 8., rect.max.y - 11.));
        p.rect_filled(bar, rad(1.), t.s3);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * self.pl.frac(), bar.height())), rad(1.), t.tx);

        // Volume popup: vertical slider above the button. Its button toggles it; it closes once the pointer leaves both (not while dragging).
        let pop = Rect::from_center_size(pos2(vol_btn.center().x, rect.min.y - 8. - 70.), vec2(44., 140.));
        let hover = ui.input(|i| i.pointer.hover_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        let away = !hover.is_some_and(|h| vol_btn.union(pop).expand(4.).contains(h)) && !down;
        self.vol_open = shown > 0.9 && self.vol_open && !away;
        if self.vol_open {
            shadow(&p, pop, 22., 6, 20, 70);
            p.rect_filled(pop, rad(22.), t.s2);
            let track = Rect::from_x_y_ranges(pop.center().x - 2.0..=pop.center().x + 2.0, pop.min.y + 16.0..=pop.max.y - 16.0);
            let r = ui.interact(track.expand2(vec2(14., 8.)), ui.scope_id().with("mini-vol-slider"), egui::Sense::click_and_drag());
            if let (true, Some(pt)) = (r.is_pointer_button_down_on(), r.interact_pointer_pos()) {
                self.volume = ((track.max.y - pt.y) / track.height()).clamp(0., 1.);
                self.pl.engine.set_volume(self.volume);
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
        // Outline with 18px rounded left corners, as (point, outward normal). A 1px transparent ring
        // offset along each normal gives the mesh an anti-aliased edge, which strips alone cannot.
        let (r, seg) = (18., 12);
        let (x0, x1, y0, y1) = (sliver.min.x, sliver.max.x, sliver.min.y, sliver.max.y);
        let mut edge = Vec::with_capacity(2 * seg + 4);
        for i in 0..=seg {
            let th = std::f32::consts::FRAC_PI_2 * i as f32 / seg as f32;
            let (s, c) = (th.sin(), th.cos());
            edge.push((pos2(x0 + r - r * s, y0 + r - r * c), vec2(-s, -c)));
        }
        for i in 0..=seg {
            let th = std::f32::consts::FRAC_PI_2 * i as f32 / seg as f32;
            let (s, c) = (th.sin(), th.cos());
            edge.push((pos2(x0 + r - r * c, y1 - r + r * s), vec2(-c, s)));
        }
        edge.push((pos2(x1, y1), vec2(0., 1.)));
        edge.push((pos2(x1, y0), vec2(0., -1.)));
        let m_pts = edge.len();
        let fw = 1.;
        let mut m = egui::Mesh::default();
        let col = |x: f32, k: f32| t.s0.gamma_multiply(lerp(a_l..=a_r, ((x - x0) / sliver.width()).clamp(0., 1.)) * shown * k);
        for &(p, _) in &edge {
            m.colored_vertex(p, col(p.x, 1.));
        }
        for &(p, nrm) in &edge {
            m.colored_vertex(p + nrm * fw, col(p.x, 0.));
        }
        for k in 1..m_pts as u32 - 1 {
            m.add_triangle(0, k, k + 1);
        }
        for k in 0..m_pts as u32 {
            let (a, b) = (k, (k + 1) % m_pts as u32);
            let (oa, ob) = (a + m_pts as u32, b + m_pts as u32);
            m.add_triangle(a, b, ob);
            m.add_triangle(a, ob, oa);
        }
        ui.painter().add(m);
        let nudge = -2. * h;
        let mut g = ui.new_child(UiBuilder::new().max_rect(sliver));
        g.set_opacity(shown);
        icon(&g, icon!("back"), pos2(sliver.center().x + nudge, sliver.center().y), 22., Color32::from_gray(250));
        if hot.clicked() {
            self.set_player(Some(self.view));
        }
    }

    fn here(&self) -> Loc {
        Loc { route: self.route, sel: self.sel.clone(), song: self.song.clone() }
    }

    /// Makes `l` the current page and returns the one it replaced.
    fn restore(&mut self, l: Loc) -> Loc {
        let was = self.here();
        (self.route, self.sel, self.song) = (l.route, l.sel, l.song);
        was
    }

    /// Opens a route that needs no selection (nav, settings, search).
    pub fn go(&mut self, r: Route) {
        if r != self.route {
            let sel = self.sel.clone();
            self.open(r, sel);
        }
    }

    /// Opens `r` showing `sel` (endpoint segment, browse id); a no-op if that page is already shown.
    pub fn open(&mut self, r: Route, sel: Option<(&'static str, String)>) {
        if r == self.route && sel == self.sel && r != Route::Song {
            return;
        }
        let was = self.restore(Loc { route: r, sel, song: self.song.clone() });
        self.back.push(was);
        self.fwd.clear();
    }

    pub fn open_artist(&mut self, id: &str) {
        self.open(Route::Artist, Some(("artists", id.to_string())));
    }

    pub fn open_album(&mut self, id: &str) {
        self.open(Route::Album, Some(("albums", id.to_string())));
    }

    pub fn open_song(&mut self, t: Track) {
        if self.route == Route::Song && self.song.as_ref().is_some_and(|s| s.id == t.id) {
            return;
        }
        let was = self.restore(Loc { route: Route::Song, sel: self.sel.clone(), song: Some(t) });
        self.back.push(was);
        self.fwd.clear();
    }

    fn back(&mut self) {
        if let Some(l) = self.back.pop() {
            let was = self.restore(l);
            self.fwd.push(was);
        }
    }

    fn forward(&mut self) {
        if let Some(l) = self.fwd.pop() {
            let was = self.restore(l);
            self.back.push(was);
        }
    }

    /// Mirrors the current song to the OS controls and applies their commands. Runs every frame, also while the sidebar is collapsed.
    fn sync_media(&mut self, ctx: &Context) {
        let Some(m) = self.media.as_mut() else { return };
        match self.pl.song() {
            Some(s) => {
                let cover = s.thumbnail.as_deref().map(|u| sized_url(u, 544));
                let dur = self.pl.dur();
                m.set_metadata(&s.title, &s.artist, &s.album, cover.as_deref(), (dur > 0.).then_some(dur));
                m.set_playback(self.pl.playing, self.pl.pos);
                m.set_volume(self.volume);
            }
            None => m.set_stopped(),
        }
        for cmd in m.poll() {
            match cmd {
                Cmd::Play if !self.pl.playing => self.pl.toggle(ctx),
                Cmd::Pause if self.pl.playing => self.pl.toggle(ctx),
                Cmd::Toggle => self.pl.toggle(ctx),
                Cmd::Next => self.pl.next(ctx),
                Cmd::Prev => self.pl.prev(ctx),
                Cmd::Seek(secs) => self.pl.seek(secs),
                Cmd::SeekBy(by) => self.pl.seek((self.pl.pos + by).max(0.)),
                Cmd::SetVolume(v) => {
                    self.volume = v.clamp(0., 1.);
                    self.pl.engine.set_volume(self.volume);
                }
                _ => {}
            }
        }
    }

    /// Fetches the current and next covers (backdrop + tone) and lyrics ahead of use, and eases the theme toward the current cover's tone.
    fn follow_track(&mut self, ctx: &Context) {
        self.pl.poll_lyrics(ctx);
        let cur = self.pl.song().and_then(|s| s.thumbnail.clone());
        for u in [cur.clone(), self.pl.next_song().and_then(|s| s.thumbnail.clone())].into_iter().flatten() {
            self.g.fetch(ctx, &u);
        }
        let want = match cur {
            None => Theme::idle(),
            // Until the cover's tone arrives keep the theme the last song left; a grey or unreadable cover goes neutral.
            Some(u) => match self.g.tone(&u) {
                None => self.theme_anim.1,
                Some(Some(tone)) => Theme::tinted(tone),
                Some(None) => Theme::idle(),
            },
        };
        let now = ctx.input(|i| i.time);
        if want != self.theme_anim.1 {
            self.theme_anim = (self.t, want, now);
        }
        let (from, to, at) = self.theme_anim;
        let k = ((now - at) / 0.6).clamp(0., 1.) as f32;
        let t = from.lerp(&to, k * k * (3. - 2. * k));
        if t != self.t {
            self.t = t;
            visuals(ctx, &t);
            ctx.request_repaint();
        }
    }

    /// Global shortcuts; skipped while a text field has focus so typing is never swallowed.
    fn keys(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (ctrl, alt) = (Modifiers::COMMAND, Modifiers::ALT);
        let hit = |m, k| ctx.input_mut(|i| i.consume_key(m, k));
        if hit(Modifiers::NONE, Key::Space) {
            self.pl.toggle(ctx);
        }
        if hit(ctrl, Key::ArrowRight) {
            self.pl.next(ctx);
        }
        if hit(ctrl, Key::ArrowLeft) {
            self.pl.prev(ctx);
        }
        // Before the plain arrows: matching ignores Alt when the pattern lacks it.
        if hit(alt, Key::ArrowLeft) {
            self.back();
        }
        if hit(alt, Key::ArrowRight) {
            self.forward();
        }
        for (k, by) in [(Key::ArrowRight, 5.), (Key::ArrowLeft, -5.)] {
            if hit(Modifiers::NONE, k) {
                let end = self.pl.dur();
                self.pl.seek((self.pl.pos + by).clamp(0., if end > 0. { end } else { f32::MAX }));
            }
        }
        for (k, by) in [(Key::ArrowUp, 0.05), (Key::ArrowDown, -0.05)] {
            if hit(ctrl, k) {
                self.volume = (self.volume + by).clamp(0., 1.);
                self.pl.engine.set_volume(self.volume);
            }
        }
        if hit(ctrl, Key::S) {
            self.pl.toggle_shuffle();
        }
        if hit(ctrl, Key::R) {
            self.pl.toggle_repeat();
        }
        if hit(ctrl, Key::L) {
            self.pl.set_like(ctx);
        }
        for (k, v) in [(Key::Num1, PlayerView::Art), (Key::Num2, PlayerView::Lyrics), (Key::Num3, PlayerView::Queue)] {
            if hit(ctrl, k) {
                self.set_player(Some(v));
            }
        }
        if hit(ctrl, Key::B) {
            self.lib_open = !self.lib_open;
        }
        if hit(ctrl, Key::P) {
            self.set_player((!self.player_open).then_some(self.view));
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

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let p = Prefs { lib_open: self.lib_open, player_open: self.player_open, lib_w: self.lib_w, player_w: self.player_w, lib_sort: self.lib_sort, lib_view: self.lib_view, view: self.view, volume: self.volume, video_on: self.video_on };
        eframe::set_value(storage, PREFS, &p);
    }

    fn on_exit(&mut self) {
        crate::backend::stop();
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        self.pl.tick(&ctx);
        self.follow_track(&ctx);
        self.keys(&ctx);
        let t = self.t;
        ui.painter().rect_filled(full, 0., t.s0);
        if !self.started {
            // egui focuses the first text edit it sees; start with nothing focused.
            self.started = true;
            ctx.memory_mut(|m| m.stop_text_input());
        }

        self.sync_media(&ctx);
        // Mouse back/forward buttons work everywhere, also while a text field has focus.
        let (b4, b5) = ctx.input(|i| (i.pointer.button_pressed(PointerButton::Extra1), i.pointer.button_pressed(PointerButton::Extra2)));
        if b4 {
            self.back();
        }
        if b5 {
            self.forward();
        }
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
        self.context_menu(&ctx);
    }
}
