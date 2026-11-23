use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{Folder as UiFolder, Query, SyncState};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store, Unlock};
use std::path::Path;
use std::time::{Duration, Instant};

mod common;

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap_at(name: &str, port: u16) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", port, Security::Plain, auth(name)))
}

fn open_app(path: &Path, name: &str, port: u16) -> Harness<'static, (bool, App)> {
    let store = Store::open(path, &[Unlock::Password("pw")]).unwrap();
    let setup = AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: String::new(),
        backend: Box::new(imap_at(name, port)),
    };
    let app = App::with_store(Box::new(RealStore::open(store, vec![setup])));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, app): &mut (bool, App)| {
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
    h.run();
    h
}

fn until(h: &mut Harness<'static, (bool, App)>, what: &str, cond: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond(&h.state().1) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
        h.run();
    }
}

fn count(app: &App, folder: UiFolder) -> usize {
    app.store.counts(&Query::new(folder)).all
}

fn on_server(name: &str, folder: &str) -> usize {
    block_on(imap_at(name, 3143).search(folder, "ALL")).map(|u| u.len()).unwrap_or(0)
}

#[test]
fn archiving_three_selected_rows_is_one_op_per_folder_and_undo_brings_them_back() {
    if !common::greenmail() {
        return;
    }
    let name = format!("bulk{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut smtp = SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth(&name)));
    for n in 0..3 {
        block_on(smtp.send(&OutgoingMessage {
            from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
            to: vec![Address { name: None, address: format!("{name}@localhost") }],
            subject: format!("Bulk {n}"),
            body: "body".into(),
            message_id: Some(format!("bulk-{n}-{name}@test")),
            ..Default::default()
        }))
        .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();

    let mut h = open_app(&path, &name, 3143);
    until(&mut h, "first sync", |a| count(a, UiFolder::Inbox) == 3 && a.store.accounts()[0].sync == SyncState::Idle);
    drop(h);

    let mut h = open_app(&path, &name, 1);
    until(&mut h, "offline", |a| a.store.accounts()[0].sync == SyncState::Failed);
    h.state_mut().1.marked = [0usize, 1, 2].into_iter().collect();
    h.key_press(egui::Key::E);
    h.run();
    assert_eq!(h.state().1.store.pending_ops(), 1);
    assert_eq!(count(&h.state().1, UiFolder::Inbox), 0);
    assert_eq!(count(&h.state().1, UiFolder::Archive), 3);
    h.key_press(egui::Key::Z);
    h.run();
    assert_eq!(h.state().1.store.pending_ops(), 0);
    assert_eq!(count(&h.state().1, UiFolder::Inbox), 3);
    drop(h);

    let mut h = open_app(&path, &name, 3143);
    until(&mut h, "online", |a| count(a, UiFolder::Inbox) == 3 && a.store.accounts()[0].sync == SyncState::Idle);
    h.state_mut().1.marked = [0usize, 1, 2].into_iter().collect();
    h.key_press(egui::Key::E);
    h.run();
    until(&mut h, "outbox drained", |a| a.store.pending_ops() == 0);
    until(&mut h, "archive synced", |a| count(a, UiFolder::Archive) == 3 && count(a, UiFolder::Inbox) == 0);
    assert_eq!(on_server(&name, "Archive"), 3);
    assert_eq!(on_server(&name, "INBOX"), 0);

    h.key_press(egui::Key::Z);
    h.run();
    until(&mut h, "reverse op drained", |a| a.store.pending_ops() == 0);
    until(&mut h, "back in the inbox", |a| count(a, UiFolder::Inbox) == 3 && count(a, UiFolder::Archive) == 0);
    assert_eq!(on_server(&name, "INBOX"), 3);
    assert_eq!(on_server(&name, "Archive"), 0);
}

fn seen_on_server(name: &str, folder: &str) -> Option<bool> {
    let delta = block_on(imap_at(name, 3143).sync_folder(folder, None)).ok()?;
    delta.added.first().map(|m| m.flags.seen)
}

#[test]
fn clicking_a_message_marks_it_seen_on_the_server() {
    if !common::greenmail() {
        return;
    }
    let name = format!("seen{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut smtp = SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth(&name)));
    block_on(smtp.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "Click to read".into(),
        body: "body".into(),
        ..Default::default()
    }))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();

    let mut h = open_app(&path, &name, 3143);
    until(&mut h, "first sync", |a| count(a, UiFolder::Inbox) == 1 && a.store.accounts()[0].sync == SyncState::Idle);
    assert_eq!(seen_on_server(&name, "INBOX"), Some(false));

    h.get_all_by_label_contains("Click to read").next().unwrap().click();
    h.run();
    h.run();
    until(&mut h, "outbox drained", |a| a.store.pending_ops() == 0);
    assert!(!h.state().1.store.list(&Query::new(UiFolder::Inbox), 0, 1).items[0].unread);
    assert_eq!(seen_on_server(&name, "INBOX"), Some(true));
}
