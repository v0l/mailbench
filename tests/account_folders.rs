use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store, Unlock};
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

fn message(uid: u32, subject: &str, from: &str, seen: bool, date: i64) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(subject.into()),
        from: vec![Address { name: None, address: from.into() }],
        date: Some(date),
        message_id: Some(format!("<{subject}-{uid}@x>")),
        flags: Flags { seen, ..Default::default() },
        ..Default::default()
    }
}

fn folder(name: &str, role: FolderRole) -> Folder {
    Folder { name: name.into(), delimiter: None, role: Some(role), selectable: true }
}

fn seed(sink: &mut StoreSink, folders: &[(&str, FolderRole)], mail: &[(&str, Vec<MessageSummary>)]) {
    let all: Vec<Folder> = folders.iter().map(|(n, r)| folder(n, *r)).collect();
    sink.put_folders(&all).unwrap();
    for (name, items) in mail {
        sink.put_summaries(name, items).unwrap();
        let next = items.iter().map(|m| m.uid).max().unwrap_or(0) + 1;
        sink.put_token(name, &SyncToken { uid_validity: 1, uid_next: next, highest_mod_seq: None }).unwrap();
    }
}

fn setup(id: &str, name: &str) -> AccountSetup {
    AccountSetup { id: id.into(), name: name.into(), address: format!("{id}@x"), sender: String::new(), backend: Box::new(Offline) }
}

type State = (bool, App);

fn open() -> (Harness<'static, State>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        let mut imap = StoreSink::new(&store, "imap", true);
        seed(
            &mut imap,
            &[("INBOX", FolderRole::Inbox), ("Sent", FolderRole::Sent)],
            &[
                ("INBOX", vec![message(1, "imap one", "ann@x", false, 1000), message(2, "imap two", "bob@x", true, 2000)]),
                ("Sent", vec![message(1, "imap sent", "me@x", true, 3000)]),
            ],
        );
        let mut nostr = StoreSink::new(&store, "nostr", true);
        seed(
            &mut nostr,
            &[("Inbox", FolderRole::Inbox), ("Sent", FolderRole::Sent), ("Archive", FolderRole::Archive)],
            &[
                ("Inbox", vec![message(1, "dm one", "npub1a", false, 4000), message(2, "dm two", "npub1b", false, 5000), message(3, "dm three", "npub1c", true, 6000)]),
                ("Archive", vec![message(4, "dm old", "npub1d", true, 100)]),
            ],
        );
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let real = RealStore::open(store, vec![setup("imap", "mail"), setup("nostr", "dms")]);
    let app = App::with_store(Box::new(real));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run();
        if h.state().1.store.counts(&Query::new(ms::Folder::Inbox)).all == 5 {
            break;
        }
        assert!(Instant::now() < deadline, "never loaded");
        std::thread::sleep(Duration::from_millis(50));
    }
    (h, dir)
}

fn rows(app: &App, q: &Query) -> usize {
    app.store.list(q, 0, 1000).items.len()
}

#[test]
fn each_account_is_a_group_with_its_own_folders() {
    let (h, _d) = open();
    let s = &h.state().1.store;
    let imap: Vec<_> = s.account_folders(0).into_iter().map(|f| f.folder).collect();
    let nostr: Vec<_> = s.account_folders(1).into_iter().map(|f| f.folder).collect();
    assert_eq!(imap, vec![ms::Folder::Inbox, ms::Folder::Sent]);
    assert_eq!(nostr, vec![ms::Folder::Inbox, ms::Folder::Sent, ms::Folder::Archive]);
    h.get_by_label_contains("dms inbox");
    h.get_by_label_contains("dms sent");
    h.get_by_label_contains("dms archive");
    h.get_by_label_contains("mail inbox");
    h.get_by_label_contains("mail sent");
}

#[test]
fn the_nostr_inbox_lists_only_nostr_rows_and_its_count_matches() {
    let (mut h, _d) = open();
    h.get_by_label_contains("dms inbox").click();
    h.run();
    h.run();
    let app = &h.state().1;
    assert_eq!(app.account, Some(1));
    let q = app.query();
    let counts = app.store.counts(&q);
    let page = app.store.list(&q, 0, 100);
    assert_eq!(counts.all, 3);
    assert_eq!(page.items.len(), counts.all);
    assert!(page.items.iter().all(|s| s.account == 1));
    assert_eq!(counts.unread, 2);
    let folder = app.store.account_folders(1).into_iter().find(|f| f.folder == ms::Folder::Inbox).unwrap();
    assert_eq!((folder.total, folder.unread), (3, 2));
}

#[test]
fn the_unified_inbox_still_lists_both_accounts() {
    let (mut h, _d) = open();
    h.get_by_label_contains("dms inbox").click();
    h.run();
    h.run();
    h.get_by_label("folder inbox").click();
    h.run();
    h.run();
    let app = &h.state().1;
    assert_eq!(app.account, None);
    let page = app.store.list(&app.query(), 0, 100);
    assert_eq!(page.items.len(), 5);
    assert!(page.items.iter().any(|s| s.account == 0) && page.items.iter().any(|s| s.account == 1));
}

#[test]
fn the_account_filter_isolates_and_clears_from_any_view() {
    let (mut h, _d) = open();
    let all = Query::new(ms::Folder::Inbox);
    assert_eq!(rows(&h.state().1, &all), 5);
    h.state_mut().1.account = None;
    h.key_press(egui::Key::V);
    h.run();
    h.run();
    let app = &h.state().1;
    assert!(app.account.is_some());
    let shown = app.account.unwrap();
    let page = app.store.list(&app.query(), 0, 100);
    assert!(!page.items.is_empty() && page.items.iter().all(|s| s.account == shown));
    h.get_by_label_contains("showing");
    h.key_press(egui::Key::V);
    h.run();
    h.run();
    assert_eq!(h.state().1.account, None);
    assert_eq!(rows(&h.state().1, &h.state().1.query()), 5);
}

#[test]
fn the_sidebar_action_isolates_one_account_and_the_chip_clears_it() {
    let (mut h, _d) = open();
    h.get_by_label("only dms").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.account, Some(1));
    assert_eq!(rows(&h.state().1, &h.state().1.query()), 3);
    h.get_by_label_contains("showing dms only").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.account, None);
}

#[test]
fn clicking_an_account_filters_to_it_and_all_accounts_clears_it() {
    let (mut h, _d) = open();
    h.get_by_label("account dms").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.account, Some(1));
    assert_eq!(rows(&h.state().1, &h.state().1.query()), 3);
    h.get_by_label("all accounts").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.account, None);
    assert_eq!(rows(&h.state().1, &h.state().1.query()), 5);
}

#[test]
fn the_phone_layout_reaches_account_folders_with_44px_targets() {
    let (mut h, _d) = open();
    h.set_size(egui::vec2(390.0, 844.0));
    h.run();
    h.run();
    assert!(h.state().1.narrow);
    h.get_by_label_contains("folders").click();
    h.run();
    h.run();
    let target = h.get_by_label_contains("dms archive").rect();
    h.get_by_label_contains("dms archive").scroll_to_me();
    h.run();
    h.run();
    let target = h.get_by_label_contains("dms archive").rect();
    assert!(target.height() >= 44.0, "tap target {target:?}");
    assert!(target.right() <= 390.0);
    h.get_by_label_contains("dms archive").click();
    h.run();
    h.run();
    assert!(!h.state().1.folders_open);
    assert_eq!(h.state().1.account, Some(1));
    assert_eq!(h.state().1.folder, ms::Folder::Archive);
    assert_eq!(rows(&h.state().1, &h.state().1.query()), 1);
    h.get_by_label_contains("showing dms only");
}

#[test]
fn shots() {
    let Ok(dir) = std::env::var("SHOT_DIR") else { return };
    let (mut h, _d) = open();
    h.get_by_label_contains("dms inbox").click();
    h.run();
    h.run();
    h.render().expect("render").save(format!("{dir}/wide.png")).unwrap();
    h.set_size(egui::vec2(390.0, 844.0));
    h.run();
    h.run();
    h.get_by_label_contains("folders").click();
    h.run();
    h.run();
    h.get_by_label_contains("dms archive").scroll_to_me();
    h.run();
    h.run();
    h.render().expect("render").save(format!("{dir}/phone-drawer.png")).unwrap();
    h.get_by_label_contains("dms archive").click();
    h.run();
    h.run();
    h.render().expect("render").save(format!("{dir}/phone-list.png")).unwrap();
}
