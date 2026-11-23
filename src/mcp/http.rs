use super::MAX_BODY;
use super::rpc::{Reply, Rpc};
use super::tools::Tools;
use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MAX_HEAD: usize = 16 * 1024;
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub struct ServerConfig {
    pub token: String,
    pub port: u16,
    pub tools: Arc<dyn Tools>,
}

pub struct Server {
    pub addrs: Vec<SocketAddr>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

pub fn ensure_loopback(addr: &str) -> Result<IpAddr, String> {
    let host = addr.trim().trim_start_matches('[').trim_end_matches(']');
    let ip: IpAddr = host.parse().map_err(|_| format!("{addr} is not an IP address; the agent server only listens on 127.0.0.1 or ::1"))?;
    if ip.is_loopback() { Ok(ip) } else { Err(format!("refusing to listen on {addr}: the agent server only listens on 127.0.0.1 or ::1")) }
}

pub async fn bind_loopback(ip: IpAddr, port: u16) -> Result<TcpListener, String> {
    if !ip.is_loopback() {
        return Err(format!("refusing to listen on {ip}: the agent server only listens on 127.0.0.1 or ::1"));
    }
    TcpListener::bind(SocketAddr::new(ip, port)).await.map_err(|e| format!("could not listen on {ip}:{port}: {e}"))
}

struct Shared {
    token: String,
    rpc: Rpc,
}

impl Server {
    pub fn start(config: ServerConfig) -> Result<Server, String> {
        crate::runtime::block_on(Self::start_async(config))
    }

    pub async fn start_async(config: ServerConfig) -> Result<Server, String> {
        if config.token.is_empty() {
            return Err("an access token is required".into());
        }
        let primary = bind_loopback(IpAddr::V4(Ipv4Addr::LOCALHOST), config.port).await?;
        let port = primary.local_addr().map_err(|e| e.to_string())?.port();
        let mut listeners = vec![primary];
        if let Ok(v6) = bind_loopback(IpAddr::V6(Ipv6Addr::LOCALHOST), port).await {
            listeners.push(v6);
        }
        let shared = Arc::new(Shared { token: config.token, rpc: Rpc::new(config.tools) });
        let mut addrs = Vec::new();
        let mut tasks = Vec::new();
        for listener in listeners {
            addrs.push(listener.local_addr().map_err(|e| e.to_string())?);
            let shared = shared.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    let Ok((stream, peer)) = listener.accept().await else { continue };
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(READ_TIMEOUT * 4, serve(stream, shared)).await;
                    });
                }
            }));
        }
        Ok(Server { addrs, tasks })
    }

    pub fn port(&self) -> u16 {
        self.addrs.first().map(|a| a.port()).unwrap_or(0)
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u8;
    let longest = a.len().max(b.len());
    for i in 0..longest {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    std::hint::black_box(diff) == 0
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    fn status(status: u16) -> Response {
        Response { status, headers: Vec::new(), body: Vec::new() }
    }

    fn json(value: &Value) -> Response {
        Response {
            status: 200,
            headers: vec![("Content-Type", "application/json".into())],
            body: serde_json::to_vec(value).unwrap_or_default(),
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

enum ReadError {
    Closed,
    Reject(u16),
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, ReadError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if buf.len() > MAX_HEAD {
            return Err(ReadError::Reject(400));
        }
        let n = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut chunk)).await.map_err(|_| ReadError::Closed)?.map_err(|_| ReadError::Closed)?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(path)) = (first.next(), first.next()) else { return Err(ReadError::Reject(400)) };
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let mut request = Request { method: method.to_string(), path: path.to_string(), headers, body: Vec::new() };
    if request.header("transfer-encoding").is_some() {
        return Err(ReadError::Reject(411));
    }
    let length = match request.header("content-length") {
        None => 0,
        Some(v) => v.parse::<usize>().map_err(|_| ReadError::Reject(400))?,
    };
    if length > MAX_BODY {
        return Err(ReadError::Reject(413));
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut chunk)).await.map_err(|_| ReadError::Closed)?.map_err(|_| ReadError::Closed)?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > MAX_BODY {
            return Err(ReadError::Reject(413));
        }
    }
    body.truncate(length);
    request.body = body;
    Ok(request)
}

fn local_host(value: &str) -> bool {
    let host = if let Some(rest) = value.strip_prefix('[') {
        match rest.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => format!("[{h}]"),
            _ => return false,
        }
    } else {
        value.split(':').next().unwrap_or_default().to_string()
    };
    matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

fn local_origin(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("http://") else { return false };
    local_host(rest.trim_end_matches('/'))
}

fn authorised(request: &Request, token: &str) -> bool {
    let Some(value) = request.header("authorization") else { return false };
    let Some((scheme, given)) = value.split_once(' ') else { return false };
    scheme.eq_ignore_ascii_case("bearer") && constant_time_eq(given.trim().as_bytes(), token.as_bytes())
}

fn route(request: Request, shared: Arc<Shared>) -> Response {
    let Some(host) = request.header("host") else { return Response::status(403) };
    if !local_host(host) {
        return Response::status(403);
    }
    if let Some(origin) = request.header("origin") {
        if !local_origin(origin) {
            return Response::status(403);
        }
    }
    if !authorised(&request, &shared.token) {
        return Response::status(401);
    }
    let path = request.path.split('?').next().unwrap_or_default();
    if path != "/mcp" {
        return Response::status(404);
    }
    let session = request.header("mcp-session-id").map(str::to_string);
    match request.method.as_str() {
        "POST" => match shared.rpc.handle(&request.body, session.as_deref()) {
            Reply::Json(value, new_session) => {
                let mut r = Response::json(&value);
                if let Some(s) = new_session {
                    r.headers.push(("Mcp-Session-Id", s));
                }
                r
            }
            Reply::Accepted => Response::status(202),
            Reply::BadRequest(_) => Response::status(400),
            Reply::UnknownSession => Response::status(404),
        },
        "DELETE" => match session {
            Some(s) if shared.rpc.end_session(&s) => Response::status(204),
            Some(_) => Response::status(404),
            None => Response::status(400),
        },
        _ => {
            let mut r = Response::status(405);
            r.headers.push(("Allow", "POST, DELETE".into()));
            r
        }
    }
}

async fn serve(mut stream: TcpStream, shared: Arc<Shared>) {
    let response = match read_request(&mut stream).await {
        Ok(request) => {
            let shared = shared.clone();
            tokio::task::spawn_blocking(move || route(request, shared)).await.unwrap_or_else(|_| Response::status(500))
        }
        Err(ReadError::Reject(status)) => Response::status(status),
        Err(ReadError::Closed) => return,
    };
    let mut out = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n", response.status, reason(response.status), response.body.len());
    for (k, v) in &response.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&response.body);
    let _ = stream.write_all(&bytes).await;
    let _ = stream.shutdown().await;
}
