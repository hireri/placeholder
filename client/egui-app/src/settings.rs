//! Settings page: a tab bar (with a search button at its right end) above titled sections of rows and blocks (cards), one tab at a time.

use crate::tip::Tip;
use crate::app::{App, VIDEO_HEIGHTS};
use crate::data::State;
use crate::gfx::*;
use crate::theme::PRESETS;
use eframe::egui::{self, pos2, vec2, Align2, Rect, ScrollArea, Sense};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

/// Preferences edited on this page; saved with the rest of the UI prefs.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Opts {
    /// 34px track rows instead of 44px.
    pub compact: bool,
    /// Album and artist headers wash with the cover's colour instead of a flat tone.
    pub tint: bool,
    /// 0 follow the system, 1 light, 2 dark.
    pub mode: u8,
    /// Colours from `theme.toml` override the generated theme.
    pub custom: bool,
    /// Theme follows the playing cover; off uses `hue` always.
    pub auto_theme: bool,
    /// Index into `PRESETS`; used when `auto_theme` is off or nothing is playing.
    pub accent: u8,
    /// Whole seconds, 0..=12; 0 is off.
    pub crossfade: u8,
    pub gapless: bool,
    /// Defaults the player starts each session with (its own toggles change them for the session only).
    pub endless: bool,
    pub video: bool,
    /// One of `VIDEO_HEIGHTS`.
    pub video_h: u32,
    /// Show the current song as Discord Rich Presence.
    pub discord: bool,
    /// Keep the presence up (marked paused, no timer) while playback is paused.
    pub discord_paused: bool,
    /// In a party: show the listener count and elapsed time instead of a progress bar.
    pub discord_party: bool,
    /// Host: let people in without asking.
    pub party_auto_accept: bool,
    /// Party notifications: join requests and answers to yours; people joining and leaving; being removed or the party ending.
    pub notify_requests: bool,
    pub notify_people: bool,
    pub notify_ended: bool,
    /// Only notify while the window is not in the foreground.
    pub notify_background: bool,
    /// Audio cache limit in MB; 0 turns the cache off.
    pub cache_mb: u16,
    /// Hide explicit songs / music-video versions from lists and search (daemon flags, filtered on arrival).
    pub hide_explicit: bool,
    pub hide_video: bool,
    /// Artist page parts.
    pub show_bio: bool,
    pub show_subs: bool,
    pub show_monthly: bool,
    /// Dark mode only: pure black window and sidebar.
    pub pure_black: bool,
    /// Scale each track toward the loudness target using the loudness YouTube reports.
    pub normalize: bool,
    /// Installed font family used for the UI ahead of Allo; empty keeps Allo.
    pub font: String,
    /// Show a romaji line under Japanese lyrics.
    pub romaji: bool,
    /// Lyrics text size in percent, one of `LYR_SIZES`.
    pub lyrics_size: u8,
    /// Closing the window hides it to the tray (when there is one) instead of quitting.
    pub close_tray: bool,
    /// Seconds an arrow key seeks, and percent Ctrl+arrow changes the volume.
    pub seek_step: u8,
    pub vol_step: u8,
    /// Adding a song that is already queued does nothing.
    pub no_dupes: bool,
    /// Keep recent searches for the command palette.
    pub keep_search: bool,
    /// Interface scale in percent, one of `SCALES`.
    pub ui_scale: u8,
    /// Points one mouse-wheel notch scrolls, one of `SCROLL`.
    pub scroll_speed: u8,
    /// Loudness target in LUFS, one of `LOUD`.
    pub target_lufs: i8,
    /// Restore shuffle and repeat on the next start.
    pub remember_modes: bool,
    /// Sleep timer lowers the volume over its last 30 seconds.
    pub sleep_fade: bool,
    /// Held arrow-key seeks grow by 5 s each press.
    pub progressive_seek: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { compact: false, tint: true, mode: 0, custom: false, auto_theme: true, accent: 0, crossfade: 0, gapless: true, endless: false, video: true, video_h: VIDEO_HEIGHTS[0], discord: true, discord_paused: false, discord_party: true, party_auto_accept: false, notify_requests: true, notify_people: false, notify_ended: true, notify_background: true, cache_mb: 512, hide_explicit: false, hide_video: false, show_bio: true, show_subs: true, show_monthly: true, pure_black: false, normalize: true, font: String::new(), romaji: false, lyrics_size: 100, close_tray: true, seek_step: 5, vol_step: 5, no_dupes: false, keep_search: true, ui_scale: 100, scroll_speed: 100, target_lufs: -14, remember_modes: false, sleep_fade: false, progressive_seek: false }
    }
}

/// The daemon's `GET /settings` (and the reply of `PUT /settings`); the lists are read-only.
#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Daemon {
    audio_quality: String,
    content_language: String,
    content_country: String,
    proxy: Proxy,
    report_history: bool,
    lyrics: LyricsCfg,
    languages: Vec<Named>,
    countries: Vec<Named>,
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct Proxy {
    enabled: bool,
    r#type: String,
    url: String,
    username: String,
    password: String,
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct LyricsCfg {
    enabled: HashMap<String, bool>,
    order: Vec<String>,
}

#[derive(Deserialize, Clone, Default)]
struct Named {
    code: String,
    name: String,
}

/// Which list the Library tab has open.
#[derive(Clone, Copy, PartialEq)]
pub enum Pick {
    Language,
    Country,
}

const SYSTEM: &str = "SYSTEM_DEFAULT";
/// Name of each tab, in bar order.
const TABS: [&str; 7] = ["Appearance", "Playback", "Library", "Storage", "Privacy", "Accounts", "System"];
pub const APPEARANCE: usize = 0;
pub const ACCOUNTS: usize = 5;
/// Shortcuts live on the System tab.
pub const KEYS: usize = 6;
const LIBRARY: usize = 2;
/// Tabs made of plain rows, which the search looks through (Accounts and System are custom layouts).
const SEARCHABLE: usize = 5;
pub const SCALES: [u8; 6] = [80, 90, 100, 110, 125, 150];
const SCROLL: [u8; 3] = [60, 100, 150];
const LYR_SIZES: [u8; 3] = [85, 100, 125];

/// Mirrors the bindings in `App::keys` (plus the palette); each key is its own chip.
const SHORTCUTS: &[(&str, &[&str])] = &[
    ("Play / pause", &["Space"]),
    ("Next track", &["Ctrl", "→"]),
    ("Previous track", &["Ctrl", "←"]),
    ("Seek forward", &["→"]),
    ("Seek back", &["←"]),
    ("Volume up", &["Ctrl", "↑"]),
    ("Volume down", &["Ctrl", "↓"]),
    ("Shuffle", &["Ctrl", "S"]),
    ("Repeat", &["Ctrl", "R"]),
    ("Like song", &["Ctrl", "L"]),
    ("Player: artwork", &["Ctrl", "1"]),
    ("Player: lyrics", &["Ctrl", "2"]),
    ("Player: queue", &["Ctrl", "3"]),
    ("Collapse library", &["Ctrl", "B"]),
    ("Show / hide player", &["Ctrl", "P"]),
    ("Back", &["Alt", "←"]),
    ("Forward", &["Alt", "→"]),
    ("Command palette", &["Ctrl", "K"]),
];

const ROW_H: f32 = 60.;
const BAR_H: f32 = 44.;
/// Room kept free left of a row's text for its control.
const CONTROL_W: f32 = 320.;

/// Layout cursor for one pass over the settings: sections are rounded cards of rows under a small title.
/// While `q` (the lowercased search text) is set, only rows whose title or description contain it are laid out.
struct Page {
    x0: f32,
    x1: f32,
    y: f32,
    q: String,
    /// Tab being drawn, named in section titles while searching.
    tab: &'static str,
    /// Title of the section waiting for its first visible row.
    head: Option<&'static str>,
    /// Background slot and top of the open section card.
    card: Option<(egui::layers::ShapeIdx, f32)>,
    rows: usize,
}

impl Page {
    /// Control edge: where a row's switch, buttons and fields end.
    fn right(&self) -> f32 {
        self.x1 - 20.
    }
}

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let t = self.t;
        let (x0, x1) = (card.min.x + 22., card.max.x - 22.);
        self.tab_bar(ui, Rect::from_min_max(pos2(x0, top), pos2(x1, top + BAR_H)));
        let area = Rect::from_min_max(pos2(x0, top + BAR_H + 12.), pos2(x1, (card.max.y - 8.).max(top + 140.)));
        let (tab, q) = (self.settings_tab.min(TABS.len() - 1), self.settings_q.trim().to_lowercase());
        let mut pui = App::region(ui, area);
        if let (LIBRARY, Some(which), true) = (tab, self.settings_pick, q.is_empty()) {
            pui.painter().rect_filled(area, rad(18.), t.s2);
            return self.picker(&mut pui, area, which);
        }
        ScrollArea::vertical().id_salt(("settings-body", tab, &q)).auto_shrink(false).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut pui, |ui| {
            // Rows are laid out as they are drawn, so the scrollable height is last frame's.
            let hid = ui.scope_id().with(("settings-h", tab, !q.is_empty()));
            let h = ui.data(|d| d.get_temp::<f32>(hid)).unwrap_or(area.height());
            let (_, virt) = ui.allocate_space(vec2(area.width(), h));
            let mut pg = Page { x0: virt.min.x, x1: virt.max.x, y: virt.min.y + 4., q, tab: TABS[tab], head: None, card: None, rows: 0 };
            if pg.q.is_empty() {
                match tab {
                    0 => self.appearance(ui, &mut pg),
                    1 => self.playback(ui, &mut pg),
                    LIBRARY => self.library_tab(ui, &mut pg),
                    3 => self.storage_tab(ui, &mut pg),
                    4 => self.privacy_tab(ui, &mut pg),
                    ACCOUNTS => self.accounts(ui, &mut pg),
                    _ => self.system(ui, &mut pg),
                }
            } else {
                for (i, name) in TABS[..SEARCHABLE].iter().enumerate() {
                    pg.tab = name;
                    match i {
                        0 => self.appearance(ui, &mut pg),
                        1 => self.playback(ui, &mut pg),
                        LIBRARY => self.library_tab(ui, &mut pg),
                        3 => self.storage_tab(ui, &mut pg),
                        _ => self.privacy_tab(ui, &mut pg),
                    }
                }
                if pg.rows == 0 {
                    let msg = format!("No settings match “{}”", self.settings_q.trim());
                    ui.painter().text(pos2(pg.x0 + 6., pg.y + 24.), Align2::LEFT_CENTER, msg, font(13.), t.tm);
                    pg.y += 48.;
                }
            }
            self.close(ui, &mut pg);
            let used = pg.y - virt.min.y + 20.;
            if (used - h).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(hid, used));
                ui.ctx().request_repaint();
            }
        });
    }

    /// Text tabs with an accent underline on the active one and a hairline under the bar; a search button at the right end expands into a field.
    fn tab_bar(&mut self, ui: &mut egui::Ui, bar: Rect) {
        let t = self.t;
        let (sid, oid) = (ui.scope_id().with("settings-search"), ui.scope_id().with("settings-search-open"));
        let mut open = ui.data(|d| d.get_temp::<bool>(oid)).unwrap_or(false) || !self.settings_q.is_empty();
        if open && ui.memory(|m| m.has_focus(sid)) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.settings_q.clear();
            open = false;
            ui.memory_mut(|m| m.surrender_focus(sid));
        }
        let k = ui.ctx().animate_bool_with_time_and_easing(oid, open, 0.18, ease_out);
        let sw = 36. + 204. * k;
        let search = Rect::from_min_size(pos2(bar.max.x - sw, bar.center().y - 18.), vec2(sw, 36.));
        let p = ui.painter().clone();
        p.hline(bar.x_range(), bar.max.y, egui::Stroke::new(1., t.s2));
        let tabs = p.with_clip_rect(Rect::from_min_max(bar.min, pos2(search.min.x - 8., bar.max.y + 1.)));
        let searching = !self.settings_q.trim().is_empty();
        let mut x = bar.min.x;
        for (i, name) in TABS.iter().enumerate() {
            let on = self.settings_tab == i && !searching;
            let tw = text_w(&p, name, font_w(13., Weight::Semi));
            let r = Rect::from_min_size(pos2(x, bar.min.y), vec2(tw + 22., BAR_H));
            let hit = press(ui, ("settings-tab", i), r.intersect(tabs.clip_rect()));
            let a = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-tab-on", i)), on, 0.18, ease_out);
            let h = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-tab-hot", i)), hit.hovered(), 0.12, ease_out);
            let c = t.tm.lerp_to_gamma(t.tx, a.max(h * 0.6));
            let cx = r.min.x + tw / 2.;
            tabs.text(pos2(r.min.x, bar.center().y), Align2::LEFT_CENTER, name, font_w(13., if on { Weight::Semi } else { Weight::Medium }), c);
            if a > 0. {
                tabs.rect_filled(Rect::from_min_max(pos2(cx - tw / 2. * a, bar.max.y - 2.), pos2(cx + tw / 2. * a, bar.max.y)), rad(1.), t.ax);
            }
            if hit.clicked() {
                (self.settings_tab, self.settings_q, open) = (i, String::new(), false);
                self.settings_pick = None;
            }
            x += tw + 22.;
        }
        if open {
            p.rect_filled(search, rad(18.), t.s0);
            if ui.memory(|m| m.has_focus(sid)) {
                p.rect_stroke(search, rad(18.), egui::Stroke::new(2., t.ax), egui::StrokeKind::Inside);
            }
            icon(ui, icon!("search"), pos2(search.min.x + 18., search.center().y), 16., t.tm);
            if k > 0.95 {
                let edit = Rect::from_min_max(pos2(search.min.x + 36., search.min.y), pos2(search.max.x - 36., search.max.y));
                let hint = egui::RichText::new("Search settings").font(font(13.)).color(t.tf);
                ui.put(edit, egui::TextEdit::singleline(&mut self.settings_q).id(sid).frame(egui::Frame::NONE).font(font(13.)).hint_text(hint).align(Align2::LEFT_CENTER).desired_width(edit.width()));
            }
            let b = Rect::from_center_size(pos2(search.max.x - 18., search.center().y), vec2(24., 24.));
            let hit = press(ui, "settings-search-clear", b);
            icon(ui, icon!("x"), b.center(), 14., if hit.hovered() { t.tx } else { t.tm });
            if hit.clicked() {
                (self.settings_q, open) = (String::new(), false);
            }
        } else {
            let hit = press(ui, "settings-search-open", search);
            p.rect_filled(search, rad(18.), t.s0.gamma_multiply(if hit.hovered() { 1. } else { 0. }));
            icon(ui, icon!("search"), search.center(), 18., if hit.hovered() { t.tx } else { t.tm });
            hit.clone().tip(&t, "Search settings");
            if hit.clicked() {
                open = true;
                ui.memory_mut(|m| m.request_focus(sid));
            }
        }
        ui.data_mut(|d| d.insert_temp(oid, open));
    }

    /// Fetches the daemon's settings once per visit; `Err` is what to show while they are not there.
    fn daemon(&self, ui: &egui::Ui) -> Result<Arc<Daemon>, String> {
        self.data.settings.ensure(ui.ctx(), || crate::backend::get("/settings"));
        match &*self.data.settings.lock() {
            State::Ready(d) => Ok(d.clone()),
            State::Failed(e) => Err(format!("Couldn't reach the daemon: {e}")),
            _ => Err("Connecting to the daemon…".into()),
        }
    }

    /// Applies `edit` to the loaded settings at once and sends only `body` (the changed field) to the daemon;
    /// the reply replaces the settings, a refusal refetches them.
    fn put(&self, ui: &egui::Ui, body: serde_json::Value, edit: impl FnOnce(&mut Daemon)) {
        if let State::Ready(d) = &mut *self.data.settings.lock() {
            edit(Arc::make_mut(d));
        }
        let (data, ctx) = (self.data.clone(), ui.ctx().clone());
        std::thread::spawn(move || {
            match crate::backend::put_json::<_, Daemon>("/settings", &body) {
                Ok(v) => *data.settings.lock() = State::Ready(Arc::new(v)),
                Err(e) => {
                    crate::toast::error(format!("Couldn't save the setting: {e}"));
                    data.settings.reset();
                }
            }
            ctx.request_repaint();
        });
    }

    /// Starts a section: its title is drawn with its first row that survives the search.
    fn sec(&self, ui: &egui::Ui, pg: &mut Page, title: &'static str) {
        self.close(ui, pg);
        pg.head = Some(title);
    }

    /// Fills in the open section's card behind its rows.
    fn close(&self, ui: &egui::Ui, pg: &mut Page) {
        if let Some((slot, y0)) = pg.card.take() {
            ui.painter().set(slot, egui::Shape::rect_filled(Rect::from_min_max(pos2(pg.x0, y0), pos2(pg.x1, pg.y)), rad(16.), self.t.s2));
            pg.y += 22.;
        }
    }

    /// Draws the pending section title.
    fn head(&self, ui: &egui::Ui, pg: &mut Page) {
        let Some(h) = pg.head.take() else { return };
        let t = self.t;
        let label = if pg.q.is_empty() { h.to_string() } else { format!("{} · {h}", pg.tab) };
        ui.painter().text(pos2(pg.x0 + 4., pg.y + 9.), Align2::LEFT_CENTER, label, font_w(12., Weight::Semi), t.tm);
        pg.y += 26.;
    }

    /// Opens the card (and draws the section title) for the first row of a section.
    fn begin_card(&self, ui: &egui::Ui, pg: &mut Page) -> bool {
        if pg.card.is_some() {
            return false;
        }
        self.head(ui, pg);
        pg.card = Some((ui.painter().add(egui::Shape::Noop), pg.y));
        true
    }

    fn visible(pg: &Page, title: &str, desc: &str) -> bool {
        pg.q.is_empty() || title.to_lowercase().contains(&pg.q) || desc.to_lowercase().contains(&pg.q)
    }

    /// A card of its own with a title row and `body_h` of custom content under it; returns the card and the body rect,
    /// or `None` when the search hides it.
    fn block(&self, ui: &egui::Ui, pg: &mut Page, title: &str, desc: &str, body_h: f32) -> Option<(Rect, Rect)> {
        if !Self::visible(pg, title, desc) {
            return None;
        }
        self.close(ui, pg);
        self.head(ui, pg);
        let (p, t) = (ui.painter(), self.t);
        let card = Rect::from_min_size(pos2(pg.x0, pg.y), vec2(pg.x1 - pg.x0, ROW_H + body_h + 8.));
        p.rect_filled(card, rad(16.), t.s2);
        let w = (card.width() - 40. - 120.).max(120.);
        trunc(p, pos2(card.min.x + 20., card.min.y + 23.), w, title, font(13.), t.tx);
        trunc(p, pos2(card.min.x + 20., card.min.y + 43.), w, desc, font(12.), t.tm);
        pg.rows += 1;
        pg.y = card.max.y + 22.;
        Some((card, Rect::from_min_max(pos2(card.min.x + 20., card.min.y + ROW_H), pos2(card.max.x - 20., card.max.y - 8.))))
    }

    /// One setting: title and description on the left; returns the row's vertical centre for the caller's control,
    /// or `None` when the search hides it.
    fn row(&self, ui: &egui::Ui, pg: &mut Page, title: &str, desc: &str) -> Option<f32> {
        if !Self::visible(pg, title, desc) {
            return None;
        }
        let (p, t) = (ui.painter(), self.t);
        if !self.begin_card(ui, pg) {
            p.hline((pg.x0 + 20.)..=(pg.x1 - 20.), pg.y, egui::Stroke::new(1., t.s3.gamma_multiply(0.6)));
        }
        let w = (pg.x1 - pg.x0 - 40. - CONTROL_W).max(120.);
        let (ty, dy) = if desc.is_empty() { (pg.y + ROW_H / 2., 0.) } else { (pg.y + 23., pg.y + 43.) };
        trunc(p, pos2(pg.x0 + 20., ty), w, title, font(13.), t.tx);
        if dy > 0. {
            trunc(p, pos2(pg.x0 + 20., dy), w, desc, font(12.), t.tm);
        }
        pg.rows += 1;
        pg.y += ROW_H;
        Some(pg.y - ROW_H / 2.)
    }

    /// A line of plain text in a card of its own (daemon status and the like); hidden while searching.
    fn info(&self, ui: &egui::Ui, pg: &mut Page, text: &str) {
        if pg.q.is_empty() {
            self.close(ui, pg);
            trunc(ui.painter(), pos2(pg.x0 + 6., pg.y + 22.), pg.x1 - pg.x0 - 12., text, font(13.), self.t.tf);
            pg.y += 44.;
        }
    }

    /// Dims the row centred on `cy` while `why` is set and, over it, blocks the controls and says why on hover.
    fn gate(&self, ui: &egui::Ui, pg: &Page, cy: f32, key: &str, why: Option<&str>) {
        let id = ui.scope_id().with(("settings-gate", key));
        let k = ui.ctx().animate_bool_with_time_and_easing(id, why.is_some(), 0.2, ease_out);
        let r = Rect::from_center_size(pos2((pg.x0 + pg.x1) / 2., cy), vec2(pg.x1 - pg.x0, ROW_H));
        self.gate_rect(ui, r, id, k, why);
    }

    fn gate_rect(&self, ui: &egui::Ui, r: Rect, id: egui::Id, k: f32, why: Option<&str>) {
        ui.painter().rect_filled(r, 0., self.t.s1.gamma_multiply(0.55 * k));
        if let Some(why) = why {
            let hit = ui.interact(r, id, Sense::click());
            if hit.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
            }
            hit.tip(&self.t, why);
        }
    }

    fn appearance(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        self.sec(ui, pg, "Theme");
        if let Some((_, body)) = self.block(ui, pg, "Mode", "Light, dark, or follow the system", 100.) {
            self.mode_tiles(ui, body);
        }
        if let Some(cy) = self.row(ui, pg, "Theme from artwork", "Colour the app from the playing cover") {
            toggle(ui, &self.t, "auto-theme", right, cy, &mut self.opts.auto_theme);
        }
        let cols = (((pg.x1 - pg.x0 - 40.) / 40.).floor() as usize).clamp(1, PRESETS.len());
        if let Some((card, body)) = self.block(ui, pg, "Accent colour", "Preset colour theme used across the app", PRESETS.len().div_ceil(cols) as f32 * 40.) {
            for (i, (name, ..)) in PRESETS.iter().enumerate() {
                let c = pos2(body.min.x + 18. + (i % cols) as f32 * 40., body.min.y + 18. + (i / cols) as f32 * 40.);
                let hit = press(ui, ("accent", i), Rect::from_center_size(c, vec2(36., 36.)));
                let k = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("accent-on", i)), self.opts.accent as usize == i, 0.18, ease_out);
                let p = ui.painter();
                p.circle_filled(c, 13., crate::theme::Theme::preset(i as u8, true).ax);
                p.circle_stroke(c, 17., egui::Stroke::new(2., self.t.tx.gamma_multiply(k)));
                if hit.clicked() {
                    self.opts.accent = i as u8;
                }
                hit.tip(&self.t, name);
            }
            let playing = self.pl.song().is_some();
            let why = (self.opts.auto_theme && playing).then_some("Turn off “Theme from artwork” to pick an accent while a song is playing");
            let id = ui.scope_id().with(("settings-gate", "accent"));
            let k = ui.ctx().animate_bool_with_time_and_easing(id, why.is_some(), 0.2, ease_out);
            self.gate_rect(ui, card.shrink2(vec2(0., 1.)), id, k, why);
        }
        if let Some(cy) = self.row(ui, pg, "Custom theme", "theme.toml in config folder; reloads on open") {
            if toggle(ui, &self.t, "custom", right, cy, &mut self.opts.custom) {
                self.reload_custom();
            }
        }
        if let Some(cy) = self.row(ui, pg, "Pure black", "True black surfaces in dark mode, for OLED") {
            toggle(ui, &self.t, "pure-black", right, cy, &mut self.opts.pure_black);
        }
        self.sec(ui, pg, "Display");
        if let Some(cy) = self.row(ui, pg, "Interface scale", "Size of text and controls") {
            let labels = SCALES.map(|v| format!("{v}%"));
            let cur = SCALES.iter().position(|v| *v == self.opts.ui_scale).unwrap_or(2);
            let sel = segmented(ui, &self.t, "ui-scale", right, cy, &labels.each_ref().map(String::as_str), cur);
            if SCALES[sel] != self.opts.ui_scale {
                self.opts.ui_scale = SCALES[sel];
                set_scale(ui.ctx(), self.opts.ui_scale);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Scroll speed", "Per mouse-wheel notch; touchpads unaffected") {
            let cur = SCROLL.iter().position(|v| *v == self.opts.scroll_speed).unwrap_or(1);
            let sel = segmented(ui, &self.t, "scroll-speed", right, cy, &["Slow", "Normal", "Fast"], cur);
            if SCROLL[sel] != self.opts.scroll_speed {
                self.opts.scroll_speed = SCROLL[sel];
                set_scroll_speed(ui.ctx(), self.opts.scroll_speed);
            }
        }
        self.sec(ui, pg, "Layout");
        if let Some(cy) = self.row(ui, pg, "Row density", "Height of track rows") {
            let sel = segmented(ui, &self.t, "density", right, cy, &["Comfortable", "Compact"], self.opts.compact as usize);
            self.opts.compact = sel == 1;
        }
        if let Some(cy) = self.row(ui, pg, "Tint headers from artwork", "Album and artist headers") {
            toggle(ui, &self.t, "tint", right, cy, &mut self.opts.tint);
        }
        if let Some(cy) = self.row(ui, pg, "Font", "Installed font family; empty uses Allo") {
            let id = ui.scope_id().with("font-buf");
            let mut buf = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| self.opts.font.clone());
            if text_field(ui, &self.t, "font", right, cy, &mut buf, false) && buf.trim() != self.opts.font {
                if set_fonts(ui.ctx(), &buf) {
                    self.opts.font = buf.trim().to_string();
                } else {
                    crate::toast::error(format!("No installed font named “{}”", buf.trim()));
                    set_fonts(ui.ctx(), &self.opts.font);
                    buf = self.opts.font.clone();
                }
            }
            ui.data_mut(|d| d.insert_temp(id, buf));
        }
        self.sec(ui, pg, "Window");
        if let Some(cy) = self.row(ui, pg, "Keep playing in the tray", "Close hides to tray; off quits and stops music") {
            toggle(ui, &self.t, "close-tray", right, cy, &mut self.opts.close_tray);
        }
    }

    /// System / light / dark as three small window previews in the current accent.
    fn mode_tiles(&mut self, ui: &egui::Ui, body: Rect) {
        let t = self.t;
        for (i, name) in ["System", "Light", "Dark"].into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(body.min.x + i as f32 * 132., body.min.y), vec2(120., 72.));
            let hit = press(ui, ("mode", i), r);
            let k = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("mode-on", i)), self.opts.mode as usize == i, 0.18, ease_out);
            let p = ui.painter();
            let draw = |p: &egui::Painter, dark: bool| {
                let th = crate::theme::Theme::preset(self.opts.accent, dark);
                p.rect_filled(r, rad(12.), th.s0);
                p.rect_filled(Rect::from_min_size(r.min + vec2(8., 8.), vec2(24., r.height() - 16.)), rad(6.), th.s2);
                for (j, w) in [52., 36., 44.].into_iter().enumerate() {
                    p.rect_filled(Rect::from_min_size(r.min + vec2(40., 12. + j as f32 * 14.), vec2(w, 8.)), rad(4.), if j == 0 { th.ax } else { th.s3 });
                }
            };
            match i {
                0 => {
                    let (l, rt) = (Rect::from_min_max(r.min, pos2(r.center().x, r.max.y)), Rect::from_min_max(pos2(r.center().x, r.min.y), r.max));
                    draw(&p.with_clip_rect(l), true);
                    draw(&p.with_clip_rect(rt), false);
                }
                _ => draw(p, i == 2),
            }
            p.rect_stroke(r, rad(12.), egui::Stroke::new(2., t.tf.gamma_multiply(0.4).lerp_to_gamma(t.ax, k.max(if hit.hovered() { 0.5 } else { 0. }))), egui::StrokeKind::Outside);
            p.text(pos2(r.min.x + 2., r.max.y + 14.), Align2::LEFT_CENTER, name, font_w(12., Weight::Medium), t.tm.lerp_to_gamma(t.tx, k));
            if hit.clicked() {
                self.opts.mode = i as u8;
            }
        }
    }

    fn playback(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        let d = self.daemon(ui);
        let offline = d.as_ref().err().map(|_| "Needs the daemon, which isn't connected");
        self.sec(ui, pg, "Audio");
        if let Some(cy) = self.row(ui, pg, "Audio quality", "Stream quality for songs") {
            const Q: [&str; 3] = ["AUTO", "LOW", "HIGH"];
            let cur = d.as_ref().ok().and_then(|d| Q.iter().position(|q| *q == d.audio_quality)).unwrap_or(0);
            let sel = segmented(ui, &self.t, "aq", right, cy, &["Auto", "Low", "High"], cur);
            if let (Ok(_), true) = (&d, sel != cur) {
                self.put(ui, json!({ "audioQuality": Q[sel] }), |d| d.audio_quality = Q[sel].into());
            }
            self.gate(ui, pg, cy, "aq", offline);
        }
        if let Some(cy) = self.row(ui, pg, "Normalize volume", "Evens out loud and quiet songs") {
            if toggle(ui, &self.t, "normalize", right, cy, &mut self.opts.normalize) {
                self.pl.engine.set_normalize(self.opts.normalize);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Loudness target", "Targets -19, -14 (default), -11 LUFS") {
            let cur = LOUD.iter().position(|v| *v == self.opts.target_lufs).unwrap_or(1);
            let sel = segmented(ui, &self.t, "loudness", right, cy, &["Quiet", "Balanced", "Loud"], cur);
            if LOUD[sel] != self.opts.target_lufs {
                self.opts.target_lufs = LOUD[sel];
                self.pl.engine.set_target(LOUD[sel]);
            }
        }
        if let Some((card, body)) = self.block(ui, pg, "Crossfade", "Overlap the end of a song with the start of the next", 130.) {
            let label = if self.opts.crossfade == 0 { "Off".into() } else { format!("{} s", self.opts.crossfade) };
            pill(ui.painter(), &self.t, card.max.x - 20., card.min.y + 33., &label, self.opts.crossfade > 0);
            crossfade_graph(ui, &self.t, Rect::from_min_size(body.min, vec2(body.width(), 70.)), self.opts.crossfade);
            let cy = body.min.y + 90.;
            if slider(ui, &self.t, "crossfade", body.min.x, body.max.x, cy, &mut self.opts.crossfade) {
                self.pl.engine.set_crossfade(self.opts.crossfade as f32);
            }
            for s in (0..=12).step_by(3) {
                let (x, lbl) = (body.min.x + 8. + (body.width() - 16.) * s as f32 / 12., if s == 0 { "Off".into() } else { format!("{s} s") });
                ui.painter().text(pos2(x, cy + 26.), Align2::CENTER_CENTER, lbl, font(11.), self.t.tf);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Gapless playback", "No pause between consecutive tracks") {
            if toggle(ui, &self.t, "gapless", right, cy, &mut self.opts.gapless) {
                self.pl.engine.set_gapless(self.opts.gapless);
            }
        }
        self.sec(ui, pg, "Queue");
        if let Some(cy) = self.row(ui, pg, "Start with Endless on", "Adds similar songs when the queue runs out") {
            toggle(ui, &self.t, "endless", right, cy, &mut self.opts.endless);
        }
        if let Some(cy) = self.row(ui, pg, "Prevent duplicate songs", "Adding a queued song does nothing") {
            if toggle(ui, &self.t, "no-dupes", right, cy, &mut self.opts.no_dupes) {
                self.pl.no_dupes = self.opts.no_dupes;
            }
        }
        if let Some(cy) = self.row(ui, pg, "Remember shuffle and repeat", "Restored in the next session") {
            toggle(ui, &self.t, "remember-modes", right, cy, &mut self.opts.remember_modes);
        }
        self.sec(ui, pg, "Video");
        if let Some(cy) = self.row(ui, pg, "Play music videos by default", "Video instead of cover when available") {
            toggle(ui, &self.t, "video", right, cy, &mut self.opts.video);
        }
        if let Some(cy) = self.row(ui, pg, "Default video quality", "Max height of music videos") {
            let labels = VIDEO_HEIGHTS.map(|h| format!("{h}p"));
            let cur = VIDEO_HEIGHTS.iter().position(|h| *h == self.opts.video_h).unwrap_or(0);
            let sel = segmented(ui, &self.t, "video-h", right, cy, &labels.each_ref().map(String::as_str), cur);
            self.opts.video_h = VIDEO_HEIGHTS[sel];
        }
        self.sec(ui, pg, "Controls");
        if let Some(cy) = self.row(ui, pg, "Seek step", "Left and right arrow jump distance") {
            let cur = SEEK.iter().position(|s| *s == self.opts.seek_step).unwrap_or(0);
            let d = stepper(ui, &self.t, "seek-step", right, cy, &format!("{} s", SEEK[cur]), cur > 0, cur + 1 < SEEK.len());
            self.opts.seek_step = SEEK[(cur as i32 + d) as usize];
        }
        if let Some(cy) = self.row(ui, pg, "Progressive seek", "Repeated seeks add 5 s each, up to 60 s") {
            toggle(ui, &self.t, "progressive-seek", right, cy, &mut self.opts.progressive_seek);
        }
        if let Some(cy) = self.row(ui, pg, "Volume step", "Ctrl with up and down arrows changes volume") {
            let cur = VOL.iter().position(|s| *s == self.opts.vol_step).unwrap_or(0);
            let d = stepper(ui, &self.t, "vol-step", right, cy, &format!("{}%", VOL[cur]), cur > 0, cur + 1 < VOL.len());
            self.opts.vol_step = VOL[(cur as i32 + d) as usize];
        }
        if let Some(cy) = self.row(ui, pg, "Fade out before sleep", "Lowers volume over the last 30 s of timer") {
            toggle(ui, &self.t, "sleep-fade", right, cy, &mut self.opts.sleep_fade);
        }
        if let Some(cy) = self.row(ui, pg, "Sleep timer", &self.sleep.describe()) {
            const SLEEP: [&str; 5] = ["Off", "15 min", "30 min", "60 min", "End of song"];
            let sel = segmented(ui, &self.t, "sleep", right, cy, &SLEEP, self.sleep.slot());
            if sel != self.sleep.slot() {
                self.sleep = match (sel, self.pl.song()) {
                    (1..=3, _) => crate::sleep::Sleep::minutes([15, 30, 60][sel - 1]),
                    (4, Some(s)) => crate::sleep::Sleep::EndOfSong(s.title.clone(), s.artist.clone()),
                    _ => Default::default(),
                };
            }
        }
    }

    fn content(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        self.sec(ui, pg, "Filters");
        let mut changed = false;
        for (key, title, desc) in [
            ("hide-explicit", "Hide explicit content", "Hides explicit songs in lists and search"),
            ("hide-video", "Hide video songs", "Hides music videos in lists and search"),
        ] {
            if let Some(cy) = self.row(ui, pg, title, desc) {
                let v = if key == "hide-explicit" { &mut self.opts.hide_explicit } else { &mut self.opts.hide_video };
                changed |= toggle(ui, &self.t, key, right, cy, v);
            }
        }
        if changed {
            crate::data::HIDE_EXPLICIT.store(self.opts.hide_explicit, std::sync::atomic::Ordering::Relaxed);
            crate::data::HIDE_VIDEO.store(self.opts.hide_video, std::sync::atomic::Ordering::Relaxed);
            self.data.refilter();
        }
        self.sec(ui, pg, "Artist page");
        for (key, title, desc) in [
            ("show-bio", "Artist description", "Biography on artist pages"),
            ("show-subs", "Subscriber count", ""),
            ("show-monthly", "Monthly listeners", ""),
        ] {
            if let Some(cy) = self.row(ui, pg, title, desc) {
                let v = match key { "show-bio" => &mut self.opts.show_bio, "show-subs" => &mut self.opts.show_subs, _ => &mut self.opts.show_monthly };
                toggle(ui, &self.t, key, right, cy, v);
            }
        }
    }

    /// Language, country and proxy: everything that needs the daemon's settings.
    fn network(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        let Ok(d) = self.daemon(ui) else { return };
        self.sec(ui, pg, "Region");
        let name = |list: &[Named], code: &str| if code == SYSTEM { "System default".into() } else { list.iter().find(|n| n.code == code).map_or(code.to_string(), |n| n.name.clone()) };
        for (i, (which, title, desc, cur, list)) in [
            (Pick::Language, "Content language", "Language for titles and shelves", &d.content_language, &d.languages),
            (Pick::Country, "Content country", "Region for music recommendations", &d.content_country, &d.countries),
        ].into_iter().enumerate() {
            let Some(cy) = self.row(ui, pg, title, desc) else { continue };
            if picker_pill(ui, &self.t, ("pick", i), right, cy, &name(list, cur)) {
                (self.settings_pick, self.settings_tab, self.settings_q) = (Some(which), LIBRARY, String::new());
            }
        }
        self.sec(ui, pg, "Proxy");
        if let Some(cy) = self.row(ui, pg, "Use a proxy", "Send daemon requests through a proxy") {
            let mut on = d.proxy.enabled;
            if toggle(ui, &self.t, "proxy", right, cy, &mut on) {
                self.put(ui, json!({ "proxy": { "enabled": on } }), |d| d.proxy.enabled = on);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Proxy type", "") {
            const KINDS: [&str; 2] = ["HTTP", "SOCKS"];
            let cur = KINDS.iter().position(|k| *k == d.proxy.r#type).unwrap_or(0);
            let sel = segmented(ui, &self.t, "proxy-type", right, cy, &KINDS, cur);
            if sel != cur {
                self.put(ui, json!({ "proxy": { "type": KINDS[sel] } }), |d| d.proxy.r#type = KINDS[sel].into());
            }
        }
        let mut buf = self.proxy_buf.take().unwrap_or_else(|| [d.proxy.url.clone(), d.proxy.username.clone(), d.proxy.password.clone()]);
        for (i, (key, title, desc, field, pw)) in [("url", "Proxy server", "host:port", &d.proxy.url, false), ("username", "Proxy username", "Leave empty if the proxy needs none", &d.proxy.username, false), ("password", "Proxy password", "", &d.proxy.password, true)].into_iter().enumerate() {
            let Some(cy) = self.row(ui, pg, title, desc) else { continue };
            if text_field(ui, &self.t, key, right, cy, &mut buf[i], pw) && buf[i] != *field {
                let v = buf[i].clone();
                self.put(ui, json!({ "proxy": { key: v } }), |d| match i {
                    0 => d.proxy.url = v,
                    1 => d.proxy.username = v,
                    _ => d.proxy.password = v,
                });
            }
        }
        self.proxy_buf = Some(buf);
    }

    /// Content filters and artist page, lyrics, then the daemon-backed region and proxy.
    fn library_tab(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        self.content(ui, pg);
        self.lyrics(ui, pg);
        self.network(ui, pg);
        if let Err(e) = self.daemon(ui) {
            self.info(ui, pg, &e);
        }
    }

    fn lyrics(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        self.sec(ui, pg, "Lyrics");
        if let Some(cy) = self.row(ui, pg, "Romanize Japanese", "Latin reading under Japanese lyrics") {
            if toggle(ui, &self.t, "romaji", right, cy, &mut self.opts.romaji) {
                self.pl.set_romaji(self.opts.romaji);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Lyrics size", "Text size in the player") {
            let cur = LYR_SIZES.iter().position(|v| *v == self.opts.lyrics_size).unwrap_or(1);
            let sel = segmented(ui, &self.t, "lyrics-size", right, cy, &["Small", "Medium", "Large"], cur);
            self.opts.lyrics_size = LYR_SIZES[sel];
            self.pl.lyr_size = self.opts.lyrics_size as f32 / 100.;
        }
        let Ok(d) = self.daemon(ui) else { return };
        self.sec(ui, pg, "Lyric providers");
        let (t, n) = (self.t, d.lyrics.order.len());
        for (i, name) in d.lyrics.order.iter().enumerate() {
            let Some(cy) = self.row(ui, pg, name, &format!("Priority {}", i + 1)) else { continue };
            let mut on = d.lyrics.enabled.get(name).copied().unwrap_or(true);
            if toggle(ui, &t, name, right, cy, &mut on) {
                self.put(ui, json!({ "lyrics": { "enabled": { name: on } } }), |d| {
                    d.lyrics.enabled.insert(name.clone(), on);
                });
            }
            for (k, (glyph, to)) in [("↑", i.checked_sub(1)), ("↓", (i + 1 < n).then_some(i + 1))].into_iter().enumerate() {
                let r = Rect::from_center_size(pos2(right - 40. - 12. - 14. - (1 - k) as f32 * 32., cy), vec2(28., 28.));
                let hit = ui.interact(r, ui.scope_id().with(("lyr-move", i, k)), Sense::click());
                let p = ui.painter();
                if to.is_some() && hit.hovered() {
                    p.rect_filled(r, rad(8.), t.s3);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                p.text(r.center(), Align2::CENTER_CENTER, glyph, font_w(14., Weight::Medium), if to.is_some() { t.tx } else { t.tf });
                if let (true, Some(to)) = (hit.clicked(), to) {
                    let mut order = d.lyrics.order.clone();
                    order.swap(i, to);
                    self.put(ui, json!({ "lyrics": { "order": order } }), |d| d.lyrics.order = order.clone());
                }
            }
        }
    }

    /// Private session and history.
    fn privacy_tab(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        let d = self.daemon(ui);
        let offline = d.as_ref().err().map(|_| "Needs the daemon, which isn't connected");
        self.sec(ui, pg, "History");
        if let Some(cy) = self.row(ui, pg, "Private session", "Pauses Discord, YouTube, Last.fm; ends after 6h idle") {
            let mut on = self.private.is_some();
            if toggle(ui, &self.t, "private", right, cy, &mut on) {
                self.private = on.then(std::time::Instant::now);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Send listening history to YouTube", "Plays appear in your YouTube Music history") {
            let mut on = d.as_ref().map_or(false, |d| d.report_history);
            if toggle(ui, &self.t, "report-history", right, cy, &mut on) && d.is_ok() {
                self.put(ui, json!({ "reportHistory": on }), |d| d.report_history = on);
            }
            self.gate(ui, pg, cy, "report-history", offline);
        }
        if let Some(cy) = self.row(ui, pg, "Remember searches", "Recent searches in the command palette") {
            if toggle(ui, &self.t, "keep-search", right, cy, &mut self.opts.keep_search) && !self.opts.keep_search {
                self.recents.clear();
            }
        }
        if let Some(cy) = self.row(ui, pg, "Clear search history", &format!("{} recent search{} saved", self.recents.len(), if self.recents.len() == 1 { "" } else { "es" })) {
            if button(ui, &self.t, "clear-search", right, cy, "Clear") {
                self.recents.clear();
                crate::toast::info("Search history cleared");
            }
        }
    }

    /// Audio cache and the files the app keeps on disk.
    fn storage_tab(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        self.sec(ui, pg, "Cache");
        let used = crate::cache::usage() as f64 / (1 << 20) as f64;
        let desc = if self.opts.cache_mb == 0 { format!("Off · {used:.0} MB still on disk") } else { format!("{used:.0} MB of {} MB used", self.opts.cache_mb) };
        if let Some((card, body)) = self.block(ui, pg, "Audio cache", &desc, 20.) {
            let frac = if self.opts.cache_mb == 0 { 0. } else { (used / self.opts.cache_mb as f64) as f32 };
            pill(ui.painter(), &self.t, card.max.x - 20., card.min.y + 33., &format!("{:.0}%", frac.min(1.) * 100.), self.opts.cache_mb > 0);
            meter(ui.painter(), &self.t, Rect::from_min_size(pos2(body.min.x, body.min.y + 4.), vec2(body.width(), 10.)), frac);
        }
        if let Some(cy) = self.row(ui, pg, "Cache limit", "Maximum size the cache can grow to") {
            const CACHE: [u16; 5] = [0, 256, 512, 1024, 2048];
            let labels = ["Off", "256 MB", "512 MB", "1 GB", "2 GB"];
            let sel = segmented(ui, &self.t, "cache-mb", right, cy, &labels, CACHE.iter().position(|m| *m == self.opts.cache_mb).unwrap_or(usize::MAX));
            if CACHE.get(sel).is_some_and(|m| *m != self.opts.cache_mb) {
                self.opts.cache_mb = CACHE[sel];
                crate::cache::set_limit_mb(self.opts.cache_mb);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Clear audio cache", "Deletes all cached songs from disk") {
            if button(ui, &self.t, "cache-clear", right, cy, "Clear") {
                crate::cache::clear();
                crate::toast::info("Audio cache cleared");
            }
        }
        self.sec(ui, pg, "Files");
        for (key, title, path) in [("open-data", "Data folder", crate::backend::data_dir()), ("open-log", "Log file", crate::backend::log_path())] {
            if let Some(cy) = self.row(ui, pg, title, &path.display().to_string()) {
                if button(ui, &self.t, key, right, cy, "Open") {
                    ui.ctx().open_url(egui::OpenUrl::same_tab(format!("file://{}", path.display())));
                }
            }
        }
        self.backup(ui, pg);
    }

    fn backup_path() -> std::path::PathBuf {
        crate::backend::data_dir().join("serafin-preferences.json")
    }

    /// Writes `opts` to the data folder, or reads them back and applies them like startup does.
    fn backup(&mut self, ui: &egui::Ui, pg: &mut Page) {
        let right = pg.right();
        let path = Self::backup_path();
        self.sec(ui, pg, "Backup");
        if let Some(cy) = self.row(ui, pg, "Export preferences", &path.display().to_string()) {
            if button(ui, &self.t, "export-opts", right, cy, "Export") {
                match serde_json::to_string_pretty(&self.opts).map_err(|e| e.to_string()).and_then(|j| std::fs::write(&path, j).map_err(|e| e.to_string())) {
                    Ok(()) => crate::toast::info(format!("Preferences saved to {}", path.display())),
                    Err(e) => crate::toast::error(format!("Can't export preferences: {e}")),
                }
            }
        }
        if let Some(cy) = self.row(ui, pg, "Import preferences", "Replaces all settings with the exported file") {
            if button(ui, &self.t, "import-opts", right, cy, "Import") {
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|j| serde_json::from_str::<Opts>(&j).map_err(|e| e.to_string())) {
                    Ok(o) => {
                        self.apply_opts(ui.ctx(), o);
                        crate::toast::info("Preferences imported");
                    }
                    Err(e) => crate::toast::error(format!("Can't import preferences: {e}")),
                }
            }
        }
    }

    /// Full-panel list for the Library tab's language or country; a pick closes it.
    fn picker(&mut self, ui: &mut egui::Ui, panel: Rect, which: Pick) {
        let Ok(d) = self.daemon(ui) else { return self.settings_pick = None };
        let (t, title, list, cur) = match which {
            Pick::Language => (self.t, "Content language", &d.languages, &d.content_language),
            Pick::Country => (self.t, "Content country", &d.countries, &d.content_country),
        };
        let b = Rect::from_center_size(pos2(panel.min.x + 36., panel.min.y + 28.), vec2(32., 32.));
        let hit = press(ui, "pick-back", b);
        if hit.hovered() {
            ui.painter().rect_filled(b, rad(8.), t.s3);
        }
        icon(ui, icon!("back"), b.center(), 18., t.tx);
        trunc(ui.painter(), pos2(b.max.x + 8., panel.min.y + 28.), panel.width() - 100., title, font_w(15., Weight::Semi), t.tx);
        if hit.clicked() {
            return self.settings_pick = None;
        }
        let sys = Named { code: SYSTEM.into(), name: "System default".into() };
        let rows = std::iter::once(&sys).chain(list.iter().filter(|n| n.code != SYSTEM)).collect::<Vec<_>>();
        let area = Rect::from_min_max(pos2(panel.min.x + 12., panel.min.y + 56.), pos2(panel.max.x - 12., panel.max.y - 8.));
        let mut lui = App::region(ui, area);
        lui.spacing_mut().item_spacing = vec2(0., 0.);
        let mut picked = None;
        ScrollArea::vertical().id_salt("settings-pick").auto_shrink(false).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show_rows(&mut lui, 36., rows.len(), |ui, range| {
            for n in &rows[range] {
                let (r, resp) = ui.allocate_exact_size(vec2(area.width(), 36.), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, rad(8.), t.s3);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let on = n.code == *cur;
                trunc(ui.painter(), pos2(r.min.x + 12., r.center().y), r.width() - 56., &n.name, font_w(13., if on { Weight::Semi } else { Weight::Medium }), t.tx);
                if on {
                    icon(ui, icon!("check"), pos2(r.max.x - 20., r.center().y), 16., t.tx);
                }
                if resp.clicked() {
                    picked = Some(n.code.clone());
                }
            }
        });
        if let Some(code) = picked {
            self.settings_pick = None;
            match which {
                Pick::Language => self.put(ui, json!({ "contentLanguage": code }), |d| d.content_language = code.clone()),
                Pick::Country => self.put(ui, json!({ "contentCountry": code }), |d| d.content_country = code.clone()),
            }
        }
    }

    fn accounts(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        // The sign-in cards are laid out by `accounts_panel` with 16px of inset, so give it a rect 16px wider each side.
        let wide = Rect::from_min_max(pos2(pg.x0 - 16., pg.y), pos2(pg.x1 + 16., pg.y + 300.));
        pg.y += self.accounts_panel(ui, wide, pg.y - 16.);
        self.lastfm(ui, pg);
        self.sec(ui, pg, "Discord");
        if let Some(cy) = self.row(ui, pg, "Discord activity", "Current song on your Discord profile") {
            toggle(ui, &self.t, "discord", pg.right(), cy, &mut self.opts.discord);
        }
        if let Some(cy) = self.row(ui, pg, "Keep activity when paused", "Show as paused instead of clearing") {
            toggle(ui, &self.t, "discord-paused", pg.right(), cy, &mut self.opts.discord_paused);
        }
        if let Some(cy) = self.row(ui, pg, "Show listening party", "Party size instead of progress bar") {
            toggle(ui, &self.t, "discord-party", pg.right(), cy, &mut self.opts.discord_party);
        }
        self.sec(ui, pg, "Listening party");
        if let Some(cy) = self.row(ui, pg, "Let people in automatically", "Accept join requests without asking") {
            toggle(ui, &self.t, "party-auto", pg.right(), cy, &mut self.opts.party_auto_accept);
        }
        if let Some(cy) = self.row(ui, pg, "Notify about join requests", "Requests, and the host's answers") {
            toggle(ui, &self.t, "notify-requests", pg.right(), cy, &mut self.opts.notify_requests);
        }
        if let Some(cy) = self.row(ui, pg, "Notify when people join or leave", "") {
            toggle(ui, &self.t, "notify-people", pg.right(), cy, &mut self.opts.notify_people);
        }
        if let Some(cy) = self.row(ui, pg, "Notify when the party ends", "Removed, or the party closes") {
            toggle(ui, &self.t, "notify-ended", pg.right(), cy, &mut self.opts.notify_ended);
        }
        if let Some(cy) = self.row(ui, pg, "Only when Serafin is in the background", "Not while the window is in front") {
            toggle(ui, &self.t, "notify-background", pg.right(), cy, &mut self.opts.notify_background);
        }
    }

    /// Last.fm: browser sign-in (the daemon keeps only the session key) and the scrobbling options.
    fn lastfm(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let (t, right) = (self.t, pg.right());
        let Some(c) = self.scrobbler.cfg() else {
            return self.info(ui, pg, "Last.fm: connecting to the daemon…");
        };
        let ctx = ui.ctx().clone();
        self.sec(ui, pg, "Last.fm");
        if !c.logged_in {
            if self.scrobbler.waiting() {
                self.scrobbler.poll_login(&ctx, false);
                if let Some(cy) = self.row(ui, pg, "Waiting for approval", "Approve Serafin on the Last.fm page") {
                    if button(ui, &t, "lfm-done", right, cy, "I've approved it") {
                        self.scrobbler.poll_login(&ctx, true);
                    }
                }
                if let Some(cy) = self.row(ui, pg, "Cancel sign-in", "") {
                    if button(ui, &t, "lfm-cancel", right, cy, "Cancel") {
                        self.scrobbler.cancel_login();
                    }
                }
            } else {
                let mut buf = self.lfm_buf.take().unwrap_or_default();
                for (i, (key, title, pw)) in [("lfm-key", "API key", false), ("lfm-secret", "API secret", true)].into_iter().enumerate() {
                    if let Some(cy) = self.row(ui, pg, title, if i == 0 { "Use your own Last.fm API account" } else { "" }) {
                        text_field(ui, &t, key, right, cy, &mut buf[i], pw);
                    }
                }
                let desc = if c.custom_key { "Using your own key" } else { "Using the built-in key" };
                if let Some(cy) = self.row(ui, pg, "Custom key", desc) {
                    let ready = !buf[0].trim().is_empty() && !buf[1].trim().is_empty();
                    let rm = c.custom_key && button(ui, &t, "lfm-key-rm", right - 90., cy, "Remove");
                    if button(ui, &t, "lfm-key-save", right, cy, "Save") && ready {
                        self.scrobbler.set_key(&ctx, &buf[0], &buf[1]);
                        buf = Default::default();
                    }
                    if rm {
                        self.scrobbler.set_key(&ctx, "", "");
                    }
                }
                self.lfm_buf = Some(buf);
            }
            return;
        }
        let toggles: [(&str, &str, &str, bool); 3] = [
            ("enabled", "Scrobble songs", "Adds plays to your Last.fm profile", c.enabled),
            ("nowPlaying", "Send now playing", "Current song on your Last.fm profile", c.now_playing),
            ("sendLikes", "Send likes", "Love on Last.fm when you like it here", c.send_likes),
        ];
        for (i, (key, title, d, on)) in toggles.into_iter().enumerate() {
            let Some(cy) = self.row(ui, pg, title, d) else { continue };
            let mut v = on;
            if toggle(ui, &t, key, right, cy, &mut v) {
                self.scrobbler.set(&ctx, json!({ key: v }), |c| match i {
                    0 => c.enabled = v,
                    1 => c.now_playing = v,
                    _ => c.send_likes = v,
                });
            }
        }
        if let Some(cy) = self.row(ui, pg, "Scrobble after", "Share of the song that must play") {
            const PCT: [f32; 4] = [0.3, 0.5, 0.7, 0.9];
            let cur = PCT.iter().position(|p| (p - c.delay_percent).abs() < 0.01).unwrap_or(usize::MAX);
            let sel = segmented(ui, &t, "lfm-pct", right, cy, &["30%", "50%", "70%", "90%"], cur);
            if sel != cur && sel < PCT.len() {
                self.scrobbler.set(&ctx, json!({ "delayPercent": PCT[sel] }), |c| c.delay_percent = PCT[sel]);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Or after at most", "Seconds that always count on long songs") {
            const SECS: [u32; 4] = [60, 120, 180, 240];
            let cur = SECS.iter().position(|s| *s == c.delay_seconds).unwrap_or(usize::MAX);
            let sel = segmented(ui, &t, "lfm-secs", right, cy, &["1 min", "2 min", "3 min", "4 min"], cur);
            if sel != cur && sel < SECS.len() {
                self.scrobbler.set(&ctx, json!({ "delaySeconds": SECS[sel] }), |c| c.delay_seconds = SECS[sel]);
            }
        }
        if let Some(cy) = self.row(ui, pg, "Minimum song length", "Shorter songs are never scrobbled") {
            const MIN: [u32; 4] = [0, 30, 60, 120];
            let cur = MIN.iter().position(|s| *s == c.min_duration).unwrap_or(usize::MAX);
            let sel = segmented(ui, &t, "lfm-min", right, cy, &["Any", "30 s", "1 min", "2 min"], cur);
            if sel != cur && sel < MIN.len() {
                self.scrobbler.set(&ctx, json!({ "minDuration": MIN[sel] }), |c| c.min_duration = MIN[sel]);
            }
        }
    }

    /// Keyboard shortcuts, then reset.
    fn system(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let t = self.t;
        let cols = if pg.x1 - pg.x0 < 560. { 1 } else { 2 };
        let per_col = SHORTCUTS.len().div_ceil(cols);
        self.sec(ui, pg, "Keyboard shortcuts");
        self.close(ui, pg);
        self.head(ui, pg);
        let p = ui.painter().clone();
        let panel = Rect::from_min_size(pos2(pg.x0, pg.y), vec2(pg.x1 - pg.x0, per_col as f32 * 40. + 16.));
        p.rect_filled(panel, rad(16.), t.s2);
        let col_w = (panel.width() - 48. - 32. * (cols - 1) as f32) / cols as f32;
        for (i, (name, keys)) in SHORTCUTS.iter().enumerate() {
            let x = panel.min.x + 24. + (i / per_col) as f32 * (col_w + 32.);
            let cy = panel.min.y + 28. + (i % per_col) as f32 * 40.;
            trunc(&p, pos2(x, cy), col_w - 120., name, font(13.), t.tx);
            let mut right = x + col_w;
            for k in keys.iter().rev() {
                let w = text_w(&p, k, font_w(12., Weight::Medium)) + 16.;
                let chip = Rect::from_min_size(pos2(right - w.max(24.), cy - 12.), vec2(w.max(24.), 24.));
                p.rect_filled(chip, rad(8.), t.s3);
                p.text(chip.center(), Align2::CENTER_CENTER, k, font_w(12., Weight::Medium), t.tx);
                right = chip.min.x - 4.;
            }
        }
        pg.y = panel.max.y + 22.;
        self.about(ui, pg);
    }

    fn about(&mut self, ui: &mut egui::Ui, pg: &mut Page) {
        let right = pg.right();
        self.sec(ui, pg, "Reset");
        if let Some(cy) = self.row(ui, pg, "Reset preferences", "Sign-ins and proxy are kept") {
            let id = ui.scope_id().with("reset-armed");
            let armed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            if button(ui, &self.t, "reset-opts", right, cy, if armed { "Click again to confirm" } else { "Reset" }) {
                if armed {
                    self.reset_opts(ui.ctx());
                    crate::toast::info("Preferences reset");
                }
                ui.data_mut(|d| d.insert_temp(id, !armed));
            }
        }
    }

    /// Defaults for every preference, pushed to the engine and filters the same way startup does.
    fn reset_opts(&mut self, ctx: &egui::Context) {
        self.apply_opts(ctx, Opts::default());
    }

    /// Makes `o` the preferences and pushes them to the engine and filters the same way startup does.
    pub fn apply_opts(&mut self, ctx: &egui::Context, mut o: Opts) {
        o.sanitize();
        if !o.keep_search {
            self.recents.clear();
        }
        self.pl.endless = o.endless;
        self.pl.no_dupes = o.no_dupes;
        self.pl.set_romaji(o.romaji);
        self.pl.lyr_size = o.lyrics_size as f32 / 100.;
        self.pl.engine.set_crossfade(o.crossfade as f32);
        self.pl.engine.set_gapless(o.gapless);
        self.pl.engine.set_normalize(o.normalize);
        self.pl.engine.set_target(o.target_lufs);
        crate::cache::set_limit_mb(o.cache_mb);
        crate::data::HIDE_EXPLICIT.store(o.hide_explicit, std::sync::atomic::Ordering::Relaxed);
        crate::data::HIDE_VIDEO.store(o.hide_video, std::sync::atomic::Ordering::Relaxed);
        set_fonts(ctx, &o.font);
        set_scale(ctx, o.ui_scale);
        set_scroll_speed(ctx, o.scroll_speed);
        (self.video_on, self.video_h) = (o.video, if VIDEO_HEIGHTS.contains(&o.video_h) { o.video_h } else { VIDEO_HEIGHTS[0] });
        self.opts = o;
        self.data.refilter();
        self.reload_custom();
    }
}

/// Applies the interface scale and keeps the window's minimum size at 800x560 points.
pub fn set_scale(ctx: &egui::Context, pct: u8) {
    let z = pct.clamp(50, 200) as f32 / 100.;
    ctx.set_zoom_factor(z);
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(vec2(800. * z, 560. * z)));
}

/// Points per mouse-wheel notch (egui's own default is 40).
pub fn set_scroll_speed(ctx: &egui::Context, pts: u8) {
    ctx.options_mut(|o| o.input_options.line_scroll_speed = pts as f32);
}

fn nearest<T: Copy + Into<i32>>(v: T, of: &[T]) -> T {
    *of.iter().min_by_key(|x| ((**x).into() - v.into()).abs()).unwrap()
}

impl Opts {
    /// Snaps values that came from an old or hand-edited file onto what the page can show.
    pub fn sanitize(&mut self) {
        self.ui_scale = nearest(self.ui_scale, &SCALES);
        self.scroll_speed = nearest(self.scroll_speed, &SCROLL);
        self.lyrics_size = nearest(self.lyrics_size, &LYR_SIZES);
        self.target_lufs = nearest(self.target_lufs, &LOUD);
        self.seek_step = nearest(self.seek_step, &SEEK);
        self.vol_step = nearest(self.vol_step, &VOL);
        self.crossfade = self.crossfade.min(12);
        self.mode = self.mode.min(2);
        self.accent = self.accent.min(PRESETS.len() as u8 - 1);
    }
}

const LOUD: [i8; 3] = [-19, -14, -11];
const SEEK: [u8; 3] = [5, 10, 15];
const VOL: [u8; 3] = [2, 5, 10];

/// Pill switch; true when clicked this frame.
pub(crate) fn toggle(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, on: &mut bool) -> bool {
    let r = Rect::from_min_size(pos2(right - 40., cy - 12.), vec2(40., 24.));
    let hit = press(ui, ("settings", key), r);
    let k = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-sw", key)), *on, 0.15, ease_out);
    let p = ui.painter();
    p.rect_filled(r, rad(12.), t.s3.lerp_to_gamma(t.ax, k));
    p.circle_filled(pos2(r.min.x + 12. + 16. * k, cy), 8., t.tm.lerp_to_gamma(t.on_ax, k));
    if hit.clicked() {
        *on = !*on;
    }
    hit.clicked()
}

/// Pill button at the right edge; true when clicked.
fn button(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, text: &str) -> bool {
    let p = ui.painter();
    let w = text_w(p, text, font_w(13., Weight::Semi)) + 32.;
    let r = Rect::from_center_size(pos2(right - w / 2., cy), vec2(w, 36.));
    let hit = press(ui, ("settings", key), r);
    p.rect_filled(r, rad(18.), if hit.hovered() { t.s3.lerp_to_gamma(t.tx, 0.1) } else { t.s3 });
    p.text(r.center(), Align2::CENTER_CENTER, text, font_w(13., Weight::Semi), t.tx);
    hit.clicked()
}

/// Two-or-more option control; returns the selected index.
fn segmented(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, opts: &[&str], sel: usize) -> usize {
    let p = ui.painter();
    let ws: Vec<f32> = opts.iter().map(|o| text_w(p, o, font_w(13., Weight::Medium)) + 28.).collect();
    let total: f32 = ws.iter().sum::<f32>() + 4.;
    let track = Rect::from_min_size(pos2(right - total, cy - 16.), vec2(total, 32.));
    p.rect_filled(track, rad(8.), t.s1);
    let (mut x, mut out) = (track.min.x + 2., sel);
    for (i, o) in opts.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, track.min.y + 2.), vec2(ws[i], 28.));
        let hit = press(ui, ("settings", key, i), r);
        let k = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-seg", key, i)), i == sel, 0.18, ease_out);
        let h = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-seg-hot", key, i)), hit.hovered(), 0.12, ease_out);
        p.rect_filled(r, rad(6.), t.s2.gamma_multiply(h * (1. - k)));
        p.rect_filled(r, rad(6.), t.axc.gamma_multiply(k));
        p.text(r.center(), Align2::CENTER_CENTER, o, font_w(13., Weight::Medium), t.tm.lerp_to_gamma(t.tx, k));
        if hit.clicked() {
            out = i;
        }
        x += ws[i];
    }
    out
}

/// Two overlapping ramps (outgoing and incoming track) over a dashed midline, after the prototype's 300x54 viewBox.
fn crossfade_graph(ui: &egui::Ui, t: &crate::theme::Theme, panel: Rect, secs: u8) {
    let p = ui.painter();
    p.rect_filled(panel, rad(12.), t.s1);
    let inner = panel.shrink2(vec2(12., 8.));
    let half = ui.ctx().animate_value_with_time(ui.scope_id().with("xf-half"), secs as f32 / 12. * 70., 0.15);
    let (sx, sy) = (inner.width() / 300., inner.height() / 54.);
    let pt = |x: f32, y: f32| pos2(inner.min.x + x * sx, inner.min.y + y * sy);
    let (a0, a1) = (150. - half, 150. + half);
    p.add(egui::Shape::convex_polygon(vec![pt(0., 6.), pt(a0, 6.), pt(a1, 54.), pt(0., 54.)], t.tm.gamma_multiply(0.55), egui::Stroke::NONE));
    p.add(egui::Shape::convex_polygon(vec![pt(300., 6.), pt(300., 54.), pt(a0, 54.), pt(a1, 6.)], t.ax.gamma_multiply(0.7), egui::Stroke::NONE));
    let x = pt(150., 0.).x;
    let mut y = inner.min.y;
    while y < inner.max.y {
        p.line_segment([pos2(x, y), pos2(x, (y + 3.).min(inner.max.y))], egui::Stroke::new(1., t.tf));
        y += 7.;
    }
}

/// 0..=12 whole-second slider spanning `l..r`; true when the value changed.
fn slider(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, l: f32, r: f32, cy: f32, v: &mut u8) -> bool {
    const MAX: f32 = 12.;
    let track = Rect::from_min_max(pos2(l, cy - 12.), pos2(r, cy + 12.));
    let hit = ui.interact(track, ui.scope_id().with(("settings", key)), Sense::click_and_drag());
    let was = *v;
    if let (true, Some(pos)) = (hit.dragged() || hit.clicked(), hit.interact_pointer_pos()) {
        *v = ((pos.x - l - 8.) / (r - l - 16.) * MAX).round().clamp(0., MAX) as u8;
    }
    if hit.hovered() || hit.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    let (l, r) = (l + 8., r - 8.);
    let x = l + (r - l) * *v as f32 / MAX;
    p.rect_filled(Rect::from_min_max(pos2(l, cy - 2.), pos2(r, cy + 2.)), rad(2.), t.s3);
    p.rect_filled(Rect::from_min_max(pos2(l, cy - 2.), pos2(x, cy + 2.)), rad(2.), t.ax);
    p.circle_filled(pos2(x, cy), if hit.dragged() { 9. } else { 8. }, t.ax);
    *v != was
}

/// Value pill (200px) that opens a list, with a chevron; true when clicked.
fn picker_pill(ui: &egui::Ui, t: &crate::theme::Theme, key: impl std::hash::Hash + std::fmt::Debug, right: f32, cy: f32, text: &str) -> bool {
    let r = Rect::from_min_size(pos2(right - 200., cy - 16.), vec2(200., 32.));
    let hit = press(ui, key, r);
    let p = ui.painter();
    p.rect_filled(r, rad(16.), if hit.hovered() { t.s3 } else { t.s1 });
    trunc(p, pos2(r.min.x + 16., cy), r.width() - 48., text, font_w(13., Weight::Medium), t.tx);
    let c = pos2(r.max.x - 18., cy);
    let s = egui::Stroke::new(1.8, t.tm);
    p.line_segment([c + vec2(-4., -2.), c + vec2(0., 2.)], s);
    p.line_segment([c + vec2(0., 2.), c + vec2(4., -2.)], s);
    hit.clicked()
}

/// `-  value  +` control; returns -1, 1 or 0 for the button pressed this frame.
fn stepper(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, label: &str, can_dec: bool, can_inc: bool) -> i32 {
    let track = Rect::from_min_size(pos2(right - 136., cy - 16.), vec2(136., 32.));
    ui.painter().rect_filled(track, rad(16.), t.s1);
    ui.painter().text(track.center(), Align2::CENTER_CENTER, label, font_w(13., Weight::Medium), t.tx);
    let mut out = 0;
    for (i, (on, dx)) in [(can_dec, 16.), (can_inc, 120.)].into_iter().enumerate() {
        let r = Rect::from_center_size(pos2(track.min.x + dx, cy), vec2(28., 28.));
        let hit = ui.interact(r, ui.scope_id().with(("settings", key, i)), Sense::click());
        if on && hit.hovered() {
            ui.painter().circle_filled(r.center(), 14., t.s3);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let (p, c, s) = (ui.painter(), r.center(), egui::Stroke::new(1.8, if on { t.tx } else { t.tf }));
        p.line_segment([c + vec2(-5., 0.), c + vec2(5., 0.)], s);
        if i == 1 {
            p.line_segment([c + vec2(0., -5.), c + vec2(0., 5.)], s);
        }
        if on && hit.clicked() {
            out = if i == 0 { -1 } else { 1 };
        }
    }
    out
}

/// Small status label right-aligned at `right`; accent-tinted while `on`.
fn pill(p: &egui::Painter, t: &crate::theme::Theme, right: f32, cy: f32, text: &str, on: bool) {
    let w = text_w(p, text, font_w(12., Weight::Semi)) + 20.;
    let r = Rect::from_center_size(pos2(right - w / 2., cy), vec2(w, 24.));
    p.rect_filled(r, rad(12.), if on { t.axc } else { t.s3 });
    p.text(r.center(), Align2::CENTER_CENTER, text, font_w(12., Weight::Semi), if on { t.tx } else { t.tm });
}

/// Horizontal usage bar filled to `frac` (0..=1).
fn meter(p: &egui::Painter, t: &crate::theme::Theme, r: Rect, frac: f32) {
    p.rect_filled(r, rad(r.height() / 2.), t.s3);
    let w = (r.width() * frac.clamp(0., 1.)).max(if frac > 0. { r.height() } else { 0. });
    p.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), rad(r.height() / 2.), if frac > 0.9 { t.tx } else { t.ax });
}

/// Single-line text box, 220px wide; true when it just lost focus (Enter or click away), i.e. the edit is done.
fn text_field(ui: &mut egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, buf: &mut String, password: bool) -> bool {
    let r = Rect::from_min_size(pos2(right - 220., cy - 16.), vec2(220., 32.));
    let id = ui.scope_id().with(("settings-field", key));
    ui.painter().rect_filled(r, rad(8.), t.s1);
    if ui.memory(|m| m.has_focus(id)) {
        ui.painter().rect_stroke(r, rad(8.), egui::Stroke::new(2., t.ax), egui::StrokeKind::Inside);
    }
    let edit = r.shrink2(vec2(12., 0.));
    ui.put(edit, egui::TextEdit::singleline(buf).id(id).frame(egui::Frame::NONE).font(font(13.)).password(password).align(Align2::LEFT_CENTER).desired_width(edit.width())).lost_focus()
}
