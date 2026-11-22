use crate::mailstore::{AuthResult, Headers, Unsubscribe};

fn split(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, v)) = out.last_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn first<'a>(all: &'a [(String, String)], name: &str) -> Option<&'a str> {
    all.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

fn address_of(value: &str) -> String {
    let inner = match (value.rfind('<'), value.rfind('>')) {
        (Some(a), Some(b)) if a < b => &value[a + 1..b],
        _ => value,
    };
    inner.trim().to_lowercase()
}

fn domain_of(address: &str) -> Option<String> {
    address.rsplit_once('@').map(|(_, d)| d.trim().to_string()).filter(|d| !d.is_empty())
}

fn method_result(value: &str, method: &str) -> Option<String> {
    let prefix = format!("{method}=");
    let results: Vec<String> = value
        .split(|c: char| c.is_whitespace() || c == ';' || c == '(')
        .filter_map(|t| t.to_lowercase().strip_prefix(&prefix).map(|r| r.trim_matches(|c| c == ')' || c == ';').to_string()))
        .filter(|r| !r.is_empty())
        .collect();
    results.iter().find(|r| *r == "pass").cloned().or_else(|| results.into_iter().next())
}

pub fn from_name(all: &[(String, String)]) -> String {
    let Some(v) = first(all, "From") else {
        return String::new();
    };
    match v.rfind('<') {
        Some(i) => v[..i].trim().to_string(),
        None => String::new(),
    }
}

fn spam(all: &[(String, String)]) -> (Option<f32>, bool) {
    let score = first(all, "X-Spam-Score").and_then(|v| {
        let tail = v.split_once("score=").map(|(_, t)| t).unwrap_or(v);
        let number: String = tail.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-')).collect();
        number.parse().ok()
    });
    let flagged = first(all, "X-Spam-Flag").is_some_and(|f| f.eq_ignore_ascii_case("yes"))
        || first(all, "X-Spam-Status").is_some_and(|s| s.to_lowercase().starts_with("yes"))
        || first(all, "X-Spam-Score").is_some_and(|s| s.to_lowercase().starts_with("spam"));
    (score, flagged)
}

fn authentication(all: &[(String, String)]) -> AuthResult {
    let Some(v) = first(all, "Authentication-Results") else {
        return AuthResult::default();
    };
    AuthResult { spf: method_result(v, "spf"), dkim: method_result(v, "dkim"), dmarc: method_result(v, "dmarc") }
}

fn unsubscribe(all: &[(String, String)]) -> Option<Unsubscribe> {
    let value = first(all, "List-Unsubscribe")?;
    let targets: Vec<&str> = value
        .split('<')
        .skip(1)
        .filter_map(|s| s.split_once('>').map(|(t, _)| t.trim()))
        .collect();
    let one_click_header = first(all, "List-Unsubscribe-Post").is_some_and(|p| p.to_lowercase().contains("one-click"));
    if let Some(url) = targets.iter().find(|t| t.to_lowercase().starts_with("https://")) {
        return Some(Unsubscribe { target: url.to_string(), one_click: one_click_header });
    }
    targets
        .iter()
        .find(|t| t.to_lowercase().starts_with("mailto:"))
        .map(|t| Unsubscribe { target: t.to_string(), one_click: false })
}

fn list_id(all: &[(String, String)]) -> Option<String> {
    let v = first(all, "List-Id")?;
    match (v.rfind('<'), v.rfind('>')) {
        (Some(a), Some(b)) if a < b => Some(v[a + 1..b].trim().to_string()),
        _ => Some(v.trim().to_string()),
    }
    .filter(|s| !s.is_empty())
}

fn reply_to_differs(all: &[(String, String)]) -> Option<String> {
    let from = domain_of(&address_of(first(all, "From")?))?;
    let reply = address_of(first(all, "Reply-To")?);
    let reply_domain = domain_of(&reply)?;
    (reply_domain != from).then_some(reply)
}

fn bulk(all: &[(String, String)]) -> bool {
    first(all, "Precedence").is_some_and(|p| matches!(p.to_lowercase().as_str(), "bulk" | "list" | "junk"))
        || first(all, "Auto-Submitted").is_some_and(|a| !a.eq_ignore_ascii_case("no"))
        || first(all, "List-Unsubscribe").is_some()
}

pub fn parse_headers(raw: &[u8]) -> Headers {
    let all = split(raw);
    let (spam_score, spam) = spam(&all);
    Headers {
        spam_score,
        spam,
        list_id: list_id(&all),
        unsubscribe: unsubscribe(&all),
        auth: authentication(&all),
        reply_to_differs: reply_to_differs(&all),
        spoof: Vec::new(),
        pgp: None,
        bulk: bulk(&all),
        hops: all.iter().filter(|(k, _)| k.eq_ignore_ascii_case("Received")).count(),
        all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEWSLETTER: &[u8] = b"From: News <news@lists.example.com>\r\nReply-To: <bounce@tracker.example.net>\r\nList-Id: The Weekly <weekly.lists.example.com>\r\nList-Unsubscribe: <mailto:unsub@lists.example.com>,\r\n <https://lists.example.com/u/abc>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\nAuthentication-Results: mx.example.org;\r\n spf=pass smtp.mailfrom=lists.example.com;\r\n dkim=pass header.d=lists.example.com;\r\n dmarc=fail (p=none) header.from=lists.example.com\r\nReceived: from a by b\r\nReceived: from c by d\r\nSubject: hi\r\n\r\nbody\r\n";

    #[test]
    fn one_click_https_unsubscribe_wins_over_mailto() {
        let h = parse_headers(NEWSLETTER);
        let u = h.unsubscribe.unwrap();
        assert_eq!(u.target, "https://lists.example.com/u/abc");
        assert!(u.one_click);
    }

    #[test]
    fn mailto_only_unsubscribe_is_not_one_click() {
        let h = parse_headers(b"From: a@x.test\r\nList-Unsubscribe: <mailto:u@x.test>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\n");
        let u = h.unsubscribe.unwrap();
        assert_eq!(u.target, "mailto:u@x.test");
        assert!(!u.one_click);
    }

    #[test]
    fn list_id_is_the_bracketed_part() {
        assert_eq!(parse_headers(NEWSLETTER).list_id.as_deref(), Some("weekly.lists.example.com"));
    }

    #[test]
    fn authentication_results_are_read_per_method() {
        let a = parse_headers(NEWSLETTER).auth;
        assert_eq!(a.spf.as_deref(), Some("pass"));
        assert_eq!(a.dkim.as_deref(), Some("pass"));
        assert_eq!(a.dmarc.as_deref(), Some("fail"));
    }

    #[test]
    fn reply_to_on_another_domain_is_flagged() {
        assert_eq!(parse_headers(NEWSLETTER).reply_to_differs.as_deref(), Some("bounce@tracker.example.net"));
        let same = parse_headers(b"From: A <a@x.test>\r\nReply-To: b@x.test\r\n\r\n");
        assert_eq!(same.reply_to_differs, None);
    }

    #[test]
    fn from_name_is_the_display_part() {
        let h = parse_headers(b"From: \"ceo@corp.test\" <x@evil.test>\r\n\r\n");
        assert_eq!(from_name(&h.all), "\"ceo@corp.test\"");
        assert_eq!(from_name(&parse_headers(b"From: a@b.test\r\n\r\n").all), "");
    }

    #[test]
    fn hops_bulk_and_all_headers_are_collected() {
        let h = parse_headers(NEWSLETTER);
        assert_eq!(h.hops, 2);
        assert!(h.bulk);
        assert!(h.all.iter().any(|(k, v)| k == "Subject" && v == "hi"));
        assert!(!h.all.iter().any(|(_, v)| v == "body"));
    }

    const SUPPORT: &[u8] = b"X-Mozilla-Status: 0001\r\nX-Spam-Status: No\r\nReceived: from sender5-op-o12.zoho.com (sender5-op-o12.zoho.com [165.173.182.12] (AS2639 ZOHO, US))\r\n\t(using TLSv1.3 with cipher TLS13_AES_256_GCM_SHA384)\r\n\tby o-0.mx (Stalwart SMTP) with ESMTPS id 4BC519484428000;\r\n\tWed, 18 Nov 2026 09:37:37 +0000\r\nAuthentication-Results: o-0.mx;\r\n\tdkim=pass header.d=novatel.ie header.s=zoho header.b=G3abQ1zH;\r\n\tspf=none (o-0.mx: no SPF records found for postmaster@sender5-op-o12.zoho.com) smtp.helo=sender5-op-o12.zoho.com;\r\n\tspf=pass (o-0.mx: domain of jose.frazao@novatel.ie designates 165.173.182.12 as permitted sender) smtp.mailfrom=jose.frazao@novatel.ie;\r\n\tiprev=pass policy.iprev=165.173.182.12;\r\n\tdmarc=pass header.from=novatel.ie policy.dmarc=none\r\nX-Spam-Score: ham, score=3.41\r\nReceived: from mail.zoho.com by mx.zohomail.com\r\n\twith SMTP id 1794994651837742.3192256476109; Wed, 18 Nov 2026 01:37:31 -0800 (PST)\r\nFrom: Jose Frazao - Novatel Communications <jose.frazao@novatel.ie>\r\nSubject: Annual VPS Payment\r\n\r\nHello\r\n";

    #[test]
    fn a_passing_mailfrom_spf_beats_a_none_helo_result() {
        let h = parse_headers(SUPPORT);
        assert_eq!(h.auth.spf.as_deref(), Some("pass"));
        assert_eq!(h.auth.dkim.as_deref(), Some("pass"));
        assert_eq!(h.auth.dmarc.as_deref(), Some("pass"));
        assert!(!h.auth.failed());
    }

    #[test]
    fn spam_score_and_verdict_are_read_from_the_server_headers() {
        let h = parse_headers(SUPPORT);
        assert_eq!(h.spam_score, Some(3.41));
        assert!(!h.spam);
        assert_eq!(h.hops, 2);
        let flagged = parse_headers(b"X-Spam-Status: Yes, score=9.5\r\nX-Spam-Score: spam, score=9.5\r\n\r\n");
        assert!(flagged.spam);
        assert_eq!(flagged.spam_score, Some(9.5));
    }

    #[test]
    fn plain_mail_has_nothing_special() {
        let h = parse_headers(b"From: a@x.test\r\nSubject: s\r\n\r\nhi\r\n");
        assert!(h.unsubscribe.is_none() && h.list_id.is_none() && !h.bulk);
        assert_eq!(h.auth, AuthResult::default());
    }
}
