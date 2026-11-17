use mailbench::backend::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

mod common;
use common::{GM_IMAP as IMAP, GM_IMAPS as IMAPS, GM_SMTP as SMTP, GM_SMTPS as SMTPS};

fn user() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    assert!(common::greenmail(), "docker is required for greenmail tests");
    let name = format!("u{}x{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    common::create_greenmail_user(&name);
    name
}

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap(name: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", IMAP, Security::Plain, auth(name)))
}

fn smtp(name: &str) -> SmtpBackend {
    SmtpBackend::new(SmtpConfig::new("127.0.0.1", SMTP, Security::Plain, auth(name)))
}

fn mail(name: &str, subject: &str) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: Some("Sender".into()), address: "sender@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: subject.into(),
        body: format!("body of {subject}"),
        ..Default::default()
    }
}

fn wait_for(imap: &mut ImapBackend, count: usize) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let uids = imap.search("INBOX", "ALL").unwrap();
        if uids.len() >= count {
            return uids;
        }
        assert!(Instant::now() < deadline, "mail did not arrive");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn send_then_sync_into_sink() {
    let name = user();
    let mut s = smtp(&name);
    s.send(&mail(&name, "first")).unwrap();
    s.send(&mail(&name, "second")).unwrap();
    let mut i = imap(&name);
    wait_for(&mut i, 2);

    let mut sink = MemorySink::default();
    let report = sync_account(&mut i, &mut sink).unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(report.resets, 1);
    let inbox = &sink.messages["INBOX"];
    let subjects: Vec<_> = inbox.values().map(|m| m.subject.clone().unwrap()).collect();
    assert_eq!(subjects, ["first", "second"]);
    let m = inbox.values().next().unwrap();
    assert_eq!(m.from[0].address, "sender@localhost");
    assert_eq!(m.from[0].name.as_deref(), Some("Sender"));
    assert!(m.message_id.is_some());
    assert!(m.date.is_some());
    assert!(sink.tokens["INBOX"].uid_next > 2);

    let report = sync_account(&mut i, &mut sink).unwrap();
    assert_eq!((report.added, report.flag_changes, report.resets), (0, 0, 0));
}

#[test]
fn incremental_sync_picks_up_new_mail_flags_and_expunges() {
    let name = user();
    let mut s = smtp(&name);
    let mut i = imap(&name);
    s.send(&mail(&name, "one")).unwrap();
    s.send(&mail(&name, "two")).unwrap();
    let uids = wait_for(&mut i, 2);

    let mut sink = MemorySink::default();
    sync_account(&mut i, &mut sink).unwrap();

    s.send(&mail(&name, "three")).unwrap();
    wait_for(&mut i, 3);
    let seen = Flags { seen: true, ..Default::default() };
    i.set_flags("INBOX", &uids[..1], &FlagUpdate::Add(seen)).unwrap();
    i.delete_messages("INBOX", &uids[1..2]).unwrap();

    let report = sync_account(&mut i, &mut sink).unwrap();
    assert_eq!(report.resets, 0);
    assert_eq!(report.added, 1);
    let inbox = &sink.messages["INBOX"];
    assert_eq!(inbox.len(), 2);
    assert!(inbox[&uids[0]].flags.seen);
    assert!(!inbox.contains_key(&uids[1]));
    assert_eq!(inbox.values().last().unwrap().subject.as_deref(), Some("three"));
}

#[test]
fn stale_uidvalidity_resets_folder() {
    let name = user();
    let mut s = smtp(&name);
    let mut i = imap(&name);
    s.send(&mail(&name, "a")).unwrap();
    wait_for(&mut i, 1);
    let stale = SyncToken { uid_validity: 1, uid_next: 50, highest_mod_seq: None };
    let delta = i.sync_folder("INBOX", Some(&stale)).unwrap();
    assert!(delta.reset);
    assert_eq!(delta.added.len(), 1);
}

#[test]
fn fetch_returns_full_message() {
    let name = user();
    smtp(&name).send(&mail(&name, "fetch me")).unwrap();
    let mut i = imap(&name);
    let uid = wait_for(&mut i, 1)[0];
    let raw = i.fetch("INBOX", uid).unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains("Subject: fetch me"));
    assert!(text.contains("body of fetch me"));
}

#[test]
fn move_creates_message_in_destination_and_removes_source() {
    let name = user();
    smtp(&name).send(&mail(&name, "mover")).unwrap();
    let mut i = imap(&name);
    let uid = wait_for(&mut i, 1)[0];
    i.create_folder("Archive").unwrap();
    i.move_messages("INBOX", &[uid], "Archive").unwrap();
    assert!(i.search("INBOX", "ALL").unwrap().is_empty());
    assert_eq!(i.search("Archive", "ALL").unwrap().len(), 1);
    let folders = i.list_folders().unwrap();
    assert!(folders.iter().any(|f| f.name == "Archive"));
    assert!(folders.iter().any(|f| f.role == Some(FolderRole::Inbox)));
}

#[test]
fn search_passes_query_through() {
    let name = user();
    let mut s = smtp(&name);
    s.send(&mail(&name, "needle")).unwrap();
    s.send(&mail(&name, "hay")).unwrap();
    let mut i = imap(&name);
    wait_for(&mut i, 2);
    assert_eq!(i.search("INBOX", "SUBJECT needle").unwrap().len(), 1);
    assert_eq!(i.search("INBOX", "UNSEEN").unwrap().len(), 2);
}

#[test]
fn watch_reports_new_mail_via_idle() {
    let name = user();
    let mut i = imap(&name);
    assert!(i.capabilities().unwrap().idle);
    let watch = i.watch("INBOX").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    smtp(&name).send(&mail(&name, "push")).unwrap();
    let event = watch.events.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(event, Event::FolderChanged { folder: "INBOX".into() });
}

#[test]
fn tls_connections_work_for_imap_and_smtp() {
    let name = user();
    let mut cfg = SmtpConfig::new("127.0.0.1", SMTPS, Security::Tls, auth(&name));
    cfg.accept_invalid_certs = true;
    SmtpBackend::new(cfg).send(&mail(&name, "tls")).unwrap();
    let mut cfg = ImapConfig::new("127.0.0.1", IMAPS, Security::Tls, auth(&name));
    cfg.accept_invalid_certs = true;
    let mut i = ImapBackend::new(cfg);
    let deadline = Instant::now() + Duration::from_secs(15);
    while i.search("INBOX", "ALL").unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn tls_rejects_untrusted_certificate_by_default() {
    let name = user();
    let cfg = ImapConfig::new("127.0.0.1", IMAPS, Security::Tls, auth(&name));
    assert!(ImapBackend::new(cfg).list_folders().is_err());
}

#[test]
fn bad_password_is_an_auth_error() {
    let name = user();
    let bad = Auth::Password { user: name, password: "wrong".into() };
    let mut i = ImapBackend::new(ImapConfig::new("127.0.0.1", IMAP, Security::Plain, bad));
    assert!(matches!(i.list_folders(), Err(BackendError::Auth(_))));
}

#[test]
fn account_routes_send_to_smtp_and_reads_to_imap() {
    let name = user();
    let mut account = Account { mail: Box::new(imap(&name)), outbound: Box::new(smtp(&name)) };
    assert!(account.capabilities().unwrap().send);
    account.send(&mail(&name, "via account")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while account.search("INBOX", "ALL").unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn smtp_backend_has_no_mail_access() {
    let name = user();
    assert!(matches!(smtp(&name).list_folders(), Err(BackendError::Unsupported(_))));
}

#[test]
fn smtp_rejects_message_without_recipients() {
    let name = user();
    let mut m = mail(&name, "x");
    m.to.clear();
    assert!(smtp(&name).send(&m).is_err());
}
