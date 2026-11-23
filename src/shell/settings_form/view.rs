use super::*;
use crate::kit::{body, dot, put, put_right, selection};

fn shown_name(d: &AccountConfig) -> &str {
    if d.name.trim().is_empty() { &d.address } else { &d.name }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

impl SettingsForm {
    pub(super) fn draw(&mut self, ui: &mut Ui, store: &Store) {
        let narrow = self.page.narrow;
        self.header(ui);
        if self.dirty() {
            self.unsaved_bar(ui);
        }
        if narrow {
            egui::CentralPanel::no_frame().show(ui, |ui| {
                pane_fill(ui, CHASSIS);
                egui::Frame::NONE.inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    if self.page.level == Level::Sections {
                        self.nav(ui);
                    } else {
                        self.page_body(ui, store);
                    }
                });
            });
        } else {
            egui::Panel::left("settings-nav")
                .exact_size(NAV_W)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::same(8)))
                .show(ui, |ui| self.nav(ui));
            egui::CentralPanel::no_frame().show(ui, |ui| {
                pane_fill(ui, CHASSIS);
                egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 8 }).show(ui, |ui| {
                    self.page_body(ui, store);
                });
            });
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        let at_root = self.page.level == Level::Sections || !self.page.narrow;
        egui::Panel::top("settings-head").frame(header_frame()).show(ui, |ui| {
            ui.horizontal(|ui| {
                if at_root {
                    if ui.add(Key::new("close").outline(VALUE).min_width(90.0)).clicked() {
                        self.acts.close = true;
                    }
                } else if ui.add(Key::new("back").outline(VALUE).min_width(90.0)).clicked() {
                    self.acts.back = true;
                }
                ui.add_space(8.0);
                let title = if self.page.narrow && self.page.level != Level::Sections {
                    self.page.section.label()
                } else {
                    "settings"
                };
                Line::new().legend(title).show(ui);
            });
        });
    }

    fn unsaved_bar(&mut self, ui: &mut Ui) {
        let problems = self.problem_count();
        egui::Panel::bottom("settings-foot").frame(footer_frame()).show(ui, |ui| {
            ui.horizontal(|ui| {
                Line::new().legend("unsaved changes").tint(WARN).show(ui);
                if problems > 0 {
                    Line::new().value(format!("{problems} to fix")).tint(FAULT).size(11.0).show(ui);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Key::new("apply").solid(READOUT).enabled(problems == 0)).clicked() {
                        self.acts.apply = true;
                    }
                    if ui.add(Key::new("discard").outline(VALUE)).clicked() {
                        self.acts.discard = true;
                    }
                });
            });
        });
    }

    fn nav(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let h = if self.page.narrow { PHONE_ROW } else { 34.0 };
        for s in Section::ALL {
            let on = s == self.page.section;
            let (rect, resp) = list_row(ui, h, on, s.label());
            put(ui.painter(), rect.left() + 14.0, rect.width() - 24.0, rect.center().y, s.label(), body(14.0), if on { VALUE } else { LEGEND });
            if resp.clicked() {
                self.page.open_section(s);
            }
        }
    }

    fn page_body(&mut self, ui: &mut Ui, store: &Store) {
        if self.page.section == Section::Accounts {
            self.accounts_page(ui, store);
            return;
        }
        egui::ScrollArea::vertical().id_salt("settings-page").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(DETAIL_MAX.min(ui.available_width()));
            match self.page.section {
                Section::General => self.panels.general(ui),
                Section::Security => self.security(ui, store),
                Section::MailHandling => self.mail_handling(ui, store),
                Section::Agents => {
                    let accounts = self.drafts.clone();
                    let narrow = self.page.narrow;
                    self.agents.show(ui, store, &accounts, narrow);
                }
                Section::Updates => self.updates(ui),
                Section::About => self.about(ui),
                Section::Accounts => {}
            }
            ui.add_space(16.0);
        });
    }

    fn security(&mut self, ui: &mut Ui, store: &Store) {
        section(ui, "unlock", "how mailbench opens its encrypted data", |ui| {
            reading(ui, "keychain", on_off(self.methods.keychain));
            reading(ui, "password", on_off(self.methods.password));
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
            ui.horizontal_wrapped(|ui| {
                let label = if self.methods.password { "change password" } else { "add password" };
                if ui.add(Key::new(label).outline(VALUE).enabled(self.password_problem().is_none())).clicked() {
                    self.acts.set_password = true;
                }
                if self.methods.password && ui.add(Key::new("remove password").outline(FAULT).enabled(self.methods.keychain)).clicked() {
                    self.acts.drop_password = true;
                }
                if self.methods.keychain {
                    if ui.add(Key::new("remove keychain unlock").outline(FAULT).enabled(self.methods.password)).clicked() {
                        self.acts.drop_keychain = true;
                    }
                } else if ui.add(Key::new("add keychain unlock").outline(VALUE)).clicked() {
                    self.acts.enroll = true;
                }
            });
            hint(ui, "At least one way to unlock must stay on.");
            if let Some((ok, text)) = &self.message {
                status(ui, *ok, text);
            }
        });
        ui.add_space(8.0);
        self.panels.keys_panel(ui, store);
    }

    fn mail_handling(&mut self, ui: &mut Ui, store: &Store) {
        let accounts = self.drafts.clone();
        section(ui, "attachments", "what happens when you open a file", |ui| {
            if ui.checkbox(&mut self.allow_open, "allow open (writes a plain copy to a temp folder)").changed() {
                let _ = store.put(SETTINGS_KIND, ALLOW_OPEN_KEY, if self.allow_open { b"1" } else { b"0" });
                self.acts.allow_open_changed = true;
            }
            hint(ui, "Save as always works. Copies made by open are deleted when mailbench closes and on the next start.");
        });
        ui.add_space(8.0);
        self.panels.mail(ui, store, &accounts);
    }

    fn updates(&mut self, ui: &mut Ui) {
        section(ui, "updates", "new versions are published on GitHub releases", |ui| {
            reading(ui, "version", env!("CARGO_PKG_VERSION"));
            ui.horizontal_wrapped(|ui| {
                if ui.add(Key::new("check for updates").outline(VALUE).enabled(!self.updater.busy())).clicked() {
                    self.updater.check(ui.ctx());
                }
                if ui.add(Key::new("releases page").outline(VALUE)).clicked() {
                    let _ = open::that(self.updater.page());
                }
            });
            self.updater.status(ui);
        });
    }

    fn about(&mut self, ui: &mut Ui) {
        section(ui, "about", "mailbench", |ui| {
            reading(ui, "version", env!("CARGO_PKG_VERSION"));
            reading(ui, "source", format!("github.com/{UPDATE_REPO}"));
            reading(ui, "data folder", self.dir.display().to_string());
            hint(ui, "Everything in the data folder is encrypted: messages, attachments, the search index, saved passwords and settings.");
        });
    }

    fn accounts_page(&mut self, ui: &mut Ui, store: &Store) {
        let narrow = self.page.narrow;
        if narrow {
            egui::ScrollArea::vertical().id_salt("accounts-narrow").auto_shrink([false, false]).show(ui, |ui| {
                if self.page.level == Level::Account && self.page.showing_account() {
                    self.account_detail(ui, store);
                } else {
                    self.accounts_list(ui);
                }
                ui.add_space(16.0);
            });
            return;
        }
        let total = ui.available_width();
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(LIST_W.min(total * 0.4), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("accounts-list").auto_shrink([false, false]).show(ui, |ui| self.accounts_list(ui));
            });
            ui.add_space(8.0);
            let rest = ui.available_width().min(DETAIL_MAX + 40.0);
            ui.allocate_ui_with_layout(egui::vec2(rest, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("accounts-detail").auto_shrink([false, false]).show(ui, |ui| {
                    self.account_detail(ui, store);
                    ui.add_space(16.0);
                });
            });
        });
    }

    fn accounts_list(&mut self, ui: &mut Ui) {
        if ui.add(Key::new("add account").outline(VALUE)).clicked() {
            self.acts.add = true;
        }
        ui.add_space(6.0);
        if let Some((ok, text)) = &self.message {
            status(ui, *ok, text);
        }
        if self.drafts.is_empty() {
            hint(ui, "No accounts yet.");
        }
        ui.spacing_mut().item_spacing.y = 2.0;
        let h = if self.page.narrow { PHONE_ROW } else { 52.0 };
        let mut clicked = None;
        for (i, d) in self.drafts.iter().enumerate() {
            let info = self.info_of(&d.id);
            let (state, tint) = state_of(info);
            let unread = info.map(|x| x.unread).unwrap_or(0);
            let on = i == self.page.account;
            let label = format!("account {}, {}, {}", d.name, d.address, kind_of(d));
            let (rect, resp) = list_row(ui, h, on, &label);
            let p = ui.painter();
            dot(p, egui::pos2(rect.left() + 14.0, rect.center().y - 7.0), tint);
            let text_w = (rect.width() - 36.0 - 84.0).max(40.0);
            put(p, rect.left() + 28.0, text_w, rect.center().y - 8.0, shown_name(d), body(13.5), if on { VALUE } else { LEGEND });
            put(p, rect.left() + 28.0, text_w, rect.center().y + 9.0, &d.address, body(11.0), LEGEND);
            put_right(p, rect.right() - 8.0, rect.center().y - 8.0, kind_of(d), legend_font(10.0), VALUE);
            let second = if unread > 0 { format!("{unread} unread") } else { state.to_string() };
            put_right(p, rect.right() - 8.0, rect.center().y + 9.0, &second, body(10.5), if unread > 0 { READOUT } else { tint });
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.page.select_account(i);
        }
        if self.drafts.len() > 1 {
            ui.add_space(8.0);
            let i = self.page.account;
            ui.horizontal_wrapped(|ui| {
                if ui.add(Key::new("move up").outline(VALUE).enabled(i > 0)).clicked() {
                    self.acts.shift = Some(-1);
                }
                if ui.add(Key::new("move down").outline(VALUE).enabled(i + 1 < self.drafts.len())).clicked() {
                    self.acts.shift = Some(1);
                }
            });
            hint(ui, "The order here is the order in the sidebar. The first account is the default.");
        }
    }

    fn account_detail(&mut self, ui: &mut Ui, store: &Store) {
        let i = self.page.account;
        if self.drafts.is_empty() {
            hint(ui, "Add an account to manage it here.");
            return;
        }
        let id = self.drafts[i].id.clone();
        let info = self.info_of(&id).cloned();
        let (state, tint) = state_of(info.as_ref());
        ui.horizontal_wrapped(|ui| {
            Line::new().value(shown_name(&self.drafts[i]).to_string()).size(16.0).show(ui);
            chip(ui, kind_of(&self.drafts[i]), TRACE, true);
            chip(ui, state, tint, true);
        });
        Line::new().note(self.drafts[i].address.clone()).size(11.0).show(ui);
        ui.add_space(6.0);
        if let Some(info) = info.as_ref().filter(|x| needs_sign_in(x)) {
            let detail = info.detail.clone().unwrap_or_default();
            card(
                ui,
                Some(FAULT),
                |ui| {
                    Line::new().legend("sign in failed").tint(FAULT).show(ui);
                },
                |ui| {
                    note(ui, format!("{} needs you to sign in again. Your stored mail is still readable and new mail is paused for this account.", self.drafts[i].address), VALUE);
                    hint(ui, &detail);
                    if let Some(e) = &self.sign_error {
                        status(ui, false, e);
                    }
                    if self.signing.is_some() {
                        Line::new().measured("waiting for your browser").size(11.0).show(ui);
                    } else if ui.add(Key::new("sign in again").solid(READOUT)).clicked() {
                        self.acts.sign_in = Some(i);
                    }
                },
            );
            ui.add_space(6.0);
        } else if let Some(info) = info.as_ref().filter(|x| x.sync == SyncState::Failed) {
            status(ui, false, info.detail.as_deref().unwrap_or("The last sync failed"));
        }
        let mut tab = self.page.tab;
        tabs(ui, &mut tab, &Tab::ALL.map(|t| (t, t.label())));
        self.page.tab = tab;
        match self.page.tab {
            Tab::Overview => self.tab_overview(ui, i),
            Tab::Servers => self.tab_servers(ui, i),
            Tab::Sync => self.tab_sync(ui, i, store),
            Tab::Notifications => self.tab_notifications(ui, i),
            Tab::Danger => self.tab_danger(ui, i),
        }
    }

    fn tab_overview(&mut self, ui: &mut Ui, i: usize) {
        let d = &mut self.drafts[i];
        row(ui, "name", |ui| {
            field(ui, &mut d.name, "shown in the account list");
        });
        issue(ui, d, Field::Name);
        row(ui, "display name", |ui| {
            field(ui, &mut d.display_name, "shown to people you write to");
        });
        if d.is_nostr() {
            reading(ui, "address", d.address.clone());
        } else {
            row(ui, "address", |ui| {
                field(ui, &mut d.address, "you@example.com");
            });
            issue(ui, d, Field::Address);
        }
        row(ui, "signature", |ui| {
            prose(ui, &mut d.signature, "added to new messages", 3);
        });
        ui.add_space(4.0);
        if i == 0 {
            status(ui, true, "This is the default account for new messages");
        } else if ui.add(Key::new("make default").outline(VALUE)).clicked() {
            self.acts.make_default = Some(i);
        }
    }

    fn tab_servers(&mut self, ui: &mut Ui, i: usize) {
        let d = &mut self.drafts[i];
        let security = |s: &Security| [Security::Tls, Security::StartTls, Security::Plain].map(|x| (x, sec_label(s).clone())).len();
        let _ = security;
        let options = || [Security::Tls, Security::StartTls, Security::Plain].map(|s| (s.clone(), sec_label(&s)));
        match &mut d.credential {
            Credential::Chat { backend } => {
                hint(ui, &format!("Chat backend: {backend}. It has no servers to configure."));
            }
            Credential::Jmap { url, password } => {
                row(ui, "jmap url", |ui| {
                    field(ui, url, "https://mail.example.com/jmap/session");
                });
                row(ui, "user name", |ui| {
                    field(ui, &mut d.username, "usually your email address");
                });
                row(ui, "password", |ui| {
                    secret(ui, password);
                });
            }
            Credential::Nostr { relays, .. } | Credential::NostrBunker { relays, .. } => {
                let text = self.relay_text.entry(d.id.clone()).or_insert_with(|| relays.join("\n"));
                row(ui, "relays", |ui| {
                    if prose(ui, text, "one relay URL per line", 4).changed() {
                        *relays = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                    }
                });
                reading(ui, "key", d.address.clone());
            }
            Credential::OAuth { provider, .. } => {
                row(ui, "user name", |ui| {
                    field(ui, &mut d.username, "usually your email address");
                });
                status(ui, true, &format!("Signed in with {}", provider.name()));
                if self.signing.is_some() {
                    Line::new().measured(format!("waiting for {} in your browser", provider.name())).size(11.0).show(ui);
                } else if ui.add(Key::new("sign in again").solid(READOUT)).clicked() {
                    self.acts.sign_in = Some(i);
                }
                if let Some(e) = &self.sign_error {
                    status(ui, false, e);
                }
            }
            Credential::Password(p) => {
                row(ui, "user name", |ui| {
                    field(ui, &mut d.username, "usually your email address");
                });
                row(ui, "password", |ui| {
                    secret(ui, p);
                });
            }
        }
        if !d.is_nostr() && !d.is_jmap() {
            ui.add_space(4.0);
            row(ui, "imap host", |ui| {
                field(ui, &mut d.imap.host, "imap.example.com");
            });
            issue(ui, d, Field::ImapHost);
            row(ui, "imap port", |ui| {
                let r = ui.add(egui::DragValue::new(&mut d.imap.port).range(1..=65535).speed(1.0));
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("imap port"));
            });
            row(ui, "imap security", |ui| {
                choice(ui, format!("imap-security-{i}"), &mut d.imap.security, options());
            });
            row(ui, "smtp host", |ui| {
                field(ui, &mut d.smtp.host, "smtp.example.com");
            });
            issue(ui, d, Field::SmtpHost);
            row(ui, "smtp port", |ui| {
                let r = ui.add(egui::DragValue::new(&mut d.smtp.port).range(1..=65535).speed(1.0));
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("smtp port"));
            });
            row(ui, "smtp security", |ui| {
                choice(ui, format!("smtp-security-{i}"), &mut d.smtp.security, options());
            });
        }
        if d.is_jmap() {
            issue(ui, d, Field::JmapUrl);
        }
        if let Some((rid, run)) = &self.retest {
            if *rid == d.id {
                run.show(ui, Some(d), false);
            }
        }
        ui.add_space(4.0);
        if ui.add(Key::new("re-test").outline(VALUE).id_salt(("retest", i))).clicked() {
            self.acts.retest = Some(i);
        }
    }

    fn tab_sync(&mut self, ui: &mut Ui, i: usize, store: &Store) {
        let d = &mut self.drafts[i];
        section(ui, "sync", "what is fetched and how often", |ui| {
            self.panels.account_sync(ui, d, i);
        });
        ui.add_space(8.0);
        section(ui, "offline search", "index message bodies for search without the server", |ui| {
            ui.checkbox(&mut d.backfill.enabled, "index bodies offline");
            if d.backfill.enabled {
                ui.horizontal_wrapped(|ui| {
                    let r = ui.add(egui::DragValue::new(&mut d.backfill.days).range(0..=36500).speed(5.0).suffix(" days"));
                    ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("keep days, 0 for all"));
                    let r = ui.add(egui::DragValue::new(&mut d.backfill.megabytes).range(0..=1_000_000).speed(20.0).suffix(" MB"));
                    ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("size cap, 0 for none"));
                });
            }
        });
        ui.add_space(8.0);
        let id = d.id.clone();
        let (messages, bodies) = *self.disk.entry(id.clone()).or_insert_with(|| StoreSink::local_counts(store, &id));
        section(ui, "on disk", "this account's share of the encrypted store", |ui| {
            reading(ui, "messages", messages.to_string());
            reading(ui, "bodies kept", bodies.to_string());
            if ui.add(Key::new("clear local mail").outline(WARN).id_salt(("clear", i))).clicked() {
                self.acts.clear = Some(i);
            }
            hint(ui, "Deletes this account's mail from this machine. It is fetched again on the next sync.");
        });
    }

    fn tab_notifications(&mut self, ui: &mut Ui, i: usize) {
        let d = &mut self.drafts[i];
        ui.checkbox(&mut d.notify.enabled, "notifications");
        if d.notify.enabled {
            let mut quiet = d.notify.quiet.is_some();
            ui.checkbox(&mut quiet, "quiet hours");
            if quiet != d.notify.quiet.is_some() {
                d.notify.quiet = quiet.then_some(crate::notify::QuietHours { start: 22 * 60, end: 7 * 60 });
            }
            if let Some(q) = d.notify.quiet.as_mut() {
                let (mut from, mut to) = (q.start / 60, q.end / 60);
                ui.horizontal_wrapped(|ui| {
                    let r = ui.add(egui::DragValue::new(&mut from).range(0..=23).suffix(":00"));
                    ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("quiet from hour"));
                    let r = ui.add(egui::DragValue::new(&mut to).range(0..=23).suffix(":00"));
                    ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("quiet until hour"));
                });
                q.start = from * 60;
                q.end = to * 60;
            }
        }
    }

    fn tab_danger(&mut self, ui: &mut Ui, i: usize) {
        let d = &self.drafts[i];
        if let Credential::Nostr { secret, .. } = &d.credential {
            section(ui, "nostr key", "anyone with this key can use the account", |ui| {
                let nsec = nostr_sdk::Keys::parse(secret).ok().and_then(|k| nostr_sdk::ToBech32::to_bech32(k.secret_key()).ok());
                match (&self.exported, nsec) {
                    (Some(text), _) => {
                        Line::new().value(text.clone()).size(11.0).wrapped(ui);
                        if ui.add(Key::new("hide key").outline(VALUE)).clicked() {
                            self.exported = None;
                        }
                    }
                    (None, Some(nsec)) => {
                        if ui.add(Key::new("export key").outline(WARN)).clicked() {
                            self.exported = Some(nsec);
                        }
                    }
                    (None, None) => hint(ui, "The key could not be read."),
                }
            });
            ui.add_space(8.0);
        }
        section(ui, "remove account", "deletes the synced mail on this machine, not on the server", |ui| {
            if ui.add(Key::new("remove").outline(FAULT).id_salt(("remove", i))).clicked() {
                self.acts.remove = Some(i);
            }
        });
    }
}
