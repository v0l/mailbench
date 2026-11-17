fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("mailbench"),
        ..Default::default()
    };
    eframe::run_native(
        "mailbench",
        options,
        Box::new(|cc| Ok(Box::new(mailbench::app::App::new(&cc.egui_ctx)))),
    )
}
