use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::accounts::{self, AccountConfig, Credential};
use mailbench::autodetect::{Net, ServerSettings};
use mailbench::backend::*;
use mailbench::dns::Srv;
use mailbench::mailstore::SyncState;
use mailbench::settings_page::{Section, Tab};
use mailbench::shell::{Env, Shell};
use mailbench::store::{KdfParams, Keychain, Store, StoreError};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::result::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct MemKeychain {
    map: RefCell<HashMap<String, Vec<u8>>>,
}

impl Keychain for MemKeychain {
    fn get(&self, a: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.map.borrow().get(a).cloned())
    }
    fn set(&self, a: &str, s: &[u8]) -> Result<(), StoreError> {
        self.map.borrow_mut().insert(a.into(), s.to_vec());
        Ok(())
    }
    fn delete(&self, a: &str) -> Result<(), StoreError> {
        self.map.borrow_mut().remove(a);
        Ok(())
    }
}

struct NoNet;

impl Net for NoNet {
    fn get(&self, _: &str) -> Option<String> {
        None
    }
    fn srv(&self, _: &str) -> Vec<Srv> {
        Vec::new()
    }
    fn mx(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

fn rejecting_imap() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut out = stream.try_clone().unwrap();
                let mut input = BufReader::new(stream);
                let _ = out.write_all(b"* OK mock ready\r\n");
                loop {
                    let mut line = String::new();
                    if input.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.trim().splitn(2, ' ');
                    let tag = parts.next().unwrap_or("").to_string();
                    let cmd = parts.next().unwrap_or("").to_uppercase();
                    let reply = if cmd.starts_with("CAPABILITY") {
                        format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n")
                    } else if cmd.starts_with("LOGIN") || cmd.starts_with("AUTHENTICATE") {
                        format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
                    } else {
                        format!("{tag} OK done\r\n")
                    };
                    if out.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

fn work(port: u16) -> AccountConfig {
    AccountConfig {
        id: "a-imap".into(),
        name: "work".into(),
        display_name: String::new(),
        signature: String::new(),
        address: "me@work.test".into(),
        username: "me".into(),
        imap: ServerSettings::new("127.0.0.1", port, Security::Plain),
        smtp: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        credential: Credential::Password("wrong".into()),
        backfill: Default::default(),
        notify: Default::default(),
        sync: Default::default(),
    }
}

fn out_dir() -> std::path::PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("settings-shots"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_at(size: egui::Vec2, prefix: &str) {
    let port = rejecting_imap();
    let dir = tempfile::tempdir().unwrap();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    {
        let store = Store::create(&dir.path().join("mail.db"), Some(&*kc), None).unwrap();
        accounts::save(&store, &work(port)).unwrap();
    }
    let env = Env { net: Arc::new(NoNet), opener: Arc::new(|_| {}) };
    let shell = Shell::new(dir.path().to_path_buf(), kc, env, Some(KdfParams::fast()));
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, shell): &mut (bool, Shell)| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            shell.show(ui);
        },
        (false, shell),
    );
    h.run_steps(3);
    h.state_mut().1.open_settings();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run_steps(3);
        let failed = h.state().1.app().is_some_and(|a| a.store.accounts().iter().any(|x| x.sync == SyncState::Failed && x.detail.as_deref().is_some_and(|d| d.starts_with("authentication failed"))));
        if failed {
            break;
        }
        assert!(Instant::now() < deadline, "the account never failed to sign in");
        std::thread::sleep(Duration::from_millis(50));
    }
    {
        let f = h.state_mut().1.settings_form().unwrap();
        f.page.open_section(Section::Accounts);
        f.page.select_account(0);
        f.page.tab = Tab::Overview;
    }
    h.run_steps(6);
    h.get_by_label("sign in again");
    h.get_by_label_contains("needs you to sign in again");
    h.render().expect("render").save(out_dir().join(format!("{prefix}fault.png"))).unwrap();

    {
        let f = h.state_mut().1.settings_form().unwrap();
        f.drafts[0].signature = "Best, me".into();
        f.drafts[0].name = String::new();
    }
    h.run_steps(6);
    h.get_by_label_contains("Enter a name");
    h.get_by_label("discard");
    let apply = h.get_by_label("apply").rect();
    let bar = h.get_by_label("discard").rect();
    assert!(apply.right() <= size.x && bar.left() >= 0.0 && apply.bottom() <= size.y, "{apply:?} {bar:?}");
    h.render().expect("render").save(out_dir().join(format!("{prefix}validation.png"))).unwrap();

    h.state_mut().1.settings_form().unwrap().drafts[0].name = "work".into();
    h.run_steps(6);
    h.render().expect("render").save(out_dir().join(format!("{prefix}unsaved.png"))).unwrap();
}

#[test]
fn unsaved_bar_validation_and_the_sign_in_card_fit_on_desktop() {
    run_at(egui::vec2(1280.0, 800.0), "settings-");
}

#[test]
fn unsaved_bar_validation_and_the_sign_in_card_fit_on_a_phone() {
    run_at(egui::vec2(390.0, 844.0), "settings-phone-");
}
