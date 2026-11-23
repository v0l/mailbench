use mailbench::app::App;
use mailbench::chat::*;
use mailbench::fake::FakeStore;
use mailbench::store::{KdfParams, Store};
use std::time::Instant;

struct Demo {
    app: App,
    handle: FakeHandle,
    started: Instant,
    step: usize,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = self.started.elapsed().as_secs_f32();
        let narrow = ui.available_width() < 700.0;
        if let Some(view) = self.app.chats.as_mut() {
            let crew = RoomId::new("crew");
            let at = |s: usize, secs: f32| self.step == s && t > secs;
            if at(0, 3.0) {
                view.select(&crew);
                view.ctrl.create("Weekend plans", vec![MemberId::new("di"), MemberId::new("ed")]);
                self.step = 1;
            } else if at(1, 5.0) {
                if let Some(id) = view.ctrl.created.take().or_else(|| view.selected.clone()) {
                    view.select(&id);
                    view.draft = "Who is in for Saturday?".into();
                    view.submit();
                }
                self.step = 2;
            } else if at(2, 7.0) {
                let id = view.selected.clone().unwrap();
                self.handle.typing(&id, "di", true);
                self.handle.say(&id, "di", "I am in, bringing the grill.");
                self.handle.say(&id, "ed", "Count me in too.");
                self.step = 3;
            } else if at(3, 9.0) {
                let id = view.selected.clone().unwrap();
                let first = view.ctrl.state.thread(&id).iter().find(|m| m.body.starts_with("I am in")).map(|m| m.id.clone());
                if let Some(first) = first {
                    view.ctrl.react(&id, &first, "+1");
                    view.reply = Some(first);
                    view.draft = "Great, see you there".into();
                }
                self.step = 4;
            } else if at(4, 11.0) {
                view.submit();
                self.step = 5;
            } else if at(5, 13.0) {
                self.handle.set_offline(true);
                self.step = 6;
            } else if at(6, 15.0) {
                view.draft = "Sent while offline".into();
                view.submit();
                self.step = 7;
            } else if at(7, 19.0) {
                self.handle.set_offline(false);
                self.step = 8;
            }
        }
        let _ = narrow;
        egui::CentralPanel::no_frame().show(ui, |ui| self.app.show(ui));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

fn main() -> eframe::Result {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::create(&dir.path().join("demo.db"), None, Some(("pw", KdfParams::fast()))).expect("store");
    let (chat, handle) = FakeChat::demo();
    handle.set_auto_reply(true);
    let local = ChatStore::new(store, "demo");
    let mut slot = Some((chat, local));
    let mut app = App::with_store(Box::new(FakeStore::concept()));
    app.prefs.developer_chat = true;
    app.chat_opener = Some(Box::new(move || {
        let (chat, local) = slot.take()?;
        Some(ChatView::new(ChatController::start(Box::new(chat), Some(local))))
    }));
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]), ..Default::default() };
    let keep = dir;
    eframe::run_native(
        "mailbench",
        options,
        Box::new(move |cc| {
            let _keep = &keep;
            mailbench::kit::setup(&cc.egui_ctx);
            app.open_chats();
            Ok(Box::new(Demo { app, handle, started: Instant::now(), step: 0 }))
        }),
    )
}
