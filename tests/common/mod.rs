#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const GM_IMAP: u16 = 3143;
pub const GM_IMAPS: u16 = 3993;
pub const GM_SMTP: u16 = 3025;
pub const GM_SMTPS: u16 = 3465;
pub const GM_API: u16 = 8080;
pub const DC_IMAP: u16 = 3144;
pub const DC_IMAPS: u16 = 3994;

pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn greenmail_ready() -> bool {
    Command::new("curl")
        .args(["-sf", "-m", "2", &format!("localhost:{GM_API}/api/service/readiness")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && imap_greeting(GM_IMAP)
        && smtp_greeting(GM_SMTP)
}

fn read_greeting(port: u16, prefix: &[u8]) -> bool {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 64];
    matches!(s.read(&mut buf), Ok(n) if n >= prefix.len() && buf.starts_with(prefix))
}

fn imap_greeting(port: u16) -> bool {
    read_greeting(port, b"* OK")
}

fn smtp_greeting(port: u16) -> bool {
    read_greeting(port, b"220")
}

fn dovecot_ready() -> bool {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", DC_IMAP)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut buf = [0u8; 256];
    let Ok(n) = s.read(&mut buf) else {
        return false;
    };
    if !buf[..n].starts_with(b"* OK") {
        return false;
    }
    let _ = s.write_all(b"a CAPABILITY\r\n");
    let mut more = [0u8; 512];
    let m = s.read(&mut more).unwrap_or(0);
    let mut all = buf[..n].to_vec();
    all.extend_from_slice(&more[..m]);
    String::from_utf8_lossy(&all).contains("STARTTLS") && TcpStream::connect(("127.0.0.1", DC_IMAPS)).is_ok()
}

fn wait(what: &str, ready: fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !ready() {
        assert!(Instant::now() < deadline, "{what} did not become ready");
        std::thread::sleep(Duration::from_millis(300));
    }
}

pub fn greenmail() -> bool {
    static LOCK: OnceLock<Mutex<bool>> = OnceLock::new();
    let mut ok = LOCK.get_or_init(|| Mutex::new(false)).lock().unwrap_or_else(|e| e.into_inner());
    if *ok {
        return true;
    }
    if !greenmail_ready() {
        if !docker_available() {
            eprintln!("docker missing, skipping greenmail test");
            return false;
        }
        let _ = Command::new("docker")
            .args([
                "run", "-d", "--rm", "--name", "mailbench-greenmail",
                "-p", "127.0.0.1:3025:3025", "-p", "127.0.0.1:3143:3143",
                "-p", "127.0.0.1:3465:3465", "-p", "127.0.0.1:3993:3993",
                "-p", "127.0.0.1:8080:8080",
                "-e", "GREENMAIL_OPTS=-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0",
                "greenmail/standalone:2.1.3",
            ])
            .output();
        wait("greenmail", greenmail_ready);
    }
    *ok = true;
    true
}

pub fn dovecot() -> bool {
    static LOCK: OnceLock<Mutex<bool>> = OnceLock::new();
    let mut ok = LOCK.get_or_init(|| Mutex::new(false)).lock().unwrap_or_else(|e| e.into_inner());
    if *ok {
        return true;
    }
    if !dovecot_ready() {
        if !docker_available() {
            eprintln!("docker missing, skipping dovecot test");
            return false;
        }
        let conf = format!(
            "{}/tests/fixtures/dovecot/dovecot.conf:/etc/dovecot/dovecot.conf:ro",
            env!("CARGO_MANIFEST_DIR")
        );
        let _ = Command::new("docker")
            .args([
                "run", "-d", "--rm", "--name", "mailbench-dovecot",
                "-p", "127.0.0.1:3144:143", "-p", "127.0.0.1:3994:993",
                "-v", &conf, "dovecot/dovecot:2.3-latest",
            ])
            .output();
        wait("dovecot", dovecot_ready);
    }
    *ok = true;
    true
}

pub fn create_greenmail_user(name: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let out = Command::new("curl")
            .args([
                "-s", "-m", "5", "-XPOST", &format!("localhost:{GM_API}/api/user"),
                "-H", "content-type: application/json",
                "-d", &format!(r#"{{"login":"{name}","email":"{name}@localhost","password":"secret"}}"#),
            ])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&out.stdout).contains(name) {
            return;
        }
        assert!(Instant::now() < deadline, "could not create greenmail user {name}");
        std::thread::sleep(Duration::from_millis(300));
    }
}
