use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::{ConvSpec, FakeStore};
use mailbench::mailstore::*;
use std::path::PathBuf;

const ME: &str = "mara.ilves@kestrel-instruments.example";
const LONG_TAIL: [&str; 6] = [
    "> Earlier in this thread we talked about the room.",
    "> The projector needs a long cable.",
    "> Parking is behind the building.",
    "> Lunch is not included.",
    "> Bring a laptop.",
    "> Doors open at nine.",
];

fn specs() -> Vec<ConvSpec> {
    vec![
        ConvSpec {
            from: "Ann Byrne",
            address: "ann@byrne.example",
            to: &["me"],
            when: "14:25",
            sent: "Thu 23 Nov 14:25:02",
            unread: false,
            attach: false,
            account: 0,
            paragraphs: &["Hi, can we meet on Friday to go over the needs request?"],
            quoted: &[],
        },
        ConvSpec {
            from: "me",
            address: ME,
            to: &["Ann Byrne"],
            when: "14:25",
            sent: "Thu 23 Nov 14:25:40",
            unread: false,
            attach: false,
            account: 0,
            paragraphs: &["On Thu, Ann Byrne wrote:\n> Hi, can we meet on Friday to go over the needs request?", "Yes, Friday works for me."],
            quoted: &["> Hi, can we meet on Friday to go over the needs request?"],
        },
        ConvSpec {
            from: "Ann Byrne",
            address: "ann@byrne.example",
            to: &["me"],
            when: "14:25",
            sent: "Thu 23 Nov 14:25:50",
            unread: false,
            attach: true,
            account: 0,
            paragraphs: &["Great, my notes are attached."],
            quoted: &[],
        },
        ConvSpec {
            from: "Ann Byrne",
            address: "ann@byrne.example",
            to: &["me"],
            when: "14:25",
            sent: "Thu 23 Nov 14:25:55",
            unread: false,
            attach: false,
            account: 0,
            paragraphs: &["Also one more thing about the room, it holds twelve people."],
            quoted: &LONG_TAIL,
        },
        ConvSpec {
            from: "Ann Byrne",
            address: "ann@byrne.example",
            to: &["me"],
            when: "14:26",
            sent: "Thu 23 Nov 14:26:30",
            unread: true,
            attach: false,
            account: 0,
            paragraphs: &["Also one more thing about the room, it holds twelve people.", "Room 4 is booked for Friday at ten."],
            quoted: &LONG_TAIL,
        },
    ]
}

fn harness_sized(store: FakeStore, size: egui::Vec2) -> Harness<'static, (bool, App)> {
    let mut app = App::with_store(Box::new(store));
    app.selected = 0;
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .build_ui_state(
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
    h.run();
    h.run();
    h.run();
    h
}

fn open_thread(size: egui::Vec2) -> Harness<'static, (bool, App)> {
    let mut h = harness_sized(FakeStore::conversation("Assesment of needs request", &specs()), size);
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    h.run();
    h
}

fn ids(h: &Harness<'static, (bool, App)>) -> Vec<u64> {
    let head = h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    h.state().1.store.thread(head).iter().map(|t| t.summary.id.0).collect()
}

fn card_top(h: &Harness<'static, (bool, App)>, label: &str) -> f32 {
    h.get_by_label_contains(label).rect().top()
}

#[test]
fn a_thread_reads_oldest_first_with_the_newest_unread_open() {
    let h = open_thread(egui::vec2(1280.0, 1000.0));
    let order = ids(&h);
    assert_eq!(order, vec![1, 2, 3, 4, 5]);
    let tops: Vec<f32> = ["Hi, can we meet", "Yes, Friday works", "Great, my notes", "Also one more thing"].iter().map(|l| card_top(&h, l)).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tops:?}");
    assert!(h.state().1.cards.is_open(5));
    for older in 1..=4 {
        assert!(!h.state().1.cards.is_open(older));
    }
    h.get_by_label_contains("Room 4 is booked");
}

#[test]
fn collapsed_cards_show_new_text_not_the_quoted_part() {
    let h = open_thread(egui::vec2(1280.0, 1000.0));
    let own = h.get_by_label_contains("(you)");
    let label = format!("{:?}", own);
    assert!(label.contains("Yes, Friday works for me."), "{label}");
    assert!(!label.contains("Hi, can we meet"), "{label}");
    let attachment = h.get_by_label_contains("Great, my notes are attached.");
    assert!(format!("{attachment:?}").contains("has attachment"));
}

#[test]
fn your_own_message_carries_the_you_mark_and_others_do_not() {
    let h = open_thread(egui::vec2(1280.0, 1000.0));
    assert_eq!(h.query_all_by_label_contains("(you)").count(), 1);
}

#[test]
fn same_sender_same_minute_cards_differ_by_snippet_and_full_time() {
    let h = open_thread(egui::vec2(1280.0, 1000.0));
    let a = format!("{:?}", h.get_by_label_contains("Great, my notes are attached."));
    let b = format!("{:?}", h.get_by_label_contains("Also one more thing"));
    assert!(a.contains("14:25") && b.contains("14:25"));
    assert_ne!(a, b);
    let head = h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    let thread = h.state().1.store.thread(head);
    assert_ne!(thread[2].sent, thread[3].sent);
    assert!(thread[2].sent.ends_with(":50") && thread[3].sent.ends_with(":55"));
}

#[test]
fn clicking_a_card_expands_it_in_place_without_moving_the_others() {
    let mut h = open_thread(egui::vec2(1280.0, 1000.0));
    let first = card_top(&h, "Hi, can we meet");
    let second = card_top(&h, "Yes, Friday works");
    h.get_by_label_contains("Great, my notes").click();
    h.run();
    h.run();
    assert!(h.state().1.cards.is_open(3) && h.state().1.cards.is_open(5));
    assert_eq!(card_top(&h, "Hi, can we meet"), first);
    assert_eq!(card_top(&h, "Yes, Friday works"), second);
    h.get_by_label_contains("my notes are attached");
}

#[test]
fn expand_all_and_collapse_all_buttons_and_keys() {
    let mut h = open_thread(egui::vec2(1280.0, 1000.0));
    h.get_by_label("expand all").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.cards.open.len(), 5);
    h.get_by_label("collapse all").click();
    h.run();
    h.run();
    assert!(h.state().1.cards.open.is_empty());
    h.key_press(egui::Key::Period);
    h.run();
    h.run();
    assert_eq!(h.state().1.cards.open.len(), 5);
    h.key_press(egui::Key::Comma);
    h.run();
    h.run();
    assert!(h.state().1.cards.open.is_empty());
}

#[test]
fn the_choice_is_kept_while_the_conversation_stays_selected() {
    let mut h = open_thread(egui::vec2(1280.0, 1000.0));
    h.get_by_label("expand all").click();
    h.run();
    h.run();
    for _ in 0..3 {
        h.run();
    }
    assert_eq!(h.state().1.cards.open.len(), 5);
}

#[test]
fn j_k_and_enter_move_between_and_toggle_cards() {
    let mut h = open_thread(egui::vec2(1280.0, 1000.0));
    h.key_press(egui::Key::K);
    h.run();
    assert_eq!(h.state().1.thread_focus.map(|i| i.0), Some(4));
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert!(h.state().1.cards.is_open(4));
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert!(!h.state().1.cards.is_open(4));
    h.key_press(egui::Key::J);
    h.run();
    assert_eq!(h.state().1.thread_focus.map(|i| i.0), Some(5));
}

#[test]
fn a_reply_does_not_repeat_the_parents_text() {
    let h = open_thread(egui::vec2(1280.0, 1000.0));
    assert_eq!(h.query_all_by_label_contains("Also one more thing").count(), 1);
    h.get_by_label_contains("Room 4 is booked");
}

#[test]
fn an_expanded_reply_hides_the_attribution_and_the_parents_quoted_lines() {
    let mut h = open_thread(egui::vec2(1280.0, 1400.0));
    h.get_by_label("expand all").click();
    h.run();
    h.run();
    assert_eq!(h.query_all_by_label_contains("Hi, can we meet on Friday").count(), 1);
    assert_eq!(h.query_all_by_label_contains("wrote:").count(), 0);
    h.get_by_label_contains("message body: Yes, Friday works for me.");
}

#[test]
fn quoted_tail_stays_folded_until_asked() {
    let mut h = open_thread(egui::vec2(1280.0, 1000.0));
    h.get_by_label_contains("quoted lines hidden").click();
    h.run();
    h.run();
    assert!(h.state().1.cards.quotes_shown(5));
    h.get_by_label_contains("quoted lines shown");
}

#[test]
fn only_the_expanded_visible_unread_card_is_marked_read_after_the_dwell() {
    let mut h = harness_sized(FakeStore::conversation("Assesment of needs request", &specs()), egui::vec2(1280.0, 1000.0));
    let unread = |h: &Harness<'static, (bool, App)>, id: u64| h.state().1.store.get(MessageId(id)).unwrap().summary.unread;
    assert!(unread(&h, 5));
    std::thread::sleep(std::time::Duration::from_millis(1200));
    h.run();
    h.run();
    assert!(!unread(&h, 5));
}

#[test]
fn collapsed_unread_cards_stay_unread() {
    let mut specs = specs();
    specs[2].unread = true;
    let mut h = harness_sized(FakeStore::conversation("Assesment of needs request", &specs), egui::vec2(1280.0, 1000.0));
    h.key_press(egui::Key::Enter);
    h.run();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    h.run();
    h.run();
    assert!(h.state().1.store.get(MessageId(3)).unwrap().summary.unread);
    assert!(!h.state().1.store.get(MessageId(5)).unwrap().summary.unread);
}

#[test]
fn long_snippets_end_with_an_ellipsis_inside_the_card_at_phone_width() {
    let h = open_thread(egui::vec2(390.0, 844.0));
    let _ = &h;
    let long = "Also one more thing about the room, it holds twelve people and the projector needs a very long cable";
    let mut width = 0.0;
    let mut text = String::new();
    let mut harness = Harness::new_ui(|ui| {
        mailbench::kit::setup(ui.ctx());
        let avail = 390.0 - 24.0 - 24.0 - 50.0;
        let g = mailbench::kit::fit_text(ui, long, mailbench::kit::body(12.5), egui::Color32::WHITE, avail);
        width = g.size().x;
        text = g.text().to_string();
        assert!(width <= avail, "{width} > {avail}");
    });
    harness.run();
    harness.run();
    drop(harness);
    assert!(text.ends_with('\u{2026}'), "{text}");
    assert!(width > 100.0);
}

#[test]
fn phone_width_cards_and_buttons_are_tappable() {
    let h = open_thread(egui::vec2(390.0, 844.0));
    for label in ["Hi, can we meet", "Yes, Friday works", "Great, my notes"] {
        let r = h.get_by_label_contains(label).rect();
        assert!(r.height() >= 44.0, "{label} is {} high", r.height());
        assert!(r.right() <= 390.0 && r.left() >= 0.0, "{label} at {r:?}");
    }
    for label in ["expand all", "collapse all", "reply"] {
        let r = h.get_by_label(label).rect();
        assert!(r.height() >= 44.0, "{label} is {} high", r.height());
        assert!(r.right() <= 390.0 && r.left() >= 0.0, "{label} at {r:?}");
    }
    let menus: Vec<_> = h.query_all_by_label_contains("actions for").collect();
    assert!(!menus.is_empty());
    for m in menus {
        let r = m.rect();
        assert!(r.width() >= 44.0 && r.height() >= 44.0, "{r:?}");
    }
}

#[test]
fn n_and_p_step_through_the_message_list_while_the_reader_has_focus() {
    let mut h = harness_sized(FakeStore::concept(), egui::vec2(1280.0, 1000.0));
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert!(h.state().1.reader_focus);
    let start = h.state().1.selected;
    h.key_press(egui::Key::N);
    h.run();
    assert_eq!(h.state().1.selected, start + 1);
    assert!(h.state().1.reader_focus);
    h.key_press(egui::Key::P);
    h.run();
    assert_eq!(h.state().1.selected, start);
}

fn shot(h: &mut Harness<'static, (bool, App)>, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
    let img = h.render().expect("render");
    img.save(dir.join(name)).unwrap();
}

#[test]
#[ignore]
fn render_thread_screens() {
    let mut h = open_thread(egui::vec2(1280.0, 800.0));
    shot(&mut h, "thread-collapsed.png");
    h.get_by_label("expand all").click();
    h.run();
    h.run();
    shot(&mut h, "thread-expanded.png");
    let mut p = open_thread(egui::vec2(390.0, 844.0));
    shot(&mut p, "thread-phone.png");
}
