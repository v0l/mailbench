use crate::accounts::{self, AccountConfig};
use crate::mailstore::{Filter, FlagChange, Folder, MailStore, MessageId, Outgoing, Query, Summary};
use crate::real::RealStore;
use crate::shell::default_data_dir;
use crate::store::{OsKeychain, Store, Unlock};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(name = "mailbench", version, about = "A keyboard-first desktop mail client. Without a subcommand it opens the window.")]
pub struct Cli {
    #[arg(long, global = true, hide = true)]
    pub demo: bool,
    #[arg(long, global = true, help = "Only use this account (id, name or address)")]
    pub account: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Accounts(JsonFlag),
    Folders(JsonFlag),
    List(ListArgs),
    Read(ReadArgs),
    Search(SearchArgs),
    Send(SendArgs),
    Sync,
    Archive(Ids),
    Flag(Ids),
    #[command(name = "mark-read")]
    MarkRead(Ids),
    Unread(Ids),
}

#[derive(Args, Debug)]
pub struct JsonFlag {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long, default_value = "inbox")]
    pub folder: String,
    #[arg(long)]
    pub unread: bool,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ReadArgs {
    pub id: String,
    #[arg(long, conflicts_with_all = ["text", "json"])]
    pub raw: bool,
    #[arg(long, conflicts_with = "json")]
    pub text: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    pub query: String,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SendArgs {
    #[arg(long, required = true)]
    pub to: Vec<String>,
    #[arg(long)]
    pub cc: Vec<String>,
    #[arg(long)]
    pub subject: String,
    #[arg(long, value_name = "FILE", help = "Body from a file, or - for standard input")]
    pub body_file: Option<String>,
    #[arg(long)]
    pub attach: Vec<PathBuf>,
    #[arg(long, help = "Show what would be sent and send nothing")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct Ids {
    #[arg(required = true)]
    pub ids: Vec<String>,
}

type Res<T> = Result<T, String>;

const SETTLE: Duration = Duration::from_secs(120);

fn until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !cond() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

fn prompt_password() -> Res<String> {
    if let Ok(p) = std::env::var("MAILBENCH_PASSWORD") {
        return Ok(p);
    }
    rpassword::prompt_password("Password: ").map_err(|e| format!("could not read the password: {e}"))
}

fn open_store() -> Res<Store> {
    let db = default_data_dir().join("mail.db");
    if !db.exists() {
        return Err(format!("no data file at {}; open the app once to set it up", db.display()));
    }
    let methods = Store::methods(&db).map_err(|e| format!("could not read the data file: {e:?}"))?;
    if methods.keychain {
        if let Ok(store) = Store::open(&db, &[Unlock::Keychain(&OsKeychain)]) {
            return Ok(store);
        }
        if !methods.password {
            return Err("the system keychain did not unlock the data and there is no password".into());
        }
    }
    let password = prompt_password()?;
    Store::open(&db, &[Unlock::Password(&password)]).map_err(|e| format!("could not unlock: {e:?}"))
}

fn chosen(store: &Store, wanted: &Option<String>) -> Res<Vec<AccountConfig>> {
    let all = accounts::load_all(store);
    if all.is_empty() {
        return Err("no accounts; add one in the app".into());
    }
    let Some(w) = wanted else { return Ok(all) };
    let w = w.to_lowercase();
    let found: Vec<_> = all
        .into_iter()
        .filter(|c| c.id.to_lowercase() == w || c.name.to_lowercase() == w || c.address.to_lowercase() == w)
        .collect();
    if found.is_empty() { Err(format!("no account matches {w}")) } else { Ok(found) }
}

struct Session {
    real: RealStore,
}

fn session(cli: &Cli) -> Res<Session> {
    let store = open_store()?;
    let configs = chosen(&store, &cli.account)?;
    let setups: Vec<_> = configs.iter().filter_map(|c| accounts::setup_for(c, &store).ok()).collect();
    let clone = store.try_clone().map_err(|e| format!("{e:?}"))?;
    let real = RealStore::open(clone, setups);
    real.set_send_hold(Duration::ZERO);
    if !until(SETTLE, || real.settled()) {
        return Err("timed out waiting for the first sync".into());
    }
    for a in real.accounts() {
        if let Some(d) = a.detail {
            eprintln!("warning: {}: {d}", a.address);
        }
    }
    Ok(Session { real })
}

fn parse_id(text: &str) -> Res<MessageId> {
    u64::from_str_radix(text.trim_start_matches("0x"), 16).map(MessageId).map_err(|_| format!("{text} is not a message id"))
}

fn folder_query(name: &str, unread: bool) -> Query {
    let known = Folder::UNIFIED.iter().chain(std::iter::once(&Folder::Snoozed)).find(|f| f.name() == name.to_lowercase());
    let filter = if unread { Filter::Unread } else { Filter::All };
    match known {
        Some(f) => Query { folder: f.clone(), filter, text: String::new() },
        None => Query { folder: Folder::Inbox, filter, text: format!("in:{name}") },
    }
}

fn id_text(id: MessageId) -> String {
    format!("{:x}", id.0)
}

fn summary_json(s: &Summary) -> Value {
    json!({
        "id": id_text(s.id),
        "from": s.from,
        "subject": s.subject,
        "when": s.when,
        "unread": s.unread,
        "flagged": s.flagged,
        "attachments": s.attachments,
        "account": s.account,
        "thread": s.thread,
    })
}

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn print_summaries(items: &[Summary], json: bool) {
    if json {
        print_json(&Value::Array(items.iter().map(summary_json).collect()));
        return;
    }
    for s in items {
        let flags = format!("{}{}{}", if s.unread { "N" } else { "-" }, if s.flagged { "F" } else { "-" }, if s.attachments { "A" } else { "-" });
        println!("{}\t{}\t{}\t{}\t{}", id_text(s.id), flags, s.when, s.from, s.subject);
    }
}

fn accounts_cmd(cli: &Cli, json: bool) -> Res<()> {
    let store = open_store()?;
    let configs = chosen(&store, &cli.account)?;
    if json {
        print_json(&Value::Array(
            configs
                .iter()
                .map(|c| json!({"id": c.id, "name": c.name, "address": c.address, "imap": c.imap.host, "smtp": c.smtp.host}))
                .collect(),
        ));
    } else {
        for c in &configs {
            println!("{}\t{}\t{}", c.id, c.name, c.address);
        }
    }
    Ok(())
}

fn folders_cmd(cli: &Cli, json: bool) -> Res<()> {
    let s = session(cli)?;
    let tree = s.real.tree();
    if json {
        print_json(&Value::Array(
            tree.iter()
                .map(|t| json!({"account": t.account, "path": t.path, "unread": t.unread, "total": t.total}))
                .collect(),
        ));
    } else {
        for t in &tree {
            println!("{}\t{}\t{}\t{}", t.account, t.path, t.unread, t.total);
        }
    }
    Ok(())
}

fn list_cmd(cli: &Cli, a: &ListArgs) -> Res<()> {
    let s = session(cli)?;
    let q = folder_query(&a.folder, a.unread);
    let first = s.real.list(&q, 0, a.limit);
    until(Duration::from_secs(20), || !s.real.searching(&q));
    let page = if q.text.is_empty() { first } else { s.real.list(&q, 0, a.limit) };
    print_summaries(&page.items, a.json);
    Ok(())
}

fn search_cmd(cli: &Cli, a: &SearchArgs) -> Res<()> {
    let s = session(cli)?;
    let folders: Vec<Folder> = if a.query.contains("in:") { vec![Folder::Inbox] } else { Folder::UNIFIED.to_vec() };
    let queries: Vec<Query> = folders
        .into_iter()
        .map(|folder| Query { folder, filter: Filter::All, text: a.query.clone() })
        .collect();
    s.real.list(&queries[0], 0, a.limit);
    until(Duration::from_secs(20), || !s.real.searching(&queries[0]));
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for q in &queries {
        for item in s.real.list(q, 0, a.limit).items {
            if seen.insert(item.id) {
                items.push(item);
            }
        }
    }
    items.truncate(a.limit);
    print_summaries(&items, a.json);
    Ok(())
}

fn read_cmd(cli: &Cli, a: &ReadArgs) -> Res<()> {
    let s = session(cli)?;
    let id = parse_id(&a.id)?;
    let mut message = s.real.get(id).map_err(|_| format!("no message {}", a.id))?;
    if a.raw {
        let mut raw = None;
        until(Duration::from_secs(30), || {
            let _ = s.real.get(id);
            raw = s.real.raw(id).ok().flatten();
            raw.is_some()
        });
        let raw = raw.ok_or("the message body could not be fetched")?;
        std::io::stdout().write_all(&raw).map_err(|e| e.to_string())?;
        return Ok(());
    }
    until(Duration::from_secs(30), || {
        message = s.real.get(id).unwrap_or(message.clone());
        message.body_state != crate::mailstore::BodyState::Loading
    });
    if message.body_state != crate::mailstore::BodyState::Ready {
        return Err("the message body could not be fetched".into());
    }
    let body = message.paragraphs.join("\n\n");
    if a.json {
        print_json(&json!({
            "id": id_text(id),
            "from": message.summary.from,
            "from_address": message.from_address,
            "to": message.to,
            "cc": message.cc,
            "subject": message.summary.subject,
            "sent": message.sent,
            "folder": message.folder.name(),
            "unread": message.summary.unread,
            "flagged": message.summary.flagged,
            "message_id": message.message_id,
            "attachments": message.attachments.iter().map(|x| json!({"name": x.name, "size": x.size, "mime": x.mime})).collect::<Vec<_>>(),
            "body": body,
            "quoted": message.quoted,
        }));
        return Ok(());
    }
    if !a.text {
        println!("From: {} <{}>", message.summary.from, message.from_address);
        println!("To: {}", message.to.join(", "));
        if !message.cc.is_empty() {
            println!("Cc: {}", message.cc.join(", "));
        }
        println!("Date: {}", message.sent);
        println!("Subject: {}", message.summary.subject);
        for x in &message.attachments {
            println!("Attachment: {} ({}, {})", x.name, x.mime, x.size);
        }
        println!();
    }
    println!("{body}");
    Ok(())
}

fn read_body(source: &Option<String>) -> Res<String> {
    match source.as_deref() {
        None => Ok(String::new()),
        Some("-") => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).map_err(|e| e.to_string())?;
            Ok(text)
        }
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}")),
    }
}

fn send_cmd(cli: &Cli, a: &SendArgs) -> Res<()> {
    let body = read_body(&a.body_file)?;
    let mut attachments = Vec::new();
    for path in &a.attach {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into());
        attachments.push(crate::backend::Attachment { name, mime: "application/octet-stream".into(), data });
    }
    let store = open_store()?;
    let configs = chosen(&store, &cli.account)?;
    drop(store);
    if configs.len() > 1 && cli.account.is_none() {
        eprintln!("note: sending from {}; pick another with --account", configs[0].address);
    }
    let from = configs[0].address.clone();
    if a.dry_run {
        println!("From: {from}\nTo: {}\nSubject: {}\nAttachments: {}\nBytes: {}", a.to.join(", "), a.subject, attachments.len(), body.len());
        return Ok(());
    }
    let mut s = session(&Cli { demo: false, account: Some(configs[0].id.clone()), command: None })?;
    let message = Outgoing {
        account: 0,
        to: a.to.join(", "),
        cc: a.cc.join(", "),
        subject: a.subject.clone(),
        body,
        attachments,
        ..Default::default()
    };
    s.real.append(Folder::Sent, message).map_err(|e| format!("{e:?}"))?;
    if !until(SETTLE, || s.real.pending() == 0) {
        return Err("the message is queued but the server did not take it; it will go out next time the app or the CLI syncs".into());
    }
    println!("sent from {from}");
    Ok(())
}

fn change_cmd(cli: &Cli, ids: &Ids, apply: impl Fn(&mut RealStore, MessageId) -> Result<(), crate::mailstore::StoreError>) -> Res<()> {
    let mut s = session(cli)?;
    for text in &ids.ids {
        let id = parse_id(text)?;
        apply(&mut s.real, id).map_err(|e| format!("{text}: {e:?}"))?;
    }
    if !until(SETTLE, || s.real.pending() == 0) {
        return Err("changes are queued but the server did not take them yet".into());
    }
    Ok(())
}

pub fn run(cli: &Cli) -> Res<()> {
    let Some(command) = &cli.command else { return Ok(()) };
    match command {
        Command::Accounts(f) => accounts_cmd(cli, f.json),
        Command::Folders(f) => folders_cmd(cli, f.json),
        Command::List(a) => list_cmd(cli, a),
        Command::Read(a) => read_cmd(cli, a),
        Command::Search(a) => search_cmd(cli, a),
        Command::Send(a) => send_cmd(cli, a),
        Command::Sync => session(cli).map(|_| ()),
        Command::Archive(i) => change_cmd(cli, i, |r, id| r.move_to(id, Folder::Archive)),
        Command::Flag(i) => change_cmd(cli, i, |r, id| r.set_flags(id, FlagChange { unread: None, flagged: Some(true) })),
        Command::MarkRead(i) => change_cmd(cli, i, |r, id| r.set_flags(id, FlagChange { unread: Some(false), flagged: None })),
        Command::Unread(i) => change_cmd(cli, i, |r, id| r.set_flags(id, FlagChange { unread: Some(true), flagged: None })),
    }
}
