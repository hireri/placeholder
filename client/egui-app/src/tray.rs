//! Single instance (a localhost lock) and the system tray. While a tray icon exists, closing the window hides it
//! and playback continues; "Quit" in the tray menu is the way out. Where windows can't be hidden (Wayland) the
//! window is destroyed instead and `main` recreates it on "show".

use eframe::egui::{Context, ViewportCommand};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::media::Cmd;

// ponytail: fixed port; swap for a per-user socket path if collisions with other software show up.
const LOCK_ADDR: &str = "127.0.0.1:47613";

static HAS_TRAY: AtomicBool = AtomicBool::new(false);
static DARK: AtomicBool = AtomicBool::new(true);
static QUITTING: AtomicBool = AtomicBool::new(false);
/// The window was closed to the tray and no longer exists; `main` keeps the player running until `show`.
static HIDDEN: AtomicBool = AtomicBool::new(false);
/// A show request that arrived while there was no window.
static WANT_SHOW: AtomicBool = AtomicBool::new(false);
/// The live window's context, `None` while hidden.
static CTX: Mutex<Option<Context>> = Mutex::new(None);
static CMDS: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());

/// Playback commands clicked in the tray menu, drained by the app each frame.
pub fn take_cmds() -> Vec<Cmd> {
    std::mem::take(&mut *CMDS.lock().unwrap())
}

pub fn set_ctx(ctx: Option<Context>) {
    *CTX.lock().unwrap() = ctx;
}

/// Wakes the window's UI thread, if there is a window.
pub fn wake() {
    if let Some(c) = &*CTX.lock().unwrap() {
        c.request_repaint();
    }
}

fn send(c: Cmd) {
    CMDS.lock().unwrap().push(c);
    wake();
}

pub fn quitting() -> bool {
    QUITTING.load(Ordering::Relaxed)
}

/// True after the window was closed to the tray.
pub fn hidden() -> bool {
    HIDDEN.swap(false, Ordering::Relaxed)
}

/// True once if a show was requested while hidden.
pub fn take_show() -> bool {
    WANT_SHOW.swap(false, Ordering::Relaxed)
}

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
    HAS_TRAY.load(Ordering::Relaxed) && !quitting()
}

/// Hides the window. Wayland can't hide one (winit ignores `Visible(false)`), so there the close goes through and the
/// window is destroyed; `main` notices `hidden()` and recreates it on `show`.
pub fn hide(ctx: &Context) {
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        HIDDEN.store(true, Ordering::Relaxed);
        return;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    ctx.send_viewport_cmd(ViewportCommand::Visible(false));
}

fn show() {
    let ctx = CTX.lock().unwrap().clone();
    let Some(ctx) = ctx else {
        WANT_SHOW.store(true, Ordering::Relaxed);
        return;
    };
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}

fn quit() {
    QUITTING.store(true, Ordering::Relaxed);
    if let Some(c) = &*CTX.lock().unwrap() {
        c.send_viewport_cmd(ViewportCommand::Close);
        c.request_repaint();
    }
}

/// The logo's small cut cropped to the glyph at 16/24/32/48 px, as (size, RGBA); the tray host picks the closest.
/// Monochrome: the logo's alpha filled white (dark panel) or black (light panel).
fn tray_icons(dark: bool) -> Vec<(u32, Vec<u8>)> {
    [&include_bytes!("../assets/logo/tray-16.png")[..], &include_bytes!("../assets/logo/tray-24.png")[..], &include_bytes!("../assets/logo/tray-32.png")[..], &include_bytes!("../assets/logo/tray-48.png")[..]]
        .map(|b| {
            let mut i = image::load_from_memory(b).expect("bundled tray icon").into_rgba8();
            let v = if dark { 255 } else { 0 };
            i.pixels_mut().for_each(|p| p.0[..3].fill(v));
            (i.width(), i.into_raw())
        })
        .into()
}

/// Repaints the tray icon for the panel's light/dark theme; cheap no-op when unchanged.
pub fn set_dark(dark: bool) {
    if HAS_TRAY.load(Ordering::Relaxed) && DARK.swap(dark, Ordering::Relaxed) != dark {
        platform::update(dark);
    }
}

pub fn start(lock: Option<TcpListener>) {
    if let Some(l) = lock {
        std::thread::spawn(move || l.incoming().flatten().for_each(|_| show()));
    }
    platform::tray();
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use ksni::{blocking::TrayMethods, menu::StandardItem, MenuItem};

    struct Tray {
        dark: bool,
    }

    static HANDLE: Mutex<Option<ksni::blocking::Handle<Tray>>> = Mutex::new(None);

    pub fn update(dark: bool) {
        if let Some(h) = &*HANDLE.lock().unwrap() {
            h.update(|t| t.dark = dark);
        }
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "metrolist".into()
        }
        fn title(&self) -> String {
            "Serafim".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            tray_icons(self.dark)
                .into_iter()
                .map(|(n, mut d)| {
                    d.chunks_exact_mut(4).for_each(|p| p.rotate_right(1)); // RGBA -> ARGB
                    ksni::Icon { width: n as i32, height: n as i32, data: d }
                })
                .collect()
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            show();
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem { label: "Play / Pause".into(), activate: Box::new(|_: &mut Self| send(Cmd::Toggle)), ..Default::default() }.into(),
                StandardItem { label: "Next".into(), activate: Box::new(|_: &mut Self| send(Cmd::Next)), ..Default::default() }.into(),
                StandardItem { label: "Previous".into(), activate: Box::new(|_: &mut Self| send(Cmd::Prev)), ..Default::default() }.into(),
                MenuItem::Separator,
                StandardItem { label: "Show Serafim".into(), activate: Box::new(|_: &mut Self| show()), ..Default::default() }.into(),
                StandardItem { label: "Quit".into(), activate: Box::new(|_: &mut Self| quit()), ..Default::default() }.into(),
            ]
        }
    }

    pub fn tray() {
        // Err means no StatusNotifier host (e.g. bare WM): leave HAS_TRAY false so the close button still quits.
        match (Tray { dark: DARK.load(Ordering::Relaxed) }).spawn() {
            Ok(h) => {
                HAS_TRAY.store(true, Ordering::Relaxed);
                *HANDLE.lock().unwrap() = Some(h); // static, so it is never dropped (a drop would remove the icon)
            }
            Err(e) => eprintln!("no system tray: {e}"),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    thread_local!(static ICON: std::cell::RefCell<Option<tray_icon::TrayIcon>> = const { std::cell::RefCell::new(None) });

    // Tray and UI both live on the main thread.
    pub fn update(dark: bool) {
        ICON.with(|t| {
            if let (Some(t), Ok(i)) = (&*t.borrow(), Icon::from_rgba(tray_icons(dark).swap_remove(2).1, 32, 32)) {
                let _ = t.set_icon(Some(i));
            }
        });
    }

    pub fn tray() {
        let menu = Menu::new();
        let [play_i, next_i, prev_i, show_i, quit_i] = ["Play / Pause", "Next", "Previous", "Show Serafim", "Quit"].map(|l| MenuItem::new(l, true, None));
        let _ = menu.append_items(&[&play_i, &next_i, &prev_i, &PredefinedMenuItem::separator(), &show_i, &quit_i]);
        let Ok(icon) = Icon::from_rgba(tray_icons(true).swap_remove(2).1, 32, 32) else { return };
        let Ok(t) = TrayIconBuilder::new().with_icon(icon).with_tooltip("Serafim").with_menu(Box::new(menu)).with_menu_on_left_click(false).build() else { return };
        let (quit_id, play_id, next_id, prev_id) = (quit_i.id().clone(), play_i.id().clone(), next_i.id().clone(), prev_i.id().clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| match e.id {
            id if id == quit_id => quit(),
            id if id == play_id => send(Cmd::Toggle),
            id if id == next_id => send(Cmd::Next),
            id if id == prev_id => send(Cmd::Prev),
            _ => show(),
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show();
            }
        }));
        HAS_TRAY.store(true, Ordering::Relaxed);
        ICON.with(|i| *i.borrow_mut() = Some(t));
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    pub fn tray() {}
    pub fn update(_: bool) {}
}
