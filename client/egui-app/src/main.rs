//! Metrolist desktop client on egui/eframe. Layout shell only.

#[macro_use]
mod gfx;
mod about;
mod app;
mod audio;
mod backend;
mod data;
mod live;
mod library;
mod media;
mod menu;
mod page;
mod player;
mod stats;
mod theme;
#[cfg(feature = "video")]
mod video;

use eframe::egui;

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Metrolist")
            .with_app_id("metrolist")
            .with_inner_size([1280., 800.])
            .with_min_inner_size([800., 560.]),
        ..Default::default()
    };
    eframe::run_native("Metrolist", opts, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
