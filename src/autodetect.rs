use crate::backend::Security;
use crate::dns::{self, Srv};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

impl ServerSettings {
    pub fn new(host: &str, port: u16, security: Security) -> Self {
        Self { host: host.to_string(), port, security }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Google,
    Microsoft,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::Microsoft => "Microsoft",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Autoconfig(String),
    Ispdb(String),
    Srv,
    Mx,
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Autoconfig(url) => format!("provider autoconfig ({})", url.split('?').next().unwrap_or(url)),
            Source::Ispdb(_) => "Mozilla ISP database".to_string(),
            Source::Srv => "DNS SRV records".to_string(),
            Source::Mx => "mail exchanger lookup".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub imap: ServerSettings,
    pub smtp: ServerSettings,
    pub username: String,
    pub provider: Option<Provider>,
    pub source: Source,
}

pub trait Net {
    fn get(&self, url: &str) -> Option<String>;
    fn srv(&self, name: &str) -> Vec<Srv>;
    fn mx(&self, domain: &str) -> Vec<String>;
}

pub fn split_address(email: &str) -> Option<(&str, &str)> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains(char::is_whitespace) {
        return None;
    }
    Some((local, domain))
}

pub fn provider_of_host(host: &str) -> Option<Provider> {
    let host = host.to_lowercase();
    let under = |suffix: &str| host == suffix || host.ends_with(&format!(".{suffix}"));
    if under("gmail.com") || under("google.com") || under("googlemail.com") {
        Some(Provider::Google)
    } else if under("outlook.com") || under("office365.com") || under("outlook.office365.com") || under("hotmail.com") || under("protection.outlook.com") {
        Some(Provider::Microsoft)
    } else {
        None
    }
}

fn substitute(text: &str, email: &str) -> String {
    let (local, domain) = split_address(email).unwrap_or((email, ""));
    text.replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
        .trim()
        .to_string()
}

fn security_of(text: &str) -> Option<Security> {
    match text.trim().to_uppercase().as_str() {
        "SSL" | "TLS" => Some(Security::Tls),
        "STARTTLS" => Some(Security::StartTls),
        "PLAIN" | "NONE" => Some(Security::Plain),
        _ => None,
    }
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children().find(|n| n.has_tag_name(name)).and_then(|n| n.text()).map(|t| t.trim().to_string())
}

struct ParsedServer {
    settings: ServerSettings,
    username: String,
}

fn server_of(node: roxmltree::Node<'_, '_>, email: &str) -> Option<ParsedServer> {
    let host = substitute(&child_text(node, "hostname")?, email);
    let port = child_text(node, "port")?.parse::<u16>().ok()?;
    let security = security_of(&child_text(node, "socketType")?)?;
    let username = substitute(&child_text(node, "username").unwrap_or_else(|| "%EMAILADDRESS%".into()), email);
    if host.is_empty() {
        return None;
    }
    Some(ParsedServer { settings: ServerSettings { host, port, security }, username })
}

pub fn parse_autoconfig(xml: &str, email: &str, source: Source) -> Option<Detected> {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    )
    .ok()?;
    let provider = doc.descendants().find(|n| n.has_tag_name("emailProvider"))?;
    let of_type = |tag: &str, kind: &str| {
        provider
            .children()
            .filter(|n| n.has_tag_name(tag) && n.attribute("type") == Some(kind))
            .find_map(|n| server_of(n, email))
    };
    let imap = of_type("incomingServer", "imap")?;
    let smtp = of_type("outgoingServer", "smtp")?;
    let known = provider_of_host(&imap.settings.host);
    Some(Detected { username: imap.username, imap: imap.settings, smtp: smtp.settings, provider: known, source })
}

pub fn imap_from_srv(tls: &[Srv], starttls: &[Srv]) -> Option<ServerSettings> {
    let usable = |list: &[Srv]| list.iter().find(|s| s.target != "." && !s.target.is_empty()).cloned();
    if let Some(s) = usable(tls) {
        return Some(ServerSettings::new(&s.target, s.port, Security::Tls));
    }
    usable(starttls).map(|s| {
        let security = if s.port == 993 { Security::Tls } else { Security::StartTls };
        ServerSettings::new(&s.target, s.port, security)
    })
}

pub fn smtp_from_srv(tls: &[Srv], starttls: &[Srv]) -> Option<ServerSettings> {
    let usable = |list: &[Srv]| list.iter().find(|s| s.target != "." && !s.target.is_empty()).cloned();
    if let Some(s) = usable(tls) {
        return Some(ServerSettings::new(&s.target, s.port, Security::Tls));
    }
    usable(starttls).map(|s| {
        let security = if s.port == 465 { Security::Tls } else { Security::StartTls };
        ServerSettings::new(&s.target, s.port, security)
    })
}

pub fn provider_guess(provider: Provider, email: &str) -> Detected {
    let (imap, smtp) = match provider {
        Provider::Google => (
            ServerSettings::new("imap.gmail.com", 993, Security::Tls),
            ServerSettings::new("smtp.gmail.com", 465, Security::Tls),
        ),
        Provider::Microsoft => (
            ServerSettings::new("outlook.office365.com", 993, Security::Tls),
            ServerSettings::new("smtp.office365.com", 587, Security::StartTls),
        ),
    };
    Detected { imap, smtp, username: email.to_string(), provider: Some(provider), source: Source::Mx }
}

pub fn detect(email: &str, net: &dyn Net) -> Option<Detected> {
    let email = email.trim();
    let (_, domain) = split_address(email)?;
    let domain = domain.to_lowercase();
    let urls = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={email}"),
    ];
    for url in urls {
        if let Some(found) = net.get(&url).and_then(|xml| parse_autoconfig(&xml, email, Source::Autoconfig(url.clone()))) {
            return Some(found);
        }
    }
    let ispdb = format!("https://autoconfig.thunderbird.net/v1.1/{domain}");
    if let Some(found) = net.get(&ispdb).and_then(|xml| parse_autoconfig(&xml, email, Source::Ispdb(ispdb.clone()))) {
        return Some(found);
    }
    let imap = imap_from_srv(
        &net.srv(&format!("_imaps._tcp.{domain}")),
        &net.srv(&format!("_imap._tcp.{domain}")),
    );
    if let Some(imap) = imap {
        let smtp = smtp_from_srv(
            &net.srv(&format!("_submissions._tcp.{domain}")),
            &net.srv(&format!("_submission._tcp.{domain}")),
        )
        .unwrap_or_else(|| ServerSettings::new(&format!("smtp.{domain}"), 587, Security::StartTls));
        let provider = provider_of_host(&imap.host);
        return Some(Detected { imap, smtp, username: email.to_string(), provider, source: Source::Srv });
    }
    let hosted = net.mx(&domain).iter().find_map(|h| provider_of_host(h.trim_end_matches('.')));
    hosted.or_else(|| provider_of_host(&domain)).map(|p| provider_guess(p, email))
}

pub struct Live;

impl Net for Live {
    fn get(&self, url: &str) -> Option<String> {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(6)).build();
        let body = agent.get(url).call().ok()?.into_string().ok()?;
        Some(body)
    }

    fn srv(&self, name: &str) -> Vec<Srv> {
        dns::query(&dns::nameservers(), name, dns::TYPE_SRV).map(|m| dns::parse_srv(&m)).unwrap_or_default()
    }

    fn mx(&self, domain: &str) -> Vec<String> {
        dns::query(&dns::nameservers(), domain, dns::TYPE_MX)
            .map(|m| dns::parse_mx(&m).into_iter().map(|x| x.exchange).collect())
            .unwrap_or_default()
    }
}
