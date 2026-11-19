//! Toast notifications. `toast::error(..)` / `toast::info(..)` work from any thread; `show` paints them bottom-centre, above the mini player (`lift`).

use crate::theme::Theme;
use eframe::egui::{self, vec2, Align2, Color32, Context, FontId, Sense, Stroke};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LIFETIME: Duration = Duration::from_millis(3500);
const MAX_W: f32 = 420.;
const PAD_X: f32 = 18.;
const PAD_Y: f32 = 10.;
const MAX: usize = 4;

struct Toast {
    text: String,
    error: bool,
    until: Instant,
}

static QUEUE: Mutex<Vec<Toast>> = Mutex::new(Vec::new());
static CTX: Mutex<Option<Context>> = Mutex::new(None);

pub fn init(ctx: &Context) {
    *CTX.lock().unwrap() = Some(ctx.clone());
}

pub fn error(text: impl Into<String>) {
    push(text.into(), true);
}

pub fn info(text: impl Into<String>) {
    push(text.into(), false);
}

fn push(text: String, error: bool) {
    let until = Instant::now() + LIFETIME;
    {
        let mut q = QUEUE.lock().unwrap();
        // A repeated message refreshes the existing toast instead of stacking.
        match q.iter_mut().find(|t| t.text == text) {
            Some(t) => t.until = until,
            None => {
                q.push(Toast { text, error, until });
                if q.len() > MAX {
                    q.remove(0);
                }
            }
        }
    }
    if let Some(c) = &*CTX.lock().unwrap() {
        c.request_repaint();
    }
}

pub fn show(ctx: &Context, t: &Theme, lift: f32) {
    const ENTER: f32 = 0.12;
    let now = Instant::now();
    let mut q = QUEUE.lock().unwrap();
    q.retain(|x| x.until > now);
    let Some(soonest) = q.iter().map(|x| x.until).min() else { return };
    ctx.request_repaint_after(soonest - now);
    let mut dismissed = None;
    egui::Area::new(egui::Id::unique("toasts")).order(egui::Order::Tooltip).anchor(Align2::CENTER_BOTTOM, [0., -(16. + lift)]).show(ctx, |ui| ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.y = 8.;
        // Each toast is sized to its own text (wrapping only past MAX_W), never to a shared width.
        for (i, x) in q.iter().enumerate() {
            let age = (LIFETIME - (x.until - now)).as_secs_f32();
            let k = (age / ENTER).clamp(0., 1.);
            if k < 1. {
                ctx.request_repaint();
            }
            let e = k;
            let ink = if x.error { Color32::from_rgb(0xe5, 0x6b, 0x6b) } else { t.tx };
            let galley = ui.painter().layout(x.text.clone(), FontId::proportional(13.), ink.gamma_multiply(e), MAX_W - 2. * PAD_X);
            let size = vec2(galley.size().x + 2. * PAD_X, galley.size().y.max(16.) + 2. * PAD_Y);
            let (rect, r) = ui.allocate_exact_size(size, Sense::click());
            let p = ui.painter();
            p.rect(rect, 255., t.s2.gamma_multiply(e), Stroke::new(1., t.s3.gamma_multiply(e)), egui::StrokeKind::Inside);
            p.galley(rect.min + vec2(PAD_X, PAD_Y), galley, ink);
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                dismissed = Some(i);
            }
        }
    }));
    if let Some(i) = dismissed {
        q.remove(i);
    }
}
