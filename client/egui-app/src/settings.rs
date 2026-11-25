//! Settings page: a category list and one panel at a time (Appearance, Playback, Content, Lyrics, Accounts, Shortcuts).

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
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
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
}

impl Default for Opts {
    fn default() -> Self {
        Opts { compact: false, tint: true, mode: 0, custom: false, auto_theme: true, accent: 0, crossfade: 0, gapless: true, endless: false, video: true, video_h: VIDEO_HEIGHTS[0], discord: true }
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

/// Which list the Content tab has open.
#[derive(Clone, Copy, PartialEq)]
pub enum Pick {
    Language,
    Country,
}

const SYSTEM: &str = "SYSTEM_DEFAULT";
const TABS: [&str; 6] = ["Appearance", "Playback", "Content", "Lyrics", "Accounts", "Shortcuts"];
pub const APPEARANCE: usize = 0;
pub const ACCOUNTS: usize = 4;
pub const KEYS: usize = 5;

/// Mirrors the bindings in `App::keys` (plus the palette); each key is its own chip.
const SHORTCUTS: &[(&str, &[&str])] = &[
    ("Play / pause", &["Space"]),
    ("Next track", &["Ctrl", "→"]),
    ("Previous track", &["Ctrl", "←"]),
    ("Seek forward 5 s", &["→"]),
    ("Seek back 5 s", &["←"]),
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

const ROW_H: f32 = 64.;

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui, card: Rect, top: f32) {
        let t = self.t;
        let p = ui.painter().clone();
        let (nav_w, gap) = (180., 16.);
        let x = card.min.x + 22.;
        for (i, name) in TABS.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, top + i as f32 * 44.), vec2(nav_w, 40.));
            let on = self.settings_tab == i;
            let hit = press(ui, ("settings-tab", i), r);
            let k = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-tab-on", i)), on, 0.18, ease_out);
            let h = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("settings-tab-hot", i)), hit.hovered(), 0.12, ease_out);
            p.rect_filled(r, rad(8.), t.s2.gamma_multiply(h * (1. - k)));
            p.rect_filled(r, rad(8.), t.axc.gamma_multiply(k));
            p.text(pos2(r.min.x + 16., r.center().y), Align2::LEFT_CENTER, name, font_w(13., if on { Weight::Semi } else { Weight::Medium }), t.tm.lerp_to_gamma(t.tx, k));
            if hit.clicked() {
                self.settings_tab = i;
            }
        }
        let panel = Rect::from_min_max(pos2(x + nav_w + gap, top), pos2(card.max.x - 22., (card.max.y - 22.).max(top + 80.)));
        p.rect_filled(panel, rad(18.), t.s2);
        let mut pui = App::region(ui, panel);
        let tab = self.settings_tab;
        if let (2, Some(which)) = (tab, self.settings_pick) {
            return self.picker(&mut pui, panel, which);
        }
        match tab {
            0 => self.appearance(&mut pui, panel),
            ACCOUNTS => self.accounts(&mut pui, panel),
            KEYS => self.shortcuts(&mut pui, panel),
            _ => {
                // Tall tabs scroll: `virt` is the full-height panel in scrolled coordinates.
                let rows = match tab {
                    1 => 7,
                    2 => 7,
                    _ => self.data.settings.ready().map_or(1, |d| d.lyrics.order.len().max(1)),
                };
                ScrollArea::vertical().auto_shrink(false).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut pui, |ui| {
                    let (_, virt) = ui.allocate_space(vec2(panel.width(), rows as f32 * ROW_H + 24.));
                    match tab {
                        1 => self.playback(ui, virt),
                        2 => self.content(ui, virt),
                        _ => self.lyrics(ui, virt),
                    }
                });
            }
        }
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

    /// Title, description and the row's vertical centre; the control is drawn by the caller at the right edge.
    fn setting(&self, ui: &egui::Ui, panel: Rect, i: usize, title: &str, desc: &str) -> f32 {
        let (p, t) = (ui.painter(), self.t);
        let y = panel.min.y + 12. + i as f32 * ROW_H;
        let w = panel.width() - 48. - 240.;
        trunc(p, pos2(panel.min.x + 24., y + 22.), w, title, font(13.), t.tx);
        trunc(p, pos2(panel.min.x + 24., y + 42.), w, desc, font(12.), t.tm);
        y + ROW_H / 2.
    }

    /// Dims the row centred on `cy` while `why` is set and, over it, blocks the controls and says why on hover.
    fn gate(&self, ui: &egui::Ui, panel: Rect, cy: f32, key: &str, why: Option<&str>) {
        let id = ui.scope_id().with(("settings-gate", key));
        let k = ui.ctx().animate_bool_with_time_and_easing(id, why.is_some(), 0.2, ease_out);
        let r = Rect::from_center_size(pos2(panel.center().x, cy), vec2(panel.width() - 16., ROW_H));
        ui.painter().rect_filled(r, rad(12.), self.t.s2.gamma_multiply(0.6 * k));
        if let Some(why) = why {
            let hit = ui.interact(r, id, Sense::click());
            if hit.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
            }
            hit.on_hover_text(why);
        }
    }

    fn appearance(&mut self, ui: &mut egui::Ui, panel: Rect) {
        let right = panel.max.x - 24.;
        let cy = self.setting(ui, panel, 0, "Row density", "Height of track rows in lists");
        let sel = segmented(ui, &self.t, "density", right, cy, &["Comfortable", "Compact"], self.opts.compact as usize);
        self.opts.compact = sel == 1;
        let cy = self.setting(ui, panel, 1, "Mode", "Light, dark, or follow the system");
        self.opts.mode = segmented(ui, &self.t, "mode", right, cy, &["System", "Light", "Dark"], self.opts.mode as usize) as u8;
        let cy = self.setting(ui, panel, 2, "Tint headers from artwork", "Colour album and artist headers from the cover");
        toggle(ui, &self.t, "tint", right, cy, &mut self.opts.tint);
        let cy = self.setting(ui, panel, 3, "Theme from artwork", "Colour the app from the playing cover instead of a fixed accent");
        toggle(ui, &self.t, "auto-theme", right, cy, &mut self.opts.auto_theme);
        let cy = self.setting(ui, panel, 4, "Accent colour", "Preset colour theme used across the app");
        for (i, (name, ..)) in PRESETS.iter().enumerate() {
            let c = pos2(right - 12. - (PRESETS.len() - 1 - i) as f32 * 26., cy);
            let hit = press(ui, ("accent", i), Rect::from_center_size(c, vec2(24., 24.)));
            let k = ui.ctx().animate_bool_with_time_and_easing(ui.scope_id().with(("accent-on", i)), self.opts.accent as usize == i, 0.18, ease_out);
            let p = ui.painter();
            p.circle_filled(c, 10., crate::theme::Theme::preset(i as u8, true).ax);
            p.circle_stroke(c, 12., egui::Stroke::new(2., self.t.tx.gamma_multiply(k)));
            if hit.clicked() {
                self.opts.accent = i as u8;
            }
            hit.on_hover_text(*name);
        }
        let playing = self.pl.song().is_some();
        self.gate(ui, panel, cy, "accent", (self.opts.auto_theme && playing).then_some("Turn off “Theme from artwork” to pick an accent while a song is playing"));
        let cy = self.setting(ui, panel, 5, "Custom theme", "Colours from theme.toml in the config folder; reloads when this page opens");
        if toggle(ui, &self.t, "custom", right, cy, &mut self.opts.custom) {
            self.reload_custom();
        }
    }

    fn playback(&mut self, ui: &mut egui::Ui, panel: Rect) {
        let right = panel.max.x - 24.;
        let cy = self.setting(ui, panel, 0, "Crossfade", "Overlap the end of a song with the start of the next");
        let label = if self.opts.crossfade == 0 { "Off".into() } else { format!("{} s", self.opts.crossfade) };
        trunc(ui.painter(), pos2(right - 200. - 12. - 40., cy), 40., &label, font(13.), self.t.tm);
        if slider(ui, &self.t, "crossfade", right, cy, &mut self.opts.crossfade) {
            self.pl.engine.set_crossfade(self.opts.crossfade as f32);
        }
        let cy = self.setting(ui, panel, 1, "Gapless playback", "Play consecutive tracks without a pause between them");
        if toggle(ui, &self.t, "gapless", right, cy, &mut self.opts.gapless) {
            self.pl.engine.set_gapless(self.opts.gapless);
        }
        let i = 2;
        const Q: [&str; 3] = ["AUTO", "LOW", "HIGH"];
        let d = self.daemon(ui);
        let cy = self.setting(ui, panel, i, "Audio quality", "Stream quality for songs");
        let cur = d.as_ref().ok().and_then(|d| Q.iter().position(|q| *q == d.audio_quality)).unwrap_or(0);
        let sel = segmented(ui, &self.t, "aq", right, cy, &["Auto", "Low", "High"], cur);
        if let (Ok(_), true) = (&d, sel != cur) {
            self.put(ui, json!({ "audioQuality": Q[sel] }), |d| d.audio_quality = Q[sel].into());
        }
        self.gate(ui, panel, cy, "aq", d.as_ref().err().map(|_| "Needs the daemon, which isn't connected"));
        let cy = self.setting(ui, panel, i, "Start with Endless on", "Keep adding similar songs when the queue runs out");
        toggle(ui, &self.t, "endless", right, cy, &mut self.opts.endless);
        let cy = self.setting(ui, panel, i + 1, "Play music videos by default", "Show the video instead of the cover when a song has one");
        toggle(ui, &self.t, "video", right, cy, &mut self.opts.video);
        let cy = self.setting(ui, panel, i + 2, "Default video quality", "Maximum height of music videos");
        let labels = VIDEO_HEIGHTS.map(|h| format!("{h}p"));
        let cur = VIDEO_HEIGHTS.iter().position(|h| *h == self.opts.video_h).unwrap_or(0);
        let sel = segmented(ui, &self.t, "video-h", right, cy, &labels.each_ref().map(String::as_str), cur);
        self.opts.video_h = VIDEO_HEIGHTS[sel];
        let cy = self.setting(ui, panel, i + 3, "Discord activity", "Show the song you are listening to on your Discord profile");
        toggle(ui, &self.t, "discord", right, cy, &mut self.opts.discord);
    }

    fn content(&mut self, ui: &mut egui::Ui, panel: Rect) {
        let d = match self.daemon(ui) {
            Ok(d) => d,
            Err(e) => {
                trunc(ui.painter(), pos2(panel.min.x + 24., panel.min.y + 34.), panel.width() - 48., &e, font(13.), self.t.tf);
                return;
            }
        };
        let right = panel.max.x - 24.;
        let name = |list: &[Named], code: &str| if code == SYSTEM { "System default".into() } else { list.iter().find(|n| n.code == code).map_or(code.to_string(), |n| n.name.clone()) };
        for (i, (which, title, desc, cur, list)) in [
            (Pick::Language, "Content language", "Language of titles and shelves from YouTube Music", &d.content_language, &d.languages),
            (Pick::Country, "Content country", "Region that decides which music YouTube Music recommends", &d.content_country, &d.countries),
        ].into_iter().enumerate() {
            let cy = self.setting(ui, panel, i, title, desc);
            let r = Rect::from_min_size(pos2(right - 200., cy - 16.), vec2(200., 32.));
            let hit = press(ui, ("pick", i), r);
            let p = ui.painter();
            p.rect_filled(r, rad(8.), if hit.hovered() { self.t.s3 } else { self.t.s1 });
            trunc(p, pos2(r.min.x + 12., cy), r.width() - 24., &name(list, cur), font_w(13., Weight::Medium), self.t.tx);
            if hit.clicked() {
                self.settings_pick = Some(which);
            }
        }
        let cy = self.setting(ui, panel, 2, "Use a proxy", "Route the daemon's requests through a proxy server");
        let mut on = d.proxy.enabled;
        if toggle(ui, &self.t, "proxy", right, cy, &mut on) {
            self.put(ui, json!({ "proxy": { "enabled": on } }), |d| d.proxy.enabled = on);
        }
        let cy = self.setting(ui, panel, 3, "Proxy type", "Protocol the proxy speaks");
        const KINDS: [&str; 2] = ["HTTP", "SOCKS"];
        let cur = KINDS.iter().position(|k| *k == d.proxy.r#type).unwrap_or(0);
        let sel = segmented(ui, &self.t, "proxy-type", right, cy, &KINDS, cur);
        if sel != cur {
            self.put(ui, json!({ "proxy": { "type": KINDS[sel] } }), |d| d.proxy.r#type = KINDS[sel].into());
        }
        let mut buf = self.proxy_buf.take().unwrap_or_else(|| [d.proxy.url.clone(), d.proxy.username.clone(), d.proxy.password.clone()]);
        for (i, (key, title, desc, field, pw)) in [("url", "Proxy server", "host:port", &d.proxy.url, false), ("username", "Proxy username", "Leave empty if the proxy needs none", &d.proxy.username, false), ("password", "Proxy password", "", &d.proxy.password, true)].into_iter().enumerate() {
            let cy = self.setting(ui, panel, 4 + i, title, desc);
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

    fn lyrics(&mut self, ui: &mut egui::Ui, panel: Rect) {
        let d = match self.daemon(ui) {
            Ok(d) => d,
            Err(e) => {
                trunc(ui.painter(), pos2(panel.min.x + 24., panel.min.y + 34.), panel.width() - 48., &e, font(13.), self.t.tf);
                return;
            }
        };
        let (t, right, n) = (self.t, panel.max.x - 24., d.lyrics.order.len());
        for (i, name) in d.lyrics.order.iter().enumerate() {
            let cy = self.setting(ui, panel, i, name, &format!("Priority {}", i + 1));
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

    /// Full-panel list for the Content tab's language or country; a pick closes it.
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

    fn accounts(&mut self, ui: &mut egui::Ui, panel: Rect) {
        self.accounts_panel(ui, panel, panel.min.y);
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, panel: Rect) {
        let (p, t) = (ui.painter().clone(), self.t);
        let col_w = (panel.width() - 48. - 32.) / 2.;
        let per_col = SHORTCUTS.len().div_ceil(2);
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
    }
}

/// Pill switch; true when clicked this frame.
fn toggle(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, on: &mut bool) -> bool {
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

/// 0..=12 whole-second slider, 200px wide; true when the value changed.
fn slider(ui: &egui::Ui, t: &crate::theme::Theme, key: &str, right: f32, cy: f32, v: &mut u8) -> bool {
    const MAX: f32 = 12.;
    let track = Rect::from_min_size(pos2(right - 200., cy - 12.), vec2(200., 24.));
    let hit = ui.interact(track, ui.scope_id().with(("settings", key)), Sense::click_and_drag());
    let was = *v;
    if let (true, Some(pos)) = (hit.dragged() || hit.clicked(), hit.interact_pointer_pos()) {
        *v = ((pos.x - track.min.x - 8.) / (track.width() - 16.) * MAX).round().clamp(0., MAX) as u8;
    }
    if hit.hovered() || hit.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    let (l, r) = (track.min.x + 8., track.max.x - 8.);
    let x = l + (r - l) * *v as f32 / MAX;
    p.rect_filled(Rect::from_min_max(pos2(l, cy - 2.), pos2(r, cy + 2.)), rad(2.), t.s3);
    p.rect_filled(Rect::from_min_max(pos2(l, cy - 2.), pos2(x, cy + 2.)), rad(2.), t.ax);
    p.circle_filled(pos2(x, cy), if hit.dragged() { 9. } else { 8. }, t.ax);
    *v != was
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
