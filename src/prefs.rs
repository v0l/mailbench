use crate::search::BackfillPolicy;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

pub const SETTINGS_KIND: &str = "settings";
pub const PREFS_KEY: &str = "prefs";
const LEGACY_DARK_KEY: &str = "reading.dark";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BodyPolicy {
    #[default]
    KeepAll,
    KeepDays(u32),
    HeadersOnly,
}

impl BodyPolicy {
    pub fn effective_backfill(&self, account: BackfillPolicy) -> BackfillPolicy {
        match self {
            BodyPolicy::KeepAll if account.enabled => account,
            BodyPolicy::KeepAll => BackfillPolicy { days: 0, ..account },
            BodyPolicy::KeepDays(n) => BackfillPolicy { enabled: true, days: (*n).max(1), ..account },
            BodyPolicy::HeadersOnly => BackfillPolicy { enabled: false, days: 0, ..account },
        }
    }

    pub fn keep_days(&self) -> Option<u32> {
        match self {
            BodyPolicy::KeepDays(n) => Some(*n),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RemoteImages {
    Never,
    #[default]
    Ask,
    Trusted,
}

impl RemoteImages {
    pub const ALL: [RemoteImages; 3] = [RemoteImages::Never, RemoteImages::Ask, RemoteImages::Trusted];

    pub fn label(&self) -> &'static str {
        match self {
            RemoteImages::Never => "never",
            RemoteImages::Ask => "ask",
            RemoteImages::Trusted => "trusted senders only",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MarkRead {
    Immediately,
    #[default]
    AfterDelay,
    Never,
}

impl MarkRead {
    pub const ALL: [MarkRead; 3] = [MarkRead::Immediately, MarkRead::AfterDelay, MarkRead::Never];

    pub fn label(&self) -> &'static str {
        match self {
            MarkRead::Immediately => "immediately",
            MarkRead::AfterDelay => "after a delay",
            MarkRead::Never => "never (only on open)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReaderWidth {
    Comfortable,
    #[default]
    Full,
}

impl ReaderWidth {
    pub const ALL: [ReaderWidth; 2] = [ReaderWidth::Comfortable, ReaderWidth::Full];

    pub fn label(&self) -> &'static str {
        match self {
            ReaderWidth::Comfortable => "comfortable",
            ReaderWidth::Full => "full width",
        }
    }

    pub fn toggled(&self) -> ReaderWidth {
        match self {
            ReaderWidth::Comfortable => ReaderWidth::Full,
            ReaderWidth::Full => ReaderWidth::Comfortable,
        }
    }
}

pub const SIDE_DEFAULT: u32 = 196;
pub const SIDE_RANGE: (f32, f32) = (140.0, 340.0);
pub const LIST_DEFAULT: u32 = 540;
pub const LIST_RANGE: (f32, f32) = (300.0, 800.0);

fn side_default() -> u32 {
    SIDE_DEFAULT
}

fn list_default() -> u32 {
    LIST_DEFAULT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutPrefs {
    #[serde(default = "side_default")]
    pub side_w: u32,
    #[serde(default = "list_default")]
    pub list_w: u32,
}

impl Default for LayoutPrefs {
    fn default() -> Self {
        LayoutPrefs { side_w: SIDE_DEFAULT, list_w: LIST_DEFAULT }
    }
}

fn one_second() -> u32 {
    1
}

pub const MIN_ZOOM: u32 = 60;
pub const MAX_ZOOM: u32 = 250;

fn full_zoom() -> u32 {
    100
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingPrefs {
    #[serde(default = "full_zoom")]
    pub zoom_percent: u32,
    #[serde(default)]
    pub remote_images: RemoteImages,
    #[serde(default)]
    pub dark_html: bool,
    #[serde(default)]
    pub mark_read: MarkRead,
    #[serde(default = "one_second")]
    pub mark_read_secs: u32,
    #[serde(default)]
    pub reader_width: ReaderWidth,
}

impl Default for ReadingPrefs {
    fn default() -> Self {
        ReadingPrefs {
            zoom_percent: 100,
            remote_images: RemoteImages::Ask,
            dark_html: false,
            mark_read: MarkRead::AfterDelay,
            mark_read_secs: 1,
            reader_width: ReaderWidth::Full,
        }
    }
}

impl ReadingPrefs {
    pub fn zoom(&self) -> f32 {
        self.zoom_percent.clamp(MIN_ZOOM, MAX_ZOOM) as f32 / 100.0
    }

    pub fn mark_read_after(&self) -> Option<Duration> {
        match self.mark_read {
            MarkRead::Immediately => Some(Duration::ZERO),
            MarkRead::AfterDelay => Some(Duration::from_secs(self.mark_read_secs as u64)),
            MarkRead::Never => None,
        }
    }

    pub fn zoom_milli(&self) -> u32 {
        self.zoom_percent.clamp(MIN_ZOOM, MAX_ZOOM) * 10
    }
}

pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub default: egui::Key,
    pub hint: bool,
}

pub const COMMANDS: [Command; 15] = [
    Command { id: "next", label: "next message", default: egui::Key::J, hint: true },
    Command { id: "previous", label: "previous message", default: egui::Key::K, hint: true },
    Command { id: "open", label: "open", default: egui::Key::Enter, hint: true },
    Command { id: "archive", label: "archive", default: egui::Key::E, hint: true },
    Command { id: "reply", label: "reply", default: egui::Key::R, hint: true },
    Command { id: "reply_all", label: "reply all", default: egui::Key::A, hint: false },
    Command { id: "forward", label: "forward", default: egui::Key::F, hint: false },
    Command { id: "snooze", label: "snooze", default: egui::Key::S, hint: true },
    Command { id: "flag", label: "flag", default: egui::Key::X, hint: true },
    Command { id: "unread", label: "toggle unread", default: egui::Key::U, hint: false },
    Command { id: "search", label: "search", default: egui::Key::Slash, hint: true },
    Command { id: "move", label: "move to folder", default: egui::Key::M, hint: true },
    Command { id: "compose", label: "compose", default: egui::Key::C, hint: true },
    Command { id: "reader_width", label: "reader width", default: egui::Key::W, hint: true },
    Command { id: "account_only", label: "this account only", default: egui::Key::V, hint: true },
];

pub const FIXED: [(&str, &str, bool); 9] = [
    ("n p", "next or previous conversation", false),
    (". ,", "expand or collapse all", false),
    ("Enter", "open or fold a message card", false),
    ("g i", "go to inbox", false),
    ("g a", "go to archive", false),
    ("o", "open file", true),
    ("S", "save files", true),
    ("#", "move to trash", false),
    ("Esc", "back", true),
];

const RESERVED: [egui::Key; 10] = [
    egui::Key::G,
    egui::Key::O,
    egui::Key::N,
    egui::Key::P,
    egui::Key::Period,
    egui::Key::Comma,
    egui::Key::Escape,
    egui::Key::Num3,
    egui::Key::ArrowDown,
    egui::Key::ArrowUp,
];

pub fn key_label(key: egui::Key) -> String {
    let name = key.symbol_or_name();
    if name.chars().count() == 1 { name.to_lowercase() } else { name.to_string() }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bindings {
    #[serde(default)]
    pub custom: BTreeMap<String, String>,
}

impl Bindings {
    pub fn key_for(&self, id: &str) -> egui::Key {
        let default = COMMANDS.iter().find(|c| c.id == id).map(|c| c.default).unwrap_or(egui::Key::Escape);
        self.custom.get(id).and_then(|n| egui::Key::from_name(n)).unwrap_or(default)
    }

    pub fn is_default(&self, id: &str) -> bool {
        !self.custom.contains_key(id)
    }

    pub fn reset(&mut self, id: &str) {
        self.custom.remove(id);
    }

    pub fn set(&mut self, id: &str, key: egui::Key) -> Result<(), String> {
        if RESERVED.contains(&key) {
            return Err(format!("{} is reserved", key_label(key)));
        }
        if let Some(other) = COMMANDS.iter().find(|c| c.id != id && self.key_for(c.id) == key) {
            return Err(format!("{} is already used for {}", key_label(key), other.label));
        }
        let default = COMMANDS.iter().find(|c| c.id == id).map(|c| c.default);
        if default == Some(key) {
            self.custom.remove(id);
        } else {
            self.custom.insert(id.to_string(), key.name().to_string());
        }
        Ok(())
    }

    pub fn hint_items(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for c in COMMANDS.iter().filter(|c| c.hint) {
            match c.id {
                "next" => out.push((format!("{} {}", key_label(self.key_for("next")), key_label(self.key_for("previous"))), "move".to_string())),
                "previous" => {}
                id => out.push((key_label(self.key_for(id)), c.label.to_string())),
            }
        }
        for (key, label, hint) in FIXED {
            if hint {
                out.push((key.to_string(), label.to_string()));
            }
        }
        out
    }

    pub fn rows(&self) -> Vec<(Option<&'static str>, String, String)> {
        let mut out: Vec<_> = COMMANDS.iter().map(|c| (Some(c.id), c.label.to_string(), key_label(self.key_for(c.id)))).collect();
        out.extend(FIXED.iter().map(|(k, l, _)| (None, l.to_string(), k.to_string())));
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    #[default]
    Dark,
    System,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Light, Theme::Dark, Theme::System];

    pub fn label(&self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        }
    }

    pub fn preference(&self) -> egui::ThemePreference {
        match self {
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
            Theme::System => egui::ThemePreference::System,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

impl Density {
    pub const ALL: [Density; 2] = [Density::Compact, Density::Comfortable];

    pub fn label(&self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
        }
    }

    pub fn row_height(&self) -> f32 {
        match self {
            Density::Compact => 21.0,
            Density::Comfortable => 25.0,
        }
    }

    pub fn reader_gap(&self) -> f32 {
        match self {
            Density::Compact => 4.0,
            Density::Comfortable => 8.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub density: Density,
    #[serde(default)]
    pub bodies: BodyPolicy,
    #[serde(default)]
    pub reading: ReadingPrefs,
    #[serde(default)]
    pub keys: Bindings,
    #[serde(default)]
    pub layout: LayoutPrefs,
    #[serde(default)]
    pub plain_text_only: bool,
}

pub fn load(store: &Store) -> Prefs {
    let saved = store.get(SETTINGS_KIND, PREFS_KEY).ok().flatten().and_then(|raw| serde_json::from_slice(&raw).ok());
    saved.unwrap_or_else(|| {
        let mut prefs = Prefs::default();
        prefs.reading.dark_html = store.get(SETTINGS_KIND, LEGACY_DARK_KEY).ok().flatten().is_some_and(|v| v.as_slice() == b"1");
        prefs
    })
}

pub fn save(store: &Store, prefs: &Prefs) -> Result<(), String> {
    let raw = serde_json::to_vec(prefs).map_err(|e| e.to_string())?;
    store.put(SETTINGS_KIND, PREFS_KEY, &raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_only_defaults_off_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let store = Store::create(&path, None, Some(("pw", crate::store::KdfParams::fast()))).unwrap();
        assert!(!load(&store).plain_text_only);
        let mut prefs = load(&store);
        prefs.plain_text_only = true;
        save(&store, &prefs).unwrap();
        drop(store);
        let reopened = Store::open(&path, &[crate::store::Unlock::Password("pw")]).unwrap();
        assert!(load(&reopened).plain_text_only);
    }

    #[test]
    fn headers_only_turns_background_body_download_off() {
        let account = BackfillPolicy { enabled: true, days: 0, megabytes: 5 };
        assert!(!BodyPolicy::HeadersOnly.effective_backfill(account).enabled);
        let off = BackfillPolicy::default();
        assert_eq!(BodyPolicy::KeepDays(30).effective_backfill(off), BackfillPolicy { enabled: true, days: 30, megabytes: 0 });
        assert_eq!(BodyPolicy::KeepAll.effective_backfill(account), account);
    }

    #[test]
    fn rebinding_refuses_taken_and_reserved_keys() {
        let mut b = Bindings::default();
        assert!(b.set("archive", egui::Key::R).is_err());
        assert!(b.set("archive", egui::Key::G).is_err());
        b.set("archive", egui::Key::B).unwrap();
        assert_eq!(b.key_for("archive"), egui::Key::B);
        assert_eq!(b.key_for("reply"), egui::Key::R);
        b.set("reply", egui::Key::E).unwrap();
        b.set("archive", egui::Key::E).unwrap_err();
        b.set("archive", egui::Key::Z).unwrap();
        b.reset("archive");
        assert_eq!(b.key_for("archive"), egui::Key::E);
    }

    #[test]
    fn old_prefs_without_new_fields_load() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.reading.zoom_percent, 100);
        assert_eq!(p.reading.remote_images, RemoteImages::Ask);
        assert_eq!(p.bodies, BodyPolicy::KeepAll);
        assert_eq!(p.reading.reader_width, ReaderWidth::Full);
        assert_eq!(p.layout, LayoutPrefs { side_w: 196, list_w: 540 });
    }

    #[test]
    fn hint_bar_follows_the_bindings() {
        let mut b = Bindings::default();
        assert!(b.hint_items().iter().any(|(k, l)| k == "e" && l == "archive"));
        b.set("archive", egui::Key::B).unwrap();
        assert!(b.hint_items().iter().any(|(k, l)| k == "b" && l == "archive"));
        assert!(b.hint_items().iter().any(|(k, l)| k == "j k" && l == "move"));
    }
}
