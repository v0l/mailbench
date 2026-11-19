use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query};
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

const DPD_LOGO_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAFAAAAAoCAIAAADmAupWAAAASklEQVR4nO3PAREAEADAQPQPJpYW3L19gm2OPb6yXgfc1rCuYV3DuoZ1Desa1jWsa1jXsK5hXcO6hnUN6xrWNaxrWNewrmFdw7oDc6UBGKimdEUAAAAASUVORK5CYII=";

fn dpd_mail(name: &str) -> String {
    format!(
        "From: DPD Ireland <noreply@dpd.ie>\r\nTo: {name}@localhost\r\nSubject: Your DPD order will be delivered today\r\nDate: Wed, 18 Nov 2026 10:31:00 +0000\r\nMessage-ID: <dpd1@dpd.ie>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"rel\"\r\n\r\n--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body style=\"margin:0;background:#f2f2f2\"><table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\"><tr><td align=\"center\"><table width=\"600\" style=\"background:#ffffff\"><tr><td style=\"background:#dc0032;padding:16px\"><img src=\"cid:dpdlogo\" alt=\"dpdlogo\" width=\"80\" height=\"40\"></td></tr><tr><td style=\"padding:24px;font-family:Arial\"><h2 style=\"color:#414042\">We're delivering your DPD parcel</h2><p style=\"font-size:28px;color:#dc0032;font-weight:bold\">TODAY</p><p>Your parcel will be delivered between 13:33 - 14:33.</p><p><a href=\"https://track.dpd.example/manage\" style=\"display:inline-block;background:#dc0032;color:#fff;padding:10px 18px;text-decoration:none\">Manage Your Delivery</a></p></td></tr></table></td></tr></table></body></html>\r\n--rel\r\nContent-Type: image/png\r\nContent-ID: <dpdlogo>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\n{DPD_LOGO_PNG}\r\n--rel--\r\n"
    )
}

#[test]
fn html_mail_renders_with_its_inline_logo_and_no_fake_attachments() {
    if !common::greenmail() {
        return;
    }
    let name = format!("rh{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut b = imap(&name);
    block_on(b.append("INBOX", dpd_mail(&name).as_bytes())).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut h = harness(RealStore::open(store, vec![setup(&name)]));
    until(&mut h, "mail in the inbox", |a| inbox(a) == 1);
    let id = h.state().1.store.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    until(&mut h, "html body fetched", |a| !a.store.get(id).unwrap().html.is_empty());
    let msg = h.state().1.store.get(id).unwrap();
    assert!(msg.attachments.is_empty(), "inline logo listed as attachment: {:?}", msg.attachments);
    assert_eq!(msg.html.inline.len(), 1);
    let deadline = Instant::now() + Duration::from_secs(30);
    let red = |img: &image::RgbaImage| img.pixels().filter(|p| p[0] > 200 && p[1] < 40 && p[2] > 30 && p[2] < 80).count();
    let green = |img: &image::RgbaImage| img.pixels().filter(|p| p[0] < 30 && p[1] > 180 && p[2] < 30).count();
    let img = loop {
        h.run();
        let img = h.render().expect("render");
        if (red(&img) > 2000 && green(&img) > 1000) || Instant::now() > deadline {
            break img;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
    img.save(dir.join("reader-html.png")).unwrap();
    assert!(red(&img) > 2000, "the DPD red header and button were not painted");
    assert!(green(&img) > 1000, "the inline cid logo was not painted");
}

#[test]
fn a_new_folder_takes_a_message_moved_with_m_and_shows_in_the_tree() {
    if !common::greenmail() {
        return;
    }
    let name = format!("rf{}", std::process::id());
    common::create_greenmail_user(&name);
    block_on(smtp(&name).send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "File me".into(),
        body: "body".into(),
        ..Default::default()
    }))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut h = harness(RealStore::open(store, vec![setup(&name)]));
    until(&mut h, "mail in the inbox", |a| inbox(a) == 1);

    h.state_mut().1.store.create_folder(0, "Receipts").unwrap();
    until(&mut h, "folder in the tree", |a| a.store.tree().iter().any(|n| n.path == "Receipts"));
    let folder = ms::Folder::Other(0, "Receipts".into());

    h.key_press(egui::Key::M);
    h.run();
    h.query_all_by_role(egui::accesskit::Role::TextInput).last().expect("picker field").type_text("rec");
    h.run();
    h.get_by_label("move to Receipts");
    h.key_press(egui::Key::Enter);
    until(&mut h, "message listed in the new folder", |a| a.store.counts(&Query::new(folder.clone())).all == 1);
    assert_eq!(inbox(&h.state().1), 0);
    let deadline = Instant::now() + Duration::from_secs(30);
    while server_state(&name, "Receipts").len() != 1 {
        h.run();
        assert!(Instant::now() < deadline, "message did not reach Receipts on the server");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(server_state(&name, "INBOX").is_empty());
    until(&mut h, "folder row unread count after resync", |a| {
        a.store.tree().iter().any(|n| n.path == "Receipts" && n.total == 1)
    });
    h.get_by_label("Receipts");
    let listed = h.state().1.store.list(&Query::new(folder), 0, 5).items;
    assert_eq!(listed[0].subject, "File me");
}

fn threaded(name: &str, id: &str, refs: &str, subject: &str) -> String {
    format!("From: Zed <zed@localhost>\r\nTo: {name}@localhost\r\nSubject: {subject}\r\nDate: Wed, 18 Nov 2026 10:31:00 +0000\r\nMessage-ID: <{id}@t>\r\n{refs}MIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nbody\r\n")
}

#[test]
fn m_on_a_thread_row_moves_every_message_and_in_the_reader_only_the_one() {
    if !common::greenmail() {
        return;
    }
    let name = format!("rt{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut b = imap(&name);
    block_on(async {
        b.append("INBOX", threaded(&name, "one", "", "Plan").as_bytes()).await.unwrap();
        b.append("INBOX", threaded(&name, "two", "References: <one@t>\r\nIn-Reply-To: <one@t>\r\n", "Re: Plan").as_bytes()).await.unwrap();
        b.append("INBOX", threaded(&name, "three", "References: <one@t> <two@t>\r\nIn-Reply-To: <two@t>\r\n", "Re: Plan").as_bytes()).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut real = RealStore::open(store, vec![setup(&name)]);
    real.create_folder(0, "Projects").unwrap();
    let mut h = harness(real);
    until(&mut h, "one thread row of three", |a| {
        let page = a.store.list(&Query::new(ms::Folder::Inbox), 0, 5);
        page.total == 1 && page.items[0].thread == 3
    });
    until(&mut h, "folder in the tree", |a| a.store.tree().iter().any(|n| n.path == "Projects"));
    let projects = ms::Folder::Other(0, "Projects".into());

    h.key_press(egui::Key::M);
    h.run();
    h.query_all_by_role(egui::accesskit::Role::TextInput).last().expect("picker field").type_text("proj");
    h.run();
    h.key_press(egui::Key::Enter);
    until(&mut h, "whole thread in Projects", |a| a.store.counts(&Query::new(projects.clone())).all == 1);
    assert_eq!(inbox(&h.state().1), 0);
    let deadline = Instant::now() + Duration::from_secs(30);
    while server_state(&name, "Projects").len() != 3 {
        h.run();
        assert!(Instant::now() < deadline, "thread did not reach Projects on the server");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(server_state(&name, "INBOX").is_empty());
}

#[test]
fn a_due_snooze_returns_to_the_inbox_on_the_server_unread() {
    if !common::greenmail() {
        return;
    }
    let name = format!("sz{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut s = smtp(&name);
    block_on(s.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "Snooze me".into(),
        body: "later".into(),
        ..Default::default()
    }))
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(store, vec![setup(&name)]);
    let mut h = harness(real);
    until(&mut h, "mail in the inbox", |a| inbox(a) == 1);
    let id = h.state().1.store.list(&Query::new(ms::Folder::Inbox), 0, 10).items[0].id;
    h.state_mut().1.store.set_flags(id, ms::FlagChange { unread: Some(false), flagged: None }).unwrap();
    until(&mut h, "marked read on the server", |_| server_seen(&name, "INBOX"));

    let wake_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 + 12;
    h.state_mut().1.store.snooze(id, wake_at).unwrap();
    until(&mut h, "snoozed locally", |a| inbox(a) == 0 && a.store.counts(&Query::new(ms::Folder::Snoozed)).all == 1);
    let deadline = Instant::now() + Duration::from_secs(30);
    while server_state(&name, "Snoozed").len() != 1 {
        h.run();
        assert!(Instant::now() < deadline, "message did not reach the Snoozed folder, which had to be created");
        std::thread::sleep(Duration::from_millis(200));
    }
    until(&mut h, "snoozed listed after resync with its wake time", |a| {
        a.store.list(&Query::new(ms::Folder::Snoozed), 0, 1).items.first().is_some_and(|m| m.when.starts_with("wakes"))
    });
    assert!(server_state(&name, "INBOX").is_empty());

    let deadline = Instant::now() + Duration::from_secs(60);
    while server_state(&name, "INBOX").len() != 1 {
        h.run();
        assert!(Instant::now() < deadline, "the due message did not come back to INBOX");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(server_state(&name, "Snoozed").is_empty());
    assert!(!server_seen(&name, "INBOX"));
    until(&mut h, "back in the inbox as unread", |a| {
        a.store.list(&Query::new(ms::Folder::Inbox), 0, 1).items.first().is_some_and(|m| m.unread)
    });
}

fn server_seen(name: &str, folder: &str) -> bool {
    let mut b = imap(name);
    block_on(async {
        let delta = b.sync_folder(folder, None).await.unwrap();
        delta.added.first().is_some_and(|m| m.flags.seen)
    })
}
