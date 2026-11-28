//! Dialogs drawn like the listening-party popover: same card, title, captions, field and buttons, centred in the upper third.

use crate::gfx::*;
use crate::party::{btn, label};
use crate::theme::Theme;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Context, Id, Key, Modifiers, Painter, Rect, Ui};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Danger,
}

const PAD: f32 = 14.;

/// Top-down layout cursor inside the card.
pub struct Card<'a> {
    pub ui: &'a mut Ui,
    t: &'a Theme,
    p: Painter,
    l: f32,
    w: f32,
    y: f32,
}

/// Shows the card; true when the user dismissed it with Esc or a click outside.
pub fn show(ctx: &Context, t: &Theme, id: &'static str, width: f32, add: impl FnOnce(&mut Card)) -> bool {
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return true;
    }
    let now = ctx.input(|i| i.time);
    let (sid, hid) = (Id::NULL.with(("modal-t", id)), Id::NULL.with(("modal-h", id)));
    // A gap in frames means the dialog was closed and reopened: restart the fade and the measurement.
    let frame = ctx.cumulative_frame_nr();
    let (t0, _) = ctx.data(|d| d.get_temp::<(f64, u64)>(sid)).filter(|s| frame <= s.1 + 2).unwrap_or_else(|| {
        ctx.data_mut(|d| d.insert_temp(hid, 0f32));
        (now, frame)
    });
    ctx.data_mut(|d| d.insert_temp(sid, (t0, frame)));
    let k = if reduced_motion() { 1. } else { ease_out((((now - t0) / 0.14) as f32).min(1.)) };
    if k < 1. {
        ctx.request_repaint();
    }
    let scr = ctx.content_rect();
    let h = ctx.data(|d| d.get_temp::<f32>(hid)).unwrap_or(0.);
    let x0 = scr.center().x - width / 2.;
    let rect = Rect::from_min_size(pos2(x0, scr.min.y + scr.height() * 0.2 + 6. * (1. - k)), vec2(width, h));
    if h > 0. && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))) {
        return true;
    }
    // Faint scrim that also keeps clicks from reaching the page beneath.
    egui::Area::new(Id::NULL.with(("modal-scrim", id))).order(egui::Order::Foreground).fixed_pos(scr.min).show(ctx, |ui| {
        ui.allocate_rect(scr, egui::Sense::click());
        ui.painter().rect_filled(scr, 0., Color32::from_black_alpha((55. * k) as u8));
    });
    egui::Area::new(Id::NULL.with(("modal-card", id))).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.set_opacity(if h > 0. { k } else { 0. });
        ui.allocate_rect(rect, egui::Sense::click());
        let p = ui.painter().clone().with_clip_rect(scr);
        shadow(&p, rect, 16., 16, 40, 102);
        p.rect_filled(rect, rad(16.), t.s2);
        let mut c = Card { ui, t, p, l: x0 + PAD, w: width - 2. * PAD, y: rect.min.y + PAD };
        add(&mut c);
        let nh = c.y + PAD - rect.min.y;
        if (nh - h).abs() > 0.5 {
            ctx.data_mut(|d| d.insert_temp(hid, nh));
            ctx.request_repaint();
        }
    });
    false
}

pub fn enter(ctx: &Context) -> bool {
    ctx.input(|i| i.key_pressed(Key::Enter))
}

impl Card<'_> {
    pub fn title(&mut self, icon_uri: &'static str, text: &str) {
        let cy = self.y + 11.;
        icon(self.ui, icon_uri, pos2(self.l + 9., cy), 18., self.t.tx);
        trunc(&self.p, pos2(self.l + 26., cy), self.w - 26., text, font_w(15., Weight::Bold), self.t.tx);
        self.y += 22. + 12.;
    }

    /// Wrapped paragraph; `strong` for the main message, otherwise a quieter hint.
    pub fn text(&mut self, text: &str, strong: bool) {
        let (size, col) = if strong { (13., self.t.tm) } else { (12., self.t.tf) };
        let g = self.p.layout(text.into(), font(size), col, self.w);
        let h = g.size().y;
        self.p.galley(pos2(self.l, self.y), g, col);
        self.y += h + 12.;
    }

    /// Captioned single-line field that takes focus as soon as the dialog opens.
    pub fn field(&mut self, id: &'static str, caption: &str, text: &mut String, hint: &str) {
        label(&self.p, self.t, pos2(self.l, self.y + 6.), self.w, caption);
        self.y += 14.;
        let r = Rect::from_min_size(pos2(self.l, self.y), vec2(self.w, 36.));
        let fid = self.ui.scope_id().with(id);
        self.p.rect_filled(r, rad(10.), self.t.s1);
        let focused = self.ui.memory(|m| m.has_focus(fid));
        if focused {
            self.p.rect_stroke(r, rad(10.), egui::Stroke::new(2., self.t.ax), egui::StrokeKind::Inside);
        }
        let inner = r.shrink2(vec2(12., 0.));
        let te = self.ui.put(inner, egui::TextEdit::singleline(text).id(fid).frame(egui::Frame::NONE).font(font(13.)).align(Align2::LEFT_CENTER)
            .hint_text(egui::RichText::new(hint).color(self.t.tm)).desired_width(inner.width()));
        if !focused {
            te.request_focus();
        }
        self.y += 36. + 12.;
    }

    /// A 44px row: name over a quieter line, and an optional button at the right. True when the button was clicked.
    pub fn row(&mut self, key: &str, name: &str, sub: &str, action: Option<&str>) -> bool {
        let (t, y) = (self.t, self.y);
        let bw = action.map_or(0., |a| text_w(&self.p, a, font_w(12.5, Weight::Semi)) + 24.);
        let tw = self.w - bw - 8.;
        trunc(&self.p, pos2(self.l, y + if sub.is_empty() { 22. } else { 14. }), tw, name, font_w(13., Weight::Semi), t.tx);
        if !sub.is_empty() {
            trunc(&self.p, pos2(self.l, y + 31.), tw, sub, font(11.5), t.tm);
        }
        let hit = action.is_some_and(|a| btn(self.ui, t, key, Rect::from_min_size(pos2(self.l + self.w - bw, y + 9.), vec2(bw, 26.)), a, true));
        self.y += 44.;
        hit
    }

    /// Right-aligned Cancel and confirm buttons. Returns `Some(true)` to confirm, `Some(false)` to cancel.
    pub fn actions(&mut self, confirm: &str, kind: Kind, enabled: bool) -> Option<bool> {
        let (t, mut out) = (self.t, None);
        let f = font_w(13., Weight::Semi);
        let h = |s: &str| text_w(&self.p, s, f.clone()) + 28.;
        let (cw, ow) = (h("Cancel"), h(confirm));
        let ok = Rect::from_min_size(pos2(self.l + self.w - ow, self.y), vec2(ow, 32.));
        let no = Rect::from_min_size(pos2(ok.min.x - 8. - cw, self.y), vec2(cw, 32.));
        if btn(self.ui, t, "m-cancel", no, "Cancel", false) {
            out = Some(false);
        }
        let hit = press(self.ui, "m-ok", ok);
        let (bg, fg) = match kind {
            Kind::Primary => (t.tx, t.s0),
            Kind::Danger => (Color32::from_rgb(0xc6, 0x28, 0x28), Color32::WHITE),
        };
        let k = if !enabled { 0.4 } else if hit.hovered() { 0.88 } else { 1. };
        self.p.rect_filled(ok, rad(10.), bg.gamma_multiply(k));
        self.p.text(ok.center(), Align2::CENTER_CENTER, confirm, f, fg.gamma_multiply(if enabled { 1. } else { 0.6 }));
        if enabled && hit.clicked() {
            out = Some(true);
        }
        self.y += 32.;
        out
    }
}
