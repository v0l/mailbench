use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bold,
    Italic,
    Code,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub inner: Range<usize>,
    pub url: Range<usize>,
}

fn edged(inner: &str) -> bool {
    inner.is_empty() || inner.starts_with(char::is_whitespace) || inner.ends_with(char::is_whitespace)
}

pub fn safe_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let lower = u.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p) && lower.len() > p.len())
}

pub fn tokens(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let found = match b[i] {
            b'`' => s[i + 1..].find('`').filter(|n| *n > 0).map(|n| Tok {
                kind: Kind::Code,
                start: i,
                end: i + n + 2,
                inner: i + 1..i + 1 + n,
                url: 0..0,
            }),
            b'*' if b.get(i + 1) == Some(&b'*') => s[i + 2..].find("**").filter(|n| !edged(&s[i + 2..i + 2 + n])).map(|n| Tok {
                kind: Kind::Bold,
                start: i,
                end: i + n + 4,
                inner: i + 2..i + 2 + n,
                url: 0..0,
            }),
            b'*' => s[i + 1..].find('*').filter(|n| !edged(&s[i + 1..i + 1 + n])).map(|n| Tok {
                kind: Kind::Italic,
                start: i,
                end: i + n + 2,
                inner: i + 1..i + 1 + n,
                url: 0..0,
            }),
            b'[' => link_at(s, i),
            _ => None,
        };
        match found {
            Some(t) => {
                i = t.end;
                out.push(t);
            }
            None => i += 1,
        }
    }
    out
}

fn link_at(s: &str, i: usize) -> Option<Tok> {
    let close = s[i + 1..].find("](")? + i + 1;
    if close == i + 1 || s[i + 1..close].contains('[') {
        return None;
    }
    let url_start = close + 2;
    let url_len = s[url_start..].find(')')?;
    let url = url_start..url_start + url_len;
    Some(Tok { kind: Kind::Link, start: i, end: url.end + 1, inner: i + 1..close, url })
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn inline(s: &str, marked: &mut bool) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for t in tokens(s) {
        if t.kind == Kind::Link && !safe_url(&s[t.url.clone()]) {
            continue;
        }
        out.push_str(&escape(&s[pos..t.start]));
        pos = t.end;
        *marked = true;
        let inner = &s[t.inner.clone()];
        match t.kind {
            Kind::Bold => out.push_str(&format!("<strong>{}</strong>", inline(inner, marked))),
            Kind::Italic => out.push_str(&format!("<em>{}</em>", inline(inner, marked))),
            Kind::Code => out.push_str(&format!("<code>{}</code>", escape(inner))),
            Kind::Link => out.push_str(&format!("<a href=\"{}\">{}</a>", escape(s[t.url.clone()].trim()), inline(inner, marked))),
        }
    }
    out.push_str(&escape(&s[pos..]));
    out
}

struct Item<'a> {
    indent: usize,
    ordered: bool,
    marker: usize,
    text: &'a str,
}

fn item(line: &str) -> Option<Item<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    for p in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(p) {
            return Some(Item { indent, ordered: false, marker: indent + 2, text: rest });
        }
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) && trimmed[digits..].starts_with(". ") {
        return Some(Item { indent, ordered: true, marker: indent + digits + 2, text: &trimmed[digits + 2..] });
    }
    None
}

fn fence(line: &str) -> bool {
    line.trim_end().starts_with("```")
}

fn list(lines: &[&str], i: &mut usize, marked: &mut bool) -> String {
    let first = item(lines[*i]).expect("list starts at an item");
    let (indent, ordered) = (first.indent, first.ordered);
    let tag = if ordered { "ol" } else { "ul" };
    let mut out = format!("<{tag}>");
    while *i < lines.len() {
        let Some(it) = item(lines[*i]) else { break };
        if it.indent < indent || (it.indent == indent && it.ordered != ordered) {
            break;
        }
        if it.indent > indent {
            break;
        }
        *i += 1;
        out.push_str("<li>");
        out.push_str(&inline(it.text, marked));
        if *i < lines.len() && item(lines[*i]).is_some_and(|n| n.indent > indent) {
            out.push_str(&list(lines, i, marked));
        }
        out.push_str("</li>");
    }
    out.push_str(&format!("</{tag}>"));
    *marked = true;
    out
}

fn blocks(lines: &[&str], marked: &mut bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
        } else if fence(line) {
            i += 1;
            let mut code = Vec::new();
            while i < lines.len() && !fence(lines[i]) {
                code.push(lines[i]);
                i += 1;
            }
            i += 1;
            *marked = true;
            out.push_str(&format!("<pre><code>{}</code></pre>\n", escape(&code.join("\n"))));
        } else if line.starts_with('>') {
            let mut inner: Vec<&str> = Vec::new();
            while i < lines.len() && lines[i].starts_with('>') {
                let l = &lines[i][1..];
                inner.push(l.strip_prefix(' ').unwrap_or(l));
                i += 1;
            }
            *marked = true;
            out.push_str(&format!("<blockquote>\n{}</blockquote>\n", blocks(&inner, marked)));
        } else if item(line).is_some() {
            out.push_str(&list(lines, &mut i, marked));
            out.push('\n');
        } else {
            let mut para = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() && !fence(lines[i]) && !lines[i].starts_with('>') && (para.is_empty() || item(lines[i]).is_none()) {
                para.push(inline(lines[i].trim_end(), marked));
                i += 1;
            }
            out.push_str(&format!("<p>{}</p>\n", para.join("<br>\n")));
        }
    }
    out
}

pub fn render(text: &str) -> (String, bool) {
    let normal = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normal.split('\n').collect();
    let mut marked = false;
    let body = blocks(&lines, &mut marked);
    (format!("<!DOCTYPE html>\n<html><body>\n{body}</body></html>\n"), marked)
}

pub fn has_marks(text: &str) -> bool {
    render(text).1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Mark,
    Bold,
    Italic,
    Code,
    Link,
    Quote,
}

fn push(out: &mut Vec<(Range<usize>, Style)>, range: Range<usize>, style: Style) {
    if range.start < range.end {
        out.push((range, style));
    }
}

fn inline_spans(s: &str, base: usize, out: &mut Vec<(Range<usize>, Style)>) {
    let mut pos = 0;
    for t in tokens(s) {
        if t.kind == Kind::Link && !safe_url(&s[t.url.clone()]) {
            continue;
        }
        push(out, base + pos..base + t.start, Style::Plain);
        pos = t.end;
        let (open, close, style) = match t.kind {
            Kind::Bold => (2, 2, Style::Bold),
            Kind::Italic => (1, 1, Style::Italic),
            Kind::Code => (1, 1, Style::Code),
            Kind::Link => (1, t.end - t.inner.end, Style::Link),
        };
        push(out, base + t.start..base + t.start + open, Style::Mark);
        push(out, base + t.inner.start..base + t.inner.end, style);
        push(out, base + t.inner.end..base + t.inner.end + close, Style::Mark);
    }
    push(out, base + pos..base + s.len(), Style::Plain);
}

pub fn spans(text: &str) -> Vec<(Range<usize>, Style)> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut in_fence = false;
    for raw in text.split_inclusive('\n') {
        let body = raw.strip_suffix('\n').unwrap_or(raw);
        let nl = raw.len() - body.len();
        let end = offset + body.len();
        if fence(body) {
            in_fence = !in_fence;
            push(&mut out, offset..end, Style::Mark);
        } else if in_fence {
            push(&mut out, offset..end, Style::Code);
        } else if body.starts_with('>') {
            let marks = body.bytes().take_while(|b| *b == b'>' || *b == b' ').count();
            push(&mut out, offset..offset + marks, Style::Mark);
            push(&mut out, offset + marks..end, Style::Quote);
        } else if let Some(it) = item(body) {
            push(&mut out, offset..offset + it.marker, Style::Mark);
            inline_spans(it.text, offset + it.marker, &mut out);
        } else {
            inline_spans(body, offset, &mut out);
        }
        push(&mut out, end..end + nl, Style::Plain);
        offset = end + nl;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Link,
    Bullets,
    Numbers,
    Quote,
    Code,
}

fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map(|(i, _)| i).unwrap_or(text.len())
}

fn chars_of(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

pub fn apply(text: &str, selection: (usize, usize), format: Format) -> (String, (usize, usize)) {
    let (a, b) = (selection.0.min(selection.1), selection.0.max(selection.1));
    let (sa, sb) = (byte_of(text, a), byte_of(text, b));
    match format {
        Format::Bold | Format::Italic | Format::Code | Format::Link => inline_format(text, sa, sb, format),
        Format::Bullets | Format::Numbers | Format::Quote => line_format(text, sa, sb, format),
    }
}

fn inline_format(text: &str, sa: usize, sb: usize, format: Format) -> (String, (usize, usize)) {
    let selected = &text[sa..sb];
    let (head, tail) = (&text[..sa], &text[sb..]);
    if format == Format::Code && selected.contains('\n') {
        let out = format!("{head}```\n{selected}\n```{tail}");
        let start = chars_of(&out, sa + 4);
        return (out, (start, start + selected.chars().count()));
    }
    let (open, close) = match format {
        Format::Bold => ("**", "**"),
        Format::Italic => ("*", "*"),
        Format::Code => ("`", "`"),
        _ => ("[", "](https://)"),
    };
    if format == Format::Link {
        let label = if selected.is_empty() { "link" } else { selected };
        let out = format!("{head}[{label}](https://){tail}");
        let url_start = chars_of(&out, sa + 1 + label.len() + 2);
        return (out, (url_start, url_start + "https://".len()));
    }
    if selected.len() >= open.len() + close.len() && selected.starts_with(open) && selected.ends_with(close) && !(format == Format::Italic && selected.starts_with("**")) {
        let inner = &selected[open.len()..selected.len() - close.len()];
        let out = format!("{head}{inner}{tail}");
        let start = chars_of(&out, sa);
        return (out, (start, start + inner.chars().count()));
    }
    let out = format!("{head}{open}{selected}{close}{tail}");
    let start = chars_of(&out, sa + open.len());
    (out, (start, start + selected.chars().count()))
}

fn line_prefix(format: Format, n: usize) -> String {
    match format {
        Format::Bullets => "- ".to_string(),
        Format::Numbers => format!("{n}. "),
        _ => "> ".to_string(),
    }
}

fn has_prefix(line: &str, format: Format) -> Option<usize> {
    match format {
        Format::Bullets => ["- ", "* ", "+ "].iter().find(|p| line.starts_with(*p)).map(|p| p.len()),
        Format::Numbers => {
            let d = line.bytes().take_while(u8::is_ascii_digit).count();
            (d > 0 && line[d..].starts_with(". ")).then_some(d + 2)
        }
        _ => line.strip_prefix('>').map(|r| if r.starts_with(' ') { 2 } else { 1 }),
    }
}

fn line_format(text: &str, sa: usize, sb: usize, format: Format) -> (String, (usize, usize)) {
    let line_start = text[..sa].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[sb..].find('\n').map(|i| i + sb).unwrap_or(text.len());
    let block = &text[line_start..line_end];
    let lines: Vec<&str> = block.split('\n').collect();
    let all = lines.iter().filter(|l| !l.trim().is_empty()).all(|l| has_prefix(l, format).is_some()) && lines.iter().any(|l| !l.trim().is_empty());
    let mut n = 0;
    let rewritten: Vec<String> = lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() && lines.len() > 1 {
                return l.to_string();
            }
            n += 1;
            if all {
                l[has_prefix(l, format).unwrap_or(0)..].to_string()
            } else {
                format!("{}{l}", line_prefix(format, n))
            }
        })
        .collect();
    let joined = rewritten.join("\n");
    let out = format!("{}{}{}", &text[..line_start], joined, &text[line_end..]);
    let start = chars_of(&out, line_start);
    let end = start + joined.chars().count();
    if lines.len() == 1 {
        (out, (end, end))
    } else {
        (out, (start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(s: &str) -> String {
        render(s).0
    }

    #[test]
    fn plain_text_has_no_marks() {
        assert!(!has_marks("hello there\nsecond line\n\n5 * 3 * 2 and a_b_c\n-- \nMe"));
    }

    #[test]
    fn bold_italic_code_links_and_escaping() {
        let h = html("**bold** and *it* and `a<b` and [x](https://e.test/?a=1&b=2) <script>");
        assert!(h.contains("<strong>bold</strong>"));
        assert!(h.contains("<em>it</em>"));
        assert!(h.contains("<code>a&lt;b</code>"));
        assert!(h.contains("<a href=\"https://e.test/?a=1&amp;b=2\">x</a>"));
        assert!(h.contains("&lt;script&gt;"));
    }

    #[test]
    fn unsafe_link_schemes_stay_text() {
        let h = html("[click](javascript:alert(1)) [d](data:text/html,x) [f](ftp://x)");
        assert!(!h.contains("<a "));
        assert!(h.contains("[click](javascript:alert(1)"));
    }

    #[test]
    fn nested_lists_and_quotes() {
        let h = html("- one\n  - inner\n- two\n\n1. a\n2. b\n\n> quoted\n> *more*");
        assert!(h.contains("<ul><li>one<ul><li>inner</li></ul></li><li>two</li></ul>"), "{h}");
        assert!(h.contains("<ol><li>a</li><li>b</li></ol>"));
        assert!(h.contains("<blockquote>"));
        assert!(h.contains("<em>more</em>"));
    }

    #[test]
    fn code_blocks_keep_marks_literal() {
        let h = html("```\nlet a = *x*;\n<b>\n```");
        assert!(h.contains("<pre><code>let a = *x*;\n&lt;b&gt;</code></pre>"), "{h}");
    }

    #[test]
    fn spans_cover_the_text_exactly() {
        let t = "**bold** text\n> quote\n- item *it*\n```\ncode\n```\n[l](https://x.y)";
        let s = spans(t);
        let mut end = 0;
        for (r, _) in &s {
            assert_eq!(r.start, end);
            end = r.end;
        }
        assert_eq!(end, t.len());
        assert!(s.iter().any(|(r, st)| *st == Style::Bold && &t[r.clone()] == "bold"));
    }

    #[test]
    fn bold_wraps_the_selection_and_unwraps_again() {
        let (t, sel) = apply("say hello now", (4, 9), Format::Bold);
        assert_eq!(t, "say **hello** now");
        assert_eq!(sel, (6, 11));
        let (back, _) = apply(&t, (4, 13), Format::Bold);
        assert_eq!(back, "say hello now");
    }

    #[test]
    fn empty_selection_places_the_cursor_between_marks() {
        let (t, sel) = apply("ab", (1, 1), Format::Italic);
        assert_eq!(t, "a**b");
        assert_eq!(sel, (2, 2));
    }

    #[test]
    fn link_selects_the_address() {
        let (t, sel) = apply("see docs", (4, 8), Format::Link);
        assert_eq!(t, "see [docs](https://)");
        assert_eq!(&t[sel.0..sel.1], "https://");
    }

    #[test]
    fn list_and_quote_prefix_every_line_and_toggle_off() {
        let (t, _) = apply("a\nb\nc", (0, 5), Format::Numbers);
        assert_eq!(t, "1. a\n2. b\n3. c");
        let (t, _) = apply(&t, (0, t.chars().count()), Format::Numbers);
        assert_eq!(t, "a\nb\nc");
        let (t, _) = apply("one\ntwo", (5, 5), Format::Quote);
        assert_eq!(t, "one\n> two");
    }

    #[test]
    fn multi_line_code_becomes_a_fence() {
        let (t, _) = apply("x\ny", (0, 3), Format::Code);
        assert_eq!(t, "```\nx\ny\n```");
    }
}
