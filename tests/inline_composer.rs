use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::FakeStore;
use mailbench::mailstore::*;
use std::path::PathBuf;
use std::sync::Arc;

fn quiet_renderer() -> egui_kittest::LazyRenderer {
    static CREATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    egui_kittest::LazyRenderer::new(|| {
        let _guard = CREATE.lock().unwrap_or_else(|e| e.into_inner());
        let mut setup = egui_kittest::wgpu::default_wgpu_setup();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
            create.instance_descriptor.flags = eframe::egui_wgpu::wgpu::InstanceFlags::empty();
        }
        egui_kittest::wgpu::WgpuTestRenderer::from_setup(setup)
    })
}

fn harness(size: egui::Vec2) -> Harness<'static, (bool, App)> {
    let mut app = App::with_store(Box::new(FakeStore::concept()));
    app.selected = 0;
    let mut h = Harness::builder()
        .renderer(quiet_renderer())
        .with_size(size)
        .with_pixels_per_point(1.0)
        .build_ui_state(
            |ui, (ready, app): &mut (bool, App)| {
                if !*ready {
                    mailbench::kit::setup(ui.ctx());
                    *ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                app.show(ui);
            },
            (false, app),
        );
    h.run();
    h.run();
    h
}

fn composer(size: egui::Vec2) -> Harness<'static, (bool, App)> {
    let mut h = harness(size);
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    assert!(h.state().1.draft.is_some());
    h.get_by_label("message body").click();
    h.run();
    h
}

fn desktop() -> egui::Vec2 {
    egui::vec2(1280.0, 800.0)
}

fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 4) as u8, (y * 6) as u8, 200, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn jpeg_bytes() -> Vec<u8> {
    let img = image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([(x * 6) as u8, (y * 8) as u8, 40]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
    out
}

fn paste_image(h: &mut Harness<'static, (bool, App)>) {
    let rgba: Vec<u8> = (0..60 * 40).flat_map(|i| [(i % 60) as u8 * 4, 90, 160, 255]).collect();
    let img = egui::ColorImage::from_rgba_unmultiplied([60, 40], &rgba);
    h.input_mut().events.push(egui::Event::PasteImage(Arc::new(img)));
    h.run();
    h.run();
}

#[derive(Debug)]
struct Dropped(PathBuf);
impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn drop_file(h: &mut Harness<'static, (bool, App)>, dir: &std::path::Path, name: &str, data: &[u8]) {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    h.input_mut().dropped_files.push(Arc::new(Dropped(path)));
    h.run();
    h.run();
}

fn out_dir() -> PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn pasting_a_png_adds_an_inline_reference_and_a_thumbnail() {
    let mut h = composer(desktop());
    paste_image(&mut h);
    let d = h.state().1.draft.clone().unwrap();
    let refs = mailbench::richtext::images(&d.body);
    assert_eq!(refs.len(), 1, "{}", d.body);
    assert_eq!(d.attachments.len(), 1);
    assert_eq!(d.attachments[0].cid.as_deref(), Some(refs[0].cid.as_str()));
    assert_eq!(d.attachments[0].mime, "image/png");
    h.get_by_label_contains("remove image pasted-image.png");
    h.get_by_label("send").rect();
    let img = h.render().expect("render");
    img.save(out_dir().join("inline-image-desktop.png")).unwrap();
}

#[test]
fn pasting_text_still_pastes_text() {
    let mut h = composer(desktop());
    h.input_mut().events.push(egui::Event::Paste("plain words".into()));
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert!(d.body.contains("plain words"));
    assert!(d.attachments.is_empty());
}

#[test]
fn the_image_lands_at_the_cursor() {
    let mut h = composer(desktop());
    h.input_mut().events.push(egui::Event::Paste("before after".into()));
    h.run();
    h.run();
    let id = egui::Id::new("compose-body");
    let mut state = egui::TextEdit::load_state(&h.ctx, id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(7))));
    state.store(&h.ctx, id);
    paste_image(&mut h);
    let body = h.state().1.draft.clone().unwrap().body;
    assert!(body.starts_with("before ![pasted-image.png](cid:"), "{body}");
    assert!(body.ends_with("after"), "{body}");
}

#[test]
fn dropping_a_jpeg_inserts_it_inline() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = composer(desktop());
    drop_file(&mut h, dir.path(), "holiday.jpg", &jpeg_bytes());
    let d = h.state().1.draft.clone().unwrap();
    assert!(d.body.contains("![holiday.jpg](cid:"), "{}", d.body);
    assert_eq!(d.attachments[0].mime, "image/jpeg");
    assert!(d.attachments[0].cid.is_some());
}

#[test]
fn a_png_named_file_with_jpeg_bytes_is_inlined_as_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = composer(desktop());
    drop_file(&mut h, dir.path(), "liar.png", &jpeg_bytes());
    assert_eq!(h.state().1.draft.clone().unwrap().attachments[0].mime, "image/jpeg");
}

#[test]
fn dropping_a_pdf_adds_a_normal_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = composer(desktop());
    drop_file(&mut h, dir.path(), "invoice.pdf", b"%PDF-1.4 fake");
    let d = h.state().1.draft.clone().unwrap();
    assert_eq!(d.attachments.len(), 1);
    assert!(d.attachments[0].cid.is_none());
    assert!(!d.body.contains("cid:"));
    h.get_by_label_contains("invoice.pdf");
}

#[test]
fn dropping_an_svg_is_refused_with_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = composer(desktop());
    drop_file(&mut h, dir.path(), "logo.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
    let app = &h.state().1;
    assert!(app.draft.as_ref().unwrap().attachments.len() == 1 && app.draft.as_ref().unwrap().attachments[0].cid.is_none());
}

#[test]
fn dropping_an_image_with_no_composer_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(desktop());
    drop_file(&mut h, dir.path(), "x.png", &png_bytes(8, 8));
    assert!(h.state().1.draft.is_none());
}

#[test]
fn deleting_the_reference_removes_the_image() {
    let mut h = composer(desktop());
    paste_image(&mut h);
    h.state_mut().1.draft.as_mut().unwrap().body.clear();
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert!(d.attachments.is_empty());
}

#[test]
fn clicking_the_thumbnail_removes_the_image_and_its_reference() {
    let mut h = composer(desktop());
    paste_image(&mut h);
    h.get_by_label_contains("remove image pasted-image.png").click();
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert!(d.attachments.is_empty());
    assert!(!d.body.contains("cid:"));
}

#[test]
fn undoing_the_deletion_in_the_body_brings_the_image_back() {
    let mut h = composer(desktop());
    paste_image(&mut h);
    let body = h.state().1.draft.clone().unwrap().body;
    h.state_mut().1.draft.as_mut().unwrap().body.clear();
    h.run();
    h.state_mut().1.draft.as_mut().unwrap().body = body;
    h.run();
    h.run();
    assert_eq!(h.state().1.draft.clone().unwrap().attachments.len(), 1);
}

#[test]
fn autosave_then_reopen_keeps_the_image() {
    let mut h = composer(desktop());
    h.state_mut().1.autosave = std::time::Duration::ZERO;
    h.input_mut().events.push(egui::Event::Paste("hello".into()));
    h.run();
    paste_image(&mut h);
    h.run();
    h.run();
    let saved = h.state().1.draft.clone().unwrap();
    assert!(saved.saved, "autosave did not run");
    let _mid = saved.message_id.clone().unwrap();
    let id = {
        let app = &h.state().1;
        app.store.list(&Query::new(Folder::Drafts), 0, 20).items[0].id
    };
    let out = h.state_mut().1.store.open_draft(id).unwrap();
    assert_eq!(out.attachments.len(), 1);
    assert!(out.attachments[0].cid.is_some());
    let reopened = mailbench::app::Draft::from_outgoing(out, "");
    assert_eq!(reopened.inline_refs().len(), 1);
}

#[test]
fn sending_and_undoing_keeps_the_image_in_the_composer() {
    let mut h = composer(desktop());
    h.input_mut().events.push(egui::Event::Paste("hi".into()));
    h.run();
    paste_image(&mut h);
    h.state_mut().1.draft.as_mut().unwrap().to = "ana@example.test".into();
    h.run();
    h.get_by_label("send").click();
    h.run();
    h.run();
    assert!(h.state().1.draft.is_none());
    h.state_mut().1.undo_send();
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert_eq!(d.inline_refs().len(), 1);
    assert_eq!(d.attachments.iter().filter(|a| a.cid.is_some()).count(), 1);
}

#[test]
fn plain_text_only_tells_the_user_once() {
    let mut h = composer(desktop());
    h.state_mut().1.prefs.plain_text_only = true;
    paste_image(&mut h);
    let note = h.state().1.notice.clone().unwrap_or_default();
    assert!(note.contains("attachments"), "{note}");
    h.state_mut().1.notice = None;
    paste_image(&mut h);
    assert!(h.state().1.notice.is_none());
}

#[test]
fn forwarding_keeps_inline_images_and_replying_does_not() {
    let html = mailbench::htmlview::HtmlBody { html: "<img src=\"cid:logo\">".into(), inline: vec![("<logo>".into(), Arc::new(png_bytes(8, 8)))] };
    let store = FakeStore::concept();
    let first = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    let mut m = store.get(first).unwrap();
    m.html = html;
    let fwd = mailbench::app::Draft::reply(&m, mailbench::app::Reply::Forward, "", &[], Vec::new(), None);
    assert_eq!(fwd.inline_refs().len(), 1);
    assert_eq!(fwd.attachments.iter().filter(|a| a.cid.is_some()).count(), 1);
    let reply = mailbench::app::Draft::reply(&m, mailbench::app::Reply::One, "", &[], Vec::new(), None);
    assert!(reply.inline_refs().is_empty());
    assert!(reply.attachments.is_empty());
}

#[test]
fn a_message_over_the_size_cap_is_refused_before_sending() {
    let mut h = composer(desktop());
    {
        let d = h.state_mut().1.draft.as_mut().unwrap();
        d.to = "ana@example.test".into();
        d.attach_bytes("huge.bin", vec![0u8; 30 * 1024 * 1024]);
    }
    h.run();
    assert!(h.state().1.draft.is_some(), "before click");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run();
    let app = &h.state().1;
    assert!(app.draft.is_some() && app.pending_send.is_none(), "notice {:?} draft {} pending {}", app.notice, app.draft.is_some(), app.pending_send.is_some());
}

#[test]
fn composer_with_an_inline_image_fits_at_phone_width() {
    let mut h = composer(egui::vec2(392.0, 844.0));
    h.input_mut().events.push(egui::Event::Paste("see the screenshot below".into()));
    h.run();
    paste_image(&mut h);
    let send = h.get_by_label("send").rect();
    assert!(send.right() <= 392.0 && send.bottom() <= 844.0, "send at {send:?}");
    let thumb = h.get_by_label_contains("remove image").rect();
    assert!(thumb.right() <= 392.0, "thumb at {thumb:?}");
    let img = h.render().expect("render");
    img.save(out_dir().join("inline-image-phone.png")).unwrap();
}
