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

pub fn form_footer(ui: &mut Ui, buttons: impl FnOnce(&mut Ui)) {
    ui.add_space(10.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([Pos2::new(r.left(), r.top()), Pos2::new(r.right(), r.top())], Stroke::new(1.0, ETCH));
    ui.add_space(8.0);
    let size = Vec2::new(ui.available_width(), KEY_H);
    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), buttons);
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

#[derive(Clone)]
struct HtmlTextures {
    source: usize,
    textures: Vec<(egui::TextureHandle, u32, u32)>,
}

fn html_textures(ui: &Ui, page: &std::sync::Arc<crate::htmlview::Rendered>) -> Vec<(egui::TextureHandle, u32, u32)> {
    let source = std::sync::Arc::as_ptr(page) as usize;
    let id = egui::Id::new("html-textures");
    if let Some(cached) = ui.ctx().data(|d| d.get_temp::<HtmlTextures>(id)).filter(|c| c.source == source) {
        return cached.textures;
    }
    let textures: Vec<_> = page
        .tiles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let image = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
            (ui.ctx().load_texture(format!("html-{source}-{i}"), image, egui::TextureOptions::LINEAR), t.top, t.height)
        })
        .collect();
    ui.ctx().data_mut(|d| d.insert_temp(id, HtmlTextures { source, textures: textures.clone() }));
    textures
}

pub fn html_body(
    ui: &mut Ui,
    renderer: &crate::htmlview::Renderer,
    id: u64,
    body: &crate::htmlview::HtmlBody,
    remote: bool,
) -> Option<String> {
    let width = (ui.available_width().floor() as u32 / 16 * 16).max(160);
    let scale = ui.ctx().pixels_per_point();
    let key = crate::htmlview::RenderKey { id, width, scale_milli: (scale * 1000.0).round() as u32, remote };
    let page = renderer.get(&key, body).or_else(|| renderer.latest_for(id));
    let Some(page) = page else {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        Line::new().note("Rendering message...").size(11.5).show(ui);
        return None;
    };
    if page.width as u32 != width || !page.scale.eq(&scale) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    }
    let shown = Vec2::new(page.width.min(ui.available_width()), page.height);
    let (rect, resp) = ui.allocate_exact_size(shown, Sense::click());
    let painter = ui.painter_at(rect);
    let ratio = rect.width() / page.width;
    for (tex, top, h) in html_textures(ui, &page) {
        let y = rect.top() + top as f32 / page.scale * ratio;
        let r = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), h as f32 / page.scale * ratio));
        painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    let hit = |p: Pos2| {
        let local = (p - rect.left_top()) / ratio;
        page.links.iter().find(|l| local.x >= l.rect[0] && local.x <= l.rect[0] + l.rect[2] && local.y >= l.rect[1] && local.y <= l.rect[1] + l.rect[3])
    };
    let hovered = resp.hover_pos().and_then(hit);
    if let Some(link) = hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        resp.clone().on_hover_text_at_pointer(&link.href);
    }
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label("message body"));
    if resp.clicked() {
        return resp.interact_pointer_pos().and_then(hit).map(|l| l.href.clone());
    }
    None
}

pub fn body_text(ui: &mut Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text).font(body(BODY_SIZE)).line_height(Some(BODY_LINE)).color(VALUE),
        )
        .wrap(),
    );
}

pub fn sender_line(ui: &mut Ui, name: &str, to: &str, when: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(name).font(body_strong(14.0)).color(VALUE));
        ui.label(egui::RichText::new(format!("to {to}")).font(body(12.5)).color(LEGEND));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            Line::new().measured(when).size(11.0).show(ui);
        });
    });
}

pub fn collapsed_message(ui: &mut Ui, from: &str, when: &str, unread: bool) -> bool {
    let font = if unread { body_strong(13.0) } else { body(13.0) };
    let text = format!("{from}   {when}");
    let resp = ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(LEGEND)).sense(Sense::click()));
    resp.clicked()
}

pub fn subject_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(body_strong(18.0)).color(VALUE)).wrap());
}

pub fn blocked_images(ui: &mut Ui, host: &str) -> bool {
    let mut load = false;
    rail_block(ui, WARN, WARN.gamma_multiply(0.12), |ui| {
        ui.horizontal(|ui| {
            ui.label(legend("remote images blocked").color(WARN));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                load = ui.add(Key::new("load once").outline(WARN)).clicked();
            });
        });
        Line::new().note(format!("This message wants to load pictures from {host}, which tells the sender you opened it.")).size(12.0).wrapped(ui);
    });
    load
}

pub fn attachment(ui: &mut Ui, name: &str, size: &str) -> egui::Response {
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
    resp
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeaderClick {
    None,
    Unsubscribe,
    Toggle,
}

pub fn header_notes(ui: &mut Ui, h: &crate::mailstore::Headers, open: bool) -> HeaderClick {
    let mut click = HeaderClick::None;
    ui.horizontal_wrapped(|ui| {
        if let Some(list) = &h.list_id {
            chip(ui, list, LEGEND, false);
        } else if h.bulk {
            chip(ui, "bulk", LEGEND, false);
        }
        if h.auth.present() {
            if h.auth.failed() {
                chip(ui, &format!("not verified: {}", h.auth.summary()), FAULT, true);
            } else {
                chip(ui, &h.auth.summary(), OK, false);
            }
        }
        if h.spam {
            chip(ui, "marked as spam", FAULT, true);
        } else if let Some(score) = h.spam_score.filter(|s| *s >= 3.0) {
            chip(ui, &format!("spam score {score:.1}"), WARN, false);
        }
        if let Some(r) = &h.reply_to_differs {
            chip(ui, &format!("replies go to {r}"), WARN, true);
        }
        if h.unsubscribe.is_some() && chip_button(ui, "unsubscribe", WARN).clicked() {
            click = HeaderClick::Unsubscribe;
        }
        if !h.all.is_empty() {
            let label = if open { "hide headers" } else { "all headers" };
            if chip_button(ui, label, VALUE).clicked() {
                click = HeaderClick::Toggle;
            }
        }
    });
    click
}

pub fn chip_button(ui: &mut Ui, text: &str, tint: Color32) -> egui::Response {
    let galley = egui_bench::text::lay(ui, egui_bench::text::caps(text, egui_bench::theme::legend_font(10.5), tint, 1.4), egui_bench::text::room(ui) - 14.0);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(14.0, 6.0), Sense::click());
    let hot = resp.hovered() || resp.has_focus();
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 2.0, if hot { BAND } else { WELL });
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, if hot { tint } else { ETCH }), egui::StrokeKind::Inside);
        p.galley(rect.center() - galley.size() / 2.0, galley, tint);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let label = text.to_string();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    resp
}

pub fn all_headers(ui: &mut Ui, h: &crate::mailstore::Headers) {
    egui::Frame::NONE.fill(WELL).stroke(Stroke::new(1.0, ETCH)).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (k, v) in &h.all {
            ui.horizontal_wrapped(|ui| {
                ui.label(legend(k).color(LEGEND));
                ui.add(egui::Label::new(egui::RichText::new(v).font(figure(11.0)).color(VALUE)).wrap());
            });
        }
    });
}

pub fn quoted_fold(ui: &mut Ui, lines: usize, open: bool) -> bool {
    if lines == 0 {
        return false;
    }
    let text = if open { format!("{lines} quoted lines shown, click to hide") } else { format!("{lines} quoted lines hidden") };
    let g = ui.fonts_mut(|f| f.layout_no_wrap(text, body(11.5), LEGEND));
    let (rect, resp) = ui.allocate_exact_size(g.size() + Vec2::new(0.0, 6.0), Sense::click());
    ui.painter().galley(rect.left_top() + Vec2::new(0.0, 3.0), g, if resp.hovered() { VALUE } else { LEGEND });
    let label = if open { format!("{lines} quoted lines shown") } else { format!("{lines} quoted lines hidden") };
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label(label));
    resp.clicked()
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
