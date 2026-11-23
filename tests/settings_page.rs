use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::accounts::{self, AccountConfig, Credential};
use mailbench::autodetect::{Net, ServerSettings};
use mailbench::backend::*;
use mailbench::dns::Srv;
use mailbench::settings_page::{Level, Section, Tab};
use mailbench::shell::{Env, Shell};
use mailbench::store::{KdfParams, Keychain, Store, StoreError, Unlock};
use std::cell::RefCell;
use std::collections::HashMap;
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

fn imap() -> AccountConfig {
    AccountConfig {
        id: "a-imap".into(),
        name: "work".into(),
        display_name: String::new(),
        signature: String::new(),
        address: "me@work.test".into(),
        username: "me".into(),
        imap: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        smtp: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        credential: Credential::Password("x".into()),
        backfill: Default::default(),
        notify: Default::default(),
        sync: Default::default(),
    }
}

fn jmap() -> AccountConfig {
    let mut c = accounts::jmap_config("http://127.0.0.1:1/jmap", "ann@fast.test", "ann", "pw");
    c.id = "b-jmap".into();
    c.name = "fast".into();
    c
}

fn nostr() -> AccountConfig {
    let mut c = accounts::nostr_config(&"0a".repeat(32), vec!["ws://127.0.0.1:1".into()]).unwrap();
    c.id = "c-nostr".into();
    c.name = "carol".into();
    c
}

struct Rig {
    dir: tempfile::TempDir,
    kc: Rc<dyn Keychain>,
    h: Harness<'static, State>,
}

fn harness(shell: Shell, size: egui::Vec2) -> Harness<'static, State> {
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

fn boot_at(size: egui::Vec2) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    {
        let store = Store::create(&dir.path().join("mail.db"), Some(&*kc), None).unwrap();
        for c in [imap(), jmap(), nostr()] {
            accounts::save(&store, &c).unwrap();
        }
    }
    let env = Env { net: Arc::new(NoNet), opener: Arc::new(|_| {}) };
    let shell = Shell::new(dir.path().to_path_buf(), kc.clone(), env, Some(KdfParams::fast()));
    let mut h = harness(shell, size);
    h.run();
    h.run();
    assert_eq!(h.state().1.screen_name(), "mail");
    h.state_mut().1.open_settings();
    let mut rig = Rig { dir, kc, h };
    rig.settle();
    rig
}

fn boot() -> Rig {
    boot_at(egui::vec2(1280.0, 800.0))
}

impl Rig {
    fn settle(&mut self) {
        for _ in 0..6 {
            self.h.run();
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.dir.path().join("mail.db"), &[Unlock::Keychain(&*self.kc)]).unwrap()
    }

    fn form(&mut self) -> &mut mailbench::shell::SettingsForm {
        self.h.state_mut().1.settings_form().unwrap()
    }

    fn accounts_page(&mut self) {
        self.form().page.open_section(Section::Accounts);
        self.settle();
    }

    fn click(&mut self, label: &str) {
        self.h.get_by_label(label).hover();
        self.settle();
        self.h.get_by_label(label).click();
        self.settle();
    }

    fn open_account(&mut self, name: &str) {
        self.h.get_by_label_contains(&format!("account {name},")).click();
        self.settle();
    }

    fn names(&self) -> Vec<String> {
        accounts::load_all(&self.store()).into_iter().map(|c| c.name).collect()
    }
}

#[test]
fn the_accounts_page_lists_each_account_with_name_address_and_kind() {
    let mut rig = boot();
    rig.accounts_page();
    rig.h.get_by_label("account work, me@work.test, IMAP");
    rig.h.get_by_label("account fast, ann@fast.test, JMAP");
    rig.h.get_by_label_contains("account carol,");
    rig.h.get_by_label_contains("NOSTR");
    rig.h.get_by_label("add account");
}

#[test]
fn selecting_an_account_shows_its_own_page() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("fast");
    assert_eq!(rig.form().page.account, 1);
    rig.h.get_by_label("added to new messages");
    rig.form().page.tab = Tab::Servers;
    rig.settle();
    rig.h.get_by_label("https://mail.example.com/jmap/session");
    rig.open_account("work");
    rig.form().page.tab = Tab::Servers;
    rig.settle();
    rig.h.get_by_label("imap.example.com");
    rig.h.get_by_label("re-test");
}

#[test]
fn editing_the_signature_and_pressing_apply_saves_and_rebuilds() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("work");
    assert!(rig.h.query_by_label("apply").is_none());
    rig.form().drafts[0].signature = "Best, me".into();
    rig.settle();
    rig.click("apply");
    let saved = accounts::load_all(&rig.store());
    assert_eq!(saved[0].signature, "Best, me");
    assert!(rig.h.query_by_label("apply").is_none());
    let app = rig.h.state().1.app().unwrap();
    assert_eq!(app.store.accounts()[0].signature, "Best, me");
}

#[test]
fn the_unsaved_bar_appears_on_edit_and_discard_restores() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("work");
    rig.h.get_by_label("added to new messages");
    assert!(rig.h.query_by_label("discard").is_none());
    rig.form().drafts[0].display_name = "Work Me".into();
    rig.settle();
    rig.h.get_by_label("apply");
    rig.click("discard");
    assert_eq!(rig.form().drafts[0].display_name, "");
    assert!(rig.h.query_by_label("discard").is_none());
    assert_eq!(accounts::load_all(&rig.store())[0].display_name, "");
}

#[test]
fn a_blank_name_is_flagged_next_to_the_field_and_blocks_apply() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("work");
    rig.form().drafts[0].name = String::new();
    rig.settle();
    rig.h.get_by_label_contains("Enter a name");
    rig.click("apply");
    assert!(rig.names().contains(&"work".to_string()));
}

#[test]
fn removing_an_account_asks_first_and_removes_only_that_one() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("fast");
    rig.form().page.tab = Tab::Danger;
    rig.settle();
    rig.click("remove");
    rig.h.get_by_label_contains("deletes the mail synced from this account");
    assert_eq!(rig.names().len(), 3);
    rig.click("keep it");
    assert_eq!(rig.names().len(), 3);
    rig.click("remove");
    rig.h.get_all_by_label("remove").last().unwrap().click();
    rig.settle();
    assert_eq!(rig.names(), ["work", "carol"]);
}

#[test]
fn reordering_accounts_persists_across_a_reopen() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("carol");
    rig.click("move up");
    assert_eq!(rig.names(), ["work", "carol", "fast"]);
    rig.form().page.level = Level::Sections;
    drop(rig.h);
    let env = Env { net: Arc::new(NoNet), opener: Arc::new(|_| {}) };
    let shell = Shell::new(rig.dir.path().to_path_buf(), rig.kc.clone(), env, Some(KdfParams::fast()));
    let mut h = harness(shell, egui::vec2(1280.0, 800.0));
    h.run();
    h.run();
    let names: Vec<String> = h.state().1.app().unwrap().store.accounts().into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["work", "carol", "fast"]);
}

#[test]
fn escape_steps_back_one_level_then_leaves_the_page() {
    let mut rig = boot();
    rig.accounts_page();
    rig.open_account("work");
    assert_eq!(rig.form().page.level, Level::Account);
    rig.h.key_press(egui::Key::Escape);
    rig.settle();
    assert_eq!(rig.form().page.level, Level::Page);
    rig.h.key_press(egui::Key::Escape);
    rig.settle();
    assert_eq!(rig.form().page.level, Level::Sections);
    rig.h.key_press(egui::Key::Escape);
    rig.settle();
    assert!(!rig.h.state().1.settings_open());
}

#[test]
fn arrow_keys_and_enter_walk_the_section_list() {
    let mut rig = boot();
    assert_eq!(rig.form().page.level, Level::Sections);
    rig.h.key_press(egui::Key::ArrowDown);
    rig.settle();
    assert_eq!(rig.form().page.section, Section::Accounts);
    rig.h.key_press(egui::Key::Enter);
    rig.settle();
    assert_eq!(rig.form().page.level, Level::Page);
}

#[test]
fn comma_opens_settings_from_the_mail_view() {
    let mut rig = boot();
    rig.form().page.level = Level::Sections;
    rig.h.key_press(egui::Key::Escape);
    rig.settle();
    assert!(!rig.h.state().1.settings_open());
    rig.h.key_press(egui::Key::Comma);
    rig.settle();
    assert!(rig.h.state().1.settings_open());
}

#[test]
fn settings_is_a_page_not_a_modal() {
    let mut rig = boot();
    rig.settle();
    assert!(rig.h.ctx.memory(|m| m.area_rect(egui::Id::new("settings"))).is_none());
    assert!(rig.h.query_by_label("close").is_some());
}

#[test]
fn leaving_with_unsaved_edits_warns_first() {
    let mut rig = boot();
    rig.accounts_page();
    rig.form().drafts[0].display_name = "Work Me".into();
    rig.settle();
    rig.click("close");
    rig.h.get_by_label_contains("Your edits to the accounts");
    assert!(rig.h.state().1.settings_open());
    rig.click("keep editing");
    assert!(rig.h.state().1.settings_open());
    rig.click("close");
    rig.click("discard and leave");
    assert!(!rig.h.state().1.settings_open());
}

#[test]
fn on_a_phone_the_list_and_the_detail_are_separate_screens_with_a_back() {
    let mut rig = boot_at(egui::vec2(390.0, 844.0));
    let img = rig.h.render().expect("render");
    let out = std::env::var_os("SCREENS_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &out {
        img.save(dir.join("settings-phone-sections.png")).unwrap();
    }
    assert!(rig.h.query_by_label("add account").is_none());
    let row = rig.h.get_by_label("accounts").rect();
    assert!(row.height() >= 44.0, "{row:?}");
    rig.h.get_by_label("accounts").click();
    rig.settle();
    assert_eq!(rig.form().page.level, Level::Page);
    let rows: Vec<_> = rig.h.get_all_by_label_contains("@").map(|n| n.rect()).collect();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.height() >= 44.0 && r.left() >= 0.0 && r.right() <= 390.0), "{rows:?}");
    assert!(rig.h.query_by_label("added to new messages").is_none());
    rig.open_account("work");
    assert_eq!(rig.form().page.level, Level::Account);
    rig.h.get_by_label("added to new messages");
    assert!(rig.h.query_by_label("add account").is_none());
    let name = rig.h.get_by_label("shown in the account list").rect();
    assert!(name.left() >= 0.0 && name.right() <= 390.0, "{name:?}");
    rig.click("back");
    assert_eq!(rig.form().page.level, Level::Page);
    rig.h.get_by_label("add account");
    rig.click("back");
    assert_eq!(rig.form().page.level, Level::Sections);
}
