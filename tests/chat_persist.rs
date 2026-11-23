use mailbench::chat::*;
use mailbench::store::{KdfParams, Store, Unlock};
use std::path::Path;
use std::time::{Duration, Instant};

fn open(path: &Path) -> Store {
    if path.exists() {
        Store::open(path, &[Unlock::Password("pw")]).unwrap()
    } else {
        Store::create(path, None, Some(("pw", KdfParams::fast()))).unwrap()
    }
}

fn controller(path: &Path, chat: FakeChat) -> ChatController {
    let store = ChatStore::new(open(path), "acct");
    ChatController::start(Box::new(chat), Some(store))
}

fn wait(c: &mut ChatController, until: impl Fn(&ChatController) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        c.pump();
        if until(c) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("condition not reached");
}

#[test]
fn rooms_messages_read_markers_and_the_outbox_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let (chat, handle) = FakeChat::with_contacts();
    let room = handle.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo"]);
    handle.say(&room, "ann", "first");
    handle.say(&room, "bo", "second");
    let mut c = controller(&path, chat);
    wait(&mut c, |c| c.state.thread(&room).len() == 2);
    c.open = Some(room.clone());
    c.mark_read(&room);
    assert_eq!(c.state.room(&room).unwrap().unread, 0);
    handle.set_offline(true);
    wait(&mut c, |c| !c.online());
    c.send(&room, ChatDraft::text("queued while offline")).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    c.pump();
    assert_eq!(c.pending(), 1);
    drop(c);

    let (chat2, handle2) = FakeChat::with_contacts();
    handle2.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo"]);
    handle2.set_offline(true);
    let mut c2 = controller(&path, chat2);
    assert_eq!(c2.state.rooms.len(), 1, "rooms come back from the encrypted store before any backend answers");
    let bodies: Vec<String> = c2.state.thread(&room).iter().map(|m| m.body.clone()).collect();
    assert_eq!(bodies, vec!["first", "second", "queued while offline"]);
    assert_eq!(c2.state.thread(&room).last().unwrap().delivery, Delivery::Sending);
    assert!(c2.state.reads.contains_key(&room));
    assert_eq!(c2.state.room(&room).unwrap().unread, 0);
    assert_eq!(c2.pending(), 1);

    handle2.set_offline(false);
    wait(&mut c2, |c| c.pending() == 0 && c.state.thread(&room).iter().all(|m| !m.is_local()));
    assert_eq!(handle2.log(&room).iter().filter(|m| m.body == "queued while offline").count(), 1);
}

#[test]
fn the_failed_state_of_an_outbox_message_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let (chat, handle) = FakeChat::with_contacts();
    let room = handle.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo"]);
    let mut c = controller(&path, chat);
    wait(&mut c, |c| c.ready && c.state.room(&room).is_some());
    handle.fail_next_sends(1);
    c.send(&room, ChatDraft::text("rejected")).unwrap();
    wait(&mut c, |c| matches!(c.state.thread(&room).last().map(|m| m.delivery.clone()), Some(Delivery::Failed(_))));
    drop(c);
    let (chat2, handle2) = FakeChat::with_contacts();
    handle2.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo"]);
    let c2 = controller(&path, chat2);
    assert!(matches!(c2.state.thread(&room).last().unwrap().delivery, Delivery::Failed(_)));
    assert_eq!(c2.pending(), 0);
}

#[test]
fn a_database_from_before_chat_opens_and_gains_chat_records_without_touching_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let store = open(&path);
        store.put("account", "a1", b"{}").unwrap();
        store.put("message", "m1", b"mail").unwrap();
    }
    let (chat, handle) = FakeChat::with_contacts();
    let room = handle.add_room("crew", "Field crew", RoomKind::Group, &["ann"]);
    handle.say(&room, "ann", "hello");
    let mut c = controller(&path, chat);
    wait(&mut c, |c| c.state.thread(&room).len() == 1);
    drop(c);
    let store = open(&path);
    assert_eq!(store.get("message", "m1").unwrap().unwrap().as_slice(), b"mail");
    assert_eq!(store.get("account", "a1").unwrap().unwrap().as_slice(), b"{}");
    let (empty, _h) = FakeChat::empty();
    let c2 = ChatController::start(Box::new(empty), Some(ChatStore::new(store, "acct")));
    assert_eq!(c2.state.thread(&room).len(), 1);
}
