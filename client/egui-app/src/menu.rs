//! Generic popup menu: rounded surface with shadow, items with leading icon or trailing check, headings, dividers, segmented icon rows.
//! Caller owns the `open` flag (toggle it from the anchor button); the menu closes itself on Esc or a press outside it.
//! It stays open after a pick so radio-style menus can show the change; set `open = false` to close on pick.

use crate::tip::Tip;
use crate::app::App;
use crate::data::{self, State, Track};
use crate::gfx::*;
use crate::theme::Theme;
use eframe::egui::{self, pos2, vec2, Align2, Area, Context, Id, Key, Order, Rect, Sense};

const W: f32 = 232.;

/// What a right-click menu is for. Set `App::ctx_menu = Some((pointer, target))` on `secondary_clicked()`; `App::context_menu` draws it.
#[derive(Clone)]
pub enum Target {
    /// A song (track row, song card, the player's current song).
    Track(Track),
    /// An album, playlist (YouTube or Spotify) or artist; `r#type` says which.
    Item(data::Item),
    /// Entry `i` of the play queue.
    Queue(usize),
}

pub enum Item<'a> {
    Heading(&'a str),
    Divider,
    /// (id, label, checked): radio-style row, checkmark on the right.
    Check(usize, &'a str, bool),
    /// (id, label, leading icon).
    Action(usize, &'a str, Option<&'static str>),
    /// Icon-only toggle row: (id, icon, tooltip, selected).
    Seg(&'a [(usize, &'static str, &'a str, bool)]),
}

fn height(i: &Item) -> f32 {
    match i {
        Item::Heading(_) => 26.,
        Item::Divider => 9.,
        Item::Check(..) | Item::Action(..) => 35.,
        Item::Seg(_) => 46.,
    }
}

/// Draws the menu with its top-left near `at` (clamped on screen, flipped upward if it would overflow the bottom).
/// `keep` is a rect whose presses don't count as "outside" (the anchor button, which toggles `open` itself).
/// Returns the id of the item clicked this frame.
pub fn show(ctx: &Context, t: &Theme, id: &'static str, open: &mut bool, at: egui::Pos2, keep: Rect, items: &[Item]) -> Option<usize> {
    if !*open {
        return None;
    }
    let h = 12. + items.iter().map(height).sum::<f32>();
    let scr = ctx.content_rect();
    let mut pos = at;
    if pos.y + h > scr.max.y - 8. {
        pos.y = (at.y - h - 12.).max(scr.min.y + 8.);
    }
    pos.x = pos.x.min(scr.max.x - W - 8.).max(8.);
    let rect = Rect::from_min_size(pos, vec2(W, h));
    let mut picked = None;
    Area::new(Id::unique(id)).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        ui.allocate_rect(rect, Sense::click()); // swallow clicks so they don't reach widgets beneath
        let p = ui.painter().clone();
        shadow(&p, rect, 14., 16, 40, 102);
        p.rect_filled(rect, rad(14.), t.s2);
        let (x0, x1) = (rect.min.x + 6., rect.max.x - 6.);
        let mut y = rect.min.y + 6.;
        let hot = |ui: &egui::Ui, k: usize, r: Rect, on: bool| {
            let resp = ui.interact(r, ui.scope_id().with(k), Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if on || resp.hovered() {
                ui.painter().rect_filled(r, rad(8.), t.s3);
            }
            resp
        };
        for (k, item) in items.iter().enumerate() {
            let hh = height(item);
            match item {
                Item::Heading(s) => {
                    p.text(pos2(x0 + 10., y + 8. + 7.), Align2::LEFT_CENTER, s.to_uppercase(), font_w(10.5, Weight::Semi), t.tf);
                }
                Item::Divider => {
                    p.rect_filled(Rect::from_min_max(pos2(x0 + 4., y + 4.), pos2(x1 - 4., y + 5.)), 0., t.s3);
                }
                Item::Check(id, label, on) => {
                    let r = Rect::from_min_size(pos2(x0, y), vec2(x1 - x0, 34.));
                    if hot(ui, k, r, false).clicked() {
                        picked = Some(*id);
                    }
                    p.text(pos2(r.min.x + 10., r.center().y), Align2::LEFT_CENTER, label, if *on { font_w(13., Weight::Medium) } else { font(13.) }, t.tx);
                    if *on {
                        icon(ui, icon!("check"), pos2(r.max.x - 18., r.center().y), 16., t.tx);
                    }
                }
                Item::Action(id, label, ic) => {
                    let r = Rect::from_min_size(pos2(x0, y), vec2(x1 - x0, 34.));
                    if hot(ui, k, r, false).clicked() {
                        picked = Some(*id);
                    }
                    let lx = if ic.is_some() { 36. } else { 10. };
                    p.text(pos2(r.min.x + lx, r.center().y), Align2::LEFT_CENTER, label, font(13.), t.tx);
                    if let Some(ic) = ic {
                        icon(ui, ic, pos2(r.min.x + 19., r.center().y), 16., t.tm);
                    }
                }
                Item::Seg(segs) => {
                    let bar = Rect::from_min_size(pos2(x0 + 4., y + 2.), vec2(x1 - x0 - 8., 40.));
                    p.rect_filled(bar, rad(12.), t.s1);
                    let inner = bar.shrink(3.);
                    let bw = (inner.width() - 2. * (segs.len() as f32 - 1.)) / segs.len() as f32;
                    for (n, (id, ic, tip, on)) in segs.iter().enumerate() {
                        let r = Rect::from_min_size(pos2(inner.min.x + n as f32 * (bw + 2.), inner.min.y), vec2(bw, 34.));
                        let resp = hot(ui, k * 100 + n, r, *on).tip(t, tip);
                        if resp.clicked() {
                            picked = Some(*id);
                        }
                        icon(ui, ic, r.center(), 16., if *on { t.tx } else { t.tm });
                    }
                }
            }
            y += hh;
        }
    });
    let (esc, outside) = ctx.input(|i| (i.key_pressed(Key::Escape), i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !keep.contains(p))));
    if esc || outside {
        *open = false;
    }
    picked
}

const PLAY: usize = 0;
const NEXT: usize = 1;
const QUEUE: usize = 2;
const SHUFFLE: usize = 3;
const OPEN: usize = 4;
const SONG: usize = 5;
const ARTIST: usize = 6;
const SAVE: usize = 8;
const REMOVE: usize = 9;
const LINK: usize = 10;
const DELETE: usize = 11;
const SUGGEST: usize = 12;

fn track_link(t: &Track) -> String {
    if t.source == "spotify" { format!("https://open.spotify.com/track/{}", t.id) } else { format!("https://music.youtube.com/watch?v={}", t.id) }
}

fn item_link(it: &data::Item) -> String {
    match it.r#type.as_str() {
        "artist" => format!("https://music.youtube.com/channel/{}", it.id),
        "playlist" => format!("https://music.youtube.com/playlist?list={}", it.id.strip_prefix("VL").unwrap_or(&it.id)),
        "spotify" => format!("https://open.spotify.com/playlist/{}", it.id),
        _ => format!("https://music.youtube.com/browse/{}", it.id),
    }
}

impl App {
    /// Opens `m`; from a menu button (`btn`) a second click closes it instead.
    pub fn toggle_menu(&mut self, btn: Option<Rect>, m: (egui::Pos2, Target)) {
        if btn.is_some() && self.ctx_menu.is_some() {
            self.ctx_menu = None;
            self.ctx_keep = Rect::NOTHING;
        } else {
            self.ctx_keep = btn.unwrap_or(Rect::NOTHING);
            self.ctx_menu = Some(m);
        }
    }

    /// Draws the right-click menu in `ctx_menu`, if any, and runs the picked action.
    pub fn context_menu(&mut self, ctx: &Context) {
        let Some((at, target)) = self.ctx_menu.clone() else { return };
        let t = self.t;
        let mut items = vec![];
        // Songs an album/playlist/artist item would play; `None` while its page is still loading.
        let mut songs: Option<Vec<Track>> = None;
        let mut saved = None;
        match &target {
            Target::Track(tr) => {
                items.extend([Item::Action(PLAY, "Play", Some(icon!("mplay"))), Item::Action(NEXT, "Play next", Some(icon!("mnext"))), Item::Action(QUEUE, "Add to queue", Some(icon!("que")))]);
                if self.party.is_guest() {
                    items.push(Item::Action(SUGGEST, "Suggest to the party", Some(icon!("party"))));
                }
                items.extend([Item::Divider, Item::Action(SONG, "Open", Some(icon!("mnote")))]);
                if tr.artist_id.is_some() {
                    items.push(Item::Action(ARTIST, "Go to artist", Some(icon!("martist"))));
                }
            }
            Target::Item(it) => {
                if let Some((kind, _)) = crate::live::target(it) {
                    songs = match &*self.data.detail(ctx, kind, &it.id).lock() {
                        State::Ready(d) => Some(d.songs()),
                        _ => None,
                    };
                }
                match &songs {
                    Some(v) if !v.is_empty() => items.extend([Item::Action(PLAY, "Play", Some(icon!("mplay"))), Item::Action(SHUFFLE, "Shuffle", Some(icon!("shuf"))), Item::Action(NEXT, "Play next", Some(icon!("mnext"))), Item::Action(QUEUE, "Add to queue", Some(icon!("que")))]),
                    Some(_) => {}
                    None => items.push(Item::Heading("Loading songs…")),
                }
                items.extend([Item::Divider, Item::Action(OPEN, match it.r#type.as_str() { "artist" => "Go to artist", "album" => "Go to album", _ => "Open playlist" }, Some(icon!("mopen")))]);
                if matches!(it.r#type.as_str(), "album" | "playlist") {
                    let lib = if it.r#type == "album" { 2 } else { 1 };
                    let on = self.lib_items.iter().any(|i| i.kind == lib && i.id == it.id);
                    saved = Some(on);
                    if on && it.editable {
                        items.push(Item::Action(DELETE, "Delete playlist", Some(icon!("mtrash"))));
                    } else {
                        items.push(Item::Action(SAVE, if on { "Remove from library" } else { "Save to library" }, Some(if on { icon!("munsave") } else { icon!("msave") })));
                    }
                }
            }
            Target::Queue(i) => {
                let Some(tr) = self.pl.track_at(*i) else {
                    self.ctx_menu = None;
                    return;
                };
                let cur = self.pl.is_current(&tr);
                if !cur {
                    items.extend([Item::Action(PLAY, "Play now", Some(icon!("mplay"))), Item::Action(NEXT, "Play next", Some(icon!("mnext"))), Item::Action(REMOVE, "Remove from queue", Some(icon!("x"))), Item::Divider]);
                }
                items.push(Item::Action(SONG, "Open", Some(icon!("mnote"))));
                if tr.artist_id.is_some() {
                    items.push(Item::Action(ARTIST, "Go to artist", Some(icon!("martist"))));
                }
            }
        }
        items.extend([Item::Divider, Item::Action(LINK, "Copy link", Some(icon!("mlink")))]);
        let mut open = true;
        let picked = show(ctx, &t, "ctx-menu", &mut open, at, self.ctx_keep, &items);
        if !open || picked.is_some() {
            self.ctx_menu = None;
            self.ctx_keep = Rect::NOTHING;
        }
        let Some(pick) = picked else { return };
        match target {
            Target::Track(tr) => match pick {
                PLAY => self.pl.play_tracks(ctx, std::slice::from_ref(&tr), 0, false),
                NEXT | QUEUE => {
                    self.pl.enqueue(ctx, std::slice::from_ref(&tr), pick == NEXT);
                    crate::toast::info(if pick == NEXT { "Playing next" } else { "Added to queue" });
                }
                SONG => self.open_song(tr),
                ARTIST => self.open_artist(tr.artist_id.as_deref().unwrap_or_default()),
                SUGGEST => self.party.suggest(&tr),
                _ => {
                    ctx.copy_text(track_link(&tr));
                    crate::toast::info("Link copied");
                }
            },
            Target::Item(it) => {
                let v = songs.unwrap_or_default();
                match pick {
                    PLAY | SHUFFLE => self.pl.play_tracks(ctx, &v, 0, pick == SHUFFLE),
                    NEXT | QUEUE => {
                        self.pl.enqueue(ctx, &v, pick == NEXT);
                        crate::toast::info(if pick == NEXT { "Playing next" } else { "Added to queue" });
                    }
                    OPEN => {
                        if let Some((kind, route)) = crate::live::target(&it) {
                            self.open(route, Some((kind, it.id.clone())));
                        }
                    }
                    DELETE => self.confirm_delete(ctx, &it.id),
                    SAVE => {
                        let kind = if it.r#type == "album" { "albums" } else { "playlists" };
                        self.data.save(ctx, kind, &it.id, !saved.unwrap_or(false));
                    }
                    _ => {
                        ctx.copy_text(item_link(&it));
                        crate::toast::info("Link copied");
                    }
                }
            }
            Target::Queue(i) => {
                let Some(tr) = self.pl.track_at(i) else { return };
                match pick {
                    PLAY => self.pl.play_index(ctx, i),
                    NEXT => self.pl.move_next(i),
                    REMOVE => {
                        self.pl.remove_at(i);
                        crate::toast::info("Removed from queue");
                    }
                    SONG => self.open_song(tr),
                    ARTIST => self.open_artist(tr.artist_id.as_deref().unwrap_or_default()),
                    _ => {
                        ctx.copy_text(track_link(&tr));
                        crate::toast::info("Link copied");
                    }
                }
            }
        }
    }
}
