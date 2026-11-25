//! Serafim desktop client on egui/eframe. Layout shell only.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[macro_use]
mod gfx;
mod about;
mod app;
mod audio;
mod backend;
mod data;
mod discord;
mod live;
mod library;
mod media;
mod menu;
mod page;
mod player;
mod stats;
mod theme;
mod toast;
mod tray;
#[cfg(feature = "video")]
mod video;

use eframe::egui;
use std::{cell::RefCell, rc::Rc, time::Duration};

type Shared = Rc<RefCell<Option<app::App>>>;

/// Lends the one long-lived `App` to each window eframe creates; eframe drops what it is given when a window closes.
struct Carry(Shared);

impl eframe::App for Carry {
    fn clear_color(&self, v: &egui::Visuals) -> [f32; 4] {
        self.0.borrow().as_ref().map_or([0.; 4], |a| eframe::App::clear_color(a, v))
    }

    fn save(&mut self, s: &mut dyn eframe::Storage) {
        if let Some(a) = self.0.borrow_mut().as_mut() {
            eframe::App::save(a, s);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, f: &mut eframe::Frame) {
        if let Some(a) = self.0.borrow_mut().as_mut() {
            eframe::App::ui(a, ui, f);
        }
    }
}

fn main() -> eframe::Result {
    tray::start(tray::acquire());
    let app: Shared = Rc::default();
    loop {
        let opts = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Serafim")
                .with_app_id("serafim")
                .with_inner_size([1280., 800.])
                .with_min_inner_size([800., 560.]),
            ..Default::default()
        };
        let shared = app.clone();
        eframe::run_native("Metrolist", opts, Box::new(move |cc| {
            // The scrutinee's borrow lasts the whole `match`, so create the app outside it.
            let mut slot = shared.borrow_mut();
            match slot.as_mut() {
                Some(a) => a.rebind(cc),
                None => *slot = Some(app::App::new(cc)),
            }
            drop(slot);
            toast::init(&cc.egui_ctx);
            tray::set_ctx(Some(cc.egui_ctx.clone()));
            Ok(Box::new(Carry(shared.clone())))
        }))?;
        tray::set_ctx(None);
        if !tray::hidden() {
            break;
        }
        // Window closed to the tray: keep playing until it is asked for again or the tray quits.
        let ctx = egui::Context::default();
        while !tray::take_show() && !tray::quitting() {
            if let Some(a) = app.borrow_mut().as_mut() {
                a.headless(&ctx);
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if tray::quitting() {
            break;
        }
    }
    backend::stop();
    Ok(())
}
