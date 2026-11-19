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
    search_id: Option<egui::Id>,
    scroll_to_selected: bool,
    go_pending: bool,
    last_shown: Option<MessageId>,
    last_head: Option<MessageId>,
    pub expanded: Option<MessageId>,
    pub thread_focus: Option<MessageId>,
    pub settings_requested: bool,
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
            search_id: None,
            scroll_to_selected: false,
            go_pending: false,
            last_shown: None,
            last_head: None,
            expanded: None,
            thread_focus: None,
            settings_requested: false,
            html: crate::htmlview::Renderer::default(),
        }
    }

    pub fn query(&self) -> Query {
        Query { folder: self.folder, filter: self.filter, text: self.search.clone() }
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
        if self.draft.is_some() {
            return Vec::new();
        }
        if Self::pressed(ctx, K::Escape) {
            return vec![Action::Back];
        }
        if ctx.egui_wants_keyboard_input() {
            return Vec::new();
        }
        let plain = ctx.input(|i| !i.modifiers.shift);
        let table: [(K, Action); 14] = [
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
            if Self::pressed(ctx, key) {
                out.push(action);
            }
        }
        if plain && Self::pressed(ctx, K::Num3) && ctx.input(|i| i.modifiers.shift) {
            out.push(Action::Trash);
        }
        out
    }

    fn targets(v: &View, whole: bool, same_folder: bool) -> Vec<(MessageId, Folder)> {
        if whole && !v.thread.is_empty() {
            let row = v.query.folder;
            return v
                .thread
                .iter()
                .filter(|t| !same_folder || t.folder == row)
                .map(|t| (t.summary.id, t.folder))
                .collect();
        }
        v.current.iter().map(|m| (m.summary.id, m.folder)).collect()
    }

    fn move_current(&mut self, v: &View, to: Folder, done: &str, whole: bool) {
        for (id, folder) in Self::targets(v, whole, true) {
            if folder == to {
                continue;
            }
            match self.store.move_to(id, to) {
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
            }
            ctx.request_repaint();
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
                    ("c", "compose"),
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
        ui.add_space(12.0);
        heading(ui, "labels");
        ui.add_space(4.0);
        for l in self.store.labels() {
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
            ui.horizontal_wrapped(|ui| {
                for a in &m.attachments {
                    attachment(ui, &a.name, &a.size);
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
