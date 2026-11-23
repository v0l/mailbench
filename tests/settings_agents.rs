use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::accounts::{self, AccountConfig, Credential};
use mailbench::autodetect::{Net, ServerSettings};
use mailbench::backend::*;
use mailbench::dns::Srv;
use mailbench::mcp::{prefs, runner, token};
use mailbench::settings_page::Section;
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

fn account(id: &str, name: &str) -> AccountConfig {
    AccountConfig {
        id: id.into(),
        name: name.into(),
        display_name: String::new(),
        signature: String::new(),
        address: format!("{name}@example.test"),
        username: name.into(),
        imap: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        smtp: ServerSettings::new("127.0.0.1", 1, Security::Plain),
        credential: Credential::Password("x".into()),
        backfill: Default::default(),
        notify: Default::default(),
        sync: Default::default(),
    }
}

struct Rig {
    dir: tempfile::TempDir,
    kc: Rc<dyn Keychain>,
    h: Harness<'static, State>,
}

fn boot(size: egui::Vec2) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let kc: Rc<dyn Keychain> = Rc::new(MemKeychain::default());
    {
        let store = Store::create(&dir.path().join("mail.db"), Some(&*kc), None).unwrap();
        accounts::save(&store, &account("a-work", "work")).unwrap();
        accounts::save(&store, &account("a-home", "home")).unwrap();
    }
    let env = Env { net: Arc::new(NoNet), opener: Arc::new(|_| {}) };
    let shell = Shell::new(dir.path().to_path_buf(), kc.clone(), env, Some(KdfParams::fast()));
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(1.0).build_ui_state(
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
    h.run_steps(3);
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
        self.h.run_steps(6);
    }

    fn agents(&mut self) {
        self.h.state_mut().1.settings_form().unwrap().page.open_section(Section::Agents);
        self.settle();
    }

    fn click(&mut self, label: &str) {
        self.h.get_by_label(label).hover();
        self.settle();
        self.h.get_by_label(label).click();
        self.settle();
    }
}

fn out_dir() -> std::path::PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("agents-shots"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn walk(size: egui::Vec2, prefix: &str) {
    let mut rig = boot(size);
    rig.agents();
    rig.h.get_by_label("run the agent server inside the app");
    assert_eq!(prefs::load(&rig.store()), prefs::AgentPrefs::default());
    rig.h.render().expect("render").save(out_dir().join(format!("{prefix}agents-off.png"))).unwrap();

    rig.click("agents may use work");
    rig.click("let agents send mail without asking from work");
    let saved = prefs::load(&rig.store());
    assert!(saved.allows("a-work") && !saved.allows("a-home"));
    assert!(saved.may_send("a-work", false));

    rig.click("create token");
    let made = token::existing(&rig.store()).unwrap();
    assert_eq!(made.len(), 64);
    assert_eq!(rig.h.state_mut().1.settings_form().unwrap().agents.token.as_deref(), Some(made.as_str()));
    rig.h.get_by_label("copy token");
    rig.h.render().expect("render").save(out_dir().join(format!("{prefix}agents-on.png"))).unwrap();
    rig.click("rotate token");
    assert_ne!(token::existing(&rig.store()).unwrap(), made);
    assert_ne!(rig.h.state_mut().1.settings_form().unwrap().agents.token.as_deref(), Some(made.as_str()));
}

#[test]
fn the_agents_page_edits_settings_and_shows_the_token_once_at_desktop_size() {
    walk(egui::vec2(1280.0, 800.0), "");
}

#[test]
fn the_agents_page_fits_a_phone() {
    let mut rig = boot(egui::vec2(392.0, 844.0));
    rig.agents();
    let width = 392.0;
    for label in ["agents may use work", "agents may use home"] {
        let r = rig.h.get_by_label(label).rect();
        assert!(r.left() >= 0.0 && r.right() <= width, "{label} {r:?}");
    }
    rig.click("agents may use work");
    let r = rig.h.get_by_label("let agents send mail without asking from work").rect();
    assert!(r.right() <= width + 1.0, "{r:?}");
    rig.click("create token");
    let r = rig.h.get_by_label("copy token").rect();
    assert!(r.left() >= 0.0 && r.right() <= width, "{r:?}");
    rig.h.render().expect("render").save(out_dir().join("phone-agents-on.png")).unwrap();
    walk(egui::vec2(392.0, 844.0), "phone-");
}

#[test]
fn the_switch_starts_the_server_inside_the_app_and_a_second_server_is_refused() {
    let mut rig = boot(egui::vec2(1280.0, 800.0));
    rig.agents();
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    {
        let store = rig.store();
        let mut p = prefs::load(&store);
        p.port = port;
        prefs::save(&store, &p).unwrap();
    }
    rig.h.state_mut().1.settings_form().unwrap().agents.prefs.port = port;
    rig.settle();
    rig.click("run the agent server inside the app");
    assert!(prefs::load(&rig.store()).enabled);
    assert!(runner::lock_held(rig.dir.path()), "the app holds mcp.lock");
    let refused = runner::take_lock(rig.dir.path()).unwrap_err();
    assert!(refused.contains("another mailbench agent server"), "{refused}");
    let again = runner::start(&rig.store(), rig.dir.path(), Some(0), false).err().unwrap();
    assert!(again.contains("already uses this mail store"), "{again}");
    let tok = token::load_or_create(&rig.store()).unwrap().0;
    let reply = ureq::post(&format!("http://127.0.0.1:{port}/mcp"))
        .set("Authorization", &format!("Bearer {tok}"))
        .send_string(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#)
        .unwrap();
    assert_eq!(reply.status(), 200);
    rig.click("run the agent server inside the app");
    assert!(!prefs::load(&rig.store()).enabled);
    assert!(!runner::lock_held(rig.dir.path()), "switching off releases the lock");
}

#[test]
fn a_headless_server_holding_the_lock_makes_the_app_switch_say_so() {
    let mut rig = boot(egui::vec2(1280.0, 800.0));
    let _held = runner::take_lock(rig.dir.path()).unwrap();
    rig.agents();
    rig.click("run the agent server inside the app");
    rig.h.get_by_label_contains("another mailbench agent server");
    assert!(prefs::load(&rig.store()).enabled);
}
