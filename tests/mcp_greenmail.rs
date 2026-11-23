use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query};
use mailbench::mcp::prefs::AgentPrefs;
use mailbench::mcp::tools::Tools;
use mailbench::mcp::{AccountRef, MailTools};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

mod common;

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap(name: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth(name)))
}

fn smtp(name: &str) -> SmtpBackend {
    SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth(name)))
}

fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn server_messages(name: &str, folder: &str) -> Vec<String> {
    let mut b = imap(name);
    let uids = block_on(b.search(folder, "ALL")).unwrap_or_default();
    uids.into_iter().filter_map(|u| block_on(b.fetch(folder, u)).ok()).map(|r| String::from_utf8_lossy(&r).to_string()).collect()
}

fn ok(v: &Value) -> &Value {
    assert_eq!(v["isError"], json!(false), "{v}");
    &v["structuredContent"]
}

fn tools(me: &str, dir: &tempfile::TempDir, send: bool) -> MailTools {
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let setup = AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{me}@localhost"),
        sender: "Me Myself".into(),
        backend: Box::new(Account { mail: Box::new(imap(me)), outbound: Box::new(smtp(me)) }),
    };
    let real = RealStore::open(store, vec![setup]);
    real.set_send_hold(Duration::ZERO);
    until("inbox sync", || real.settled());
    let mut prefs = AgentPrefs::default();
    prefs.set_allowed("acct", true);
    prefs.set_send("acct", send);
    let accounts = vec![AccountRef { id: "acct".into(), name: "test".into(), address: format!("{me}@localhost"), nostr: false }];
    MailTools::new(Box::new(real), accounts, Box::new(move || prefs.clone()), Some(dir.path().to_path_buf()))
}

fn users(prefix: &str) -> (String, String) {
    assert!(common::greenmail(), "docker is required for greenmail tests");
    let tag = format!("{prefix}{}", std::process::id());
    let (me, peer) = (format!("{tag}me"), format!("{tag}peer"));
    common::create_greenmail_user(&me);
    common::create_greenmail_user(&peer);
    (me, peer)
}

#[test]
fn an_agent_draft_is_not_delivered_until_send_is_allowed_and_then_goes_out_once_with_a_sent_copy() {
    let (me, peer) = users("mq");
    let dir = tempfile::tempdir().unwrap();
    let t = tools(&me, &dir, false);
    let v = t.call(
        "create_draft",
        &json!({"account": "test", "to": [format!("{peer}@localhost")], "subject": "From the agent", "body": "**bold** hello\n\nsecond paragraph"}),
    );
    let id = ok(&v)["draft_id"].as_str().unwrap().to_string();
    let queued = t.call("send_message", &json!({"draft_id": id}));
    assert_eq!(ok(&queued)["status"], "queued");
    std::thread::sleep(Duration::from_secs(3));
    assert!(server_messages(&peer, "INBOX").is_empty());
    drop(t);

    let dir2 = tempfile::tempdir().unwrap();
    let t = tools(&me, &dir2, true);
    let v = t.call("create_draft", &json!({"account": "test", "to": [format!("{peer}@localhost")], "subject": "Zebrafinch from the agent", "body": "**bold** hello\n\nsecond paragraph"}));
    let id = ok(&v)["draft_id"].as_str().unwrap().to_string();
    let sent = t.call("send_message", &json!({"draft_id": id}));
    assert_eq!(ok(&sent)["status"], "sent");
    until("delivery", || !server_messages(&peer, "INBOX").is_empty());
    std::thread::sleep(Duration::from_secs(2));
    let got = server_messages(&peer, "INBOX");
    assert_eq!(got.len(), 1, "sent exactly once");
    let raw = &got[0];
    assert!(raw.contains("Subject: Zebrafinch from the agent"), "{raw}");
    assert!(raw.contains("Me Myself"), "From name from the composer path: {raw}");
    assert!(raw.contains("format=flowed"), "{raw}");
    assert!(raw.contains("text/html"), "markdown alternative: {raw}");
    until("copy in Sent", || !server_messages(&me, "Sent").is_empty() || !server_messages(&me, "INBOX.Sent").is_empty());
    let lines: Vec<Value> = std::fs::read_to_string(dir2.path().join("mcp-audit.jsonl")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.iter().filter(|l| l["result"] == "sent").count(), 1);
    let mut left = 1;
    until("draft cleaned up", || {
        let r = t.call("search_messages", &json!({"query": "zebrafinch", "folder": "drafts"}));
        left = ok(&r)["total"].as_u64().unwrap();
        left == 0
    });
    assert_eq!(left, 0);
    let _ = (Query::new(ms::Folder::Inbox), ms::Folder::Sent);
}
