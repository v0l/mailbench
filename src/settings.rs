use crate::accounts::AccountConfig;
use crate::backend::StoreSink;
use crate::prefs::{self, Bindings, BodyPolicy, Density, Prefs, RemoteImages, Theme};
use crate::store::Store;
use egui::Ui;
use egui_bench::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

const BYTES_PER_MB: f64 = 1_048_576.0;
const DEFAULT_FREE_DAYS: u32 = 365;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub bytes: u64,
    pub messages: u64,
    pub bodies: u64,
}

impl StoreStats {
    pub fn read(store: &Store) -> Self {
        StoreStats {
            bytes: store.size_on_disk(),
            messages: store.count("message").unwrap_or(0),
            bodies: store.count("body").unwrap_or(0),
        }
    }

    pub fn size_text(&self) -> String {
        format!("{:.1} MB", self.bytes as f64 / BYTES_PER_MB)
    }
}

#[derive(Default)]
pub struct Outcome {
    pub prefs_changed: bool,
    pub bodies_changed: bool,
}

pub struct Panels {
    pub prefs: Prefs,
    saved: Prefs,
    pub stats: StoreStats,
    pub backup_path: String,
    pub free_days: u32,
    pub capturing: Option<&'static str>,
    pub note: Option<(bool, String)>,
    folders: HashMap<String, Vec<String>>,
    pub key_path: String,
    pub key_secret: String,
    pub key_note: Option<(bool, String)>,
    pub keys: Vec<crate::crypt::KeyInfo>,
}

pub fn default_backup_path(dir: &std::path::Path) -> PathBuf {
    dir.join("mailbench-backup.db")
}

impl Panels {
    pub fn new(store: &Store, accounts: &[AccountConfig], backup: PathBuf) -> Self {
        let prefs = prefs::load(store);
        let folders = accounts
            .iter()
            .map(|a| {
                let names = StoreSink::new(store, &a.id, true)
                    .folders()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|f| f.selectable)
                    .map(|f| f.name)
                    .collect();
                (a.id.clone(), names)
            })
            .collect();
        Panels {
            saved: prefs.clone(),
            prefs,
            stats: StoreStats::read(store),
            backup_path: backup.display().to_string(),
            free_days: DEFAULT_FREE_DAYS,
            capturing: None,
            note: None,
            folders,
            key_path: String::new(),
            key_secret: String::new(),
            key_note: None,
            keys: crate::crypt::list_keys(store).unwrap_or_default(),
        }
    }

    pub fn folders_of(&self, account: &str) -> &[String] {
        self.folders.get(account).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn account_sync(&self, ui: &mut Ui, draft: &mut AccountConfig, index: usize) {
        if draft.is_nostr() {
            hint(ui, "Nostr accounts follow their relays, so there is no sync window or poll interval.");
            return;
        }
        let folders = self.folders_of(&draft.id);
        row(ui, "sync window", |ui| {
            let r = ui.add(egui::DragValue::new(&mut draft.sync.window_months).range(0..=240).speed(0.2).suffix(" months"));
            ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("sync window months, 0 for all"));
        });
        row(ui, "poll interval", |ui| {
            let r = ui.add(egui::DragValue::new(&mut draft.sync.poll_seconds).range(0..=3600).speed(1.0).suffix(" s"));
            ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("poll seconds, 0 for the default"));
        });
        hint(ui, "The poll interval applies when the server has no IDLE push. Only mail inside the window is fetched.");
        if folders.is_empty() {
            hint(ui, "Folders to sync are listed here after the first sync.");
            return;
        }
        egui::CollapsingHeader::new("folders to sync").id_salt(("folders", index)).show(ui, |ui| {
            for name in folders {
                let mut on = draft.sync.syncs(name);
                let inbox = name.eq_ignore_ascii_case("INBOX");
                if ui.add_enabled(!inbox, egui::Checkbox::new(&mut on, format!("sync {name}"))).changed() {
                    draft.sync.set_syncs(name, on);
                }
            }
        });
    }

    pub fn general(&mut self, ui: &mut Ui) {
        self.appearance(ui);
        ui.add_space(8.0);
        self.reading(ui);
        ui.add_space(8.0);
        self.keyboard(ui);
    }

    pub fn mail(&mut self, ui: &mut Ui, store: &Store, accounts: &[AccountConfig]) {
        self.remote_images(ui);
        ui.add_space(8.0);
        self.writing(ui);
        ui.add_space(8.0);
        self.bodies(ui, store, accounts);
        ui.add_space(8.0);
        self.store_panel(ui, store, accounts);
    }

    pub fn flush(&mut self, store: &Store) -> Outcome {
        let mut out = Outcome::default();
        if self.prefs != self.saved {
            match prefs::save(store, &self.prefs) {
                Ok(()) => {
                    out.bodies_changed = self.prefs.bodies != self.saved.bodies;
                    out.prefs_changed = true;
                    self.saved = self.prefs.clone();
                }
                Err(e) => self.note = Some((false, format!("Could not save settings: {e}"))),
            }
        }
        out
    }

    fn appearance(&mut self, ui: &mut Ui) {
        section(ui, "appearance", "theme and list density", |ui| {
            row(ui, "theme", |ui| {
                for option in Theme::ALL {
                    ui.selectable_value(&mut self.prefs.theme, option, format!("theme {}", option.label()));
                }
            });
            row(ui, "density", |ui| {
                for option in Density::ALL {
                    ui.selectable_value(&mut self.prefs.density, option, format!("density {}", option.label()));
                }
            });
            hint(ui, "Density sets the row height in the list and the spacing in the reader.");
        });
    }

    fn bodies(&mut self, ui: &mut Ui, store: &Store, accounts: &[AccountConfig]) {
        section(ui, "message bodies", "what stays on this machine", |ui| {
            let mut mode = match self.prefs.bodies {
                BodyPolicy::KeepAll => 0,
                BodyPolicy::KeepDays(_) => 1,
                BodyPolicy::HeadersOnly => 2,
            };
            ui.horizontal(|ui| {
                ui.radio_value(&mut mode, 0, "keep all");
                ui.radio_value(&mut mode, 1, "keep the last days");
                ui.radio_value(&mut mode, 2, "headers only (fetch on open)");
            });
            let days = self.prefs.bodies.keep_days().unwrap_or(90);
            self.prefs.bodies = match mode {
                0 => BodyPolicy::KeepAll,
                1 => BodyPolicy::KeepDays(days),
                _ => BodyPolicy::HeadersOnly,
            };
            if let BodyPolicy::KeepDays(days) = &mut self.prefs.bodies {
                row(ui, "keep days", |ui| {
                    let r = ui.add(egui::DragValue::new(days).range(1..=36500).speed(5.0).suffix(" days"));
                    ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("keep bodies days"));
                });
            }
            ui.add_space(4.0);
            row(ui, "older than", |ui| {
                let r = ui.add(egui::DragValue::new(&mut self.free_days).range(0..=36500).speed(5.0).suffix(" days"));
                ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("free space older than days"));
                if ui.add(Key::new("free space").outline(WARN)).clicked() {
                    self.free_space(store, accounts);
                }
            });
            hint(ui, "Drops bodies older than this from the store and compacts it. Headers stay, and bodies are fetched again when you open the message.");
        });
    }

    pub fn free_space(&mut self, store: &Store, accounts: &[AccountConfig]) {
        let cutoff = chrono::Utc::now().timestamp() - self.free_days as i64 * 86_400;
        let mut dropped = 0;
        for a in accounts {
            dropped += StoreSink::new(store, &a.id, true).drop_bodies_before(cutoff).unwrap_or(0);
        }
        let result = store.vacuum();
        self.stats = StoreStats::read(store);
        self.note = Some(match result {
            Ok(()) => (true, format!("Dropped {dropped} bodies and compacted the store")),
            Err(e) => (false, format!("Dropped {dropped} bodies, but compacting failed: {e}")),
        });
    }

    pub fn reindex(&mut self, store: &Store, accounts: &[AccountConfig]) {
        let result = store.clear_index().map_err(|e| e.to_string()).and_then(|_| {
            let mut total = 0;
            for a in accounts {
                total += StoreSink::new(store, &a.id, true).reindex().map_err(|e| e.to_string())?;
            }
            Ok(total)
        });
        self.stats = StoreStats::read(store);
        self.note = Some(match result {
            Ok(n) => (true, format!("Search index rebuilt for {n} messages")),
            Err(e) => (false, format!("Could not rebuild the index: {e}")),
        });
    }

    pub fn backup(&mut self, store: &Store) {
        let path = PathBuf::from(self.backup_path.trim());
        self.note = Some(match store.backup_to(&path) {
            Ok(()) => (true, format!("Encrypted backup written to {}", path.display())),
            Err(e) => (false, format!("Could not write the backup: {e}")),
        });
    }

    fn store_panel(&mut self, ui: &mut Ui, store: &Store, accounts: &[AccountConfig]) {
        section(ui, "store", "the encrypted database on disk", |ui| {
            reading(ui, "size on disk", self.stats.size_text());
            reading(ui, "messages", self.stats.messages.to_string());
            reading(ui, "bodies kept", self.stats.bodies.to_string());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add(Key::new("re-index").outline(VALUE)).clicked() {
                    self.reindex(store, accounts);
                }
            });
            row(ui, "backup file", |ui| {
                field(ui, &mut self.backup_path, "where to write the copy");
            });
            ui.horizontal(|ui| {
                if ui.add(Key::new("export backup").outline(VALUE)).clicked() {
                    self.backup(store);
                }
            });
            hint(ui, "The copy stays encrypted with the same unlock methods as this store.");
            if let Some((ok, text)) = &self.note {
                status(ui, *ok, text);
            }
        });
    }

    pub fn import_key(&mut self, store: &Store, pkcs12: bool) {
        use crate::crypt;
        let path = PathBuf::from(self.key_path.trim());
        let result = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display())).and_then(|data| {
            let r = if pkcs12 { crypt::import_pkcs12(store, &data, &self.key_secret) } else { crypt::import_pgp(store, &data, &self.key_secret) };
            r.map_err(|e| e.to_string())
        });
        self.key_note = Some(match result {
            Ok(info) => {
                self.key_path.clear();
                self.key_secret.clear();
                (true, format!("Imported {}", if info.label.is_empty() { info.id } else { info.label }))
            }
            Err(e) => (false, e),
        });
        self.keys = crypt::list_keys(store).unwrap_or_default();
    }

    pub fn keys_panel(&mut self, ui: &mut Ui, store: &Store) {
        section(ui, "keys", "PGP and S/MIME keys kept in the encrypted store", |ui| {
            if self.keys.is_empty() {
                hint(ui, "No keys imported. Imported keys decrypt mail and never leave the store.");
            }
            let mut remove = None;
            for k in &self.keys {
                ui.horizontal_wrapped(|ui| {
                    let kind = if k.kind == crate::crypt::PGP_KIND { "pgp" } else { "s/mime" };
                    Line::new().legend(kind).show(ui);
                    Line::new().value(if k.label.is_empty() { &k.id } else { &k.label }).size(12.0).show(ui);
                    if ui.add(Key::new("remove").outline(LEGEND).id_salt(("remove key", &k.id))).clicked() {
                        remove = Some((k.kind, k.id.clone()));
                    }
                });
            }
            if let Some((kind, id)) = remove {
                let _ = crate::crypt::remove_key(store, kind, &id);
                self.keys = crate::crypt::list_keys(store).unwrap_or_default();
            }
            ui.add_space(4.0);
            row(ui, "key file", |ui| {
                field(ui, &mut self.key_path, "path to a PGP key or .p12 file");
            });
            row(ui, "passphrase", |ui| {
                secret(ui, &mut self.key_secret);
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add(Key::new("import pgp key").outline(VALUE)).clicked() {
                    self.import_key(store, false);
                }
                if ui.add(Key::new("import pkcs12").outline(VALUE)).clicked() {
                    self.import_key(store, true);
                }
            });
            hint(ui, "The passphrase unlocks the file once. The key is then stored without it, inside the encrypted store.");
            if let Some((ok, text)) = &self.key_note {
                status(ui, *ok, text);
            }
        });
    }

    fn reading(&mut self, ui: &mut Ui) {
        section(ui, "reading", "how messages are shown", |ui| {
            let reading = &mut self.prefs.reading;
            row(ui, "default zoom", |ui| {
                let r = ui.add(egui::DragValue::new(&mut reading.zoom_percent).range(prefs::MIN_ZOOM..=prefs::MAX_ZOOM).speed(1.0).suffix("%"));
                ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("default zoom percent"));
            });
            row(ui, "mark as read", |ui| {
                for option in prefs::MarkRead::ALL {
                    ui.selectable_value(&mut reading.mark_read, option, option.label());
                }
            });
            if reading.mark_read == prefs::MarkRead::AfterDelay {
                row(ui, "mark read delay", |ui| {
                    let r = ui.add(egui::DragValue::new(&mut reading.mark_read_secs).range(1..=60).suffix(" s"));
                    ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("mark read delay seconds"));
                });
            }
            row(ui, "reader width", |ui| {
                for option in prefs::ReaderWidth::ALL {
                    ui.selectable_value(&mut reading.reader_width, option, format!("width {}", option.label()));
                }
            });
            ui.checkbox(&mut reading.dark_html, "dark reading for HTML");
        });
    }

    fn remote_images(&mut self, ui: &mut Ui) {
        section(ui, "remote images", "pictures that load from the web", |ui| {
            let reading = &mut self.prefs.reading;
            row(ui, "remote images", |ui| {
                for option in RemoteImages::ALL {
                    ui.selectable_value(&mut reading.remote_images, option, format!("images {}", option.label()));
                }
            });
            hint(ui, "Trusted senders and domains are chosen from the banner on a message and apply when this is set to ask or trusted.");
        });
    }

    fn writing(&mut self, ui: &mut Ui) {
        section(ui, "writing", "how messages are sent", |ui| {
            ui.checkbox(&mut self.prefs.plain_text_only, "send as plain text only");
            hint(ui, "Off: messages with bold, links, lists, quotes or code also carry an HTML copy. On: only the text you typed is sent.");
        });
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        section(ui, "keyboard", "the same keys as the hint bar", |ui| {
            let ctx = ui.ctx().clone();
            if let Some(id) = self.capturing {
                let pressed = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.alt => Some(*key),
                        _ => None,
                    })
                });
                if let Some(key) = pressed {
                    self.capturing = None;
                    if key != egui::Key::Escape {
                        self.note = Some(match self.prefs.keys.set(id, key) {
                            Ok(()) => (true, format!("{} is now {}", label_of(id), prefs::key_label(key))),
                            Err(e) => (false, e),
                        });
                    }
                }
            }
            let rows = self.prefs.keys.rows();
            for (id, label, key) in rows {
                ui.horizontal(|ui| {
                    Line::new().legend(&label).show(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let Some(id) = id else {
                            Line::new().value(&key).size(12.0).show(ui);
                            return;
                        };
                        if !self.prefs.keys.is_default(id) && ui.add(Key::new("reset").outline(LEGEND).id_salt(("reset", id))).clicked() {
                            self.prefs.keys.reset(id);
                        }
                        let waiting = self.capturing == Some(id);
                        let text = if waiting { "press a key".to_string() } else { key.clone() };
                        let resp = ui.add(Key::new(text).outline(VALUE).id_salt(("rebind", id)).min_width(90.0));
                        let name = format!("rebind {label}");
                        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label(name));
                        if resp.clicked() {
                            self.capturing = if waiting { None } else { Some(id) };
                        }
                    });
                });
            }
            if self.capturing.is_some() {
                hint(ui, "Press the new key. Escape cancels.");
            }
            if ui.add(Key::new("reset all keys").outline(LEGEND)).clicked() {
                self.prefs.keys = Bindings::default();
            }
        });
    }
}

fn label_of(id: &str) -> &'static str {
    prefs::COMMANDS.iter().find(|c| c.id == id).map(|c| c.label).unwrap_or("key")
}
