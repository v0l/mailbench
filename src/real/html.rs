use crate::mailstore::{Block, Span};
use scraper::{Html, Node};

const MAX_HTML: usize = 2_000_000;
const MAX_DEPTH: usize = 120;
const MAX_BLOCKS: usize = 3000;

#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    link: Option<String>,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    heading: bool,
    quote: usize,
}

impl Builder {
    fn push_text(&mut self, text: &str, style: &Style) {
        let mut out = String::with_capacity(text.len());
        let mut space = self.spans.last().is_none_or(|s| s.text.ends_with([' ', '\n']));
        for c in text.chars() {
            if c.is_whitespace() || c == '\u{200b}' || c == '\u{feff}' || c == '\u{34f}' {
                if !space {
                    out.push(' ');
                    space = true;
                }
            } else {
                out.push(c);
                space = false;
            }
        }
        if out.is_empty() {
            return;
        }
        self.add(out, style);
    }

    fn add(&mut self, text: String, style: &Style) {
        if let Some(last) = self.spans.last_mut() {
            if last.bold == style.bold && last.italic == style.italic && last.link == style.link {
                last.text.push_str(&text);
                return;
            }
        }
        self.spans.push(Span { text, bold: style.bold, italic: style.italic, link: style.link.clone() });
    }

    fn line_break(&mut self, style: &Style) {
        self.add("\n".to_string(), style);
    }

    fn flush(&mut self) {
        let mut spans = std::mem::take(&mut self.spans);
        while spans.last().is_some_and(|s| s.text.trim_end().is_empty() && s.link.is_none()) {
            spans.pop();
        }
        if let Some(last) = spans.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
        }
        if let Some(first) = spans.first_mut() {
            let trimmed = first.text.trim_start().to_string();
            first.text = trimmed;
        }
        spans.retain(|s| !s.text.is_empty());
        if spans.is_empty() || self.blocks.len() >= MAX_BLOCKS {
            return;
        }
        self.blocks.push(Block::Text { spans, heading: self.heading, quote: self.quote > 0 });
    }
}

fn safe_link(href: &str) -> Option<String> {
    let h = href.trim();
    let lower = h.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")).then(|| h.to_string())
}

fn walk(node: ego_tree::NodeRef<'_, Node>, style: &Style, b: &mut Builder, depth: usize) {
    if depth > MAX_DEPTH || b.blocks.len() >= MAX_BLOCKS {
        return;
    }
    match node.value() {
        Node::Text(t) => b.push_text(t, style),
        Node::Element(e) => {
            let name = e.name();
            if matches!(name, "script" | "style" | "head" | "title" | "noscript" | "template" | "svg" | "iframe" | "object") {
                return;
            }
            if e.attr("hidden").is_some() || e.attr("style").is_some_and(hidden_by_style) {
                return;
            }
            let mut next = style.clone();
            match name {
                "b" | "strong" | "th" => next.bold = true,
                "i" | "em" | "cite" => next.italic = true,
                "a" => {
                    if let Some(l) = e.attr("href").and_then(safe_link) {
                        next.link = Some(l);
                    }
                }
                _ => {}
            }
            match name {
                "br" => {
                    b.line_break(style);
                    return;
                }
                "hr" => {
                    b.flush();
                    b.blocks.push(Block::Rule);
                    return;
                }
                "img" => {
                    let alt = e.attr("alt").map(str::trim).unwrap_or_default();
                    if !alt.is_empty() && alt.len() < 120 {
                        let mut s = next.clone();
                        s.italic = true;
                        b.push_text(&format!("[{alt}]"), &s);
                    }
                    return;
                }
                _ => {}
            }
            let heading = matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
            let block = heading
                || matches!(
                    name,
                    "p" | "div" | "tr" | "table" | "ul" | "ol" | "li" | "blockquote" | "section" | "article" | "header"
                        | "footer" | "pre" | "td" | "th" | "center" | "main" | "aside" | "nav" | "form" | "dl" | "dt" | "dd"
                );
            if block {
                b.flush();
            }
            if heading {
                next.bold = true;
                b.heading = true;
            }
            if name == "blockquote" {
                b.quote += 1;
            }
            if name == "li" {
                b.push_text("\u{2022} ", &next);
            }
            for child in node.children() {
                walk(child, &next, b, depth + 1);
            }
            if block {
                b.flush();
            }
            if heading {
                b.heading = false;
            }
            if name == "blockquote" {
                b.quote -= 1;
            }
        }
        _ => {
            for child in node.children() {
                walk(child, style, b, depth + 1);
            }
        }
    }
}

fn hidden_by_style(style: &str) -> bool {
    let s: String = style.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    s.contains("display:none") || s.contains("visibility:hidden") || s.contains("font-size:0") || s.contains("max-height:0")
}

pub fn render(html: &str) -> Vec<Block> {
    let html = if html.len() > MAX_HTML { &html[..html.floor_char_boundary(MAX_HTML)] } else { html };
    let doc = Html::parse_document(html);
    let mut b = Builder::default();
    walk(doc.tree.root(), &Style::default(), &mut b, 0);
    b.flush();
    b.blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { spans, .. } => Some(spans.iter().map(|s| s.text.as_str()).collect::<String>()),
                Block::Rule => None,
            })
            .collect()
    }

    #[test]
    fn paragraphs_become_blocks_and_entities_decode() {
        let blocks = render("<html><body><p>Hello &amp; welcome</p><p>Second<br>line</p></body></html>");
        assert_eq!(text_of(&blocks), vec!["Hello & welcome", "Second\nline"]);
    }

    #[test]
    fn scripts_styles_and_hidden_preheaders_are_dropped() {
        let blocks = render("<style>p{color:red}</style><script>alert(1)</script><div style=\"display:none\">preheader</div><p>Shown</p>");
        assert_eq!(text_of(&blocks), vec!["Shown"]);
    }

    #[test]
    fn links_keep_their_target_and_unsafe_schemes_are_dropped() {
        let blocks = render("<p>See <a href=\"https://example.com/x\">this</a> and <a href=\"javascript:alert(1)\">that</a></p>");
        let Block::Text { spans, .. } = &blocks[0] else { panic!("text block") };
        let linked: Vec<_> = spans.iter().filter_map(|s| s.link.as_deref()).collect();
        assert_eq!(linked, vec!["https://example.com/x"]);
        assert_eq!(text_of(&blocks), vec!["See this and that"]);
    }

    #[test]
    fn emphasis_headings_lists_and_quotes_are_marked() {
        let blocks = render("<h1>Title</h1><p><b>bold</b> <i>it</i></p><ul><li>one</li><li>two</li></ul><blockquote>old</blockquote>");
        let Block::Text { heading, .. } = &blocks[0] else { panic!("text block") };
        assert!(*heading);
        let Block::Text { spans, .. } = &blocks[1] else { panic!("text block") };
        assert!(spans[0].bold && spans.iter().any(|s| s.italic));
        assert_eq!(text_of(&blocks)[2], "\u{2022} one");
        assert!(matches!(blocks.last(), Some(Block::Text { quote: true, .. })));
    }

    #[test]
    fn table_layout_mail_flattens_to_readable_text() {
        let blocks = render("<table><tr><td>Name</td></tr><tr><td><img src=\"https://t.example/p.gif\" alt=\"logo\"></td></tr></table>");
        assert_eq!(text_of(&blocks), vec!["Name", "[logo]"]);
    }

    #[test]
    fn pathological_nesting_does_not_overflow_the_stack() {
        let html = format!("{}x{}", "<div>".repeat(5000), "</div>".repeat(5000));
        let _ = render(&html);
    }
}
