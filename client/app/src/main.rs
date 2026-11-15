//! Metrolist desktop client (gpui). Layout shell only; daemon plumbing lives in `backend`.

#[macro_use]
mod ui;
mod assets;
mod backend;
mod library;
mod page;
mod player;
mod shell;
mod theme;

use gpui::*;

fn main() {
    Application::new().with_assets(assets::Assets).run(|cx: &mut App| {
        cx.set_global(theme::Theme::new(265., 0.14, 0.014, true));
        cx.on_window_closed(|cx| if cx.windows().is_empty() { cx.quit() }).detach();
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(800.)), cx))),
            window_min_size: Some(size(px(800.), px(560.))),
            titlebar: Some(TitlebarOptions { title: Some("Metrolist".into()), ..Default::default() }),
            app_id: Some("metrolist".into()),
            ..Default::default()
        };
        cx.open_window(opts, |_, cx| cx.new(shell::Shell::new)).unwrap();
        cx.activate(true);
    });
}
