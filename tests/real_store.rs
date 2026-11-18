use egui_kittest::Harness;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{self as ms, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store, Unlock};
use std::path::Path;
use std::time::{Duration, Instant};

mod common;

type State = (bool, App);

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap(name: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth(name)))
}

fn smtp(name: &str) -> SmtpBackend {
    SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth(name)))
}

fn setup(name: &str) -> AccountSetup {
    AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: String::new(),
        backend: Box::new(Account { mail: Box::new(imap(name)), outbound: Box::new(smtp(name)) }),
    }
}

fn harness(store: RealStore) -> Harness<'static, State> {
    let app = App::with_store(Box::new(store));
    Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
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
    )
}

fn until(h: &mut Harness<'static, State>, what: &str, cond: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run();
        if cond(&h.state().1) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn inbox(app: &App) -> usize {
    app.store.counts(&Query::new(ms::Folder::Inbox)).all
}

fn server_state(name: &str, folder: &str) -> Vec<(u32, bool)> {
    let mut b = imap(name);
    block_on(async {
        let mut out = Vec::new();
        for uid in b.search(folder, "ALL").await.unwrap_or_default() {
            let delta = b.sync_folder(folder, None).await.unwrap();
            let flagged = delta.added.iter().find(|m| m.uid == uid).map(|m| m.flags.flagged).unwrap_or(false);
            out.push((uid, flagged));
        }
        out
    })
}

fn open_store(path: &Path) -> Store {
    Store::open(path, &[Unlock::Password("pw")]).unwrap()
}

#[test]
fn synced_mail_shows_archives_flags_and_survives_restart() {
    if !common::greenmail() {
        return;
    }
    let name = format!("rs{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut s = smtp(&name);
    block_on(s.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "Real store hello".into(),
        body: "plain body line".into(),
        ..Default::default()
    }))
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(store, vec![setup(&name)]);
    let mut h = harness(real);
    until(&mut h, "mail in the inbox", |a| inbox(a) == 1);
    let first = h.state().1.store.list(&Query::new(ms::Folder::Inbox), 0, 10).items[0].clone();
    assert_eq!(first.subject, "Real store hello");
    assert_eq!(first.from, "Zed");
    let msg = h.state().1.store.get(first.id).unwrap();
    assert_eq!(msg.body_state, ms::BodyState::Loading);
    until(&mut h, "body fetched on open", |a| {
        a.store.get(first.id).unwrap().paragraphs == vec!["plain body line"]
    });

    h.key_press(egui::Key::X);
    until(&mut h, "flag on the server", |_| server_state(&name, "INBOX").iter().any(|(_, f)| *f));
    assert!(h.state().1.store.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].flagged);

    h.key_press(egui::Key::E);
    until(&mut h, "archived locally", |a| inbox(a) == 0);
    let deadline = Instant::now() + Duration::from_secs(30);
    while server_state(&name, "Archive").len() != 1 {
        h.run();
        assert!(Instant::now() < deadline, "message did not reach Archive on the server");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(server_state(&name, "INBOX").is_empty());
    until(&mut h, "archive listed after resync", |a| {
        a.store.counts(&Query::new(ms::Folder::Archive)).all == 1
    });
    let archived = h.state().1.store.list(&Query::new(ms::Folder::Archive), 0, 1).items[0].id;
    until(&mut h, "archived body fetched", |a| {
        a.store.get(archived).unwrap().paragraphs == vec!["plain body line"]
    });
    drop(h);

    let store = open_store(&path);
    let offline = AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: String::new(),
        backend: Box::new(ImapBackend::new(ImapConfig::new("127.0.0.1", 1, Security::Plain, auth(&name)))),
    };
    let real = RealStore::open(store, vec![offline]);
    let mut h = harness(real);
    until(&mut h, "archive after restart", |a| a.store.counts(&Query::new(ms::Folder::Archive)).all == 1);
    let app = &h.state().1;
    let item = app.store.list(&Query::new(ms::Folder::Archive), 0, 1).items[0].clone();
    assert_eq!(item.subject, "Real store hello");
    until(&mut h, "stored body parsed offline", |a| {
        a.store.get(item.id).unwrap().paragraphs == vec!["plain body line"]
    });
}
