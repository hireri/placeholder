//! Tooltips: delayed on first hover, instant while moving between neighbours, faded, never dismissed by scrolling.

use crate::gfx::{ease_out, font_w, rad, shadow, tween, Weight};
use crate::theme::Theme;
use eframe::egui::{self, pos2, vec2, Align2, Id, LayerId, Order, Rect, Response};

const DELAY: f64 = 0.5;
/// A tooltip shown this recently makes the next one appear without the delay.
const WARM: f64 = 0.35;

/// Shows `text` next to `resp` once it has been hovered for a moment. `pin` keeps it up regardless of hover (e.g. while scrolling).
pub fn tip(resp: &Response, t: &Theme, text: &str, pin: bool) {
    let ctx = &resp.ctx;
    let id = resp.id.with("tip");
    let warm = Id::NULL.with("tip-warm");
    let now = ctx.input(|i| i.time);
    let (hovered, pressed) = (resp.hovered(), resp.is_pointer_button_down_on());
    let since = ctx.data_mut(|d| {
        let s = d.get_temp::<f64>(id);
        match (hovered, s) {
            (true, None) => { d.insert_temp(id, now); },
            (false, Some(_)) => { d.remove::<f64>(id); }
            _ => {}
        }
        d.get_temp::<f64>(id)
    });
    let hot = ctx.data(|d| d.get_temp::<f64>(warm)).is_some_and(|w| now - w < WARM);
    let due = since.map_or(f64::MAX, |s| s + if hot { 0. } else { DELAY });
    let want = pin || (hovered && !pressed && now >= due);
    if hovered && !pressed && now < due {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(due - now));
    }
    let k = tween(ctx, id.with("a"), want as u8 as f32, if want { 0.12 } else { 0.08 }, ease_out);
    if k < 0.01 {
        return;
    }
    if want {
        ctx.data_mut(|d| d.insert_temp(warm, now));
    }

    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, id));
    let g = p.layout_no_wrap(text.to_owned(), font_w(12., Weight::Medium), t.tx.gamma_multiply(k));
    let size = g.size() + vec2(20., 12.);
    let screen = ctx.content_rect().shrink(6.);
    let a = resp.rect;
    let above = a.min.y - 8. - size.y > screen.min.y;
    let rise = (1. - k) * 3.;
    let y = if above { a.min.y - 8. - size.y + rise } else { a.max.y + 8. - rise };
    let x = (a.center().x - size.x / 2.).clamp(screen.min.x, (screen.max.x - size.x).max(screen.min.x));
    let r = Rect::from_min_size(pos2(x, y), size);
    shadow(&p, r, 8., 4, 14, (70. * k) as u8);
    p.rect_filled(r, rad(8.), t.s3.gamma_multiply(k));
    p.rect_stroke(r, rad(8.), egui::Stroke::new(1., t.tf.gamma_multiply(0.35 * k)), egui::StrokeKind::Inside);
    p.galley(Align2::CENTER_CENTER.anchor_size(r.center(), g.size()).min, g, t.tx);
}

pub trait Tip: Sized {
    fn tip(self, t: &Theme, text: &str) -> Self;
}

impl Tip for Response {
    fn tip(self, t: &Theme, text: &str) -> Self {
        tip(&self, t, text, false);
        self
    }
}
