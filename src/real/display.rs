use crate::backend::Address;
use crate::htmlview::HtmlBody;
use crate::mailstore::{AttachmentData, AttachmentInfo};
use mail_parser::{MessageParser, PartKind};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct Body {
    pub paragraphs: Vec<String>,
    pub html: HtmlBody,
    pub quoted: Vec<String>,
    pub attachments: Vec<AttachmentInfo>,
    pub remote_images: Option<String>,
}

pub fn person(a: &Address) -> String {
    match a.name.as_deref() {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => a.address.clone(),
    }
}

pub fn people(list: &[Address]) -> Vec<String> {
    list.iter().map(person).collect()
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn weekday(days: i64) -> &'static str {
    DAYS[(days + 4).rem_euclid(7) as usize]
}

pub fn when(ts: i64, now: i64) -> String {
    let (day, now_day) = (ts.div_euclid(86_400), now.div_euclid(86_400));
    let secs = ts.rem_euclid(86_400);
    if day == now_day {
        format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
    } else if day < now_day && now_day - day < 7 {
        weekday(day).to_string()
    } else {
        let (_, m, d) = civil(day);
        format!("{d} {}", MONTHS[m as usize - 1])
    }
}

pub fn wake(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) => format!("wakes {}", t.format("%a %-d %b %H:%M")),
        None => "snoozed".into(),
    }
}

pub fn sent(ts: i64) -> String {
    let day = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (_, m, d) = civil(day);
    format!("{} {d} {} {:02}:{:02}", weekday(day), MONTHS[m as usize - 1], secs / 3600, secs % 3600 / 60)
}

pub fn size(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1000 {
        format!("{} KB", bytes / 1000)
    } else {
        format!("{bytes} B")
    }
}

#[derive(Clone, Debug, Default)]
pub struct Envelope {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub to: Vec<crate::mailstore::Contact>,
    pub cc: Vec<crate::mailstore::Contact>,
}

fn contacts(list: Option<mail_parser::AddressList<'_>>) -> Vec<crate::mailstore::Contact> {
    list.map(|l| {
        l.mailboxes()
            .filter_map(|m| {
                Some(crate::mailstore::Contact { name: m.name().unwrap_or_default().to_string(), address: m.address()?.to_string() })
            })
            .collect()
    })
    .unwrap_or_default()
}

pub fn envelope(raw: &[u8]) -> Envelope {
    let Some(msg) = MessageParser::default().parse(raw) else { return Envelope::default() };
    let texts = |l: Option<mail_parser::TextList<'_>>| -> Vec<String> {
        l.map(|l| l.iter().map(str::to_string).collect()).unwrap_or_default()
    };
    let mut references = texts(msg.references());
    if references.is_empty() {
        references = texts(msg.in_reply_to());
    }
    Envelope {
        message_id: msg.message_id().map(str::to_string),
        in_reply_to: texts(msg.in_reply_to()).into_iter().next(),
        references,
        to: contacts(msg.to()),
        cc: contacts(msg.cc()),
    }
}

pub fn plain_text(raw: &[u8]) -> String {
    MessageParser::default()
        .parse(raw)
        .map(|m| m.text_bodies().map(|t| t.into_owned()).collect::<Vec<_>>().join("\n\n"))
        .unwrap_or_default()
}

pub fn has_attachments(raw: &[u8]) -> bool {
    MessageParser::default().parse(raw).is_some_and(|m| m.attachments().any(|p| !is_inline_image(&p, &html_of(&m))))
}

fn html_of(msg: &mail_parser::Message) -> String {
    msg.html_body().filter(|p| matches!(p.kind(), PartKind::Html)).find_map(|p| p.text()).map(|h| h.into_owned()).unwrap_or_default()
}

fn content_id(p: &mail_parser::MessagePart) -> Option<String> {
    p.content_id().map(|c| c.trim().trim_matches(['<', '>']).to_string()).filter(|c| !c.is_empty())
}

fn is_inline_image(p: &mail_parser::MessagePart, html: &str) -> bool {
    let Some(cid) = content_id(p) else { return false };
    html.to_ascii_lowercase().contains(&format!("cid:{}", cid.to_ascii_lowercase()))
}

fn image_host(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("<img") {
        rest = &rest[i + 4..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let Some(s) = tag.find("src=") else { continue };
        let value = tag[s + 4..].trim_start_matches(['"', '\'']);
        let Some(url) = value.strip_prefix("http://").or_else(|| value.strip_prefix("https://")) else {
            continue;
        };
        let host: String = url.chars().take_while(|c| !matches!(c, '/' | '"' | '\'' | ' ' | '?' | ':')).collect();
        if !host.is_empty() {
            return Some(host);
        }
    }
    None
}

fn split_text(text: &str) -> (Vec<String>, Vec<String>) {
    let mut paragraphs = Vec::new();
    let mut quoted = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let flush = |current: &mut Vec<&str>, out: &mut Vec<String>| {
        if !current.is_empty() {
            out.push(current.join("\n"));
            current.clear();
        }
    };
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim_start().starts_with('>') {
            flush(&mut current, &mut paragraphs);
            quoted.push(line.to_string());
        } else if line.trim().is_empty() {
            flush(&mut current, &mut paragraphs);
        } else {
            current.push(line);
        }
    }
    flush(&mut current, &mut paragraphs);
    (paragraphs, quoted)
}

fn mime_of(p: &mail_parser::MessagePart) -> String {
    p.content_type()
        .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
        .unwrap_or_else(|| "application/octet-stream".into())
}

pub fn extract_attachments(raw: &[u8]) -> Vec<AttachmentData> {
    let Some(msg) = MessageParser::default().parse(raw) else { return Vec::new() };
    let html = html_of(&msg);
    msg.attachments()
        .filter(|p| !is_inline_image(p, &html))
        .map(|p| AttachmentData {
            name: p.attachment_name().unwrap_or("attachment").to_string(),
            mime: mime_of(&p),
            data: Arc::new(p.decoded().into_owned()),
        })
        .collect()
}

pub fn parse_body(raw: &[u8]) -> Body {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Body::default();
    };
    let text = msg.text_bodies().map(|t| t.into_owned()).collect::<Vec<_>>().join("\n\n");
    let (paragraphs, quoted) = split_text(&text);
    let html = html_of(&msg);
    let mut inline = Vec::new();
    let mut attachments = Vec::new();
    for p in msg.attachments() {
        if is_inline_image(&p, &html) {
            if let Some(cid) = content_id(&p) {
                inline.push((cid, Arc::new(p.decoded().into_owned())));
            }
        } else {
            attachments.push(AttachmentInfo {
                name: p.attachment_name().unwrap_or("attachment").to_string(),
                size: size(p.decoded_len()),
                mime: mime_of(&p),
            });
        }
    }
    let remote_images = image_host(&html);
    Body { paragraphs, html: HtmlBody { html, inline }, quoted, attachments, remote_images }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_220_000;

    #[test]
    fn when_is_time_today_weekday_this_week_and_date_after() {
        assert_eq!(when(NOW - 3600, NOW), "10:20");
        assert_eq!(when(NOW - 86_400 * 2, NOW), "Wed");
        assert_eq!(when(NOW - 86_400 * 30, NOW), "18 Oct");
    }

    #[test]
    fn sent_has_weekday_date_and_time() {
        assert_eq!(sent(1_700_220_000), "Fri 17 Nov 11:20");
    }

    #[test]
    fn quoted_lines_fold_out_of_the_paragraphs() {
        let raw = b"From: a@x.test\r\nSubject: s\r\n\r\nHello there\r\nsecond line\r\n\r\n> old one\r\n> old two\r\n\r\nBye\r\n";
        let b = parse_body(raw);
        assert_eq!(b.paragraphs, vec!["Hello there\nsecond line", "Bye"]);
        assert_eq!(b.quoted, vec!["> old one", "> old two"]);
    }

    #[test]
    fn html_is_kept_for_rendering_and_remote_images_are_reported_not_loaded() {
        let raw = b"From: a@x.test\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n<html><body><p>Hi <b>friend</b></p><img src=\"https://track.example.com/p.gif\"></body></html>\r\n";
        let b = parse_body(raw);
        assert!(b.html.html.contains("<b>friend</b>"));
        assert!(b.paragraphs.join(" ").contains("Hi"));
        assert_eq!(b.remote_images.as_deref(), Some("track.example.com"));
    }

    #[test]
    fn inline_cid_images_are_not_listed_as_attachments() {
        let raw = b"From: a@x.test\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>We're delivering</p><img src=\"cid:dpdlogo\">\r\n--b\r\nContent-Type: image/png\r\nContent-ID: <dpdlogo>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--b\r\nContent-Type: application/pdf; name=\"q.pdf\"\r\nContent-Disposition: attachment; filename=\"q.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--b--\r\n";
        let b = parse_body(raw);
        assert_eq!(b.attachments.len(), 1);
        assert_eq!(b.attachments[0].name, "q.pdf");
        assert_eq!(b.html.inline.len(), 1);
        assert_eq!(b.html.inline[0].0, "dpdlogo");
        assert_eq!(b.html.inline[0].1.as_slice(), b"Hello");
        assert!(has_attachments(raw));
    }

    #[test]
    fn a_mail_whose_only_parts_are_inline_images_has_no_attachments() {
        let raw = b"From: a@x.test\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:a\">\r\n--b\r\nContent-Type: image/png\r\nContent-ID: <a>\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--b--\r\n";
        assert!(!has_attachments(raw));
        assert!(parse_body(raw).attachments.is_empty());
    }

    #[test]
    fn attachments_are_listed_with_sizes() {
        let raw = b"From: a@x.test\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nsee file\r\n--b\r\nContent-Type: application/pdf; name=\"q.pdf\"\r\nContent-Disposition: attachment; filename=\"q.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--b--\r\n";
        let b = parse_body(raw);
        assert_eq!(b.attachments.len(), 1);
        assert_eq!(b.attachments[0].name, "q.pdf");
        assert_eq!(b.attachments[0].size, "5 B");
        assert!(has_attachments(raw));
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    fn mail(part_headers: &str, body: &str) -> Vec<u8> {
        format!(
            "From: a@x.test\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\n{part_headers}\r\n\r\n{body}\r\n--b--\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn base64_part_decodes_to_the_original_bytes() {
        let raw = mail("Content-Type: application/pdf; name=\"q.pdf\"\r\nContent-Disposition: attachment; filename=\"q.pdf\"\r\nContent-Transfer-Encoding: base64", "SGVsbG8gd29ybGQ=");
        let a = extract_attachments(&raw);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "q.pdf");
        assert_eq!(a[0].mime, "application/pdf");
        assert_eq!(a[0].data.as_slice(), b"Hello world");
    }

    #[test]
    fn quoted_printable_part_decodes_to_the_original_bytes() {
        let raw = mail("Content-Type: text/csv; name=\"d.csv\"\r\nContent-Disposition: attachment; filename=\"d.csv\"\r\nContent-Transfer-Encoding: quoted-printable", "caf=C3=A9,1=\r\n23");
        let a = extract_attachments(&raw);
        assert_eq!(a[0].data.as_slice(), "café,123".as_bytes());
    }

    #[test]
    fn rfc2231_filename_is_decoded() {
        let raw = mail("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*=UTF-8''r%C3%A9sum%C3%A9%20v2.txt\r\nContent-Transfer-Encoding: 7bit", "plain");
        let a = extract_attachments(&raw);
        assert_eq!(a[0].name, "résumé v2.txt");
        assert_eq!(a[0].data.as_slice(), b"plain");
        assert_eq!(parse_body(&raw).attachments[0].name, "résumé v2.txt");
    }

    #[test]
    fn rfc2231_continuations_are_joined() {
        let raw = mail("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*0*=UTF-8''long%20na; filename*1*=me.bin\r\nContent-Transfer-Encoding: 7bit", "x");
        assert_eq!(extract_attachments(&raw)[0].name, "long name.bin");
    }
}
