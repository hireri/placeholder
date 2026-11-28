//! Full-window view of a page's cover or artist picture: it grows out of the hero over a darkening scrim.
//! Click or Esc sends it back; right-click offers to copy or save it.

use crate::app::{App, Route};
use crate::gfx::*;
use crate::menu::{show_sized, Item};
use eframe::egui::{self, lerp, pos2, vec2, Area, Color32, Context, Id, Key, Order, Rect, Sense};
use std::io::Read;

const COPY: usize = 0;
const SAVE: usize = 1;

pub struct Viewer {
    pub url: String,
    title: String,
    /// The page it was opened on; leaving it drops the viewer.
    page: (Route, Option<(&'static str, String)>),
    /// The hero art's slot and corner radius, kept current by the hero.
    pub from: Rect,
    pub radius: f32,
    closing: bool,
    menu: Option<egui::Pos2>,
}

fn anim_id() -> Id {
    Id::NULL.with("viewer-anim")
}

/// The largest version of a cover worth showing or saving.
fn full_url(url: &str) -> String {
    // SoundCloud names its sizes instead of taking dimensions.
    sized_url(url, 1200).replace("-t300x300.", "-t500x500.")
}

/// Blocking: the picture's bytes, as served.
fn fetch(url: &str) -> Option<Vec<u8>> {
    let mut bytes = vec![];
    let full = ureq::get(&full_url(url)).call();
    // Not every picture has the larger variant, so fall back to the address the page uses.
    full.or_else(|_| ureq::get(url).call()).ok()?.into_reader().read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn copy(ctx: Context, url: String) {
    std::thread::spawn(move || match fetch(&url).and_then(|b| image::load_from_memory(&b).ok()) {
        Some(img) => {
            let img = img.to_rgba8();
            ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()));
            ctx.request_repaint();
            crate::toast::info("Image copied");
        }
        None => crate::toast::error("Couldn't copy the image"),
    });
}

/// A file name for `title` that every platform accepts, with the extension matching the bytes.
fn file_name(title: &str, bytes: &[u8]) -> String {
    let stem: String = title.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).take(80).collect();
    let stem = stem.trim().trim_start_matches('.');
    let ext = image::guess_format(bytes).ok().and_then(|f| f.extensions_str().first().copied()).unwrap_or("jpg");
    format!("{}.{ext}", if stem.is_empty() { "cover" } else { stem })
}

fn save(url: String, title: String) {
    std::thread::spawn(move || {
        let Some(bytes) = fetch(&url) else { return crate::toast::error("Couldn't download the image") };
        let Some(path) = rfd::FileDialog::new().set_title("Save image").set_file_name(file_name(&title, &bytes)).save_file() else { return };
        match std::fs::write(&path, bytes) {
            Ok(()) => crate::toast::info("Image saved"),
            Err(e) => crate::toast::error(format!("Couldn't save the image: {e}")),
        }
    });
}

impl App {
    pub fn open_viewer(&mut self, ctx: &Context, url: &str, title: &str, from: Rect, radius: f32) {
        // Seed the tween at 0 so the first frame starts on the hero.
        ctx.data_mut(|d| d.remove::<(f32, f32, f64)>(anim_id()));
        tween(ctx, anim_id(), 0., 1., ease_out_soft);
        self.ctx_menu = None;
        self.viewer = Some(Viewer { url: url.into(), title: title.into(), page: (self.route, self.sel.clone()), from, radius, closing: false, menu: None });
    }

    pub fn image_viewer(&mut self, ctx: &Context) {
        let t = self.t;
        let page = (self.route, self.sel.clone());
        let Some(v) = self.viewer.as_mut() else { return };
        if v.page != page {
            self.viewer = None;
            return;
        }
        let secs = if reduced_motion() { 0.001 } else if v.closing { 0.22 } else { 0.3 };
        let k = tween(ctx, anim_id(), if v.closing { 0. } else { 1. }, secs, ease_out_soft);
        if v.closing && k <= 0.001 {
            self.viewer = None;
            ctx.request_repaint(); // the hero draws its art again
            return;
        }
        let scr = ctx.content_rect();
        // The hero's own texture is already loaded; the large one replaces it when it arrives.
        let Some(small) = cover_texture(ctx, &v.url, v.from.size()) else {
            self.viewer = None;
            return;
        };
        let aspect = small.size.x / small.size.y;
        let h = (scr.height() * 0.82).min(scr.width() * 0.82 / aspect).min(800.);
        let to = Rect::from_center_size(scr.center(), vec2(h * aspect, h));
        let at = |a: egui::Pos2, b: egui::Pos2| pos2(lerp(a.x..=b.x, k), lerp(a.y..=b.y, k));
        let rect = Rect::from_min_max(at(v.from.min, to.min), at(v.from.max, to.max));
        let corners = rad(lerp(v.radius..=12., k));
        let large = full_url(&v.url);
        touch_cover(ctx, &large);
        let big = match egui::Image::new(large).load_for_size(ctx, to.size()) {
            Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture),
            _ => None,
        };

        let mut dismiss = false;
        let mut right = None;
        Area::new(Id::unique("viewer")).order(Order::Foreground).fixed_pos(scr.min).show(ctx, |ui| {
            let r = ui.allocate_rect(scr, Sense::click()); // also keeps clicks and the wheel off the page
            ui.painter().rect_filled(scr, 0., Color32::from_black_alpha((215. * k) as u8));
            shadow(ui.painter(), rect, corners.nw as f32, 16, 40, (120. * k) as u8);
            let tex = big.unwrap_or(small);
            // Cropped like the hero at the start, the whole picture by the end.
            let uv = crop_uv(tex.size.x / tex.size.y, rect.width() / rect.height());
            egui::Image::new(tex).fit_to_exact_size(rect.size()).uv(uv).corner_radius(corners).paint_at(ui, rect);
            dismiss = r.clicked();
            right = r.interact_pointer_pos().filter(|p| r.secondary_clicked() && rect.contains(*p));
        });

        // A press outside the menu, or Esc, only closes the menu.
        let had_menu = v.menu.is_some();
        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        let mut picked = None;
        if let Some(pos) = v.menu {
            let mut open = true;
            picked = show_sized(ctx, &t, "viewer-menu", &mut open, pos, Rect::NOTHING, &[Item::Action(COPY, "Copy image", Some(icon!("mlink"))), Item::Action(SAVE, "Save image…", Some(icon!("msave")))], 160.);
            if !open || picked.is_some() {
                v.menu = None;
            }
        }
        if right.is_some() && !v.closing {
            v.menu = right;
        } else if !had_menu && (dismiss || esc) {
            v.closing = true;
            ctx.request_repaint();
        }
        match picked {
            Some(COPY) => copy(ctx.clone(), v.url.clone()),
            Some(SAVE) => save(v.url.clone(), v.title.clone()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_names_are_safe_and_match_the_bytes() {
        assert_eq!(file_name("AC/DC: Live?", b"\x89PNG\r\n\x1a\n"), "AC_DC_ Live_.png");
        assert_eq!(file_name(" .. ", b"nonsense"), "cover.jpg");
    }

    #[test]
    fn full_size_addresses() {
        assert_eq!(full_url("https://x/a=w544-h544-l90-rj"), "https://x/a=w1200-h1200-l90-rj");
        assert_eq!(full_url("https://i1.sndcdn.com/a-t300x300.jpg"), "https://i1.sndcdn.com/a-t500x500.jpg");
    }
}
