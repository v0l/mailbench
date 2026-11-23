use super::*;
use egui::Color32;
use crate::thread::{snippet, without_parent_quote, without_parent_text};

fn recipients_line(list: &[String]) -> String {
    if list.is_empty() { "nobody".into() } else { list.join(", ") }
}

impl App {
    pub(super) fn reader(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let Some(m) = &v.current else {
            empty_state(ui, "Nothing selected", "Pick a message from the list, or press j to start at the top.", &[("j", "next"), ("/", "search")]);
            return;
        };
        self.sync_previews(&ui.ctx().clone(), m);
        self.reader_pane_w = ui.available_width();
        self.on_screen.clear();
        let full = self.prefs.reading.reader_width == prefs::ReaderWidth::Full;
        let narrow = self.narrow;
        let spans = v.thread.iter().any(|t| t.summary.account != v.thread[0].summary.account);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !full {
                ui.set_max_width(ui.available_width().min(if m.html.is_empty() { MEASURE + 44.0 } else { 820.0 }));
            }
            self.reader_content_w = ui.available_width();
            ui.spacing_mut().item_spacing.y = self.prefs.density.reader_gap();
            subject_line(ui, &m.summary.subject);
            if v.thread.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    Line::new().note(format!("{} messages in this conversation", v.thread.len())).size(11.5).show(ui);
                    ui.add_space(6.0);
                    if tap_key(ui, narrow, Key::new("expand all").outline(VALUE)).clicked() {
                        actions.push(Action::ExpandAll);
                    }
                    if tap_key(ui, narrow, Key::new("collapse all").outline(VALUE)).clicked() {
                        actions.push(Action::CollapseAll);
                    }
                });
            }
            let server_flags = accounts.get(m.summary.account).is_none_or(|a| a.server_flags);
            for (i, t) in v.thread.iter().enumerate() {
                let id = t.summary.id;
                let open = self.cards.is_open(id.0);
                let focused = id == m.summary.id;
                let est = Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), card_height(narrow)));
                let near = ui.is_rect_visible(est);
                let fetched;
                let msg: Option<&Message> = if focused {
                    Some(m)
                } else if open || (near && !self.snippets.contains_key(&id)) {
                    fetched = self.store.get(id).ok();
                    fetched.as_ref()
                } else {
                    None
                };
                if let Some(msg) = msg.filter(|g| g.body_state == BodyState::Ready) {
                    self.snippets.entry(id).or_insert_with(|| snippet(&msg.paragraphs));
                }
                let snip = self.snippets.get(&id).cloned();
                let to = recipients_line(&t.to);
                let account = if spans { accounts.get(t.summary.account).map(|a| a.name.as_str()) } else { None };
                let head = CardHead {
                    name: &t.summary.from,
                    address: &t.from_address,
                    to: &to,
                    when: &t.summary.when,
                    full: &t.sent,
                    unread: t.summary.unread,
                    flagged: t.summary.flagged,
                    mine: t.mine,
                    attach: t.summary.attachments,
                    account,
                    snippet: snip.as_deref(),
                    focused,
                    open,
                };
                let r = message_card_head(ui, &head, narrow);
                if r.toggle {
                    actions.push(Action::ToggleCard(id));
                }
                egui::Popup::menu(&r.menu).show(|ui| {
                    ui.set_min_width(150.0);
                    let unread_label = if t.summary.unread { "mark read" } else { "mark unread" };
                    let flag_label = if t.summary.flagged { "unflag" } else { "flag" };
                    let entries: Vec<(&str, Vec<Action>, Color32)> = vec![
                        ("reply", vec![Action::Reply(Reply::One)], VALUE),
                        ("reply all", vec![Action::Reply(Reply::All)], VALUE),
                        ("forward", vec![Action::Reply(Reply::Forward)], VALUE),
                        (flag_label, vec![Action::One(Box::new(Action::ToggleFlag))], VALUE),
                        (unread_label, vec![Action::One(Box::new(Action::ToggleUnread))], VALUE),
                        ("delete", vec![Action::One(Box::new(Action::Trash))], FAULT),
                    ];
                    for (label, mut acts, tint) in entries {
                        if label.ends_with("flag") && !server_flags {
                            continue;
                        }
                        if tap_key(ui, narrow, Key::new(label).outline(tint).min_width(ui.available_width()).id_salt((id.0, label))).clicked() {
                            actions.push(Action::FocusThread(id));
                            actions.append(&mut acts);
                            ui.close();
                        }
                    }
                });
                let mut body_rect = r.rect;
                if open {
                    if let Some(msg) = msg {
                        let parent = if i > 0 { self.store.get(v.thread[i - 1].summary.id).ok() } else { None };
                        let inner = egui::Frame::NONE.inner_margin(Margin { left: 10, right: 4, top: 4, bottom: 8 }).show(ui, |ui| {
                            ui.push_id(("card-body", id.0), |ui| self.card_body(ui, msg, parent.as_ref(), focused, actions));
                        });
                        body_rect = r.rect.union(inner.response.rect);
                        if ui.is_rect_visible(inner.response.rect) {
                            self.on_screen.insert(id);
                            if t.summary.unread {
                                self.dwell.entry(id).or_insert_with(Instant::now);
                            }
                        }
                    }
                }
                if self.scroll_card == Some(id) {
                    self.scroll_card = None;
                    ui.scroll_to_rect(body_rect, None);
                }
            }
            ui.add_space(10.0);
            if v.thread.len() > 1 {
                Line::new().note(format!("Replying to {} ({})", m.summary.from, m.summary.when)).size(11.5).show(ui);
            }
            ui.horizontal_wrapped(|ui| {
                if tap_key(ui, narrow, Key::new("reply").solid(READOUT).min_width(90.0)).clicked() {
                    actions.push(Action::Reply(Reply::One));
                }
                if tap_key(ui, narrow, Key::new("reply all").outline(VALUE).min_width(90.0)).clicked() {
                    actions.push(Action::Reply(Reply::All));
                }
                if tap_key(ui, narrow, Key::new("forward").outline(VALUE).min_width(90.0)).clicked() {
                    actions.push(Action::Reply(Reply::Forward));
                }
                if tap_key(ui, narrow, Key::new("archive").outline(VALUE).min_width(90.0).enabled(m.folder != Folder::Archive)).clicked() {
                    actions.push(Action::One(Box::new(Action::Archive)));
                }
                let more = tap_key(ui, narrow, Key::more());
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
                        if label == "flag" && !server_flags {
                            continue;
                        }
                        if ui.add(Key::new(label).outline(tint).min_width(ui.available_width())).clicked() {
                            actions.push(Action::One(Box::new(action)));
                            ui.close();
                        }
                    }
                });
            });
        });
    }

    fn card_body(&mut self, ui: &mut Ui, m: &Message, parent: Option<&Message>, focused: bool, actions: &mut Vec<Action>) {
        let id = m.summary.id;
        let picture = crate::real::display::header_value(&m.headers.all, "X-Nostr-Picture");
        if let Some(url) = picture.filter(|_| self.images_loaded.contains(&id)) {
            let avatar = crate::htmlview::HtmlBody {
                html: format!("<img src=\"{}\" width=\"48\" height=\"48\">", url.replace('"', "")),
                inline: Vec::new(),
            };
            let _ = html_body(ui, &self.html, id.0 ^ 0xA7A7, &avatar, true, &mut HtmlState::default());
        }
        match header_notes(ui, &m.headers, self.headers_open && focused) {
            HeaderClick::None => {}
            HeaderClick::Unsubscribe => {
                actions.push(Action::FocusThread(id));
                actions.push(Action::Unsubscribe);
            }
            HeaderClick::Toggle => {
                actions.push(Action::FocusThread(id));
                actions.push(Action::ToggleHeaders);
            }
        }
        if self.headers_open && focused {
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
        let policy = self.prefs.reading.remote_images;
        let trust = trust_keys(&m.from_address);
        let trusted = policy != RemoteImages::Never && trust.iter().any(|k| self.trusted.contains(k));
        let may_load = policy == RemoteImages::Ask;
        if let Some(host) = &m.remote_images {
            if !(may_load && self.images_loaded.contains(&id)) && !trusted {
                let domain = trust[1].trim_start_matches("domain:");
                match blocked_images(ui, host, &m.from_address, domain, policy) {
                    Some(ImageChoice::Once) => {
                        actions.push(Action::FocusThread(id));
                        actions.push(Action::LoadImagesOnce);
                    }
                    Some(ImageChoice::Sender) => actions.push(Action::TrustImages(trust[0].clone())),
                    Some(ImageChoice::Domain) => actions.push(Action::TrustImages(trust[1].clone())),
                    None => {}
                }
                ui.add_space(4.0);
            } else {
                Line::new().note(format!("Images from {host} loaded for this message")).size(11.5).show(ui);
            }
        }
        if m.html.is_empty() {
            let paras = match parent {
                Some(p) => without_parent_text(&m.paragraphs, &p.paragraphs),
                None => m.paragraphs.clone(),
            };
            let quoted = match parent {
                Some(p) => without_parent_quote(&m.quoted, &p.paragraphs),
                None => m.quoted.clone(),
            };
            let text = paras.join("\n\n");
            let body_id = if focused { egui::Id::new(PLAIN_BODY) } else { egui::Id::new((PLAIN_BODY, id.0)) };
            let picked = plain_body(ui, &text, self.prefs.reading.zoom(), body_id);
            if picked.is_some() || (focused && ui.ctx().memory(|mem| mem.focused().is_none())) {
                self.html_state.selected_text = picked.map(|t| (id.0, t));
            }
            let shown = self.cards.quotes_shown(id.0);
            if quoted_fold(ui, quoted.len(), shown) {
                actions.push(Action::ToggleCardQuotes(id));
            }
            if shown {
                for line in &quoted {
                    ui.add(egui::Label::new(egui::RichText::new(line).font(body_italic(13.0)).color(LEGEND)));
                }
            }
        } else {
            let remote = trusted || (may_load && self.images_loaded.contains(&id));
            if focused {
                if let Some(find) = &mut self.html_state.find {
                    let count = self.html_state.match_count;
                    if find_bar(ui, find, count) {
                        actions.push(Action::Back);
                    }
                }
            }
            if let Some(link) = html_body(ui, &self.html, id.0, &m.html, remote, &mut self.html_state) {
                match link.mismatch {
                    Some((shown, real)) => actions.push(Action::WarnLink(link.href, shown, real)),
                    None => actions.push(Action::OpenLink(link.href)),
                }
            }
        }
        ui.add_space(6.0);
        if focused {
            self.previews_ui(ui, actions);
        }
        ui.horizontal_wrapped(|ui| {
            for (i, a) in m.attachments.iter().enumerate() {
                let chip = attachment(ui, &a.name, &a.size);
                egui::Popup::menu(&chip).show(|ui| {
                    ui.set_min_width(120.0);
                    if self.allow_open && ui.add(Key::new("open").outline(VALUE).id_salt(("open", i)).min_width(ui.available_width())).clicked() {
                        actions.push(Action::FocusThread(id));
                        actions.push(Action::OpenAttachment(i));
                        ui.close();
                    }
                    if ui.add(Key::new("save as").outline(VALUE).id_salt(("save", i)).min_width(ui.available_width())).clicked() {
                        actions.push(Action::FocusThread(id));
                        actions.push(Action::SaveAttachment(i));
                        ui.close();
                    }
                });
            }
            if m.attachments.len() > 1 && ui.add(Key::new("save all").outline(VALUE)).clicked() {
                actions.push(Action::FocusThread(id));
                actions.push(Action::SaveAllAttachments);
            }
        });
    }
}
