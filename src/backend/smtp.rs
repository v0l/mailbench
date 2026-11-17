use super::error::{BackendError, Result};
use super::types::*;
use super::Backend;
use lettre::message::header::{InReplyTo, MessageId, References};
use lettre::message::{Mailbox, MessageBuilder};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
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

    fn transport(&self) -> Result<SmtpTransport> {
        let cfg = &self.cfg;
        let tls = || -> Result<TlsParameters> {
            TlsParameters::builder(cfg.host.clone())
                .dangerous_accept_invalid_certs(cfg.accept_invalid_certs)
                .build()
                .map_err(|e| BackendError::Connection(e.to_string()))
        };
        let mut builder = SmtpTransport::builder_dangerous(cfg.host.as_str())
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

pub(crate) fn build_message(m: &OutgoingMessage) -> Result<Message> {
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
    b = b.header(MessageId::from(new_message_id(&m.from.address)));
    if let Some(id) = &m.in_reply_to {
        b = b.header(InReplyTo::from(id.clone()));
    }
    if !m.references.is_empty() {
        b = b.header(References::from(m.references.join(" ")));
    }
    b.subject(m.subject.clone())
        .body(m.body.clone())
        .map_err(|e| BackendError::Protocol(e.to_string()))
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

impl Backend for SmtpBackend {
    fn kind(&self) -> &'static str {
        "smtp"
    }

    fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities {
            send: true,
            ..Default::default()
        })
    }

    fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
            return Err(BackendError::Protocol("no recipients".into()));
        }
        let email = build_message(message)?;
        self.transport()?.send(&email).map_err(|e| {
            let text = e.to_string();
            if e.is_permanent() && text.contains("uthentication") {
                BackendError::Auth(text)
            } else if e.is_permanent() || e.is_transient() {
                BackendError::Protocol(text)
            } else {
                BackendError::Connection(text)
            }
        })?;
        Ok(())
    }
}
