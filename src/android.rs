use crate::shell::{Env, Shell, default_data_dir};
use crate::store::OsKeychain;
use crate::store::android::set_data_dir;
use std::rc::Rc;

#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    if let Some(dir) = app.internal_data_path() {
        set_data_dir(dir);
    }
    let opts = eframe::NativeOptions { android_app: Some(app), renderer: eframe::Renderer::Glow, ..Default::default() };
    let _ = eframe::run_native(
        "mailbench",
        opts,
        Box::new(|cc| {
            crate::kit::setup(&cc.egui_ctx);
            Ok(Box::new(Shell::new(default_data_dir(), Rc::new(OsKeychain), Env::live(), None)))
        }),
    );
}
