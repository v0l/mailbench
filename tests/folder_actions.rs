use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Outgoing, Query};
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

    async fn set_flags(&mut self, _folder: &str, _uids: &[u32], _update: &FlagUpdate) -> Result<()> {
        Err(BackendError::Connection("offline".into()))
    }

    async fn move_messages(&mut self, _folder: &str, _uids: &[u32], _destination: &str) -> Result<()> {
        Err(BackendError::Connection("offline".into()))
    }

    async fn delete_messages(&mut self, _folder: &str, _uids: &[u32]) -> Result<()> {
        Err(BackendError::Connection("offline".into()))
    }

    async fn search(&mut self, _folder: &str, _query: &str) -> Result<Vec<u32>> {
        Err(BackendError::Connection("offline".into()))
    }

    async fn append(&mut self, _folder: &str, _raw: &[u8]) -> Result<()> {
        Err(BackendError::Connection("offline".into()))
    }
}

fn message(uid: u32, subject: &str, seen: bool, date: i64) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(subject.into()),
        from: vec![Address { name: None, address: "ann@x".into() }],
        date: Some(date),
        message_id: Some(format!("<{subject}-{uid}@x>")),
        flags: Flags { seen, ..Default::default() },
        ..Default::default()
    }
}

fn folder(name: &str, role: FolderRole) -> Folder {
    Folder { name: name.into(), delimiter: None, role: Some(role), selectable: true }
}

const ROLES: [(&str, FolderRole); 6] = [
    ("INBOX", FolderRole::Inbox),
    ("Drafts", FolderRole::Drafts),
    ("Sent", FolderRole::Sent),
    ("Archive", FolderRole::Archive),
    ("Junk", FolderRole::Junk),
    ("Trash", FolderRole::Trash),
];

fn seed(sink: &mut StoreSink, tag: &str) {
    let all: Vec<Folder> = ROLES.iter().map(|(n, r)| folder(n, *r)).collect();
    sink.put_folders(&all).unwrap();
    for (n, (name, _)) in ROLES.iter().enumerate() {
        let items = vec![
            message(1, &format!("{tag} {name} a"), false, 1000 + n as i64),
            message(2, &format!("{tag} {name} b"), false, 2000 + n as i64),
            message(3, &format!("{tag} {name} c"), true, 3000 + n as i64),
        ];
        sink.put_summaries(name, &items).unwrap();
        sink.put_token(name, &SyncToken { uid_validity: 1, uid_next: 4, highest_mod_seq: None }).unwrap();
    }
}

fn setup(id: &str, name: &str) -> AccountSetup {
    AccountSetup { id: id.into(), name: name.into(), address: format!("{id}@x"), sender: String::new(), backend: Box::new(Offline) }
}

type State = (bool, App);

fn open(accounts: usize) -> (Harness<'static, State>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        for i in 0..accounts {
            let mut sink = StoreSink::new(&store, &format!("acct{i}"), true);
            seed(&mut sink, &format!("a{i}"));
        }
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let setups = (0..accounts).map(|i| setup(&format!("acct{i}"), &format!("acct{i}"))).collect();
    let real = RealStore::open(store, setups);
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
        if h.state().1.store.counts(&Query::new(ms::Folder::Trash)).all == 3 * accounts {
            break;
        }
        assert!(Instant::now() < deadline, "never loaded");
        std::thread::sleep(Duration::from_millis(50));
    }
    (h, dir)
}

fn settle(h: &mut Harness<'static, State>) {
    h.run();
    h.run();
    h.run();
}

fn count(h: &Harness<'static, State>, f: ms::Folder) -> usize {
    h.state().1.store.counts(&Query::new(f)).all
}

fn unread(h: &Harness<'static, State>, f: ms::Folder) -> usize {
    h.state().1.store.counts(&Query::new(f)).unread
}

fn go(h: &mut Harness<'static, State>, f: ms::Folder) {
    let app = &mut h.state_mut().1;
    app.folder = f;
    app.account = None;
    app.selected = 0;
    app.marked.clear();
    settle(h);
}

fn local_draft(h: &mut Harness<'static, State>, subject: &str) {
    let out = Outgoing { account: 0, to: "x@example.test".into(), subject: subject.into(), body: "hi".into(), ..Default::default() };
    h.state_mut().1.store.save_draft(out).unwrap();
    settle(h);
}

fn subjects(h: &Harness<'static, State>, f: ms::Folder) -> Vec<String> {
    h.state().1.store.list(&Query::new(f), 0, 100).items.into_iter().map(|s| s.subject).collect()
}

#[test]
fn delete_key_moves_a_server_draft_to_trash_and_undo_restores_it() {
    let (mut h, _d) = open(1);
    go(&mut h, ms::Folder::Drafts);
    let (drafts, trash, ops) = (count(&h, ms::Folder::Drafts), count(&h, ms::Folder::Trash), h.state().1.store.pending_ops());
    let subject = h.state().1.store.list(&Query::new(ms::Folder::Drafts), 0, 1).items[0].subject.clone();
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Drafts), drafts - 1);
    assert_eq!(count(&h, ms::Folder::Trash), trash + 1);
    assert!(subjects(&h, ms::Folder::Trash).contains(&subject));
    assert_eq!(h.state().1.store.pending_ops(), ops + 1);
    assert_eq!(h.state().1.notice.as_deref(), Some("Moved to trash"));
    h.key_press(egui::Key::Z);
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Drafts), drafts);
    assert_eq!(count(&h, ms::Folder::Trash), trash);
    assert_eq!(h.state().1.store.pending_ops(), ops);
}

#[test]
fn delete_key_moves_a_local_draft_to_trash_and_undo_restores_it() {
    let (mut h, _d) = open(1);
    local_draft(&mut h, "unsent thoughts");
    go(&mut h, ms::Folder::Drafts);
    let row = subjects(&h, ms::Folder::Drafts).iter().position(|s| s == "unsent thoughts").unwrap();
    h.state_mut().1.selected = row;
    let (drafts, trash) = (count(&h, ms::Folder::Drafts), count(&h, ms::Folder::Trash));
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Drafts), drafts - 1);
    assert_eq!(count(&h, ms::Folder::Trash), trash + 1);
    assert!(subjects(&h, ms::Folder::Trash).contains(&"unsent thoughts".to_string()));
    assert!(!subjects(&h, ms::Folder::Drafts).contains(&"unsent thoughts".to_string()));
    h.key_press(egui::Key::Z);
    settle(&mut h);
    assert!(subjects(&h, ms::Folder::Drafts).contains(&"unsent thoughts".to_string()));
    assert_eq!(count(&h, ms::Folder::Trash), trash);
}

#[test]
fn a_local_draft_in_trash_is_pushed_to_trash_not_drafts() {
    let (mut h, _d) = open(1);
    local_draft(&mut h, "unsent thoughts");
    go(&mut h, ms::Folder::Drafts);
    let row = subjects(&h, ms::Folder::Drafts).iter().position(|s| s == "unsent thoughts").unwrap();
    h.state_mut().1.selected = row;
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert_eq!(h.state().1.store.pending_ops(), 1);
}

#[test]
fn the_row_context_menu_deletes_a_draft() {
    let (mut h, _d) = open(1);
    go(&mut h, ms::Folder::Drafts);
    let (drafts, trash) = (count(&h, ms::Folder::Drafts), count(&h, ms::Folder::Trash));
    h.get_by_label_contains("a0 Drafts a").click_secondary();
    settle(&mut h);
    h.get_by_label("delete").click();
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Drafts), drafts - 1);
    assert_eq!(count(&h, ms::Folder::Trash), trash + 1);
}

#[test]
fn a_multi_selection_of_drafts_is_deleted_together() {
    let (mut h, _d) = open(1);
    local_draft(&mut h, "second draft");
    go(&mut h, ms::Folder::Drafts);
    let (drafts, trash) = (count(&h, ms::Folder::Drafts), count(&h, ms::Folder::Trash));
    h.state_mut().1.marked = [0usize, 1].into_iter().collect();
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Drafts), drafts - 2);
    assert_eq!(count(&h, ms::Folder::Trash), trash + 2);
    assert_eq!(h.state().1.notice.as_deref(), Some("Moved to trash (2)"));
}

#[test]
fn deleting_in_trash_asks_first_and_then_removes_for_good() {
    let (mut h, _d) = open(1);
    go(&mut h, ms::Folder::Trash);
    let (trash, ops) = (count(&h, ms::Folder::Trash), h.state().1.store.pending_ops());
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    h.get_by_label("cancel");
    assert_eq!(count(&h, ms::Folder::Trash), trash);
    h.get_by_label("cancel").click();
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Trash), trash);
    assert_eq!(h.state().1.store.pending_ops(), ops);
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    h.get_by_label("delete").click();
    settle(&mut h);
    assert_eq!(count(&h, ms::Folder::Trash), trash - 1);
    assert_eq!(h.state().1.store.pending_ops(), ops + 1);
}

#[test]
fn a_local_draft_in_trash_is_deleted_for_good() {
    let (mut h, _d) = open(1);
    local_draft(&mut h, "unsent thoughts");
    go(&mut h, ms::Folder::Drafts);
    let row = subjects(&h, ms::Folder::Drafts).iter().position(|s| s == "unsent thoughts").unwrap();
    h.state_mut().1.selected = row;
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    go(&mut h, ms::Folder::Trash);
    let row = subjects(&h, ms::Folder::Trash).iter().position(|s| s == "unsent thoughts").unwrap();
    h.state_mut().1.selected = row;
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    h.get_by_label("delete").click();
    settle(&mut h);
    assert!(!subjects(&h, ms::Folder::Trash).contains(&"unsent thoughts".to_string()));
    assert!(!subjects(&h, ms::Folder::Drafts).contains(&"unsent thoughts".to_string()));
}

#[test]
fn mark_all_as_read_works_in_every_role_folder() {
    for f in [ms::Folder::Junk, ms::Folder::Trash, ms::Folder::Drafts, ms::Folder::Sent, ms::Folder::Archive] {
        let (mut h, _d) = open(1);
        go(&mut h, f.clone());
        assert_eq!(unread(&h, f.clone()), 2, "{f:?}");
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::I);
        settle(&mut h);
        assert_eq!(unread(&h, f.clone()), 0, "{f:?}");
        assert_eq!(h.state().1.store.pending_ops(), 1, "{f:?}");
        h.key_press(egui::Key::Z);
        settle(&mut h);
        assert_eq!(unread(&h, f.clone()), 2, "{f:?}");
    }
}

#[test]
fn mark_all_as_read_in_an_account_folder_leaves_the_other_account_alone() {
    let (mut h, _d) = open(2);
    {
        let app = &mut h.state_mut().1;
        app.folder = ms::Folder::Junk;
        app.account = Some(1);
        app.selected = 0;
    }
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::I);
    settle(&mut h);
    let s = &h.state().1.store;
    assert_eq!(s.counts(&Query::in_account(ms::Folder::Junk, 1)).unread, 0);
    assert_eq!(s.counts(&Query::in_account(ms::Folder::Junk, 0)).unread, 2);
}

#[test]
fn mark_read_on_a_selection_works_in_every_role_folder() {
    for f in [ms::Folder::Junk, ms::Folder::Trash, ms::Folder::Drafts, ms::Folder::Sent, ms::Folder::Archive] {
        let (mut h, _d) = open(1);
        go(&mut h, f.clone());
        h.state_mut().1.marked = [1usize, 2].into_iter().collect();
        h.key_press(egui::Key::U);
        settle(&mut h);
        assert_eq!(unread(&h, f.clone()), 0, "{f:?}");
    }
}

#[test]
fn the_row_context_menu_marks_a_row_read() {
    let (mut h, _d) = open(1);
    go(&mut h, ms::Folder::Junk);
    h.get_by_label_contains("a0 Junk a").click_secondary();
    settle(&mut h);
    h.get_by_label("mark read").click();
    settle(&mut h);
    assert_eq!(unread(&h, ms::Folder::Junk), 1);
}

#[test]
fn the_sidebar_shows_unread_counts_for_every_role_folder() {
    let (h, _d) = open(2);
    for f in [ms::Folder::Inbox, ms::Folder::Junk, ms::Folder::Trash, ms::Folder::Drafts, ms::Folder::Sent, ms::Folder::Archive] {
        let label = format!("folder {}", f.name());
        let node = h.get_by_label(&label);
        assert_eq!(node.value().as_deref(), Some("4"), "{f:?}");
    }
    for f in [ms::Folder::Junk, ms::Folder::Trash, ms::Folder::Drafts] {
        let label = format!("acct1 {}", f.name());
        let node = h.get_by_label(&label);
        assert_eq!(node.value().as_deref(), Some("2"), "{f:?}");
    }
}
