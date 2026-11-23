use super::error::{BackendError, Result};
use super::types::*;
use super::Backend;
use async_trait::async_trait;
use lettre::message::header::{InReplyTo, MessageId, References};
use lettre::message::{Attachment as MimeAttachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::time::Duration;

pub use super::types::Security as SmtpSecurity;

#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub auth: Auth,
    pub accept_invalid_certs: bool,
    pub timeout: Duration,
}

impl SmtpConfig {
    pub fn new(host: &str, port: u16, security: Security, auth: Auth) -> Self {
        Self {
            host: host.to_string(),
            port,
            security,
            auth,
            accept_invalid_certs: false,
            timeout: Duration::from_secs(30),
        }
    }
}

pub struct SmtpBackend {
    cfg: SmtpConfig,
}

impl SmtpBackend {
    pub fn new(cfg: SmtpConfig) -> Self {
        Self { cfg }
    }

    fn transport(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
        let cfg = &self.cfg;
        let tls = || -> Result<TlsParameters> {
            TlsParameters::builder(cfg.host.clone())
                .dangerous_accept_invalid_certs(cfg.accept_invalid_certs)
                .build()
                .map_err(|e| BackendError::Connection(e.to_string()))
        };
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(cfg.host.as_str())
            .port(cfg.port)
            .timeout(Some(cfg.timeout));
        builder = match cfg.security {
            Security::Tls => builder.tls(Tls::Wrapper(tls()?)),
            Security::StartTls => builder.tls(Tls::Required(tls()?)),
            Security::Plain => builder.tls(Tls::None),
        };
        builder = match &cfg.auth {
            Auth::Password { user, password } => builder
                .credentials(Credentials::new(user.clone(), password.clone()))
                .authentication(vec![Mechanism::Plain, Mechanism::Login]),
            Auth::XOAuth2 { user, access_token } => builder
                .credentials(Credentials::new(user.clone(), access_token.clone()))
                .authentication(vec![Mechanism::Xoauth2]),
        };
        Ok(builder.build())
    }
}

fn mailbox(a: &Address) -> Result<Mailbox> {
    let addr = a
        .address
        .parse()
        .map_err(|e| BackendError::Protocol(format!("bad address {}: {e}", a.address)))?;
    Ok(Mailbox::new(a.name.clone(), addr))
}

pub fn build_message(m: &OutgoingMessage) -> Result<Message> {
    build(m, false)
}

pub fn render_message(m: &OutgoingMessage, keep_bcc: bool) -> Result<Vec<u8>> {
    Ok(build(m, keep_bcc)?.formatted())
}

#[derive(Clone)]
struct AgentMarker;

impl lettre::message::header::Header for AgentMarker {
    fn name() -> lettre::message::header::HeaderName {
        lettre::message::header::HeaderName::new_from_ascii_str("X-Mailbench-Agent")
    }

    fn parse(_: &str) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(AgentMarker)
    }

    fn display(&self) -> lettre::message::header::HeaderValue {
        lettre::message::header::HeaderValue::new(Self::name(), "1".to_string())
    }
}

fn build(m: &OutgoingMessage, keep_bcc: bool) -> Result<Message> {
    let mut b: MessageBuilder = Message::builder().from(mailbox(&m.from)?);
    for a in &m.to {
        b = b.to(mailbox(a)?);
    }
    for a in &m.cc {
        b = b.cc(mailbox(a)?);
    }
    for a in &m.bcc {
        b = b.bcc(mailbox(a)?);
    }
    if keep_bcc {
        b = b.keep_bcc();
    }
    let id = m.message_id.clone().unwrap_or_else(|| new_message_id(&m.from.address));
    b = b.header(MessageId::from(bracket(&id)));
    if let Some(id) = &m.in_reply_to {
        b = b.header(InReplyTo::from(bracket(id)));
    }
    if !m.references.is_empty() {
        b = b.header(References::from(m.references.iter().map(|r| bracket(r)).collect::<Vec<_>>().join(" ")));
    }
    let mut b = b.subject(m.subject.clone());
    if m.agent {
        b = b.header(AgentMarker);
    }
    if m.to.is_empty() && m.cc.is_empty() && m.bcc.is_empty() {
        let from: lettre::Address = m
            .from
            .address
            .parse()
            .map_err(|e| BackendError::Protocol(format!("bad address {}: {e}", m.from.address)))?;
        b = b.envelope(lettre::address::Envelope::new(Some(from.clone()), vec![from]).map_err(|e| BackendError::Protocol(e.to_string()))?);
    }
    let body_part = body_part(m);
    let built = if m.attachments.is_empty() {
        match body_part {
            BodyPart::Single(s) => b.singlepart(s),
            BodyPart::Alternative(a) => b.multipart(a),
        }
    } else {
        let mut mixed = match body_part {
            BodyPart::Single(s) => MultiPart::mixed().singlepart(s),
            BodyPart::Alternative(a) => MultiPart::mixed().multipart(a),
        };
        for a in &m.attachments {
            let mime = ContentType::parse(&a.mime).unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
            mixed = mixed.singlepart(MimeAttachment::new(a.name.clone()).body(a.data.clone(), mime));
        }
        b.multipart(mixed)
    };
    built.map_err(|e| BackendError::Protocol(e.to_string()))
}

enum BodyPart {
    Single(SinglePart),
    Alternative(MultiPart),
}

fn body_part(m: &OutgoingMessage) -> BodyPart {
    let plain = plain_part(&m.body);
    if !m.html_alternative {
        return BodyPart::Single(plain);
    }
    let (html, marked) = crate::richtext::render(&m.body);
    if !marked {
        return BodyPart::Single(plain);
    }
    BodyPart::Alternative(MultiPart::alternative().singlepart(plain).singlepart(SinglePart::html(html)))
}

fn plain_part(body: &str) -> SinglePart {
    let kind = ContentType::parse("text/plain; charset=utf-8; format=flowed").expect("valid");
    SinglePart::builder().header(kind).body(crate::flowed::flow(body))
}

pub fn bracket(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') { id.to_string() } else { format!("<{id}>") }
}

pub fn new_message_id_for(from: &str) -> String {
    new_message_id(from)
}

fn new_message_id(from: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let domain = from.rsplit('@').next().unwrap_or("localhost");
    format!(
        "<{nanos:x}.{:x}.{:x}@{domain}>",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn smtp_error(e: lettre::transport::smtp::Error) -> BackendError {
    let text = e.to_string();
    if e.is_permanent() && (text.contains("uthentication") || text.contains("535") || text.contains("534")) {
        BackendError::Auth(text)
    } else if e.is_permanent() || e.is_transient() {
        BackendError::Protocol(text)
    } else {
        BackendError::Connection(text)
    }
}

#[async_trait]
impl Backend for SmtpBackend {
    fn kind(&self) -> &'static str {
        "smtp"
    }

    fn fork(&self) -> Option<Box<dyn Backend>> {
        Some(Box::new(SmtpBackend::new(self.cfg.clone())))
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities {
            send: true,
            ..Default::default()
        })
    }

    async fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
            return Err(BackendError::Protocol("no recipients".into()));
        }
        let email = build_message(message)?;
        self.transport()?.send(email).await.map_err(smtp_error)?;
        Ok(())
    }

    async fn check(&mut self) -> Result<()> {
        match self.transport()?.test_connection().await {
            Ok(true) => Ok(()),
            Ok(false) => Err(BackendError::Connection("the server closed the connection".into())),
            Err(e) => Err(smtp_error(e)),
        }
    }
}
