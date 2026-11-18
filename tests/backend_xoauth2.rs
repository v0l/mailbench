use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mailbench::backend::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

const EXPECTED: &str = "user=alice@example.com\x01auth=Bearer ya29.token\x01\x01";

fn auth() -> Auth {
    Auth::XOAuth2 { user: "alice@example.com".into(), access_token: "ya29.token".into() }
}

fn decoded(b64: &str) -> String {
    String::from_utf8(STANDARD.decode(b64.trim()).unwrap()).unwrap()
}

fn mock_imap() -> (u16, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut input = BufReader::new(stream);
        out.write_all(b"* OK mock ready\r\n").unwrap();
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).unwrap() == 0 {
                return;
            }
            let mut parts = line.trim().splitn(2, ' ');
            let tag = parts.next().unwrap().to_string();
            let cmd = parts.next().unwrap_or("").to_string();
            if cmd.starts_with("AUTHENTICATE XOAUTH2") {
                out.write_all(b"+ \r\n").unwrap();
                let mut resp = String::new();
                input.read_line(&mut resp).unwrap();
                tx.send(decoded(&resp)).unwrap();
                out.write_all(format!("{tag} OK authenticated\r\n").as_bytes()).unwrap();
            } else if cmd.starts_with("CAPABILITY") {
                out.write_all(format!("* CAPABILITY IMAP4rev1 AUTH=XOAUTH2\r\n{tag} OK done\r\n").as_bytes()).unwrap();
            } else {
                out.write_all(format!("{tag} OK done\r\n").as_bytes()).unwrap();
            }
        }
    });
    (port, rx)
}

fn mock_smtp() -> (u16, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut input = BufReader::new(stream);
        out.write_all(b"220 mock ESMTP\r\n").unwrap();
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).unwrap() == 0 {
                return;
            }
            let upper = line.to_uppercase();
            if upper.starts_with("EHLO") {
                out.write_all(b"250-mock\r\n250 AUTH XOAUTH2\r\n").unwrap();
            } else if upper.starts_with("AUTH XOAUTH2") {
                let initial = line.trim().splitn(3, ' ').nth(2).map(str::to_string);
                let b64 = match initial {
                    Some(v) => v,
                    None => {
                        out.write_all(b"334 \r\n").unwrap();
                        let mut resp = String::new();
                        input.read_line(&mut resp).unwrap();
                        resp
                    }
                };
                tx.send(decoded(&b64)).unwrap();
                out.write_all(b"235 2.7.0 ok\r\n").unwrap();
            } else if upper.starts_with("MAIL") || upper.starts_with("RCPT") || upper.starts_with("RSET") {
                out.write_all(b"250 ok\r\n").unwrap();
            } else if upper.starts_with("DATA") {
                out.write_all(b"354 go\r\n").unwrap();
                let mut data = Vec::new();
                let mut byte = [0u8; 1];
                while !data.ends_with(b"\r\n.\r\n") {
                    if input.read(&mut byte).unwrap() == 0 {
                        return;
                    }
                    data.push(byte[0]);
                }
                out.write_all(b"250 queued\r\n").unwrap();
            } else if upper.starts_with("QUIT") {
                out.write_all(b"221 bye\r\n").unwrap();
                return;
            } else {
                out.write_all(b"250 ok\r\n").unwrap();
            }
        }
    });
    (port, rx)
}

#[test]
fn imap_xoauth2_sends_the_encoded_bearer_string() {
    let (port, rx) = mock_imap();
    let mut backend = ImapBackend::new(ImapConfig::new("127.0.0.1", port, Security::Plain, auth()));
    mailbench::runtime::block_on(backend.list_folders()).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), EXPECTED);
}

#[test]
fn smtp_xoauth2_sends_the_encoded_bearer_string() {
    let (port, rx) = mock_smtp();
    let mut backend = SmtpBackend::new(SmtpConfig::new("127.0.0.1", port, Security::Plain, auth()));
    let msg = OutgoingMessage {
        from: Address { name: None, address: "alice@example.com".into() },
        to: vec![Address { name: None, address: "bob@example.com".into() }],
        subject: "hi".into(),
        body: "hello".into(),
        ..Default::default()
    };
    mailbench::runtime::block_on(backend.send(&msg)).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), EXPECTED);
}

#[allow(dead_code)]
fn unused(_: TcpStream) {}
