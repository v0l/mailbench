use crate::accounts::{self, AccountConfig, Credential, Step};
use crate::app::App;
use crate::autodetect::{self, Detected, Net, Provider, ServerSettings};
use crate::backend::Security;
use crate::kit::{form_footer, header_frame, pane_fill, title};
use crate::oauth;
use crate::real::RealStore;
use crate::store::{KdfParams, Keychain, Store, StoreError, Unlock, UnlockMethods};
use egui::Ui;
use egui_bench::prelude::*;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

const COLUMN: f32 = 540.0;
const MIN_PASSWORD: usize = 8;

#[derive(Clone)]
pub struct Env {
    pub net: Arc<dyn Net + Send + Sync>,
    pub opener: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Env {
    pub fn live() -> Env {
        Env {
            net: Arc::new(autodetect::Live),
            opener: Arc::new(|url| {
                let _ = open::that(url);
            }),
        }
    }
}

pub fn strength(password: &str) -> (f32, &'static str) {
    const COMMON: [&str; 8] = ["password", "12345678", "qwertyui", "letmein1", "iloveyou", "admin123", "welcome1", "password1"];
    if COMMON.contains(&password.to_lowercase().as_str()) {
        return (10.0, "weak");
    }
    let mut pool = 0.0f32;
    if password.chars().any(|c| c.is_ascii_lowercase()) {
        pool += 26.0;
    }
    if password.chars().any(|c| c.is_ascii_uppercase()) {
        pool += 26.0;
    }
    if password.chars().any(|c| c.is_ascii_digit()) {
        pool += 10.0;
    }
    if password.chars().any(|c| !c.is_ascii_alphanumeric()) {
        pool += 33.0;
    }
    let mut distinct: Vec<char> = password.chars().collect();
    distinct.sort_unstable();
    distinct.dedup();
    let effective = distinct.len().min(password.chars().count()) as f32 * 0.5 + password.chars().count() as f32 * 0.5;
    let bits = if pool == 0.0 { 0.0 } else { effective * pool.log2() };
    let label = if bits < 30.0 {
        "weak"
    } else if bits < 45.0 {
        "fair"
    } else if bits < 65.0 {
        "good"
    } else {
        "strong"
    };
    (bits, label)
}

fn strength_tint(label: &str) -> egui::Color32 {
    match label {
        "weak" => FAULT,
        "fair" => WARN,
        _ => OK,
    }
}

fn page(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    egui::Panel::top("shell-head").frame(header_frame()).show(ui, |ui| title(ui));
    egui::CentralPanel::no_frame().show(ui, |ui| {
        pane_fill(ui, CHASSIS);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width();
            let col = COLUMN.min(w - 24.0);
            ui.add_space(28.0);
            ui.horizontal_top(|ui| {
                ui.add_space(((w - col) / 2.0).max(12.0));
                ui.vertical(|ui| {
                    ui.set_width(col);
                    body(ui);
                });
            });
            ui.add_space(28.0);
        });
    });
}

fn submitted(ui: &Ui, r: &egui::Response) -> bool {
    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protect {
    Keychain,
    Password,
    Both,
}

pub struct FirstForm {
    pub mode: Protect,
    pub password: String,
    pub confirm: String,
    pub error: Option<String>,
    pub keychain_available: bool,
}

impl FirstForm {
    fn new(keychain_available: bool) -> Self {
        let mode = if keychain_available { Protect::Keychain } else { Protect::Password };
        FirstForm { mode, password: String::new(), confirm: String::new(), error: None, keychain_available }
    }

    fn wants_password(&self) -> bool {
        self.mode != Protect::Keychain
    }

    fn problem(&self) -> Option<&'static str> {
        if !self.wants_password() {
            return None;
        }
        if self.password.chars().count() < MIN_PASSWORD {
            return Some("Use at least 8 characters");
        }
        if self.password != self.confirm {
            return Some("The two passwords do not match");
        }
        None
    }

    fn show(&mut self, ui: &mut Ui) -> bool {
        let mut go = false;
        page(ui, |ui| {
            section(ui, "protect your mail", "one decision, made once", |ui| {
                note(
                    ui,
                    "Everything mailbench keeps on this machine is encrypted: messages, attachments, the search index, saved passwords and settings. Choose what unlocks it.",
                    VALUE,
                );
                ui.add_space(6.0);
                let keychain_available = self.keychain_available;
                ui.add_enabled_ui(keychain_available, |ui| {
                    ui.radio_value(&mut self.mode, Protect::Keychain, "Use the system keychain (recommended)");
                });
                if keychain_available {
                    hint(ui, "Unlocks silently from then on. There is no password to remember.");
                } else {
                    hint(ui, "No system keychain found. Start a Secret Service provider such as gnome-keyring or KeePassXC to use it.");
                }
                ui.add_space(4.0);
                ui.radio_value(&mut self.mode, Protect::Password, "Use a password");
                hint(ui, "You type it each time you start mailbench. Stretched with Argon2id.");
                ui.add_space(4.0);
                ui.add_enabled_ui(keychain_available, |ui| {
                    ui.radio_value(&mut self.mode, Protect::Both, "Use both");
                });
                hint(ui, "The keychain unlocks silently. The password is the fallback when the keychain is not there.");
                if self.wants_password() {
                    ui.add_space(8.0);
                    row(ui, "password", |ui| {
                        secret(ui, &mut self.password);
                    });
                    row(ui, "again", |ui| {
                        let r = secret(ui, &mut self.confirm);
                        if submitted(ui, &r) {
                            go = true;
                        }
                    });
                    if !self.password.is_empty() {
                        let (_, label) = strength(&self.password);
                        Line::new().legend("strength").gap(10.0).value(label).tint(strength_tint(label)).size(11.0).show(ui);
                    }
                    ui.add_space(4.0);
                    note(
                        ui,
                        "If you lose this password the local data cannot be opened. Your mail is still on the server and can be synced again. Local drafts and settings are lost.",
                        WARN,
                    );
                }
                if let Some(e) = &self.error {
                    ui.add_space(6.0);
                    status(ui, false, e);
                }
                if self.wants_password() {
                    if let Some(p) = self.problem() {
                        if !self.password.is_empty() || !self.confirm.is_empty() {
                            hint(ui, p);
                        }
                    }
                }
                form_footer(ui, |ui| {
                    if ui.add(Key::new("continue").solid(READOUT).min_width(120.0).enabled(self.problem().is_none())).clicked() {
                        go = true;
                    }
                });
            });
        });
        go
    }
}

pub struct Unlocking {
    rx: Receiver<Result<Store, StoreError>>,
}

pub struct UnlockForm {
    pub password: String,
    pub error: Option<String>,
    pub note: Option<String>,
    pub has_password: bool,
    focused: bool,
    working: Option<Unlocking>,
}

impl UnlockForm {
    fn new(error: Option<String>, note: Option<String>, has_password: bool) -> Self {
        UnlockForm { password: String::new(), error, note, has_password, focused: !has_password, working: None }
    }

    pub fn busy(&self) -> bool {
        self.working.is_some()
    }

    fn show(&mut self, ui: &mut Ui) -> bool {
        let mut go = false;
        let busy = self.working.is_some();
        page(ui, |ui| {
            section(ui, "unlock", "everything on disk is encrypted", |ui| {
                if let Some(n) = &self.note {
                    note(ui, n, WARN);
                    ui.add_space(4.0);
                }
                if self.has_password {
                    row(ui, "password", |ui| {
                        let r = ui.add_enabled_ui(!busy, |ui| secret(ui, &mut self.password)).inner;
                        if !self.focused {
                            r.request_focus();
                            self.focused = true;
                        }
                        if !busy && submitted(ui, &r) {
                            go = true;
                        }
                    });
                }
                if let Some(e) = &self.error {
                    status(ui, false, e);
                }
                if self.has_password {
                    form_footer(ui, |ui| {
                        if self.working.is_some() {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0, KEY_H), egui::Sense::hover());
                            let r = egui_bench::prelude::busy(ui, rect, None, "unlocking", READOUT);
                            ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("unlocking"));
                        } else if ui.add(Key::new("unlock").solid(READOUT).min_width(120.0).enabled(!self.password.is_empty())).clicked() {
                            go = true;
                        }
                    });
                }
            });
        });
        go
    }
}

#[derive(Default)]
pub struct TestRun {
    pub results: Vec<(Step, Result<(), String>)>,
    rx: Option<Receiver<(Step, Result<(), String>)>>,
    pub for_key: String,
    pub inbox_missing: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TestRun {
    fn start(&mut self, cfg: &AccountConfig, key: String, ctx: &egui::Context) {
        self.results.clear();
        self.for_key = key;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let cfg = cfg.clone();
        let ctx = ctx.clone();
        let missing = self.inbox_missing.clone();
        missing.store(false, std::sync::atomic::Ordering::SeqCst);
        std::thread::spawn(move || {
            let mut ok = true;
            accounts::run_checks(&cfg, |step, result| {
                ok &= result.is_ok();
                let _ = tx.send((step, result));
                ctx.request_repaint();
            });
            if ok && matches!(cfg.credential, Credential::Nostr { .. } | Credential::NostrBunker { .. }) && accounts::nostr_lacks_inbox_list(&cfg) {
                missing.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            ctx.request_repaint();
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(item) => self.results.push(item),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return;
                }
            }
        }
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    pub fn all_ok(&self) -> bool {
        Step::ALL.iter().all(|s| self.results.iter().any(|(r, res)| r == s && res.is_ok()))
    }

    fn show(&self, ui: &mut Ui, cfg: Option<&AccountConfig>, oauth_offered: bool) {
        if self.results.is_empty() && !self.running() {
            return;
        }
        for step in Step::ALL {
            match self.results.iter().find(|(s, _)| *s == step) {
                Some((_, Ok(()))) => status(ui, true, &format!("{}: connected and signed in", step.label())),
                Some((_, Err(e))) => {
                    status(ui, false, &format!("{}: {e}", step.label()));
                    if let Some(cfg) = cfg {
                        note(ui, accounts::fix_hint(cfg, e, oauth_offered), LEGEND);
                    }
                }
                None if self.running() => {
                    Line::new().measured(format!("{}: checking", step.label())).size(11.0).show(ui);
                }
                None => {
                    Line::new().note(format!("{}: not tested", step.label())).size(11.0).show(ui);
                }
            }
        }
    }
}

enum Detect {
    Idle,
    Running(Receiver<Option<Detected>>),
    Found(String),
    Missing,
}

pub struct AddForm {
    pub id: String,
    pub email: String,
    pub password: String,
    pub username: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    pub edit: bool,
    pub nostr: bool,
    pub publish_inbox: Option<bool>,
    pub relays: String,
    app_key: String,
    bunker_user: Option<(String, String)>,
    bunker_rx: Option<Receiver<(String, Result<String, String>)>>,
    bunker_error: Option<String>,
    pub provider: Option<Provider>,
    pub oauth: Option<(Provider, oauth::OAuthTokens)>,
    pub test: TestRun,
    pub first: bool,
    detect: Detect,
    detected_for: String,
    oauth_rx: Option<Receiver<Result<oauth::OAuthTokens, String>>>,
    oauth_error: Option<String>,
    pub saved_error: Option<String>,
}

pub enum AddEvent {
    None,
    Save(AccountConfig),
    Cancel,
}

fn sec_label(s: &Security) -> String {
    match s {
        Security::Tls => "TLS".into(),
        Security::StartTls => "STARTTLS".into(),
        Security::Plain => "none (not encrypted)".into(),
    }
}

fn sec_summary(s: &ServerSettings) -> String {
    format!("{}:{} {}", s.host, s.port, sec_label(&s.security))
}

impl AddForm {
    pub fn new(first: bool) -> Self {
        AddForm {
            id: accounts::new_id(),
            email: String::new(),
            password: String::new(),
            username: String::new(),
            imap_host: String::new(),
            imap_port: "993".into(),
            imap_security: Security::Tls,
            smtp_host: String::new(),
            smtp_port: "587".into(),
            smtp_security: Security::StartTls,
            edit: false,
            nostr: false,
            publish_inbox: None,
            relays: String::new(),
            app_key: crate::backend::new_app_key(),
            bunker_user: None,
            bunker_rx: None,
            bunker_error: None,
            provider: None,
            oauth: None,
            test: TestRun::default(),
            first,
            detect: Detect::Idle,
            detected_for: String::new(),
            oauth_rx: None,
            oauth_error: None,
            saved_error: None,
        }
    }

    fn is_bunker(&self) -> bool {
        self.password.trim().starts_with("bunker://")
    }

    fn start_bunker(&mut self, ctx: &egui::Context) {
        let url = self.password.trim().to_string();
        let app_key = self.app_key.clone();
        let (tx, rx) = channel();
        self.bunker_rx = Some(rx);
        self.bunker_error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::runtime::block_on(crate::backend::bunker_user(&url, &app_key)).map(|k| k.to_hex()).map_err(|e| e.to_string());
            let _ = tx.send((url, result));
            ctx.request_repaint();
        });
    }

    fn email(&self) -> String {
        self.email.trim().to_string()
    }

    pub fn config(&self) -> Result<AccountConfig, String> {
        if self.nostr {
            if self.password.trim().is_empty() {
                return Err("Enter your nsec or a bunker:// url".into());
            }
            let relays = self.relays.split([',', ' ', '\n']).map(str::trim).filter(|r| !r.is_empty()).map(String::from).collect();
            if self.is_bunker() {
                let url = self.password.trim().to_string();
                let user = match &self.bunker_user {
                    Some((for_url, user)) if *for_url == url => user.clone(),
                    _ => return Err("Connect to the bunker first".into()),
                };
                let mut cfg = accounts::nostr_bunker_account(&url, &self.app_key, &user, relays).map_err(|e| e.to_string())?;
                if let Credential::NostrBunker { publish_inbox, .. } = &mut cfg.credential {
                    *publish_inbox = self.publish_inbox == Some(true);
                }
                cfg.id = self.id.clone();
                return Ok(cfg);
            }
            let mut cfg = accounts::nostr_config(&self.password, relays).map_err(|_| "That is not a valid nsec".to_string())?;
            if let Credential::Nostr { publish_inbox, .. } = &mut cfg.credential {
                *publish_inbox = self.publish_inbox == Some(true);
            }
            cfg.id = self.id.clone();
            return Ok(cfg);
        }
        let email = self.email();
        if autodetect::split_address(&email).is_none() {
            return Err("Enter your email address".into());
        }
        let port = |text: &str, what: &str| text.trim().parse::<u16>().map_err(|_| format!("The {what} port must be a number"));
        if self.imap_host.trim().is_empty() || self.smtp_host.trim().is_empty() {
            return Err("Enter the server hosts under edit server settings".into());
        }
        let credential = match &self.oauth {
            Some((provider, tokens)) => Credential::OAuth { provider: *provider, tokens: tokens.clone() },
            None if self.password.is_empty() => return Err("Enter the password".into()),
            None => Credential::Password(self.password.clone()),
        };
        let username = if self.username.trim().is_empty() { email.clone() } else { self.username.trim().to_string() };
        let name = email.split('@').next().unwrap_or(&email).to_string();
        Ok(AccountConfig {
            id: self.id.clone(),
            name,
            display_name: String::new(),
            signature: String::new(),
            address: email,
            username,
            imap: ServerSettings::new(self.imap_host.trim(), port(&self.imap_port, "incoming")?, self.imap_security.clone()),
            smtp: ServerSettings::new(self.smtp_host.trim(), port(&self.smtp_port, "outgoing")?, self.smtp_security.clone()),
            credential,
            backfill: Default::default(),
            notify: Default::default(),
            sync: Default::default(),
        })
    }

    fn key(&self) -> String {
        if self.nostr {
            return format!("nostr\0{}\0{}", self.password.trim(), self.relays.trim());
        }
        match self.config() {
            Ok(c) => format!("{c:?}"),
            Err(e) => e,
        }
    }

    fn apply(&mut self, found: &Detected) {
        self.imap_host = found.imap.host.clone();
        self.imap_port = found.imap.port.to_string();
        self.imap_security = found.imap.security.clone();
        self.smtp_host = found.smtp.host.clone();
        self.smtp_port = found.smtp.port.to_string();
        self.smtp_security = found.smtp.security.clone();
        self.username = found.username.clone();
        self.provider = found.provider;
    }

    fn start_detect(&mut self, env: &Env, ctx: &egui::Context) {
        let email = self.email();
        if autodetect::split_address(&email).is_none() {
            return;
        }
        self.detected_for = email.clone();
        let (tx, rx) = channel();
        let net = env.net.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(autodetect::detect(&email, net.as_ref()));
            ctx.request_repaint();
        });
        self.detect = Detect::Running(rx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Detect::Running(rx) = &self.detect {
            match rx.try_recv() {
                Ok(Some(found)) => {
                    self.apply(&found);
                    self.detect = Detect::Found(found.source.describe());
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    let domain = self.email().rsplit('@').next().unwrap_or_default().to_string();
                    if self.imap_host.is_empty() {
                        self.imap_host = format!("imap.{domain}");
                        self.smtp_host = format!("smtp.{domain}");
                    }
                    self.provider = autodetect::provider_of_host(&domain);
                    self.edit = true;
                    self.detect = Detect::Missing;
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if let Some(rx) = &self.oauth_rx {
            match rx.try_recv() {
                Ok(Ok(tokens)) => {
                    if let Some(p) = self.provider {
                        self.oauth = Some((p, tokens));
                    }
                    self.oauth_rx = None;
                }
                Ok(Err(e)) => {
                    self.oauth_error = Some(e);
                    self.oauth_rx = None;
                }
                Err(TryRecvError::Disconnected) => self.oauth_rx = None,
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if let Some(rx) = &self.bunker_rx {
            match rx.try_recv() {
                Ok((url, Ok(user))) => {
                    self.bunker_user = Some((url, user));
                    self.bunker_rx = None;
                }
                Ok((_, Err(e))) => {
                    self.bunker_error = Some(e);
                    self.bunker_rx = None;
                }
                Err(TryRecvError::Disconnected) => self.bunker_rx = None,
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        self.test.poll(ctx);
        let key = self.key();
        if key != self.test.for_key && !self.test.running() {
            self.test.results.clear();
        }
    }

    fn start_oauth(&mut self, provider: Provider, env: &Env, ctx: &egui::Context) {
        let Some(client) = oauth::client_for(provider) else { return };
        let (tx, rx) = channel();
        let opener = env.opener.clone();
        let email = self.email();
        let ctx = ctx.clone();
        self.oauth_error = None;
        std::thread::spawn(move || {
            let result = oauth::sign_in(&oauth::endpoints(provider), &client, &email, &*opener, Duration::from_secs(300));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.oauth_rx = Some(rx);
    }

    fn server_rows(&mut self, ui: &mut Ui) {
        let options = || [Security::Tls, Security::StartTls, Security::Plain].map(|s| (s.clone(), sec_label(&s)));
        row(ui, "user name", |ui| {
            field(ui, &mut self.username, "usually your email address");
        });
        row(ui, "imap host", |ui| {
            field(ui, &mut self.imap_host, "imap.example.com");
        });
        row(ui, "imap port", |ui| {
            field(ui, &mut self.imap_port, "993");
        });
        row(ui, "imap security", |ui| {
            choice(ui, "imap-security", &mut self.imap_security, options());
        });
        row(ui, "smtp host", |ui| {
            field(ui, &mut self.smtp_host, "smtp.example.com");
        });
        row(ui, "smtp port", |ui| {
            field(ui, &mut self.smtp_port, "587");
        });
        row(ui, "smtp security", |ui| {
            choice(ui, "smtp-security", &mut self.smtp_security, options());
        });
    }

    fn show(&mut self, ui: &mut Ui, env: &Env) -> AddEvent {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let mut event = AddEvent::None;
        let mut detect = false;
        let mut run_test = false;
        let mut sign_in = None;
        page(ui, |ui| {
            section(ui, "add account", "your address first, the rest is looked up", |ui| {
                ui.checkbox(&mut self.nostr, "nostr account (NIP-17 private messages)");
                if self.nostr {
                    row(ui, "nsec or bunker", |ui| {
                        secret(ui, &mut self.password);
                    });
                    if self.is_bunker() {
                        let connected = matches!(&self.bunker_user, Some((u, _)) if *u == self.password.trim());
                        if let (true, Some((_, user))) = (connected, &self.bunker_user) {
                            let npub = nostr_sdk::PublicKey::from_hex(user).ok().and_then(|k| nostr_sdk::ToBech32::to_bech32(&k).ok()).unwrap_or_else(|| user.clone());
                            status(ui, true, &format!("Connected to the bunker as {npub}. No nsec is stored."));
                        } else if self.bunker_rx.is_some() {
                            hint(ui, "Waiting for the bunker to approve. Check your signer app.");
                        } else {
                            if ui.add(Key::new("connect bunker").outline(VALUE).min_width(130.0)).clicked() {
                                self.start_bunker(ui.ctx());
                            }
                            if let Some(e) = &self.bunker_error {
                                status(ui, false, e);
                            }
                        }
                    }
                    row(ui, "relays", |ui| {
                        field(ui, &mut self.relays, "wss://relay.example");
                    });
                    hint(ui, "Leave relays empty to use your kind 10050 list, then your NIP-65 relays, then the defaults.");
                    let parsed = self.config();
                    self.test.show(ui, parsed.as_ref().ok(), false);
                    if let Some(e) = &self.saved_error {
                        status(ui, false, e);
                    }
                    let asked = self.test.inbox_missing.load(std::sync::atomic::Ordering::SeqCst) && !self.test.running();
                    if asked && self.publish_inbox.is_none() {
                        status(ui, false, "Publish your DM relay list?");
                        hint(ui, "You have no kind 10050 list, so other people cannot tell where to send you messages.");
                        ui.horizontal(|ui| {
                            if ui.add(Key::new("publish relay list").solid(READOUT)).clicked() {
                                self.publish_inbox = Some(true);
                            }
                            if ui.add(Key::new("not now").outline(VALUE)).clicked() {
                                self.publish_inbox = Some(false);
                            }
                        });
                    }
                    let answered = !asked || self.publish_inbox.is_some();
                    let tested = self.test.all_ok() && self.test.for_key == self.key() && answered;
                    form_footer(ui, |ui| {
                        let ok = parsed.is_ok();
                        if ui.add(Key::new("save and sync").solid(READOUT).min_width(130.0).enabled(tested && ok)).clicked() {
                            if let Ok(cfg) = &parsed {
                                event = AddEvent::Save(cfg.clone());
                            }
                        }
                        if ui.add(Key::new("test connection").outline(VALUE).min_width(130.0).enabled(ok && !self.test.running())).clicked() {
                            run_test = true;
                        }
                        if !self.first && ui.add(Key::new("cancel").outline(VALUE).min_width(90.0)).clicked() {
                            event = AddEvent::Cancel;
                        }
                    });
                    return;
                }
                row(ui, "email", |ui| {
                    let r = field_then(ui, &mut self.email, "you@example.com", 110.0, |ui| {
                        detect = ui.add(Key::new("detect").outline(VALUE).min_width(90.0)).clicked();
                    });
                    if submitted(ui, &r) || (r.lost_focus() && self.email() != self.detected_for) {
                        detect = true;
                    }
                });
                match &self.detect {
                    Detect::Idle => {}
                    Detect::Running(_) => {
                        Line::new().measured("looking up server settings").size(11.0).show(ui);
                    }
                    Detect::Found(source) => status(ui, true, &format!("Found settings from the {source}")),
                    Detect::Missing => {
                        note(ui, "No settings found for this domain. Fill them in under edit server settings.", WARN);
                    }
                }
                if !self.imap_host.is_empty() {
                    let imap = ServerSettings::new(&self.imap_host, self.imap_port.parse().unwrap_or(0), self.imap_security.clone());
                    let smtp = ServerSettings::new(&self.smtp_host, self.smtp_port.parse().unwrap_or(0), self.smtp_security.clone());
                    reading(ui, "incoming", sec_summary(&imap));
                    reading(ui, "outgoing", sec_summary(&smtp));
                }
                ui.add_space(2.0);
                ui.checkbox(&mut self.edit, "edit server settings");
                if self.edit {
                    self.server_rows(ui);
                }
                ui.add_space(6.0);
                if let Some((provider, _)) = &self.oauth {
                    status(ui, true, &format!("Signed in with {}", provider.name()));
                    if ui.add(Key::new("use a password instead").outline(VALUE)).clicked() {
                        self.oauth = None;
                    }
                } else {
                    row(ui, "password", |ui| {
                        secret(ui, &mut self.password);
                    });
                    if let Some(provider) = self.provider {
                        if oauth::client_for(provider).is_some() {
                            if self.oauth_rx.is_some() {
                                Line::new().measured(format!("waiting for {} in your browser", provider.name())).size(11.0).show(ui);
                            } else if ui.add(Key::new(format!("sign in with {}", provider.name())).solid(READOUT)).clicked() {
                                sign_in = Some(provider);
                            }
                        } else {
                            note(ui, oauth::missing_client_note(provider), WARN);
                        }
                    }
                    if let Some(e) = &self.oauth_error {
                        status(ui, false, e);
                    }
                }
                ui.add_space(6.0);
                let parsed = self.config();
                let offered = self.provider.is_some_and(|p| oauth::client_for(p).is_some());
                self.test.show(ui, parsed.as_ref().ok(), offered);
                if let Err(e) = &parsed {
                    if !self.email.is_empty() && self.test.results.is_empty() {
                        hint(ui, e);
                    }
                }
                if let Some(e) = &self.saved_error {
                    status(ui, false, e);
                }
                let tested = self.test.all_ok() && self.test.for_key == self.key();
                form_footer(ui, |ui| {
                    if let Ok(cfg) = &parsed {
                        if ui.add(Key::new("save and sync").solid(READOUT).min_width(130.0).enabled(tested)).clicked() {
                            event = AddEvent::Save(cfg.clone());
                        }
                    } else {
                        ui.add(Key::new("save and sync").solid(READOUT).min_width(130.0).enabled(false));
                    }
                    if ui
                        .add(Key::new("test connection").outline(VALUE).min_width(130.0).enabled(parsed.is_ok() && !self.test.running()))
                        .clicked()
                    {
                        run_test = true;
                    }
                    if !self.first && ui.add(Key::new("cancel").outline(VALUE).min_width(90.0)).clicked() {
                        event = AddEvent::Cancel;
                    }
                });
            });
        });
        if detect {
            self.start_detect(env, &ctx);
        }
        if let Some(p) = sign_in {
            self.start_oauth(p, env, &ctx);
        }
        if run_test {
            if let Ok(cfg) = self.config() {
                let key = self.key();
                self.test.start(&cfg, key, &ctx);
            }
        }
        event
    }
}

pub struct SettingsForm {
    pub drafts: Vec<AccountConfig>,
    original: Vec<AccountConfig>,
    removing: Option<usize>,
    pub new_password: String,
    pub new_confirm: String,
    pub message: Option<(bool, String)>,
    retest: Option<(String, TestRun)>,
    methods: UnlockMethods,
    pub allow_open: bool,
    pub panels: crate::settings::Panels,
    updater: egui_bench::update::Updater,
}

pub enum SettingsEvent {
    None,
    Prefs,
    Close,
    Rebuild,
    AddAccount,
    AllowOpen,
}

fn methods_of(store: &Store) -> UnlockMethods {
    UnlockMethods { keychain: store.has_keychain().unwrap_or(false), password: store.has_password().unwrap_or(false) }
}

impl SettingsForm {
    fn new(store: &Store, dir: &std::path::Path) -> Self {
        let drafts = accounts::load_all(store);
        SettingsForm {
            panels: crate::settings::Panels::new(store, &drafts, crate::settings::default_backup_path(dir)),
            original: drafts.clone(),
            drafts,
            removing: None,
            new_password: String::new(),
            new_confirm: String::new(),
            message: None,
            retest: None,
            methods: methods_of(store),
            allow_open: allow_open_setting(store),
            updater: egui_bench::update::Updater::new(UPDATE_REPO, Some(env!("CARGO_PKG_VERSION").to_string())),
        }
    }

    fn password_problem(&self) -> Option<&'static str> {
        if self.new_password.chars().count() < MIN_PASSWORD {
            return Some("Use at least 8 characters");
        }
        if self.new_password != self.new_confirm {
            return Some("The two passwords do not match");
        }
        None
    }

    fn set_password(&mut self, store: &Store, kdf: KdfParams) {
        match store.set_password(&self.new_password, kdf) {
            Ok(()) => self.message = Some((true, "Password saved".into())),
            Err(e) => self.message = Some((false, format!("Could not set the password: {e}"))),
        }
        self.new_password.clear();
        self.new_confirm.clear();
        self.methods = methods_of(store);
    }

    fn show(&mut self, ctx: &egui::Context, store: &Store, kc: &dyn Keychain, kdf: KdfParams) -> SettingsEvent {
        if let Some((_, run)) = &mut self.retest {
            run.poll(ctx);
        }
        let mut event = SettingsEvent::None;
        if let Some(i) = self.removing {
            let name = self.drafts[i].name.clone();
            let mut keep = false;
            let m = egui::Modal::new(egui::Id::new("remove-account"))
                .backdrop_color(egui::Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(400.0);
                    modal_title(ui, &format!("Remove {name}?"));
                    note(
                        ui,
                        "This deletes the mail synced from this account on this machine. Nothing is deleted on the server, and you can add the account again.",
                        LEGEND,
                    );
                    form_footer(ui, |ui| {
                        if ui.add(Key::new("remove").solid(FAULT).min_width(100.0)).clicked() {
                            let id = self.drafts[i].id.clone();
                            match accounts::remove(store, &id) {
                                Ok(()) => {
                                    self.drafts.remove(i);
                                    self.original.retain(|c| c.id != id);
                                    self.message = Some((true, "Account removed".into()));
                                    event = SettingsEvent::Rebuild;
                                }
                                Err(e) => self.message = Some((false, format!("Could not remove: {e}"))),
                            }
                            self.removing = None;
                        }
                        if ui.add(Key::new("keep it").outline(VALUE).min_width(100.0)).clicked() {
                            keep = true;
                        }
                    });
                });
            if keep || m.should_close() {
                self.removing = None;
            }
            return event;
        }
        let mut close = false;
        let mut apply = false;
        let mut add = false;
        let mut remove = None;
        let mut retest = None;
        let mut enroll = false;
        let mut drop_keychain = false;
        let mut drop_password = false;
        let mut set_password = false;
        let mut allow_open_changed = false;
        let mut prefs_event = false;
        let m = egui::Modal::new(egui::Id::new("settings"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(560.0_f32.min(ctx.content_rect().width() - 48.0));
                modal_title(ui, "Settings");
                let w = ui.available_width();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height((ctx.content_rect().height() * 0.7).max(320.0))
                    .show(ui, |ui| {
                        ui.set_max_width(w);
                        for (i, draft) in self.drafts.iter_mut().enumerate() {
                            let title = draft.address.clone();
                            section(ui, &title, "", |ui| {
                                row(ui, "name", |ui| {
                                    field(ui, &mut draft.name, "shown in the account list");
                                });
                                row(ui, "display name", |ui| {
                                    field(ui, &mut draft.display_name, "shown to people you write to");
                                });
                                row(ui, "signature", |ui| {
                                    prose(ui, &mut draft.signature, "added to new messages", 3);
                                });
                                reading(ui, "incoming", sec_summary(&draft.imap));
                                reading(ui, "outgoing", sec_summary(&draft.smtp));
                                if let Some((id, run)) = &self.retest {
                                    if *id == draft.id {
                                        run.show(ui, Some(draft), false);
                                    }
                                }
                                ui.horizontal(|ui| {
                                    if ui.add(Key::new("re-test").outline(VALUE).id_salt(("retest", i))).clicked() {
                                        retest = Some(i);
                                    }
                                    if ui.add(Key::new("remove").outline(FAULT).id_salt(("remove", i))).clicked() {
                                        remove = Some(i);
                                    }
                                    ui.checkbox(&mut draft.backfill.enabled, "index bodies offline");
                                    if draft.backfill.enabled {
                                        let r = ui.add(egui::DragValue::new(&mut draft.backfill.days).range(0..=36500).speed(5.0).suffix(" days"));
                                        ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("keep days, 0 for all"));
                                        let r = ui.add(egui::DragValue::new(&mut draft.backfill.megabytes).range(0..=1_000_000).speed(20.0).suffix(" MB"));
                                        ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("size cap, 0 for none"));
                                    }
                                });
                                self.panels.account_sync(ui, draft, i);
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut draft.notify.enabled, "notifications");
                                    if draft.notify.enabled {
                                        let mut quiet = draft.notify.quiet.is_some();
                                        ui.checkbox(&mut quiet, "quiet hours");
                                        if quiet != draft.notify.quiet.is_some() {
                                            draft.notify.quiet = quiet.then_some(crate::notify::QuietHours { start: 22 * 60, end: 7 * 60 });
                                        }
                                        if let Some(q) = draft.notify.quiet.as_mut() {
                                            let (mut from, mut to) = (q.start / 60, q.end / 60);
                                            let r = ui.add(egui::DragValue::new(&mut from).range(0..=23).suffix(":00"));
                                            ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("quiet from hour"));
                                            let r = ui.add(egui::DragValue::new(&mut to).range(0..=23).suffix(":00"));
                                            ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("quiet until hour"));
                                            q.start = from * 60;
                                            q.end = to * 60;
                                        }
                                    }
                                });
                            });
                            ui.add_space(8.0);
                        }
                        if ui.add(Key::new("add account").outline(VALUE)).clicked() {
                            add = true;
                        }
                        ui.add_space(8.0);
                        section(ui, "unlock", "how mailbench opens its encrypted data", |ui| {
                            reading(ui, "keychain", if self.methods.keychain { "on" } else { "off" });
                            reading(ui, "password", if self.methods.password { "on" } else { "off" });
                            ui.add_space(4.0);
                            row(ui, "new password", |ui| {
                                secret(ui, &mut self.new_password);
                            });
                            row(ui, "again", |ui| {
                                secret(ui, &mut self.new_confirm);
                            });
                            if !self.new_password.is_empty() {
                                let (_, label) = strength(&self.new_password);
                                Line::new().legend("strength").gap(10.0).value(label).tint(strength_tint(label)).size(11.0).show(ui);
                            }
                            ui.horizontal(|ui| {
                                let label = if self.methods.password { "change password" } else { "add password" };
                                if ui.add(Key::new(label).outline(VALUE).enabled(self.password_problem().is_none())).clicked() {
                                    set_password = true;
                                }
                                if self.methods.password
                                    && ui.add(Key::new("remove password").outline(FAULT).enabled(self.methods.keychain)).clicked()
                                {
                                    drop_password = true;
                                }
                                if self.methods.keychain {
                                    if ui.add(Key::new("remove keychain unlock").outline(FAULT).enabled(self.methods.password)).clicked() {
                                        drop_keychain = true;
                                    }
                                } else if ui.add(Key::new("add keychain unlock").outline(VALUE)).clicked() {
                                    enroll = true;
                                }
                            });
                            hint(ui, "At least one way to unlock must stay on.");
                            if let Some((ok, text)) = &self.message {
                                status(ui, *ok, text);
                            }
                        });
                        ui.add_space(8.0);
                        let outcome = self.panels.show(ui, store, &self.drafts);
                        prefs_event = outcome.prefs_changed;
                        ui.add_space(8.0);
                        section(ui, "updates", "new versions are published on GitHub releases", |ui| {
                            reading(ui, "version", env!("CARGO_PKG_VERSION"));
                            ui.horizontal(|ui| {
                                if ui.add(Key::new("check for updates").outline(VALUE).enabled(!self.updater.busy())).clicked() {
                                    self.updater.check(ui.ctx());
                                }
                                if ui.add(Key::new("releases page").outline(VALUE)).clicked() {
                                    let _ = open::that(self.updater.page());
                                }
                            });
                            self.updater.status(ui);
                        });
                        ui.add_space(8.0);
                        section(ui, "attachments", "what happens when you open a file", |ui| {
                            if ui.checkbox(&mut self.allow_open, "allow open (writes a plain copy to a temp folder)").changed() {
                                let _ = store.put(SETTINGS_KIND, ALLOW_OPEN_KEY, if self.allow_open { b"1" } else { b"0" });
                                allow_open_changed = true;
                            }
                            hint(ui, "Save as always works. Copies made by open are deleted when mailbench closes and on the next start.");
                        });
                    });
                form_footer(ui, |ui| {
                    if ui.add(Key::new("apply").solid(READOUT).min_width(100.0)).clicked() {
                        apply = true;
                    }
                    if ui.add(Key::new("close").outline(VALUE).min_width(100.0)).clicked() {
                        close = true;
                    }
                });
            });
        if allow_open_changed {
            event = SettingsEvent::AllowOpen;
        }
        if prefs_event {
            event = SettingsEvent::Prefs;
        }
        if let Some(i) = remove {
            self.removing = Some(i);
        }
        if let Some(i) = retest {
            let cfg = self.drafts[i].clone();
            let mut run = TestRun::default();
            run.start(&cfg, String::new(), ctx);
            self.retest = Some((cfg.id.clone(), run));
        }
        if set_password {
            self.set_password(store, kdf);
        }
        if enroll {
            self.message = Some(match store.enroll_keychain(kc) {
                Ok(()) => (true, "Keychain unlock added".into()),
                Err(e) => (false, format!("The keychain is not available: {e}")),
            });
            self.methods = methods_of(store);
        }
        if drop_keychain {
            self.message = Some(match store.remove_keychain(kc) {
                Ok(()) => (true, "Keychain unlock removed".into()),
                Err(StoreError::Locked) => (false, "Keep at least one way to unlock".into()),
                Err(e) => (false, format!("Could not remove it: {e}")),
            });
            self.methods = methods_of(store);
        }
        if drop_password {
            self.message = Some(match store.remove_password() {
                Ok(()) => (true, "Password removed".into()),
                Err(StoreError::Locked) => (false, "Keep at least one way to unlock".into()),
                Err(e) => (false, format!("Could not remove it: {e}")),
            });
            self.methods = methods_of(store);
        }
        if apply {
            let mut changed = false;
            for draft in &self.drafts {
                if self.original.iter().find(|o| o.id == draft.id) != Some(draft) {
                    if accounts::save(store, draft).is_ok() {
                        changed = true;
                    }
                }
            }
            if changed {
                self.original = self.drafts.clone();
                event = SettingsEvent::Rebuild;
            }
        }
        if add {
            event = SettingsEvent::AddAccount;
        }
        if close || m.should_close() {
            event = SettingsEvent::Close;
        }
        event
    }
}

pub struct Main {
    pub app: App,
}

enum Screen {
    First(FirstForm),
    Unlock(UnlockForm),
    Add(Box<AddForm>, Option<Box<Main>>),
    Main(Box<Main>),
}

pub struct Shell {
    dir: PathBuf,
    keychain: Rc<dyn Keychain>,
    env: Env,
    kdf: Option<KdfParams>,
    ctx: Option<egui::Context>,
    screen: Screen,
    store: Option<Store>,
    settings: Option<SettingsForm>,
    open_dir: PathBuf,
}

pub const UPDATE_REPO: &str = "v0l/mailbench";
pub const SETTINGS_KIND: &str = "settings";
pub const ALLOW_OPEN_KEY: &str = "attachments.allow_open";

pub const IMAGE_TRUST_KIND: &str = "image-trust";

pub fn allow_open_setting(store: &Store) -> bool {
    store.get(SETTINGS_KIND, ALLOW_OPEN_KEY).ok().flatten().map(|v| v.as_slice() != b"0").unwrap_or(true)
}

pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MAILBENCH_DATA") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "android")]
    if let Some(dir) = crate::store::android::data_dir() {
        return dir;
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("mailbench")
}

impl Shell {
    pub fn new(dir: PathBuf, keychain: Rc<dyn Keychain>, env: Env, kdf: Option<KdfParams>) -> Shell {
        let mut shell = Shell {
            dir,
            keychain,
            env,
            kdf,
            ctx: None,
            screen: Screen::First(FirstForm::new(true)),
            store: None,
            settings: None,
            open_dir: crate::attach::default_open_dir(),
        };
        crate::attach::wipe_open_dir(&shell.open_dir);
        shell.start();
        shell
    }

    pub fn with_open_dir(dir: PathBuf, keychain: Rc<dyn Keychain>, env: Env, kdf: Option<KdfParams>, open_dir: PathBuf) -> Shell {
        crate::attach::wipe_open_dir(&open_dir);
        let mut shell = Shell::new(dir, keychain, env, kdf);
        shell.open_dir = open_dir;
        shell.apply_open_settings();
        shell
    }

    fn apply_prefs(&mut self) {
        let Some(store) = &self.store else { return };
        let prefs = crate::prefs::load(store);
        let rebuild = matches!(&self.screen, Screen::Main(m) if m.app.prefs.bodies != prefs.bodies);
        if let Screen::Main(m) = &mut self.screen {
            m.app.html_state.dark = prefs.reading.dark_html;
            m.app.html_state.default_zoom = prefs.reading.zoom_milli();
            m.app.prefs = prefs;
        }
        if rebuild {
            self.build_main();
        }
    }

    fn apply_open_settings(&mut self) {
        let allow = self.store.as_ref().map(allow_open_setting).unwrap_or(true);
        let dir = self.open_dir.clone();
        let trusted: std::collections::HashSet<String> =
            self.store.as_ref().and_then(|s| s.list(IMAGE_TRUST_KIND).ok()).unwrap_or_default().into_iter().collect();
        let saver = self.store.as_ref().and_then(|s| s.try_clone().ok());
        if let Screen::Main(m) = &mut self.screen {
            m.app.allow_open = allow;
            m.app.open_dir = dir;
            m.app.trusted = trusted;
            if let Some(prefs_store) = self.store.as_ref().and_then(|s| s.try_clone().ok()) {
                m.app.prefs_saver = Box::new(move |prefs| {
                    let _ = crate::prefs::save(&prefs_store, prefs);
                });
            }
            if let Some(saver) = saver {
                m.app.trust_saver = Box::new(move |key| {
                    let _ = saver.put(IMAGE_TRUST_KIND, key, b"1");
                });
            }
        }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("mail.db")
    }

    fn start(&mut self) {
        let db = self.db_path();
        if !db.exists() {
            self.screen = Screen::First(FirstForm::new(self.keychain.available()));
            return;
        }
        let methods = match Store::methods(&db) {
            Ok(m) => m,
            Err(e) => {
                self.screen = Screen::Unlock(UnlockForm::new(Some(format!("Could not read the data file: {e}")), None, false));
                return;
            }
        };
        let mut note = None;
        if methods.keychain {
            match Store::open(&db, &[Unlock::Keychain(&*self.keychain)]) {
                Ok(store) => {
                    self.opened(store);
                    return;
                }
                Err(e) => {
                    note = Some(if methods.password {
                        "The system keychain is not available, so unlock with your password.".to_string()
                    } else {
                        format!("The system keychain did not unlock your data: {e}. It is the only way in, so unlock it and restart.")
                    });
                }
            }
        }
        self.screen = Screen::Unlock(UnlockForm::new(None, note, methods.password));
    }

    fn opened(&mut self, store: Store) {
        let accounts = accounts::load_all(&store);
        self.store = Some(store);
        if accounts.is_empty() {
            self.screen = Screen::Add(Box::new(AddForm::new(true)), None);
        } else {
            self.build_main();
        }
    }

    fn build_main(&mut self) {
        let Some(store) = &self.store else { return };
        let configs = accounts::load_all(store);
        if configs.is_empty() {
            self.screen = Screen::Add(Box::new(AddForm::new(true)), None);
            return;
        }
        let setups: Vec<_> = configs.iter().filter_map(|c| accounts::setup_for(c, store).ok()).collect();
        let Ok(clone) = store.try_clone() else { return };
        let real = RealStore::open(clone, setups);
        let prefs = crate::prefs::load(store);
        real.set_body_policy(prefs.bodies);
        for c in &configs {
            real.set_backfill(&c.id, prefs.bodies.effective_backfill(c.backfill));
            real.set_sync_options(&c.id, c.sync.clone());
            real.set_signature(&c.id, &c.signature);
            real.set_notify(&c.id, c.notify);
        }
        real.set_notifier(std::sync::Arc::new(crate::notify::DesktopNotifier));
        let mut app = App::with_store(Box::new(real));
        app.html_state.dark = prefs.reading.dark_html;
        app.html_state.default_zoom = prefs.reading.zoom_milli();
        app.prefs = prefs;
        self.screen = Screen::Main(Box::new(Main { app }));
        self.apply_open_settings();
        self.attach();
    }

    fn attach(&mut self) {
        if let (Screen::Main(_), Some(ctx)) = (&self.screen, &self.ctx) {
            ctx.request_repaint();
        }
    }

    pub fn screen_name(&self) -> &'static str {
        match &self.screen {
            Screen::First(_) => "first run",
            Screen::Unlock(_) => "unlock",
            Screen::Add(..) => "add account",
            Screen::Main(_) => "mail",
        }
    }

    pub fn first_form(&mut self) -> Option<&mut FirstForm> {
        match &mut self.screen {
            Screen::First(f) => Some(f),
            _ => None,
        }
    }

    pub fn unlock_form(&mut self) -> Option<&mut UnlockForm> {
        match &mut self.screen {
            Screen::Unlock(f) => Some(f),
            _ => None,
        }
    }

    pub fn add_form(&mut self) -> Option<&mut AddForm> {
        match &mut self.screen {
            Screen::Add(f, _) => Some(f),
            _ => None,
        }
    }

    pub fn app(&self) -> Option<&App> {
        match &self.screen {
            Screen::Main(m) => Some(&m.app),
            _ => None,
        }
    }

    pub fn app_mut(&mut self) -> Option<&mut App> {
        match &mut self.screen {
            Screen::Main(m) => Some(&mut m.app),
            _ => None,
        }
    }

    pub fn settings_open(&self) -> bool {
        self.settings.is_some()
    }

    pub fn open_settings(&mut self) {
        if let Some(store) = &self.store {
            self.settings = Some(SettingsForm::new(store, &self.dir));
        }
    }

    pub fn settings_form(&mut self) -> Option<&mut SettingsForm> {
        self.settings.as_mut()
    }

    fn kdf(&self) -> KdfParams {
        self.kdf.unwrap_or_else(|| KdfParams::calibrate(Duration::from_millis(500)))
    }

    fn submit_first(&mut self) {
        let Screen::First(form) = &mut self.screen else { return };
        if form.problem().is_some() {
            return;
        }
        let mode = form.mode;
        let password = form.password.clone();
        let kdf = self.kdf();
        let db = self.db_path();
        let _ = std::fs::create_dir_all(&self.dir);
        let keychain: Option<&dyn Keychain> = (mode != Protect::Password).then_some(&*self.keychain);
        let pw = (mode != Protect::Keychain).then(|| (password.as_str(), kdf));
        match Store::create(&db, keychain, pw) {
            Ok(store) => self.opened(store),
            Err(e) => {
                let _ = std::fs::remove_file(&db);
                let text = match (mode, e) {
                    (Protect::Password, e) => format!("Could not create the encrypted store: {e}"),
                    _ => "No system keychain found. Choose a password instead.".to_string(),
                };
                if let Screen::First(form) = &mut self.screen {
                    form.error = Some(text);
                }
            }
        }
    }

    fn submit_unlock(&mut self) {
        let db = self.db_path();
        let Screen::Unlock(form) = &mut self.screen else { return };
        if form.working.is_some() {
            return;
        }
        let password = zeroize::Zeroizing::new(form.password.clone());
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Store::open(&db, &[Unlock::Password(&password)]));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        form.error = None;
        form.working = Some(Unlocking { rx });
    }

    fn poll_unlock(&mut self) {
        let Screen::Unlock(form) = &mut self.screen else { return };
        let Some(w) = &form.working else { return };
        let result = match w.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(StoreError::Locked),
        };
        form.working = None;
        match result {
            Ok(store) => self.opened(store),
            Err(StoreError::WrongKey) => {
                form.error = Some("Wrong password. Try again.".into());
                form.password.clear();
                form.focused = false;
            }
            Err(e) => form.error = Some(format!("Could not unlock: {e}")),
        }
    }

    fn save_account(&mut self, cfg: AccountConfig) {
        let Some(store) = &self.store else { return };
        match accounts::save(store, &cfg) {
            Ok(()) => self.build_main(),
            Err(e) => {
                if let Screen::Add(form, _) = &mut self.screen {
                    form.saved_error = Some(format!("Could not save the account: {e}"));
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.ctx = Some(ctx.clone());
        match &mut self.screen {
            Screen::First(form) => {
                if form.show(ui) {
                    self.submit_first();
                }
            }
            Screen::Unlock(form) => {
                if form.show(ui) {
                    self.submit_unlock();
                }
                self.poll_unlock();
            }
            Screen::Add(form, _) => match form.show(ui, &self.env) {
                AddEvent::None => {}
                AddEvent::Save(cfg) => self.save_account(cfg),
                AddEvent::Cancel => {
                    let Screen::Add(_, back) = std::mem::replace(&mut self.screen, Screen::First(FirstForm::new(true))) else {
                        return;
                    };
                    match back {
                        Some(main) => self.screen = Screen::Main(main),
                        None => self.build_main(),
                    }
                }
            },
            Screen::Main(main) => {
                main.app.show(ui);
                if std::mem::take(&mut main.app.settings_requested) {
                    self.open_settings();
                }
            }
        }
        self.settings_ui(&ctx);
    }

    fn settings_ui(&mut self, ctx: &egui::Context) {
        let (Some(form), Some(store)) = (&mut self.settings, &self.store) else { return };
        let kdf = self.kdf.unwrap_or_else(|| KdfParams::calibrate(Duration::from_millis(500)));
        match form.show(ctx, store, &*self.keychain, kdf) {
            SettingsEvent::None => {}
            SettingsEvent::Close => self.settings = None,
            SettingsEvent::AllowOpen => self.apply_open_settings(),
            SettingsEvent::Prefs => self.apply_prefs(),
            SettingsEvent::Rebuild => {
                self.build_main();
                if matches!(self.screen, Screen::Add(..)) {
                    self.settings = None;
                }
            }
            SettingsEvent::AddAccount => {
                self.settings = None;
                let back = match std::mem::replace(&mut self.screen, Screen::First(FirstForm::new(true))) {
                    Screen::Main(m) => Some(m),
                    _ => None,
                };
                self.screen = Screen::Add(Box::new(AddForm::new(false)), back);
            }
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        crate::attach::wipe_open_dir(&self.open_dir);
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::no_frame().show(ui, |ui| self.show(ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_grows_with_length_and_variety() {
        assert_eq!(strength("aaaaaaaa").1, "weak");
        assert_eq!(strength("password").1, "weak");
        assert!(["good", "strong"].contains(&strength("correct horse battery staple").1));
        assert!(strength("Tr0ub4dor&3xyz").0 > strength("trouble").0);
    }
}
