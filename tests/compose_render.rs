use mail_parser::MessageParser;
use mailbench::backend::{render_message, Address, OutgoingMessage};

fn message(body: &str) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: Some("Me".into()), address: "me@example.test".into() },
        to: vec![Address { name: None, address: "you@example.test".into() }],
        subject: "hi".into(),
        body: body.into(),
        ..Default::default()
    }
}

fn render(m: &OutgoingMessage) -> Vec<u8> {
    render_message(m, false).unwrap()
}

fn body_of(m: &mail_parser::Message) -> String {
    let t = m.body_text(0).unwrap().to_string();
    t.strip_suffix("\r\n").or(t.strip_suffix('\n')).unwrap_or(&t).to_string()
}

fn unflow(text: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = text.split("\r\n").collect();
    for (i, l) in lines.iter().enumerate() {
        let (depth, rest) = {
            let mut d = 0;
            let mut r = *l;
            while let Some(x) = r.strip_prefix('>') {
                d += 1;
                r = x;
            }
            (d, r)
        };
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let soft = rest.ends_with(' ') && *l != "-- ";
        if soft {
            out.push_str(&">".repeat(depth));
            if depth > 0 {
                out.push(' ');
            }
            out.push_str(rest);
        } else {
            out.push_str(l);
            if i + 1 < lines.len() {
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn typed_line_breaks_survive_a_round_trip() {
    let raw = render(&message("one\ntwo\n\nthree"));
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let body = body_of(&parsed);
    assert_eq!(body.replace("\r\n", "\n"), "one\ntwo\n\nthree");
    let ct = parsed.root_part().content_type().unwrap();
    assert_eq!(ct.attribute("format"), Some("flowed"));
}

#[test]
fn a_long_line_is_wrapped_with_soft_breaks_and_rejoins() {
    let long = vec!["lorem"; 60].join(" ");
    let raw = render(&message(&long));
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let body = body_of(&parsed);
    assert!(body.split("\r\n").count() > 1);
    assert!(body.split("\r\n").all(|l| l.len() <= 78));
    assert_eq!(unflow(&body), long);
}

#[test]
fn a_quoted_reply_keeps_its_prefixes() {
    let body = "thanks\n\nOn Mon, Ana wrote:\n> first\n> second line\n>\n>> older\n\n-- \nMe";
    let raw = render(&message(body));
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let got = body_of(&parsed).replace("\r\n", "\n");
    assert_eq!(got, body);
}

#[test]
fn the_signature_separator_keeps_its_trailing_space() {
    let raw = render(&message("hello\n-- \nMe"));
    let text = String::from_utf8_lossy(&raw).to_string();
    assert!(text.contains("\r\n-- \r\nMe"), "{text}");
}

#[test]
fn no_line_goes_over_998_bytes() {
    let raw = render(&message(&"x".repeat(5000)));
    for l in raw.split(|b| *b == b'\n') {
        assert!(l.len() <= 999, "{}", l.len());
    }
    let parsed = MessageParser::default().parse(&raw).unwrap();
    assert_eq!(unflow(&body_of(&parsed)).replace(' ', ""), "x".repeat(5000));
}
