use mailbench::backend::*;
use nostr_relay_builder::MockRelay;
use nostr_sdk::prelude::*;
use std::io::{Read, Write};
use std::net::TcpListener;

fn secret(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

async fn backend(n: u8, relay: &str) -> NostrBackend {
    NostrBackend::new(NostrConfig::new(&secret(n), vec![relay.to_string()])).unwrap()
}

fn mail(from: &NostrBackend, to: &NostrBackend, subject: &str, body: &str, reply_to: Option<String>) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: None, address: from.address() },
        to: vec![Address { name: None, address: to.address() }],
        subject: subject.into(),
        body: body.into(),
        in_reply_to: reply_to,
        ..Default::default()
    }
}

async fn summaries(b: &mut NostrBackend, folder: &str) -> Vec<MessageSummary> {
    let state = b.open_folder(folder).await.unwrap();
    b.fetch_summaries(folder, &state.uids).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_accounts_exchange_dms_and_a_reply_threads() {
    let relay = MockRelay::run().await.unwrap();
    let url = relay.url().await.to_string();
    let mut alice = backend(1, &url).await;
    let mut bob = backend(2, &url).await;

    alice.send(&mail(&alice, &bob, "Plans", "lunch friday?", None)).await.unwrap();

    let inbox = summaries(&mut bob, "Inbox").await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].subject.as_deref(), Some("Plans"));
    assert_eq!(inbox[0].from[0].address, alice.address());
    let sent = summaries(&mut alice, "Sent").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message_id, inbox[0].message_id);
    assert!(summaries(&mut alice, "Inbox").await.is_empty());

    let raw = bob.fetch("Inbox", inbox[0].uid).await.unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("Subject: Plans"));

    let parent = inbox[0].message_id.clone();
    bob.send(&mail(&bob, &alice, "Re: Plans", "yes", parent.clone())).await.unwrap();
    let back = summaries(&mut alice, "Inbox").await;
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].in_reply_to, vec![parent.clone().unwrap()]);
    assert!(back[0].references.iter().any(|r| inbox[0].references.contains(r)));
    assert_eq!(summaries(&mut bob, "Sent").await.len(), 1);

    let again = summaries(&mut alice, "Inbox").await;
    assert_eq!(again.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_session_backfills_the_history_from_the_relay() {
    let relay = MockRelay::run().await.unwrap();
    let url = relay.url().await.to_string();
    let mut alice = backend(1, &url).await;
    let bob = backend(2, &url).await;
    alice.send(&mail(&alice, &bob, "One", "first", None)).await.unwrap();
    alice.send(&mail(&alice, &bob, "Two", "second", None)).await.unwrap();

    let mut fresh = backend(1, &url).await;
    assert_eq!(summaries(&mut fresh, "Sent").await.len(), 2);
}

fn serve_once(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}/photo.txt")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kind_15_file_opens_as_a_decrypted_attachment() {
    let relay = MockRelay::run().await.unwrap();
    let url = relay.url().await.to_string();
    let (key, nonce) = ([3u8; 32], [4u8; 12]);
    let file_url = serve_once(encrypt_file(&key, &nonce, b"attached words").unwrap());

    let alice = Keys::parse(&secret(1)).unwrap();
    let mut bob = backend(2, &url).await;
    let rumor = EventBuilder::new(Kind::from_u16(15), file_url)
        .tags([
            Tag::public_key(bob.public_key()),
            Tag::parse(["file-type", "text/plain"]).unwrap(),
            Tag::parse(["encryption-algorithm", "aes-gcm"]).unwrap(),
            Tag::parse(["decryption-key", &hex_encode(&key)]).unwrap(),
            Tag::parse(["decryption-nonce", &hex_encode(&nonce)]).unwrap(),
        ])
        .build(alice.public_key());
    let wrap = EventBuilder::gift_wrap(&alice, &bob.public_key(), rumor, []).await.unwrap();
    let client = Client::new(alice.clone());
    client.add_relay(url.as_str()).await.unwrap();
    client.connect().await;
    client.send_event(&wrap).await.unwrap();

    let inbox = summaries(&mut bob, "Inbox").await;
    assert_eq!(inbox.len(), 1);
    let raw = bob.fetch("Inbox", inbox[0].uid).await.unwrap();
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
    let attachments: Vec<_> = parsed.attachments().collect();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].decoded().as_ref(), b"attached words");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dm_body_word_is_found_by_search_after_sync() {
    use mailbench::store::{KdfParams, Store};
    let relay = MockRelay::run().await.unwrap();
    let url = relay.url().await.to_string();
    let mut alice = backend(1, &url).await;
    let mut bob = backend(2, &url).await;
    let long = format!("{} the quarterly xylophone invoice", "padding ".repeat(20));
    alice.send(&mail(&alice, &bob, "Plans", &long, None)).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut sink = StoreSink::new(&store, "nostr-acct", true);
    let report = sync_account(&mut bob, &mut sink).await.unwrap();
    assert_eq!(report.added, 1);
    let hits = sink.search_hits("xylophone").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "Inbox");
    assert!(sink.search_hits("nonexistentword").unwrap().is_empty());
}
