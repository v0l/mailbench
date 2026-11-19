use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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
    Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
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

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
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
