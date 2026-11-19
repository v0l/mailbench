use mailbench::accounts::{self, AccountConfig, Credential};
use mailbench::autodetect::ServerSettings;
use mailbench::backend::*;
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

mod common;

fn config(name: &str) -> AccountConfig {
    AccountConfig {
        id: "acct".into(),
        name: "test".into(),
        display_name: String::new(),
        signature: String::new(),
        address: format!("{name}@localhost"),
        username: name.to_string(),
        imap: ServerSettings::new("127.0.0.1", 3143, Security::Plain),
        smtp: ServerSettings::new("127.0.0.1", 3025, Security::Plain),
        credential: Credential::Password("secret".into()),
        backfill: Default::default(),
        notify: Default::default(),
    }
}

fn mailbench(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mailbench"))
        .args(args)
        .env("MAILBENCH_DATA", dir)
        .env("MAILBENCH_PASSWORD", "pw")
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = mailbench(dir, args);
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn seeded(prefix: &str) -> (String, tempfile::TempDir) {
    assert!(common::greenmail(), "docker is required for greenmail tests");
    let name = format!("{prefix}{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut smtp = SmtpBackend::new(SmtpConfig::new(
        "127.0.0.1",
        3025,
        Security::Plain,
        Auth::Password { user: name.clone(), password: "secret".into() },
    ));
    block_on(smtp.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "Quarterly pineapple report".into(),
        body: "The pineapples are ripe.".into(),
        ..Default::default()
    }))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    accounts::save(&store, &config(&name)).unwrap();
    (name, dir)
}

fn json_of(dir: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&ok(dir, args)).unwrap()
}

fn shape(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), json!("<id>"));
    }
    v
}

#[test]
fn list_read_search_and_flags_work_end_to_end_with_json() {
    if !common::greenmail() {
        return;
    }
    let (name, dir) = seeded("cl");
    let d = dir.path();

    let accounts = json_of(d, &["accounts", "--json"]);
    assert_eq!(accounts, json!([{"id": "acct", "name": "test", "address": format!("{name}@localhost"), "imap": "127.0.0.1", "smtp": "127.0.0.1"}]));

    let list = json_of(d, &["list", "--json"]);
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        shape(&items[0]),
        json!({"id": "<id>", "from": "Zed", "subject": "Quarterly pineapple report", "when": items[0]["when"], "unread": true, "flagged": false, "attachments": false, "account": 0, "thread": 1})
    );
    let id = items[0]["id"].as_str().unwrap().to_string();

    let read = json_of(d, &["read", &id, "--json"]);
    assert_eq!(read["subject"], "Quarterly pineapple report");
    assert_eq!(read["from_address"], "zed@localhost");
    assert!(read["body"].as_str().unwrap().contains("pineapples are ripe"));

    let text = ok(d, &["read", &id, "--text"]);
    assert_eq!(text.trim(), "The pineapples are ripe.");
    let raw = ok(d, &["read", &id, "--raw"]);
    assert!(raw.contains("Subject: Quarterly pineapple report"));

    let found = json_of(d, &["search", "pineapple", "--json"]);
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(found[0]["id"], id.as_str());
    assert_eq!(json_of(d, &["search", "zebra", "--json"]), json!([]));

    ok(d, &["mark-read", &id]);
    assert_eq!(json_of(d, &["list", "--unread", "--json"]), json!([]));
    ok(d, &["flag", &id]);
    assert_eq!(json_of(d, &["list", "--json"])[0]["flagged"], true);

    ok(d, &["archive", &id]);
    assert_eq!(json_of(d, &["list", "--json"]), json!([]));
    assert_eq!(json_of(d, &["list", "--folder", "archive", "--json"]).as_array().unwrap().len(), 1);
}

#[test]
fn send_delivers_through_smtp_and_a_second_account_run_sees_it() {
    if !common::greenmail() {
        return;
    }
    let (name, dir) = seeded("cs");
    let d = dir.path();
    let body = d.join("body.txt");
    std::fs::write(&body, "Sent from the terminal.").unwrap();
    let to = format!("{name}@localhost");
    let out = mailbench(d, &["send", "--to", &to, "--subject", "From the CLI", "--body-file", body.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let mut seen = false;
    for _ in 0..30 {
        let list = json_of(d, &["list", "--json"]);
        if list.as_array().unwrap().iter().any(|m| m["subject"] == "From the CLI") {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    assert!(seen, "the sent mail never reached the inbox");
}

#[test]
fn send_dry_run_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    accounts::save(&store, &config("nobody")).unwrap();
    drop(store);
    let out = ok(dir.path(), &["send", "--to", "a@b.c", "--subject", "Hi", "--dry-run"]);
    assert!(out.contains("To: a@b.c") && out.contains("Subject: Hi"));
}

#[test]
fn the_password_is_never_an_argument() {
    let out = Command::new(env!("CARGO_BIN_EXE_mailbench")).args(["list", "--password", "pw"]).output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn two_connections_write_the_same_store_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let first = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
    let second = first.try_clone().unwrap();
    let mode: String = rusqlite::Connection::open(&path).unwrap().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, "wal");
    let writer = std::thread::spawn(move || {
        for i in 0..200 {
            second.put("k", &format!("b{i}"), b"x").unwrap();
        }
    });
    for i in 0..200 {
        first.put("k", &format!("a{i}"), b"x").unwrap();
        first.list("k").unwrap();
    }
    writer.join().unwrap();
    assert_eq!(first.list("k").unwrap().len(), 400);
}

#[test]
fn snooze_and_mark_all_read_use_the_bulk_paths() {
    if !common::greenmail() {
        return;
    }
    let (_name, dir) = seeded("cb");
    let d = dir.path();
    let id = json_of(d, &["list", "--json"])[0]["id"].as_str().unwrap().to_string();
    assert!(ok(d, &["mark-all-read"]).contains("marked 1 read"));
    assert_eq!(json_of(d, &["list", "--unread", "--json"]), json!([]));
    ok(d, &["snooze", &id, "--hours", "2"]);
    assert_eq!(json_of(d, &["list", "--json"]), json!([]));
    assert_eq!(json_of(d, &["list", "--folder", "snoozed", "--json"]).as_array().unwrap().len(), 1);
}
