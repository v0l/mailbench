use egui_kittest::Harness;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{Folder as UiFolder, Outgoing, Query, SyncState};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store, Unlock};
use std::path::Path;
use std::time::{Duration, Instant};

mod common;

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap_at(name: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth(name)))
}

type State = (bool, App);

fn open_app(path: &Path, name: &str, size: egui::Vec2) -> Harness<'static, State> {
    let store = Store::open(path, &[Unlock::Password("pw")]).unwrap();
    let setup = AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: String::new(),
        backend: Box::new(imap_at(name)),
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
    h.run();
    h
}

fn until(h: &mut Harness<'static, State>, what: &str, cond: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond(&h.state().1) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
        h.run();
    }
}

fn count(app: &App, f: UiFolder) -> usize {
    app.store.counts(&Query::new(f)).all
}

fn on_server(name: &str, folder: &str) -> usize {
    block_on(imap_at(name).search(folder, "ALL")).map(|u| u.len()).unwrap_or(0)
}

fn unseen_on_server(name: &str, folder: &str) -> usize {
    block_on(imap_at(name).search(folder, "UNSEEN")).map(|u| u.len()).unwrap_or(0)
}

fn junk_mail(n: usize) -> Vec<u8> {
    format!("From: Spam <spam{n}@example.test>\r\nTo: x@localhost\r\nSubject: Win big {n}\r\nMessage-ID: <junk{n}@example.test>\r\nDate: Mon, 01 Jan 2024 10:0{n}:00 +0000\r\n\r\nbody\r\n").into_bytes()
}

#[test]
fn deleting_a_draft_and_marking_junk_read_reach_the_server() {
    if !common::greenmail() {
        return;
    }
    let name = format!("fa{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut b = imap_at(&name);
    block_on(b.create_folder("Junk")).ok();
    for n in 1..=3 {
        block_on(b.append_flagged("Junk", &Flags::default(), &junk_mail(n))).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();

    let mut h = open_app(&path, &name, egui::vec2(1280.0, 800.0));
    until(&mut h, "junk synced", |a| count(a, UiFolder::Junk) == 3 && a.store.accounts()[0].sync == SyncState::Idle);
    for subject in ["first draft", "second draft"] {
        let out = Outgoing { account: 0, to: "someone@example.test".into(), subject: subject.into(), body: "hi".into(), ..Default::default() };
        h.state_mut().1.store.save_draft(out).unwrap();
    }
    until(&mut h, "drafts on the server", |_| on_server(&name, "Drafts") == 2);
    until(&mut h, "drafts listed", |a| count(a, UiFolder::Drafts) == 2);

    h.state_mut().1.folder = UiFolder::Drafts;
    h.run();
    h.run();
    if let Ok(dir) = std::env::var("SHOT_DIR") {
        h.render().expect("render").save(format!("{dir}/drafts-wide.png")).unwrap();
    }
    h.key_press(egui::Key::Delete);
    h.run();
    h.run();
    until(&mut h, "outbox drained", |a| a.store.pending_ops() == 0);
    until(&mut h, "draft in trash", |a| count(a, UiFolder::Drafts) == 1 && count(a, UiFolder::Trash) == 1);
    assert_eq!(on_server(&name, "Drafts"), 1);
    assert_eq!(on_server(&name, "Trash"), 1);
    if let Ok(dir) = std::env::var("SHOT_DIR") {
        h.render().expect("render").save(format!("{dir}/drafts-after-delete-wide.png")).unwrap();
    }

    h.state_mut().1.folder = UiFolder::Junk;
    h.run();
    h.run();
    assert_eq!(unseen_on_server(&name, "Junk"), 3);
    if let Ok(dir) = std::env::var("SHOT_DIR") {
        h.render().expect("render").save(format!("{dir}/junk-unread-wide.png")).unwrap();
    }
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::I);
    h.run();
    h.run();
    until(&mut h, "outbox drained", |a| a.store.pending_ops() == 0);
    assert_eq!(unseen_on_server(&name, "Junk"), 0);
    assert_eq!(h.state().1.store.counts(&Query::new(UiFolder::Junk)).unread, 0);
    if let Ok(dir) = std::env::var("SHOT_DIR") {
        h.render().expect("render").save(format!("{dir}/junk-read-wide.png")).unwrap();
        h.set_size(egui::vec2(390.0, 844.0));
        h.run();
        h.run();
        h.render().expect("render").save(format!("{dir}/junk-read-phone.png")).unwrap();
        h.state_mut().1.folder = UiFolder::Drafts;
        h.run();
        h.run();
        h.render().expect("render").save(format!("{dir}/drafts-phone.png")).unwrap();
    }
}
