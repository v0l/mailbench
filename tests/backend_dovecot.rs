use mailbench::backend::*;
use std::time::Duration;

mod common;
use common::{DC_IMAP as IMAP, DC_IMAPS as IMAPS};

fn server() -> bool {
    common::dovecot()
}

macro_rules! need_server {
    () => {
        if !server() {
            return;
        }
    };
}

fn unique(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!("{prefix}{}x{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn config(user: &str, port: u16, security: Security) -> ImapConfig {
    let mut cfg = ImapConfig::new(
        "127.0.0.1",
        port,
        security,
        Auth::Password { user: user.into(), password: "secret".into() },
    );
    cfg.accept_invalid_certs = true;
    cfg
}

fn raw_session(user: &str) -> imap::Session<imap::Connection> {
    imap::ClientBuilder::new("127.0.0.1", IMAPS)
        .mode(imap::ConnectionMode::Tls)
        .danger_skip_tls_verify(true)
        .connect()
        .unwrap()
        .login(user, "secret")
        .map_err(|e| e.0)
        .unwrap()
}

fn append(session: &mut imap::Session<imap::Connection>, folder: &str, subject: &str) {
    let msg = format!(
        "From: a@example.com\r\nTo: b@example.com\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.com>\r\nDate: Mon, 1 Jan 2024 10:00:00 +0000\r\n\r\nbody {subject}\r\n"
    );
    session.append(folder, msg.as_bytes()).finish().unwrap();
}

#[test]
fn starttls_login_and_list() {
    need_server!();
    let user = unique("st");
    let mut b = ImapBackend::new(config(&user, IMAP, Security::StartTls));
    let folders = b.list_folders().unwrap();
    assert!(folders.iter().any(|f| f.role == Some(FolderRole::Inbox)));
}

#[test]
fn starttls_is_required_by_server_for_plain_login() {
    need_server!();
    let user = unique("pl");
    let mut b = ImapBackend::new(config(&user, IMAP, Security::Plain));
    assert!(b.list_folders().is_err());
}

#[test]
fn server_advertises_condstore_and_backend_reports_it() {
    need_server!();
    let user = unique("cap");
    let mut b = ImapBackend::new(config(&user, IMAPS, Security::Tls));
    let caps = b.capabilities().unwrap();
    assert!(caps.condstore);
    assert!(caps.idle);
    assert!(caps.move_command);
}

#[test]
fn changedsince_returns_only_changed_flags() {
    need_server!();
    let user = unique("cs");
    let mut raw = raw_session(&user);
    for s in ["one", "two", "three"] {
        append(&mut raw, "INBOX", s);
    }
    let mut b = ImapBackend::new(config(&user, IMAPS, Security::Tls));
    let first = b.sync_folder("INBOX", None).unwrap();
    assert!(first.reset);
    assert_eq!(first.added.len(), 3);
    assert!(first.token.highest_mod_seq.is_some());

    raw.select("INBOX").unwrap();
    let target = first.added[1].uid;
    raw.uid_store(target.to_string(), "+FLAGS (\\Flagged)").unwrap();

    let second = b.sync_folder("INBOX", Some(&first.token)).unwrap();
    assert!(!second.reset);
    assert!(second.added.is_empty());
    assert_eq!(second.flag_changes.len(), 1);
    assert_eq!(second.flag_changes[0].0, target);
    assert!(second.flag_changes[0].1.flagged);
    assert!(second.token.highest_mod_seq > first.token.highest_mod_seq);

    let third = b.sync_folder("INBOX", Some(&second.token)).unwrap();
    assert!(third.flag_changes.is_empty());
}

#[test]
fn uidvalidity_change_forces_full_resync() {
    need_server!();
    let user = unique("uv");
    let mut raw = raw_session(&user);
    raw.create("Box").unwrap();
    append(&mut raw, "Box", "old1");
    append(&mut raw, "Box", "old2");
    let mut b = ImapBackend::new(config(&user, IMAPS, Security::Tls));
    let mut sink = MemorySink::default();
    sync_account(&mut b, &mut sink).unwrap();
    assert_eq!(sink.messages["Box"].len(), 2);
    let old_validity = sink.tokens["Box"].uid_validity;

    raw.delete("Box").unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    raw.create("Box").unwrap();
    append(&mut raw, "Box", "fresh");

    let report = sync_account(&mut b, &mut sink).unwrap();
    assert_eq!(report.resets, 1);
    assert_ne!(sink.tokens["Box"].uid_validity, old_validity);
    let subjects: Vec<_> = sink.messages["Box"].values().map(|m| m.subject.clone().unwrap()).collect();
    assert_eq!(subjects, ["fresh"]);
}
