use mailbench::autodetect::*;
use mailbench::backend::Security;
use mailbench::dns::{self, Srv};
use std::collections::HashMap;

#[derive(Default)]
struct Fixture {
    pages: HashMap<String, String>,
    srv: HashMap<String, Vec<Srv>>,
    mx: HashMap<String, Vec<String>>,
    asked: std::cell::RefCell<Vec<String>>,
}

impl Net for Fixture {
    fn get(&self, url: &str) -> Option<String> {
        self.asked.borrow_mut().push(url.to_string());
        self.pages.iter().find(|(k, _)| url.starts_with(k.as_str())).map(|(_, v)| v.clone())
    }
    fn srv(&self, name: &str) -> Vec<Srv> {
        self.srv.get(name).cloned().unwrap_or_default()
    }
    fn mx(&self, domain: &str) -> Vec<String> {
        self.mx.get(domain).cloned().unwrap_or_default()
    }
}

const AUTOCONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <domain>example.org</domain>
    <incomingServer type="pop3">
      <hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.%EMAILDOMAIN%</hostname><port>993</port><socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username><authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

const ISPDB: &str = r#"<clientConfig version="1.1">
  <emailProvider id="isp.test">
    <incomingServer type="imap">
      <hostname>mail.isp.test</hostname><port>143</port><socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>mail.isp.test</hostname><port>465</port><socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

fn srv(target: &str, port: u16) -> Vec<Srv> {
    vec![Srv { priority: 0, weight: 1, port, target: target.into() }]
}

#[test]
fn autoconfig_xml_gives_imap_and_smtp_with_substituted_user() {
    let mut net = Fixture::default();
    net.pages.insert("https://autoconfig.example.org/mail/config-v1.1.xml".into(), AUTOCONFIG.into());
    let found = detect("mara@example.org", &net).unwrap();
    assert_eq!(found.imap, ServerSettings::new("imap.example.org", 993, Security::Tls));
    assert_eq!(found.smtp, ServerSettings::new("smtp.example.org", 587, Security::StartTls));
    assert_eq!(found.username, "mara");
    assert!(matches!(found.source, Source::Autoconfig(_)));
}

#[test]
fn well_known_autoconfig_is_tried_second() {
    let mut net = Fixture::default();
    net.pages.insert("https://example.org/.well-known/autoconfig/mail/config-v1.1.xml".into(), AUTOCONFIG.into());
    let found = detect("mara@example.org", &net).unwrap();
    assert_eq!(found.imap.host, "imap.example.org");
    let asked = net.asked.borrow();
    assert!(asked[0].starts_with("https://autoconfig.example.org/mail/config-v1.1.xml"));
    assert!(asked[1].starts_with("https://example.org/.well-known/autoconfig/"));
}

#[test]
fn ispdb_is_used_when_the_provider_has_no_autoconfig() {
    let mut net = Fixture::default();
    net.pages.insert("https://autoconfig.thunderbird.net/v1.1/isp.test".into(), ISPDB.into());
    let found = detect("me@isp.test", &net).unwrap();
    assert_eq!(found.imap, ServerSettings::new("mail.isp.test", 143, Security::StartTls));
    assert_eq!(found.smtp, ServerSettings::new("mail.isp.test", 465, Security::Tls));
    assert_eq!(found.username, "me@isp.test");
    assert!(matches!(found.source, Source::Ispdb(_)));
}

#[test]
fn srv_records_are_used_after_the_web_sources_fail() {
    let mut net = Fixture::default();
    net.srv.insert("_imaps._tcp.srv.test".into(), srv("imap.srv.test", 993));
    net.srv.insert("_submission._tcp.srv.test".into(), srv("smtp.srv.test", 587));
    let found = detect("a@srv.test", &net).unwrap();
    assert_eq!(found.imap, ServerSettings::new("imap.srv.test", 993, Security::Tls));
    assert_eq!(found.smtp, ServerSettings::new("smtp.srv.test", 587, Security::StartTls));
    assert_eq!(found.source, Source::Srv);
}

#[test]
fn mx_pointing_at_google_or_microsoft_guesses_the_hosted_settings() {
    let mut net = Fixture::default();
    net.mx.insert("corp.test".into(), vec!["aspmx.l.google.com.".into()]);
    let g = detect("a@corp.test", &net).unwrap();
    assert_eq!(g.imap.host, "imap.gmail.com");
    assert_eq!(g.provider, Some(Provider::Google));
    net.mx.insert("firm.test".into(), vec!["firm-test.mail.protection.outlook.com.".into()]);
    let m = detect("a@firm.test", &net).unwrap();
    assert_eq!(m.imap.host, "outlook.office365.com");
    assert_eq!(m.smtp, ServerSettings::new("smtp.office365.com", 587, Security::StartTls));
    assert_eq!(m.provider, Some(Provider::Microsoft));
}

#[test]
fn nothing_found_is_none_and_bad_addresses_are_refused() {
    let net = Fixture::default();
    assert!(detect("a@nowhere.test", &net).is_none());
    assert!(detect("not an address", &net).is_none());
    assert!(detect("@x.test", &net).is_none());
}

fn name_wire(name: &str) -> Vec<u8> {
    dns::encode_name(name)
}

fn response(qname: &str, qtype: u16, answers: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut m = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, answers.len() as u8, 0, 0, 0, 0];
    m.extend(name_wire(qname));
    m.extend(qtype.to_be_bytes());
    m.extend(1u16.to_be_bytes());
    for (t, rdata) in answers {
        m.extend([0xC0, 12]);
        m.extend(t.to_be_bytes());
        m.extend(1u16.to_be_bytes());
        m.extend(60u32.to_be_bytes());
        m.extend((rdata.len() as u16).to_be_bytes());
        m.extend(rdata);
    }
    m
}

#[test]
fn dns_srv_response_fixture_parses_and_orders_by_priority() {
    let rec = |prio: u16, weight: u16, port: u16, target: &str| {
        let mut r = Vec::new();
        r.extend(prio.to_be_bytes());
        r.extend(weight.to_be_bytes());
        r.extend(port.to_be_bytes());
        r.extend(name_wire(target));
        (dns::TYPE_SRV, r)
    };
    let msg = response(
        "_imaps._tcp.example.org",
        dns::TYPE_SRV,
        &[rec(20, 1, 993, "backup.example.org"), rec(10, 5, 993, "imap.example.org")],
    );
    let found = dns::parse_srv(&msg);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0], Srv { priority: 10, weight: 5, port: 993, target: "imap.example.org".into() });
    assert_eq!(found[1].target, "backup.example.org");
}

#[test]
fn dns_mx_response_with_name_compression_parses() {
    let mut rdata = 10u16.to_be_bytes().to_vec();
    rdata.extend([4, b'm', b'a', b'i', b'l', 0xC0, 12]);
    let msg = response("example.org", dns::TYPE_MX, &[(dns::TYPE_MX, rdata)]);
    let mx = dns::parse_mx(&msg);
    assert_eq!(mx.len(), 1);
    assert_eq!(mx[0].preference, 10);
    assert_eq!(mx[0].exchange, "mail.example.org");
}

#[test]
fn dns_error_and_truncated_answers_give_nothing() {
    let mut msg = response("x.test", dns::TYPE_SRV, &[]);
    msg[3] = 0x83;
    assert!(dns::parse_srv(&msg).is_empty());
    assert!(dns::parse_srv(&[1, 2, 3]).is_empty());
}

#[test]
fn query_packet_is_a_valid_question() {
    let q = dns::encode_query(7, "_imaps._tcp.example.org", dns::TYPE_SRV);
    assert_eq!(&q[..2], &[0, 7]);
    assert_eq!(q[4..6], [0, 1]);
    assert_eq!(&q[12..13], &[6]);
}
