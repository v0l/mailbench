use mailbench::fake::FakeStore;
use mailbench::mailstore::*;
use mailbench::mcp::audit;
use mailbench::mcp::prefs::AgentPrefs;
use mailbench::mcp::tools::Tools;
use mailbench::mcp::{AccountRef, MailTools};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

#[path = "mcp_tools.rs"]
#[allow(dead_code)]
mod base;

fn build(prefs: AgentPrefs, dir: Option<&std::path::Path>, clock: Arc<AtomicI64>, accounts: Vec<AccountRef>) -> Arc<MailTools> {
    let mut seeded = base::seeded();
    seeded.push(base::message(50, 0, Folder::Inbox, "Dana Dee", "Question", "Can you reply?"));
    let t = MailTools::new(Box::new(FakeStore::from_messages(seeded)), accounts, Box::new(move || prefs.clone()), dir.map(|d| d.to_path_buf()))
        .with_clock(Box::new(move || clock.load(Ordering::SeqCst)));
    Arc::new(t)
}

fn setup(send: bool) -> (Arc<MailTools>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut p = base::allow(&["kestrel", "home"]);
    if send {
        p.set_send("id-kestrel", true);
    }
    (build(p, Some(dir.path()), Arc::new(AtomicI64::new(1_700_000_000)), base::account_refs()), dir)
}

fn folder_total(t: &MailTools, folder: &str) -> u64 {
    let v = t.call("search_messages", &json!({"query": "", "folder": folder, "limit": 50}));
    base::ok(&v)["total"].as_u64().unwrap()
}

fn draft(t: &MailTools) -> String {
    let v = t.call("create_draft", &json!({"account": "kestrel", "to": ["Pat <pat@example.test>"], "cc": ["c@example.test"], "subject": "Hello there", "body": "Hi Pat\n\nbye"}));
    base::ok(&v)["draft_id"].as_str().unwrap().to_string()
}

fn audit_lines(dir: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(audit::path_in(dir)).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn create_draft_makes_a_draft_and_sends_nothing() {
    let (t, _d) = setup(true);
    let id = draft(&t);
    assert_eq!(folder_total(&t, "drafts"), 1);
    assert_eq!(folder_total(&t, "sent"), 0);
    let m = t.call("get_message", &json!({"id": id}));
    assert_eq!(base::ok(&m)["subject"], "Hello there");
}

#[test]
fn create_draft_for_a_disallowed_account_is_refused() {
    let (t, _d) = setup(true);
    assert_eq!(base::err(&t.call("create_draft", &json!({"account": "lists", "to": ["a@b.test"], "subject": "s", "body": "b"}))), "not allowed");
    assert_eq!(folder_total(&t, "drafts"), 0);
}

#[test]
fn create_draft_validates_recipients_and_counts() {
    let (t, _d) = setup(true);
    let many: Vec<String> = (0..21).map(|n| format!("p{n}@example.test")).collect();
    assert!(base::err(&t.call("create_draft", &json!({"account": "kestrel", "to": many, "subject": "s", "body": "b"}))).contains("at most 20"));
    let split: Vec<String> = (0..10).map(|n| format!("p{n}@example.test")).collect();
    assert!(base::err(&t.call("create_draft", &json!({"account": "kestrel", "to": split.clone(), "cc": split.clone(), "bcc": ["z@example.test"], "subject": "s", "body": "b"}))).contains("at most 20"));
    assert!(base::err(&t.call("create_draft", &json!({"account": "kestrel", "to": ["not an address"], "subject": "s", "body": "b"}))).contains("not an email address"));
    assert!(base::err(&t.call("create_draft", &json!({"account": "kestrel", "to": [], "subject": "s", "body": "b"}))).contains("recipient"));
    assert!(base::err(&t.call("create_draft", &json!({"account": "kestrel", "to": ["a@b.test\r\nBcc: x@y.test"], "subject": "s", "body": "b"}))).contains("control"));
    assert_eq!(folder_total(&t, "drafts"), 0);
    let twenty: Vec<String> = (0..20).map(|n| format!("p{n}@example.test")).collect();
    base::ok(&t.call("create_draft", &json!({"account": "kestrel", "to": twenty, "subject": "s", "body": "b"})));
}

#[test]
fn a_reply_draft_carries_the_thread_headers() {
    let (t, _d) = setup(true);
    let v = t.call("create_draft", &json!({"account": "kestrel", "to": ["d@example.test"], "subject": "Re: Question", "body": "yes", "reply_to_id": "32"}));
    let id = base::ok(&v)["draft_id"].as_str().unwrap().to_string();
    let m = t.call("get_message", &json!({"id": id}));
    assert_eq!(base::ok(&m)["subject"], "Re: Question");
}

#[test]
fn send_message_with_the_setting_off_queues_and_sends_nothing() {
    let (t, d) = setup(false);
    let id = draft(&t);
    let v = t.call("send_message", &json!({"draft_id": id}));
    assert_eq!(base::ok(&v)["status"], "queued");
    assert!(v["content"][0]["text"].as_str().unwrap().contains("queued for the user's approval"));
    assert_eq!(folder_total(&t, "sent"), 0);
    assert_eq!(folder_total(&t, "drafts"), 1);
    let log = audit_lines(d.path());
    assert_eq!(log.len(), 2);
    assert_eq!(log[1]["result"], "queued");
}

#[test]
fn the_draft_shows_the_agent_marker_for_the_app_banner() {
    let (t, _d) = setup(false);
    let id = draft(&t);
    let store = t.store_for_tests();
    let m = store.get(parse(&id)).unwrap();
    assert!(is_agent_draft(&m.headers));
}

fn parse(id: &str) -> MessageId {
    MessageId(u64::from_str_radix(id, 16).unwrap())
}

#[test]
fn an_agent_draft_with_a_normal_attachment_builds_sends_and_reads_back_unchanged() {
    use base64::Engine;
    let (t, _d) = setup(true);
    let bytes = vec![0u8, 1, 2, 250, 251, 252];
    let v = t.call(
        "create_draft",
        &json!({"account": "kestrel", "to": ["pat@example.test"], "subject": "with file", "body": "see attached", "attachments": [{"name": "board.bin", "mime": "application/x-board", "data_base64": base64::engine::general_purpose::STANDARD.encode(&bytes)}]}),
    );
    let id = base::ok(&v)["draft_id"].as_str().unwrap().to_string();
    let out = t.store_for_tests().open_draft(parse(&id)).unwrap();
    assert_eq!(out.attachments.len(), 1);
    assert_eq!(out.attachments[0].name, "board.bin");
    assert_eq!(out.attachments[0].mime, "application/x-board");
    assert_eq!(out.attachments[0].data, bytes);
    assert!(out.attachments[0].cid.is_none());
    let sent = t.call("send_message", &json!({"draft_id": id}));
    assert_eq!(base::ok(&sent)["status"], "sent");
    let sent_id = parse(base::ok(&sent)["sent_id"].as_str().unwrap());
    let data = t.store_for_tests().attachments(sent_id).unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].name, "board.bin");

    let message = mailbench::backend::OutgoingMessage {
        from: mailbench::backend::Address { name: None, address: "me@example.test".into() },
        to: vec![mailbench::backend::Address { name: None, address: "pat@example.test".into() }],
        body: "see attached".into(),
        attachments: out.attachments.clone(),
        html_alternative: true,
        ..Default::default()
    };
    let raw = mailbench::backend::render_message(&message, false).unwrap();
    assert!(!String::from_utf8_lossy(&raw).to_ascii_lowercase().contains("content-id"));
}

#[test]
fn send_message_with_the_setting_on_sends_once_and_audits_without_bodies() {
    let (t, d) = setup(true);
    let id = draft(&t);
    let v = t.call("send_message", &json!({"draft_id": id}));
    assert_eq!(base::ok(&v)["status"], "sent");
    assert_eq!(folder_total(&t, "sent"), 1);
    assert_eq!(folder_total(&t, "drafts"), 0);
    let again = t.call("send_message", &json!({"draft_id": id}));
    base::err(&again);
    assert_eq!(folder_total(&t, "sent"), 1);
    let log = audit_lines(d.path());
    let sent: Vec<&Value> = log.iter().filter(|l| l["result"] == "sent").collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["tool"], "send_message");
    assert_eq!(sent[0]["account"], "id-kestrel");
    assert_eq!(sent[0]["subject"], "Hello there");
    assert_eq!(sent[0]["recipients"], json!(["pat@example.test", "c@example.test"]));
    assert!(sent[0]["message_id"].as_str().unwrap().contains('@'));
    let raw = std::fs::read_to_string(audit::path_in(d.path())).unwrap();
    assert!(!raw.contains("Hi Pat") && !raw.contains("bye"));
}

#[test]
fn send_is_per_account_not_global() {
    let (t, _d) = setup(true);
    let v = t.call("create_draft", &json!({"account": "home", "to": ["x@example.test"], "subject": "from home", "body": "b"}));
    let id = base::ok(&v)["draft_id"].as_str().unwrap().to_string();
    assert_eq!(base::ok(&t.call("send_message", &json!({"draft_id": id})))["status"], "queued");
    assert_eq!(folder_total(&t, "sent"), 0);
}

#[test]
fn only_agent_drafts_can_be_sent_so_incoming_mail_cannot_be_resent() {
    let (t, _d) = setup(true);
    assert!(base::err(&t.call("send_message", &json!({"draft_id": "32"}))).contains("only drafts created"));
    assert_eq!(folder_total(&t, "sent"), 0);
}

#[test]
fn the_users_own_drafts_cannot_be_sent_by_an_agent() {
    let mut p = base::allow(&["kestrel"]);
    p.set_send("id-kestrel", true);
    let own = base::message(60, 0, Folder::Drafts, "me", "Users own draft", "half written");
    let t = MailTools::new(Box::new(FakeStore::from_messages(vec![own])), base::account_refs(), Box::new(move || p.clone()), None);
    assert!(base::err(&t.call("send_message", &json!({"draft_id": "3c"}))).contains("only drafts created"));
    assert_eq!(folder_total(&t, "sent"), 0);
}

#[test]
fn a_disallowed_account_draft_cannot_be_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = base::allow(&["kestrel"]);
    p.set_send("id-home", true);
    let t = build(p, Some(dir.path()), Arc::new(AtomicI64::new(1_700_000_000)), base::account_refs());
    assert_eq!(base::err(&t.call("send_message", &json!({"draft_id": "3"}))), "not allowed");
}

#[test]
fn the_twenty_first_send_in_an_hour_is_refused_then_allowed_after_the_hour() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(1_700_000_000));
    let mut p = base::allow(&["kestrel"]);
    p.set_send("id-kestrel", true);
    let t = build(p, Some(dir.path()), clock.clone(), base::account_refs());
    for n in 0..20 {
        let id = draft(&t);
        assert_eq!(base::ok(&t.call("send_message", &json!({"draft_id": id})))["status"], "sent", "send {n}");
    }
    let id = draft(&t);
    assert!(base::err(&t.call("send_message", &json!({"draft_id": id}))).contains("hourly send cap"));
    assert_eq!(folder_total(&t, "sent"), 20);
    clock.fetch_add(3601, Ordering::SeqCst);
    assert_eq!(base::ok(&t.call("send_message", &json!({"draft_id": id})))["status"], "sent");
}

#[test]
fn the_cap_survives_a_restart_because_it_reads_the_audit_file() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(1_700_000_000));
    let mut p = base::allow(&["kestrel"]);
    p.set_send("id-kestrel", true);
    let first = build(p.clone(), Some(dir.path()), clock.clone(), base::account_refs());
    for _ in 0..20 {
        let id = draft(&first);
        base::ok(&first.call("send_message", &json!({"draft_id": id})));
    }
    let second = build(p, Some(dir.path()), clock, base::account_refs());
    let id = draft(&second);
    assert!(base::err(&second.call("send_message", &json!({"draft_id": id}))).contains("hourly send cap"));
}

#[test]
fn nostr_accounts_draft_but_do_not_send_unless_their_setting_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut accounts = base::account_refs();
    accounts[0].nostr = true;
    let mut p = base::allow(&["kestrel"]);
    p.set_send("id-kestrel", true);
    let t = build(p.clone(), Some(dir.path()), Arc::new(AtomicI64::new(1_700_000_000)), accounts.iter().map(|a| AccountRef { id: a.id.clone(), name: a.name.clone(), address: a.address.clone(), nostr: a.nostr }).collect());
    let id = draft(&t);
    assert_eq!(base::ok(&t.call("send_message", &json!({"draft_id": id})))["status"], "queued");
    assert_eq!(folder_total(&t, "sent"), 0);
    p.nostr_send = true;
    let t = build(p, Some(dir.path()), Arc::new(AtomicI64::new(1_700_000_000)), accounts);
    let id = draft(&t);
    assert_eq!(base::ok(&t.call("send_message", &json!({"draft_id": id})))["status"], "sent");
}

#[test]
fn prefs_default_to_nothing_allowed_and_nothing_sendable() {
    let p = AgentPrefs::default();
    assert!(!p.enabled && p.allowed.is_empty() && p.send.is_empty() && !p.nostr_send);
    assert!(!p.may_send("any", false));
    let mut p = p;
    p.set_send("a", true);
    assert!(p.may_send("a", false));
    p.set_allowed("a", false);
    assert!(p.send.is_empty());
}

#[allow(dead_code)]
fn unused(_: Mutex<()>) {}
