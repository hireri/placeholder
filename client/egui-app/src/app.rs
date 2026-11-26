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
use crate::settings::Opts;
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
    /// A home shelf listed in full; `sel` is ("shelf", shelf title).
    Shelf,
}

#[derive(Clone, Copy, PartialEq, Default)]
enum IslState {
    #[default]
    Idle,
    Import,
    Done,
}

/// What the titlebar island shows: its state, the text it keeps while collapsing, bar fill and the unread dot.
#[derive(Default)]
struct Isl {
    st: IslState,
    label: String,
    bar: bool,
    frac: f32,
    unread: bool,
    since: f64,
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
    /// `theme.toml`, loaded while the custom theme option is on.
    custom: Option<crate::theme::Custom>,
    /// Modified time of `theme.toml` when `custom` was last read.
    custom_seen: Option<std::time::SystemTime>,
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
    pub lib_items: Vec<LibItem>,
    pub lib_loaded: bool,
    pub lib_rev: usize,
    /// Query the Search page shows, set by the palette.
    pub submitted: String,
    pub data: Arc<Data>,
    /// Album/artist/playlist shown by the Album and Artist routes: (endpoint segment, browse id).
    pub sel: Option<(&'static str, String)>,
    /// The single shown by the Album route when `sel` is `("song", _)`: a song with no album, as a one-track playlist.
    pub song: Option<Track>,
    /// An album/playlist/artist whose hover-play was clicked before its page had loaded: (endpoint segment, browse id).
    pub want_play: Option<(&'static str, String)>,
    /// Right-click menu: where it opened and what it is for. Drawn by `menu::context_menu`.
    pub ctx_menu: Option<(egui::Pos2, crate::menu::Target)>,
    /// Button that opened `ctx_menu`: presses on it toggle the menu instead of counting as "outside".
    pub ctx_keep: egui::Rect,
    /// Command palette (Ctrl K) while open. Drawn by `App::palette`.
    pub palette: Option<crate::palette::State>,
    /// Last searches run from the palette, newest first (max 5). Kept between runs.
    pub recents: Vec<String>,
    /// Profile dropdown: when it opened, while it is open. Drawn by `App::me_dropdown`.
    pub(crate) me_menu: Option<f64>,
    /// The profile button, whose presses toggle the dropdown instead of counting as "outside".
    me_btn: Rect,
    /// Titlebar status island.
    isl: Isl,
    pub home_chip: usize,
    pub pl: Player,
    pub volume: f32,
    pub vol_open: bool,
    pub opts: Opts,
    /// Category shown on the Settings page.
    pub settings_tab: usize,
    /// Language or country list open on the Content tab.
    pub settings_pick: Option<crate::settings::Pick>,
    /// Proxy text fields (server, username, password), seeded from the daemon's settings once they load.
    pub proxy_buf: Option<[String; 3]>,
    /// Music videos play in the player's floating rect (`true`) or show their thumbnail.
    pub video_on: bool,
    /// Requested video height (one of `video::HEIGHTS`).
    pub video_h: u32,
    #[cfg(feature = "video")]
    pub video: Option<crate::video::Video>,
    /// OS media controls (MPRIS/SMTC); `None` when the platform backend is unavailable.
    media: Option<Media>,
    discord: crate::discord::Discord,
    pub scrobbler: crate::scrobble::Scrobbler,
    pub sleep: crate::sleep::Sleep,
    started: bool,
    /// Playlist waiting on the delete popup: (id, label).
    delete_ask: Option<(String, String)>,
    pub lfm_buf: Option<[String; 2]>,
}

/// Selectable video heights; the player's floating rect is at most ~720px wide, so 480 is the default.
/// No 360p or lower: YouTube's SABR stream for those takes ~12 s to start or hangs.
pub const VIDEO_HEIGHTS: [u32; 3] = [480, 720, 1080];

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
    opts: Opts,
    recents: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { lib_open: true, player_open: true, lib_w: LIB_OPEN, player_w: PLAYER_W, lib_sort: LibSort::Recents, lib_view: LibView::List, view: PlayerView::Art, volume: 0.7, opts: Opts::default(), recents: Vec::new() }
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
    let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.override_text_color = Some(t.tx);
    v.selection.bg_fill = t.ax.gamma_multiply(0.35);
    v.selection.stroke = egui::Stroke::NONE;
    v.text_cursor.stroke.color = t.ax;
    v.window_fill = t.s0;
    v.panel_fill = t.s0;
    ctx.set_visuals(v);
}

/// 0..1 progress that eases toward `on`, reversing smoothly if toggled mid-flight.
pub fn anim(ctx: &Context, id: &str, on: bool) -> f32 {
    ctx.animate_bool_with_time_and_easing(Id::unique(id), on, 0.3, |t| 1. - (1. - t).powi(4))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install(&cc.egui_ctx);
        let prefs: Prefs = cc.storage.and_then(|s| eframe::get_value(s, PREFS)).unwrap_or_default();
        let volume = prefs.volume.clamp(0., 1.);
        let mut pl = Player::new(audio::Engine::new(crate::tray::wake));
        let media = Media::new(crate::tray::wake, os_window(cc));
        pl.engine.set_volume(volume);
        pl.endless = prefs.opts.endless;
        pl.engine.set_crossfade(prefs.opts.crossfade as f32);
        pl.engine.set_gapless(prefs.opts.gapless);
        crate::cache::set_limit_mb(prefs.opts.cache_mb);
        pl.engine.set_normalize(prefs.opts.normalize);
        crate::data::HIDE_EXPLICIT.store(prefs.opts.hide_explicit, std::sync::atomic::Ordering::Relaxed);
        crate::data::HIDE_VIDEO.store(prefs.opts.hide_video, std::sync::atomic::Ordering::Relaxed);
        let t = Theme::idle(true);
        visuals(&cc.egui_ctx, &t);
        App {
            t,
            theme_anim: (t, t, 0.),
            custom: prefs.opts.custom.then(crate::theme::load_custom).flatten(),
            custom_seen: crate::theme::mtime(),
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
            lib_items: Vec::new(),
            lib_loaded: false,
            lib_rev: 0,
            submitted: String::new(),
            data: Arc::new(Data::new()),
            sel: None,
            song: None,
            want_play: None,
            ctx_menu: None,
            ctx_keep: egui::Rect::NOTHING,
            palette: None,
            recents: prefs.recents,
            me_menu: None,
            me_btn: Rect::NOTHING,
            isl: Isl::default(),
            home_chip: 0,
            pl,
            volume,
            vol_open: false,
            opts: prefs.opts,
            settings_tab: 0,
            settings_pick: None,
            proxy_buf: None,
            video_on: prefs.opts.video,
            video_h: if VIDEO_HEIGHTS.contains(&prefs.opts.video_h) { prefs.opts.video_h } else { VIDEO_HEIGHTS[0] },
            #[cfg(feature = "video")]
            video: None,
            media,
            discord: crate::discord::Discord::new(),
            scrobbler: Default::default(),
            sleep: Default::default(),
            started: false,
            delete_ask: None,
            lfm_buf: None,
        }
    }

    /// A clipped child `Ui` covering `rect`.
    pub fn region(ui: &mut Ui, rect: Rect) -> Ui {
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child
    }

    /// Name of the first signed-in account, if any.
    /// The signed-in YouTube Music profile as (avatar, name).
    fn youtube_user(&self) -> Option<(Option<String>, String)> {
        match &*self.data.status.lock() {
            State::Ready(s) if s.youtube.logged_in => s.youtube.user.as_ref().map(|u| (u.avatar_url.clone(), u.name.clone())),
            _ => None,
        }
    }

    fn user_name(&self) -> Option<String> {
        match &*self.data.status.lock() {
            State::Ready(s) => [&s.youtube, &s.spotify].into_iter().filter(|s| s.logged_in).find_map(|s| s.user.as_ref().map(|u| u.name.clone())),
            _ => None,
        }
    }

    fn titlebar(&mut self, ui: &mut Ui, tb: Rect) {
        let t = self.t;
        let mut owned = App::region(ui, tb);
        let ui = &mut owned;
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let cy = tb.center().y;

        // Left: the logo (over the library, tinted to the accent) with back and forward just right of it.
        icon(ui, icon!("serafin"), pos2(tb.min.x + 30., cy), 30., t.ax);
        if icon_button(ui, &t, "back", pos2(tb.min.x + 71., cy), 32., icon!("back"), 16., if self.back.is_empty() { t.tf } else { t.tm }).clicked() {
            self.back();
        }
        if icon_button(ui, &t, "fwd", pos2(tb.min.x + 105., cy), 32., icon!("fwd"), 16., if self.fwd.is_empty() { t.tf } else { t.tm }).clicked() {
            self.forward();
        }

        // Right cluster, laid out from the right edge: status island, then the profile.
        self.data.status.ensure(&ctx, || crate::backend::get("/auth/status"));
        let user = self.youtube_user();
        let name = user.as_ref().map_or_else(|| "Sign in".into(), |u| u.1.clone());
        let me_w = 3. + 26. + 8. + text_w(&p, &name, font(13.)).min(100.) + 10.;
        let me = Rect::from_min_size(pos2(tb.max.x - 12. - me_w, cy - 16.), vec2(me_w, 32.));
        self.me_btn = me;
        if tonal(ui, &t, "me", me, 16., None).clicked() {
            if self.me_menu.is_some() {
                self.me_menu = None;
            } else {
                (self.palette, self.ctx_menu, self.lib_menu, self.vol_open) = (None, None, false, false);
                self.me_menu = Some(ctx.input(|i| i.time));
            }
        } else if self.me_menu.is_some() {
            p.rect_filled(me, rad(16.), t.s2);
        }
        self.avatar(ui, user.as_ref().and_then(|u| u.0.as_deref()), pos2(me.min.x + 16., cy), 13., user.as_ref().map(|u| u.1.as_str()));
        trunc(&p, pos2(me.min.x + 37., cy), 100., &name, font(13.), t.tx);
        self.island(ui, me.min.x - 6., cy);

        // Middle: a search pill centred on the window whatever the clusters beside it do. It only opens the palette.
        let w = (tb.width() - 640.).clamp(140., 300.);
        let pill = Rect::from_center_size(pos2(tb.center().x, cy), vec2(w, 32.));
        let r = ui.interact(pill, ui.scope_id().with("pill"), egui::Sense::click());
        if r.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let hot = ctx.animate_bool_with_time(r.id.with("hot"), r.hovered(), 0.09);
        p.rect_filled(pill, rad(16.), t.s2.lerp_to_gamma(t.s3, hot));
        icon(ui, icon!("search"), pos2(pill.min.x + 20., cy), 16., t.tm);
        let (kbd, kf) = ("Ctrl K", egui::FontId::monospace(10.5));
        let kw = text_w(&p, kbd, kf.clone());
        p.text(pos2(pill.max.x - 12., cy), egui::Align2::RIGHT_CENTER, kbd, kf, t.tf);
        trunc(&p, pos2(pill.min.x + 36., cy), pill.max.x - 12. - kw - 8. - (pill.min.x + 36.), "Search or run a command", font(13.), t.tm);
        if r.clicked() {
            self.open_palette(&ctx);
        }
    }

    /// Moves the status island between idle, importing and done from the Spotify matcher's real progress.
    fn island_tick(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        let (done, pending) = self.data.import_progress();
        let isl = &mut self.isl;
        if pending > 0 {
            (isl.st, isl.bar) = (IslState::Import, true);
            isl.label = format!("{done} / {}", done + pending);
            isl.frac = done as f32 / (done + pending) as f32;
        } else if isl.st == IslState::Import {
            let n = self.data.import_collect();
            (isl.st, isl.bar, isl.since, isl.frac) = (IslState::Done, false, now, 1.);
            isl.label = format!("Imported {n} song{}", if n == 1 { "" } else { "s" });
        }
        if isl.st == IslState::Done {
            if now - isl.since >= 3.2 {
                (isl.st, isl.unread) = (IslState::Idle, true);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(3.2 - (now - isl.since)));
            }
        }
    }

    /// The notifications bell, which widens into the Spotify import status while an import runs. Grows leftwards from `right`.
    fn island(&mut self, ui: &mut Ui, right: f32, cy: f32) {
        let (t, ctx, st) = (self.t, ui.ctx().clone(), self.isl.st);
        let p = ui.painter().clone();
        let on = if st == IslState::Idle { 0. } else { 1. };
        let spring = |x| bezier(0.16, 1., 0.3, 1., x);
        // Digits measure as 8s so the island holds still while the count ticks (Inter's digits are proportional).
        let probe: String = self.isl.label.chars().map(|c| if c.is_ascii_digit() { '8' } else { c }).collect();
        let lw = text_w(&p, &probe, font(12.));
        let full = 32. + lw + if self.isl.bar { 8. + 44. } else { 0. } + 12.;
        let w = lerp(32.0..=full, tween(&ctx, Id::unique("isl-w"), on, 0.32, |x| bezier(0.32, 0.72, 0., 1., x)));
        let rect = Rect::from_min_size(pos2(right - w, cy - 16.), vec2(w, 32.));
        let resp = ui.interact(rect, ui.scope_id().with("isl"), egui::Sense::click());
        if resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let hot = tween(&ctx, Id::unique("isl-h"), resp.hovered() as u8 as f32, 0.12, ease_out);
        let fill = tween(&ctx, Id::unique("isl-f"), on, 0.24, ease_out);
        p.rect_filled(rect, rad(16.), t.s2.gamma_multiply(hot * (1. - fill)));
        p.rect_filled(rect, rad(16.), t.s1.gamma_multiply(fill));
        let fg = t.tm.lerp_to_gamma(t.tx, hot);

        // Bell, import arrow and check cross-fade in place, each scaling up from 0.6.
        let c = pos2(rect.min.x + 16., cy);
        for (i, (uri, shown)) in [(icon!("bell"), IslState::Idle), (icon!("imp"), IslState::Import), (icon!("check"), IslState::Done)].into_iter().enumerate() {
            let to = (st == shown) as u8 as f32;
            let (a, s) = (tween(&ctx, Id::unique(("isl-a", i)), to, 0.16, ease_out), tween(&ctx, Id::unique(("isl-s", i)), to, 0.28, spring));
            if a > 0.01 {
                icon(ui, uri, c, 16. * (0.6 + 0.4 * s), if i == 2 { t.ax } else { fg }.gamma_multiply(a));
            }
        }

        // Label fades in 90ms after the width starts opening (200ms), and out first when closing (120ms).
        let k = tween(&ctx, Id::unique("isl-l"), on, 0.29, |x| x);
        let a = if st == IslState::Idle { 1. - (1. - k) * 0.29 / 0.12 } else { (k * 0.29 - 0.09) / 0.2 }.clamp(0., 1.);
        if a > 0. {
            let pc = p.with_clip_rect(rect);
            pc.text(pos2(rect.min.x + 32., cy), egui::Align2::LEFT_CENTER, &self.isl.label, font(12.), fg.gamma_multiply(a));
            if self.isl.bar {
                let track = Rect::from_min_size(pos2(rect.min.x + 32. + lw + 8., cy - 1.5), vec2(44., 3.));
                pc.rect_filled(track, rad(2.), t.s3.gamma_multiply(a));
                let f = tween(&ctx, Id::unique("isl-p"), self.isl.frac, 0.32, |x| x);
                pc.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * f, 3.)), rad(2.), t.ax.gamma_multiply(a));
            }
        }

        // Unread dot after an import finished, ringed in the page colour.
        let d = (self.isl.unread && st == IslState::Idle) as u8 as f32;
        let (da, ds) = (tween(&ctx, Id::unique("isl-da"), d, 0.12, ease_out), tween(&ctx, Id::unique("isl-ds"), d, 0.24, spring));
        if da > 0.01 {
            let at = pos2(rect.max.x - 9., rect.min.y + 9.);
            p.circle_filled(at, 6. * ds, t.s0.gamma_multiply(da));
            p.circle_filled(at, 4. * ds, t.ax.gamma_multiply(da));
        }

        let clicked = resp.clicked();
        resp.on_hover_text(match st {
            IslState::Idle => "Notifications",
            IslState::Import => "Importing from Spotify",
            IslState::Done => "Spotify import complete",
        });
        if clicked {
            match st {
                IslState::Import => self.go(Route::Import),
                // TODO: a notifications panel; for now opening the bell only clears the unread dot.
                IslState::Idle => self.isl.unread = false,
                IslState::Done => {}
            }
        }
    }

    /// Account dropdown under the profile button: right-aligned, fades in with a 6px rise. Closes on Esc, an outside press or a pick.
    fn me_dropdown(&mut self, ctx: &Context) {
        let Some(t0) = self.me_menu else { return };
        if self.palette.is_some() || self.ctx_menu.is_some() || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.me_menu = None;
            return;
        }
        let (t, now) = (self.t, ctx.input(|i| i.time));
        let k = if reduced_motion() { 1. } else { ease_out((((now - t0) / 0.14) as f32).min(1.)) };
        if k < 1. {
            ctx.request_repaint();
        }
        const ITEMS: [(&str, Option<&str>); 4] = [("Settings", Some(icon!("gear"))), ("Accounts", None), ("Keyboard shortcuts", None), ("Spotify import", None)];
        let scr = ctx.content_rect();
        let rect = Rect::from_min_size(pos2(scr.max.x - 12. - 220., scr.min.y + 48. + 6. * (1. - k)), vec2(220., 6. + 52. + ITEMS.len() as f32 * 35. - 1. + 6.));
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !self.me_btn.contains(p)));
        if outside {
            self.me_menu = None;
            return;
        }
        let mut picked = None;
        egui::Area::new(Id::unique("me-menu")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            ui.set_opacity(k);
            ui.allocate_rect(rect, egui::Sense::click()); // clicks on the menu must not reach what lies beneath
            let p = ui.painter().clone().with_clip_rect(scr);
            shadow(&p, rect, 14., 16, 40, 102);
            p.rect_filled(rect, rad(14.), t.s2);
            let name = self.user_name().unwrap_or_else(|| "Not signed in".into());
            trunc(&p, pos2(rect.min.x + 16., rect.min.y + 6. + 16.), rect.width() - 32., &name, font_w(13., Weight::Bold), t.tx);
            trunc(&p, pos2(rect.min.x + 16., rect.min.y + 6. + 34.), rect.width() - 32., "YouTube Music · Spotify", font(12.), t.tm);
            for (i, (label, ic)) in ITEMS.iter().enumerate() {
                let r = Rect::from_min_size(pos2(rect.min.x + 6., rect.min.y + 6. + 52. + i as f32 * 35.), vec2(rect.width() - 12., 34.));
                let resp = ui.interact(r, ui.scope_id().with(i), egui::Sense::click());
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    p.rect_filled(r, rad(8.), t.s3);
                }
                trunc(&p, pos2(r.min.x + 10., r.center().y), r.width() - 20. - 24., label, font(13.), t.tx);
                if let Some(ic) = ic {
                    icon(ui, ic, pos2(r.max.x - 10. - 8., r.center().y), 16., t.tm);
                }
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        });
        let Some(i) = picked else { return };
        self.me_menu = None;
        match i {
            0 | 1 | 2 => {
                self.settings_tab = [crate::settings::APPEARANCE, crate::settings::ACCOUNTS, crate::settings::KEYS][i];
                self.go(Route::Settings);
            }
            _ => self.go(Route::Import),
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

        // Transport: shuffle, prev, play, next, repeat, endless.
        let x0 = rect.min.x + 40.;
        let play = if self.pl.playing { icon!("pause") } else { icon!("play") };
        for (i, (ic, s)) in [(icon!("shuf"), 16.), (icon!("rw"), 18.), (play, 24.), (icon!("ff"), 18.), (icon!("rep"), 16.), (icon!("inf"), 18.)].into_iter().enumerate() {
            let c = pos2(x0 + i as f32 * 40., tc);
            let on = match i {
                0 => self.pl.shuffle,
                4 => self.pl.repeat,
                5 => self.pl.endless,
                _ => true,
            };
            if matches!(i, 0 | 4 | 5) && on {
                p.circle_filled(c, 18., t.tx.gamma_multiply(0.24));
            }
            let buffering = i == 2 && self.pl.buffering();
            if buffering {
                crate::page::spinner(&p, c, 10., ui.input(|i| i.time) as f32, t.tx);
                ui.ctx().request_repaint();
            }
            if glass_button(ui, &t, ("mini", i), c, 36., ic, s, if buffering { Color32::TRANSPARENT } else if on { t.tx } else { t.tm }).clicked() {
                match i {
                    0 => self.pl.toggle_shuffle(),
                    1 => self.pl.prev(ui.ctx()),
                    2 => self.pl.toggle(ui.ctx()),
                    3 => self.pl.next(ui.ctx()),
                    4 => self.pl.toggle_repeat(),
                    _ => self.pl.toggle_endless(),
                }
            }
        }
        let ldiv = x0 + 200. + 32.;
        p.rect_filled(Rect::from_center_size(pos2(ldiv, tc), vec2(1., 28.)), 0., t.s3);

        // Right cluster: favourite, lyrics, queue, volume (the track block opens the sidebar).
        let rx = rect.max.x - 40.;
        let at = |i: usize| pos2(rx - (3 - i) as f32 * 40., tc);
        let rdiv = at(0).x - 32.;
        p.rect_filled(Rect::from_center_size(pos2(rdiv, tc), vec2(1., 28.)), 0., t.s3);
        if glass_button(ui, &t, "mini-fav", at(0), 36., if self.pl.liked { icon!("starf") } else { icon!("star") }, 18., if self.pl.liked { t.ax } else { t.tx }).clicked() {
            self.pl.set_like(ui.ctx());
        }
        if glass_button(ui, &t, "mini-lyr", at(1), 36., icon!("lyr"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Lyrics));
        }
        if glass_button(ui, &t, "mini-que", at(2), 36., icon!("que"), 18., t.tx).clicked() {
            self.set_player(Some(PlayerView::Queue));
        }
        let vol_btn = Rect::from_center_size(at(3), vec2(36., 36.));
        if glass_button(ui, &t, "mini-vol", at(3), 36., if self.volume < 0.35 { icon!("vlo") } else { icon!("vhi") }, 18., t.tx).clicked() {
            self.vol_open = !self.vol_open;
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
        let h = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with("peek-h"), hot.hovered(), 0.15, ease_out);
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

    pub fn reload_custom(&mut self) {
        self.custom = self.opts.custom.then(crate::theme::load_custom).flatten();
        self.custom_seen = crate::theme::mtime();
    }

    /// Makes `l` the current page and returns the one it replaced.
    fn restore(&mut self, l: Loc) -> Loc {
        let was = self.here();
        if l.route == Route::Settings && self.route != Route::Settings {
            // Refetch the daemon's settings each time the page opens.
            (self.settings_pick, self.proxy_buf, self.lfm_buf) = (None, None, None);
            self.reload_custom();
            self.data.settings.reset();
        }
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
        let song = self.song.clone();
        self.open_with(r, sel, song);
    }

    fn open_with(&mut self, r: Route, sel: Option<(&'static str, String)>, song: Option<Track>) {
        if r == self.route && sel == self.sel {
            return;
        }
        let was = self.restore(Loc { route: r, sel, song });
        self.back.push(was);
        self.fwd.clear();
    }

    pub fn open_artist(&mut self, id: &str) {
        self.open(Route::Artist, Some(("artists", id.to_string())));
    }

    pub fn open_album(&mut self, id: &str) {
        self.open(Route::Album, Some(("albums", id.to_string())));
    }

    /// Songs have no page of their own: one on an album opens that album, a single opens as a one-track playlist.
    pub fn open_song(&mut self, t: Track) {
        match t.album_id.clone() {
            Some(id) => self.open_album(&id),
            None => {
                let sel = Some(("song", t.id.clone()));
                self.open_with(Route::Album, sel, Some(t));
            }
        }
    }

    /// Plays what an album/playlist/artist plays as soon as its page has loaded (started by a card's play button).
    pub fn resolve_play(&mut self, ctx: &egui::Context) {
        let Some((kind, id)) = self.want_play.clone() else { return };
        let d = self.data.detail(ctx, kind, &id);
        let state = match &*d.lock() {
            State::Ready(p) => Ok(p.songs()),
            State::Failed(e) => Err(Some(e.clone())),
            _ => Err(None),
        };
        match state {
            Ok(songs) => {
                if songs.is_empty() {
                    crate::toast::error("Nothing to play here");
                } else {
                    self.pl.play_tracks(ctx, &songs, 0, false);
                }
                self.want_play = None;
            }
            Err(Some(e)) => {
                crate::toast::error(format!("Couldn't load it: {e}"));
                self.want_play = None;
            }
            Err(None) => {}
        }
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

    /// Reattaches to a freshly created window after the previous one was destroyed (hidden to the tray on Wayland):
    /// everything tied to the old egui context is rebuilt, the player and library state carry over.
    pub fn rebind(&mut self, cc: &eframe::CreationContext<'_>) {
        install(&cc.egui_ctx);
        visuals(&cc.egui_ctx, &self.t);
        self.g = Gfx::new();
        #[cfg(feature = "video")]
        {
            self.video = None;
        }
        self.started = false;
        self.pl.reset_clock();
        self.theme_anim.2 = f64::NEG_INFINITY;
    }

    /// One step of the player while no window exists: advances the queue and serves media keys and tray commands.
    /// Deleting a playlist is permanent: ask first, unless Shift is held.
    pub fn confirm_delete(&mut self, ctx: &Context, id: &str) {
        if ctx.input(|i| i.modifiers.shift) {
            return self.data.delete_playlist(ctx, id);
        }
        let label = self.data.library_item(id).map_or_else(|| "This playlist".into(), |i| format!("“{}”", i.title));
        self.delete_ask = Some((id.into(), label));
    }

    fn delete_modal(&mut self, ctx: &Context) {
        let Some((id, label)) = self.delete_ask.clone() else { return };
        let mut answer = None;
        let m = egui::Modal::new(egui::Id::new("delete-playlist")).show(ctx, |ui| {
            ui.set_width(320.);
            ui.heading("Delete playlist?");
            ui.add_space(6.);
            ui.label(format!("{label} will be permanently deleted from YouTube Music. This can't be undone."));
            ui.label(egui::RichText::new("Hold Shift to skip this prompt.").weak());
            ui.add_space(10.);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Delete").color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(0xc6, 0x28, 0x28))).clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        if answer == Some(true) {
            self.data.delete_playlist(ctx, &id);
        }
        if answer.is_some() || m.should_close() {
            self.delete_ask = None;
        }
    }

    pub fn headless(&mut self, ctx: &Context) {
        self.pl.tick(ctx);
        if let Some((t, on)) = self.pl.like_event.take() {
            self.scrobbler.love(ctx, &t, on);
            self.data.mirror_like(t, on);
        }
        self.resolve_play(ctx);
        self.sync_media(ctx);
    }

    /// Mirrors the current song to the OS controls and applies their commands. Runs every frame, also while the sidebar is collapsed.
    fn sync_media(&mut self, ctx: &Context) {
        let mut cmds = crate::tray::take_cmds();
        let (dur, now) = (self.pl.dur(), self.pl.song());
        self.scrobbler.update(ctx, now.map(|s| (s.title.as_str(), s.artist.as_str(), s.album.as_str())), self.pl.playing, self.pl.pos, dur);
        if self.sleep.due(now.map(|s| (s.title.as_str(), s.artist.as_str()))) && self.pl.playing {
            self.pl.toggle(ctx);
        }
        let cover = self.pl.song().and_then(|s| s.thumbnail.as_deref().map(|u| sized_url(u, 544)));
        let song = self.pl.song().map(|s| (s.title.as_str(), s.artist.as_str(), cover.as_deref()));
        if self.opts.discord {
            self.discord.update(song, self.pl.playing, self.pl.pos, self.pl.dur());
        } else {
            self.discord.clear();
        }
        if let Some(m) = self.media.as_mut() {
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
            cmds.extend(m.poll());
        }
        for cmd in cmds {
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
        crate::tray::set_dark(ctx.system_theme() != Some(egui::Theme::Light));
        let cur = self.pl.song().and_then(|s| s.thumbnail.clone());
        for u in [cur.clone(), self.pl.next_song().and_then(|s| s.thumbnail.clone())].into_iter().flatten() {
            self.g.fetch(ctx, &u);
        }
        // Pick up edits to theme.toml while it is on.
        if self.opts.custom {
            if crate::theme::mtime() != self.custom_seen {
                self.reload_custom();
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
        let dark = match self.opts.mode {
            1 => false,
            2 => true,
            _ => ctx.system_theme() != Some(egui::Theme::Light),
        };
        let want = match cur {
            _ if !self.opts.auto_theme => Theme::preset(self.opts.accent, dark),
            None => Theme::preset(self.opts.accent, dark),
            // Until the cover's tone arrives keep the theme the last song left; a grey or unreadable cover goes neutral.
            Some(u) => match self.g.tone(&u) {
                None => self.theme_anim.1,
                Some(Some(tone)) => Theme::tinted(tone, dark),
                Some(None) => Theme::idle(dark),
            },
        };
        // Artwork theming wins while a cover is driving it; the custom colours apply otherwise.
        let artwork = self.opts.auto_theme && self.pl.song().is_some_and(|s| s.thumbnail.is_some());
        let want = self.custom.as_ref().filter(|_| !artwork).map_or(want, |c| want.custom(c));
        let want = if dark && self.opts.pure_black { want.pure_black() } else { want };
        let now = ctx.input(|i| i.time);
        if want != self.theme_anim.1 {
            // Light/dark flips are instant; everything else eases from what is on screen.
            let from = if want.dark != self.theme_anim.1.dark { want } else { self.t };
            self.theme_anim = (from, want, now);
        }
        let (from, to, at) = self.theme_anim;
        let k = ((now - at) / 0.5).clamp(0., 1.) as f32;
        let t = from.lerp(&to, k * k * (3. - 2. * k));
        if t != self.t {
            self.t = t;
            visuals(ctx, &t);
        }
        // Keep frames coming for the whole blend, including the first one where nothing has moved yet.
        if k < 1. {
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
        if hit(Modifiers::NONE, Key::Slash) {
            // The slash is also a text event; drop it so it doesn't land in the box the palette is about to focus.
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(s) if s == "/")));
            self.open_palette(ctx);
        }
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
        let p = Prefs { lib_open: self.lib_open, player_open: self.player_open, lib_w: self.lib_w, player_w: self.player_w, lib_sort: self.lib_sort, lib_view: self.lib_view, view: self.view, volume: self.volume, opts: self.opts, recents: self.recents.clone() };
        eframe::set_value(storage, PREFS, &p);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::backend::stop();
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        if ctx.input(|i| i.viewport().close_requested()) && crate::tray::hide_on_close() {
            crate::tray::hide(&ctx);
        }
        self.pl.tick(&ctx);
        if let Some((t, on)) = self.pl.like_event.take() {
            self.scrobbler.love(&ctx, &t, on);
            self.data.mirror_like(t, on);
        }
        self.resolve_play(&ctx);
        crate::gfx::trim_covers(&ctx);
        self.follow_track(&ctx);
        self.palette_toggle(&ctx);
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
        self.island_tick(&ctx);
        self.titlebar(ui, tb);
        if pl_t < 1. {
            self.peek(ui, pl_r, 1. - pl_t);
            self.mini(ui, full, 1. - pl_t);
        }
        // Mini player: 64 tall plus 14 margin; lift toasts clear of it while it's showing.
        crate::toast::show(&ctx, &self.t, (1. - pl_t) * 82.);
        self.context_menu(&ctx);
        self.me_dropdown(&ctx);
        self.palette(&ctx);
        self.delete_modal(&ctx);
    }
}
