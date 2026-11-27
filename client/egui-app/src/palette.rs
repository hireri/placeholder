//! Command palette (Ctrl K, "/" or the titlebar pill): the app's search box and a launcher for pages and player commands.
//! `App::palette` draws it and runs the pick.

use crate::app::{App, PlayerView, Route};
use crate::data::{Item, Suggest};
use crate::gfx::*;
use eframe::egui::{
    self, emath::TSTransform, pos2, scroll_area::ScrollBarVisibility, text::LayoutJob, vec2, Align2, Area, Color32, Context, FontId, Id, Key, LayerId, Modifiers, Order, Painter, Rect,
    ScrollArea, Sense, Stroke, StrokeKind, TextFormat,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Act {
    Go(Route),
    Settings,
    Player,
    Library,
    PlayPause,
    Next,
    Prev,
    Shuffle,
    Repeat,
    Like,
    View(PlayerView),
}

struct Entry {
    section: &'static str,
    label: &'static str,
    hint: &'static str,
    icon: &'static str,
    act: Act,
}

const fn e(section: &'static str, label: &'static str, hint: &'static str, icon: &'static str, act: Act) -> Entry {
    Entry { section, label, hint, icon, act }
}

const GO: &str = "Go to";
const CMD: &str = "Commands";
const ENTRIES: &[Entry] = &[
    e(GO, "Home", "", icon!("homef"), Act::Go(Route::Home)),
    e(GO, "Downloads", "", icon!("impf"), Act::Go(Route::Downloads)),
    e(GO, "Local Music", "", icon!("dirf"), Act::Go(Route::Local)),
    e(GO, "History & stats", "", icon!("histf"), Act::Go(Route::History)),
    e(GO, "Settings", "", icon!("gear"), Act::Settings),
    e(CMD, "Show or hide the player", "Ctrl P", icon!("colr"), Act::Player),
    e(CMD, "Collapse or expand the library", "Ctrl B", icon!("col"), Act::Library),
    e(CMD, "Play or pause", "Space", icon!("play"), Act::PlayPause),
    e(CMD, "Next song", "Ctrl →", icon!("ff"), Act::Next),
    e(CMD, "Previous song", "Ctrl ←", icon!("rw"), Act::Prev),
    e(CMD, "Shuffle", "Ctrl S", icon!("shuf"), Act::Shuffle),
    e(CMD, "Repeat", "Ctrl R", icon!("rep"), Act::Repeat),
    e(CMD, "Like current song", "Ctrl L", icon!("star"), Act::Like),
    e(CMD, "Show lyrics", "Ctrl 2", icon!("lyrf"), Act::View(PlayerView::Lyrics)),
    e(CMD, "Show queue", "Ctrl 3", icon!("quef"), Act::View(PlayerView::Queue)),
    e(CMD, "Show visualizer", "Ctrl 4", icon!("viz"), Act::View(PlayerView::Viz)),
];

/// Open palette: the text typed, the highlighted row and what is animating.
pub struct State {
    /// What the input shows; differs from `base` while an arrow-key highlight previews a recent search or completion.
    query: String,
    /// What the user typed: the list is built and fetched from this.
    base: String,
    sel: usize,
    /// Arrow keys were used since the last edit; until then the highlight follows the "Search for" row.
    moved: bool,
    opened: f64,
    /// Set when it was dismissed: it fades out over `CLOSE` seconds and is then dropped.
    closing: Option<f64>,
    /// Last edit: suggestions are fetched once typing pauses.
    typed: f64,
    /// Newest suggestions that arrived, shown while those for the current text load.
    last: Option<Arc<Suggest>>,
    /// A recent search being removed, and when that began.
    gone: Option<(String, f64)>,
}

impl State {
    fn new(now: f64, query: String) -> Self {
        State { base: query.clone(), query, sel: 0, moved: false, opened: now, closing: None, typed: now, last: None, gone: None }
    }
}

/// What a row does when picked.
#[derive(Clone)]
enum Kind {
    /// A past search, shown while the box is empty.
    Recent(String),
    /// Search YouTube Music for exactly this text.
    Search(String),
    /// A completion YouTube Music suggested for the typed text.
    Complete(String),
    Entity(Item),
    Entry(&'static Entry),
}

enum Line {
    Head(&'static str),
    Row(Kind),
}

/// Entries whose label contains every query word (any order), those starting with the first word ahead of the rest.
fn filter(q: &str) -> Vec<&'static Entry> {
    let q = q.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut v: Vec<_> = ENTRIES.iter().filter(|e| { let l = e.label.to_lowercase(); words.iter().all(|w| l.contains(w)) }).collect();
    if let Some(w) = words.first() {
        v.sort_by_key(|e| !e.label.to_lowercase().starts_with(w)); // stable: keeps the original order within each group
    }
    v
}

fn push_entries(lines: &mut Vec<Line>, entries: Vec<&'static Entry>) {
    for (k, it) in entries.iter().enumerate() {
        if k.checked_sub(1).is_none_or(|j| entries[j].section != it.section) {
            lines.push(Line::Head(it.section));
        }
        lines.push(Line::Row(Kind::Entry(it)));
    }
}

fn build(query: &str, recents: &[String], sugg: Option<&Suggest>) -> Vec<Line> {
    let q = query.trim();
    let mut lines = vec![];
    if q.is_empty() {
        if !recents.is_empty() {
            lines.push(Line::Head("Recent searches"));
            lines.extend(recents.iter().take(3).map(|s| Line::Row(Kind::Recent(s.clone()))));
        }
        push_entries(&mut lines, ENTRIES.iter().collect());
        return lines;
    }
    if let Some(items) = sugg.map(|s| s.items.iter().take(3).collect::<Vec<_>>()).filter(|v| !v.is_empty()) {
        lines.push(Line::Head("Songs, albums and artists"));
        lines.extend(items.into_iter().map(|i| Line::Row(Kind::Entity(i.clone()))));
    }
    lines.push(Line::Head("Search YouTube Music"));
    lines.push(Line::Row(Kind::Search(q.to_string())));
    if let Some(s) = sugg {
        let ql = q.to_lowercase();
        lines.extend(s.queries.iter().filter(|c| c.to_lowercase() != ql).take(3).map(|c| Line::Row(Kind::Complete(c.clone()))));
    }
    push_entries(&mut lines, filter(q));
    lines
}

const W: f32 = 560.;
const TOP: f32 = 72.;
const INPUT_H: f32 = 48.;
const FOOT_H: f32 = 36.;
const MAX_H: f32 = 480.;
const ROW: f32 = 36.;
const ENT: f32 = 44.;
const HEAD: f32 = 28.;
const CLOSE: f64 = 0.12;
/// Open timings in seconds: scrim fade, box scale and rise.
const OPEN_VEIL: f32 = 0.1;
const OPEN_BOX: f32 = 0.2;
const RECENTS: usize = 5;

fn height(l: &Line) -> f32 {
    match l {
        Line::Head(_) => HEAD,
        Line::Row(Kind::Entity(_)) => ENT,
        Line::Row(_) => ROW,
    }
}

/// Single-line text made of differently styled runs, ellipsised at `max_w`.
fn rich(p: &Painter, pos: egui::Pos2, max_w: f32, parts: &[(&str, FontId, Color32)]) {
    if max_w < 8. {
        return;
    }
    let mut job = LayoutJob::default();
    for (s, f, c) in parts.iter().filter(|p| !p.0.is_empty()) {
        job.append(s, 0., TextFormat { font_id: f.clone(), color: *c, ..Default::default() });
    }
    job.wrap.max_width = max_w;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = p.layout_job(job);
    p.galley(Align2::LEFT_CENTER.anchor_size(pos, g.size()).min, g, Color32::WHITE);
}

/// `s` split around the first case-insensitive occurrence of `q`: (before, match, after). All of it is "before" when absent.
fn split<'a>(s: &'a str, q: &str) -> (&'a str, &'a str, &'a str) {
    let (ls, lq) = (s.to_lowercase(), q.to_lowercase());
    match ls.find(&lq).filter(|_| ls.len() == s.len() && !lq.is_empty()) {
        Some(i) => (&s[..i], &s[i..i + lq.len()], &s[i + lq.len()..]),
        None => (s, "", ""),
    }
}

fn type_label(t: &str) -> &'static str {
    match t {
        "song" => "Song",
        "album" => "Album",
        "artist" => "Artist",
        "playlist" => "Playlist",
        _ => "",
    }
}

impl App {
    /// Opens the palette over everything else; any open menu or popover closes first.
    pub fn open_palette(&mut self, ctx: &Context) {
        if self.palette.as_ref().is_some_and(|s| s.closing.is_none()) {
            return;
        }
        self.me_menu = None;
        self.ctx_menu = None;
        self.lib_menu = false;
        self.vol_open = false;
        // On the Search page it reopens on the query shown there, selected so typing replaces it.
        let query = if self.route == Route::Search { self.submitted.clone() } else { String::new() };
        let id = Id::unique("palette-input");
        let mut edit = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        edit.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&query)));
        edit.store(ctx, id);
        self.palette = Some(State::new(ctx.input(|i| i.time), query));
    }

    /// Ctrl/Cmd+K opens or closes the palette. Runs before `keys`, so it also works while a text field has focus.
    pub fn palette_toggle(&mut self, ctx: &Context) {
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            match &mut self.palette {
                Some(st) if st.closing.is_none() => st.closing = Some(ctx.input(|i| i.time)),
                _ => self.open_palette(ctx),
            }
        }
    }

    fn remember_search(&mut self, q: &str) {
        if !self.opts.keep_search {
            return;
        }
        self.recents.retain(|s| s != q);
        self.recents.insert(0, q.to_string());
        self.recents.truncate(RECENTS);
    }

    fn run_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() {
            return;
        }
        self.remember_search(q);
        self.submitted = q.to_string();
        self.go(Route::Search);
    }

    pub fn palette(&mut self, ctx: &Context) {
        let Some(mut st) = self.palette.take() else { return };
        let (t, now, calm) = (self.t, ctx.input(|i| i.time), reduced_motion());
        if st.closing.is_some_and(|c| now - c >= CLOSE) {
            return;
        }
        let live = st.closing.is_none();
        let [esc, enter, down, up] = ctx.input_mut(|i| [Key::Escape, Key::Enter, Key::ArrowDown, Key::ArrowUp].map(|k| i.consume_key(Modifiers::NONE, k)));

        // A removed recent search stays as a collapsing row for 140ms, then leaves the store.
        if let Some((s, at)) = st.gone.clone() {
            if now - at >= 0.14 {
                self.recents.retain(|r| *r != s);
                st.gone = None;
            }
        }
        // Suggestions are asked for once typing pauses; until they arrive the previous ones stay up.
        let q = st.base.trim().to_string();
        if q.is_empty() {
            st.last = None;
        } else if now - st.typed >= 0.15 {
            if let Some(s) = self.data.suggest(ctx, &q).ready() {
                st.last = Some(s);
            }
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(160));
        }

        let lines = build(&st.base, &self.recents, st.last.as_deref());
        let n = lines.iter().filter(|l| matches!(l, Line::Row(_))).count();
        if live && n > 0 && (down || up) {
            st.sel = (st.sel + if down { 1 } else { n - 1 }) % n;
            st.moved = true;
        }
        if !st.moved {
            let mut rows = lines.iter().filter_map(|l| if let Line::Row(k) = l { Some(k) } else { None });
            st.sel = rows.position(|k| matches!(k, Kind::Search(_))).unwrap_or(0);
        }
        st.sel = st.sel.min(n.saturating_sub(1));

        // Open: see OPEN_*. Close: both 120ms, the same way back.
        let (veil, rise) = match st.closing {
            Some(c) if !calm => {
                let k = 1. - ease_out(((now - c) / CLOSE) as f32);
                (k, k)
            }
            Some(_) => (0., 0.),
            None if calm => (1., 1.),
            None => {
                let t = (now - st.opened) as f32;
                (ease_out((t / OPEN_VEIL).min(1.)), bezier(0.16, 1., 0.3, 1., (t / OPEN_BOX).min(1.)))
            }
        };
        if st.closing.is_some() || (now - st.opened) < OPEN_BOX as f64 {
            ctx.request_repaint();
        }

        let scr = ctx.content_rect();
        let w = W.min(scr.width() - 48.);
        let x0 = scr.center().x - w / 2.;
        let top = scr.min.y + TOP;
        let content_h: f32 = lines.iter().map(|l| match l {
            Line::Row(Kind::Recent(s)) => match &st.gone {
                Some((g, at)) if g == s => ROW * (1. - ((now - at) / 0.14) as f32).clamp(0., 1.),
                _ => ROW,
            },
            l => height(l),
        }).sum();
        let max_h = MAX_H.min(scr.height() - TOP - 16.).max(INPUT_H + FOOT_H + 80.);
        let res_h = (content_h + 8.).min(max_h - INPUT_H - FOOT_H);
        let boxr = Rect::from_min_size(pos2(x0, top), vec2(w, INPUT_H + res_h + FOOT_H));
        let list_r = Rect::from_min_size(pos2(x0 + 8., top + INPUT_H), vec2(w - 16., res_h - 8.));
        let foot_r = Rect::from_min_size(pos2(x0, boxr.max.y - FOOT_H), vec2(w, FOOT_H));

        let input_id = Id::unique("palette-input");
        // Like Google: highlighting a recent search or completion with the arrows copies it into the input to edit; any other row restores what was typed.
        if live && (down || up) {
            let shown = match lines.iter().filter_map(|l| if let Line::Row(k) = l { Some(k) } else { None }).nth(st.sel) {
                Some(Kind::Recent(s) | Kind::Complete(s)) => s.clone(),
                _ => st.base.clone(),
            };
            if shown != st.query {
                let mut edit = egui::text_edit::TextEditState::load(ctx, input_id).unwrap_or_default();
                edit.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(shown.chars().count()))));
                edit.store(ctx, input_id);
                st.query = shown;
            }
        }
        let (mut run, mut close) = (None::<Kind>, esc && live);
        Area::new(Id::unique("palette-scrim")).order(Order::Foreground).fixed_pos(scr.min).show(ctx, |ui| {
            let r = ui.allocate_rect(scr, Sense::click());
            close |= r.clicked() && live;
            ui.painter().rect_filled(scr, 0., Color32::from_black_alpha((107. * veil) as u8));
        });
        let box_layer = LayerId::new(Order::Tooltip, Id::unique("palette-box"));
        Area::new(box_layer.id).order(Order::Tooltip).fixed_pos(boxr.min).show(ctx, |ui| {
            ui.set_opacity(veil);
            ui.allocate_rect(boxr, Sense::click()); // clicks on the box must not count as outside
            let p = ui.painter().clone().with_clip_rect(scr.expand(80.));
            shadow(&p, boxr, 12., 24, 64, 115);
            p.rect_filled(boxr, rad(12.), t.s2);
            p.rect_stroke(boxr, rad(12.), Stroke::new(1., t.s3), StrokeKind::Outside);

            // Input row.
            let mid = pos2(x0 + 16. + 8., top + INPUT_H / 2.);
            icon(ui, icon!("search"), mid, 16., t.tm);
            let edit = Rect::from_min_max(pos2(x0 + 42., top), pos2(boxr.max.x - 16., top + INPUT_H));
            if live {
                ui.memory_mut(|m| m.request_focus(input_id)); // modal: typing always goes to the palette
            }
            let te = ui.put(edit, egui::TextEdit::singleline(&mut st.query).id(input_id).frame(egui::Frame::NONE).font(font(15.)).align(Align2::LEFT_CENTER)
                .hint_text(egui::RichText::new("Search or type a command").color(t.tm)).desired_width(edit.width()));
            if te.changed() {
                (st.sel, st.typed, st.moved, st.base) = (0, now, false, st.query.clone());
                ctx.request_repaint(); // the list below was built from the previous text
            }

            // Rows.
            let moved = down || up;
            let mut lui = App::region(ui, list_r);
            lui.spacing_mut().item_spacing = vec2(0., 0.);
            let out = ScrollArea::vertical().id_salt("palette-list").auto_shrink(false).scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden).show(&mut lui, |ui| {
                let mut k = 0;
                for (i, line) in lines.iter().enumerate() {
                    let mut p = ui.painter().clone();
                    match line {
                        Line::Head(s) => {
                            let (r, _) = ui.allocate_exact_size(vec2(list_r.width(), HEAD), Sense::hover());
                            let mut job = LayoutJob::single_section(s.to_uppercase(), TextFormat { font_id: font_w(10.5, Weight::Semi), color: t.tf, extra_letter_spacing: 0.735, ..Default::default() });
                            job.wrap.max_width = r.width() - 20.;
                            let g = p.layout_job(job);
                            p.galley(pos2(r.min.x + 10., r.max.y - 4. - g.size().y), g, t.tf);
                        }
                        Line::Row(kind) => {
                            let this = k;
                            k += 1;
                            let gone = matches!((kind, &st.gone), (Kind::Recent(s), Some((g, _))) if g == s);
                            let collapse = match &st.gone {
                                Some((_, at)) if gone => (1. - ((now - at) / 0.14) as f32).clamp(0., 1.),
                                _ => 1.,
                            };
                            let h = height(line) * collapse;
                            let (r, resp) = ui.allocate_exact_size(vec2(list_r.width(), h), Sense::click());
                            if collapse < 1. {
                                p.multiply_opacity((collapse * 2.).min(1.)); // opacity runs out within the first 100 of the 140ms
                            }
                            let p = p.with_clip_rect(r.intersect(p.clip_rect()));
                            let vr = Rect::from_min_size(r.min, vec2(r.width(), height(line)));
                            let on = this == st.sel;
                            if live && !gone && resp.hovered() && ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                st.sel = this;
                            }
                            if resp.hovered() && live {
                                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if on {
                                if moved {
                                    ui.scroll_to_rect(r, None);
                                }
                                p.rect_filled(vr, rad(8.), t.s3);
                            }
                            let cy = vr.center().y;
                            let (icx, tx) = (vr.min.x + 10. + 9., vr.min.x + 10. + 18. + 12.);
                            let reg = font(13.);
                            match kind {
                                Kind::Recent(s) => {
                                    icon(ui, icon!("histf"), pos2(icx, cy), 18., t.tm);
                                    // The x shows on hover or selection and takes its own clicks.
                                    let xr = Rect::from_center_size(pos2(vr.max.x - 10. - 12., cy), vec2(24., 24.));
                                    let xh = ui.interact(xr, ui.scope_id().with(("palette-rm", i)), Sense::click());
                                    if !gone && live && (on || resp.hovered() || xh.hovered()) {
                                        if xh.hovered() {
                                            p.circle_filled(xr.center(), 12., t.s1);
                                            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        icon(ui, icon!("x"), xr.center(), 14., if xh.hovered() { t.tx } else { t.tf });
                                    }
                                    trunc(&p, pos2(tx, cy), vr.max.x - 10. - 24. - 8. - tx, s, reg, t.tx);
                                    if xh.clicked() && live && st.gone.is_none() {
                                        st.gone = Some((s.clone(), now));
                                        ctx.request_repaint();
                                    } else if resp.clicked() && !xh.hovered() && !gone {
                                        run = Some(kind.clone());
                                    }
                                    continue;
                                }
                                Kind::Search(q) => {
                                    icon(ui, icon!("search"), pos2(icx, cy), 18., t.tm);
                                    rich(&p, pos2(tx, cy), vr.max.x - 10. - tx, &[("Search for “", reg.clone(), t.tx), (q, font_w(13., Weight::Semi), t.tx), ("”", reg, t.tx)]);
                                }
                                Kind::Complete(c) => {
                                    icon(ui, icon!("search"), pos2(icx, cy), 18., t.tm);
                                    // What was typed stays regular and muted, what YouTube Music adds is bold.
                                    let (pre, m, post) = split(c, st.base.trim());
                                    let semi = font_w(13., Weight::Semi);
                                    rich(&p, pos2(tx, cy), vr.max.x - 10. - tx, &[(pre, semi.clone(), t.tx), (m, reg, t.tm), (post, semi, t.tx)]);
                                }
                                Kind::Entity(it) => {
                                    let cover = Rect::from_min_size(pos2(vr.min.x + 10., cy - 15.), vec2(30., 30.));
                                    let cr = if it.r#type == "artist" { 15. } else { 6. };
                                    p.rect_filled(cover, rad(cr), t.s3);
                                    if let Some(u) = &it.thumbnail {
                                        paint_cover(ui, &sized_url(u, 96), cover, rad(cr), Color32::WHITE);
                                    }
                                    let tx = cover.max.x + 12.;
                                    let (pre, m, post) = split(&it.title, st.base.trim());
                                    rich(&p, pos2(tx, cy - 8.), vr.max.x - 10. - tx, &[(pre, reg.clone(), t.tx), (m, reg.clone(), t.ax), (post, reg, t.tx)]);
                                    let sub = [Some(type_label(&it.r#type)).filter(|s| !s.is_empty()), it.subtitle.as_deref().filter(|s| !s.is_empty())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                                    trunc(&p, pos2(tx, cy + 8.), vr.max.x - 10. - tx, &sub, font(12.), t.tm);
                                }
                                Kind::Entry(en) => {
                                    icon(ui, en.icon, pos2(icx, cy), 18., t.tm);
                                    let hint_w = if en.hint.is_empty() { 0. } else { text_w(&p, en.hint, FontId::monospace(10.5)) + 12. };
                                    trunc(&p, pos2(tx, cy), vr.max.x - 10. - hint_w - tx, en.label, reg, t.tx);
                                    p.text(pos2(vr.max.x - 10., cy), Align2::RIGHT_CENTER, en.hint, FontId::monospace(10.5), t.tf);
                                }
                            }
                            if resp.clicked() && live {
                                run = Some(kind.clone());
                            }
                        }
                    }
                }
            });
            scroll_fades(&lui, &out, t.s2);

            // Footer.
            p.hline(foot_r.x_range(), foot_r.min.y, Stroke::new(1., t.s3));
            let mut x = foot_r.min.x + 16.;
            for (key, what) in [("↑↓", "Navigate"), ("Enter", "Open"), ("Esc", "Close")] {
                let mono = FontId::monospace(10.5);
                p.text(pos2(x, foot_r.center().y), Align2::LEFT_CENTER, key, mono.clone(), t.tm);
                x += text_w(&p, key, mono) + 6.;
                p.text(pos2(x, foot_r.center().y), Align2::LEFT_CENTER, what, font(12.), t.tf);
                x += text_w(&p, what, font(12.)) + 16.;
            }
        });
        // Scale 0.97 → 1 about the box's top centre while it rises 8px; shapes only, so input keeps its coordinates.
        if rise < 1. {
            let s = 0.97 + 0.03 * rise;
            let pivot = vec2(boxr.center().x, boxr.min.y);
            ctx.transform_layer_shapes_after_rounding(box_layer, TSTransform::new(pivot * (1. - s) + vec2(0., -8. * (1. - rise)), s));
        }

        if enter && live && run.is_none() {
            let mut rows = lines.iter().filter_map(|l| if let Line::Row(k) = l { Some(k) } else { None });
            run = rows.nth(st.sel).cloned();
        }
        if run.is_some() {
            close = true;
        }
        if close && st.closing.is_none() {
            st.closing = Some(now);
        }
        self.palette = Some(st);
        if let Some(kind) = run {
            match kind {
                Kind::Recent(s) | Kind::Search(s) | Kind::Complete(s) => self.run_search(&s),
                Kind::Entity(it) => {
                    self.remember_search(&it.title);
                    self.open_item(ctx, &it);
                }
                Kind::Entry(en) => self.run_act(ctx, en.act),
            }
        }
    }

    fn run_act(&mut self, ctx: &Context, a: Act) {
        match a {
            Act::Go(r) => self.go(r),
            Act::Settings => {
                (self.settings_tab, self.settings_q) = (crate::settings::APPEARANCE, String::new());
                self.go(Route::Settings);
            }
            Act::Player => self.set_player((!self.player_open).then_some(self.view)),
            Act::Library => self.lib_open = !self.lib_open,
            Act::PlayPause => self.pl.toggle(ctx),
            Act::Next => self.pl.next(ctx),
            Act::Prev => self.pl.prev(ctx),
            Act::Shuffle => self.pl.toggle_shuffle(),
            Act::Repeat => self.pl.toggle_repeat(),
            Act::Like => self.pl.set_like(ctx),
            Act::View(v) => self.set_player(Some(v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build, filter, split, Kind, Line};
    use crate::data::{Item, Suggest};

    fn labels(q: &str) -> Vec<&'static str> {
        filter(q).iter().map(|e| e.label).collect()
    }

    #[test]
    fn filter_matches_words_in_any_order_and_ranks_prefix_first() {
        assert_eq!(labels("").len(), 16);
        assert_eq!(labels("NEXT"), ["Next song"]);
        assert_eq!(labels("song next"), ["Next song"]);
        assert!(labels("zzz").is_empty());
        // "s" prefixes Settings/Shuffle/Show…, which come before the later mid-word hits ("Downloads").
        let v = labels("s");
        let first_non_prefix = v.iter().position(|l| !l.to_lowercase().starts_with('s')).unwrap();
        assert!(v[..first_non_prefix].starts_with(&["Settings"]));
        assert!(v[first_non_prefix..].iter().all(|l| !l.to_lowercase().starts_with('s')));
    }

    #[test]
    fn split_finds_the_typed_part_case_insensitively() {
        assert_eq!(split("Night Bus Home", "bus"), ("Night ", "Bus", " Home"));
        assert_eq!(split("Night", "zzz"), ("Night", "", ""));
    }

    #[test]
    fn rows_follow_the_typing_state() {
        let rows = |lines: &[Line]| lines.iter().filter_map(|l| if let Line::Row(k) = l { Some(k.clone()) } else { None }).collect::<Vec<_>>();
        // Empty box: at most three recents, then every page and command.
        let recents: Vec<String> = ["a", "b", "c", "d"].map(String::from).into();
        let empty = rows(&build("", &recents, None));
        assert!(matches!(&empty[..4], [Kind::Recent(_), Kind::Recent(_), Kind::Recent(_), Kind::Entry(_)]));
        assert_eq!(empty.len(), 3 + 16);
        // Typing: "Search for" comes first, completions skip the typed text itself, entities cap at three.
        let sugg = Suggest {
            queries: ["night", "night bus", "night shift", "night drive", "night owl"].map(String::from).into(),
            items: (0..5).map(|i| Item { r#type: "song".into(), title: format!("Night {i}"), ..Default::default() }).collect(),
        };
        let typed = rows(&build(" night ", &recents, Some(&sugg)));
        // Results come first, then "Search for", then the completions.
        assert!(matches!(&typed[..5], [Kind::Entity(_), Kind::Entity(_), Kind::Entity(_), Kind::Search(q), Kind::Complete(_)] if q == "night"));
        assert_eq!(typed.iter().filter(|k| matches!(k, Kind::Complete(_))).count(), 3);
        assert!(!typed.iter().any(|k| matches!(k, Kind::Complete(c) if c == "night")));
        assert_eq!(typed.iter().filter(|k| matches!(k, Kind::Entity(_))).count(), 3);
    }
}
