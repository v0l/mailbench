use mailbench::accounts;
use mailbench::autodetect::ServerSettings;
use mailbench::backend::Security;
use mailbench::settings_page::{Section, Tab};
use mailbench::shell::{Env, Shell};
use mailbench::store::{Keychain, Result, Store, StoreError, Unlock};
use std::path::PathBuf;
use std::rc::Rc;

struct FileKc(PathBuf);

impl Keychain for FileKc {
    fn get(&self, a: &str) -> Result<Option<Vec<u8>>> {
        Ok(std::fs::read(self.0.join(a)).ok())
    }
    fn set(&self, a: &str, s: &[u8]) -> Result<()> {
        std::fs::write(self.0.join(a), s).map_err(|e| StoreError::Keychain(e.to_string()))
    }
    fn delete(&self, a: &str) -> Result<()> {
        let _ = std::fs::remove_file(self.0.join(a));
        Ok(())
    }
}

fn seed(dir: &std::path::Path, kc: &FileKc) {
    let store = Store::create(&dir.join("mail.db"), Some(kc), None).unwrap();
    let mut imap = accounts::jmap_config("http://127.0.0.1:1/jmap", "me@work.test", "me", "x");
    imap.id = "a-imap".into();
    imap.name = "work".into();
    imap.credential = accounts::Credential::Password("x".into());
    imap.imap = ServerSettings::new("imap.work.test", 993, Security::Tls);
    imap.smtp = ServerSettings::new("smtp.work.test", 587, Security::StartTls);
    let mut jmap = accounts::jmap_config("http://127.0.0.1:1/jmap", "ann@fast.test", "ann", "pw");
    jmap.id = "b-jmap".into();
    jmap.name = "fast".into();
    let mut nostr = accounts::nostr_config(&"0a".repeat(32), vec!["wss://relay.example".into()]).unwrap();
    nostr.id = "c-nostr".into();
    nostr.name = "carol".into();
    for c in [imap, jmap, nostr] {
        accounts::save(&store, &c).unwrap();
    }
    drop(store);
}

struct Demo {
    shell: Shell,
    section: Option<Section>,
    tab: Option<Tab>,
    account: usize,
    edit: Option<String>,
    frames: u32,
    comma: bool,
    quit_after: u32,
}

impl eframe::App for Demo {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let key = |k: egui::Key| egui::Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: Default::default() };
        if self.comma && self.frames == 20 {
            raw.events.push(key(egui::Key::Comma));
        }
        if self.comma && self.frames == 200 {
            raw.events.push(key(egui::Key::Escape));
        }
        if self.frames == 20 {
            if let Some(text) = self.edit.clone() {
                if let Some(f) = self.shell.settings_form() {
                    f.drafts[self.account].signature = text;
                }
                raw.events.push(egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: Some(egui::Key::S),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                });
            }
        }
        if self.frames > self.quit_after {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        if self.frames == 5 && !self.comma {
            self.shell.open_settings();
        }
        if self.frames == 8 {
            if let Some(f) = self.shell.settings_form() {
                if let Some(s) = self.section {
                    f.page.open_section(s);
                    if s == Section::Accounts {
                        f.focus_account(["a-imap", "b-jmap", "c-nostr"][self.account]);
                        if let Some(t) = self.tab {
                            f.page.tab = t;
                        }
                    }
                }
            }
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.shell.show(ui));
    }
}

fn parse_section(name: &str) -> Option<Section> {
    Section::ALL.into_iter().find(|s| s.label() == name)
}

fn parse_tab(name: &str) -> Option<Tab> {
    Tab::ALL.into_iter().find(|t| t.label() == name)
}

fn main() -> eframe::Result {
    let dir = PathBuf::from(std::env::var("DEMO_DIR").expect("DEMO_DIR"));
    std::fs::create_dir_all(dir.join("kc")).unwrap();
    let kc = FileKc(dir.join("kc"));
    if !dir.join("mail.db").exists() {
        seed(&dir, &kc);
    }
    let _ = Unlock::Keychain(&kc);
    let section = std::env::var("SECTION").ok().and_then(|s| parse_section(&s));
    let tab = std::env::var("TAB").ok().and_then(|s| parse_tab(&s));
    let account = std::env::var("ACCOUNT").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let edit = std::env::var("EDIT_SIGNATURE").ok();
    let quit_after = std::env::var("QUIT_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let (w, h) = std::env::var("SIZE").ok().and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280.0), b.parse().unwrap_or(800.0)))).unwrap_or((1280.0, 800.0));
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([w, h]), ..Default::default() };
    eframe::run_native(
        "mailbench settings demo",
        options,
        Box::new(move |cc| {
            mailbench::kit::setup(&cc.egui_ctx);
            let env = Env { net: std::sync::Arc::new(mailbench::autodetect::Live), opener: std::sync::Arc::new(|_| {}) };
            let shell = Shell::new(dir, Rc::new(kc), env, Some(mailbench::store::KdfParams::fast()));
            Ok(Box::new(Demo { shell, section, tab, account, edit, frames: 0, comma: std::env::var("COMMA").is_ok(), quit_after }))
        }),
    )
}
