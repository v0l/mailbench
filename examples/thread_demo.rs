use mailbench::app::App;
use mailbench::fake::{ConvSpec, FakeStore};

const TAIL: [&str; 4] = [
    "> Earlier in this thread we talked about the room.",
    "> The projector needs a long cable.",
    "> Parking is behind the building.",
    "> Doors open at nine.",
];

fn spec(from: &'static str, address: &'static str, to: &'static [&'static str], sent: &'static str, unread: bool, attach: bool, paragraphs: &'static [&'static str], quoted: &'static [&'static str]) -> ConvSpec {
    ConvSpec { from, address, to, when: "14:25", sent, unread, attach, account: 0, paragraphs, quoted }
}

struct Demo {
    app: App,
    frames: u32,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if self.frames == 3 {
            ui.ctx().input_mut(|i| {
                i.events.push(egui::Event::Key { key: egui::Key::Enter, physical_key: Some(egui::Key::Enter), pressed: true, repeat: false, modifiers: Default::default() });
            });
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.app.show(ui));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}

fn main() -> eframe::Result {
    let specs = vec![
        spec("Ann Byrne", "ann@byrne.example", &["me"], "Thu 23 Nov 14:25:02", false, false, &["Hi, can we meet on Friday to go over the assessment of needs request?"], &[]),
        spec("me", "mara.ilves@kestrel-instruments.example", &["Ann Byrne"], "Thu 23 Nov 14:25:40", false, false, &["Yes, Friday works for me."], &["> Hi, can we meet on Friday to go over the assessment of needs request?"]),
        spec("Ann Byrne", "ann@byrne.example", &["me"], "Thu 23 Nov 14:25:50", false, true, &["Great, my notes are attached."], &[]),
        spec("Ann Byrne", "ann@byrne.example", &["me"], "Thu 23 Nov 14:25:55", false, false, &["Also one more thing about the room, it holds twelve people and the projector needs a very long cable."], &TAIL),
        spec("Ann Byrne", "ann@byrne.example", &["me"], "Thu 23 Nov 14:26:30", true, false, &["Room 4 is booked for Friday at ten."], &TAIL),
    ];
    let store = FakeStore::conversation("Assesment of needs request", &specs);
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]), ..Default::default() };
    eframe::run_native(
        "mailbench",
        options,
        Box::new(move |cc| {
            mailbench::kit::setup(&cc.egui_ctx);
            Ok(Box::new(Demo { app: App::with_store(Box::new(store)), frames: 0 }))
        }),
    )
}
