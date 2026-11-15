//! Generic popup menu: rounded surface with shadow, items with trailing icon/check, headings, dividers, segmented icon rows.
//! Caller owns the `open` flag (toggle it from the anchor button); the menu closes itself on Esc or a press outside it.
//! It stays open after a pick so radio-style menus can show the change; set `open = false` to close on pick.

use crate::gfx::*;
use crate::theme::Theme;
use eframe::egui::{self, pos2, vec2, Align2, Area, Context, Id, Key, Order, Rect, Sense};

const W: f32 = 232.;

#[allow(dead_code)] // Divider and Action are for the next menus
pub enum Item<'a> {
    Heading(&'a str),
    Divider,
    /// (id, label, checked): radio-style row, checkmark on the right.
    Check(usize, &'a str, bool),
    /// (id, label, trailing icon).
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
                    p.text(pos2(r.min.x + 10., r.center().y), Align2::LEFT_CENTER, label, font(13.), t.tx);
                    if let Some(ic) = ic {
                        icon(ui, ic, pos2(r.max.x - 18., r.center().y), 16., t.tm);
                    }
                }
                Item::Seg(segs) => {
                    let bar = Rect::from_min_size(pos2(x0 + 4., y + 2.), vec2(x1 - x0 - 8., 40.));
                    p.rect_filled(bar, rad(12.), t.s1);
                    let inner = bar.shrink(3.);
                    let bw = (inner.width() - 2. * (segs.len() as f32 - 1.)) / segs.len() as f32;
                    for (n, (id, ic, tip, on)) in segs.iter().enumerate() {
                        let r = Rect::from_min_size(pos2(inner.min.x + n as f32 * (bw + 2.), inner.min.y), vec2(bw, 34.));
                        let resp = hot(ui, k * 100 + n, r, *on).on_hover_text(*tip);
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
