use mailbench::app::App;
use mailbench::backend::*;
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store};
use std::time::{Duration, Instant};

struct Offline;

#[async_trait::async_trait]
impl Backend for Offline {
    fn kind(&self) -> &'static str {
        "offline"
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        Err(BackendError::Connection("offline".into()))
    }
}

const MESSAGES: u32 = 100_000;
const BODIES: u32 = 150;

fn raw_body(uid: u32) -> Vec<u8> {
    let text = format!("Hello number {uid}, here is the news.\r\n").repeat(200);
    format!(
        "From: Sender {s} <s{s}@example{d}.test>\r\nTo: me@example.test\r\nSubject: Newsletter number {uid}\r\nMessage-ID: <{uid}@example.test>\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}",
        s = uid % 300,
        d = uid % 17
    )
    .into_bytes()
}

struct Walker {
    app: App,
    started: Instant,
    pressed: u32,
    last: Instant,
}

impl eframe::App for Walker {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.started.elapsed() > Duration::from_secs(8) && self.last.elapsed() > Duration::from_millis(250) && self.pressed < 80 {
            self.last = Instant::now();
            self.pressed += 1;
            raw.events.push(egui::Event::Key { key: egui::Key::J, physical_key: Some(egui::Key::J), pressed: true, repeat: false, modifiers: Default::default() });
            raw.events.push(egui::Event::Key { key: egui::Key::J, physical_key: Some(egui::Key::J), pressed: false, repeat: false, modifiers: Default::default() });
        }
        if self.pressed >= 80 && self.last.elapsed() > Duration::from_secs(2) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::no_frame().show(ui, |ui| self.app.show(ui));
    }
}

fn main() -> eframe::Result {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
    {
        let mut sink = StoreSink::new(&store, "acct", true);
        sink.put_folders(&[Folder { name: "INBOX".into(), delimiter: None, role: Some(FolderRole::Inbox), selectable: true }]).unwrap();
        for batch in (1..=MESSAGES).collect::<Vec<_>>().chunks(5000) {
            let messages: Vec<_> = batch
                .iter()
                .map(|&uid| MessageSummary {
                    uid,
                    subject: Some(format!("Newsletter number {uid}")),
                    from: vec![Address { name: Some(format!("Sender {}", uid % 300)), address: format!("s{}@example{}.test", uid % 300, uid % 17) }],
                    to: vec![Address { name: None, address: "me@example.test".into() }],
                    date: Some(1_700_000_000 + uid as i64),
                    flags: Flags { seen: uid % 3 != 0, ..Default::default() },
                    ..Default::default()
                })
                .collect();
            sink.put_summaries("INBOX", &messages).unwrap();
        }
        sink.put_token("INBOX", &SyncToken { uid_validity: 7, uid_next: MESSAGES + 1, highest_mod_seq: None }).unwrap();
        for uid in (MESSAGES - BODIES + 1)..=MESSAGES {
            sink.put_body("INBOX", uid, &raw_body(uid)).unwrap();
        }
    }
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "test".into(), address: "me@example.test".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 720.0]), ..Default::default() };
    eframe::run_native(
        "mailbench",
        options,
        Box::new(move |cc| {
            mailbench::kit::setup(&cc.egui_ctx);
            Ok(Box::new(Walker { app: App::with_store(Box::new(real)), started: Instant::now(), pressed: 0, last: Instant::now() }))
        }),
    )
}
