use crate::app::{Filter, State};
use crate::data::*;
use crate::kit::*;
use crate::Screen;
use egui::{Margin, Ui};
use egui_bench::prelude::*;

pub fn show(ui: &mut Ui, s: &mut State) {
    let failed = s.screen == Screen::Error;
    header(ui, s, failed);
    keys_bar(ui, s);
    egui::Panel::left("a-side")
        .exact_size(196.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(Margin::same(8)))
        .show(ui, |ui| side(ui, failed));
    egui::Panel::left("a-list")
        .exact_size(540.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(CHASSIS))
        .show(ui, |ui| list(ui, s, failed));
    egui::CentralPanel::no_frame().show(ui, |ui| {
        pane_fill(ui, CHASSIS);
        egui::Frame::NONE.inner_margin(Margin { left: 22, right: 22, top: 16, bottom: 10 }).show(ui, |ui| {
            reader(ui, s);
        });
    });
    if s.screen == Screen::Composer {
        composer(ui.ctx(), s);
    }
}

fn header(ui: &mut Ui, s: &mut State, failed: bool) {
    egui::Panel::top("a-head").frame(header_frame()).show(ui, |ui| {
        ui.horizontal(|ui| {
            title(ui);
            ui.add_space(18.0);
            ui.allocate_ui(egui::vec2(360.0, KEY_H), |ui| {
                field(ui, &mut s.search, "search all accounts, press /");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(Key::new("compose").solid(READOUT).min_width(100.0));
                ui.add_space(8.0);
                if failed {
                    lamp(ui, "1 account needs you", false, true);
                } else {
                    lamp(ui, "idle on 3 of 3", true, false);
                }
            });
        });
    });
}

fn keys_bar(ui: &mut Ui, s: &State) {
    egui::Panel::bottom("a-foot").frame(footer_frame()).show(ui, |ui| {
        let _ = s;
        keycap_bar(
            ui,
            &[("j k", "move"), ("Enter", "open"), ("e", "archive"), ("r", "reply"), ("s", "snooze"), ("x", "flag"), ("/", "search")],
            Some(("synced 09:41", TRACE)),
        );
    });
}

fn side(ui: &mut Ui, failed: bool) {
    ui.spacing_mut().item_spacing.y = 2.0;
    heading(ui, "accounts");
    ui.add_space(4.0);
    for (i, a) in ACCOUNTS.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, 2.0, BAND);
        }
        let state = if failed && i == 0 { AccountState::Failed } else { a.state };
        let tint = match state {
            AccountState::Idle => OK,
            AccountState::Syncing => TRACE,
            AccountState::Failed => FAULT,
        };
        p.circle_filled(egui::pos2(rect.left() + 10.0, rect.top() + 13.0), 3.5, tint);
        put(p, rect.left() + 22.0, 100.0, rect.top() + 13.0, a.name, body_strong(13.0), VALUE);
        let status = match state {
            AccountState::Idle => "idle",
            AccountState::Syncing => "sync",
            AccountState::Failed => "sign in",
        };
        put(p, rect.left() + 22.0, 140.0, rect.top() + 28.0, status, legend_font(9.5), if state == AccountState::Failed { FAULT } else { LEGEND });
        if a.unread > 0 {
            put_right(p, rect.right() - 6.0, rect.top() + 13.0, &a.unread.to_string(), figure(11.0), READOUT);
        }
    }
    ui.add_space(12.0);
    heading(ui, "inbox");
    ui.add_space(4.0);
    for (i, (name, n)) in FOLDERS.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
        let p = ui.painter();
        if i == 0 {
            selection(p, rect);
        } else if resp.hovered() {
            p.rect_filled(rect, 2.0, BAND);
        }
        put(p, rect.left() + 12.0, 120.0, rect.center().y, name, body(13.0), if i == 0 { VALUE } else { LEGEND });
        if *n > 0 {
            put_right(p, rect.right() - 6.0, rect.center().y, &n.to_string(), figure(11.0), if i == 0 { READOUT } else { LEGEND });
        }
    }
    ui.add_space(12.0);
    heading(ui, "labels");
    ui.add_space(4.0);
    for (name, n) in LABELS {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, 2.0, BAND);
        }
        put(p, rect.left() + 12.0, 120.0, rect.center().y, name, body(13.0), LEGEND);
        if n > 0 {
            put_right(p, rect.right() - 6.0, rect.center().y, &n.to_string(), figure(11.0), LEGEND);
        }
    }
}

fn list(ui: &mut Ui, s: &mut State, failed: bool) {
    pane_fill(ui, CHASSIS);
    ui.painter().line_segment(
        [ui.max_rect().right_top(), ui.max_rect().right_bottom()],
        egui::Stroke::new(1.0, ETCH),
    );
    let msgs = inbox();
    let unread = msgs.iter().filter(|m| m.unread).count();
    let flagged = msgs.iter().filter(|m| m.flagged).count();
    egui::Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
            segments(
                ui,
                &mut s.filter,
                &[
                    (Filter::All, "all", Some(msgs.len())),
                    (Filter::Unread, "unread", Some(unread)),
                    (Filter::Flagged, "flagged", Some(flagged)),
                ],
            );
        });
        ui.add_space(6.0);
        if failed {
            egui::Frame::NONE.inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                fault_card(ui, "kestrel", ACCOUNTS[0].address);
            });
            ui.add_space(8.0);
        }
        let empty = s.screen == Screen::Empty;
        if empty {
            empty_state(
                ui,
                "Inbox zero",
                "Everything in kestrel, home and lists is read and filed. New mail shows up here as it arrives.",
                &[("g a", "go to archive"), ("c", "compose")],
            );
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, m) in msgs.iter().enumerate() {
                if matches!(s.filter, Filter::Unread) && !m.unread {
                    continue;
                }
                if matches!(s.filter, Filter::Flagged) && !m.flagged {
                    continue;
                }
                let r = dense_row(ui, m, i == s.selected, Some(ACCOUNTS[m.account].name));
                if r.clicked() {
                    s.selected = i;
                }
            }
        });
    });
}

fn reader(ui: &mut Ui, s: &mut State) {
    if s.screen == Screen::Empty {
        empty_state(ui, "Nothing selected", "Pick a message from the list, or press j to start at the top.", &[("j", "next"), ("/", "search")]);
        return;
    }
    let turn = &THREAD[2];
    ui.vertical(|ui| {
        ui.set_max_width((ui.available_width()).min(MEASURE + 44.0));
        ui.spacing_mut().item_spacing.y = 8.0;
        subject_line(ui, THREAD_SUBJECT);
        sender_line(ui, turn.from, "me, Wren Okafor", "Fri 17 Nov 09:41", true);
        ui.horizontal(|ui| {
            chip(ui, "kestrel", LEGEND, false);
            chip(ui, "new", TRACE, true);
            chip(ui, "flagged", READOUT, true);
            ui.add_space(6.0);
            Line::new().note("3 messages in this conversation").size(11.5).show(ui);
        });
        ui.add_space(4.0);
        if s.screen == Screen::Reader {
            blocked_images(ui, BLOCKED_HOST);
            ui.add_space(4.0);
        }
        for p in turn.paragraphs {
            body_text(ui, p);
            ui.add_space(4.0);
        }
        quoted_fold(ui, turn.quoted_lines);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (n, sz) in ATTACHMENTS {
                attachment(ui, n, sz);
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(Key::new("reply").solid(READOUT).min_width(90.0));
            ui.add(Key::new("reply all").outline(VALUE).min_width(90.0));
            ui.add(Key::new("forward").outline(VALUE).min_width(90.0));
            ui.add(Key::new("archive").outline(VALUE).min_width(90.0));
            ui.add(Key::more());
        });
    });
}

fn composer(ctx: &egui::Context, s: &mut State) {
    egui::Modal::new(egui::Id::new("compose"))
        .backdrop_color(egui::Color32::from_black_alpha(150))
        .show(ctx, |ui| {
            ui.set_width(640.0);
            modal_title(ui, "reply to Tomás Rivera");
            address_rows(ui, &mut s.draft, true);
            row(ui, "from", |ui| {
                ui.allocate_ui(egui::vec2(260.0, KEY_H), |ui| {
                    let mut v = 0usize;
                    choice(ui, "from", &mut v, ACCOUNTS.iter().enumerate().map(|(i, a)| (i, a.address.to_string())));
                });
            });
            ui.add_space(6.0);
            body_editor(ui, &mut s.draft.body, 9);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                attachment(ui, "revc-fiducials.png", "41 KB");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Line::new().note("Saved to drafts 09:58, encrypted").size(11.0).show(ui);
                });
            });
            footer(ui, send_keys);
        });
}
