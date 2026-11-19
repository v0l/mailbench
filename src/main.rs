use clap::Parser;
use mailbench::cli::{Cli, run};
use mailbench::shell::{Env, Shell, default_data_dir};
use mailbench::store::OsKeychain;
use std::rc::Rc;

fn main() -> eframe::Result {
    let cli = Cli::parse();
    if cli.command.is_some() {
        if let Err(e) = run(&cli) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let demo = cli.demo;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("mailbench"),
        ..Default::default()
    };
    eframe::run_native(
        "mailbench",
        options,
        Box::new(move |cc| {
            if demo {
                return Ok(Box::new(mailbench::app::App::new(&cc.egui_ctx)) as Box<dyn eframe::App>);
            }
            mailbench::kit::setup(&cc.egui_ctx);
            Ok(Box::new(Shell::new(default_data_dir(), Rc::new(OsKeychain), Env::live(), None)))
        }),
    )
}
