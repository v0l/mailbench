use crate::app::State;
use crate::data::*;
use crate::kit::*;
use crate::Screen;
use egui::{Margin, Ui};
use egui_bench::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    Account(usize),
}

pub fn show(ui: &mut Ui, s: &mut State) {
    let failed = s.screen == Screen::Error;
    let mut scope = Scope::All;
    egui::Panel::top("b-head").frame(header_frame()).show(ui, |ui| {
        ui.horizontal(|ui| {
            title(ui);
            ui.add_space(18.0);
            let mut opts = vec![(Scope::All, "all mail".to_string())];
            opts.extend(ACCOUNTS.iter().enumerate().map(|(i, a)| (Scope::Account(i), a.name.to_string())));
            let refs: Vec<(Scope, &str)> = opts.iter().map(|(v, l)| (*v, l.as_str())).collect();
            ui.allocate_ui(egui::vec2(330.0, 24.0), |ui| {
                tabs(ui, &mut scope, &refs);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(Key::new("compose").solid(READOUT).min_width(100.0));
                ui.add_space(8.0);
                if failed {
                    lamp(ui, "kestrel signed out", false, true);
                } else {
                    chip(ui, "idle", TRACE, true);
                }
                ui.add_space(8.0);
                ui.allocate_ui(egui::vec2(220.0, KEY_H), |ui| {
                    field(ui, &mut s.search, "search, press /");
                });
            });
        });
    });
    egui::Panel::bottom("b-foot").frame(footer_frame()).show(ui, |ui| {
        keycap_bar(
            ui,
            &[("j k", "conversation"), ("Enter", "expand turn"), ("r", "reply here"), ("Ctrl+Enter", "send"), ("e", "archive"), ("Esc", "back to list")],
            Some(("synced 09:41", TRACE)),
        );
    });
    egui::Panel::left("b-list")
        .exact_size(430.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(CHASSIS))
        .show(ui, |ui| {
            pane_fill(ui, CHASSIS);
            ui.painter().line_segment([ui.max_rect().right_top(), ui.max_rect().right_bottom()], egui::Stroke::new(1.0, ETCH));
            list(ui, s, failed);
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        pane_fill(ui, CHASSIS);
        egui::Frame::NONE.inner_margin(Margin { left: 24, right: 24, top: 16, bottom: 10 }).show(ui, |ui| {
            conversation(ui, s);
        });
    });
}

fn list(ui: &mut Ui, s: &mut State, failed: bool) {
    if s.screen == Screen::Empty {
        empty_state(
            ui,
            "No conversations here",
            "Nothing is waiting in all mail. Archived threads are one search away.",
            &[("g a", "archive"), ("/", "search")],
        );
        return;
    }
    if failed {
        egui::Frame::NONE.inner_margin(Margin { left: 8, right: 8, top: 8, bottom: 0 }).show(ui, |ui| {
            fault_card(ui, "kestrel", ACCOUNTS[0].address);
        });
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, m) in inbox().iter().enumerate().take(12) {
            if roomy_row(ui, m, i == s.selected, Some(ACCOUNTS[m.account].name)).clicked() {
                s.selected = i;
            }
        }
    });
}

fn conversation(ui: &mut Ui, s: &mut State) {
    if s.screen == Screen::Empty {
        empty_state(ui, "Nothing open", "Press Enter on a conversation to read it here, with the reply box under it.", &[("Enter", "open"), ("j", "next")]);
        return;
    }
    ui.vertical(|ui| {
        ui.set_max_width(ui.available_width().min(MEASURE + 40.0));
        ui.spacing_mut().item_spacing.y = 8.0;
        subject_line(ui, THREAD_SUBJECT);
        ui.horizontal(|ui| {
            chip(ui, "kestrel", LEGEND, false);
            chip(ui, "1 new", TRACE, true);
            ui.add_space(6.0);
            Line::new().note("Tomás Rivera, Wren Okafor, you").size(11.5).show(ui);
        });
        ui.add_space(2.0);
        let composing = s.screen == Screen::Composer;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(MEASURE + 40.0));
            ui.spacing_mut().item_spacing.y = 6.0;
            for (i, t) in THREAD.iter().enumerate() {
                let open = i == 2 || (composing && i == 2);
                if open {
                    open_turn(ui, t, s.screen == Screen::Reader, composing);
                } else {
                    folded_turn(ui, t);
                }
            }
            ui.add_space(6.0);
            reply_box(ui, s, composing);
        });
    });
}

fn folded_turn(ui: &mut Ui, t: &Turn) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, if resp.hovered() { BAND } else { WELL });
    p.line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, ETCH));
    let cy = rect.center().y;
    let col = if t.mine { READOUT } else { VALUE };
    put(&p, rect.left() + 12.0, 130.0, cy, t.from, body_strong(13.0), col);
    let first = t.paragraphs[0];
    put(&p, rect.left() + 142.0, rect.width() - 142.0 - 130.0, cy, first, body(12.5), LEGEND);
    put_right(&p, rect.right() - 10.0, cy, t.when, figure(10.5), LEGEND);
}

fn open_turn(ui: &mut Ui, t: &Turn, blocked: bool, compact: bool) {
    rail_block(ui, if t.new { TRACE } else { ETCH }, PANEL, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add_space(4.0);
        sender_line(ui, t.from, t.to, t.when, t.new);
        if blocked {
            blocked_images(ui, BLOCKED_HOST);
        }
        for p in t.paragraphs.iter().take(if compact { 1 } else { 9 }) {
            body_text(ui, p);
        }
        if !compact {
            quoted_fold(ui, t.quoted_lines);
            ui.horizontal_wrapped(|ui| {
                for (n, sz) in ATTACHMENTS {
                    attachment(ui, n, sz);
                }
            });
        }
        ui.add_space(2.0);
    });
}

fn reply_box(ui: &mut Ui, s: &mut State, composing: bool) {
    rail_block(ui, READOUT, WELL, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(legend("reply to Tomás Rivera, Wren Okafor"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Line::new().note("draft saved 09:58, encrypted").size(11.0).show(ui);
            });
        });
        let rows = if composing { 5 } else { 2 };
        body_editor(ui, &mut s.reply, rows);
        if composing {
            ui.horizontal(|ui| {
                attachment(ui, "revc-fiducials.png", "41 KB");
            });
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(Key::new("send").solid(READOUT).min_width(100.0));
                ui.add(Key::new("reply all").outline(VALUE).min_width(100.0));
                ui.add(Key::new("attach").outline(VALUE).min_width(80.0));
                ui.add(Key::new("discard").outline(FAULT).min_width(90.0));
            });
        });
        ui.add_space(2.0);
    });
}
