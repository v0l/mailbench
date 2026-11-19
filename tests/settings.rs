use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::accounts::{self, AccountConfig, Credential};
use mailbench::autodetect::{Net, ServerSettings};
use mailbench::backend::*;
use mailbench::dns::Srv;
use mailbench::prefs::{self, BodyPolicy, RemoteImages};
use mailbench::shell::{Env, Shell};
use mailbench::store::{KdfParams, Keychain, Store, StoreError, Unlock};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::result::Result;
use std::sync::Arc;

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

type State = (bool, Shell);

const ACCOUNT: &str = "acct1";
const OLD: i64 = 1_000_000;

fn account() -> AccountConfig {
    AccountConfig {
        id: ACCOUNT.into(),
        name: "work".into(),
        display_name: String::new(),
        signature: String::new(),
        address: "me@localhost".into(),
        username: "me".into(),
        imap: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        smtp: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        credential: Credential::Password("x".into()),
        backfill: Default::default(),
        notify: Default::default(),
        sync: Default::default(),
    }
}

fn seed(store: &Store) {
    accounts::save(store, &account()).unwrap();
    let mut sink = StoreSink::new(store, ACCOUNT, true);
    let folder = |name: &str| Folder { name: name.into(), delimiter: Some("/".into()), role: None, selectable: true };
    sink.put_folders(&[folder("INBOX"), folder("Big archive")]).unwrap();
    let summary = MessageSummary {
        uid: 1,
        subject: Some("Ancient zebracorn budget".into()),
        from: vec![Address { name: None, address: "old@example.test".into() }],
        date: Some(OLD),
        ..Default::default()
    };
    sink.put_summaries("INBOX", &[summary]).unwrap();
    sink.put_body("INBOX", 1, b"Subject: Ancient zebracorn budget\r\n\r\nmarmalade payload\r\n").unwrap();
}

struct Rig {
    dir: tempfile::TempDir,
    kc: Rc<dyn Keychain>,
    h: Harness<'static, State>,
}

fn boot() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    {
        let store = Store::create(&dir.path().join("mail.db"), Some(&*kc), None).unwrap();
        seed(&store);
    }
    let env = Env { net: Arc::new(NoNet), opener: Arc::new(|_| {}) };
    let shell = Shell::new(dir.path().to_path_buf(), kc.clone(), env, Some(KdfParams::fast()));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 6000.0)).with_pixels_per_point(1.0).build_ui_state(
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
    );
    h.run();
    h.run();
    assert_eq!(h.state().1.screen_name(), "mail");
    h.state_mut().1.open_settings();
    let mut rig = Rig { dir, kc, h };
    rig.settle();
    rig
}

impl Rig {
    fn store(&self) -> Store {
        Store::open(&self.dir.path().join("mail.db"), &[Unlock::Keychain(&*self.kc)]).unwrap()
    }

    fn settle(&mut self) {
        let mut last = None;
        for _ in 0..200 {
            self.h.run();
            let at = self.h.query_by_label("apply").map(|n| n.rect());
            if at.is_some() && at == last {
                return;
            }
            last = at;
        }
    }

    fn click(&mut self, label: &str) {
        self.h.get_by_label(label).hover();
        self.settle();
        self.h.get_by_label(label).click();
        self.settle();
    }
}

#[test]
fn account_sync_options_are_saved_and_the_folder_list_is_offered() {
    let mut rig = boot();
    rig.h.get_by_label_contains("folders to sync");
    {
        let f = rig.h.state_mut().1.settings_form().unwrap();
        assert_eq!(f.panels.folders_of(ACCOUNT), ["INBOX", "Big archive"]);
        let d = &mut f.drafts[0];
        d.sync.set_syncs("Big archive", false);
        d.sync.window_months = 6;
        d.sync.poll_seconds = 120;
        d.notify.enabled = false;
    }
    rig.settle();
    rig.click("apply");
    let saved = accounts::load_all(&rig.store()).remove(0);
    assert_eq!(saved.sync.window_months, 6);
    assert_eq!(saved.sync.excluded_folders, ["Big archive"]);
    assert_eq!((saved.sync.window_months, saved.sync.poll_seconds, saved.notify.enabled), (6, 120, false));
    let filter = saved.sync.filter(2_000_000_000);
    assert_eq!(filter.since, Some(2_000_000_000 - 6 * 30 * 86_400));
    assert_eq!(saved.sync.poll().as_secs(), 120);
}

#[test]
fn bodies_policy_persists_and_free_space_drops_old_bodies() {
    let mut rig = boot();
    rig.h.get_by_label_contains("OLDER THAN");
    {
        let sink_store = rig.store();
        assert_eq!(StoreSink::new(&sink_store, ACCOUNT, true).body_uids("INBOX").unwrap(), [1]);
    }
    rig.h.state_mut().1.settings_form().unwrap().panels.prefs.bodies = BodyPolicy::KeepDays(30);
    rig.settle();
    assert_eq!(prefs::load(&rig.store()).bodies, BodyPolicy::KeepDays(30));
    rig.click("free space");
    let store = rig.store();
    let sink = StoreSink::new(&store, ACCOUNT, true);
    assert!(sink.body_uids("INBOX").unwrap().is_empty());
    assert_eq!(sink.summary("INBOX", 1).unwrap().unwrap().subject.as_deref(), Some("Ancient zebracorn budget"));
    assert_eq!(sink.search("zebracorn").unwrap().len(), 1);
    assert!(sink.search("marmalade").unwrap().is_empty());
    rig.h.get_by_label_contains("Dropped 1 bodies");
}

#[test]
fn store_section_shows_counts_reindexes_and_writes_an_encrypted_backup() {
    let mut rig = boot();
    let stats = rig.h.state_mut().1.settings_form().unwrap().panels.stats;
    assert_eq!((stats.messages, stats.bodies), (1, 1));
    assert!(stats.bytes > 0);
    rig.click("re-index");
    rig.h.get_by_label_contains("Search index rebuilt for 1 messages");
    assert_eq!(StoreSink::new(&rig.store(), ACCOUNT, true).search("marmalade").unwrap().len(), 1);

    let backup = rig.dir.path().join("copy.db");
    rig.h.state_mut().1.settings_form().unwrap().panels.backup_path = backup.display().to_string();
    rig.click("export backup");
    rig.h.get_by_label_contains("Encrypted backup written");
    let raw = std::fs::read(&backup).unwrap();
    assert!(!raw.windows(9).any(|w| w == b"zebracorn" || w == b"marmalade"));
    let copy = Store::open(&backup, &[Unlock::Keychain(&*rig.kc)]).unwrap();
    assert_eq!(StoreSink::new(&copy, ACCOUNT, true).search("zebracorn").unwrap().len(), 1);
    rig.click("export backup");
    rig.h.get_by_label_contains("already exists");
}

#[test]
fn reading_preferences_persist() {
    let mut rig = boot();
    rig.h.get_by_label_contains("dark reading for HTML");
    {
        let f = rig.h.state_mut().1.settings_form().unwrap();
        f.panels.prefs.reading.zoom_percent = 130;
        f.panels.prefs.reading.dark_html = true;
        f.panels.prefs.reading.remote_images = RemoteImages::Trusted;
    }
    rig.settle();
    let saved = prefs::load(&rig.store()).reading;
    assert_eq!(saved.zoom_percent, 130);
    assert!(saved.dark_html);
    assert_eq!(saved.remote_images, RemoteImages::Trusted);
    assert_eq!(saved.zoom_milli(), 1300);
    let app = rig.h.state().1.app().unwrap();
    assert_eq!(app.html_state.default_zoom, 1300);
    assert!(app.html_state.dark);
    let live = rig.h.state().1.app().unwrap().prefs.reading.clone();
    assert_eq!(live, saved);
}

#[test]
fn theme_and_density_persist_and_apply() {
    let mut rig = boot();
    rig.h.get_by_label_contains("density");
    {
        let f = rig.h.state_mut().1.settings_form().unwrap();
        f.panels.prefs.theme = prefs::Theme::Light;
        f.panels.prefs.density = prefs::Density::Compact;
    }
    rig.settle();
    let saved = prefs::load(&rig.store());
    assert_eq!((saved.theme, saved.density), (prefs::Theme::Light, prefs::Density::Compact));
    assert_eq!(rig.h.state().1.app().unwrap().prefs.density.row_height(), 21.0);
    assert_eq!(rig.h.ctx.options(|o| o.theme_preference), egui::ThemePreference::Light);
}

#[test]
fn nostr_accounts_are_recognised_so_server_sync_options_are_hidden() {
    let mut nostr = account();
    nostr.credential = Credential::Nostr { secret: "x".into(), relays: vec![], publish_inbox: false };
    assert!(nostr.is_nostr());
    assert!(!account().is_nostr());
}

#[test]
fn keyboard_list_rebinds_and_persists() {
    let mut rig = boot();
    rig.h.get_by_label_contains("next message");
    rig.click("rebind archive");
    rig.h.get_by_label_contains("Press the new key");
    rig.h.key_press(egui::Key::B);
    rig.settle();
    assert_eq!(rig.store_keys().key_for("archive"), egui::Key::B);
    assert_eq!(rig.h.state().1.app().unwrap().prefs.keys.key_for("archive"), egui::Key::B);
    rig.click("rebind reply");
    rig.h.key_press(egui::Key::B);
    rig.settle();
    rig.h.get_by_label_contains("already used for archive");
    assert_eq!(rig.store_keys().key_for("reply"), egui::Key::R);
    rig.click("reset all keys");
    assert_eq!(rig.store_keys().key_for("archive"), egui::Key::E);
}

impl Rig {
    fn store_keys(&self) -> prefs::Bindings {
        prefs::load(&self.store()).keys
    }
}

#[allow(dead_code)]
fn unused(_: &Path) {}

mod common;

fn greenmail_user() -> String {
    format!("sf{}", std::process::id())
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_filter_stops_an_excluded_folder_from_syncing_and_the_window_limits_mail() {
    if !common::greenmail() {
        return;
    }
    let name = greenmail_user();
    common::create_greenmail_user(&name);
    let auth = Auth::Password { user: name.clone(), password: "secret".into() };
    let mut imap = ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth));
    imap.create_folder("Big archive").await.unwrap();
    let raw = |subject: &str, date: &str| {
        format!("From: a@localhost\r\nTo: {name}@localhost\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{subject}@x>\r\n\r\nbody\r\n").into_bytes()
    };
    imap.append("INBOX", &raw("recent", &chrono::Utc::now().to_rfc2822())).await.unwrap();
    imap.append("INBOX", &raw("ancient", "Mon, 05 Jan 2015 10:00:00 +0000")).await.unwrap();
    imap.append("Big archive", &raw("archived", &chrono::Utc::now().to_rfc2822())).await.unwrap();

    let mut options = mailbench::accounts::SyncOptions::default();
    options.set_syncs("Big archive", false);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut sink = StoreSink::new(&store, "acct", true);
    let report = sync_account_with(&mut imap, &mut sink, &options.filter(0)).await.unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(sink.messages("INBOX").unwrap().len(), 2);
    assert!(sink.messages("Big archive").unwrap().is_empty());
    assert!(sink.sync_token_for("Big archive").is_none());

    options.set_syncs("Big archive", true);
    let report = sync_account_with(&mut imap, &mut sink, &options.filter(0)).await.unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(sink.messages("Big archive").unwrap().len(), 1);

    let windowed = Store::create(&dir.path().join("w.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut sink = StoreSink::new(&windowed, "acct", true);
    let mut options = mailbench::accounts::SyncOptions { window_months: 12, ..Default::default() };
    options.set_syncs("Big archive", false);
    let filter = options.filter(chrono::Utc::now().timestamp());
    let report = sync_account_with(&mut imap, &mut sink, &filter).await.unwrap();
    assert_eq!(report.added, 1);
    let kept = sink.messages("INBOX").unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].subject.as_deref(), Some("recent"));
}
