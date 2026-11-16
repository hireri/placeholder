//! Single instance (a localhost lock) and the system tray. While a tray icon exists, closing the window hides it
//! and playback continues; "Quit" in the tray menu is the way out.

use eframe::egui::{Context, ViewportCommand};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};

// ponytail: fixed port; swap for a per-user socket path if collisions with other software show up.
const LOCK_ADDR: &str = "127.0.0.1:47613";

static HAS_TRAY: AtomicBool = AtomicBool::new(false);
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Takes the instance lock. If another instance holds it, asks that one to show its window and exits.
pub fn acquire() -> Option<TcpListener> {
    match TcpListener::bind(LOCK_ADDR) {
        Ok(l) => Some(l),
        Err(_) => {
            if let Ok(mut s) = TcpStream::connect(LOCK_ADDR) {
                let _ = s.write_all(b"show");
                std::process::exit(0);
            }
            None // something else owns the port: run anyway rather than refuse to start
        }
    }
}

/// True when the close button should hide to the tray instead of quitting.
pub fn hide_on_close() -> bool {
    HAS_TRAY.load(Ordering::Relaxed) && !QUITTING.load(Ordering::Relaxed)
}

pub fn hide(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    ctx.send_viewport_cmd(ViewportCommand::Visible(false));
}

fn show(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}

fn quit(ctx: &Context) {
    QUITTING.store(true, Ordering::Relaxed);
    ctx.send_viewport_cmd(ViewportCommand::Close);
    ctx.request_repaint();
}

/// Round accent-coloured placeholder until the app has a real icon asset.
fn icon_rgba(n: u32) -> Vec<u8> {
    let (c, r) = (n as f32 / 2., n as f32 / 2. - 1.);
    (0..n * n)
        .flat_map(|i| {
            let (x, y) = ((i % n) as f32 + 0.5 - c, (i / n) as f32 + 0.5 - c);
            let a = (r - (x * x + y * y).sqrt() + 0.5).clamp(0., 1.);
            [0x8b, 0x7c, 0xf6, (a * 255.) as u8]
        })
        .collect()
}

pub fn start(ctx: &Context, lock: Option<TcpListener>) {
    if let Some(l) = lock {
        let c = ctx.clone();
        std::thread::spawn(move || l.incoming().flatten().for_each(|_| show(&c)));
    }
    platform::tray(ctx);
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use ksni::{blocking::TrayMethods, menu::StandardItem, MenuItem};

    struct Tray(Context);

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "metrolist".into()
        }
        fn title(&self) -> String {
            "Metrolist".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let mut d = icon_rgba(32);
            d.chunks_exact_mut(4).for_each(|p| p.rotate_right(1)); // RGBA -> ARGB
            vec![ksni::Icon { width: 32, height: 32, data: d }]
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            show(&self.0);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem { label: "Show Metrolist".into(), activate: Box::new(|t: &mut Self| show(&t.0)), ..Default::default() }.into(),
                StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| quit(&t.0)), ..Default::default() }.into(),
            ]
        }
    }

    pub fn tray(ctx: &Context) {
        // Err means no StatusNotifier host (e.g. bare WM): leave HAS_TRAY false so the close button still quits.
        match Tray(ctx.clone()).spawn() {
            Ok(h) => {
                HAS_TRAY.store(true, Ordering::Relaxed);
                std::mem::forget(h); // the handle's drop would remove the icon; it lives for the whole run
            }
            Err(e) => eprintln!("no system tray: {e}"),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    pub fn tray(ctx: &Context) {
        let menu = Menu::new();
        let (show_i, quit_i) = (MenuItem::new("Show Metrolist", true, None), MenuItem::new("Quit", true, None));
        let _ = menu.append_items(&[&show_i, &quit_i]);
        let Ok(icon) = Icon::from_rgba(icon_rgba(32), 32, 32) else { return };
        let Ok(t) = TrayIconBuilder::new().with_icon(icon).with_tooltip("Metrolist").with_menu(Box::new(menu)).with_menu_on_left_click(false).build() else { return };
        let (c1, c2) = (ctx.clone(), ctx.clone());
        let quit_id = quit_i.id().clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| if e.id == quit_id { quit(&c1) } else { show(&c1) }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show(&c2);
            }
        }));
        HAS_TRAY.store(true, Ordering::Relaxed);
        std::mem::forget(t); // dropping removes the icon; keep it for the whole run
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    pub fn tray(_: &super::Context) {}
}
