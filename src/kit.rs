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
    add_fallbacks(ctx);
}

const CJK_FONTS: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\YuGothM.ttc",
];

const EMOJI_FONTS: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
    "/System/Library/Fonts/Apple Color Emoji.ttc",
    "C:\\Windows\\Fonts\\seguiemj.ttf",
];

fn add_fallbacks(ctx: &egui::Context) {
    let defaults = egui::FontDefinitions::default();
    let mut stack: Vec<(String, std::sync::Arc<egui::FontData>)> = defaults
        .families
        .get(&FontFamily::Proportional)
        .into_iter()
        .flatten()
        .filter_map(|n| defaults.font_data.get(n).map(|d| (format!("fallback-{n}"), d.clone())))
        .collect();
    if let Some(bytes) = CJK_FONTS.iter().find_map(|p| std::fs::read(p).ok()) {
        stack.push(("fallback-cjk".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes))));
    }
    if let Some(bytes) = EMOJI_FONTS.iter().find_map(|p| std::fs::read(p).ok()) {
        stack.push(("fallback-emoji".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes))));
    }
    for (name, data) in stack {
        let families = ["body", "body-strong", "body-italic"]
            .into_iter()
            .map(|f| FontFamily::Name(f.into()))
            .chain([FontFamily::Proportional, FontFamily::Monospace])
            .map(|family| egui::epaint::text::InsertFontFamily { family, priority: egui::epaint::text::FontPriority::Lowest })
            .collect();
        ctx.add_font(egui::FontInsert { name, data: (*data).clone(), families });
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

#[derive(Clone, Default)]
struct HtmlTextures {
    source: usize,
    textures: Vec<Option<egui::TextureHandle>>,
}

#[derive(Clone, Debug, Default)]
pub struct FindState {
    pub query: String,
    pub index: usize,
    pub focus: bool,
    pub jump: bool,
}

#[derive(Default)]
pub struct HtmlState {
    pub zoom: std::collections::HashMap<u64, u32>,
    pub dark: bool,
    pub default_zoom: u32,
    pub shown_width: f32,
    pub find: Option<FindState>,
    pub selection: Option<(crate::htmlview::TextPos, crate::htmlview::TextPos)>,
    pub selection_for: Option<crate::htmlview::RenderKey>,
    pub selected_text: Option<(u64, String)>,
    pub match_count: usize,
}

impl HtmlState {
    fn base_zoom(&self) -> u32 {
        if self.default_zoom == 0 { 1000 } else { self.default_zoom }
    }
}

pub struct LinkClick {
    pub href: String,
    pub mismatch: Option<(String, String)>,
}

const ZOOM_STEPS: [u32; 11] = [500, 600, 700, 800, 900, 1000, 1100, 1250, 1500, 1750, 2000];

fn zoom_step(current: u32, up: bool) -> u32 {
    if up {
        ZOOM_STEPS.iter().copied().find(|z| *z > current).unwrap_or(2000)
    } else {
        ZOOM_STEPS.iter().rev().copied().find(|z| *z < current).unwrap_or(500)
    }
}

fn rect_of(origin: Pos2, r: [f32; 4], ratio: f32) -> Rect {
    Rect::from_min_size(origin + Vec2::new(r[0], r[1]) * ratio, Vec2::new(r[2], r[3]) * ratio)
}

pub fn find_bar(ui: &mut Ui, find: &mut FindState, count: usize) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut find.query).hint_text("find in message").desired_width(220.0));
        if find.focus {
            edit.request_focus();
            find.focus = false;
        }
        if edit.changed() {
            find.index = 0;
            find.jump = true;
        }
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let shift = ui.input(|i| i.modifiers.shift);
            step_find(find, count, !shift);
            edit.request_focus();
        }
        let label = if find.query.trim().is_empty() {
            String::new()
        } else if count == 0 {
            "no matches".to_string()
        } else {
            format!("{} of {count}", find.index.min(count - 1) + 1)
        };
        Line::new().measured(label).size(11.0).show(ui);
        if ui.add(Key::new("prev").outline(VALUE)).clicked() {
            step_find(find, count, false);
        }
        if ui.add(Key::new("next").outline(VALUE)).clicked() {
            step_find(find, count, true);
        }
        if ui.add(Key::new("done").outline(VALUE)).clicked() {
            close = true;
        }
    });
    close
}

fn step_find(find: &mut FindState, count: usize, forward: bool) {
    if count == 0 {
        return;
    }
    find.index = if forward { (find.index + 1) % count } else { (find.index + count - 1) % count };
    find.jump = true;
}

pub fn html_body(
    ui: &mut Ui,
    renderer: &crate::htmlview::Renderer,
    id: u64,
    body: &crate::htmlview::HtmlBody,
    remote: bool,
    state: &mut HtmlState,
) -> Option<LinkClick> {
    let width = (ui.available_width().floor() as u32 / 16 * 16).max(160);
    let scale = ui.ctx().pixels_per_point();
    let mut zoom = state.zoom.get(&id).copied().unwrap_or(state.base_zoom());
    if !ui.ctx().egui_wants_keyboard_input() {
        let (reset, inn, out) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Equals) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Plus),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Minus),
            )
        });
        if reset {
            zoom = state.base_zoom();
        } else if inn {
            zoom = zoom_step(zoom, true);
        } else if out {
            zoom = zoom_step(zoom, false);
        }
        state.zoom.insert(id, zoom);
    }
    let key = crate::htmlview::RenderKey { id, width, scale_milli: (scale * 1000.0).round() as u32, remote, zoom_milli: zoom, dark: state.dark };
    let page = renderer.get(&key, body).or_else(|| renderer.latest_for(id));
    let Some(page) = page else {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        Line::new().note("Rendering message...").size(11.5).show(ui);
        return None;
    };
    if page.key != key {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    }
    if state.selection_for.as_ref() != Some(&page.key) {
        state.selection = None;
        state.selection_for = Some(page.key.clone());
    }
    let overflow = page.width > ui.available_width() + 1.0;
    state.shown_width = page.width;
    let shown = Vec2::new(if overflow { page.width } else { page.width.min(ui.available_width()) }, page.height);
    let (rect, resp) = if overflow {
        egui::ScrollArea::horizontal()
            .id_salt(("html-scroll", id))
            .auto_shrink([false, true])
            .show(ui, |ui| ui.allocate_exact_size(shown, Sense::click_and_drag()))
            .inner
    } else {
        ui.allocate_exact_size(shown, Sense::click_and_drag())
    };
    let painter = ui.painter_at(rect);
    let ratio = rect.width() / page.width;
    paint_tiles(ui, renderer, &page, rect, ratio);

    let local = |p: Pos2| (p - rect.left_top()) / ratio;
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        if let Some(p) = origin.and_then(|p| page.pos_at(local(p).x, local(p).y)) {
            state.selection = Some((p, p));
        }
    } else if resp.dragged() {
        if let (Some((a, _)), Some(p)) = (state.selection, resp.interact_pointer_pos().and_then(|p| page.pos_at(local(p).x, local(p).y))) {
            state.selection = Some((a, p));
        }
    }
    if let Some((a, b)) = state.selection {
        for r in page.selection_rects(a, b) {
            painter.rect_filled(rect_of(rect.left_top(), r, ratio), 0.0, READOUT.gamma_multiply(0.35));
        }
    }

    let mut matches = Vec::new();
    if let Some(find) = &mut state.find {
        matches = page.find(&find.query);
        if !matches.is_empty() {
            find.index = find.index.min(matches.len() - 1);
        }
        let current = find.index;
        for (i, hit) in matches.iter().enumerate() {
            let color = if i == current { WARN.gamma_multiply(0.65) } else { Color32::from_rgba_unmultiplied(255, 220, 60, 90) };
            for r in &hit.rects {
                painter.rect_filled(rect_of(rect.left_top(), *r, ratio), 1.0, color);
            }
        }
        if find.jump {
            find.jump = false;
            if let Some(r) = matches.get(current).and_then(|h| h.rects.first()) {
                ui.scroll_to_rect(rect_of(rect.left_top(), *r, ratio).expand(40.0), Some(egui::Align::Center));
            }
        }
        if !ui.ctx().egui_wants_keyboard_input() {
            let (n, shift) = ui.input(|i| (i.key_pressed(egui::Key::N) && !i.modifiers.command, i.modifiers.shift));
            if n {
                step_find(find, matches.len(), !shift);
            }
        }
    }
    state.match_count = matches.len();

    let hit = |p: Pos2| {
        let l = local(p);
        page.links.iter().find(|k| l.x >= k.rect[0] && l.x <= k.rect[0] + k.rect[2] && l.y >= k.rect[1] && l.y <= k.rect[1] + k.rect[3])
    };
    let hovered = resp.hover_pos().and_then(hit);
    if let Some(link) = hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let tip = match &link.mismatch {
            Some((shown, real)) => format!("Text says {shown} but this goes to {real}\n{}", link.href),
            None => link.href.clone(),
        };
        resp.clone().on_hover_text_at_pointer(tip);
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label("message body"));

    let wants_copy = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
    state.selected_text = match state.selection {
        Some((a, b)) if a != b => Some(page.selected_text(a, b)).filter(|t| !t.trim().is_empty()).map(|t| (id, t)),
        _ => None,
    };
    if wants_copy {
        let text = match state.selection {
            Some((a, b)) if a != b || !page.blocks.is_empty() => {
                let t = page.selected_text(a, b);
                if t.is_empty() { page.plain_text() } else { t }
            }
            _ => page.plain_text(),
        };
        ui.ctx().copy_text(text);
    }

    if resp.clicked() {
        state.selection = None;
        return resp.interact_pointer_pos().and_then(hit).map(|l| LinkClick { href: l.href.clone(), mismatch: l.mismatch.clone() });
    }
    None
}

fn paint_tiles(ui: &Ui, renderer: &crate::htmlview::Renderer, page: &std::sync::Arc<crate::htmlview::Rendered>, rect: Rect, ratio: f32) {
    let source = std::sync::Arc::as_ptr(page) as usize;
    let cache_id = egui::Id::new(("html-textures", page.key.id));
    let mut cache = ui
        .ctx()
        .data(|d| d.get_temp::<HtmlTextures>(cache_id))
        .filter(|c| c.source == source)
        .unwrap_or(HtmlTextures { source, textures: vec![None; page.tile_rows.len()] });
    let clip = ui.clip_rect();
    let painter = ui.painter_at(rect);
    let mut visible = Vec::new();
    for (i, (top, h)) in page.tile_rows.iter().enumerate() {
        let y = rect.top() + *top as f32 / page.scale * ratio;
        let r = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), *h as f32 / page.scale * ratio));
        let near = r.expand2(Vec2::new(0.0, clip.height() * 0.5));
        if !near.intersects(clip) {
            continue;
        }
        visible.push(i);
        if cache.textures[i].is_none() {
            if let Some(tile) = renderer.tile(page, i) {
                let image = egui::ColorImage::from_rgba_unmultiplied([tile.width as usize, tile.height as usize], &tile.rgba);
                cache.textures[i] = Some(ui.ctx().load_texture(format!("html-{source}-{i}"), image, egui::TextureOptions::LINEAR));
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
        if let Some(tex) = &cache.textures[i] {
            painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
    }
    if let (Some(&first), Some(&last)) = (visible.first(), visible.last()) {
        for (i, t) in cache.textures.iter_mut().enumerate() {
            if i + 1 < first || i > last + 1 {
                *t = None;
            }
        }
        renderer.keep_tiles(page, first.saturating_sub(1), last + 1);
    }
    ui.ctx().data_mut(|d| d.insert_temp(cache_id, cache));
}

pub fn body_text(ui: &mut Ui, text: &str, zoom: f32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text).font(body(BODY_SIZE * zoom)).line_height(Some(BODY_LINE * zoom)).color(VALUE),
        )
        .wrap(),
    );
}

pub const PLAIN_BODY: &str = "plain-body";

pub fn plain_body(ui: &mut Ui, text: &str, zoom: f32, id: egui::Id) -> Option<String> {
    let mut shown = text;
    let out = egui::TextEdit::multiline(&mut shown)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(body(BODY_SIZE * zoom))
        .text_color(VALUE)
        .desired_width(ui.available_width())
        .desired_rows(1)
        .show(ui);
    let label = format!("message body: {text}");
    ui.ctx().accesskit_node_builder(out.response.id, |node| node.set_label(label));
    let range = out.cursor_range?;
    let [a, b] = range.sorted_cursors();
    let (a, b) = (a.index.0, b.index.0);
    (a != b).then(|| text.chars().skip(a).take(b - a).collect::<String>()).filter(|t| !t.trim().is_empty())
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

pub struct CardHead<'a> {
    pub name: &'a str,
    pub address: &'a str,
    pub to: &'a str,
    pub when: &'a str,
    pub full: &'a str,
    pub unread: bool,
    pub flagged: bool,
    pub mine: bool,
    pub attach: bool,
    pub account: Option<&'a str>,
    pub snippet: Option<&'a str>,
    pub focused: bool,
    pub open: bool,
    pub draft: bool,
}

pub struct CardResp {
    pub toggle: bool,
    pub menu: egui::Response,
    pub rect: Rect,
}

pub const TAP: f32 = 44.0;

pub fn card_height(narrow: bool) -> f32 {
    if narrow { 84.0 } else { 52.0 }
}

fn lay_text(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> std::sync::Arc<egui::Galley> {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, color))
}

pub fn ellipsize(text: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.trim_end_matches("...").chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>().trim_end().to_string() + "\u{2026}";
        if measure(&candidate) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "\u{2026}"
}

pub fn fit_text(ui: &mut Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let fitted = ellipsize(text, max_w, |s| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), color)).size().x);
    lay_text(ui, &fitted, font, color)
}

fn badge(ui: &mut Ui, p: &egui::Painter, at: Pos2, text: &str, tint: Color32) -> f32 {
    let g = lay_text(ui, &text.to_uppercase(), body_strong(9.5), tint);
    let r = Rect::from_min_size(at, g.size() + Vec2::new(10.0, 4.0));
    p.rect_stroke(r, 2.0, Stroke::new(1.0, tint), egui::StrokeKind::Inside);
    p.galley(r.min + Vec2::new(5.0, 2.0), g, tint);
    r.width() + 6.0
}

pub fn message_card_head(ui: &mut Ui, h: &CardHead, narrow: bool) -> CardResp {
    let size = Vec2::new(ui.available_width(), card_height(narrow));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let btn = if narrow { TAP } else { 30.0 };
    let menu_rect = Rect::from_min_size(Pos2::new(rect.right() - btn - 4.0, rect.top() + if narrow { 2.0 } else { 4.0 }), Vec2::splat(btn));
    let menu = ui.interact(menu_rect, resp.id.with("card-menu"), Sense::click());
    let label = format!(
        "{}{}{}, {}, {}",
        h.name,
        if h.mine { " (you)" } else { "" },
        if h.unread { ", unread" } else { "" },
        h.when,
        if h.open || h.focused { "" } else { h.snippet.unwrap_or_default() }
    );
    let label = if h.attach { format!("{label}, has attachment") } else { label };
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    let menu_label = format!("actions for {}", h.name);
    menu.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, menu_label.clone()));
    if resp.hovered() || menu.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered() || menu.hovered();
        let whole = ui.painter().clone();
        whole.rect_filled(rect, 2.0, if h.open || hot { BAND } else { WELL });
        whole.rect_stroke(rect, 2.0, Stroke::new(1.0, if h.focused { READOUT } else { ETCH }), egui::StrokeKind::Inside);
        if h.draft {
            let r = rect.shrink(0.5);
            let outline = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            whole.extend(egui::Shape::dashed_line(&outline, Stroke::new(1.0, WARN), 5.0, 3.0));
        }
        if h.focused {
            whole.rect_filled(Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())), 0.0, READOUT);
        }
        let tint = if menu.hovered() { VALUE } else { LEGEND };
        for dx in [-5.0, 0.0, 5.0] {
            whole.circle_filled(menu_rect.center() + Vec2::new(dx, 0.0), 1.6, tint);
        }
        if h.unread {
            whole.circle_filled(Pos2::new(rect.left() + 13.0, rect.top() + 16.0), 3.5, READOUT);
        }
        let left = rect.left() + 24.0;
        let right = menu_rect.left() - 6.0;
        let p = whole.with_clip_rect(Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(right, rect.bottom())));
        let when = lay_text(ui, h.when, figure(11.0), LEGEND);
        p.galley(Pos2::new(right - when.size().x, rect.top() + 9.0), when.clone(), LEGEND);
        let y1 = rect.top() + 8.0;
        let name = lay_text(ui, h.name, body_strong(13.5), VALUE);
        let mut x = left;
        p.galley(Pos2::new(x, y1), name.clone(), VALUE);
        x += name.size().x + 8.0;
        if !narrow {
            let extra = if h.open { format!("<{}>", h.address) } else { format!("to {}", h.to) };
            let g = lay_text(ui, &extra, body(12.0), LEGEND);
            p.galley(Pos2::new(x, y1 + 1.5), g.clone(), LEGEND);
            x += g.size().x + 8.0;
        }
        if h.draft {
            x += badge(ui, &p, Pos2::new(x, y1 + 1.0), "draft", WARN);
        }
        if h.mine && !h.draft {
            x += badge(ui, &p, Pos2::new(x, y1 + 1.0), "you", READOUT);
        }
        if let Some(a) = h.account {
            x += badge(ui, &p, Pos2::new(x, y1 + 1.0), a, LEGEND);
        }
        if h.flagged {
            x += badge(ui, &p, Pos2::new(x, y1 + 1.0), "flagged", WARN);
        }
        if h.attach {
            clip_glyph(&p, Pos2::new(x + 8.0, y1 + 9.0), LEGEND);
        }
        let to_line = format!("to {}", h.to);
        let snippet = h.snippet.unwrap_or_default();
        let lines: Vec<(&str, Color32)> = match (narrow, h.open) {
            (false, true) => vec![(to_line.as_str(), LEGEND)],
            (false, false) => vec![(snippet, LEGEND)],
            (true, true) => vec![(h.address, LEGEND), (to_line.as_str(), LEGEND)],
            (true, false) => vec![(to_line.as_str(), LEGEND), (snippet, VALUE)],
        };
        for (i, (text, color)) in lines.into_iter().enumerate() {
            let g = fit_text(ui, text, body(12.5), color, (right - left).max(20.0));
            p.galley(Pos2::new(left, rect.top() + 29.0 + i as f32 * 22.0), g, color);
        }
    }
    let resp = resp.on_hover_text(format!("{}\n{}", h.full, h.address));
    CardResp { toggle: resp.clicked() && !menu.clicked(), menu, rect }
}

pub fn tap_key(ui: &mut Ui, narrow: bool, key: Key) -> egui::Response {
    if !narrow {
        return ui.add(key);
    }
    let w = key.width(ui).max(TAP);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, TAP), Sense::hover());
    key.paint(ui, rect)
}

pub fn subject_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(body_strong(18.0)).color(VALUE)).wrap());
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageChoice {
    Once,
    Sender,
    Domain,
}

pub fn blocked_images(ui: &mut Ui, host: &str, sender: &str, domain: &str, policy: crate::prefs::RemoteImages) -> Option<ImageChoice> {
    let mut choice = None;
    rail_block(ui, WARN, WARN.gamma_multiply(0.12), |ui| {
        ui.label(legend("remote images blocked").color(WARN));
        if policy != crate::prefs::RemoteImages::Never {
            ui.horizontal_wrapped(|ui| {
                if policy == crate::prefs::RemoteImages::Ask && ui.add(Key::new("load once").outline(WARN)).clicked() {
                    choice = Some(ImageChoice::Once);
                }
                if ui.add(Key::new("always from sender").outline(WARN)).on_hover_text(format!("Always load images in mail from {sender}")).clicked() {
                    choice = Some(ImageChoice::Sender);
                }
                if ui.add(Key::new("always from domain").outline(WARN)).on_hover_text(format!("Always load images in mail from {domain}")).clicked() {
                    choice = Some(ImageChoice::Domain);
                }
            });
        }
        Line::new().note(format!("This message wants to load pictures from {host}, which tells the sender you opened it.")).size(12.0).wrapped(ui);
    });
    choice
}

pub fn attachment(ui: &mut Ui, name: &str, size: &str) -> egui::Response {
    let g = ui.fonts_mut(|f| f.layout_no_wrap(name.to_string(), body(12.5), VALUE));
    let s = ui.fonts_mut(|f| f.layout_no_wrap(size.to_string(), figure(10.5), LEGEND));
    let w = 26.0 + g.size().x + 12.0 + s.size().x + 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
    let label = format!("{name} {size}");
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
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
        for w in &h.spoof {
            chip(ui, &w.text(), if w.is_alarm() { FAULT } else { LEGEND }, w.is_alarm());
        }
        if let Some(p) = &h.pgp {
            chip(ui, &p.text(), if p.is_alarm() { FAULT } else { LEGEND }, p.is_alarm());
        }
        if let Some((ok, text)) = &h.decrypt_note {
            chip(ui, text, if *ok { OK } else { FAULT }, !*ok);
        }
        if let Some(v) = &h.smime {
            let tint = if v.is_alarm() { FAULT } else { OK };
            if chip_button(ui, &v.text(), tint).clicked() {
                click = HeaderClick::Toggle;
            }
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
        if let Some(v) = &h.smime {
            for (k, val) in v.detail_lines() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(legend(&format!("cert {k}")).color(LEGEND));
                    ui.add(egui::Label::new(egui::RichText::new(val).font(figure(11.0)).color(VALUE)).wrap());
                });
            }
        }
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

fn format_job(text: &str, wrap: f32) -> egui::text::LayoutJob {
    use crate::richtext::Style;
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    let base = |font: FontId, color: Color32| egui::TextFormat { font_id: font, color, line_height: Some(BODY_LINE), ..Default::default() };
    for (range, style) in crate::richtext::spans(text) {
        let fmt = match style {
            Style::Plain => base(body(BODY_SIZE), VALUE),
            Style::Mark => base(body(BODY_SIZE), LEGEND.gamma_multiply(0.8)),
            Style::Bold => base(body_strong(BODY_SIZE), VALUE),
            Style::Italic => base(body_italic(BODY_SIZE), VALUE),
            Style::Code => egui::TextFormat { background: BAND, ..base(egui::FontId::monospace(BODY_SIZE - 1.0), VALUE) },
            Style::Link => egui::TextFormat { underline: Stroke::new(1.0, READOUT), ..base(body(BODY_SIZE), READOUT) },
            Style::Image => egui::TextFormat { background: BAND, ..base(body(BODY_SIZE - 1.0), READOUT) },
            Style::Quote => base(body_italic(BODY_SIZE), LEGEND),
        };
        job.append(&text[range], 0.0, fmt);
    }
    job
}

const FORMATS: [(&str, crate::richtext::Format, Option<(egui::Modifiers, egui::Key)>); 7] = [
    ("bold", crate::richtext::Format::Bold, Some((egui::Modifiers::COMMAND, egui::Key::B))),
    ("italic", crate::richtext::Format::Italic, Some((egui::Modifiers::COMMAND, egui::Key::I))),
    ("link", crate::richtext::Format::Link, Some((egui::Modifiers::COMMAND, egui::Key::K))),
    ("list", crate::richtext::Format::Bullets, None),
    ("numbered", crate::richtext::Format::Numbers, None),
    ("quote", crate::richtext::Format::Quote, None),
    ("code", crate::richtext::Format::Code, None),
];

pub fn body_editor(ui: &mut Ui, text: &mut String, rows: usize) {
    let max_h = (ui.ctx().content_rect().height() - 390.0).clamp(140.0, 520.0);
    let id = egui::Id::new("compose-body");
    let mut wanted: Option<crate::richtext::Format> = None;
    for (_, format, key) in FORMATS {
        if let Some((mods, k)) = key {
            if ui.input_mut(|i| i.consume_key(mods, k)) {
                wanted = Some(format);
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (label, format, _) in FORMATS {
            if ui.add(Key::new(label).outline(VALUE).min_width(44.0)).clicked() {
                wanted = Some(format);
            }
        }
    });
    if let Some(format) = wanted {
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let (a, b) = state.cursor.char_range().map(|r| (r.primary.index.into(), r.secondary.index.into())).unwrap_or_else(|| {
            let n = text.chars().count();
            (n, n)
        });
        let (out, (s, e)) = crate::richtext::apply(text, (a, b), format);
        *text = out;
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(s), egui::text::CCursor::new(e))));
        state.store(ui.ctx(), id);
        ui.memory_mut(|m| m.request_focus(id));
    }
    egui::Frame::NONE
        .fill(WELL)
        .stroke(Stroke::new(1.0, ETCH))
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).stick_to_bottom(false).show(ui, |ui| {
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let job = format_job(buf.as_str(), wrap);
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let r = ui.add(
                    egui::TextEdit::multiline(text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .font(body(BODY_SIZE))
                        .text_color(VALUE)
                        .desired_rows(rows)
                        .desired_width(ui.available_width())
                        .layouter(&mut layouter),
                );
                ui.ctx().accesskit_node_builder(r.id, |node| node.set_label("message body"));
            });
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
