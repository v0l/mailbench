use super::audit;
use super::mail::{MailTools, Res, id_text, store_error};
use super::prefs::AgentPrefs;
use super::sanitize;
use super::tools::{definition, text_result};
use crate::mailstore::{Contact, Folder, OutAttachment, Outgoing, split_recipients};
use base64::Engine;
use serde_json::{Value, json};

pub const MAX_RECIPIENTS: usize = 20;
pub const SENDS_PER_HOUR: usize = 20;
pub const MAX_DRAFT_ATTACHMENTS: usize = 10;

pub fn definitions() -> Vec<Value> {
    let list = json!({"type": "array", "items": {"type": "string"}, "maxItems": MAX_RECIPIENTS});
    vec![
        definition(
            "create_draft",
            "Create a draft in the user's Drafts folder. Sends nothing. The user, or send_message when the user allowed it, sends it later.",
            json!({
                "account": {"type": "string"},
                "to": list,
                "cc": list,
                "bcc": list,
                "subject": {"type": "string"},
                "body": {"type": "string", "description": "Plain text; Markdown is rendered as in the composer"},
                "reply_to_id": {"type": "string", "description": "Message id this draft answers"},
                "attachments": {"type": "array", "maxItems": MAX_DRAFT_ATTACHMENTS, "items": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}, "mime": {"type": "string"}, "data_base64": {"type": "string"}},
                    "required": ["name", "data_base64"],
                }},
            }),
            &["account", "to", "subject", "body"],
        ),
        definition(
            "send_message",
            "Send a draft made by create_draft. Unless the user allowed agents to send for that account it is only queued for the user's approval.",
            json!({"draft_id": {"type": "string"}}),
            &["draft_id"],
        ),
    ]
}

fn list_arg(args: &Value, key: &str) -> Res<Vec<String>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(|s| s.trim().to_string()).ok_or_else(|| format!("{key} must be a list of strings")))
            .collect::<Res<Vec<_>>>()
            .map(|v| v.into_iter().filter(|s| !s.is_empty()).collect()),
        Some(Value::String(s)) => Ok(split_recipients(s)),
        _ => Err(format!("{key} must be a list of strings")),
    }
}

fn checked_recipient(raw: &str) -> Res<String> {
    if raw.chars().any(|c| c.is_control()) {
        return Err(format!("recipient {raw:?} has control characters"));
    }
    let c = Contact::parse(raw);
    let a = c.address.trim();
    let valid = a.contains('@') && !a.contains(char::is_whitespace) && !a.starts_with('@') && !a.ends_with('@') && !a.contains(['<', '>', ',', ';']);
    if !valid {
        return Err(format!("{raw} is not an email address"));
    }
    Ok(Contact { name: c.name, address: a.to_string() }.formatted())
}

fn addresses_of(list: &[String]) -> Vec<String> {
    list.iter().map(|r| Contact::parse(r).address).collect()
}

impl MailTools {
    pub(super) fn create_draft(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let index = self.resolve_account(prefs, args.get("account").and_then(Value::as_str).unwrap_or_default())?;
        let to = list_arg(args, "to")?.iter().map(|r| checked_recipient(r)).collect::<Res<Vec<_>>>()?;
        let cc = list_arg(args, "cc")?.iter().map(|r| checked_recipient(r)).collect::<Res<Vec<_>>>()?;
        let bcc = list_arg(args, "bcc")?.iter().map(|r| checked_recipient(r)).collect::<Res<Vec<_>>>()?;
        let count = to.len() + cc.len() + bcc.len();
        if count == 0 {
            return Err("at least one recipient is required".into());
        }
        if count > MAX_RECIPIENTS {
            return Err(format!("a message may have at most {MAX_RECIPIENTS} recipients, this has {count}"));
        }
        let subject = args.get("subject").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if subject.chars().any(|c| c.is_control()) {
            return Err("the subject has control characters".into());
        }
        let body = args.get("body").and_then(Value::as_str).ok_or("body is required")?.to_string();
        let attachments = self.draft_attachments(args)?;
        let (in_reply_to, references) = match args.get("reply_to_id").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            None => (None, Vec::new()),
            Some(id) => {
                let m = self.message(prefs, id)?;
                match m.message_id {
                    Some(mid) => {
                        let mut refs = m.references.clone();
                        if !refs.contains(&mid) {
                            refs.push(mid.clone());
                        }
                        (Some(mid), refs)
                    }
                    None => (None, Vec::new()),
                }
            }
        };
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let accounts = store.accounts();
        let signature = accounts.get(index).map(|a| a.signature.clone()).unwrap_or_default();
        let address = self.accounts[index].address.clone();
        let mid = crate::backend::new_message_id_for(&address).trim_matches(['<', '>']).to_string();
        let out = Outgoing {
            account: index,
            to: to.join(", "),
            cc: cc.join(", "),
            bcc: bcc.join(", "),
            subject: subject.clone(),
            body: format!("{}{}", body.trim_end(), crate::app::signature_block(&signature)),
            attachments,
            in_reply_to,
            references,
            message_id: Some(mid.clone()),
            agent: true,
        };
        let id = store.append(Folder::Drafts, out).map_err(store_error)?;
        drop(store);
        let all: Vec<String> = addresses_of(&to).into_iter().chain(addresses_of(&cc)).chain(addresses_of(&bcc)).collect();
        self.audit("create_draft", index, &all, &subject, &mid, "drafted");
        Ok(text_result(
            &format!("Draft {} created in the user's Drafts folder. Nothing was sent.", id_text(id)),
            json!({"draft_id": id_text(id), "account": self.accounts[index].id, "status": "drafted", "recipients": count}),
        ))
    }

    fn draft_attachments(&self, args: &Value) -> Res<Vec<OutAttachment>> {
        let Some(list) = args.get("attachments").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
        let list = list.as_array().ok_or("attachments must be a list")?;
        if list.len() > MAX_DRAFT_ATTACHMENTS {
            return Err(format!("at most {MAX_DRAFT_ATTACHMENTS} attachments"));
        }
        let mut out = Vec::new();
        let mut total = 0usize;
        for item in list {
            let name = sanitize::display(item.get("name").and_then(Value::as_str).ok_or("each attachment needs a name")?);
            let name = name.replace(['/', '\\'], "_");
            if name.is_empty() {
                return Err("an attachment name is empty".into());
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(item.get("data_base64").and_then(Value::as_str).ok_or("each attachment needs data_base64")?)
                .map_err(|_| "attachment data is not valid base64".to_string())?;
            total += data.len();
            if total > super::MAX_BODY {
                return Err("attachments are too large for one call".into());
            }
            let mime = item.get("mime").and_then(Value::as_str).filter(|m| !m.contains(char::is_control)).map(str::to_string).unwrap_or_else(|| crate::app::guess_mime(&name).to_string());
            out.push(OutAttachment { name, mime, data });
        }
        Ok(out)
    }

    fn audit(&self, tool: &str, index: usize, recipients: &[String], subject: &str, message_id: &str, result: &str) {
        let Some(dir) = &self.dir else { return };
        let account = self.accounts.get(index).map(|a| a.id.as_str()).unwrap_or_default();
        let _ = audit::write(dir, &audit::Entry { time: (self.clock)(), tool, account, recipients, subject, message_id, result });
    }

    fn sent_last_hour(&self, index: usize) -> usize {
        let now = (self.clock)();
        let mut memory = self.sends.lock().unwrap_or_else(|e| e.into_inner());
        memory.retain(|(_, at)| *at > now - 3600);
        let local = memory.iter().filter(|(i, _)| *i == index).count();
        let logged = self.dir.as_ref().map(|d| audit::sent_since(d, &self.accounts[index].id, now - 3600)).unwrap_or(0);
        local.max(logged)
    }

    pub(super) fn send_message(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let id_text_in = args.get("draft_id").and_then(Value::as_str).ok_or("draft_id is required")?;
        let m = self.message(prefs, id_text_in)?;
        let id = m.summary.id;
        if m.folder != Folder::Drafts || !crate::mailstore::is_agent_draft(&m.headers) {
            return Err("only drafts created with create_draft can be sent".into());
        }
        let index = m.summary.account;
        let account = &self.accounts[index];
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let out = store.open_draft(id).map_err(|_| "the draft could not be opened".to_string())?;
        let all: Vec<String> = [&out.to, &out.cc, &out.bcc].iter().flat_map(|l| split_recipients(l)).map(|r| Contact::parse(&r).address).collect();
        if all.len() > MAX_RECIPIENTS {
            return Err(format!("a message may have at most {MAX_RECIPIENTS} recipients"));
        }
        if all.is_empty() {
            return Err("the draft has no recipients".into());
        }
        let mid = out.message_id.clone().unwrap_or_default();
        if !prefs.may_send(&account.id, account.nostr) {
            drop(store);
            self.audit("send_message", index, &all, &out.subject, &mid, "queued");
            return Ok(text_result(
                "queued for the user's approval. The draft is in the user's Drafts folder; the user presses Ctrl+Enter to send it.",
                json!({"status": "queued", "draft_id": id_text(id), "account": account.id}),
            ));
        }
        drop(store);
        if self.sent_last_hour(index) >= SENDS_PER_HOUR {
            self.audit("send_message", index, &all, &out.subject, &mid, "refused: hourly cap");
            return Err(format!("hourly send cap of {SENDS_PER_HOUR} messages for this account reached; try again later"));
        }
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.set_send_html(!prefs.plain_text_only);
        let subject = out.subject.clone();
        let mut sending = out;
        sending.agent = false;
        let sent_id = store.append(Folder::Sent, sending).map_err(store_error)?;
        store.discard_draft(index, &mid);
        drop(store);
        self.sends.lock().unwrap_or_else(|e| e.into_inner()).push((index, (self.clock)()));
        self.audit("send_message", index, &all, &subject, &mid, "sent");
        Ok(text_result(
            &format!("Sent from {}. A copy is in Sent.", account.address),
            json!({"status": "sent", "message_id": mid, "sent_id": id_text(sent_id), "account": account.id}),
        ))
    }
}
