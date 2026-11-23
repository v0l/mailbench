use mail_parser::MessageParser;
use mailbench::backend::{render_message, Address, Attachment, OutgoingMessage};

fn message(body: &str, html: bool) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: Some("Me".into()), address: "me@example.test".into() },
        to: vec![Address { name: None, address: "you@example.test".into() }],
        subject: "hi".into(),
        body: body.into(),
        html_alternative: html,
        ..Default::default()
    }
}

fn render(m: &OutgoingMessage) -> Vec<u8> {
    render_message(m, false).unwrap()
}

fn html_of(raw: &[u8]) -> String {
    let parsed = MessageParser::default().parse(raw).unwrap();
    parsed.body_html(0).unwrap().to_string()
}

fn text_of(raw: &[u8]) -> String {
    let parsed = MessageParser::default().parse(raw).unwrap();
    parsed.body_text(0).unwrap().replace("\r\n", "\n")
}

#[test]
fn unmarked_text_stays_one_plain_part() {
    let raw = render(&message("just words\nand more", true));
    let s = String::from_utf8_lossy(&raw);
    assert!(!s.contains("multipart"), "{s}");
    assert!(s.contains("format=flowed"));
}

#[test]
fn bold_text_builds_an_alternative_with_the_typed_text_and_html() {
    let typed = "hello **world**\nsecond";
    let raw = render(&message(typed, true));
    let s = String::from_utf8_lossy(&raw);
    assert!(s.contains("multipart/alternative"));
    assert_eq!(text_of(&raw).trim_end(), typed);
    let html = html_of(&raw);
    assert!(html.contains("<strong>world</strong>"), "{html}");
    assert!(html.contains("<br>"));
}

#[test]
fn a_javascript_link_is_escaped_text_not_a_link() {
    let raw = render(&message("**x** [bad](javascript:alert(1)) [ok](https://e.test/)", true));
    let html = html_of(&raw);
    assert!(!html.contains("href=\"javascript"), "{html}");
    assert!(html.contains("[bad](javascript:alert(1))"));
    assert!(html.contains("<a href=\"https://e.test/\">ok</a>"));
}

#[test]
fn nested_lists_and_code_blocks_render() {
    let raw = render(&message("- a\n  - b\n- c\n\n```\n<x> & *y*\n```", true));
    let html = html_of(&raw);
    assert!(html.contains("<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>"), "{html}");
    assert!(html.contains("<pre><code>&lt;x&gt; &amp; *y*</code></pre>"), "{html}");
}

#[test]
fn a_reply_quote_is_plain_in_text_and_a_blockquote_in_html() {
    let body = "Thanks **a lot**\n\nOn Mon, Ana wrote:\n> first line\n> second";
    let raw = render(&message(body, true));
    assert!(text_of(&raw).contains("> first line\n> second"));
    let html = html_of(&raw);
    assert!(html.contains("<blockquote>"), "{html}");
    assert!(html.contains("first line"));
}

#[test]
fn attachments_wrap_the_alternative_part_in_mixed() {
    let mut m = message("**bold**", true);
    m.attachments.push(Attachment { name: "a.txt".into(), mime: "text/plain".into(), data: b"data".to_vec(), ..Default::default() });
    let raw = render(&m);
    let s = String::from_utf8_lossy(&raw);
    assert!(s.find("multipart/mixed").unwrap() < s.find("multipart/alternative").unwrap());
    let parsed = MessageParser::default().parse(&raw).unwrap();
    assert_eq!(parsed.attachments().count(), 1);
    assert!(html_of(&raw).contains("<strong>bold</strong>"));
}

#[test]
fn plain_only_sends_no_html_part() {
    let raw = render(&message("**bold**", false));
    let s = String::from_utf8_lossy(&raw);
    assert!(!s.contains("multipart") && !s.contains("<strong>"));
}
