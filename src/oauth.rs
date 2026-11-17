use crate::autodetect::Provider;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct Client {
    pub id: String,
    pub secret: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Endpoints {
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
    pub redirect_host: String,
    pub extra: Vec<(String, String)>,
}

pub fn endpoints(provider: Provider) -> Endpoints {
    match provider {
        Provider::Google => Endpoints {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scope: "https://mail.google.com/".into(),
            redirect_host: "127.0.0.1".into(),
            extra: vec![("access_type".into(), "offline".into()), ("prompt".into(), "consent".into())],
        },
        Provider::Microsoft => Endpoints {
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            scope: "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access".into(),
            redirect_host: "localhost".into(),
            extra: vec![("prompt".into(), "select_account".into())],
        },
    }
}

pub fn client_for(provider: Provider) -> Option<Client> {
    let (id_var, secret_var, built_id, built_secret) = match provider {
        Provider::Google => (
            "MAILBENCH_GOOGLE_CLIENT_ID",
            "MAILBENCH_GOOGLE_CLIENT_SECRET",
            option_env!("MAILBENCH_GOOGLE_CLIENT_ID"),
            option_env!("MAILBENCH_GOOGLE_CLIENT_SECRET"),
        ),
        Provider::Microsoft => (
            "MAILBENCH_MICROSOFT_CLIENT_ID",
            "MAILBENCH_MICROSOFT_CLIENT_SECRET",
            option_env!("MAILBENCH_MICROSOFT_CLIENT_ID"),
            option_env!("MAILBENCH_MICROSOFT_CLIENT_SECRET"),
        ),
    };
    let id = std::env::var(id_var).ok().filter(|v| !v.is_empty()).or_else(|| built_id.map(str::to_string))?;
    let secret = std::env::var(secret_var).ok().filter(|v| !v.is_empty()).or_else(|| built_secret.map(str::to_string));
    Some(Client { id, secret })
}

pub fn missing_client_note(provider: Provider) -> String {
    let var = match provider {
        Provider::Google => "MAILBENCH_GOOGLE_CLIENT_ID",
        Provider::Microsoft => "MAILBENCH_MICROSOFT_CLIENT_ID",
    };
    format!(
        "This build has no {} OAuth client id, so sign in with {} is off. Set {var} and restart, or use an app password.",
        provider.name(),
        provider.name()
    )
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn pkce_from(verifier: &str) -> Pkce {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce { verifier: verifier.to_string(), challenge }
}

pub fn new_pkce() -> Pkce {
    let bytes: [u8; 32] = rand::random();
    pkce_from(&URL_SAFE_NO_PAD.encode(bytes))
}

pub fn new_state() -> String {
    let bytes: [u8; 16] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 + 0 => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn auth_url(ep: &Endpoints, client_id: &str, redirect: &str, state: &str, challenge: &str, hint: &str) -> String {
    let mut params = vec![
        ("client_id", client_id.to_string()),
        ("redirect_uri", redirect.to_string()),
        ("response_type", "code".to_string()),
        ("scope", ep.scope.clone()),
        ("state", state.to_string()),
        ("code_challenge", challenge.to_string()),
        ("code_challenge_method", "S256".to_string()),
    ];
    if !hint.is_empty() {
        params.push(("login_hint", hint.to_string()));
    }
    for (k, v) in &ep.extra {
        params.push((k.as_str(), v.clone()));
    }
    let query = params.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect::<Vec<_>>().join("&");
    format!("{}?{query}", ep.auth_url)
}

pub fn parse_callback(request_line: &str, expected_state: &str) -> Result<String, String> {
    let target = request_line.split_whitespace().nth(1).ok_or("empty request")?;
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "code" => code = Some(decode(v)),
            "state" => state = Some(decode(v)),
            "error_description" => error = Some(decode(v)),
            "error" if error.is_none() => error = Some(decode(v)),
            _ => {}
        }
    }
    if let Some(e) = error {
        return Err(format!("the provider refused the sign in: {e}"));
    }
    if state.as_deref() != Some(expected_state) {
        return Err("the sign in answer did not match this request".into());
    }
    code.filter(|c| !c.is_empty()).ok_or_else(|| "the sign in answer had no code".to_string())
}

pub fn parse_token_response(json: &str, previous_refresh: Option<&str>, now: i64) -> Result<OAuthTokens, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("bad token response: {e}"))?;
    if let Some(err) = value.get("error").and_then(|e| e.as_str()) {
        let detail = value.get("error_description").and_then(|d| d.as_str()).unwrap_or("");
        return Err(format!("{err} {detail}").trim().to_string());
    }
    let access = value.get("access_token").and_then(|v| v.as_str()).ok_or("no access token in the response")?;
    let refresh = value
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .or(previous_refresh)
        .ok_or("the provider sent no refresh token")?;
    let lifetime = value.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3600);
    Ok(OAuthTokens { access_token: access.to_string(), refresh_token: refresh.to_string(), expires_at: now + lifetime })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    match agent.post(url).send_form(form) {
        Ok(resp) => resp.into_string().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(_, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            match parse_token_response(&body, None, 0) {
                Err(e) => Err(e),
                Ok(_) => Err("token request failed".into()),
            }
        }
        Err(e) => Err(format!("could not reach the provider: {e}")),
    }
}

pub fn exchange(ep: &Endpoints, client: &Client, code: &str, verifier: &str, redirect: &str) -> Result<OAuthTokens, String> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", client.id.as_str()),
        ("code_verifier", verifier),
    ];
    if let Some(secret) = &client.secret {
        form.push(("client_secret", secret.as_str()));
    }
    let body = post_form(&ep.token_url, &form)?;
    parse_token_response(&body, None, now())
}

pub fn refresh(ep: &Endpoints, client: &Client, refresh_token: &str) -> Result<OAuthTokens, String> {
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client.id.as_str()),
    ];
    if let Some(secret) = &client.secret {
        form.push(("client_secret", secret.as_str()));
    }
    let body = post_form(&ep.token_url, &form)?;
    parse_token_response(&body, Some(refresh_token), now())
}

fn wait_for_request(listener: &TcpListener, timeout: Duration) -> Result<(TcpStream, String), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = text.lines().next().unwrap_or("").to_string();
                if line.starts_with("GET /favicon") || !line.contains('?') {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                return Ok((stream, line));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("timed out waiting for the browser".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn reply(mut stream: TcpStream, ok: bool) {
    let body = if ok {
        "<!doctype html><title>mailbench</title><body style=\"font-family:sans-serif;background:#17191d;color:#d5dbe3;padding:3em\"><h2>Signed in</h2><p>You can close this tab and go back to mailbench.</p>"
    } else {
        "<!doctype html><title>mailbench</title><body style=\"font-family:sans-serif;background:#17191d;color:#d5dbe3;padding:3em\"><h2>Sign in failed</h2><p>Go back to mailbench to see why.</p>"
    };
    let _ = stream.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
            .as_bytes(),
    );
}

pub fn sign_in(
    ep: &Endpoints,
    client: &Client,
    hint: &str,
    open: &dyn Fn(&str),
    timeout: Duration,
) -> Result<OAuthTokens, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://{}:{port}", ep.redirect_host);
    let pkce = new_pkce();
    let state = new_state();
    open(&auth_url(ep, &client.id, &redirect, &state, &pkce.challenge, hint));
    let (stream, line) = wait_for_request(&listener, timeout)?;
    match parse_callback(&line, &state) {
        Ok(code) => {
            let tokens = exchange(ep, client, &code, &pkce.verifier, &redirect);
            reply(stream, tokens.is_ok());
            tokens
        }
        Err(e) => {
            reply(stream, false);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        let p = pkce_from("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_verifier_is_long_enough() {
        let p = new_pkce();
        assert!(p.verifier.len() >= 43 && p.verifier.len() <= 128);
        assert_ne!(p.verifier, new_pkce().verifier);
    }

    #[test]
    fn auth_url_carries_pkce_state_and_loopback_redirect() {
        let ep = endpoints(Provider::Google);
        let url = auth_url(&ep, "cid", "http://127.0.0.1:5555", "st", "chal", "me@gmail.com");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        for part in [
            "client_id=cid",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555",
            "code_challenge=chal",
            "code_challenge_method=S256",
            "state=st",
            "login_hint=me%40gmail.com",
            "access_type=offline",
            "scope=https%3A%2F%2Fmail.google.com%2F",
        ] {
            assert!(url.contains(part), "{part} missing from {url}");
        }
    }

    #[test]
    fn callback_checks_state_and_reports_provider_errors() {
        assert_eq!(parse_callback("GET /?code=ab%2Fc&state=s1 HTTP/1.1", "s1").unwrap(), "ab/c");
        assert!(parse_callback("GET /?code=x&state=other HTTP/1.1", "s1").is_err());
        let err = parse_callback("GET /?error=access_denied&error_description=No+thanks HTTP/1.1", "s1").unwrap_err();
        assert!(err.contains("No thanks"), "{err}");
    }

    #[test]
    fn token_response_keeps_the_old_refresh_token_when_none_is_sent() {
        let t = parse_token_response(r#"{"access_token":"a","expires_in":100}"#, Some("old"), 1000).unwrap();
        assert_eq!(t, OAuthTokens { access_token: "a".into(), refresh_token: "old".into(), expires_at: 1100 });
        let t = parse_token_response(r#"{"access_token":"a","refresh_token":"new"}"#, Some("old"), 0).unwrap();
        assert_eq!(t.refresh_token, "new");
        let err = parse_token_response(r#"{"error":"invalid_grant","error_description":"expired"}"#, None, 0).unwrap_err();
        assert_eq!(err, "invalid_grant expired");
    }

    fn fake_token_server() -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/token", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let mut got = 0;
            loop {
                let n = s.read(&mut buf[got..]).unwrap();
                got += n;
                let text = String::from_utf8_lossy(&buf[..got]).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    if body.len() >= len {
                        let json = r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3600}"#;
                        let _ = s.write_all(
                            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len())
                                .as_bytes(),
                        );
                        return body.to_string();
                    }
                }
            }
        });
        (url, handle)
    }

    #[test]
    fn sign_in_runs_the_loopback_flow_and_exchanges_the_code() {
        let (token_url, server) = fake_token_server();
        let ep = Endpoints {
            auth_url: "http://auth.invalid/authorize".into(),
            token_url,
            scope: "mail".into(),
            redirect_host: "127.0.0.1".into(),
            extra: vec![],
        };
        let client = Client { id: "cid".into(), secret: None };
        let opener = |url: &str| {
            let query = url.split_once('?').unwrap().1;
            let get = |name: &str| {
                decode(query.split('&').find_map(|p| p.strip_prefix(&format!("{name}=")).map(str::to_string)).unwrap().as_str())
            };
            let redirect = get("redirect_uri");
            let state = get("state");
            assert_eq!(get("code_challenge_method"), "S256");
            std::thread::spawn(move || {
                let host = redirect.trim_start_matches("http://").to_string();
                let mut s = TcpStream::connect(&host).unwrap();
                write!(s, "GET /?code=thecode&state={state} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
                let mut sink = String::new();
                let _ = s.read_to_string(&mut sink);
            });
        };
        let tokens = sign_in(&ep, &client, "me@example.com", &opener, Duration::from_secs(10)).unwrap();
        assert_eq!(tokens.access_token, "at-1");
        assert_eq!(tokens.refresh_token, "rt-1");
        let posted = server.join().unwrap();
        assert!(posted.contains("code=thecode") && posted.contains("code_verifier="), "{posted}");
        assert!(posted.contains("grant_type=authorization_code"));
    }

    #[test]
    fn sign_in_times_out_without_a_browser() {
        let ep = Endpoints {
            auth_url: "http://auth.invalid/authorize".into(),
            token_url: "http://127.0.0.1:1/token".into(),
            scope: "mail".into(),
            redirect_host: "127.0.0.1".into(),
            extra: vec![],
        };
        let client = Client { id: "cid".into(), secret: None };
        let err = sign_in(&ep, &client, "", &|_| {}, Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }
}
