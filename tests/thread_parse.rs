use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::{ConvSpec, FakeStore};
use mailbench::real::display::parse_body;

const PARENT: &str = "Can we move the review to Thursday afternoon so Dana can join?";
const NEW: &str = "Thursday afternoon works, I will book the small room.";

fn raw(text: &str) -> Vec<u8> {
    format!("From: a@x.test\r\nTo: b@x.test\r\nSubject: Review\r\nMessage-ID: <1@x.test>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}").into_bytes()
}

fn spec(from: &'static str, address: &'static str, text: &str) -> ConvSpec {
    let body = parse_body(&raw(text));
    let leak = |v: Vec<String>| -> &'static [&'static str] {
        let items: Vec<&'static str> = v.into_iter().map(|s| &*Box::leak(s.into_boxed_str())).collect();
        Box::leak(items.into_boxed_slice())
    };
    ConvSpec {
        from,
        address,
        to: &["me"],
        when: "10:00",
        sent: "Mon 1 Jan 10:00:00",
        unread: false,
        attach: false,
        account: 0,
        paragraphs: leak(body.paragraphs),
        quoted: leak(body.quoted),
    }
}

fn thread_with(reply: &str) -> Harness<'static, (bool, App)> {
    let parent = spec("Dana Roe", "dana@x.test", PARENT);
    let child = spec("Kieran", "kieran@x.test", reply);
    let store = FakeStore::conversation("Review", &[parent, child]);
    let mut app = App::with_store(Box::new(store));
    app.selected = 0;
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 1400.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, app): &mut (bool, App)| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            app.show(ui);
        },
        (false, app),
    );
    for _ in 0..3 {
        h.run();
    }
    h.key_press(egui::Key::Enter);
    for _ in 0..3 {
        h.run();
    }
    h.get_by_label("expand all").click();
    h.run();
    h.run();
    h
}

fn assert_no_repeat(reply: &str) {
    let h = thread_with(reply);
    let sentence = "move the review to Thursday afternoon";
    assert_eq!(h.query_all_by_label_contains(sentence).count(), 1, "parent sentence shown more than once for {reply:?}");
    assert_eq!(h.query_all_by_label_contains("wrote:").count(), 0);
    assert_eq!(h.query_all_by_label_contains("Subject: Review").count(), 0);
    h.get_by_label_contains(&format!("message body: {NEW}"));
}

#[test]
fn a_plain_wrote_reply_with_quote_lines_shows_only_new_text() {
    assert_no_repeat(&format!("{NEW}\r\n\r\nOn Mon, 1 Jan 2024 at 09:00, Dana Roe <dana@x.test> wrote:\r\n> {PARENT}\r\n"));
}

#[test]
fn an_outlook_style_header_block_with_the_parent_below_shows_only_new_text() {
    assert_no_repeat(&format!(
        "{NEW}\r\n\r\nFrom: Dana Roe <dana@x.test>\r\nSent: Monday, 1 January 2024 09:00\r\nTo: Kieran\r\nSubject: Review\r\n\r\n{PARENT}\r\n"
    ));
}

#[test]
fn a_top_posted_reply_with_the_quote_below_shows_only_new_text() {
    assert_no_repeat(&format!("{NEW}\r\n\r\n> {PARENT}\r\n"));
}

#[test]
fn a_bottom_posted_reply_with_the_new_text_under_the_quote_shows_only_new_text() {
    assert_no_repeat(&format!("On Mon, 1 Jan 2024 at 09:00, Dana Roe <dana@x.test> wrote:\r\n> {PARENT}\r\n\r\n{NEW}\r\n"));
}

#[test]
fn the_quote_toggle_still_reveals_the_quoted_lines_of_an_unrelated_quote() {
    let h = thread_with(&format!("{NEW}\r\n\r\n> Something Dana never wrote in this thread at all.\r\n"));
    h.get_by_label_contains("quoted lines hidden");
}
