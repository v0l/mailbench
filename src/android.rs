use crate::shell::{Env, Shell, default_data_dir, surface_screen_rect};
use crate::store::OsKeychain;
use crate::store::android::set_data_dir;
use std::rc::Rc;

struct AndroidShell {
    app: android_activity::AndroidApp,
    shell: Shell,
}

impl eframe::App for AndroidShell {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let size = self.app.native_window().map(|w| (w.width(), w.height()));
        if let Some(rect) = size.and_then(|(w, h)| surface_screen_rect(w, h, ctx.pixels_per_point())) {
            raw_input.screen_rect = Some(rect);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.shell.ui(ui, frame);
    }
}

#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    if let Some(dir) = app.internal_data_path() {
        set_data_dir(dir);
    }
    let handle = app.clone();
    let opts = eframe::NativeOptions { android_app: Some(app), renderer: eframe::Renderer::Glow, ..Default::default() };
    let _ = eframe::run_native(
        "mailbench",
        opts,
        Box::new(|cc| {
            crate::kit::setup(&cc.egui_ctx);
            let shell = Shell::new(default_data_dir(), Rc::new(OsKeychain), Env::live(), None);
            Ok(Box::new(AndroidShell { app: handle, shell }))
        }),
    );
}
