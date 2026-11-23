use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::MailStore;
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

struct Gated {
    open: Arc<AtomicBool>,
}

fn folder(name: &str, role: Option<FolderRole>) -> Folder {
    Folder { name: name.into(), delimiter: Some("/".into()), role, selectable: true }
}

#[async_trait::async_trait]
impl Backend for Gated {
    fn kind(&self) -> &'static str {
        "gated"
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities { server_folders: true, ..Default::default() })
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        while !self.open.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(vec![
            folder("INBOX", Some(FolderRole::Inbox)),
            folder("Sent", Some(FolderRole::Sent)),
            folder("Archive", Some(FolderRole::Archive)),
            folder("Projects", None),
        ])
    }

    async fn open_folder(&mut self, _folder: &str) -> Result<FolderState> {
        Ok(FolderState { token: SyncToken { uid_validity: 1, uid_next: 1, highest_mod_seq: None }, uids: Vec::new() })
    }
}

type State = (bool, App);

fn out_dir() -> PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("folders_syncing"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(h: &mut Harness<'static, State>) {
    h.run();
    h.run();
}

fn until(h: &mut Harness<'static, State>, what: &str, cond: impl Fn(&Harness<'static, State>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        run(h);
        if cond(h) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn check(size: egui::Vec2, shot: &str) {
    let open = Arc::new(AtomicBool::new(false));
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let setup = AccountSetup {
        id: "a".into(),
        name: "acct".into(),
        address: "a@x".into(),
        sender: String::new(),
        backend: Box::new(Gated { open: open.clone() }),
    };
    let app = App::with_store(Box::new(RealStore::open(store, vec![setup])));
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, app): &mut State| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            app.show(ui);
        },
        (false, app),
    );
    run(&mut h);
    let narrow = size.x < 500.0;
    if narrow {
        h.get_by_label_contains("folders").click();
        run(&mut h);
    }
    for name in ["inbox", "drafts", "sent", "archive", "junk", "trash"] {
        let label = format!("acct {name} waiting");
        let row = h.get_by_label(&label);
        assert!(row.accesskit_node().is_disabled());
        let r = row.rect();
        assert!(r.left() >= 0.0 && r.right() <= size.x, "{name} at {r:?}");
    }
    h.get_by_label("acct syncing folders");
    assert!(h.query_by_label("new folder in acct").is_none());
    h.render().expect("render").save(out_dir().join(format!("{shot}-waiting.png"))).unwrap();

    open.store(true, Ordering::SeqCst);
    until(&mut h, "real folder rows", |h| h.query_by_label("acct inbox").is_some());
    assert!(h.query_by_label("acct syncing folders").is_none());
    assert!(h.query_by_label("acct inbox waiting").is_none());
    h.get_by_label("acct archive");
    let add = h.get_by_label("new folder in acct");
    add.scroll_to_me();
    run(&mut h);
    let add = h.get_by_label("new folder in acct").rect();
    for name in ["acct inbox", "acct sent", "acct archive"] {
        let r = h.get_by_label(name).rect();
        assert!(add.top() >= r.bottom() - 1.0, "new folder {add:?} above {name} {r:?}");
    }
    h.render().expect("render").save(out_dir().join(format!("{shot}-listed.png"))).unwrap();
}

#[test]
fn placeholder_folders_show_until_the_list_exists_at_desktop_width() {
    check(egui::vec2(1280.0, 800.0), "desktop");
}

#[test]
fn placeholder_folders_show_until_the_list_exists_at_phone_width() {
    check(egui::vec2(390.0, 844.0), "phone");
}
