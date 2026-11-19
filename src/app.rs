use crate::fake::FakeStore;
use crate::kit::*;
use crate::mailstore::*;
use egui::{Margin, Ui};
use egui_bench::prelude::*;
use std::collections::HashSet;

pub const ROW_H: f32 = 25.0;
const LIST_W: f32 = 540.0;
const SIDE_W: f32 = 196.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reply {
    One,
    All,
    Forward,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Next,
    Previous,
    Open,
    Archive,
    Snooze,
    ToggleFlag,
    ToggleUnread,
    Junk,
    Trash,
    Reply(Reply),
    Compose,
    Search,
    Back,
    LoadImagesOnce,
    ToggleQuotes,
    ToggleHeaders,
    OpenLink(String),
    Unsubscribe,
    GoTo(Folder),
    Expand(MessageId),
    One(Box<Action>),
    OpenAttachment(usize),
    SaveAttachment(usize),
    SaveAllAttachments,
    ViewImage(usize),
    CloseViewer,
    OpenMove,
    MoveTo(Folder, bool),
    ToggleBranch(usize, String),
    NewFolder(usize, String),
    RenameFolder(Folder, String),
    DeleteFolder(Folder),
}

pub type SavePicker = Box<dyn FnMut(&str, &str) -> Option<std::path::PathBuf>>;

fn rfd_picker() -> SavePicker {
    Box::new(|name, ext| {
        let mut dialog = rfd::FileDialog::new().set_file_name(name);
        if !ext.is_empty() {
            dialog = dialog.add_filter(ext, &[ext]);
        }
        dialog.save_file()
    })
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub title: String,
    pub account: usize,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    pub saved: bool,
}

impl Draft {
    pub fn blank() -> Draft {
        Draft {
            title: "new message".into(),
            account: 0,
            to: String::new(),
            cc: String::new(),
            subject: String::new(),
            body: String::new(),
            saved: false,
        }
    }

    pub fn reply(m: &Message, mode: Reply) -> Draft {
        let (prefix, title) = match mode {
            Reply::Forward => ("Fwd: ", "forward"),
            _ => ("Re: ", "reply to"),
        };
        let subject = if m.summary.subject.starts_with(prefix) {
            m.summary.subject.clone()
        } else {
            format!("{prefix}{}", m.summary.subject)
        };
        let to = if mode == Reply::Forward { String::new() } else { m.summary.from.clone() };
        let cc = if mode == Reply::All {
            m.to.iter().filter(|t| t.as_str() != "me").cloned().collect::<Vec<_>>().join(", ")
        } else {
            String::new()
        };
        let body = if mode == Reply::Forward {
            format!("\n\nForwarded message from {}:\n\n{}", m.summary.from, m.paragraphs.join("\n\n"))
        } else {
            String::new()
        };
        Draft {
            title: format!("{title} {}", m.summary.from),
            account: m.summary.account,
            to,
            cc,
            subject,
            body,
            saved: false,
        }
    }
}

struct Mover {
    query: String,
    selected: usize,
    whole: bool,
}

enum FolderDialog {
    Create { account: usize, prefix: String, name: String },
    Rename { folder: Folder, prefix: String, name: String },
    Delete { folder: Folder },
}

fn fuzzy_score(query: &str, text: &str) -> Option<usize> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut at = 0;
    let mut score = 0;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = text[at..].iter().position(|c| *c == q)? + at;
        score += match last {
            Some(l) => found - l - 1,
            None => found * 2,
        };
        last = Some(found);
        at = found + 1;
    }
    Some(score + text.len() / 8)
}

struct Target {
    folder: Folder,
    label: String,
}

struct View {
    query: Query,
    counts: Counts,
    total: usize,
    current: Option<Message>,
    head: Option<Summary>,
    thread: Vec<ThreadItem>,
}

fn unsubscribe(u: Unsubscribe) -> String {
    if u.one_click {
        let target = u.target;
        std::thread::spawn(move || {
            let _ = ureq::post(&target).send_string("List-Unsubscribe=One-Click");
        });
        "Unsubscribe request sent".into()
    } else {
        match open::that(&u.target) {
            Ok(()) => "Unsubscribe page opened".into(),
            Err(_) => "Could not open the unsubscribe link".into(),
        }
    }
}

pub struct App {
    pub store: Box<dyn MailStore>,
    pub folder: Folder,
    pub filter: Filter,
    pub search: String,
    pub selected: usize,
    pub draft: Option<Draft>,
    pub notice: Option<String>,
    pub images_loaded: HashSet<MessageId>,
    pub quotes_open: bool,
    pub headers_open: bool,
    pub rendered_rows: usize,
    pub reader_focus: bool,
    pub force_failed: bool,
    mover: Option<Mover>,
    dialog: Option<FolderDialog>,
    collapsed: HashSet<(usize, String)>,
    search_id: Option<egui::Id>,
    scroll_to_selected: bool,
    go_pending: bool,
    last_shown: Option<MessageId>,
    last_head: Option<MessageId>,
    pub expanded: Option<MessageId>,
    pub thread_focus: Option<MessageId>,
    pub settings_requested: bool,
    pub allow_open: bool,
    pub open_dir: std::path::PathBuf,
    pub save_picker: SavePicker,
    pub opener: Box<dyn FnMut(&std::path::Path) -> bool>,
    previews: Option<(MessageId, Vec<Option<egui::TextureHandle>>)>,
    pub viewing: Option<usize>,
    html: crate::htmlview::Renderer,
}

impl App {
    pub fn new(ctx: &egui::Context) -> App {
        crate::kit::setup(ctx);
        App::with_store(Box::new(FakeStore::concept()))
    }

    pub fn with_store(store: Box<dyn MailStore>) -> App {
        App {
            store,
            folder: Folder::Inbox,
            filter: Filter::All,
            search: String::new(),
            selected: 0,
            draft: None,
            notice: None,
            images_loaded: HashSet::new(),
            quotes_open: false,
            headers_open: false,
            rendered_rows: 0,
            reader_focus: false,
            force_failed: false,
            mover: None,
            dialog: None,
            collapsed: HashSet::new(),
            search_id: None,
            scroll_to_selected: false,
            go_pending: false,
            last_shown: None,
            last_head: None,
            expanded: None,
            thread_focus: None,
            settings_requested: false,
            allow_open: true,
            open_dir: crate::attach::default_open_dir(),
            save_picker: rfd_picker(),
            opener: Box::new(|p| open::that(p).is_ok()),
            previews: None,
            viewing: None,
            html: crate::htmlview::Renderer::default(),
        }
    }

    pub fn query(&self) -> Query {
        Query { folder: self.folder.clone(), filter: self.filter, text: self.search.clone() }
    }

    fn view(&mut self) -> View {
        let query = self.query();
        let counts = self.store.counts(&query);
        let total = match self.filter {
            Filter::All => counts.all,
            Filter::Unread => counts.unread,
            Filter::Flagged => counts.flagged,
        };
        if total > 0 && self.selected >= total {
            self.selected = total - 1;
        }
        let head = self.store.list(&query, self.selected, 1).items.into_iter().next();
        let head_id = head.as_ref().map(|s| s.id);
        if head_id != self.last_head {
            self.expanded = None;
            self.thread_focus = None;
            self.last_head = head_id;
        }
        let thread = head_id.map(|id| self.store.thread(id)).unwrap_or_default();
        let shown = self
            .expanded
            .filter(|e| thread.iter().any(|t| t.summary.id == *e))
            .or_else(|| thread.last().map(|t| t.summary.id))
            .or(head_id);
        let current = shown.and_then(|id| self.store.get(id).ok());
        let id = current.as_ref().map(|m| m.summary.id);
        if id != self.last_shown {
            self.quotes_open = false;
            self.last_shown = id;
        }
        View { query, counts, total, current, head, thread }
    }

    fn pressed(ctx: &egui::Context, key: egui::Key) -> bool {
        ctx.input(|i| i.key_pressed(key) && !i.modifiers.command && !i.modifiers.alt)
    }

    fn key_actions(&mut self, ctx: &egui::Context) -> Vec<Action> {
        use egui::Key as K;
        if self.draft.is_some() || self.dialog.is_some() {
            return Vec::new();
        }
        if self.mover.is_some() {
            return Vec::new();
        }
        if Self::pressed(ctx, K::Escape) {
            return vec![Action::Back];
        }
        if ctx.egui_wants_keyboard_input() {
            return Vec::new();
        }
        let plain = ctx.input(|i| !i.modifiers.shift);
        let table: [(K, Action); 15] = [
            (K::J, Action::Next),
            (K::ArrowDown, Action::Next),
            (K::K, Action::Previous),
            (K::ArrowUp, Action::Previous),
            (K::Enter, Action::Open),
            (K::E, Action::Archive),
            (K::S, Action::Snooze),
            (K::X, Action::ToggleFlag),
            (K::U, Action::ToggleUnread),
            (K::R, Action::Reply(Reply::One)),
            (K::A, Action::Reply(Reply::All)),
            (K::F, Action::Reply(Reply::Forward)),
            (K::C, Action::Compose),
            (K::M, Action::OpenMove),
            (K::Slash, Action::Search),
        ];
        let mut out = Vec::new();
        if self.go_pending {
            if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                self.go_pending = false;
                if Self::pressed(ctx, K::A) {
                    return vec![Action::GoTo(Folder::Archive)];
                }
                if Self::pressed(ctx, K::I) {
                    return vec![Action::GoTo(Folder::Inbox)];
                }
            }
        } else if Self::pressed(ctx, K::G) {
            self.go_pending = true;
            return out;
        }
        for (key, action) in table {
            if key == K::S && !plain {
                continue;
            }
            if Self::pressed(ctx, key) {
                out.push(action);
            }
        }
        if plain && Self::pressed(ctx, K::O) {
            out.push(Action::OpenAttachment(0));
        }
        if !plain && Self::pressed(ctx, K::S) {
            out.push(Action::SaveAllAttachments);
        }
        if plain && Self::pressed(ctx, K::Num3) && ctx.input(|i| i.modifiers.shift) {
            out.push(Action::Trash);
        }
        out
    }

    fn targets(v: &View, whole: bool, same_folder: bool) -> Vec<(MessageId, Folder)> {
        if whole && !v.thread.is_empty() {
            let row = v.query.folder.clone();
            return v
                .thread
                .iter()
                .filter(|t| !same_folder || t.folder == row)
                .map(|t| (t.summary.id, t.folder.clone()))
                .collect();
        }
        v.current.iter().map(|m| (m.summary.id, m.folder.clone())).collect()
    }

    fn move_current(&mut self, v: &View, to: Folder, done: &str, whole: bool) {
        for (id, folder) in Self::targets(v, whole, true) {
            if folder == to {
                continue;
            }
            match self.store.move_to(id, to.clone()) {
                Ok(()) => self.notice = Some(done.into()),
                Err(e) => self.notice = Some(format!("Could not move the message: {e:?}")),
            }
        }
    }

    fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for a in actions {
            let v = self.view();
            let (a, whole) = match a {
                Action::One(inner) => (*inner, false),
                a => (a, true),
            };
            match a {
                Action::One(_) => {}
                Action::Expand(id) => self.expanded = Some(id),
                Action::Next | Action::Previous if self.reader_focus && v.thread.len() > 1 => {
                    let shown = v.current.as_ref().map(|m| m.summary.id);
                    let at = self
                        .thread_focus
                        .or(shown)
                        .and_then(|f| v.thread.iter().position(|t| t.summary.id == f))
                        .unwrap_or(v.thread.len() - 1);
                    let to = if a == Action::Next { (at + 1).min(v.thread.len() - 1) } else { at.saturating_sub(1) };
                    self.thread_focus = Some(v.thread[to].summary.id);
                }
                Action::Next => {
                    if self.selected + 1 < v.total {
                        self.selected += 1;
                    }
                    self.scroll_to_selected = true;
                }
                Action::Previous => {
                    self.selected = self.selected.saturating_sub(1);
                    self.scroll_to_selected = true;
                }
                Action::Open => {
                    if let Some(f) = self.thread_focus.take().filter(|_| self.reader_focus) {
                        self.expanded = Some(f);
                    }
                    for (id, _) in Self::targets(&v, true, false) {
                        let _ = self.store.set_flags(id, FlagChange { unread: Some(false), flagged: None });
                    }
                    self.reader_focus = true;
                }
                Action::Archive => self.move_current(&v, Folder::Archive, "Archived", whole),
                Action::Snooze => self.move_current(&v, Folder::Snoozed, "Snoozed until tomorrow 08:00", whole),
                Action::Junk => self.move_current(&v, Folder::Junk, "Moved to junk", whole),
                Action::Trash => self.move_current(&v, Folder::Trash, "Moved to trash", whole),
                Action::ToggleFlag => {
                    let current = if whole { v.head.as_ref() } else { v.current.as_ref().map(|m| &m.summary) };
                    if let Some(s) = current {
                        let on = !s.flagged;
                        for (id, _) in Self::targets(&v, whole, false) {
                            let _ = self.store.set_flags(id, FlagChange { unread: None, flagged: Some(on) });
                        }
                        self.notice = Some(if on { "Flagged".into() } else { "Flag removed".into() });
                    }
                }
                Action::ToggleUnread => {
                    let current = if whole { v.head.as_ref() } else { v.current.as_ref().map(|m| &m.summary) };
                    if let Some(s) = current {
                        let unread = !s.unread;
                        for (id, _) in Self::targets(&v, whole, false) {
                            let _ = self.store.set_flags(id, FlagChange { unread: Some(unread), flagged: None });
                        }
                    }
                }
                Action::Reply(mode) => {
                    if let Some(m) = &v.current {
                        self.draft = Some(Draft::reply(m, mode));
                    }
                }
                Action::Compose => self.draft = Some(Draft::blank()),
                Action::Search => {
                    if let Some(id) = self.search_id {
                        ctx.memory_mut(|m| m.request_focus(id));
                        ctx.input_mut(|i| {
                            i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t == "/"))
                        });
                    }
                }
                Action::Back => {
                    if let Some(id) = self.search_id {
                        ctx.memory_mut(|m| m.surrender_focus(id));
                    }
                    if self.reader_focus {
                        self.reader_focus = false;
                    } else if !self.search.is_empty() {
                        self.search.clear();
                        self.selected = 0;
                    }
                }
                Action::LoadImagesOnce => {
                    if let Some(m) = &v.current {
                        self.images_loaded.insert(m.summary.id);
                    }
                }
                Action::ToggleQuotes => self.quotes_open = !self.quotes_open,
                Action::ToggleHeaders => self.headers_open = !self.headers_open,
                Action::OpenLink(link) => {
                    let _ = open::that(link);
                }
                Action::Unsubscribe => {
                    if let Some(u) = v.current.as_ref().and_then(|m| m.headers.unsubscribe.clone()) {
                        self.notice = Some(unsubscribe(u));
                    }
                }
                Action::GoTo(f) => self.go_folder(f),
                Action::OpenAttachment(i) => self.open_attachment(&v, i),
                Action::SaveAttachment(i) => self.save_attachment(&v, i),
                Action::SaveAllAttachments => self.save_all(&v),
                Action::ViewImage(i) => self.viewing = Some(i),
                Action::CloseViewer => self.viewing = None,
                Action::OpenMove => {
                    if v.current.is_some() {
                        self.mover = Some(Mover { query: String::new(), selected: 0, whole: whole && !self.reader_focus });
                        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("m"))));
                    }
                }
                Action::MoveTo(f, whole) => {
                    let said = format!("Moved to {}", f.name());
                    self.move_current(&v, f, &said, whole);
                }
                Action::ToggleBranch(account, path) => {
                    if !self.collapsed.remove(&(account, path.clone())) {
                        self.collapsed.insert((account, path));
                    }
                }
                Action::NewFolder(account, prefix) => {
                    self.dialog = Some(FolderDialog::Create { account, prefix, name: String::new() });
                }
                Action::RenameFolder(folder, prefix) => {
                    let name = folder.name().strip_prefix(prefix.as_str()).unwrap_or(folder.name()).to_string();
                    self.dialog = Some(FolderDialog::Rename { folder, prefix, name });
                }
                Action::DeleteFolder(folder) => self.dialog = Some(FolderDialog::Delete { folder }),
            }
            ctx.request_repaint();
        }
    }

    fn attachment_data(&mut self, v: &View) -> Option<Vec<AttachmentData>> {
        let m = v.current.as_ref()?;
        if m.attachments.is_empty() {
            return None;
        }
        match self.store.attachments(m.summary.id) {
            Ok(items) if !items.is_empty() => Some(items),
            Ok(_) => None,
            Err(e) => {
                self.notice = Some(format!("Could not read the attachment: {e:?}"));
                None
            }
        }
    }

    fn save_attachment(&mut self, v: &View, index: usize) {
        let Some(items) = self.attachment_data(v) else { return };
        let Some(item) = items.get(index) else { return };
        let name = crate::attach::safe_name(&item.name, "attachment");
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
        let Some(path) = (self.save_picker)(&name, &ext) else { return };
        self.notice = Some(match std::fs::write(&path, item.data.as_slice()) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save: {e}"),
        });
    }

    fn save_all(&mut self, v: &View) {
        let Some(items) = self.attachment_data(v) else { return };
        let Some(path) = (self.save_picker)("attachments.zip", "zip") else { return };
        self.notice = Some(match crate::attach::zip_all(&items).and_then(|bytes| std::fs::write(&path, bytes)) {
            Ok(()) => format!("Saved {} attachments to {}", items.len(), path.display()),
            Err(e) => format!("Could not save: {e}"),
        });
    }

    fn open_attachment(&mut self, v: &View, index: usize) {
        if !self.allow_open {
            if v.current.as_ref().is_some_and(|m| !m.attachments.is_empty()) {
                self.notice = Some("Opening attachments is turned off in settings. Use save as instead.".into());
            }
            return;
        }
        let Some(items) = self.attachment_data(v) else { return };
        let Some(item) = items.get(index) else { return };
        self.notice = Some(match crate::attach::write_private(&self.open_dir, &item.name, &item.data) {
            Ok(path) => {
                if (self.opener)(&path) {
                    format!("Opened a plain copy of {} at {}. It is deleted when mailbench closes.", item.name, path.display())
                } else {
                    format!("No program could open {}. A plain copy is at {} until mailbench closes.", item.name, path.display())
                }
            }
            Err(e) => format!("Could not write the copy: {e}"),
        });
    }

    fn sync_previews(&mut self, ctx: &egui::Context, m: &Message) {
        if self.previews.as_ref().is_some_and(|(id, _)| *id == m.summary.id) {
            return;
        }
        self.viewing = None;
        if !m.attachments.iter().any(|a| crate::attach::is_previewable_image(&a.mime)) {
            self.previews = Some((m.summary.id, Vec::new()));
            return;
        }
        let data = self.store.attachments(m.summary.id).unwrap_or_default();
        let textures = data
            .iter()
            .enumerate()
            .map(|(i, d)| {
                if !crate::attach::is_previewable_image(&d.mime) {
                    return None;
                }
                let image = crate::attach::decode_image(&d.data)?;
                Some(ctx.load_texture(format!("attachment-{}-{i}", m.summary.id.0), image, egui::TextureOptions::LINEAR))
            })
            .collect();
        self.previews = Some((m.summary.id, textures));
    }

    fn preview_texture(&self, index: usize) -> Option<&egui::TextureHandle> {
        self.previews.as_ref()?.1.get(index)?.as_ref()
    }

    fn previews_ui(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some((_, textures)) = &self.previews else { return };
        for (i, t) in textures.iter().enumerate() {
            let Some(t) = t else { continue };
            let size = t.size_vec2();
            let scale = (360.0 / size.x).min(240.0 / size.y).min(1.0);
            let resp = ui.add(egui::Image::new(t).fit_to_exact_size(size * scale).sense(egui::Sense::click()));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                actions.push(Action::ViewImage(i));
            }
        }
    }

    fn viewer(&self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(i) = self.viewing else { return };
        let Some(t) = self.preview_texture(i) else { return };
        let r = egui::Modal::new(egui::Id::new("image-viewer"))
            .backdrop_color(egui::Color32::from_black_alpha(200))
            .show(ctx, |ui| {
                egui::ScrollArea::both().max_height(ctx.content_rect().height() * 0.8).max_width(ctx.content_rect().width() * 0.8).show(ui, |ui| {
                    ui.add(egui::Image::new(t).fit_to_original_size(1.0));
                });
                if ui.add(Key::new("close").outline(VALUE)).clicked() {
                    actions.push(Action::CloseViewer);
                }
            });
        if r.should_close() {
            actions.push(Action::CloseViewer);
        }
    }

    fn go_folder(&mut self, f: Folder) {
        if f != self.folder {
            self.folder = f;
            self.selected = 0;
            self.filter = Filter::All;
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let keys = self.key_actions(&ctx);
        self.apply(&ctx, keys);
        let v = self.view();
        let accounts = self.store.accounts();
        let failed = self.force_failed || accounts.iter().any(|a| a.sync == SyncState::Failed);
        let mut actions = Vec::new();
        self.header(ui, &accounts, failed, &mut actions);
        self.hint_bar(ui);
        egui::Panel::left("side")
            .exact_size(SIDE_W)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(Margin::same(8)))
            .show(ui, |ui| self.side(ui, &accounts, &mut actions));
        egui::Panel::left("list")
            .exact_size(LIST_W)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(CHASSIS))
            .show(ui, |ui| self.list(ui, &v, &accounts, &mut actions));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            pane_fill(ui, CHASSIS);
            egui::Frame::NONE.inner_margin(Margin { left: 22, right: 22, top: 16, bottom: 10 }).show(ui, |ui| {
                self.reader(ui, &v, &accounts, &mut actions);
            });
        });
        self.composer(&ctx, &accounts);
        self.viewer(&ctx, &mut actions);
        self.move_picker(&ctx, v.current.as_ref(), &mut actions);
        self.folder_dialog(&ctx);
        self.apply(&ctx, actions);
    }

    fn header(&mut self, ui: &mut Ui, accounts: &[AccountInfo], failed: bool, actions: &mut Vec<Action>) {
        egui::Panel::top("head").frame(header_frame()).show(ui, |ui| {
            ui.horizontal(|ui| {
                title(ui);
                ui.add_space(18.0);
                ui.allocate_ui(egui::vec2(360.0, KEY_H), |ui| {
                    let r = field(ui, &mut self.search, "search all accounts, press /");
                    self.search_id = Some(r.id);
                    if r.changed() {
                        self.selected = 0;
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Key::new("compose").solid(READOUT).min_width(100.0)).clicked() {
                        actions.push(Action::Compose);
                    }
                    ui.add_space(8.0);
                    if ui.add(Key::new("settings").outline(VALUE).min_width(90.0)).clicked() {
                        self.settings_requested = true;
                    }
                    ui.add_space(8.0);
                    if failed {
                        let n = accounts.iter().filter(|a| a.sync == SyncState::Failed).count().max(1);
                        let text = if n == 1 { "1 account needs you".to_string() } else { format!("{n} accounts need you") };
                        chip(ui, &text, FAULT, true);
                    } else {
                        let idle = accounts.iter().filter(|a| a.sync == SyncState::Idle).count();
                        let syncing = accounts.iter().any(|a| a.sync == SyncState::Syncing);
                        let text = if syncing && idle < accounts.len() {
                            format!("syncing, idle on {} of {}", idle, accounts.len())
                        } else {
                            format!("idle on {} of {}", idle, accounts.len())
                        };
                        chip(ui, &text, TRACE, true);
                    }
                });
            });
        });
    }

    fn hint_bar(&self, ui: &mut Ui) {
        egui::Panel::bottom("foot").frame(footer_frame()).show(ui, |ui| {
            let (text, tint) = match &self.notice {
                Some(n) => (n.as_str(), READOUT),
                None => ("synced 09:41", TRACE),
            };
            keycap_bar(
                ui,
                &[
                    ("j k", "move"),
                    ("Enter", "open"),
                    ("e", "archive"),
                    ("r", "reply"),
                    ("s", "snooze"),
                    ("x", "flag"),
                    ("/", "search"),
                    ("m", "move"),
                    ("c", "compose"),
                    ("o", "open file"),
                    ("S", "save files"),
                    ("Esc", "back"),
                ],
                Some((text, tint)),
            );
        });
    }

    fn side(&mut self, ui: &mut Ui, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 2.0;
        heading(ui, "accounts");
        ui.add_space(4.0);
        for a in accounts {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            let tint = match a.sync {
                SyncState::Idle => OK,
                SyncState::Syncing => TRACE,
                SyncState::Failed => FAULT,
            };
            p.circle_filled(egui::pos2(rect.left() + 10.0, rect.top() + 13.0), 3.5, tint);
            put(p, rect.left() + 22.0, 100.0, rect.top() + 13.0, &a.name, body_strong(13.0), VALUE);
            let status = match a.sync {
                SyncState::Idle => "idle",
                SyncState::Syncing => "sync",
                SyncState::Failed => "sign in",
            };
            let col = if a.sync == SyncState::Failed { FAULT } else { LEGEND };
            put(p, rect.left() + 22.0, 140.0, rect.top() + 28.0, status, legend_font(9.5), col);
            if a.unread > 0 {
                put_right(p, rect.right() - 6.0, rect.top() + 13.0, &a.unread.to_string(), figure(11.0), READOUT);
            }
        }
        ui.add_space(12.0);
        heading(ui, "inbox");
        ui.add_space(4.0);
        for f in self.store.folders() {
            let on = f.folder == self.folder;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
            let p = ui.painter();
            if on {
                selection(p, rect);
            } else if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            put(p, rect.left() + 12.0, 120.0, rect.center().y, f.folder.name(), body(13.0), if on { VALUE } else { LEGEND });
            let n = if matches!(f.folder, Folder::Inbox) { f.unread } else if matches!(f.folder, Folder::Drafts) { f.total } else { 0 };
            if n > 0 {
                put_right(p, rect.right() - 6.0, rect.center().y, &n.to_string(), figure(11.0), if on { READOUT } else { LEGEND });
            }
            if resp.clicked() {
                actions.push(Action::GoTo(f.folder));
            }
        }
        let labels = self.store.labels();
        if !labels.is_empty() {
            ui.add_space(12.0);
            heading(ui, "labels");
            ui.add_space(4.0);
            for l in labels {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, 2.0, BAND);
                }
                put(p, rect.left() + 12.0, 120.0, rect.center().y, &l.name, body(13.0), LEGEND);
                if l.unread > 0 {
                    put_right(p, rect.right() - 6.0, rect.center().y, &l.unread.to_string(), figure(11.0), LEGEND);
                }
            }
        }
        self.tree(ui, accounts, actions);
    }

    fn tree(&mut self, ui: &mut Ui, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let tree = self.store.tree();
        if tree.is_empty() && accounts.is_empty() {
            return;
        }
        for (account, info) in accounts.iter().enumerate() {
            ui.add_space(12.0);
            heading(ui, &info.name.to_lowercase());
            ui.add_space(4.0);
            let mut hidden_under: Option<usize> = None;
            for node in tree.iter().filter(|n| n.account == account) {
                if let Some(depth) = hidden_under {
                    if node.depth > depth {
                        continue;
                    }
                    hidden_under = None;
                }
                let folded = self.collapsed.contains(&(account, node.path.clone()));
                if folded && node.children {
                    hidden_under = Some(node.depth);
                }
                self.tree_row(ui, node, folded, actions);
            }
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            put(p, rect.left() + 12.0, 120.0, rect.center().y, "+ new folder", body(12.0), LEGEND);
            ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label(format!("new folder in {}", info.name)));
            if resp.clicked() {
                actions.push(Action::NewFolder(account, String::new()));
            }
        }
    }

    fn tree_row(&self, ui: &mut Ui, node: &TreeFolder, folded: bool, actions: &mut Vec<Action>) {
        let on = node.folder == self.folder;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
        let p = ui.painter();
        if on {
            selection(p, rect);
        } else if resp.hovered() {
            p.rect_filled(rect, 2.0, BAND);
        }
        let indent = rect.left() + 12.0 + node.depth as f32 * 12.0;
        if node.children {
            let c = egui::pos2(indent - 1.0, rect.center().y);
            let points = if folded {
                vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(-2.0, 4.0), c + egui::vec2(3.0, 0.0)]
            } else {
                vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
            };
            p.add(egui::Shape::convex_polygon(points, LEGEND, egui::Stroke::NONE));
        }
        let text_x = indent + 8.0;
        let col = if !node.selectable { TRACE } else if on { VALUE } else { LEGEND };
        put(p, text_x, (rect.right() - text_x - 28.0).max(20.0), rect.center().y, &node.leaf, body(13.0), col);
        if node.unread > 0 {
            put_right(p, rect.right() - 6.0, rect.center().y, &node.unread.to_string(), figure(11.0), if on { READOUT } else { LEGEND });
        }
        ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(node.path.clone()));
        if resp.clicked() {
            let on_arrow = node.children
                && resp.interact_pointer_pos().is_some_and(|pos| pos.x < indent + 6.0);
            if on_arrow || !node.selectable {
                actions.push(Action::ToggleBranch(node.account, node.path.clone()));
            } else {
                actions.push(Action::GoTo(node.folder.clone()));
            }
        }
        let parent = node.path[..node.path.len() - node.leaf.len()].to_string();
        resp.context_menu(|ui| {
            ui.set_min_width(140.0);
            let child_prefix = format!("{}{}", node.path, node.delimiter);
            let items = [
                ("new subfolder", Action::NewFolder(node.account, child_prefix)),
                ("rename", Action::RenameFolder(node.folder.clone(), parent)),
                ("delete", Action::DeleteFolder(node.folder.clone())),
            ];
            for (label, action) in items {
                let tint = if label == "delete" { FAULT } else { VALUE };
                if ui.add(Key::new(label).outline(tint).min_width(ui.available_width())).clicked() {
                    actions.push(action);
                    ui.close();
                }
            }
        });
    }

    fn move_targets(&self, account: usize) -> Vec<Target> {
        let mut out: Vec<Target> = self
            .store
            .folders()
            .into_iter()
            .map(|f| Target { label: f.folder.name().to_string(), folder: f.folder })
            .filter(|t| t.folder != Folder::Drafts && t.folder != Folder::Sent)
            .collect();
        out.extend(
            self.store
                .tree()
                .into_iter()
                .filter(|n| n.account == account && n.selectable)
                .map(|n| Target { label: n.path, folder: n.folder }),
        );
        out
    }

    fn move_picker(&mut self, ctx: &egui::Context, current: Option<&Message>, actions: &mut Vec<Action>) {
        let Some(mover) = self.mover.as_mut() else { return };
        let Some(m) = current else {
            self.mover = None;
            return;
        };
        let account = m.summary.account;
        let from = m.folder.clone();
        let whole = mover.whole;
        let query = mover.query.clone();
        let mut ranked: Vec<(usize, Target)> = self
            .move_targets(account)
            .into_iter()
            .filter(|t| t.folder != from)
            .filter_map(|t| fuzzy_score(&query, &t.label).map(|s| (s, t)))
            .collect();
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        let Some(mover) = self.mover.as_mut() else { return };
        let (mut picked, mut close) = (None, false);
        let r = egui::Modal::new(egui::Id::new("move-to"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                modal_title(ui, "move to");
                let r = field(ui, &mut mover.query, "type a folder name");
                if !r.has_focus() {
                    r.request_focus();
                }
                if r.changed() {
                    mover.selected = 0;
                }
                ui.input(|i| {
                    if i.key_pressed(egui::Key::ArrowDown) && mover.selected + 1 < ranked.len() {
                        mover.selected += 1;
                    }
                    if i.key_pressed(egui::Key::ArrowUp) {
                        mover.selected = mover.selected.saturating_sub(1);
                    }
                });
                ui.add_space(6.0);
                if ranked.is_empty() {
                    Line::new().note("No folder matches").size(11.5).show(ui);
                }
                for (i, (_, t)) in ranked.iter().take(10).enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
                    let p = ui.painter();
                    if i == mover.selected {
                        selection(p, rect);
                    } else if resp.hovered() {
                        p.rect_filled(rect, 2.0, BAND);
                    }
                    put(p, rect.left() + 12.0, rect.width() - 20.0, rect.center().y, &t.label, body(13.0), VALUE);
                    ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("move to {}", t.label)));
                    if resp.clicked() {
                        picked = Some(t.folder.clone());
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    picked = ranked.get(mover.selected).map(|(_, t)| t.folder.clone());
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        if r.should_close() {
            close = true;
        }
        if let Some(f) = picked {
            actions.push(Action::MoveTo(f, whole));
            self.mover = None;
        } else if close {
            self.mover = None;
        }
    }

    fn folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        let (mut submit, mut cancel) = (false, false);
        let r = egui::Modal::new(egui::Id::new("folder-dialog"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                match dialog {
                    FolderDialog::Create { prefix, name, .. } => {
                        modal_title(ui, "new folder");
                        if !prefix.is_empty() {
                            Line::new().note(format!("inside {prefix}")).size(11.5).show(ui);
                        }
                        let r = field(ui, name, "folder name");
                        if !r.has_focus() {
                            r.request_focus();
                        }
                    }
                    FolderDialog::Rename { name, .. } => {
                        modal_title(ui, "rename folder");
                        let r = field(ui, name, "folder name");
                        if !r.has_focus() {
                            r.request_focus();
                        }
                    }
                    FolderDialog::Delete { folder } => {
                        modal_title(ui, "delete folder");
                        Line::new()
                            .note(format!("Delete {} and every message in it on the server?", folder.name()))
                            .size(11.5)
                            .show(ui);
                    }
                }
                let (label, tint) = match dialog {
                    FolderDialog::Create { .. } => ("create", READOUT),
                    FolderDialog::Rename { .. } => ("rename", READOUT),
                    FolderDialog::Delete { .. } => ("delete", FAULT),
                };
                form_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        submit = ui.add(Key::new(label).solid(tint).min_width(100.0)).clicked();
                        cancel = ui.add(Key::new("cancel").outline(VALUE).min_width(90.0)).clicked();
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });
        if r.should_close() {
            cancel = true;
        }
        if submit {
            let result = match self.dialog.take() {
                Some(FolderDialog::Create { account, prefix, name }) if !name.trim().is_empty() => {
                    self.store.create_folder(account, &format!("{prefix}{}", name.trim())).map(|_| "Folder created")
                }
                Some(FolderDialog::Rename { folder, prefix, name }) if !name.trim().is_empty() => {
                    self.store.rename_folder(&folder, &format!("{prefix}{}", name.trim())).map(|_| "Folder renamed")
                }
                Some(FolderDialog::Delete { folder }) => {
                    if folder == self.folder {
                        self.go_folder(Folder::Inbox);
                    }
                    self.store.delete_folder(&folder).map(|_| "Folder deleted")
                }
                _ => return,
            };
            self.notice = Some(match result {
                Ok(said) => said.into(),
                Err(e) => format!("Could not change the folder: {e:?}"),
            });
        } else if cancel {
            self.dialog = None;
        }
    }

    fn list(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        pane_fill(ui, CHASSIS);
        ui.painter().line_segment(
            [ui.max_rect().right_top(), ui.max_rect().right_bottom()],
            egui::Stroke::new(1.0, ETCH),
        );
        egui::Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                let before = self.filter;
                segments(
                    ui,
                    &mut self.filter,
                    &[
                        (Filter::All, "all", Some(v.counts.all)),
                        (Filter::Unread, "unread", Some(v.counts.unread)),
                        (Filter::Flagged, "flagged", Some(v.counts.flagged)),
                    ],
                );
                if before != self.filter {
                    self.selected = 0;
                }
            });
            ui.add_space(6.0);
            for (i, a) in accounts.iter().enumerate() {
                if a.sync == SyncState::Failed || (self.force_failed && i == 0) {
                    egui::Frame::NONE.inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                        fault_card(ui, &a.name, &a.address);
                    });
                    ui.add_space(8.0);
                    break;
                }
            }
            if v.total == 0 {
                self.rendered_rows = 0;
                self.empty_list(ui);
                return;
            }
            self.rows(ui, v, accounts, actions);
        });
    }

    fn empty_list(&self, ui: &mut Ui) {
        let filtered = !self.search.is_empty() || self.filter != Filter::All;
        if filtered {
            empty_state(ui, "No matches", "Nothing here fits the search and filter. Press Esc to clear the search.", &[("Esc", "clear"), ("/", "search")]);
        } else if self.folder == Folder::Inbox {
            empty_state(
                ui,
                "Inbox zero",
                "Everything in kestrel, home and lists is read and filed. New mail shows up here as it arrives.",
                &[("g a", "go to archive"), ("c", "compose")],
            );
        } else {
            empty_state(ui, &format!("Nothing in {}", self.folder.name()), "Messages you move here will show up in this folder.", &[("g i", "go to inbox"), ("c", "compose")]);
        }
    }

    fn rows(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let columns: [(&str, f32); 5] = [("", 20.0), ("from", 150.0), ("subject", 200.0), ("account", 110.0), ("time", 60.0)];
        let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), egui_bench::table::ROW_H), egui::Sense::hover());
        egui_bench::table::header(&ui.painter_at(head), head, &columns);
        let selected = self.selected;
        let scroll_to = std::mem::take(&mut self.scroll_to_selected);
        let mut painted = 0;
        let mut clicked = None;
        let mut opened = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            let total = v.total;
            ui.set_height(total as f32 * ROW_H);
            let first = (viewport.min.y / ROW_H).floor().max(0.0) as usize;
            let last = ((viewport.max.y / ROW_H).ceil() as usize + 1).min(total);
            let page = self.store.list(&v.query, first, last.saturating_sub(first));
            let origin = ui.min_rect().min;
            let width = ui.available_width();
            for (n, s) in page.items.iter().enumerate() {
                let i = first + n;
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(0.0, i as f32 * ROW_H),
                    egui::vec2(width, ROW_H),
                );
                let resp = ui.interact(rect, ui.id().with(("row", i)), egui::Sense::click());
                let name = accounts.get(s.account).map(|a| a.name.as_str());
                paint_row(&ui.painter_at(rect), rect, s, i == selected, resp.hovered(), name);
                ui.ctx().accesskit_node_builder(resp.id, |node| {
                    node.set_label(format!("{} {}", s.from, s.subject));
                });
                painted += 1;
                if resp.clicked() {
                    clicked = Some(i);
                }
                if resp.double_clicked() {
                    opened = Some(i);
                }
            }
            if scroll_to {
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(0.0, selected as f32 * ROW_H),
                    egui::vec2(width, ROW_H),
                );
                ui.scroll_to_rect(rect, None);
            }
        });
        self.rendered_rows = painted;
        if let Some(i) = clicked {
            self.selected = i;
        }
        if let Some(i) = opened {
            self.selected = i;
            actions.push(Action::Open);
        }
    }

    fn reader(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let Some(m) = &v.current else {
            empty_state(ui, "Nothing selected", "Pick a message from the list, or press j to start at the top.", &[("j", "next"), ("/", "search")]);
            return;
        };
        let account = accounts.get(m.summary.account).map(|a| a.name.as_str()).unwrap_or("");
        self.sync_previews(&ui.ctx().clone(), m);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(if m.html.is_empty() { MEASURE + 44.0 } else { 820.0 }));
            ui.spacing_mut().item_spacing.y = 8.0;
            subject_line(ui, &m.summary.subject);
            let shown_at = v.thread.iter().position(|t| t.summary.id == m.summary.id).unwrap_or(v.thread.len());
            for t in v.thread.iter().take(shown_at) {
                if collapsed_message(ui, &t.summary.from, &t.summary.when, t.summary.unread, self.thread_focus == Some(t.summary.id)) {
                    actions.push(Action::Expand(t.summary.id));
                }
            }
            let mut to = m.to.join(", ");
            if to.is_empty() {
                to = "nobody".into();
            }
            sender_line(ui, &m.summary.from, &to, &m.sent);
            ui.horizontal(|ui| {
                chip(ui, account, LEGEND, false);
                if m.summary.new {
                    chip(ui, "new", TRACE, true);
                }
                if m.summary.flagged {
                    chip(ui, "flagged", READOUT, true);
                }
                ui.add_space(6.0);
                if v.thread.len() > 1 {
                    Line::new().note(format!("{} messages in this conversation", v.thread.len())).size(11.5).show(ui);
                }
            });
            ui.add_space(4.0);
            match header_notes(ui, &m.headers, self.headers_open) {
                HeaderClick::None => {}
                HeaderClick::Unsubscribe => actions.push(Action::Unsubscribe),
                HeaderClick::Toggle => actions.push(Action::ToggleHeaders),
            }
            if self.headers_open {
                all_headers(ui, &m.headers);
            }
            match m.body_state {
                BodyState::Loading => {
                    Line::new().note("Loading message...").size(11.5).show(ui);
                }
                BodyState::Failed => {
                    Line::new().note("Could not load this message. It will load when the server is reachable.").size(11.5).show(ui);
                }
                BodyState::Ready => {}
            }
            if let Some(host) = &m.remote_images {
                if !self.images_loaded.contains(&m.summary.id) {
                    if blocked_images(ui, host) {
                        actions.push(Action::LoadImagesOnce);
                    }
                    ui.add_space(4.0);
                } else {
                    Line::new().note(format!("Images from {host} loaded for this message")).size(11.5).show(ui);
                }
            }
            if m.html.is_empty() {
                for p in &m.paragraphs {
                    body_text(ui, p);
                    ui.add_space(4.0);
                }
                if quoted_fold(ui, m.quoted.len(), self.quotes_open) {
                    actions.push(Action::ToggleQuotes);
                }
                if self.quotes_open {
                    for line in &m.quoted {
                        ui.add(egui::Label::new(egui::RichText::new(line).font(body_italic(13.0)).color(LEGEND)));
                    }
                }
            } else {
                let remote = self.images_loaded.contains(&m.summary.id);
                if let Some(link) = html_body(ui, &self.html, m.summary.id.0, &m.html, remote) {
                    actions.push(Action::OpenLink(link));
                }
            }
            ui.add_space(6.0);
            self.previews_ui(ui, actions);
            ui.horizontal_wrapped(|ui| {
                for (i, a) in m.attachments.iter().enumerate() {
                    let chip = attachment(ui, &a.name, &a.size);
                    egui::Popup::menu(&chip).show(|ui| {
                        ui.set_min_width(120.0);
                        if self.allow_open && ui.add(Key::new("open").outline(VALUE).id_salt(("open", i)).min_width(ui.available_width())).clicked() {
                            actions.push(Action::OpenAttachment(i));
                            ui.close();
                        }
                        if ui.add(Key::new("save as").outline(VALUE).id_salt(("save", i)).min_width(ui.available_width())).clicked() {
                            actions.push(Action::SaveAttachment(i));
                            ui.close();
                        }
                    });
                }
                if m.attachments.len() > 1 && ui.add(Key::new("save all").outline(VALUE)).clicked() {
                    actions.push(Action::SaveAllAttachments);
                }
            });
            for t in v.thread.iter().skip(shown_at + 1) {
                if collapsed_message(ui, &t.summary.from, &t.summary.when, t.summary.unread, self.thread_focus == Some(t.summary.id)) {
                    actions.push(Action::Expand(t.summary.id));
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(Key::new("reply").solid(READOUT).min_width(90.0)).clicked() {
                    actions.push(Action::Reply(Reply::One));
                }
                if ui.add(Key::new("reply all").outline(VALUE).min_width(90.0)).clicked() {
                    actions.push(Action::Reply(Reply::All));
                }
                if ui.add(Key::new("forward").outline(VALUE).min_width(90.0)).clicked() {
                    actions.push(Action::Reply(Reply::Forward));
                }
                if ui
                    .add(Key::new("archive").outline(VALUE).min_width(90.0).enabled(m.folder != Folder::Archive))
                    .clicked()
                {
                    actions.push(Action::One(Box::new(Action::Archive)));
                }
                let more = ui.add(Key::more());
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(140.0);
                    let entries = [
                        ("mark unread", Action::ToggleUnread, VALUE),
                        ("flag", Action::ToggleFlag, VALUE),
                        ("snooze", Action::Snooze, VALUE),
                        ("move to junk", Action::Junk, VALUE),
                        ("delete", Action::Trash, FAULT),
                    ];
                    for (label, action, tint) in entries {
                        if ui.add(Key::new(label).outline(tint).min_width(ui.available_width())).clicked() {
                            actions.push(Action::One(Box::new(action)));
                            ui.close();
                        }
                    }
                });
            });
        });
    }

    fn composer(&mut self, ctx: &egui::Context, accounts: &[AccountInfo]) {
        let Some(draft) = self.draft.as_mut() else { return };
        let (mut send, mut save, mut discard) = (false, false, false);
        let r = egui::Modal::new(egui::Id::new("compose"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(640.0);
                modal_title(ui, &draft.title);
                row(ui, "to", |ui| {
                    field(ui, &mut draft.to, "name or address");
                });
                row(ui, "cc", |ui| {
                    field(ui, &mut draft.cc, "copy to");
                });
                row(ui, "subject", |ui| {
                    field(ui, &mut draft.subject, "subject");
                });
                row(ui, "from", |ui| {
                    ui.allocate_ui(egui::vec2(260.0, KEY_H), |ui| {
                        choice(ui, "from", &mut draft.account, accounts.iter().enumerate().map(|(i, a)| (i, a.address.clone())));
                    });
                });
                ui.add_space(6.0);
                body_editor(ui, &mut draft.body, 9);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let note = if draft.saved { "Saved to drafts, encrypted" } else { "Not saved yet" };
                        Line::new().note(note).size(11.0).show(ui);
                    });
                });
                form_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        send = ui.add(Key::new("send").solid(READOUT).min_width(100.0).enabled(!draft.to.trim().is_empty())).clicked();
                        save = ui.add(Key::new("save draft").outline(VALUE).min_width(100.0)).clicked();
                        discard = ui.add(Key::new("discard").outline(FAULT).min_width(90.0)).clicked();
                    });
                });
            });
        if r.should_close() {
            discard = true;
        }
        if send || save {
            let d = self.draft.clone().unwrap();
            let out = Outgoing { account: d.account, to: d.to, cc: d.cc, subject: d.subject, body: d.body };
            let (folder, said) = if send { (Folder::Sent, "Sent") } else { (Folder::Drafts, "Saved to drafts") };
            match self.store.append(folder, out) {
                Ok(_) => self.notice = Some(said.into()),
                Err(e) => self.notice = Some(format!("Could not save: {e:?}")),
            }
            if send {
                self.draft = None;
            } else if let Some(d) = self.draft.as_mut() {
                d.saved = true;
            }
        } else if discard {
            self.draft = None;
        }
    }
}

fn paint_row(p: &egui::Painter, rect: egui::Rect, m: &Summary, selected: bool, hovered: bool, account: Option<&str>) {
    use egui::Pos2;
    if selected {
        selection(p, rect);
    } else if hovered {
        p.rect_filled(rect, 0.0, BAND.gamma_multiply(0.6));
    }
    let cy = rect.center().y;
    if m.new {
        dot(p, Pos2::new(rect.left() + 13.0, cy), TRACE);
    } else if m.unread {
        dot(p, Pos2::new(rect.left() + 13.0, cy), READOUT);
    }
    let (name_font, name_col, subj_col) =
        if m.unread { (body_strong(13.0), VALUE, VALUE) } else { (body(13.0), LEGEND, LEGEND) };
    let mut x = rect.left() + 26.0;
    put(p, x, 140.0, cy, &m.from, name_font, name_col);
    x += 150.0;
    let right = rect.right() - 10.0;
    put_right(p, right, cy, &m.when, figure(11.0), if m.new { TRACE } else { LEGEND });
    let mut tail = right - 56.0;
    if m.thread > 1 {
        put_right(p, tail, cy, &m.thread.to_string(), figure(10.5), LEGEND);
        tail -= 22.0;
    }
    if m.attachments {
        clip_glyph(p, Pos2::new(tail - 6.0, cy), LEGEND);
        tail -= 18.0;
    }
    if m.flagged {
        flag(p, Pos2::new(tail - 8.0, cy), READOUT);
        tail -= 18.0;
    }
    if let Some(name) = account {
        let g = p.layout_no_wrap(name.to_uppercase(), legend_font(9.5), LEGEND.gamma_multiply(0.8));
        let w = g.size().x;
        p.galley(Pos2::new(tail - w, cy - g.size().y / 2.0), g, LEGEND);
        tail -= w + 14.0;
    }
    put(p, x, (tail - x).max(10.0), cy, &m.subject, body(13.0), subj_col);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::no_frame().show(ui, |ui| self.show(ui));
    }
}
