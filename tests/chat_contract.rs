use mailbench::chat::*;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

async fn two_contacts(b: &mut dyn ChatBackend) -> Vec<MemberId> {
    let contacts = b.contacts().await.unwrap();
    assert!(contacts.len() >= 2, "the backend must offer at least two contacts to run the contract");
    contacts.iter().take(2).map(|c| c.id.clone()).collect()
}

async fn next_event(rx: &mut UnboundedReceiver<ChatEvent>, want: impl Fn(&ChatEvent) -> bool) -> ChatEvent {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("event in time").expect("stream open");
        if want(&ev) {
            return ev;
        }
    }
}

pub async fn contract(b: &mut dyn ChatBackend) {
    let me = b.me();
    let caps = b.capabilities();
    let people = two_contacts(b).await;
    let mut rx = b.subscribe().await.unwrap();

    let room = b.create_room("Contract room", &people).await.unwrap();
    assert_eq!(room.name, "Contract room");
    assert_eq!(room.members.len(), 3);
    assert!(room.member(&me.id).is_some());
    assert!(b.rooms().await.unwrap().iter().any(|r| r.id == room.id));
    assert!(b.history(&room.id, None, 50).await.unwrap().is_empty());

    let sent = b.send(&room.id, &ChatDraft::text("first")).await.unwrap();
    assert_eq!(sent.sender, me.id);
    assert_eq!(sent.body, "first");
    assert_eq!(sent.room, room.id);
    assert!(matches!(sent.delivery, Delivery::Sent | Delivery::Delivered));
    let mut reply = ChatDraft::text("second");
    reply.reply_to = Some(sent.id.clone());
    let second = b.send(&room.id, &reply).await.unwrap();
    assert_eq!(second.reply_to, Some(sent.id.clone()));
    assert_ne!(sent.id, second.id);

    let all = b.history(&room.id, None, 50).await.unwrap();
    assert_eq!(all.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(), vec!["first", "second"]);

    let last = b.history(&room.id, None, 1).await.unwrap();
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].id, second.id);
    let before = b.history(&room.id, Some(&last[0].id), 1).await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].id, sent.id);
    assert!(b.history(&room.id, Some(&sent.id), 10).await.unwrap().is_empty());

    b.mark_read(&room.id, &second.id).await.unwrap();
    let rooms = b.rooms().await.unwrap();
    assert_eq!(rooms.iter().find(|r| r.id == room.id).unwrap().unread, 0);

    if caps.edit {
        b.edit(&room.id, &sent.id, "first, edited").await.unwrap();
        let ev = next_event(&mut rx, |e| matches!(e, ChatEvent::Edited { .. })).await;
        assert_eq!(ev, ChatEvent::Edited { room: room.id.clone(), id: sent.id.clone(), body: "first, edited".into() });
        let h = b.history(&room.id, None, 10).await.unwrap();
        assert!(h.iter().find(|m| m.id == sent.id).unwrap().edited);
    } else {
        assert!(matches!(b.edit(&room.id, &sent.id, "x").await, Err(ChatError::Unsupported(_))));
    }

    if caps.reactions {
        b.react(&room.id, &second.id, "\u{1F44D}", true).await.unwrap();
        let ev = next_event(&mut rx, |e| matches!(e, ChatEvent::Reaction { .. })).await;
        assert!(matches!(ev, ChatEvent::Reaction { added: true, .. }));
        let h = b.history(&room.id, None, 10).await.unwrap();
        assert!(h.iter().find(|m| m.id == second.id).unwrap().reacted("\u{1F44D}", &me.id));
        b.react(&room.id, &second.id, "\u{1F44D}", false).await.unwrap();
        let h = b.history(&room.id, None, 10).await.unwrap();
        assert!(h.iter().find(|m| m.id == second.id).unwrap().reactions.is_empty());
    } else {
        assert!(matches!(b.react(&room.id, &second.id, "x", true).await, Err(ChatError::Unsupported(_))));
    }

    let extra = b.contacts().await.unwrap().into_iter().map(|c| c.id).find(|id| !people.contains(id));
    if let Some(extra) = extra {
        b.add_members(&room.id, &[extra.clone()]).await.unwrap();
        let rooms = b.rooms().await.unwrap();
        assert!(rooms.iter().find(|r| r.id == room.id).unwrap().member(&extra).is_some());
    }

    b.set_topic(&room.id, "A topic").await.unwrap();
    let rooms = b.rooms().await.unwrap();
    assert_eq!(rooms.iter().find(|r| r.id == room.id).unwrap().topic, "A topic");

    if caps.delete {
        b.delete(&room.id, &second.id).await.unwrap();
        let h = b.history(&room.id, None, 10).await.unwrap();
        let gone = h.iter().find(|m| m.id == second.id).map_or(true, |m| m.deleted);
        assert!(gone);
    } else {
        assert!(matches!(b.delete(&room.id, &second.id).await, Err(ChatError::Unsupported(_))));
    }

    let missing = RoomId::new("no-such-room");
    assert!(b.send(&missing, &ChatDraft::text("x")).await.is_err());

    b.leave(&room.id).await.unwrap();
    assert!(b.rooms().await.unwrap().iter().all(|r| r.id != room.id));
    next_event(&mut rx, |e| matches!(e, ChatEvent::RoomLeft(id) if id == &room.id)).await;
}

#[tokio::test]
async fn the_fake_backend_passes_the_contract() {
    let (mut chat, _handle) = FakeChat::with_contacts();
    contract(&mut chat).await;
}

#[tokio::test]
async fn the_fake_backend_without_extras_passes_the_contract() {
    let (mut chat, handle) = FakeChat::with_contacts();
    handle.set_capabilities(Capabilities { paged_history: true, ..Default::default() });
    contract(&mut chat).await;
}

#[tokio::test]
async fn an_offline_backend_reports_offline_and_recovers() {
    let (mut chat, handle) = FakeChat::with_contacts();
    let room = handle.add_room("r", "Room", RoomKind::Group, &["ann", "bo"]);
    handle.set_offline(true);
    let err = chat.send(&room, &ChatDraft::text("x")).await.unwrap_err();
    assert_eq!(err, ChatError::Offline);
    assert!(err.retryable());
    handle.set_offline(false);
    chat.send(&room, &ChatDraft::text("x")).await.unwrap();
}

#[tokio::test]
async fn pushed_messages_arrive_on_the_stream() {
    let (mut chat, handle) = FakeChat::with_contacts();
    let room = handle.add_room("r", "Room", RoomKind::Group, &["ann", "bo"]);
    let mut rx = chat.subscribe().await.unwrap();
    handle.say(&room, "ann", "hello");
    let ev = next_event(&mut rx, |e| matches!(e, ChatEvent::NewMessage(_))).await;
    let ChatEvent::NewMessage(m) = ev else { unreachable!() };
    assert_eq!(m.body, "hello");
    assert_eq!(m.sender, MemberId::new("ann"));
}
