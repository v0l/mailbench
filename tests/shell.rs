use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use mailbench::autodetect::{Net, Provider};
use mailbench::backend::*;
use mailbench::dns::Srv;
use mailbench::mailstore::{self as ms, Query};
use mailbench::shell::{Env, Protect, Shell};
use mailbench::store::{Keychain, KdfParams, Store, StoreError};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::result::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod common;

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

struct DeadKeychain;

impl Keychain for DeadKeychain {
    fn available(&self) -> bool {
        false
    }
    fn get(&self, _: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Err(StoreError::Keychain("no secret service".into()))
    }
    fn set(&self, _: &str, _: &[u8]) -> Result<(), StoreError> {
        Err(StoreError::Keychain("no secret service".into()))
    }
    fn delete(&self, _: &str) -> Result<(), StoreError> {
        Err(StoreError::Keychain("no secret service".into()))
    }
}

#[derive(Default)]
struct FixtureNet {
    pages: HashMap<String, String>,
}

impl Net for FixtureNet {
    fn get(&self, url: &str) -> Option<String> {
        self.pages.iter().find(|(k, _)| url.starts_with(k.as_str())).map(|(_, v)| v.clone())
    }
    fn srv(&self, _: &str) -> Vec<Srv> {
        Vec::new()
    }
    fn mx(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

fn env(net: FixtureNet) -> Env {
    Env { net: Arc::new(net), opener: Arc::new(|_| {}) }
}

const AUTOCONFIG: &str = r#"<clientConfig version="1.1"><emailProvider id="example.test">
<incomingServer type="imap"><hostname>imap.example.test</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
<outgoingServer type="smtp"><hostname>smtp.example.test</hostname><port>587</port><socketType>STARTTLS</socketType><username>%EMAILADDRESS%</username></outgoingServer>
</emailProvider></clientConfig>"#;

type State = (bool, Shell);

fn harness(shell: Shell) -> Harness<'static, State> {
    harness_sized(shell, egui::vec2(1280.0, 800.0))
}

fn harness_sized(shell: Shell, size: egui::Vec2) -> Harness<'static, State> {
    Harness::builder().with_size(size).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, shell): &mut State| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            shell.show(ui);
        },
        (false, shell),
    )
}

fn boot(dir: &Path, kc: Rc<dyn Keychain>, env: Env) -> Harness<'static, State> {
    let mut h = harness(Shell::new(dir.to_path_buf(), kc, env, Some(KdfParams::fast())));
    h.run();
    h.run();
    h
}

fn until(h: &mut Harness<'static, State>, what: &str, cond: impl Fn(&mut Shell) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        h.run_steps(3);
        if cond(&mut h.state_mut().1) {
            h.run_steps(3);
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn first_run_with_password(h: &mut Harness<'static, State>, pw: &str) {
    h.get_by_label_contains("Use a password").click();
    h.run();
    {
        let f = h.state_mut().1.first_form().expect("first run screen");
        f.password = pw.into();
        f.confirm = pw.into();
    }
    h.run();
    h.get_by_label("continue").click();
    h.run();
    h.run();
}

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn db_exists(d: &Path) -> bool {
    d.join("mail.db").exists()
}

#[test]
fn first_run_with_password_then_restart_unlocks_and_wrong_password_errors() {
    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), kc.clone(), env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "first run");
    h.get_by_label_contains("Everything mailbench keeps on this machine is encrypted");
    first_run_with_password(&mut h, "correct horse battery");
    assert!(db_exists(d.path()));
    assert_eq!(h.state().1.screen_name(), "add account", "no accounts yet, so the first account is added next");
    drop(h);

    let mut h = boot(d.path(), kc.clone(), env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "unlock");
    h.state_mut().1.unlock_form().unwrap().password = "not the password".into();
    h.run();
    h.get_by_label("unlock").click();
    h.step();
    until(&mut h, "wrong password reported", |s| !s.unlock_form().is_some_and(|f| f.busy()));
    assert_eq!(h.state().1.screen_name(), "unlock");
    h.get_by_label_contains("Wrong password");
    h.state_mut().1.unlock_form().unwrap().password = "correct horse battery".into();
    h.run();
    h.get_by_label("unlock").click();
    h.step();
    until(&mut h, "unlocked", |s| s.screen_name() != "unlock");
    assert_eq!(h.state().1.screen_name(), "add account");
}

#[test]
fn first_run_with_keychain_restarts_without_an_unlock_screen() {
    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), kc.clone(), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.screen_name(), "add account");
    drop(h);
    let h = boot(d.path(), kc, env(FixtureNet::default()));
    assert_ne!(h.state().1.screen_name(), "unlock");
    assert_eq!(h.state().1.screen_name(), "add account");
}

#[test]
fn dead_keychain_on_first_run_says_so_and_a_password_still_works() {
    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(DeadKeychain);
    let mut h = boot(d.path(), kc, env(FixtureNet::default()));
    h.run();
    assert_eq!(h.state().1.screen_name(), "first run");
    h.get_by_label_contains("No system keychain found");
    assert_eq!(h.state_mut().1.first_form().unwrap().mode, Protect::Password);
    assert!(!db_exists(d.path()));
    first_run_with_password(&mut h, "correct horse battery");
    assert_eq!(h.state().1.screen_name(), "add account");
}

#[test]
fn keychain_failing_on_a_later_start_falls_back_to_the_password() {
    let d = dir();
    let good: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), good, env(FixtureNet::default()));
    h.get_by_label("Use both").click();
    h.run();
    {
        let f = h.state_mut().1.first_form().unwrap();
        f.password = "correct horse battery".into();
        f.confirm = "correct horse battery".into();
    }
    h.run();
    h.get_by_label("continue").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.screen_name(), "add account");
    drop(h);

    let mut h = boot(d.path(), Rc::new(DeadKeychain), env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "unlock");
    h.get_by_label_contains("keychain is not available");
    h.state_mut().1.unlock_form().unwrap().password = "correct horse battery".into();
    h.run();
    h.get_by_label("unlock").click();
    h.step();
    until(&mut h, "unlocked", |s| s.screen_name() != "unlock");
    assert_eq!(h.state().1.screen_name(), "add account");
}

#[test]
fn a_slow_password_unlock_keeps_the_screen_responsive_and_shows_progress() {
    let d = dir();
    let slow = KdfParams { memory_kib: 64 * 1024, iterations: 6, lanes: 1 };
    Store::create(&d.path().join("mail.db"), None, Some(("correct horse battery", slow))).unwrap();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), kc, env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "unlock");
    h.state_mut().1.unlock_form().unwrap().password = "correct horse battery".into();
    h.run();
    h.get_by_label("unlock").click();
    let frame = Instant::now();
    h.step();
    assert!(frame.elapsed() < Duration::from_millis(150), "the frame after unlock took {:?}", frame.elapsed());
    assert!(h.state_mut().1.unlock_form().is_some_and(|f| f.busy()));
    h.step();
    h.get_by_label_contains("unlocking");
    until(&mut h, "unlocked", |s| s.screen_name() != "unlock");
    assert_eq!(h.state().1.screen_name(), "add account");
}

#[test]
fn password_shorter_than_eight_characters_cannot_continue() {
    let d = dir();
    let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
    h.get_by_label_contains("Use a password").click();
    h.run();
    {
        let f = h.state_mut().1.first_form().unwrap();
        f.password = "short".into();
        f.confirm = "short".into();
    }
    h.run();
    h.get_by_label("continue").click();
    h.run();
    assert_eq!(h.state().1.screen_name(), "first run");
    assert!(!db_exists(d.path()));
}

#[test]
fn autodetect_fills_the_server_settings_from_autoconfig() {
    let d = dir();
    let mut net = FixtureNet::default();
    net.pages.insert("https://autoconfig.example.test/mail/config-v1.1.xml".into(), AUTOCONFIG.into());
    let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(net));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.state_mut().1.add_form().unwrap().email = "mara@example.test".into();
    h.run();
    h.get_by_label("detect").click();
    until(&mut h, "detected settings", |s| s.add_form().unwrap().imap_host == "imap.example.test");
    let f = h.state_mut().1.add_form().unwrap();
    assert_eq!((f.imap_port.as_str(), f.smtp_host.as_str(), f.smtp_port.as_str()), ("993", "smtp.example.test", "587"));
    assert_eq!(f.username, "mara@example.test");
    h.get_by_label_contains("Found settings from the provider autoconfig");
}

#[test]
fn gmail_domain_without_an_oauth_client_offers_app_passwords() {
    let d = dir();
    let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    {
        let f = h.state_mut().1.add_form().unwrap();
        f.email = "someone@gmail.com".into();
    }
    h.run();
    h.get_by_label("detect").click();
    until(&mut h, "google settings", |s| s.add_form().unwrap().provider == Some(Provider::Google));
    h.get_by_label_contains("app password");
    assert_eq!(h.state_mut().1.add_form().unwrap().imap_host, "imap.gmail.com");
}

fn fill_greenmail(h: &mut Harness<'static, State>, name: &str, password: &str) {
    {
        let f = h.state_mut().1.add_form().unwrap();
        f.email = format!("{name}@localhost");
        f.username = name.into();
        f.password = password.into();
        f.imap_host = "127.0.0.1".into();
        f.imap_port = common::GM_IMAP.to_string();
        f.imap_security = Security::Plain;
        f.smtp_host = "127.0.0.1".into();
        f.smtp_port = common::GM_SMTP.to_string();
        f.smtp_security = Security::Plain;
        f.edit = true;
    }
    h.run();
}

fn greenmail_user() -> String {
    format!("lf{}", std::process::id())
}

fn jmap_user(tag: &str) -> String {
    assert!(common::stalwart(), "docker is required for stalwart tests");
    let name = format!("sh{tag}{}", std::process::id());
    common::create_stalwart_user(&name);
    name
}

#[test]
fn add_jmap_account_through_the_form_syncs_opens_and_sends() {
    if !common::stalwart() {
        return;
    }
    let name = jmap_user("ok");
    let address = format!("{name}@example.org");
    let mut seed = JmapBackend::new(JmapConfig::new(
        &format!("http://127.0.0.1:{}", common::SW_HTTP),
        Auth::Password { user: name.clone(), password: "secret".into() },
    ));
    mailbench::runtime::block_on(seed.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: address.clone() },
        to: vec![Address { name: None, address: address.clone() }],
        subject: "Hello over JMAP".into(),
        body: "first jmap body".into(),
        ..Default::default()
    }))
    .unwrap();

    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), kc.clone(), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.get_by_label_contains("JMAP account").click();
    h.run();
    {
        let f = h.state_mut().1.add_form().unwrap();
        f.email = address.clone();
        f.username = name.clone();
        f.password = "secret".into();
        f.jmap_url = format!("http://127.0.0.1:{}", common::SW_HTTP);
    }
    save(&mut h, "jmap-add-account");
    assert!(h.get_by_label("save and sync").accesskit_node().is_disabled());
    h.get_by_label("test connection").click();
    until(&mut h, "both steps ok", |s| s.add_form().unwrap().test.all_ok());
    save(&mut h, "jmap-add-account-tested");
    h.get_by_label("save and sync").click();
    h.run();
    assert_eq!(h.state().1.screen_name(), "mail");
    until(&mut h, "message synced", |s| s.app().is_some_and(|a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 1));
    {
        let app = h.state().1.app().unwrap();
        assert_eq!(app.store.list(&Query::new(ms::Folder::Inbox), 0, 5).items[0].subject, "Hello over JMAP");
    }
    h.key_press(egui::Key::Enter);
    until(&mut h, "body shown", |s| s.app().is_some_and(|a| a.store.list(&Query::new(ms::Folder::Inbox), 0, 1).items.first().is_some_and(|m| a.store.get(m.id).is_ok_and(|m| !m.paragraphs.is_empty()))));
    h.get_by_label_contains("first jmap body");
    save(&mut h, "jmap-inbox-reader");

    h.key_press(egui::Key::C);
    h.run();
    h.run();
    {
        let app = h.state_mut().1.app_mut().unwrap();
        let draft = app.draft.as_mut().expect("composer");
        draft.to = address.clone();
        draft.subject = "Sent from the UI".into();
        draft.body = "second jmap body".into();
    }
    save(&mut h, "jmap-composer");
    h.get_by_label("send").click();
    until(&mut h, "sent mail arrives in the inbox", |s| s.app().is_some_and(|a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 2));
    let subjects: Vec<String> = h.state().1.app().unwrap().store.list(&Query::new(ms::Folder::Inbox), 0, 5).items.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.contains(&"Sent from the UI".to_string()), "{subjects:?}");
    save(&mut h, "jmap-after-send");
    drop(h);

    let mut h = boot(d.path(), kc, env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "mail", "the saved jmap account is back after a restart");
    until(&mut h, "messages from the store", |s| s.app().is_some_and(|a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 2));
}

#[test]
fn jmap_form_rejects_a_wrong_password_and_keeps_saving_disabled() {
    if !common::stalwart() {
        return;
    }
    let name = jmap_user("bad");
    let d = dir();
    let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.get_by_label_contains("JMAP account").click();
    h.run();
    {
        let f = h.state_mut().1.add_form().unwrap();
        f.email = format!("{name}@example.org");
        f.password = "wrong".into();
        f.jmap_url = format!("http://127.0.0.1:{}", common::SW_HTTP);
    }
    h.run();
    h.get_by_label("test connection").click();
    until(&mut h, "test finished", |s| !s.add_form().unwrap().test.running());
    h.get_by_label_contains("authentication failed");
    assert!(h.get_by_label("save and sync").accesskit_node().is_disabled());
}

#[test]
fn jmap_form_fits_at_phone_width() {
    let d = dir();
    let mut h = harness_sized(
        Shell::new(d.path().to_path_buf(), Rc::new(MemKeychain::default()), env(FixtureNet::default()), Some(KdfParams::fast())),
        egui::vec2(390.0, 844.0),
    );
    h.run();
    h.run();
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.get_by_label_contains("JMAP account").click();
    h.run();
    h.run();
    let key = h.get_by_label("test connection").rect();
    assert!(key.right() <= 390.0 && key.left() >= 0.0, "test connection at {key:?}");
    let img = h.render().expect("render");
    img.save(out_dir().join("phone-jmap-add-account.png")).unwrap();
    {
        let f = h.state_mut().1.add_form().unwrap();
        f.email = "someone@example.org".into();
        f.username = "someone".into();
        f.password = "secret".into();
        f.jmap_url = "https://api.fastmail.com/jmap/session".into();
        f.test.for_key = f.key();
        f.test.results = vec![(mailbench::accounts::Step::Imap, Ok(())), (mailbench::accounts::Step::Smtp, Ok(()))];
    }
    h.run();
    h.run();
    for text in ["JMAP session: connected and signed in", "sending (EmailSubmission): ready"] {
        let r = h.get_by_label_contains(text).rect();
        assert!(r.left() >= 0.0 && r.right() <= 390.0 && r.bottom() <= 844.0, "{text} at {r:?}");
    }
    for label in ["test connection", "save and sync"] {
        let r = h.get_by_label(label).rect();
        assert!(r.left() >= 0.0 && r.right() <= 390.0 && r.bottom() <= 844.0, "{label} at {r:?}");
    }
    let img = h.render().expect("render");
    img.save(out_dir().join("phone-jmap-add-account-tested.png")).unwrap();
}

fn setup_greenmail_account(h: &mut Harness<'static, State>, name: &str) {
    h.get_by_label("continue").click();
    h.run();
    h.run();
    fill_greenmail(h, name, "secret");
}

#[test]
fn add_account_against_greenmail_tests_then_saves_and_syncs_the_inbox() {
    if !common::greenmail() {
        return;
    }
    let name = greenmail_user();
    common::create_greenmail_user(&name);
    let mut smtp = SmtpBackend::new(SmtpConfig::new("127.0.0.1", common::GM_SMTP, Security::Plain, Auth::Password { user: name.clone(), password: "secret".into() }));
    mailbench::runtime::block_on(smtp.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "Login flow hello".into(),
        body: "first sync body".into(),
        ..Default::default()
    }))
    .unwrap();
    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), kc.clone(), env(FixtureNet::default()));
    setup_greenmail_account(&mut h, &name);
    h.get_by_label("test connection").click();
    until(&mut h, "both steps ok", |s| s.add_form().unwrap().test.all_ok());
    h.get_by_label_contains("incoming mail (IMAP): connected and signed in");
    h.get_by_label_contains("outgoing mail (SMTP): connected and signed in");
    h.get_by_label("save and sync").click();
    h.run();
    assert_eq!(h.state().1.screen_name(), "mail");
    until(&mut h, "message synced", |s| {
        s.app().is_some_and(|a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 1)
    });
    let app = h.state().1.app().unwrap();
    assert_eq!(app.store.list(&Query::new(ms::Folder::Inbox), 0, 5).items[0].subject, "Login flow hello");
    drop(h);

    let mut h = boot(d.path(), kc, env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "mail", "the saved account is back after a restart");
    until(&mut h, "message from the store", |s| {
        s.app().is_some_and(|a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 1)
    });
}

#[test]
fn a_wrong_password_shows_the_server_message_and_blocks_saving() {
    if !common::greenmail() {
        return;
    }
    let name = format!("{}x", greenmail_user());
    common::create_greenmail_user(&name);
    let d = dir();
    let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    fill_greenmail(&mut h, &name, "wrong");
    h.get_by_label("test connection").click();
    until(&mut h, "test result", |s| !s.add_form().unwrap().test.running() && !s.add_form().unwrap().test.results.is_empty());
    let results = h.state_mut().1.add_form().unwrap().test.results.clone();
    let err = results[0].1.clone().unwrap_err();
    assert!(err.starts_with("authentication failed"), "{err}");
    h.get_by_label_contains("incoming mail (IMAP): authentication failed");
    h.get_by_label_contains("Check the user name and password");
    assert!(!h.state_mut().1.add_form().unwrap().test.all_ok());
    assert_eq!(h.state().1.screen_name(), "add account");
}

#[test]
fn removing_an_account_in_settings_deletes_its_synced_data() {
    if !common::greenmail() {
        return;
    }
    let name = format!("{}r", greenmail_user());
    common::create_greenmail_user(&name);
    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut h = boot(d.path(), kc.clone(), env(FixtureNet::default()));
    setup_greenmail_account(&mut h, &name);
    h.get_by_label("test connection").click();
    until(&mut h, "both steps ok", |s| s.add_form().unwrap().test.all_ok());
    h.get_by_label("save and sync").click();
    h.run();
    until(&mut h, "idle", |s| {
        s.app().is_some_and(|a| a.store.accounts().iter().all(|x| x.sync == ms::SyncState::Idle))
    });
    h.state_mut().1.open_settings();
    h.run();
    h.run();
    h.run();
    h.run();
    h.get_by_label("remove").click();
    h.run();
    h.run();
    h.run();
    h.run();
    h.get_by_label_contains("deletes the mail synced from this account");
    h.get_by_label("remove").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.screen_name(), "add account");
    drop(h);
    let h = boot(d.path(), kc, env(FixtureNet::default()));
    assert_eq!(h.state().1.screen_name(), "add account", "no account left after a restart");
}

#[test]
fn settings_fit_at_phone_width() {
    let d = dir();
    let mut h = harness_sized(
        Shell::new(d.path().to_path_buf(), Rc::new(MemKeychain::default()), env(FixtureNet::default()), Some(KdfParams::fast())),
        egui::vec2(390.0, 844.0),
    );
    h.run();
    h.run();
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.state_mut().1.open_settings();
    h.run();
    h.run();
    h.run();
    assert!(h.state().1.settings_open());
    let close = h.get_by_label("close").rect();
    assert!(close.right() <= 390.0 && close.left() >= 0.0, "close at {close:?}");
    let img = h.render().expect("render");
    img.save(out_dir().join("phone-settings.png")).unwrap();
}

fn out_dir() -> PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn save(h: &mut Harness<'static, State>, name: &str) {
    h.run();
    h.run();
    let img = h.render().expect("render");
    img.save(out_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
fn render_login_screens() {
    let d = dir();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    let mut net = FixtureNet::default();
    net.pages.insert("https://autoconfig.example.test/mail/config-v1.1.xml".into(), AUTOCONFIG.into());
    let mut h = boot(d.path(), kc.clone(), env(net));
    h.get_by_label_contains("Use both").click();
    h.run();
    {
        let f = h.state_mut().1.first_form().unwrap();
        f.password = "correct horse".into();
        f.confirm = "correct horse".into();
    }
    save(&mut h, "first-run");
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.state_mut().1.add_form().unwrap().email = "mara@example.test".into();
    save(&mut h, "add-account");
    h.get_by_label("detect").click();
    until(&mut h, "detected", |s| s.add_form().unwrap().imap_host == "imap.example.test");
    h.state_mut().1.add_form().unwrap().password = "app-password-123".into();
    save(&mut h, "autodetect-result");
    drop(h);

    let mut h = boot(d.path(), Rc::new(DeadKeychain), env(FixtureNet::default()));
    h.state_mut().1.unlock_form().unwrap().error = Some("Wrong password. Try again.".into());
    save(&mut h, "unlock");
    drop(h);

    if !common::greenmail() {
        return;
    }
    let name = format!("{}s", greenmail_user());
    common::create_greenmail_user(&name);
    let d2 = dir();
    let mut h = boot(d2.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    fill_greenmail(&mut h, &name, "wrong");
    h.get_by_label("test connection").click();
    until(&mut h, "test result", |s| !s.add_form().unwrap().test.running() && !s.add_form().unwrap().test.results.is_empty());
    save(&mut h, "test-failing");
    fill_greenmail(&mut h, &name, "secret");
    h.get_by_label("test connection").click();
    until(&mut h, "both steps ok", |s| s.add_form().unwrap().test.all_ok());
    save(&mut h, "test-ok");
    h.get_by_label("save and sync").click();
    h.run();
    until(&mut h, "idle", |s| s.app().is_some_and(|a| a.store.accounts().iter().all(|x| x.sync == ms::SyncState::Idle)));
    h.state_mut().1.open_settings();
    save(&mut h, "settings");
}

#[test]
fn copy_has_no_em_or_en_dashes() {
    for f in ["src/shell.rs", "src/accounts.rs", "src/autodetect.rs", "src/oauth.rs"] {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(f)).unwrap();
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{f}");
    }
}

#[test]
fn starting_the_shell_wipes_leftover_open_copies() {
    let d = dir();
    let open_dir = d.path().join("open");
    let leftover = mailbench::attach::write_private(&open_dir, "old.pdf", b"plain").unwrap();
    assert!(leftover.exists());
    let _shell = Shell::with_open_dir(d.path().to_path_buf(), Rc::new(MemKeychain::default()), env(FixtureNet::default()), Some(KdfParams::fast()), open_dir.clone());
    assert!(!leftover.exists());
    assert!(!open_dir.exists());
}

#[test]
fn a_nostr_account_without_a_relay_list_is_asked_to_publish_one() {
    mailbench::runtime::block_on(async {
        let relay = nostr_relay_builder::MockRelay::run().await.unwrap();
        let url = relay.url().await.to_string();
        let d = dir();
        let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
        h.get_by_label("continue").click();
        h.run();
        h.run();
        {
            let f = h.state_mut().1.add_form().unwrap();
            f.nostr = true;
            f.password = "05".repeat(32);
            f.relays = url;
        }
        h.run();
        h.get_by_label("test connection").click();
        until(&mut h, "prompt", |s| !s.add_form().unwrap().test.running() && s.add_form().unwrap().test.all_ok());
        h.run();
        assert!(h.state_mut().1.add_form().unwrap().test.inbox_missing.load(std::sync::atomic::Ordering::SeqCst));
        h.get_by_label_contains("Publish your DM relay list?");
        assert!(!h.state_mut().1.add_form().unwrap().config().is_ok_and(|_| false));
        h.get_by_label("publish relay list").click();
        h.run();
        let f = h.state_mut().1.add_form().unwrap();
        assert_eq!(f.publish_inbox, Some(true));
        match f.config().unwrap().credential {
            mailbench::accounts::Credential::Nostr { publish_inbox, .. } => assert!(publish_inbox),
            _ => panic!("not a nostr credential"),
        }
    });
}

struct ApproveAll;

impl nostr_connect::prelude::NostrConnectSignerActions for ApproveAll {
    fn approve(&self, _: &nostr_sdk::PublicKey, _: &nostr_connect::prelude::NostrConnectRequest) -> bool {
        true
    }
}

#[test]
fn add_form_pastes_a_bunker_url_connects_and_only_then_allows_saving() {
    use nostr_connect::prelude::*;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let relay = rt.block_on(nostr_relay_builder::MockRelay::run()).unwrap();
    let relay_url = rt.block_on(relay.url()).to_string();
    let user = nostr_sdk::Keys::parse(&"01".repeat(32)).unwrap();
    let signer = NostrConnectRemoteSigner::new(NostrConnectKeys { signer: nostr_sdk::Keys::generate(), user: user.clone() }, [relay_url.as_str()], None, None).unwrap();
    let bunker = signer.bunker_uri().to_string();
    rt.spawn(async move {
        let _ = signer.serve(ApproveAll).await;
    });
    std::thread::sleep(Duration::from_millis(500));

    let d = dir();
    let mut h = boot(d.path(), Rc::new(MemKeychain::default()), env(FixtureNet::default()));
    h.get_by_label("continue").click();
    h.run();
    h.run();
    h.get_by_label_contains("nostr account").click();
    h.run();
    {
        let f = h.state_mut().1.add_form().unwrap();
        f.password = bunker.clone();
        f.relays = relay_url.clone();
    }
    h.run();
    assert!(h.query_by_label_contains("Connected to the bunker").is_none());
    assert!(h.get_by_label("save and sync").accesskit_node().is_disabled());
    h.get_by_label("connect bunker").click();
    let npub = nostr_sdk::ToBech32::to_bech32(&user.public_key()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    while h.query_by_label_contains(&npub).is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "bunker never connected");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(h.get_by_label("save and sync").accesskit_node().is_disabled());
    h.get_by_label("test connection").click();
    until(&mut h, "test done", |s| !s.add_form().unwrap().test.running() && s.add_form().unwrap().test.all_ok());
    h.run();
    h.get_by_label_contains("Publish your DM relay list?");
    assert!(h.get_by_label("save and sync").accesskit_node().is_disabled());
    h.get_by_label("publish relay list").click();
    h.run();
    assert!(!h.get_by_label("save and sync").accesskit_node().is_disabled());
    let cfg = h.state_mut().1.add_form().unwrap().config().unwrap();
    assert!(matches!(cfg.credential, mailbench::accounts::Credential::NostrBunker { publish_inbox: true, .. }));
}
