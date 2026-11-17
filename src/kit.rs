use crate::data::Msg;
use egui::{Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, epaint::PathShape};
use egui_bench::prelude::*;

pub const BODY_SIZE: f32 = 14.5;
pub const BODY_LINE: f32 = 22.5;
pub const MEASURE: f32 = 560.0;

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("body".into()))
}

pub fn body_strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("body-strong".into()))
}

pub fn body_italic(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("body-italic".into()))
}

pub fn setup(ctx: &egui::Context) {
    egui_bench::install(ctx);
    for (name, family, bytes) in [
        ("plex-sans", "body", &include_bytes!("../assets/IBMPlexSans-Regular.ttf")[..]),
        ("plex-sans-semibold", "body-strong", &include_bytes!("../assets/IBMPlexSans-SemiBold.ttf")[..]),
        ("plex-sans-italic", "body-italic", &include_bytes!("../assets/IBMPlexSans-Italic.ttf")[..]),
    ] {
        ctx.add_font(egui::FontInsert::new(
            name,
            egui::FontData::from_static(bytes),
            vec![egui::epaint::text::InsertFontFamily {
                family: FontFamily::Name(family.into()),
                priority: egui::epaint::text::FontPriority::Highest,
            }],
        ));
    }
}

pub fn put(p: &egui::Painter, x: f32, w: f32, cy: f32, text: &str, font: FontId, col: Color32) {
    let clip = Rect::from_min_max(Pos2::new(x, cy - 20.0), Pos2::new(x + w, cy + 20.0));
    p.with_clip_rect(clip.intersect(p.clip_rect())).text(Pos2::new(x, cy), Align2::LEFT_CENTER, text, font, col);
}

pub fn put_right(p: &egui::Painter, right: f32, cy: f32, text: &str, font: FontId, col: Color32) {
    p.text(Pos2::new(right, cy), Align2::RIGHT_CENTER, text, font, col);
}

pub fn title(ui: &mut Ui) {
    ui.label(
        egui::RichText::new("MAILBENCH").font(legend_font(16.0)).color(VALUE).extra_letter_spacing(2.5),
    );
}

pub fn header_frame() -> egui::Frame {
    egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 6 })
}

pub fn footer_frame() -> egui::Frame {
    egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin { left: 12, right: 12, top: 5, bottom: 6 })
}

pub fn rule(ui: &Ui, r: Rect) {
    ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, ETCH));
}

pub fn pane_fill(ui: &Ui, fill: Color32) -> Rect {
    let r = ui.max_rect();
    ui.painter().rect_filled(r, 0.0, fill);
    rule(ui, r);
    r
}

pub fn keycap_bar(ui: &mut Ui, items: &[(&str, &str)], status: Option<(&str, Color32)>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (key, action) in items {
            let kg = ui.fonts_mut(|f| f.layout_no_wrap(key.to_string(), figure(11.0), VALUE));
            let ag = ui.fonts_mut(|f| f.layout_no_wrap(action.to_string(), body(12.0), LEGEND));
            let kw = kg.size().x + 10.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(kw + 6.0 + ag.size().x, 20.0), Sense::hover());
            let p = ui.painter();
            let kr = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 1.0), Vec2::new(kw, 18.0));
            p.rect_filled(kr, 2.0, WELL);
            p.rect_stroke(kr, 2.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
            p.galley(kr.center() - kg.size() / 2.0, kg, VALUE);
            p.galley(Pos2::new(kr.right() + 6.0, rect.center().y - ag.size().y / 2.0), ag, LEGEND);
        }
        if let Some((text, tint)) = status {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Line::new().value(text).tint(tint).size(11.0).show(ui);
            });
        }
    });
}

pub fn dot(p: &egui::Painter, c: Pos2, tint: Color32) {
    p.circle_filled(c, 3.5, tint);
}

pub fn flag(p: &egui::Painter, at: Pos2, tint: Color32) {
    let pts = vec![at + Vec2::new(0.0, -5.0), at + Vec2::new(8.0, -2.0), at + Vec2::new(0.0, 1.0)];
    p.add(PathShape::convex_polygon(pts, tint, Stroke::NONE));
    p.line_segment([at + Vec2::new(0.0, -5.0), at + Vec2::new(0.0, 6.0)], Stroke::new(1.5, tint));
}

pub fn clip_glyph(p: &egui::Painter, c: Pos2, tint: Color32) {
    let s = Stroke::new(1.2, tint);
    p.line_segment([c + Vec2::new(-3.0, 4.0), c + Vec2::new(-3.0, -3.0)], s);
    p.line_segment([c + Vec2::new(-3.0, -3.0), c + Vec2::new(2.0, -3.0)], s);
    p.line_segment([c + Vec2::new(2.0, -3.0), c + Vec2::new(2.0, 5.0)], s);
    p.line_segment([c + Vec2::new(2.0, 5.0), c + Vec2::new(-1.0, 5.0)], s);
}

pub fn selection(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, BAND);
    p.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, READOUT_DIM), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(r.left_top(), Vec2::new(3.0, r.height())), 0.0, READOUT);
}

pub fn marker(p: &egui::Painter, m: &Msg, c: Pos2) {
    if m.new {
        dot(p, c, TRACE);
    } else if m.unread {
        dot(p, c, READOUT);
    }
}

pub fn dense_row(ui: &mut Ui, m: &Msg, selected: bool, account: Option<&str>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 25.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter_at(rect);
    if selected {
        selection(&p, rect);
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, BAND.gamma_multiply(0.6));
    }
    let cy = rect.center().y;
    marker(&p, m, Pos2::new(rect.left() + 13.0, cy));
    let (name_font, name_col, subj_col) = if m.unread {
        (body_strong(13.0), VALUE, VALUE)
    } else {
        (body(13.0), LEGEND, LEGEND)
    };
    let mut x = rect.left() + 26.0;
    put(&p, x, 140.0, cy, m.from, name_font, name_col);
    x += 150.0;
    let right = rect.right() - 10.0;
    put_right(&p, right, cy, m.when, figure(11.0), if m.new { TRACE } else { LEGEND });
    let mut tail = right - 56.0;
    if m.count > 1 {
        put_right(&p, tail, cy, &m.count.to_string(), figure(10.5), LEGEND);
        tail -= 22.0;
    }
    if m.attach {
        clip_glyph(&p, Pos2::new(tail - 6.0, cy), LEGEND);
        tail -= 18.0;
    }
    if m.flagged {
        flag(&p, Pos2::new(tail - 8.0, cy), READOUT);
        tail -= 18.0;
    }
    if let Some(name) = account {
        let g = p.layout_no_wrap(name.to_uppercase(), legend_font(9.5), LEGEND.gamma_multiply(0.8));
        let w = g.size().x;
        p.galley(Pos2::new(tail - w, cy - g.size().y / 2.0), g, LEGEND);
        tail -= w + 14.0;
    }
    put(&p, x, (tail - x).max(10.0), cy, m.subject, body(13.0).clone(), subj_col);
    resp
}

pub fn roomy_row(ui: &mut Ui, m: &Msg, selected: bool, account: Option<&str>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 62.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter_at(rect);
    if selected {
        selection(&p, rect);
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, BAND.gamma_multiply(0.6));
    }
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, ETCH.gamma_multiply(0.7)));
    let x = rect.left() + 26.0;
    let right = rect.right() - 12.0;
    marker(&p, m, Pos2::new(rect.left() + 13.0, rect.top() + 16.0));
    let strong = m.unread;
    let (name_font, name_col, subj_col) = if strong {
        (body_strong(13.5), VALUE, VALUE)
    } else {
        (body(13.5), LEGEND, LEGEND)
    };
    put_right(&p, right, rect.top() + 16.0, m.when, figure(11.0), if m.new { TRACE } else { LEGEND });
    let mut tail = right - 50.0;
    if m.count > 1 {
        put_right(&p, tail, rect.top() + 16.0, &format!("{}", m.count), figure(10.5), LEGEND);
        tail -= 24.0;
    }
    if m.flagged {
        flag(&p, Pos2::new(tail - 8.0, rect.top() + 16.0), READOUT);
        tail -= 18.0;
    }
    put(&p, x, tail - x, rect.top() + 16.0, m.from, name_font, name_col);
    put(&p, x, right - x - 20.0, rect.top() + 33.0, m.subject, body(13.0), subj_col);
    let snippet_w = right - x - if m.attach { 36.0 } else { 0.0 } - if account.is_some() { 64.0 } else { 0.0 };
    put(&p, x, snippet_w, rect.top() + 49.0, m.snippet, body(12.0), LEGEND.gamma_multiply(0.85));
    if m.attach {
        clip_glyph(&p, Pos2::new(right - 6.0, rect.top() + 49.0), LEGEND);
    }
    if let Some(name) = account {
        let g = p.layout_no_wrap(name.to_uppercase(), legend_font(9.5), LEGEND.gamma_multiply(0.8));
        p.galley(Pos2::new(right - g.size().x - 20.0, rect.top() + 43.0), g, LEGEND);
    }
    resp
}

pub fn body_text(ui: &mut Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text).font(body(BODY_SIZE)).line_height(Some(BODY_LINE)).color(VALUE),
        )
        .wrap(),
    );
}

pub fn sender_line(ui: &mut Ui, name: &str, to: &str, when: &str, new: bool) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(name).font(body_strong(14.0)).color(VALUE));
        ui.label(egui::RichText::new(format!("to {to}")).font(body(12.5)).color(LEGEND));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            Line::new().value(when).tint(if new { TRACE } else { LEGEND }).size(11.0).show(ui);
        });
    });
}

pub fn subject_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(body_strong(18.0)).color(VALUE)).wrap());
}

pub fn blocked_images(ui: &mut Ui, host: &str) {
    rail_block(ui, WARN, WARN.gamma_multiply(0.12), |ui| {
        ui.horizontal(|ui| {
            ui.label(legend("remote images blocked").color(WARN));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(Key::new("load once").outline(WARN));
            });
        });
        Line::new().note(format!("This message wants to load pictures from {host}, which tells the sender you opened it.")).size(12.0).wrapped(ui);
    });
}

pub fn attachment(ui: &mut Ui, name: &str, size: &str) {
    let g = ui.fonts_mut(|f| f.layout_no_wrap(name.to_string(), body(12.5), VALUE));
    let s = ui.fonts_mut(|f| f.layout_no_wrap(size.to_string(), figure(10.5), LEGEND));
    let w = 26.0 + g.size().x + 12.0 + s.size().x + 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, if resp.hovered() { BAND } else { WELL });
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    clip_glyph(p, rect.left_center() + Vec2::new(13.0, 0.0), LEGEND);
    p.galley(Pos2::new(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g.clone(), VALUE);
    p.galley(Pos2::new(rect.left() + 26.0 + g.size().x + 12.0, rect.center().y - s.size().y / 2.0), s, LEGEND);
}

pub fn quoted_fold(ui: &mut Ui, lines: u32) {
    if lines > 0 {
        Line::new().note(format!("{lines} quoted lines hidden")).size(11.5).show(ui);
    }
}

pub fn body_editor(ui: &mut Ui, text: &mut String, rows: usize) {
    egui::Frame::NONE
        .fill(WELL)
        .stroke(Stroke::new(1.0, ETCH))
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(text)
                    .frame(egui::Frame::NONE)
                    .font(body(BODY_SIZE))
                    .text_color(VALUE)
                    .desired_rows(rows)
                    .desired_width(ui.available_width())
                    .return_key(None),
            );
            ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("message body"));
        });
}

pub struct Draft {
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
}

impl Draft {
    pub fn sample() -> Draft {
        Draft {
            to: "Tomás Rivera".into(),
            cc: "Wren Okafor".into(),
            subject: "Re: Rev C stencil quote and lead time".into(),
            body: "Thanks Tomás. Leave the panel edge unpopulated, Wren wants the test coupon there.\n\nI will pay the invoice this afternoon so it ships next Friday. Please send the tracking number when it goes out.\n\nMara".into(),
        }
    }
}

pub fn address_rows(ui: &mut Ui, d: &mut Draft, with_subject: bool) {
    row(ui, "to", |ui| {
        field(ui, &mut d.to, "name or address");
    });
    row(ui, "cc", |ui| {
        field(ui, &mut d.cc, "copy to");
    });
    if with_subject {
        row(ui, "subject", |ui| {
            field(ui, &mut d.subject, "subject");
        });
    }
}

pub fn send_keys(ui: &mut Ui) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add(Key::new("send").solid(READOUT).min_width(100.0));
        ui.add(Key::new("save draft").outline(VALUE).min_width(100.0));
        ui.add(Key::new("discard").outline(FAULT).min_width(90.0));
        ui.add(Key::new("attach").outline(VALUE).min_width(80.0));
    });
}

pub fn fault_card(ui: &mut Ui, account: &str, address: &str) {
    card(
        ui,
        Some(FAULT),
        |ui| {
            Line::new().legend("sign in failed").tint(FAULT).show(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                chip(ui, account, FAULT, true);
            });
        },
        |ui| {
            note(ui, format!("The server for {address} refused the password. Nothing was deleted. Your stored mail is still readable and new mail is paused for this account."), VALUE);
            ui.add_space(2.0);
            Line::new().legend("server said").column(ui, 98.0).value("AUTHENTICATIONFAILED").show(ui);
            Line::new().legend("next try").column(ui, 98.0).measured("in 28 s").show(ui);
            Line::new().legend("host").column(ui, 98.0).value("imap.kestrel-instruments.example:993").show(ui);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(Key::new("sign in again").solid(READOUT).min_width(130.0));
                ui.add(Key::new("work offline").outline(VALUE).min_width(120.0));
                ui.add(Key::new("settings").outline(VALUE).min_width(90.0));
            });
        },
    );
}

pub fn empty_state(ui: &mut Ui, headline: &str, detail: &str, keys: &[(&str, &str)]) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.30).max(24.0));
        ui.label(egui::RichText::new(headline).font(body_strong(20.0)).color(VALUE));
        ui.add_space(4.0);
        ui.scope(|ui| {
            ui.set_max_width(360.0);
            ui.add(egui::Label::new(egui::RichText::new(detail).font(body(13.5)).color(LEGEND)).wrap());
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let w: f32 = keys.len() as f32 * 140.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            keycap_bar(ui, keys, None);
        });
    });
}
