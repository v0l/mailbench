use mailbench::app::App;
use mailbench::backend::*;
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store};
use std::sync::Arc;
use std::time::Duration;

struct Demo {
    app: App,
    frames: u32,
    me: String,
}

fn key(ui: &egui::Ui, key: egui::Key, modifiers: egui::Modifiers) {
    ui.ctx().input_mut(|i| {
        i.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
    });
}

fn clipboard_image() -> Option<egui::ColorImage> {
    let out = std::process::Command::new("wl-paste").args(["--type", "image/png"]).output().ok()?;
    let img = image::load_from_memory(&out.stdout).ok()?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        match self.frames {
            10 => key(ui, egui::Key::C, Default::default()),
            20 => {
                if let Some(d) = self.app.draft.as_mut() {
                    d.to = format!("{}@localhost", self.me);
                    d.subject = "Photo of the board".into();
                    d.body = "Here is the board as it stands now:\n\n\nTell me what you think.".into();
                }
            }
            30 => {
                if let Some(img) = clipboard_image() {
                    ui.ctx().input_mut(|i| i.events.push(egui::Event::PasteImage(Arc::new(img))));
                    println!("pasted clipboard image");
                }
            }
            110 => key(ui, egui::Key::Enter, egui::Modifiers::COMMAND),
            260 => {
                self.app.folder = mailbench::mailstore::Folder::Inbox;
                self.app.selected = 0;
            }
            270 => key(ui, egui::Key::Enter, Default::default()),
            _ => {}
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.app.show(ui));
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

fn main() -> eframe::Result {
    let me = std::env::args().nth(1).expect("greenmail user");
    let dir = tempfile::tempdir().unwrap();
    let auth = Auth::Password { user: me.clone(), password: "secret".into() };
    let backend = Account {
        mail: Box::new(ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth.clone()))),
        outbound: Box::new(SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth))),
    };
    let setup = AccountSetup { id: "acct".into(), name: "demo".into(), address: format!("{me}@localhost"), sender: "Demo".into(), backend: Box::new(backend) };
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(store, vec![setup]);
    real.set_send_hold(Duration::ZERO);
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]), ..Default::default() };
    let _keep = dir;
    eframe::run_native(
        "mailbench",
        options,
        Box::new(move |cc| {
            mailbench::kit::setup(&cc.egui_ctx);
            Ok(Box::new(Demo { app: App::with_store(Box::new(real)), frames: 0, me }))
        }),
    )
}
