use super::controller::ChatController;
use super::fake::FakeHandle;
use super::model::*;
use crate::kit::*;
use crate::notify::{Notice as DesktopNotice, Notifier};
use egui::{Align, Color32, Layout, Margin, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const REACTIONS: [&str; 6] = ["+1", "\u{2665}", "\u{2605}", "\u{2713}", "?", "!"];
pub const GROUP_WITHIN_SECS: i64 = 300;
pub const ROW_WIDE: f32 = 56.0;
pub const ROW_NARROW: f32 = 64.0;
pub const NARROW_BELOW: f32 = 700.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatExit {
    Stay,
    Mail,
}

#[derive(Clone, Debug, Default)]
pub struct NewChat {
    pub picked: BTreeSet<MemberId>,
    pub name: String,
}

pub fn hour_minute(ts: i64) -> String {
    let secs = ts.rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

pub fn day_label(ts: i64) -> String {
    crate::real::display::sent(ts).split_whitespace().take(3).collect::<Vec<_>>().join(" ")
}

pub fn same_day(a: i64, b: i64) -> bool {
    a.div_euclid(86_400) == b.div_euclid(86_400)
}

pub fn grouped_with_previous(prev: Option<&ChatMessage>, m: &ChatMessage) -> bool {
    prev.is_some_and(|p| p.sender == m.sender && same_day(p.sent_at, m.sent_at) && m.sent_at - p.sent_at <= GROUP_WITHIN_SECS && !p.deleted)
}

pub fn snippet_of(m: &ChatMessage) -> String {
    if m.deleted {
        return "message deleted".into();
    }
    let line = m.body.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return m.attachments.first().map(|a| a.name.clone()).unwrap_or_default();
    }
    line.chars().take(80).collect()
}

pub fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000 { format!("{:.1} MB", bytes as f64 / 1e6) } else { format!("{} KB", (bytes / 1000).max(1)) }
}

pub fn list_time(ts: i64, now: i64) -> String {
    crate::real::display::when(ts, now)
}

fn reaction_chip(ui: &mut Ui, text: &str, mine: bool) -> egui::Response {
    let tint = if mine { READOUT } else { LEGEND };
    let size = Vec2::new(ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), body(12.5), tint)).size().x + 16.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, if resp.hovered() { BAND } else { WELL });
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, if mine { READOUT } else { ETCH }), egui::StrokeKind::Inside);
    put(p, rect.left() + 8.0, rect.width(), rect.center().y, text, body(12.5), tint);
    let label = format!("reaction {text}");
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    resp
}

fn text_line(ui: &mut Ui, text: impl Into<String>, size: f32, fault: bool) -> egui::Response {
    let tint = if fault { FAULT } else { LEGEND };
    ui.add(egui::Label::new(egui::RichText::new(text.into()).font(body(size)).color(tint)).wrap())
}

fn palette(id: &str) -> Color32 {
    const COLORS: [Color32; 6] = [
        Color32::from_rgb(0x4F, 0x7C, 0xAC),
        Color32::from_rgb(0x8A, 0x6F, 0xB0),
        Color32::from_rgb(0x4E, 0x9A, 0x82),
        Color32::from_rgb(0xB0, 0x7A, 0x4F),
        Color32::from_rgb(0xA8, 0x5F, 0x72),
        Color32::from_rgb(0x6B, 0x8E, 0x3F),
    ];
    let h = id.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[h as usize % COLORS.len()]
}

fn avatar(ui: &mut Ui, id: &str, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.center(), size / 2.0, palette(id));
    let g = ui.fonts_mut(|f| f.layout_no_wrap(initials(name), body_strong(size * 0.4), Color32::WHITE));
    p.galley(rect.center() - g.size() / 2.0, g, Color32::WHITE);
}

pub struct ChatView {
    pub ctrl: ChatController,
    pub selected: Option<RoomId>,
    pub draft: String,
    pub reply: Option<MessageId>,
    pub attachments: Vec<ChatAttachment>,
    pub search: String,
    pub members_open: bool,
    pub new_chat: Option<NewChat>,
    pub topic: String,
    pub add_pick: BTreeSet<MemberId>,
    pub add_note: bool,
    pub divider: Option<MessageId>,
    divider_due: bool,
    pub jump: Option<MessageId>,
    pub to_bottom: bool,
    pub at_bottom: bool,
    pub dwell: Duration,
    pub force_offset: Option<f32>,
    pub offset: f32,
    pub content_h: f32,
    pub handle: Option<FakeHandle>,
    pub notifier: Option<Arc<dyn Notifier>>,
    pub editing: Option<MessageId>,
    pub confirm_leave: bool,
    pub narrow: bool,
    bottom_since: Option<Instant>,
    requested: Option<(RoomId, f32, f32)>,
    shift: Option<(f32, f32)>,
    file_seq: u32,
    script_seq: usize,
}

impl ChatView {
    pub fn new(ctrl: ChatController) -> ChatView {
        ChatView {
            ctrl,
            selected: None,
            draft: String::new(),
            reply: None,
            attachments: Vec::new(),
            search: String::new(),
            members_open: false,
            new_chat: None,
            topic: String::new(),
            add_pick: BTreeSet::new(),
            add_note: false,
            divider: None,
            divider_due: false,
            jump: None,
            to_bottom: false,
            at_bottom: true,
            dwell: Duration::from_millis(800),
            force_offset: None,
            offset: 0.0,
            content_h: 0.0,
            handle: None,
            notifier: None,
            editing: None,
            confirm_leave: false,
            narrow: false,
            bottom_since: None,
            requested: None,
            shift: None,
            file_seq: 0,
            script_seq: 0,
        }
    }

    pub fn demo() -> ChatView {
        let (chat, handle) = super::fake::FakeChat::demo();
        handle.set_auto_reply(true);
        let mut view = ChatView::new(ChatController::start(Box::new(chat), None));
        view.handle = Some(handle);
        view.notifier = Some(Arc::new(crate::notify::DesktopNotifier));
        view
    }

    pub fn unread(&self) -> u32 {
        self.ctrl.state.total_unread()
    }

    pub fn select(&mut self, room: &RoomId) {
        self.divider = None;
        self.divider_due = true;
        self.selected = Some(room.clone());
        self.ctrl.open = Some(room.clone());
        self.members_open = false;
        self.new_chat = None;
        self.reply = None;
        self.editing = None;
        self.confirm_leave = false;
        self.topic = self.ctrl.state.room(room).map(|r| r.topic.clone()).unwrap_or_default();
        self.bottom_since = None;
        self.to_bottom = true;
        self.at_bottom = true;
        if self.ctrl.state.thread(room).len() < super::controller::PAGE {
            self.ctrl.load_history(room);
        }
    }

    fn resolve_divider(&mut self) {
        if !self.divider_due {
            return;
        }
        let Some(room) = self.selected.clone() else { return };
        let list = self.ctrl.state.thread(&room);
        if list.is_empty() {
            return;
        }
        let me = self.ctrl.me().clone();
        let marker = self.ctrl.state.reads.get(&room).cloned();
        let unread = self.ctrl.state.room(&room).is_some_and(|r| r.unread > 0);
        self.divider = match marker.and_then(|id| list.iter().position(|m| m.id == id)) {
            Some(i) => list[i + 1..].iter().find(|m| m.sender != me && !m.deleted).map(|m| m.id.clone()),
            None => list.iter().find(|m| m.sender != me && !m.deleted).map(|m| m.id.clone()).filter(|_| unread),
        };
        self.divider_due = false;
    }

    pub fn back(&mut self) -> ChatExit {
        if self.members_open {
            self.members_open = false;
        } else if self.new_chat.is_some() {
            self.new_chat = None;
        } else if self.selected.is_some() {
            self.selected = None;
            self.ctrl.open = None;
        } else {
            return ChatExit::Mail;
        }
        ChatExit::Stay
    }

    pub fn submit(&mut self) {
        let Some(room) = self.selected.clone() else { return };
        if let Some(id) = self.editing.take() {
            let text = self.draft.trim().to_string();
            if !text.is_empty() {
                self.ctrl.edit(&room, &id, &text);
            }
            self.draft.clear();
            return;
        }
        let draft = ChatDraft { body: self.draft.trim().to_string(), reply_to: self.reply.take(), attachments: std::mem::take(&mut self.attachments) };
        if self.ctrl.send(&room, draft).is_some() {
            self.draft.clear();
            self.to_bottom = true;
        }
    }

    pub fn open_add_people(&mut self) {
        self.members_open = true;
        self.add_pick.clear();
    }

    fn take_enter(ctx: &egui::Context) -> bool {
        ctx.input_mut(|i| {
            let at = i.events.iter().position(|e| {
                matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift && !modifiers.command && !modifiers.alt)
            });
            match at {
                Some(p) => {
                    i.events.remove(p);
                    true
                }
                None => false,
            }
        })
    }

    pub fn show(&mut self, ui: &mut Ui) -> ChatExit {
        let ctx = ui.ctx().clone();
        self.narrow = ui.available_width() < NARROW_BELOW;
        if self.ctrl.pump() {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        self.drain_notices();
        pick_created(self);
        self.resolve_divider();
        self.watch_dwell();
        let mut exit = ChatExit::Stay;
        let narrow = self.narrow;
        egui::Frame::NONE.fill(CHASSIS).show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            ui.set_min_size(full.size());
            if narrow {
                exit = self.show_narrow(ui);
            } else {
                exit = self.show_wide(ui);
            }
        });
        exit
    }

    fn drain_notices(&mut self) {
        let Some(n) = self.notifier.clone() else { return };
        for note in std::mem::take(&mut self.ctrl.notices) {
            n.show(DesktopNotice { title: note.title, body: note.body, focus: 0, account: 0 }, Box::new(|| {}));
        }
    }

    fn watch_dwell(&mut self) {
        let Some(room) = self.selected.clone() else { return };
        if !self.at_bottom || self.new_chat.is_some() || self.members_open && self.narrow {
            self.bottom_since = None;
            return;
        }
        let unread = self.ctrl.state.room(&room).is_some_and(|r| r.unread > 0);
        if !unread {
            return;
        }
        let since = *self.bottom_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= self.dwell {
            self.ctrl.mark_read(&room);
            self.bottom_since = None;
        }
    }

    fn show_wide(&mut self, ui: &mut Ui) -> ChatExit {
        let mut exit = ChatExit::Stay;
        egui::Panel::left("chat-rooms").default_size(320.0).size_range(240.0..=420.0).resizable(true).frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(Margin::same(8))).show(ui, |ui| {
            exit = self.rooms_pane(ui);
        });
        if self.members_open && self.selected.is_some() {
            egui::Panel::right("chat-members").default_size(280.0).size_range(220.0..=360.0).frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(Margin::same(10))).show(ui, |ui| {
                self.members_pane(ui);
            });
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            pane_fill(ui, CHASSIS);
            self.main_pane(ui);
        });
        exit
    }

    fn show_narrow(&mut self, ui: &mut Ui) -> ChatExit {
        let mut exit = ChatExit::Stay;
        egui::Frame::NONE.inner_margin(Margin::same(8)).show(ui, |ui| {
            if self.new_chat.is_some() {
                self.new_chat_pane(ui);
            } else if self.selected.is_some() && self.members_open {
                self.members_pane(ui);
            } else if self.selected.is_some() {
                self.room_pane(ui);
            } else {
                exit = self.rooms_pane(ui);
            }
        });
        exit
    }

    fn main_pane(&mut self, ui: &mut Ui) {
        egui::Frame::NONE.inner_margin(Margin::same(10)).show(ui, |ui| {
            if self.new_chat.is_some() {
                self.new_chat_pane(ui);
            } else if self.selected.is_some() {
                self.room_pane(ui);
            } else {
                empty_state(ui, "No chat open", "Pick a room on the left, or start a new chat.", &[("t", "back to mail")]);
            }
        });
    }

    fn rooms_pane(&mut self, ui: &mut Ui) -> ChatExit {
        let mut exit = ChatExit::Stay;
        let narrow = self.narrow;
        ui.horizontal(|ui| {
            if tap_key(ui, narrow, Key::new("mail").outline(VALUE)).clicked() {
                exit = ChatExit::Mail;
            }
            if tap_key(ui, narrow, Key::new("new chat").solid(READOUT)).clicked() {
                self.new_chat = Some(NewChat::default());
            }
        });
        ui.add_space(6.0);
        let r = field(ui, &mut self.search, "search chats");
        ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("search chats"));
        ui.add_space(6.0);
        if !self.ctrl.online() {
            text_line(ui, "Offline. Messages wait in the outbox.", 11.5, false);
        }
        let now = crate::chat::controller::now_secs();
        let mut open: Option<(RoomId, Option<MessageId>)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !self.search.trim().is_empty() {
                let hits = self.ctrl.search(&self.search);
                if hits.is_empty() {
                    text_line(ui, "No messages match.", 12.0, false);
                }
                for m in hits {
                    let room = self.ctrl.state.room(&m.room).map(|r| r.title(self.ctrl.me())).unwrap_or_default();
                    let who = self.ctrl.state.name_of(&m.room, &m.sender);
                    let label = format!("result in {room}, {who}: {}", snippet_of(&m));
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), if narrow { ROW_NARROW } else { ROW_WIDE }), Sense::click());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(rect, 2.0, BAND);
                    }
                    put(p, rect.left() + 8.0, rect.width() - 16.0, rect.top() + 16.0, &format!("{room}  {who}"), body_strong(13.0), VALUE);
                    put(p, rect.left() + 8.0, rect.width() - 16.0, rect.top() + 38.0, &snippet_of(&m), body(12.5), LEGEND);
                    resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
                    if resp.clicked() {
                        open = Some((m.room.clone(), Some(m.id.clone())));
                    }
                }
                return;
            }
            for room in self.ctrl.state.ordered_rooms() {
                let h = if narrow { ROW_NARROW } else { ROW_WIDE };
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
                let on = self.selected.as_ref() == Some(&room.id);
                let title = room.title(self.ctrl.me());
                let last = room.last_message.as_ref();
                let snippet = last.map(|m| {
                    let who = if &m.sender == self.ctrl.me() { "you".to_string() } else { self.ctrl.state.name_of(&room.id, &m.sender) };
                    format!("{who}: {}", snippet_of(m))
                });
                let dim = if room.muted { 0.5 } else { 1.0 };
                if ui.is_rect_visible(rect) {
                    let p = ui.painter().clone();
                    if on {
                        selection(&p, rect);
                    } else if resp.hovered() {
                        p.rect_filled(rect, 2.0, BAND);
                    }
                    let c = egui::pos2(rect.left() + 8.0 + 20.0, rect.center().y);
                    p.circle_filled(c, 20.0, palette(room.id.as_str()).gamma_multiply(dim));
                    let g = ui.fonts_mut(|f| f.layout_no_wrap(initials(&title), body_strong(15.0), Color32::WHITE));
                    p.galley(c - g.size() / 2.0, g, Color32::WHITE);
                    let left = rect.left() + 56.0;
                    let w = rect.right() - left - 52.0;
                    let ttl = fit_text(ui, &title, body_strong(13.5), VALUE.gamma_multiply(dim), w);
                    p.galley(egui::pos2(left, rect.top() + 9.0), ttl, VALUE);
                    if let Some(s) = &snippet {
                        let g = fit_text(ui, s, body(12.5), LEGEND.gamma_multiply(dim), rect.right() - left - 40.0);
                        p.galley(egui::pos2(left, rect.top() + 31.0), g, LEGEND);
                    }
                    if let Some(m) = last {
                        put_right(&p, rect.right() - 6.0, rect.top() + 15.0, &list_time(m.sent_at, now), figure(11.0), LEGEND);
                    }
                    if room.unread > 0 {
                        let txt = room.unread.to_string();
                        let badge = Rect::from_center_size(egui::pos2(rect.right() - 18.0, rect.top() + 39.0), Vec2::new(24.0, 18.0));
                        p.rect_filled(badge, 9.0, if room.muted { ETCH } else { READOUT });
                        put(&p, badge.left() + 6.0, 20.0, badge.center().y, &txt, figure(11.0), CHASSIS);
                    }
                    if room.pinned {
                        put_right(&p, rect.right() - 6.0, rect.top() + 29.0 + 22.0, "pin", legend_font(8.5), LEGEND);
                    }
                }
                let label = format!(
                    "room {title}{}{}{}",
                    if room.unread > 0 { format!(", {} unread", room.unread) } else { String::new() },
                    if room.muted { ", muted" } else { "" },
                    snippet.map(|s| format!(", {s}")).unwrap_or_default()
                );
                resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
                if resp.clicked() {
                    open = Some((room.id.clone(), None));
                }
            }
            if self.ctrl.state.rooms.is_empty() {
                text_line(ui, "No chats yet. Press new chat to start one.", 12.0, false);
            }
        });
        if let Some((room, jump)) = open {
            self.select(&room);
            self.jump = jump;
            self.search.clear();
        }
        self.dev_controls(ui);
        exit
    }

    fn dev_controls(&mut self, ui: &mut Ui) {
        let Some(handle) = self.handle.clone() else { return };
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let online = self.ctrl.online();
            if ui.add(Key::new(if online { "go offline" } else { "go online" }).outline(LEGEND)).clicked() {
                handle.set_offline(online);
            }
            if ui.add(Key::new("fail next send").outline(LEGEND)).clicked() {
                handle.fail_next_sends(1);
            }
            if ui.add(Key::new("script message").outline(LEGEND)).clicked() {
                if let Some(room) = self.selected.clone() {
                    let who = self.ctrl.state.room(&room).and_then(|r| r.members.iter().find(|m| &m.id != self.ctrl.me()).cloned());
                    if let Some(who) = who {
                        self.script_seq += 1;
                        handle.typing(&room, who.id.as_str(), true);
                        handle.say(&room, who.id.as_str(), &format!("Scripted message {}", self.script_seq));
                    }
                }
            }
        });
    }

    fn room_pane(&mut self, ui: &mut Ui) {
        let Some(room) = self.selected.clone().and_then(|id| self.ctrl.state.room(&id).cloned()) else {
            self.selected = None;
            return;
        };
        let narrow = self.narrow;
        let me = self.ctrl.me().clone();
        ui.horizontal(|ui| {
            if narrow && tap_key(ui, narrow, Key::new("back").outline(VALUE)).clicked() {
                self.back();
            }
            ui.vertical(|ui| {
                subject_line(ui, &room.title(&me));
                let names = room.members.iter().map(|m| if m.id == me { "you".to_string() } else { m.display_name.clone() }).collect::<Vec<_>>().join(", ");
                let count = format!("{} members", room.members.len());
                ui.horizontal_wrapped(|ui| {
                    text_line(ui, &count, 11.5, false);
                    if !room.topic.is_empty() {
                        text_line(ui, format!("{}", room.topic), 11.5, false);
                    }
                });
                if !narrow {
                    text_line(ui, names, 11.0, false);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if tap_key(ui, narrow, Key::new("members").outline(VALUE)).clicked() {
                    self.members_open = !self.members_open;
                    self.topic = room.topic.clone();
                }
            });
        });
        ui.add_space(4.0);
        if !self.ctrl.online() {
            text_line(ui, format!("Offline. {} message(s) waiting to send.", self.ctrl.pending()), 11.5, false);
        }
        if let Some(err) = self.ctrl.error.clone() {
            ui.horizontal(|ui| {
                text_line(ui, format!("Problem: {err}"), 11.5, false);
                if ui.add(Key::new("dismiss").outline(LEGEND)).clicked() {
                    self.ctrl.error = None;
                }
            });
        }
        egui::Panel::bottom("chat-composer").frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 4 })).show_inside(ui, |ui| {
            self.composer(ui, &room);
        });
        self.messages(ui, &room);
    }

    fn messages(&mut self, ui: &mut Ui, room: &Room) {
        let ctx = ui.ctx().clone();
        let caps = self.ctrl.caps.clone();
        let narrow = self.narrow;
        let me = self.ctrl.me().clone();
        let list: Vec<ChatMessage> = self.ctrl.state.thread(&room.id).to_vec();
        let forced = self.force_offset.take();
        let stick = (self.at_bottom || self.to_bottom) && forced.is_none();
        let mut area = egui::ScrollArea::vertical().id_salt(("chat-scroll", room.id.as_str())).auto_shrink([false, false]).stick_to_bottom(stick);
        if let Some(y) = forced {
            area = area.vertical_scroll_offset(y);
        }
        let mut react: Option<(MessageId, String)> = None;
        let mut reply: Option<MessageId> = None;
        let mut edit: Option<(MessageId, String)> = None;
        let mut delete: Option<MessageId> = None;
        let mut retry: Option<MessageId> = None;
        let mut discard: Option<MessageId> = None;
        let mut older = false;
        let jump = self.jump.clone();
        let mut jumped = false;
        let out = area.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            if !self.ctrl.loaded_all.get(&room.id).copied().unwrap_or(false) && caps.paged_history {
                if self.ctrl.loading.get(&room.id).copied().unwrap_or(false) {
                    text_line(ui, "Loading older messages...", 11.5, false);
                } else if ui.add(Key::new("load older").outline(LEGEND)).clicked() {
                    older = true;
                }
            }
            let mut prev: Option<&ChatMessage> = None;
            for m in &list {
                if prev.is_none_or(|p| !same_day(p.sent_at, m.sent_at)) {
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| {
                        text_line(ui, day_label(m.sent_at), 11.0, false);
                    });
                }
                if self.divider.as_ref() == Some(&m.id) {
                    ui.horizontal(|ui| {
                        let r = ui.label(egui::RichText::new("new messages").font(body_strong(11.0)).color(READOUT));
                        let y = r.rect.center().y;
                        ui.painter().hline(r.rect.right() + 6.0..=ui.max_rect().right(), y, Stroke::new(1.0, READOUT));
                    });
                }
                let grouped = grouped_with_previous(prev, m);
                let own = m.sender == me;
                let resp = ui.scope(|ui| {
                    self.message(ui, room, m, &list, grouped, own, narrow, &caps, &mut |a| match a {
                        Act::React(id, e) => react = Some((id, e)),
                        Act::Reply(id) => reply = Some(id),
                        Act::Edit(id, body) => edit = Some((id, body)),
                        Act::Delete(id) => delete = Some(id),
                        Act::Retry(id) => retry = Some(id),
                        Act::Discard(id) => discard = Some(id),
                    });
                });
                if jump.as_ref() == Some(&m.id) {
                    ui.scroll_to_rect(resp.response.rect, Some(Align::Center));
                    jumped = true;
                }
                prev = Some(m);
            }
            let typing = self.ctrl.state.typing_in(&room.id);
            if caps.typing && !typing.is_empty() {
                let names: Vec<String> = typing.iter().map(|t| self.ctrl.state.name_of(&room.id, t)).collect();
                let verb = if names.len() == 1 { "is" } else { "are" };
                text_line(ui, format!("{} {verb} typing...", names.join(", ")), 11.5, false);
            }
            if self.to_bottom {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
        if jumped {
            self.jump = None;
        }
        let viewport_h = out.inner_rect.height();
        let max_off = (out.content_size.y - viewport_h).max(0.0);
        self.offset = out.state.offset().y;
        self.content_h = out.content_size.y;
        let at_bottom = max_off - out.state.offset().y < 6.0;
        if self.to_bottom && at_bottom {
            self.to_bottom = false;
        }
        self.at_bottom = at_bottom;
        if let Some((r, off, h)) = self.requested.clone() {
            if r == room.id {
                if self.ctrl.history_arrived.as_ref().is_some_and(|(ar, _)| ar == &room.id) {
                    self.ctrl.history_arrived = None;
                    self.shift = Some((off, h));
                    self.requested = None;
                }
            }
        }
        if let Some((off, h)) = self.shift.take() {
            let target = off + (out.content_size.y - h).max(0.0);
            self.force_offset = Some(target);
            self.offset = target;
            ctx.request_repaint();
        } else if !self.ctrl.loading.get(&room.id).copied().unwrap_or(false) && out.state.offset().y < 4.0 && !self.at_bottom_only(&out) {
            older = true;
        }
        if older && self.requested.is_none() {
            self.requested = Some((room.id.clone(), out.state.offset().y, out.content_size.y));
            self.ctrl.load_history(&room.id);
        }
        if !self.at_bottom {
            let rect = Rect::from_min_size(egui::pos2(out.inner_rect.right() - 150.0, out.inner_rect.bottom() - 52.0), Vec2::new(140.0, 40.0));
            let mut jump_down = false;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                if tap_key(ui, narrow, Key::new("jump to latest").solid(READOUT)).clicked() {
                    jump_down = true;
                }
            });
            if jump_down {
                self.to_bottom = true;
            }
        }
        if let Some((id, e)) = react {
            self.ctrl.react(&room.id, &id, &e);
        }
        if let Some(id) = reply {
            self.reply = Some(id);
            self.editing = None;
        }
        if let Some((id, body)) = edit {
            self.editing = Some(id);
            self.draft = body;
            self.reply = None;
        }
        if let Some(id) = delete {
            self.ctrl.delete(&room.id, &id);
        }
        if let Some(id) = retry {
            self.ctrl.retry(&id);
        }
        if let Some(id) = discard {
            self.ctrl.discard(&id);
        }
    }

    fn at_bottom_only(&self, out: &egui::scroll_area::ScrollAreaOutput<()>) -> bool {
        out.content_size.y <= out.inner_rect.height() + 1.0
    }

    #[allow(clippy::too_many_arguments)]
    fn message(&self, ui: &mut Ui, room: &Room, m: &ChatMessage, list: &[ChatMessage], grouped: bool, own: bool, narrow: bool, caps: &Capabilities, act: &mut dyn FnMut(Act)) {
        let name = self.ctrl.state.name_of(&room.id, &m.sender);
        let content = |ui: &mut Ui, act: &mut dyn FnMut(Act)| {
            if !grouped {
                let shown = if own { "you".to_string() } else { name.clone() };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(shown).font(body_strong(12.5)).color(if own { READOUT } else { palette(m.sender.as_str()) }));
                    text_line(ui, hour_minute(m.sent_at), 11.0, false);
                });
            }
            let fill = if own { Color32::from_rgb(0x3A, 0x2D, 0x1A) } else { BAND };
            let max_w = (ui.available_width() - 8.0).min(if narrow { 280.0 } else { 520.0 });
            let failed = matches!(m.delivery, Delivery::Failed(_));
            let bubble = egui::Frame::NONE.fill(fill).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.set_max_width(max_w);
                if let Some(q) = m.reply_to.as_ref().and_then(|id| list.iter().find(|x| &x.id == id)) {
                    let who = self.ctrl.state.name_of(&room.id, &q.sender);
                    text_line(ui, format!("{who}: {}", snippet_of(q)), 11.0, false);
                }
                if m.deleted {
                    ui.label(egui::RichText::new("message deleted").font(body_italic(13.0)).color(LEGEND));
                } else if !m.body.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(&m.body).font(body(13.5)).color(VALUE)).wrap());
                }
                for a in &m.attachments {
                    text_line(ui, format!("file: {} ({})", a.name, size_text(a.size)), 12.0, false);
                }
                ui.horizontal(|ui| {
                    if m.edited && !m.deleted {
                        text_line(ui, "edited", 10.5, false);
                    }
                    if own {
                        match &m.delivery {
                            Delivery::Sending => text_line(ui, "sending", 10.5, false),
                            Delivery::Sent => text_line(ui, "\u{2713} sent", 10.5, false),
                            Delivery::Delivered => text_line(ui, "\u{2713}\u{2713} delivered", 10.5, false),
                            Delivery::Failed(why) => text_line(ui, format!("failed: {why}"), 10.5, true),
                        };
                    }
                    if grouped {
                        text_line(ui, hour_minute(m.sent_at), 10.5, false);
                    }
                });
                if failed {
                    ui.horizontal(|ui| {
                        if tap_key(ui, narrow, Key::new("retry").solid(READOUT).id_salt(("retry", m.id.as_str()))).clicked() {
                            act(Act::Retry(m.id.clone()));
                        }
                        if tap_key(ui, narrow, Key::new("delete").outline(FAULT).id_salt(("discard", m.id.as_str()))).clicked() {
                            act(Act::Discard(m.id.clone()));
                        }
                    });
                }
            });
            if !m.reactions.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for r in &m.reactions {
                        let mine = r.members.iter().any(|x| x == self.ctrl.me());
                        let text = format!("{} {}", r.emoji, r.members.len());
                        let resp = reaction_chip(ui, &text, mine);
                        if resp.clicked() && caps.reactions {
                            act(Act::React(m.id.clone(), r.emoji.clone()));
                        }
                    }
                });
            }
            if !m.deleted && !m.is_local() && !failed {
                let more = ui.interact(bubble.response.rect, ui.id().with(("bubble", m.id.as_str())), Sense::click());
                if more.hovered() {
                    ui.painter().rect_stroke(bubble.response.rect, 6.0, Stroke::new(1.0, LEGEND), egui::StrokeKind::Outside);
                }
                more.widget_info({
                    let l = format!("actions for message from {name}");
                    move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, l.clone())
                });
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(150.0);
                    if tap_key(ui, narrow, Key::new("reply").outline(VALUE).id_salt(("reply", m.id.as_str()))).clicked() {
                        act(Act::Reply(m.id.clone()));
                        ui.close();
                    }
                    if caps.reactions {
                        ui.horizontal_wrapped(|ui| {
                            for e in REACTIONS {
                                if tap_key(ui, narrow, Key::new(e).outline(VALUE).id_salt(("react", m.id.as_str(), e))).clicked() {
                                    act(Act::React(m.id.clone(), e.to_string()));
                                    ui.close();
                                }
                            }
                        });
                    }
                    if own && caps.edit && tap_key(ui, narrow, Key::new("edit").outline(VALUE).id_salt(("edit", m.id.as_str()))).clicked() {
                        act(Act::Edit(m.id.clone(), m.body.clone()));
                        ui.close();
                    }
                    if own && caps.delete && tap_key(ui, narrow, Key::new("delete message").outline(FAULT).id_salt(("del", m.id.as_str()))).clicked() {
                        act(Act::Delete(m.id.clone()));
                        ui.close();
                    }
                });
            }
        };
        if own {
            ui.with_layout(Layout::top_down(Align::Max), |ui| content(ui, act));
        } else {
            ui.horizontal_top(|ui| {
                if grouped {
                    ui.add_space(38.0);
                } else {
                    avatar(ui, m.sender.as_str(), &name, 32.0);
                    ui.add_space(6.0);
                }
                ui.vertical(|ui| content(ui, act));
            });
        }
    }

    fn composer(&mut self, ui: &mut Ui, room: &Room) {
        let narrow = self.narrow;
        let caps = self.ctrl.caps.clone();
        if let Some(q) = self.reply.as_ref().and_then(|id| self.ctrl.state.message(&room.id, id)).cloned() {
            ui.horizontal(|ui| {
                let who = self.ctrl.state.name_of(&room.id, &q.sender);
                text_line(ui, format!("Replying to {who}: {}", snippet_of(&q)), 11.5, false);
                if tap_key(ui, narrow, Key::new("cancel reply").outline(LEGEND)).clicked() {
                    self.reply = None;
                }
            });
        }
        if self.editing.is_some() {
            ui.horizontal(|ui| {
                text_line(ui, "Editing a message", 11.5, false);
                if tap_key(ui, narrow, Key::new("cancel edit").outline(LEGEND)).clicked() {
                    self.editing = None;
                    self.draft.clear();
                }
            });
        }
        if !self.attachments.is_empty() {
            ui.horizontal_wrapped(|ui| {
                let mut remove = None;
                for (i, a) in self.attachments.iter().enumerate() {
                    if chip_button(ui, &format!("{} x", a.name), LEGEND).clicked() {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove {
                    self.attachments.remove(i);
                }
            });
        }
        let id = egui::Id::new("chat-composer-text");
        let focused = ui.ctx().memory(|m| m.has_focus(id));
        let enter = focused && Self::take_enter(ui.ctx());
        let over = caps.max_message_len.is_some_and(|max| self.draft.chars().count() > max);
        ui.horizontal(|ui| {
            let side = if narrow { 76.0 } else { 150.0 };
            let w = (ui.available_width() - side).max(80.0);
            egui::Frame::NONE.fill(WELL).stroke(Stroke::new(1.0, ETCH)).corner_radius(2).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_width(w - 18.0);
                let r = ui.add(
                    egui::TextEdit::multiline(&mut self.draft)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .font(body(14.0))
                        .text_color(VALUE)
                        .desired_rows(if narrow { 1 } else { 2 })
                        .hint_text("write a message, Enter sends, Shift+Enter adds a line")
                        .desired_width(f32::INFINITY),
                );
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("message"));
            });
            ui.vertical(|ui| {
                let can = !over && !(self.draft.trim().is_empty() && self.attachments.is_empty());
                if tap_key(ui, narrow, Key::new("send").solid(READOUT).enabled(can)).clicked() || (enter && can) {
                    self.submit();
                }
                if caps.attachments && self.editing.is_none() && tap_key(ui, narrow, Key::new("attach").outline(VALUE)).clicked() {
                    self.file_seq += 1;
                    self.attachments.push(ChatAttachment { name: format!("photo-{}.jpg", self.file_seq), size: 120_000, mime: "image/jpeg".into() });
                }
            });
        });
        if over {
            text_line(ui, "This message is longer than the backend allows.", 11.0, true);
        }
    }

    fn members_pane(&mut self, ui: &mut Ui) {
        let Some(room) = self.selected.clone().and_then(|id| self.ctrl.state.room(&id).cloned()) else { return };
        let narrow = self.narrow;
        let me = self.ctrl.me().clone();
        ui.horizontal(|ui| {
            if tap_key(ui, narrow, Key::new(if narrow { "back to chat" } else { "close" }).outline(VALUE)).clicked() {
                self.members_open = false;
            }
        });
        subject_line(ui, &format!("{} members", room.members.len()));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for m in &room.members {
                ui.horizontal(|ui| {
                    avatar(ui, m.id.as_str(), &m.display_name, if narrow { 36.0 } else { 28.0 });
                    let shown = if m.id == me { format!("{} (you)", m.display_name) } else { m.display_name.clone() };
                    ui.label(egui::RichText::new(shown).font(body(13.0)).color(VALUE));
                    if m.role != Role::Member {
                        chip(ui, m.role.label(), LEGEND, false);
                    }
                    if self.ctrl.caps.presence {
                        if let Some(p) = self.ctrl.state.presence.get(&m.id).copied().or(m.presence) {
                            let tint = if p == Presence::Online { OK } else { LEGEND };
                            dot(ui.painter(), egui::pos2(ui.cursor().left() + 4.0, ui.cursor().center().y), tint);
                        }
                    }
                });
                ui.add_space(2.0);
            }
            ui.add_space(10.0);
            heading(ui, "topic");
            let r = field(ui, &mut self.topic, "room topic");
            ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("room topic"));
            if tap_key(ui, narrow, Key::new("set topic").outline(VALUE)).clicked() {
                self.ctrl.set_topic(&room.id, self.topic.trim());
            }
            ui.add_space(10.0);
            heading(ui, "add people");
            let candidates: Vec<Member> = self.ctrl.contacts.iter().filter(|c| room.member(&c.id).is_none()).cloned().collect();
            if candidates.is_empty() {
                text_line(ui, "Everyone you know is already here.", 11.5, false);
            }
            for c in &candidates {
                let mut on = self.add_pick.contains(&c.id);
                let r = ui.add_sized(Vec2::new(ui.available_width(), if narrow { 44.0 } else { 24.0 }), egui::Checkbox::new(&mut on, format!("add {}", c.display_name)));
                if r.changed() {
                    if on {
                        self.add_pick.insert(c.id.clone());
                    } else {
                        self.add_pick.remove(&c.id);
                    }
                }
            }
            if !self.add_pick.is_empty() && tap_key(ui, narrow, Key::new("add selected").solid(READOUT)).clicked() {
                self.ctrl.add_members(&room.id, self.add_pick.iter().cloned().collect());
                self.add_pick.clear();
            }
            ui.add_space(10.0);
            if self.confirm_leave {
                text_line(ui, "Leave this room? You will lose access to its history.", 12.0, true);
                ui.horizontal(|ui| {
                    if tap_key(ui, narrow, Key::new("confirm leave").solid(FAULT)).clicked() {
                        self.ctrl.leave(&room.id);
                        self.selected = None;
                        self.members_open = false;
                        self.ctrl.open = None;
                        self.confirm_leave = false;
                    }
                    if tap_key(ui, narrow, Key::new("stay").outline(VALUE)).clicked() {
                        self.confirm_leave = false;
                    }
                });
            } else if tap_key(ui, narrow, Key::new("leave room").outline(FAULT)).clicked() {
                self.confirm_leave = true;
            }
        });
    }

    fn new_chat_pane(&mut self, ui: &mut Ui) {
        let narrow = self.narrow;
        let Some(mut nc) = self.new_chat.take() else { return };
        let mut keep = true;
        let mut create = false;
        ui.horizontal(|ui| {
            if tap_key(ui, narrow, Key::new("cancel").outline(VALUE)).clicked() {
                keep = false;
            }
        });
        subject_line(ui, "New chat");
        text_line(ui, "Pick one person for a direct chat, or several and a name for a group.", 12.0, false);
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height((ui.available_height() - 120.0).max(120.0)).show(ui, |ui| {
            for c in self.ctrl.contacts.clone() {
                let mut on = nc.picked.contains(&c.id);
                let r = ui.add_sized(Vec2::new(ui.available_width(), if narrow { 44.0 } else { 26.0 }), egui::Checkbox::new(&mut on, format!("person {}", c.display_name)));
                if r.changed() {
                    if on {
                        nc.picked.insert(c.id.clone());
                    } else {
                        nc.picked.remove(&c.id);
                    }
                }
            }
        });
        ui.add_space(6.0);
        let group = nc.picked.len() > 1;
        if group {
            let r = field(ui, &mut nc.name, "group name");
            ui.ctx().accesskit_node_builder(r.id, |n| n.set_label("group name"));
        }
        let ready = !nc.picked.is_empty() && (!group || !nc.name.trim().is_empty());
        let label = if group { "create group" } else { "start chat" };
        if tap_key(ui, narrow, Key::new(label).solid(READOUT).enabled(ready)).clicked() {
            create = true;
        }
        if create {
            self.ctrl.create(&nc.name, nc.picked.iter().cloned().collect());
            keep = false;
        }
        if keep {
            self.new_chat = Some(nc);
        }
        if let Some(id) = self.ctrl.created.take() {
            self.select(&id);
        }
    }
}

pub fn pick_created(view: &mut ChatView) {
    if let Some(id) = view.ctrl.created.take() {
        view.select(&id);
    }
}

enum Act {
    React(MessageId, String),
    Reply(MessageId),
    Edit(MessageId, String),
    Delete(MessageId),
    Retry(MessageId),
    Discard(MessageId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(sender: &str, at: i64) -> ChatMessage {
        ChatMessage::new("x", &RoomId::new("r"), &MemberId::new(sender), at, "t")
    }

    #[test]
    fn consecutive_messages_group_within_five_minutes_per_sender() {
        let a = msg("ann", 1_000);
        assert!(grouped_with_previous(Some(&a), &msg("ann", 1_000 + 299)));
        assert!(!grouped_with_previous(Some(&a), &msg("ann", 1_000 + 301)));
        assert!(!grouped_with_previous(Some(&a), &msg("bo", 1_010)));
        assert!(!grouped_with_previous(None, &a));
    }

    #[test]
    fn grouping_breaks_at_midnight() {
        let a = msg("ann", 86_400 - 10);
        assert!(!grouped_with_previous(Some(&a), &msg("ann", 86_400 + 10)));
    }

    #[test]
    fn day_labels_drop_the_time() {
        assert_eq!(day_label(1_700_000_000), "Tue 14 Nov");
        assert_eq!(hour_minute(1_700_000_000), "22:13");
    }
}
