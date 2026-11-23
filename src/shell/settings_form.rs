use super::{ALLOW_OPEN_KEY, Env, SETTINGS_KIND, TestRun, UPDATE_REPO, allow_open_setting, sec_summary, strength, strength_tint};
use crate::accounts::{self, AccountConfig, Credential};
use crate::backend::{Security, StoreSink};
use crate::kit::{footer_frame, header_frame, pane_fill, selection};
use crate::mailstore::{AccountInfo, SyncState};
use crate::oauth;
use crate::settings_page::{Level, Nav, Outcome, PageState, Section, Tab, move_item};
use crate::store::{KdfParams, Keychain, Store, StoreError, UnlockMethods};
use egui::{Rect, Sense, Ui};
use egui_bench::prelude::*;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

mod view;
mod agents;

const MIN_PASSWORD: usize = 8;
const NARROW_BELOW: f32 = 700.0;
const NAV_W: f32 = 190.0;
const LIST_W: f32 = 300.0;
const DETAIL_MAX: f32 = 640.0;
const PHONE_ROW: f32 = 44.0;

pub struct SettingsForm {
    pub drafts: Vec<AccountConfig>,
    original: Vec<AccountConfig>,
    pub page: PageState,
    live_ids: Vec<String>,
    infos: Vec<AccountInfo>,
    removing: Option<usize>,
    clearing: Option<usize>,
    leaving: Option<Leave>,
    pub new_password: String,
    pub new_confirm: String,
    pub message: Option<(bool, String)>,
    retest: Option<(String, TestRun)>,
    methods: UnlockMethods,
    pub allow_open: bool,
    pub panels: crate::settings::Panels,
    pub agents: agents::AgentsPanel,
    updater: egui_bench::update::Updater,
    relay_text: HashMap<String, String>,
    exported: Option<String>,
    signing: Option<Signing>,
    sign_error: Option<String>,
    disk: HashMap<String, (u64, u64)>,
    dir: std::path::PathBuf,
    acts: Acts,
    confirmed_leave: Option<Leave>,
    removed: bool,
}

struct Signing {
    id: String,
    provider: crate::autodetect::Provider,
    rx: Receiver<Result<oauth::OAuthTokens, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Leave {
    Close,
    Add,
}

#[derive(Default)]
struct Acts {
    apply: bool,
    discard: bool,
    close: bool,
    add: bool,
    back: bool,
    remove: Option<usize>,
    clear: Option<usize>,
    retest: Option<usize>,
    sign_in: Option<usize>,
    shift: Option<i32>,
    make_default: Option<usize>,
    enroll: bool,
    drop_keychain: bool,
    drop_password: bool,
    set_password: bool,
    allow_open_changed: bool,
}

pub enum SettingsEvent {
    None,
    Prefs,
    Agents,
    Close,
    Rebuild,
    AddAccount,
    AllowOpen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Address,
    ImapHost,
    SmtpHost,
    JmapUrl,
}

fn problem_of(cfg: &AccountConfig, field: Field) -> Option<&'static str> {
    match field {
        Field::Name if cfg.name.trim().is_empty() => Some("Enter a name"),
        Field::Address if !cfg.is_nostr() && !cfg.address.contains('@') => Some("Enter an email address"),
        Field::ImapHost if !cfg.is_nostr() && !cfg.is_jmap() && cfg.imap.host.trim().is_empty() => Some("Enter the incoming server host"),
        Field::SmtpHost if !cfg.is_nostr() && !cfg.is_jmap() && cfg.smtp.host.trim().is_empty() => Some("Enter the outgoing server host"),
        Field::JmapUrl => match &cfg.credential {
            Credential::Jmap { url, .. } if !url.trim().starts_with("http") => Some("Enter the JMAP URL, starting with https://"),
            _ => None,
        },
        _ => None,
    }
}

const FIELDS: [Field; 5] = [Field::Name, Field::Address, Field::ImapHost, Field::SmtpHost, Field::JmapUrl];

fn problems(cfg: &AccountConfig) -> usize {
    FIELDS.iter().filter(|f| problem_of(cfg, **f).is_some()).count()
}

fn issue(ui: &mut Ui, cfg: &AccountConfig, field: Field) {
    if let Some(text) = problem_of(cfg, field) {
        status(ui, false, text);
    }
}

pub fn kind_of(cfg: &AccountConfig) -> &'static str {
    if cfg.is_nostr() {
        "NOSTR"
    } else if cfg.is_jmap() {
        "JMAP"
    } else {
        "IMAP"
    }
}

fn needs_sign_in(info: &AccountInfo) -> bool {
    info.sync == SyncState::Failed && info.detail.as_deref().is_some_and(|d| d.starts_with("authentication failed"))
}

fn state_of(info: Option<&AccountInfo>) -> (&'static str, egui::Color32) {
    match info {
        None => ("not synced yet", LEGEND),
        Some(i) if needs_sign_in(i) => ("needs sign in", WARN),
        Some(i) => match i.sync {
            SyncState::Idle => ("ok", OK),
            SyncState::Syncing => ("syncing", TRACE),
            SyncState::Failed => ("error", FAULT),
        },
    }
}

fn methods_of(store: &Store) -> UnlockMethods {
    UnlockMethods { keychain: store.has_keychain().unwrap_or(false), password: store.has_password().unwrap_or(false) }
}

fn sec_label(s: &Security) -> String {
    match s {
        Security::Tls => "TLS".into(),
        Security::StartTls => "STARTTLS".into(),
        Security::Plain => "none (not encrypted)".into(),
    }
}

fn relays_of(c: &mut Credential) -> Option<&mut Vec<String>> {
    match c {
        Credential::Nostr { relays, .. } | Credential::NostrBunker { relays, .. } => Some(relays),
        _ => None,
    }
}

fn list_row(ui: &mut Ui, h: f32, on: bool, label: &str) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::click());
    let p = ui.painter();
    if on {
        selection(p, rect);
    } else if resp.hovered() {
        p.rect_filled(rect, 2.0, BAND);
    }
    let label = label.to_string();
    ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(label));
    (rect, resp)
}

impl SettingsForm {
    pub fn new(store: &Store, dir: &std::path::Path) -> Self {
        let drafts = accounts::load_all(store);
        SettingsForm {
            panels: crate::settings::Panels::new(store, &drafts, crate::settings::default_backup_path(dir)),
            agents: agents::AgentsPanel::new(store),
            original: drafts.clone(),
            live_ids: drafts.iter().map(|d| d.id.clone()).collect(),
            drafts,
            page: PageState::new(),
            infos: Vec::new(),
            removing: None,
            clearing: None,
            leaving: None,
            new_password: String::new(),
            new_confirm: String::new(),
            message: None,
            retest: None,
            methods: methods_of(store),
            allow_open: allow_open_setting(store),
            updater: egui_bench::update::Updater::new(UPDATE_REPO, Some(env!("CARGO_PKG_VERSION").to_string())),
            relay_text: HashMap::new(),
            exported: None,
            signing: None,
            sign_error: None,
            disk: HashMap::new(),
            dir: dir.to_path_buf(),
            acts: Acts::default(),
            confirmed_leave: None,
            removed: false,
        }
    }

    pub fn focus_account(&mut self, id: &str) {
        self.page.open_section(Section::Accounts);
        if let Some(i) = self.drafts.iter().position(|d| d.id == id) {
            self.page.accounts = self.drafts.len();
            self.page.select_account(i);
        }
    }

    pub fn dirty(&self) -> bool {
        self.drafts.iter().any(|d| self.original.iter().find(|o| o.id == d.id) != Some(d))
    }

    fn problem_count(&self) -> usize {
        self.drafts.iter().map(problems).sum()
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

    fn info_of(&self, id: &str) -> Option<&AccountInfo> {
        self.live_ids.iter().position(|x| x == id).and_then(|i| self.infos.get(i))
    }

    fn poll_sign_in(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.signing else { return };
        match s.rx.try_recv() {
            Ok(Ok(tokens)) => {
                let (id, provider) = (s.id.clone(), s.provider);
                if let Some(d) = self.drafts.iter_mut().find(|d| d.id == id) {
                    d.credential = Credential::OAuth { provider, tokens };
                }
                self.signing = None;
            }
            Ok(Err(e)) => {
                self.sign_error = Some(e);
                self.signing = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => self.signing = None,
        }
    }

    fn start_sign_in(&mut self, i: usize, env: &Env, ctx: &egui::Context) {
        let draft = &self.drafts[i];
        let Credential::OAuth { provider, .. } = &draft.credential else {
            self.page.tab = Tab::Servers;
            return;
        };
        let provider = *provider;
        let Some(client) = oauth::client_for(provider) else {
            self.sign_error = Some(oauth::missing_client_note(provider));
            return;
        };
        let (tx, rx) = channel();
        let opener = env.opener.clone();
        let email = draft.address.clone();
        let repaint = ctx.clone();
        self.sign_error = None;
        std::thread::spawn(move || {
            let result = oauth::sign_in(&oauth::endpoints(provider), &client, &email, &*opener, Duration::from_secs(300));
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        self.signing = Some(Signing { id: draft.id.clone(), provider, rx });
    }

    fn modal_open(&self) -> bool {
        self.removing.is_some() || self.clearing.is_some() || self.leaving.is_some()
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if self.modal_open() || self.panels.capturing.is_some() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.acts.apply = true;
        }
        let (esc, up, down, enter) = ctx.input(|i| {
            (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter))
        });
        let typing = ctx.egui_wants_keyboard_input();
        let focused = ctx.memory(|m| m.focused().is_some());
        let nav = if esc && !typing {
            Some(Nav::Back)
        } else if typing || focused {
            None
        } else if up {
            Some(Nav::Up)
        } else if down {
            Some(Nav::Down)
        } else if enter {
            Some(Nav::Enter)
        } else {
            None
        };
        if let Some(nav) = nav {
            if self.page.press(nav) == Outcome::Leave {
                self.acts.close = true;
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, store: &Store, kc: &dyn Keychain, kdf: KdfParams, env: &Env, infos: &[AccountInfo]) -> SettingsEvent {
        let ctx = ui.ctx().clone();
        if let Some((_, run)) = &mut self.retest {
            run.poll(&ctx);
        }
        self.poll_sign_in(&ctx);
        self.infos = infos.to_vec();
        self.page.narrow = ui.available_width() < NARROW_BELOW;
        self.page.accounts = self.drafts.len();
        self.page.account = self.page.account.min(self.drafts.len().saturating_sub(1));
        self.acts = Acts::default();
        self.keys(&ctx);
        self.draw(ui, store);
        self.draw_modals(&ctx, store);
        let outcome = self.panels.flush(store);
        let mut event = SettingsEvent::None;
        if self.acts.allow_open_changed {
            event = SettingsEvent::AllowOpen;
        }
        if outcome.prefs_changed {
            event = SettingsEvent::Prefs;
        }
        if self.agents.flush(store) {
            event = SettingsEvent::Agents;
        }
        let acts = std::mem::take(&mut self.acts);
        if let Some(i) = acts.remove {
            self.removing = Some(i);
        }
        if let Some(i) = acts.clear {
            self.clearing = Some(i);
        }
        if let Some(i) = acts.retest {
            let cfg = self.drafts[i].clone();
            let mut run = TestRun::default();
            run.start(&cfg, String::new(), &ctx);
            self.retest = Some((cfg.id.clone(), run));
        }
        if let Some(i) = acts.sign_in {
            self.start_sign_in(i, env, &ctx);
        }
        if acts.set_password {
            self.set_password(store, kdf);
        }
        if acts.enroll {
            self.message = Some(match store.enroll_keychain(kc) {
                Ok(()) => (true, "Keychain unlock added".into()),
                Err(e) => (false, format!("The keychain is not available: {e}")),
            });
            self.methods = methods_of(store);
        }
        if acts.drop_keychain {
            self.message = Some(match store.remove_keychain(kc) {
                Ok(()) => (true, "Keychain unlock removed".into()),
                Err(StoreError::Locked) => (false, "Keep at least one way to unlock".into()),
                Err(e) => (false, format!("Could not remove it: {e}")),
            });
            self.methods = methods_of(store);
        }
        if acts.drop_password {
            self.message = Some(match store.remove_password() {
                Ok(()) => (true, "Password removed".into()),
                Err(StoreError::Locked) => (false, "Keep at least one way to unlock".into()),
                Err(e) => (false, format!("Could not remove it: {e}")),
            });
            self.methods = methods_of(store);
        }
        if let Some(delta) = acts.shift {
            if self.relocate(store, self.page.account, delta) {
                event = SettingsEvent::Rebuild;
            }
        }
        if let Some(i) = acts.make_default {
            let mut moved = false;
            let mut at = i;
            while at > 0 {
                moved |= self.relocate(store, at, -1);
                at -= 1;
            }
            self.page.account = 0;
            if moved {
                event = SettingsEvent::Rebuild;
            }
        }
        if acts.discard {
            self.drafts = self.original.clone();
            self.relay_text.clear();
        }
        if acts.apply && self.problem_count() == 0 {
            if self.apply(store) {
                event = SettingsEvent::Rebuild;
            }
        }
        if acts.add {
            if self.dirty() {
                self.leaving = Some(Leave::Add);
            } else {
                event = SettingsEvent::AddAccount;
            }
        }
        if acts.close {
            if self.dirty() {
                self.leaving = Some(Leave::Close);
            } else {
                event = SettingsEvent::Close;
            }
        }
        if acts.back {
            if self.page.press(Nav::Back) == Outcome::Leave {
                if self.dirty() {
                    self.leaving = Some(Leave::Close);
                } else {
                    event = SettingsEvent::Close;
                }
            }
        }
        if std::mem::take(&mut self.removed) {
            event = SettingsEvent::Rebuild;
        }
        if let Some(leave) = self.confirmed_leave.take() {
            event = match leave {
                Leave::Close => SettingsEvent::Close,
                Leave::Add => SettingsEvent::AddAccount,
            };
        }
        event
    }

    fn relocate(&mut self, store: &Store, from: usize, delta: i32) -> bool {
        let Some(to) = move_item(&mut self.drafts, from, delta) else { return false };
        move_item(&mut self.live_ids, from, delta);
        self.page.account = to;
        let ids: Vec<String> = self.drafts.iter().map(|d| d.id.clone()).collect();
        let _ = accounts::save_order(store, &ids);
        true
    }

    fn apply(&mut self, store: &Store) -> bool {
        let mut changed = false;
        for i in 0..self.drafts.len() {
            if self.drafts[i].is_nostr() {
                let unique = accounts::unique_name(&self.drafts[i].name, &self.drafts[i].id, &self.drafts);
                self.drafts[i].name = unique;
            }
        }
        for draft in &self.drafts {
            if self.original.iter().find(|o| o.id == draft.id) != Some(draft) && accounts::save(store, draft).is_ok() {
                changed = true;
            }
        }
        if changed {
            self.original = self.drafts.clone();
        }
        changed
    }

    fn draw_modals(&mut self, ctx: &egui::Context, store: &Store) {
        if let Some(i) = self.removing {
            let name = self.drafts[i].name.clone();
            let mut keep = false;
            let m = egui::Modal::new(egui::Id::new("remove-account"))
                .backdrop_color(egui::Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(400.0_f32.min(ctx.content_rect().width() - 48.0));
                    modal_title(ui, &format!("Remove {name}?"));
                    note(
                        ui,
                        "This deletes the mail synced from this account on this machine. Nothing is deleted on the server, and you can add the account again.",
                        LEGEND,
                    );
                    crate::kit::form_footer(ui, |ui| {
                        if ui.add(Key::new("remove").solid(FAULT).min_width(100.0)).clicked() {
                            let id = self.drafts[i].id.clone();
                            match accounts::remove(store, &id) {
                                Ok(()) => {
                                    self.drafts.remove(i);
                                    self.live_ids.retain(|x| *x != id);
                                    self.original.retain(|c| c.id != id);
                                    let ids: Vec<String> = self.drafts.iter().map(|d| d.id.clone()).collect();
                                    let _ = accounts::save_order(store, &ids);
                                    self.message = Some((true, "Account removed".into()));
                                    self.removed = true;
                                    self.page.level = Level::Page;
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
        }
        if let Some(i) = self.clearing {
            let name = self.drafts[i].name.clone();
            let mut keep = false;
            let m = egui::Modal::new(egui::Id::new("clear-account-mail"))
                .backdrop_color(egui::Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(400.0_f32.min(ctx.content_rect().width() - 48.0));
                    modal_title(ui, &format!("Clear local mail for {name}?"));
                    note(
                        ui,
                        "This deletes the mail synced from this account on this machine and fetches it again. Nothing is deleted on the server.",
                        LEGEND,
                    );
                    crate::kit::form_footer(ui, |ui| {
                        if ui.add(Key::new("clear local mail").solid(FAULT).min_width(100.0)).clicked() {
                            let id = self.drafts[i].id.clone();
                            match StoreSink::purge(store, &id) {
                                Ok(()) => {
                                    self.message = Some((true, "Local mail cleared".into()));
                                    self.disk.remove(&id);
                                    self.removed = true;
                                }
                                Err(e) => self.message = Some((false, format!("Could not clear: {e}"))),
                            }
                            self.clearing = None;
                        }
                        if ui.add(Key::new("keep mail").outline(VALUE).min_width(100.0)).clicked() {
                            keep = true;
                        }
                    });
                });
            if keep || m.should_close() {
                self.clearing = None;
            }
        }
        if let Some(leave) = self.leaving {
            let mut keep = false;
            let m = egui::Modal::new(egui::Id::new("unsaved-changes"))
                .backdrop_color(egui::Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(400.0_f32.min(ctx.content_rect().width() - 48.0));
                    modal_title(ui, "Leave with unsaved changes?");
                    note(ui, "Your edits to the accounts have not been applied yet.", LEGEND);
                    crate::kit::form_footer(ui, |ui| {
                        if ui.add(Key::new("discard and leave").solid(FAULT).min_width(100.0)).clicked() {
                            self.drafts = self.original.clone();
                            self.leaving = None;
                            self.confirmed_leave = Some(leave);
                        }
                        if ui.add(Key::new("keep editing").outline(VALUE).min_width(100.0)).clicked() {
                            keep = true;
                        }
                    });
                });
            if keep || m.should_close() {
                self.leaving = None;
            }
        }
    }
}
