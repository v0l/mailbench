use mailbench::mcp::tools::Unimplemented;
use mailbench::mcp::{Server, ServerConfig, ensure_loopback};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

const TOKEN: &str = "test-token-0123456789";

fn start() -> Server {
    Server::start(ServerConfig { token: TOKEN.into(), port: 0, tools: Arc::new(Unimplemented) }).unwrap()
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

fn raw(port: u16, request: &[u8]) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let _ = s.write_all(request);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default().split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let headers = lines.filter_map(|l| l.split_once(": ")).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Reply { status, headers, body: body.to_string() }
}

struct Opts<'a> {
    host: &'a str,
    origin: Option<&'a str>,
    auth: Option<&'a str>,
    session: Option<&'a str>,
    method: &'a str,
}

impl Default for Opts<'_> {
    fn default() -> Self {
        Opts { host: "127.0.0.1", origin: None, auth: Some(TOKEN), session: None, method: "POST" }
    }
}

fn call(port: u16, o: &Opts, body: &[u8]) -> Reply {
    let mut head = format!("{} /mcp HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", o.method, o.host, body.len());
    if let Some(origin) = o.origin {
        head.push_str(&format!("Origin: {origin}\r\n"));
    }
    if let Some(a) = o.auth {
        head.push_str(&format!("Authorization: Bearer {a}\r\n"));
    }
    if let Some(s) = o.session {
        head.push_str(&format!("Mcp-Session-Id: {s}\r\n"));
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    raw(port, &bytes)
}

fn rpc(id: u64, method: &str, params: Value) -> Vec<u8> {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string().into_bytes()
}

fn init(port: u16) -> String {
    let r = call(port, &Opts::default(), &rpc(1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}})));
    assert_eq!(r.status, 200, "{}", r.body);
    r.header("Mcp-Session-Id").unwrap().to_string()
}

#[test]
fn initialize_negotiates_version_and_issues_a_session() {
    let s = start();
    let r = call(s.port(), &Opts::default(), &rpc(1, "initialize", json!({"protocolVersion": "2024-11-05"})));
    assert_eq!(r.status, 200);
    assert!(r.header("Mcp-Session-Id").is_some());
    let v = r.json();
    assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    assert!(v["result"]["instructions"].as_str().unwrap().contains("untrusted"));
    let r = call(s.port(), &Opts::default(), &rpc(1, "initialize", json!({"protocolVersion": "1999-01-01"})));
    assert_eq!(r.json()["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn tools_list_names_are_exactly_the_read_tools_and_never_delete_or_send_email() {
    let s = start();
    let session = init(s.port());
    let o = Opts { session: Some(&session), ..Default::default() };
    let r = call(s.port(), &o, &rpc(2, "tools/list", json!({})));
    let names: Vec<String> = r.json()["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["list_accounts", "list_folders", "search_messages", "get_message", "get_thread", "get_attachment"]);
    for t in r.json()["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[test]
fn ping_notifications_and_sessions() {
    let s = start();
    let session = init(s.port());
    let o = Opts { session: Some(&session), ..Default::default() };
    assert_eq!(call(s.port(), &o, &rpc(3, "ping", json!({}))).json()["result"], json!({}));
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
    assert_eq!(call(s.port(), &o, note.as_bytes()).status, 202);
    let none = Opts::default();
    assert_eq!(call(s.port(), &none, &rpc(4, "ping", json!({}))).status, 400);
    let wrong = Opts { session: Some("nope"), ..Default::default() };
    assert_eq!(call(s.port(), &wrong, &rpc(4, "ping", json!({}))).status, 404);
    let del = Opts { session: Some(&session), method: "DELETE", ..Default::default() };
    assert_eq!(call(s.port(), &del, b"").status, 204);
    assert_eq!(call(s.port(), &o, &rpc(5, "ping", json!({}))).status, 404);
}

#[test]
fn get_is_not_allowed() {
    let s = start();
    let o = Opts { method: "GET", ..Default::default() };
    assert_eq!(call(s.port(), &o, b"").status, 405);
}

#[test]
fn a_wrong_or_missing_token_gets_401_with_no_body() {
    let s = start();
    for auth in [Some("wrong"), Some("test-token-0123456789x"), Some(""), None] {
        let o = Opts { auth, ..Default::default() };
        let r = call(s.port(), &o, &rpc(1, "initialize", json!({})));
        assert_eq!(r.status, 401, "{auth:?}");
        assert!(r.body.is_empty());
    }
}

#[test]
fn a_foreign_host_header_is_refused() {
    let s = start();
    for host in ["evil.example", "evil.example:7480", "127.0.0.1.evil.example", "localhost.evil.example", ""] {
        let o = Opts { host, ..Default::default() };
        assert_eq!(call(s.port(), &o, &rpc(1, "initialize", json!({}))).status, 403, "{host}");
    }
    for host in ["localhost", "localhost:7480", "127.0.0.1:1", "[::1]", "[::1]:7480"] {
        let o = Opts { host, ..Default::default() };
        assert_eq!(call(s.port(), &o, &rpc(1, "initialize", json!({}))).status, 200, "{host}");
    }
}

#[test]
fn a_foreign_origin_is_refused() {
    let s = start();
    for origin in ["https://evil.example", "http://evil.example", "null", "http://localhost.evil.example", "https://localhost"] {
        let o = Opts { origin: Some(origin), ..Default::default() };
        assert_eq!(call(s.port(), &o, &rpc(1, "initialize", json!({}))).status, 403, "{origin}");
    }
    let ok = Opts { origin: Some("http://localhost:7480"), ..Default::default() };
    assert_eq!(call(s.port(), &ok, &rpc(1, "initialize", json!({}))).status, 200);
}

#[test]
fn a_body_over_one_megabyte_is_refused() {
    let s = start();
    let big = vec![b' '; 1_000_001];
    let r = call(s.port(), &Opts::default(), &big);
    assert_eq!(r.status, 413);
    let fine = vec![b' '; 900_000];
    assert_ne!(call(s.port(), &Opts::default(), &fine).status, 413);
}

#[test]
fn no_cors_headers_are_sent() {
    let s = start();
    let r = call(s.port(), &Opts::default(), &rpc(1, "initialize", json!({})));
    assert!(r.headers.iter().all(|(k, _)| !k.to_ascii_lowercase().starts_with("access-control")));
    let o = Opts { method: "OPTIONS", ..Default::default() };
    let r = call(s.port(), &o, b"");
    assert!(r.headers.iter().all(|(k, _)| !k.to_ascii_lowercase().starts_with("access-control")));
}

#[test]
fn only_loopback_addresses_are_accepted() {
    for ok in ["127.0.0.1", "::1", "[::1]", "127.0.0.2"] {
        assert!(ensure_loopback(ok).is_ok(), "{ok}");
    }
    for bad in ["0.0.0.0", "::", "192.168.1.5", "10.0.0.1", "example.com", "localhost", ""] {
        assert!(ensure_loopback(bad).is_err(), "{bad}");
    }
    let refused = mailbench::runtime::block_on(mailbench::mcp::bind_loopback("0.0.0.0".parse().unwrap(), 0));
    assert!(refused.is_err());
}

#[test]
fn the_server_listens_on_loopback_only() {
    let s = start();
    assert!(s.addrs.iter().all(|a| a.ip().is_loopback()));
}

#[test]
fn an_empty_token_cannot_start_a_server() {
    assert!(Server::start(ServerConfig { token: String::new(), port: 0, tools: Arc::new(Unimplemented) }).is_err());
}
