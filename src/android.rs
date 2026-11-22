use crate::insets::inset_rect;
use crate::shell::{Env, Shell, default_data_dir};
use crate::store::OsKeychain;
use crate::store::android::set_data_dir;
use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use std::rc::Rc;

fn system_dimen_px(env: &mut JNIEnv, name: &str) -> jni::errors::Result<f32> {
    let ctx = ndk_context::android_context();
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let resources = env.call_method(&activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
    let jname = env.new_string(name)?;
    let jtype = env.new_string("dimen")?;
    let jpkg = env.new_string("android")?;
    let id = env
        .call_method(
            &resources,
            "getIdentifier",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
            &[JValue::Object(&jname), JValue::Object(&jtype), JValue::Object(&jpkg)],
        )?
        .i()?;
    if id == 0 {
        return Ok(0.0);
    }
    let px = env.call_method(&resources, "getDimensionPixelSize", "(I)I", &[JValue::Int(id)])?.i()?;
    Ok(px as f32)
}

fn system_bar_px() -> (f32, f32) {
    let ctx = ndk_context::android_context();
    let Ok(vm) = (unsafe { JavaVM::from_raw(ctx.vm().cast()) }) else { return (0.0, 0.0) };
    let Ok(mut env) = vm.attach_current_thread() else { return (0.0, 0.0) };
    let top = system_dimen_px(&mut env, "status_bar_height");
    let bottom = system_dimen_px(&mut env, "navigation_bar_height");
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    (top.unwrap_or(0.0), bottom.unwrap_or(0.0))
}

struct Insetted {
    shell: Shell,
    top_px: f32,
    bottom_px: f32,
}

impl eframe::App for Insetted {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let rect = inset_rect(ui.max_rect(), self.top_px, self.bottom_px, ui.ctx().pixels_per_point());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        self.shell.ui(&mut child, frame);
    }
}

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
            let (top_px, bottom_px) = system_bar_px();
            let shell = Shell::new(default_data_dir(), Rc::new(OsKeychain), Env::live(), None);
            Ok(Box::new(Insetted { shell, top_px, bottom_px }))
        }),
    );
}
