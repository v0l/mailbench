use crate::app::State;
use crate::data::*;
use crate::kit::*;
use crate::Screen;
use egui::{Margin, Ui};
use egui_bench::prelude::*;

const COLUMN: f32 = 620.0;

pub fn show(ui: &mut Ui, s: &mut State) {
    egui::Panel::top("c-head").frame(header_frame()).show(ui, |ui| {
        ui.horizontal(|ui| {
            title(ui);
            ui.add_space(18.0);
            ui.allocate_ui(egui::vec2(520.0, KEY_H), |ui| {
                field(ui, &mut s.command, "type a command or search, Ctrl+K");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if s.screen == Screen::Error {
                    lamp(ui, "kestrel signed out", false, true);
                } else {
                    chip(ui, "idle on 3 of 3", TRACE, true);
                }
                ui.add_space(8.0);
                if s.screen != Screen::Empty {
                    Line::new().legend("left").measured("14").show(ui);
                }
            });
        });
    });
    egui::Panel::bottom("c-foot").frame(footer_frame()).show(ui, |ui| {
        let hints: &[(&str, &str)] = match s.screen {
            Screen::Composer => &[("Ctrl+Enter", "send"), ("Esc", "keep as draft"), ("Tab", "next field")],
            Screen::Empty => &[("Ctrl+K", "commands"), ("c", "compose"), ("g a", "archive")],
            _ => &[("e", "archive"), ("r", "reply"), ("s", "snooze"), ("v", "move"), ("#", "delete"), ("x", "flag"), ("Ctrl+K", "commands")],
        };
        keycap_bar(ui, hints, Some(("synced 09:41", TRACE)));
    });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        pane_fill(ui, CHASSIS);
        let pad = ((ui.available_width() - COLUMN) / 2.0).max(0.0) as i8;
        egui::Frame::NONE.inner_margin(Margin { left: pad, right: pad, top: 14, bottom: 0 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            body(ui, s);
        });
    });
}

fn body(ui: &mut Ui, s: &mut State) {
    if s.screen == Screen::Empty {
        empty_state(
            ui,
            "Queue clear",
            "You decided on all 31 messages today. The next thing that arrives appears here by itself.",
            &[("c", "compose"), ("g a", "archive"), ("Ctrl+K", "commands")],
        );
        return;
    }
    if s.screen == Screen::Error {
        fault_card(ui, "kestrel", ACCOUNTS[0].address);
    }
    progress_line(ui);
    focus_card(ui, s);
    if s.screen != Screen::Composer {
        up_next(ui);
    }
}

fn progress_line(ui: &mut Ui) {
    ui.horizontal(|ui| {
        Line::new().legend("queue").column(ui, 60.0).value("17 of 31 decided").show(ui);
    });
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, WELL);
    ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * 17.0 / 31.0, 3.0)), 0.0, READOUT);
}

fn focus_card(ui: &mut Ui, s: &mut State) {
    let turn = &THREAD[2];
    let composing = s.screen == Screen::Composer;
    rail_block(ui, TRACE, PANEL, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add_space(6.0);
        ui.set_max_width(ui.available_width().min(MEASURE + 20.0));
        sender_line(ui, turn.from, "me, Wren Okafor", "Fri 17 Nov 09:41", true);
        subject_line(ui, THREAD_SUBJECT);
        ui.horizontal(|ui| {
            chip(ui, "kestrel", LEGEND, false);
            chip(ui, "new", TRACE, true);
            chip(ui, "2 files", LEGEND, false);
        });
        if s.screen == Screen::Reader {
            blocked_images(ui, BLOCKED_HOST);
        }
        for p in turn.paragraphs.iter().take(if composing { 1 } else { 3 }) {
            body_text(ui, p);
        }
        if composing {
            ui.add_space(4.0);
            ui.label(legend("reply to Tomás Rivera, Wren Okafor"));
            body_editor(ui, &mut s.reply, 5);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(Key::new("send").solid(READOUT).min_width(100.0));
                    ui.add(Key::new("keep draft").outline(VALUE).min_width(100.0));
                    ui.add(Key::new("discard").outline(FAULT).min_width(90.0));
                });
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                for (n, sz) in ATTACHMENTS {
                    attachment(ui, n, sz);
                }
            });
            ui.add_space(4.0);
            action_row(ui);
        }
        ui.add_space(4.0);
    });
}

fn action_row(ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        for (key, label, solid) in [("E", "archive", true), ("R", "reply", false), ("S", "snooze", false), ("V", "move", false), ("X", "flag", false)] {
            let text = format!("{key}  {label}");
            let k = Key::new(text).min_width(112.0);
            ui.add(if solid { k.solid(READOUT) } else { k.outline(VALUE) });
        }
        ui.add(Key::new("#  delete").outline(FAULT).min_width(112.0));
    });
    ui.horizontal(|ui| {
        Line::new().legend("snooze until").show(ui);
        for t in ["this evening 18:00", "tomorrow 08:00", "monday 08:00"] {
            chip(ui, t, READOUT, false);
        }
    });
}

fn up_next(ui: &mut Ui) {
    ui.add_space(6.0);
    heading(ui, "up next");
    ui.spacing_mut().item_spacing.y = 0.0;
    for m in inbox().iter().skip(1).take(6) {
        dense_row(ui, m, false, None);
    }
}
