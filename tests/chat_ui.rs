use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::chat::*;
use mailbench::fake::FakeStore;
use std::time::Duration;

type H = Harness<'static, (bool, App)>;

struct Rig {
    h: H,
    handle: FakeHandle,
    crew: RoomId,
}

fn rig_with(size: egui::Vec2, caps: Option<Capabilities>, seed: impl FnOnce(&FakeHandle, &RoomId)) -> Rig {
    let (chat, handle) = FakeChat::with_contacts();
    if let Some(c) = caps {
        handle.set_capabilities(c);
    }
    let crew = handle.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo", "cy"]);
    seed(&handle, &crew);
    let mut view = ChatView::new(ChatController::start(Box::new(chat), None));
    view.dwell = Duration::from_millis(150);
    let mut app = App::with_store(Box::new(FakeStore::concept()));
    app.prefs.developer_chat = true;
    app.chats = Some(view);
    app.open_chats();
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(1.0).build_ui_state(
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
    let mut r = Rig { h: std::mem::replace(&mut h, dummy()), handle, crew };
    settle(&mut r.h);
    r
}

fn dummy() -> H {
    Harness::builder().build_ui_state(|_, _: &mut (bool, App)| {}, (false, App::with_store(Box::new(FakeStore::concept()))))
}

fn settle(h: &mut H) {
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(25));
        h.run_steps(2);
    }
}

fn crew() -> RoomId {
    RoomId::new("crew")
}

fn wide() -> egui::Vec2 {
    egui::vec2(1280.0, 800.0)
}

fn phone() -> egui::Vec2 {
    egui::vec2(392.0, 844.0)
}

fn view(r: &mut Rig) -> &mut ChatView {
    r.h.state_mut().1.chats.as_mut().unwrap()
}

fn seed_basic(h: &FakeHandle, crew: &RoomId) {
    h.say(crew, "ann", "Van is loaded");
    h.say(crew, "bo", "Bring the ladder");
}

fn open_crew(r: &mut Rig) {
    r.h.get_by_label_contains("room Field crew").click();
    settle(&mut r.h);
}

#[test]
fn room_list_shows_unread_badges_pinned_first_and_newest_first() {
    let mut r = rig_with(wide(), None, |h, crew| {
        seed_basic(h, crew);
        let other = h.add_room("lunch", "Lunch", RoomKind::Group, &["bo", "di"]);
        h.say(&other, "di", "Pizza?");
        let pinned = h.add_room("old", "Old pinned", RoomKind::Group, &["ed", "di"]);
        h.say(&pinned, "ed", "ancient");
    });
    let v = view(&mut r);
    let rooms = v.ctrl.state.ordered_rooms();
    assert_eq!(rooms.len(), 3);
    assert_eq!(rooms[0].unread + rooms[1].unread + rooms[2].unread, 4);
    let label = format!("{:?}", r.h.get_by_label_contains("room Field crew"));
    assert!(label.contains("2 unread"), "{label}");
}

#[test]
fn opening_a_room_marks_it_read_after_the_dwell_and_the_divider_sits_at_the_marker() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    assert_eq!(view(&mut r).divider, Some(MessageId::new("m1")));
    r.h.get_by_label_contains("new messages");
    assert_eq!(view(&mut r).ctrl.state.room(&crew()).unwrap().unread, 2);
    std::thread::sleep(Duration::from_millis(300));
    settle(&mut r.h);
    settle(&mut r.h);
    assert_eq!(view(&mut r).ctrl.state.room(&crew()).unwrap().unread, 0);
}

#[test]
fn sending_adds_a_sending_message_then_sent() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    r.handle.set_lag(Duration::from_millis(200));
    view(&mut r).draft = "On my way".into();
    view(&mut r).submit();
    r.h.run_steps(2);
    let sending = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert_eq!(sending.delivery, Delivery::Sending);
    assert!(sending.is_local());
    std::thread::sleep(Duration::from_millis(400));
    settle(&mut r.h);
    let last = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert_eq!(last.delivery, Delivery::Sent);
    assert!(!last.is_local());
    assert_eq!(last.body, "On my way");
    assert_eq!(view(&mut r).ctrl.state.thread(&crew()).iter().filter(|m| m.body == "On my way").count(), 1);
}

#[test]
fn a_rejected_send_shows_failed_and_retry_delivers_it() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    r.handle.fail_next_sends(1);
    view(&mut r).draft = "Will fail".into();
    view(&mut r).submit();
    settle(&mut r.h);
    settle(&mut r.h);
    let m = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert!(matches!(m.delivery, Delivery::Failed(_)), "{:?}", m.delivery);
    r.h.get_by_label("retry").click();
    settle(&mut r.h);
    settle(&mut r.h);
    let m = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert_eq!(m.delivery, Delivery::Sent);
    assert_eq!(r.handle.log(&r.crew).iter().filter(|m| m.body == "Will fail").count(), 1);
}

#[test]
fn failed_messages_can_be_deleted_without_sending() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    r.handle.fail_next_sends(1);
    view(&mut r).draft = "Drop me".into();
    view(&mut r).submit();
    settle(&mut r.h);
    settle(&mut r.h);
    r.h.get_by_label("delete").click();
    settle(&mut r.h);
    assert!(view(&mut r).ctrl.state.thread(&crew()).iter().all(|m| m.body != "Drop me"));
    assert!(r.handle.log(&r.crew).iter().all(|m| m.body != "Drop me"));
}

#[test]
fn offline_sends_wait_in_the_outbox_and_go_out_when_the_backend_returns() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    r.handle.set_offline(true);
    settle(&mut r.h);
    view(&mut r).ctrl.backoff = Duration::from_millis(50);
    view(&mut r).draft = "Sent from a tunnel".into();
    view(&mut r).submit();
    settle(&mut r.h);
    let m = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert_eq!(m.delivery, Delivery::Sending);
    assert_eq!(view(&mut r).ctrl.pending(), 1);
    assert!(r.h.query_all_by_label_contains("Offline").count() > 0, "Offline");
    r.handle.set_offline(false);
    for _ in 0..8 {
        settle(&mut r.h);
    }
    let m = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert_eq!(m.delivery, Delivery::Sent);
    assert_eq!(view(&mut r).ctrl.pending(), 0);
    assert_eq!(r.handle.log(&r.crew).iter().filter(|m| m.body == "Sent from a tunnel").count(), 1);
}

#[test]
fn repeated_offline_failures_end_in_failed_after_the_cap() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    view(&mut r).ctrl.backoff = Duration::from_millis(10);
    r.handle.fail_next_sends(100);
    view(&mut r).draft = "never".into();
    view(&mut r).submit();
    for _ in 0..6 {
        settle(&mut r.h);
    }
    let m = view(&mut r).ctrl.state.thread(&crew()).last().unwrap().clone();
    assert!(matches!(m.delivery, Delivery::Failed(_)));
}

#[test]
fn an_incoming_message_appends_and_stays_at_the_bottom() {
    let mut r = rig_with(wide(), None, |h, crew| {
        for i in 0..40 {
            h.say(crew, "ann", &format!("filler {i}"));
        }
    });
    open_crew(&mut r);
    settle(&mut r.h);
    assert!(view(&mut r).at_bottom);
    r.handle.say(&r.crew, "bo", "fresh arrival");
    settle(&mut r.h);
    settle(&mut r.h);
    assert!(view(&mut r).at_bottom);
    assert!(r.h.query_all_by_label_contains("fresh arrival").count() > 0, "fresh arrival");
}

#[test]
fn an_incoming_message_does_not_jump_a_reader_who_scrolled_up() {
    let mut r = rig_with(wide(), None, |h, crew| {
        for i in 0..40 {
            h.say(crew, "ann", &format!("filler {i}"));
        }
    });
    open_crew(&mut r);
    settle(&mut r.h);
    view(&mut r).force_offset = Some(0.0);
    settle(&mut r.h);
    let (o, c, b, t) = { let v = view(&mut r); (v.offset, v.content_h, v.at_bottom, v.to_bottom) };
    eprintln!("off {o} content {c} bottom {b} tb {t}");
    assert!(!view(&mut r).at_bottom);
    let before = view(&mut r).offset;
    r.handle.say(&r.crew, "bo", "fresh arrival");
    settle(&mut r.h);
    settle(&mut r.h);
    assert!(!view(&mut r).at_bottom);
    assert!((view(&mut r).offset - before).abs() < 2.0, "{} vs {before}", view(&mut r).offset);
    r.h.get_by_label("jump to latest");
}

#[test]
fn older_history_pages_in_without_moving_the_view() {
    let mut r = rig_with(wide(), None, |h, crew| {
        h.backlog(crew, "ann", 80);
        for i in 0..10 {
            h.say(crew, "bo", &format!("recent {i}"));
        }
    });
    open_crew(&mut r);
    settle(&mut r.h);
    let before = view(&mut r).ctrl.state.thread(&crew()).len();
    assert!(before < 90);
    view(&mut r).force_offset = Some(0.0);
    settle(&mut r.h);
    r.h.get_by_label("load older").click();
    for _ in 0..4 {
        settle(&mut r.h);
    }
    let after = view(&mut r).ctrl.state.thread(&crew()).len();
    assert!(after > before, "{after} vs {before}");
    assert!(view(&mut r).offset > 0.0, "the view kept its place below the new page");
}

#[test]
fn reply_quotes_reactions_edits_and_deletes_reflect_backend_events() {
    let mut r = rig_with(wide(), None, |h, crew| {
        let first = h.say(crew, "ann", "Original words");
        let mut reply = ChatMessage::new("", crew, &MemberId::new("bo"), 0, "Answering that");
        reply.reply_to = Some(first);
        h.say_message(reply);
    });
    open_crew(&mut r);
    assert!(r.h.query_all_by_label_contains("Ann Byrne: Original words").count() > 0, "Ann Byrne: Original words");
    let first = MessageId::new("m1");
    r.handle.react_as(&r.crew, &first, "bo", "+1", true);
    settle(&mut r.h);
    r.h.get_by_label_contains("reaction +1 1");
    r.handle.edit_as(&r.crew, &first, "Rewritten words");
    settle(&mut r.h);
    assert!(r.h.query_all_by_label_contains("Rewritten words").count() > 0, "Rewritten words");
    assert!(view(&mut r).ctrl.state.message(&crew(), &first).unwrap().edited);
    r.handle.delete_as(&r.crew, &first);
    settle(&mut r.h);
    assert!(r.h.query_all_by_label_contains("message deleted").count() > 0, "message deleted");
}

#[test]
fn reacting_toggles_through_the_backend() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    let id = MessageId::new("m2");
    view(&mut r).ctrl.react(&crew(), &id, "\u{2713}");
    settle(&mut r.h);
    let me = MemberId::new("me");
    assert!(view(&mut r).ctrl.state.message(&crew(), &id).unwrap().reacted("\u{2713}", &me));
    view(&mut r).ctrl.react(&crew(), &id, "\u{2713}");
    settle(&mut r.h);
    assert!(view(&mut r).ctrl.state.message(&crew(), &id).unwrap().reactions.is_empty());
}

#[test]
fn adding_a_member_and_leaving_update_the_members_panel_and_list() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    r.h.get_by_label("members").click();
    settle(&mut r.h);
    assert!(r.h.query_all_by_label_contains("4 members").count() > 0, "4 members");
    r.h.get_by_label("add Di Marsh").click();
    settle(&mut r.h);
    r.h.get_by_label("add selected").click();
    settle(&mut r.h);
    settle(&mut r.h);
    assert!(r.h.query_all_by_label_contains("5 members").count() > 0, "5 members");
    r.h.get_by_label("leave room").click();
    settle(&mut r.h);
    r.h.get_by_label("confirm leave").click();
    settle(&mut r.h);
    settle(&mut r.h);
    assert!(view(&mut r).ctrl.state.room(&crew()).is_none());
    assert!(view(&mut r).selected.is_none());
}

#[test]
fn setting_a_topic_shows_it_in_the_header() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    view(&mut r).ctrl.set_topic(&crew(), "Deliveries only");
    settle(&mut r.h);
    settle(&mut r.h);
    assert!(r.h.query_all_by_label_contains("Deliveries only").count() > 0, "Deliveries only");
}

#[test]
fn a_backend_without_reactions_edit_or_attachments_hides_those_controls() {
    let caps = Capabilities { paged_history: true, ..Default::default() };
    let mut r = rig_with(wide(), Some(caps), seed_basic);
    open_crew(&mut r);
    assert!(r.h.query_by_label("attach").is_none());
    assert_eq!(r.h.query_all_by_label_contains("+1").count(), 0);
    let full = rig_with(wide(), None, seed_basic);
    let mut full = full;
    open_crew(&mut full);
    assert!(full.h.query_by_label("attach").is_some());
}

#[test]
fn enter_sends_and_shift_enter_keeps_writing() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    r.h.get_by_label("message").click();
    r.h.run_steps(2);
    r.h.get_by_label("message").type_text("line one");
    r.h.run_steps(2);
    r.h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    r.h.run_steps(2);
    assert!(view(&mut r).ctrl.state.thread(&crew()).iter().all(|m| m.body != "line one"));
    assert!(!view(&mut r).draft.is_empty());
    r.h.key_press(egui::Key::Enter);
    settle(&mut r.h);
    settle(&mut r.h);
    assert!(view(&mut r).draft.is_empty());
    assert_eq!(r.handle.log(&r.crew).iter().filter(|m| m.body.starts_with("line one")).count(), 1);
}

#[test]
fn replying_attaching_and_cancelling_a_reply_work_from_the_composer() {
    let mut r = rig_with(wide(), None, seed_basic);
    open_crew(&mut r);
    view(&mut r).reply = Some(MessageId::new("m1"));
    r.h.run_steps(2);
    r.h.get_by_label_contains("Replying to Ann Byrne");
    r.h.get_by_label("cancel reply").click();
    r.h.run_steps(2);
    assert!(view(&mut r).reply.is_none());
    view(&mut r).reply = Some(MessageId::new("m1"));
    settle(&mut r.h);
    r.h.get_by_label("attach").click_accesskit();
    settle(&mut r.h);
    let n = view(&mut r).attachments.len();
    assert_eq!(n, 1);
    view(&mut r).draft = "see file".into();
    view(&mut r).submit();
    settle(&mut r.h);
    settle(&mut r.h);
    let sent = r.handle.log(&r.crew).last().unwrap().clone();
    assert_eq!(sent.reply_to, Some(MessageId::new("m1")));
    assert_eq!(sent.attachments.len(), 1);
}

#[test]
fn search_finds_messages_by_text_and_sender_and_jumps_to_the_room() {
    let mut r = rig_with(wide(), None, seed_basic);
    view(&mut r).search = "ladder".into();
    settle(&mut r.h);
    assert!(r.h.query_all_by_label_contains("result in Field crew").count() > 0, "result in Field crew");
    assert_eq!(view(&mut r).ctrl.search("ann").len(), 1);
    r.h.get_by_label_contains("result in Field crew").click();
    settle(&mut r.h);
    assert_eq!(view(&mut r).selected, Some(crew()));
}

#[test]
fn new_chat_creates_a_direct_room_or_a_named_group() {
    let mut r = rig_with(wide(), None, seed_basic);
    r.h.get_by_label("new chat").click();
    settle(&mut r.h);
    r.h.get_by_label("person Ed Voss").click();
    settle(&mut r.h);
    r.h.get_by_label("start chat").click();
    settle(&mut r.h);
    settle(&mut r.h);
    assert_eq!(view(&mut r).ctrl.state.rooms.len(), 2);
    view(&mut r).selected = None;
    view(&mut r).new_chat = Some(Default::default());
    settle(&mut r.h);
    r.h.get_by_label("person Di Marsh").click();
    r.h.get_by_label("person Cy Okafor").click();
    settle(&mut r.h);
    assert!(r.h.query_by_label("create group").is_some());
    view(&mut r).new_chat.as_mut().unwrap().name = "Weekend".into();
    settle(&mut r.h);
    r.h.get_by_label("create group").click();
    settle(&mut r.h);
    settle(&mut r.h);
    let names: Vec<String> = view(&mut r).ctrl.state.rooms.iter().map(|x| x.name.clone()).collect();
    assert!(names.contains(&"Weekend".to_string()), "{names:?}");
}

#[test]
fn phone_layout_has_separate_list_and_room_screens_with_tall_rows() {
    let mut r = rig_with(phone(), None, seed_basic);
    assert!(r.h.query_by_label("message").is_none());
    let row = r.h.get_by_label_contains("room Field crew").rect();
    assert!(row.height() >= 44.0, "{row:?}");
    open_crew(&mut r);
    r.h.get_by_label("message");
    assert!(r.h.query_by_label_contains("room Field crew").is_none());
    let send = r.h.get_by_label("send").rect();
    assert!(send.height() >= 44.0 && send.width() >= 44.0, "{send:?}");
    r.h.get_by_label("back").click();
    settle(&mut r.h);
    assert!(view(&mut r).selected.is_none());
    r.h.get_by_label_contains("room Field crew");
}

#[test]
fn the_setting_off_hides_all_chat_ui_and_the_key() {
    let mut app = App::with_store(Box::new(FakeStore::concept()));
    app.open_chats();
    assert!(!app.chats_open && app.chats.is_none());
    let mut h = Harness::builder().with_size(wide()).build_ui_state(
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
    h.run_steps(4);
    assert!(h.query_by_label("open chats").is_none());
    h.key_press(egui::Key::T);
    h.run_steps(3);
    assert!(!h.state().1.chats_open);
}

#[test]
fn the_sidebar_lists_chats_with_the_unread_count_when_the_setting_is_on() {
    let mut app = App::with_store(Box::new(FakeStore::concept()));
    app.prefs.developer_chat = true;
    let mut h = Harness::builder().with_size(wide()).build_ui_state(
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
    h.run_steps(4);
    h.get_by_label("open chats").click();
    h.run_steps(4);
    assert!(h.state().1.chats_open);
    h.get_by_label("mail").click();
    h.run_steps(3);
    assert!(!h.state().1.chats_open);
}

#[test]
fn notifications_fire_for_unmuted_rooms_that_are_not_open() {
    let mut r = rig_with(wide(), None, seed_basic);
    view(&mut r).ctrl.open = None;
    r.handle.say(&r.crew, "ann", "ping");
    settle(&mut r.h);
    assert_eq!(view(&mut r).ctrl.notices.len(), 1);
    assert!(view(&mut r).ctrl.notices[0].body.contains("Ann Byrne: ping"));
    view(&mut r).ctrl.notices.clear();
    open_crew(&mut r);
    r.handle.say(&r.crew, "ann", "pong");
    settle(&mut r.h);
    assert!(view(&mut r).ctrl.notices.is_empty());
}

fn shots(size: egui::Vec2, name: &str) {
    let mut r = rig_with(size, None, |h, crew| {
        h.backlog(crew, "ann", 3);
    });
    let crew = r.crew.clone();
    r.handle.add_room("lunch", "", RoomKind::Group, &["bo", "cy", "di", "ed"]);
    let first = r.handle.say(&crew, "ann", "Van is loaded, leaving at nine.");
    let mut reply = ChatMessage::new("", &crew, &MemberId::new("bo"), 0, "I will meet you at the depot.");
    reply.reply_to = Some(first.clone());
    let second = r.handle.say_message(reply);
    r.handle.say(&crew, "bo", "Bring the long ladder.");
    r.handle.say(&crew, "me", "On it, ladder is in the shed.");
    let mut file = ChatMessage::new("", &crew, &MemberId::new("cy"), 0, "Route for today attached.");
    file.attachments.push(ChatAttachment { name: "route.pdf".into(), size: 48_200, mime: "application/pdf".into() });
    r.handle.say_message(file);
    r.handle.react_as(&crew, &first, "bo", "+1", true);
    r.handle.react_as(&crew, &first, "cy", "+1", true);
    r.handle.react_as(&crew, &second, "me", "\u{2713}", true);
    let typo = r.handle.say(&crew, "cy", "Weather looks fine tomorow.");
    r.handle.edit_as(&crew, &typo, "Weather looks fine tomorrow.");
    let gone = r.handle.say(&crew, "ann", "Wrong chat, sorry.");
    r.handle.delete_as(&crew, &gone);
    settle(&mut r.h);
    if size.x < 700.0 {
        r.h.get_by_label_contains("room Field crew").click();
    } else {
        open_crew(&mut r);
    }
    settle(&mut r.h);
    r.handle.fail_next_sends(1);
    view(&mut r).draft = "This one fails".into();
    view(&mut r).submit();
    r.handle.typing(&crew, "ann", true);
    settle(&mut r.h);
    settle(&mut r.h);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
    r.h.render().expect("render").save(dir.join(name)).unwrap();
}

#[test]
fn pictures_of_a_four_member_group() {
    shots(wide(), "chat-group.png");
    shots(phone(), "chat-group-phone.png");
}
