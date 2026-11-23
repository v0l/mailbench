use crate::insets::{InsetsPx, inset_rect};
use crate::shell::{Env, Shell, default_data_dir, surface_screen_rect};
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

fn window_insets_px(env: &mut JNIEnv, activity_ptr: usize) -> jni::errors::Result<Option<InsetsPx>> {
    let activity = unsafe { JObject::from_raw(activity_ptr as jni::sys::jobject) };
    let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
    let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
    let insets = env
        .call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?
        .l()?;
    if insets.is_null() {
        return Ok(None);
    }
    let mut side = |name: &str| -> jni::errors::Result<f32> {
        Ok(env.call_method(&insets, name, "()I", &[])?.i()? as f32)
    };
    Ok(Some(InsetsPx {
        left: side("getSystemWindowInsetLeft")?,
        top: side("getSystemWindowInsetTop")?,
        right: side("getSystemWindowInsetRight")?,
        bottom: side("getSystemWindowInsetBottom")?,
    }))
}

fn system_bar_px(activity_ptr: usize) -> InsetsPx {
    let ctx = ndk_context::android_context();
    let Ok(vm) = (unsafe { JavaVM::from_raw(ctx.vm().cast()) }) else { return InsetsPx::default() };
    let Ok(mut env) = vm.attach_current_thread() else { return InsetsPx::default() };
    let real = window_insets_px(&mut env, activity_ptr).unwrap_or(None);
    let top = system_dimen_px(&mut env, "status_bar_height").unwrap_or(0.0);
    let bottom = system_dimen_px(&mut env, "navigation_bar_height").unwrap_or(0.0);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    log::info!("insets real={real:?} dimens top={top} bottom={bottom}");
    real.unwrap_or(InsetsPx { top, bottom, ..Default::default() })
}

struct Insetted {
    shell: Shell,
    insets: InsetsPx,
    seen_size: egui::Vec2,
    activity_ptr: usize,
    app: android_activity::AndroidApp,
}

impl eframe::App for Insetted {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let size = self.app.native_window().map(|w| (w.width(), w.height()));
        if let Some(rect) = size.and_then(|(w, h)| surface_screen_rect(w, h, ctx.pixels_per_point())) {
            raw_input.screen_rect = Some(rect);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let size = ui.max_rect().size();
        if size != self.seen_size {
            self.seen_size = size;
            self.insets = system_bar_px(self.activity_ptr);
        }
        let rect = inset_rect(ui.max_rect(), self.insets, ui.ctx().pixels_per_point());
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
    let activity_ptr = app.activity_as_ptr() as usize;
    crate::android_bridge::set_activity(activity_ptr);
    let handle = app.clone();
    let opts = eframe::NativeOptions { android_app: Some(app), renderer: eframe::Renderer::Glow, ..Default::default() };
    let _ = eframe::run_native(
        "mailbench",
        opts,
        Box::new(|cc| {
            crate::kit::setup(&cc.egui_ctx);
            let shell = Shell::new(default_data_dir(), Rc::new(OsKeychain), Env::live(), None);
            Ok(Box::new(Insetted { shell, insets: InsetsPx::default(), seen_size: egui::Vec2::ZERO, activity_ptr, app: handle }))
        }),
    );
}
