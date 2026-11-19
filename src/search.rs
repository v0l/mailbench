use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub days: u32,
    #[serde(default)]
    pub megabytes: u32,
}

impl BackfillPolicy {
    pub fn on() -> Self {
        BackfillPolicy { enabled: true, days: 0, megabytes: 0 }
    }

    pub fn cutoff(&self, now: i64) -> Option<i64> {
        (self.days > 0).then(|| now - self.days as i64 * 86_400)
    }

    pub fn byte_cap(&self) -> Option<u64> {
        (self.megabytes > 0).then(|| self.megabytes as u64 * 1_048_576)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub terms: Vec<String>,
    pub phrases: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub has_attachment: bool,
    pub unread: Option<bool>,
    pub flagged: Option<bool>,
    pub folder: Option<String>,
    pub before: Option<i64>,
    pub after: Option<i64>,
}

fn split(input: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                was_quoted = true;
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() || was_quoted {
                    out.push((std::mem::take(&mut cur), was_quoted));
                }
                was_quoted = false;
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() || was_quoted {
        out.push((cur, was_quoted));
    }
    out
}

pub fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.split(['-', '/']);
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1000..=9999).contains(&y) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400)
}

impl Parsed {
    pub fn parse(input: &str) -> Parsed {
        let mut p = Parsed::default();
        for (word, quoted) in split(input) {
            let lower = word.to_lowercase();
            if quoted && !lower.contains(':') {
                if !lower.trim().is_empty() {
                    p.phrases.push(lower.trim().to_string());
                }
                continue;
            }
            let Some((key, value)) = lower.split_once(':') else {
                if !lower.is_empty() {
                    p.terms.push(lower);
                }
                continue;
            };
            let value = value.trim().to_string();
            let handled = match (key, value.as_str()) {
                (_, "") => false,
                ("from", v) => {
                    p.from.push(v.into());
                    true
                }
                ("to", v) => {
                    p.to.push(v.into());
                    true
                }
                ("subject", v) => {
                    p.subject.push(v.into());
                    true
                }
                ("has", "attachment" | "attachments") => {
                    p.has_attachment = true;
                    true
                }
                ("is", "unread") => {
                    p.unread = Some(true);
                    true
                }
                ("is", "read") => {
                    p.unread = Some(false);
                    true
                }
                ("is", "flagged" | "starred") => {
                    p.flagged = Some(true);
                    true
                }
                ("in", v) => {
                    p.folder = Some(v.into());
                    true
                }
                ("before", v) => match parse_date(v) {
                    Some(t) => {
                        p.before = Some(t);
                        true
                    }
                    None => false,
                },
                ("after", v) => match parse_date(v) {
                    Some(t) => {
                        p.after = Some(t);
                        true
                    }
                    None => false,
                },
                _ => false,
            };
            if !handled {
                p.terms.push(lower);
            }
        }
        p
    }

    pub fn has_text(&self) -> bool {
        !self.terms.is_empty() || !self.phrases.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        *self == Parsed::default()
    }

    pub fn highlights(&self) -> Vec<String> {
        self.terms
            .iter()
            .chain(&self.phrases)
            .chain(&self.subject)
            .chain(&self.from)
            .filter(|t| !t.is_empty())
            .cloned()
            .collect()
    }

    pub fn index_query(&self) -> Option<String> {
        self.has_text().then(|| self.terms.iter().chain(&self.phrases).cloned().collect::<Vec<_>>().join(" "))
    }

    pub fn server_query(&self) -> Option<String> {
        if !self.has_text() {
            return None;
        }
        let quote = |t: &str| format!("TEXT \"{}\"", t.replace(['"', '\\'], ""));
        Some(self.terms.iter().chain(&self.phrases).map(|t| quote(t)).collect::<Vec<_>>().join(" "))
    }
}

pub fn spans(text: &str, needles: &[String]) -> Vec<(usize, usize)> {
    let lower: String = text.chars().flat_map(|c| c.to_lowercase()).collect();
    if lower.len() != text.len() {
        return Vec::new();
    }
    let mut found: Vec<(usize, usize)> = Vec::new();
    for n in needles {
        if n.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(i) = lower[from..].find(n.as_str()) {
            found.push((from + i, from + i + n.len()));
            from += i + n.len();
        }
    }
    found.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in found {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_become_terms() {
        let p = Parsed::parse("Invoice March");
        assert_eq!(p.terms, ["invoice", "march"]);
        assert!(p.has_text());
    }

    #[test]
    fn quoted_phrases_stay_together() {
        let p = Parsed::parse("\"Parts Bin\" order");
        assert_eq!(p.phrases, ["parts bin"]);
        assert_eq!(p.terms, ["order"]);
    }

    #[test]
    fn field_operators() {
        let p = Parsed::parse("from:mara to:jonas@x.test subject:\"board rev\" budget");
        assert_eq!(p.from, ["mara"]);
        assert_eq!(p.to, ["jonas@x.test"]);
        assert_eq!(p.subject, ["board rev"]);
        assert_eq!(p.terms, ["budget"]);
    }

    #[test]
    fn flag_operators() {
        let p = Parsed::parse("has:attachment is:unread is:flagged in:Archive");
        assert!(p.has_attachment);
        assert_eq!(p.unread, Some(true));
        assert_eq!(p.flagged, Some(true));
        assert_eq!(p.folder.as_deref(), Some("archive"));
        assert!(!p.has_text());
        assert_eq!(Parsed::parse("is:read").unread, Some(false));
    }

    #[test]
    fn dates_are_start_of_day_utc() {
        let p = Parsed::parse("after:2023-11-14 before:2023/11/16");
        assert_eq!(p.after, Some(1_699_920_000));
        assert_eq!(p.before, Some(1_700_092_800));
        assert_eq!(parse_date("1970-01-02"), Some(86_400));
    }

    #[test]
    fn bad_dates_and_unknown_keys_are_plain_terms() {
        let p = Parsed::parse("before:soon https://x.test foo:");
        assert_eq!(p.before, None);
        assert_eq!(p.terms, ["before:soon", "https://x.test", "foo:"]);
    }

    #[test]
    fn empty_and_blank_inputs() {
        assert!(Parsed::parse("").is_empty());
        assert!(Parsed::parse("   ").is_empty());
        assert!(Parsed::parse("is:unread").has_text() == false);
    }

    #[test]
    fn server_query_asks_for_text() {
        let p = Parsed::parse("rover \"gear box\" is:unread");
        assert_eq!(p.server_query().unwrap(), "TEXT \"rover\" TEXT \"gear box\"");
        assert_eq!(Parsed::parse("is:unread").server_query(), None);
    }

    #[test]
    fn highlight_spans_merge_overlaps() {
        let spans = spans("Parts bin parts", &["parts".to_string(), "bin".to_string(), "ts b".to_string()]);
        assert_eq!(spans, [(0, 9), (10, 15)]);
    }

    #[test]
    fn highlights_cover_terms_phrases_and_fields() {
        let p = Parsed::parse("rover from:mara \"gear box\"");
        assert_eq!(p.highlights(), ["rover", "gear box", "mara"]);
    }
}
