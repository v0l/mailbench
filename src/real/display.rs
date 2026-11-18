use crate::backend::Address;
use crate::mailstore::{AttachmentInfo, Block};
use mail_parser::{MessageParser, PartKind};

#[derive(Clone, Debug, Default)]
pub struct Body {
    pub paragraphs: Vec<String>,
    pub blocks: Vec<Block>,
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

pub fn has_attachments(raw: &[u8]) -> bool {
    MessageParser::default().parse(raw).is_some_and(|m| m.attachments().len() > 0)
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

pub fn parse_body(raw: &[u8]) -> Body {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Body::default();
    };
    let text = msg.text_bodies().map(|t| t.into_owned()).collect::<Vec<_>>().join("\n\n");
    let (paragraphs, quoted) = split_text(&text);
    let attachments = msg
        .attachments()
        .map(|p| AttachmentInfo {
            name: p.attachment_name().unwrap_or("attachment").to_string(),
            size: size(p.decoded_len()),
        })
        .collect();
    let remote_images = msg
        .html_body()
        .filter(|p| matches!(p.kind(), PartKind::Html))
        .filter_map(|p| p.text())
        .find_map(|h| image_host(&h));
    let blocks = msg
        .html_body()
        .filter(|p| matches!(p.kind(), PartKind::Html))
        .find_map(|p| p.text())
        .map(|h| super::html::render(&h))
        .unwrap_or_default();
    Body { paragraphs, blocks, quoted, attachments, remote_images }
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
    fn html_becomes_text_and_remote_images_are_reported_not_loaded() {
        let raw = b"From: a@x.test\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n<html><body><p>Hi <b>friend</b></p><img src=\"https://track.example.com/p.gif\"></body></html>\r\n";
        let b = parse_body(raw);
        assert!(b.paragraphs.join(" ").contains("Hi"));
        assert!(!b.paragraphs.join(" ").contains('<'));
        assert_eq!(b.remote_images.as_deref(), Some("track.example.com"));
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
