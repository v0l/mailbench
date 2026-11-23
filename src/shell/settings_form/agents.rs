use super::*;
use crate::mcp::prefs::{self, AgentPrefs};

pub struct AgentsPanel {
    pub prefs: AgentPrefs,
    saved: AgentPrefs,
    pub token: Option<String>,
    pub has_token: bool,
    pub running: Option<Result<u16, String>>,
    pub note: Option<(bool, String)>,
}

impl AgentsPanel {
    pub fn new(store: &Store) -> AgentsPanel {
        let prefs = prefs::load(store);
        AgentsPanel { saved: prefs.clone(), prefs, token: None, has_token: crate::mcp::token::existing(store).is_some(), running: None, note: None }
    }

    pub fn flush(&mut self, store: &Store) -> bool {
        if self.prefs == self.saved {
            return false;
        }
        match prefs::save(store, &self.prefs) {
            Ok(()) => {
                self.saved = self.prefs.clone();
                true
            }
            Err(e) => {
                self.note = Some((false, format!("Could not save: {e}")));
                false
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, store: &Store, accounts: &[AccountConfig], narrow: bool) {
        section(ui, "agents", "let AI agents read and draft mail through a local server", |ui| {
            ui.checkbox(&mut self.prefs.enabled, "run the agent server inside the app");
            hint(ui, "Off by default. It listens on this computer only (127.0.0.1) and needs the access token below.");
            match &self.running {
                Some(Ok(port)) if self.prefs.enabled => {
                    status(ui, true, &format!("Listening on http://127.0.0.1:{port}/mcp"));
                }
                Some(Err(e)) if self.prefs.enabled => status(ui, false, e),
                _ => {}
            }
            row(ui, "port", |ui| {
                let mut port = self.prefs.port;
                let r = ui.add(egui::DragValue::new(&mut port).range(1024..=65535));
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("agent server port"));
                self.prefs.port = port;
            });
            if narrow {
                hint(ui, "Default 7480.");
            }
        });
        ui.add_space(8.0);
        section(ui, "allowed accounts", "agents see only the accounts ticked here", |ui| {
            if accounts.is_empty() {
                hint(ui, "No accounts yet.");
            }
            for a in accounts {
                let name = if a.name.trim().is_empty() { a.address.clone() } else { a.name.clone() };
                let mut allowed = self.prefs.allows(&a.id);
                if ui.checkbox(&mut allowed, format!("agents may use {name}")).changed() {
                    self.prefs.set_allowed(&a.id, allowed);
                }
                if allowed {
                    let mut send = self.prefs.send.contains(&a.id);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        if ui.checkbox(&mut send, format!("let agents send mail without asking from {name}")).changed() {
                            self.prefs.set_send(&a.id, send);
                        }
                    });
                }
            }
            if accounts.iter().any(|a| a.is_nostr()) {
                ui.checkbox(&mut self.prefs.nostr_send, "let agents send from nostr accounts too");
            }
            hint(ui, "With sending off, agents create drafts. You see them in Drafts with a banner and send them with Ctrl+Enter. Sending without asking is capped at 20 per hour per account.");
        });
        ui.add_space(8.0);
        section(ui, "access token", "clients send it as a bearer token", |ui| {
            match self.token.clone() {
                Some(token) => {
                    reading(ui, "token", token.clone());
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(Key::new("copy token").outline(VALUE)).clicked() {
                            ui.ctx().copy_text(token.clone());
                            self.note = Some((true, "Token copied".into()));
                        }
                    });
                    hint(ui, "Shown once. Later, run `mailbench mcp token` to read it again.");
                }
                None if self.has_token => hint(ui, "A token is set. Rotate it to get a new one, or run `mailbench mcp token`."),
                None => hint(ui, "No token yet."),
            }
            ui.horizontal_wrapped(|ui| {
                let label = if self.has_token { "rotate token" } else { "create token" };
                if ui.add(Key::new(label).outline(VALUE)).clicked() {
                    let made = if self.has_token { crate::mcp::token::rotate(store) } else { crate::mcp::token::load_or_create(store).map(|(t, _)| t) };
                    match made {
                        Ok(t) => {
                            self.token = Some(t);
                            self.has_token = true;
                            self.note = Some((true, "New token created. Clients with the old one stop working.".into()));
                        }
                        Err(e) => self.note = Some((false, format!("Could not make a token: {e}"))),
                    }
                }
            });
            if let Some((ok, text)) = &self.note {
                status(ui, *ok, text);
            }
        });
    }
}
