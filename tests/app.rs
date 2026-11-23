use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::FakeStore;
use mailbench::mailstore::*;
use mailbench::search::Parsed;
use std::path::PathBuf;

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

fn harness(store: FakeStore) -> Harness<'static, (bool, App)> {
    harness_sized(store, egui::vec2(1280.0, 800.0))
}

fn harness_sized(store: FakeStore, size: egui::Vec2) -> Harness<'static, (bool, App)> {
    let mut app = App::with_store(Box::new(store));
    app.selected = 0;
    let mut h = Harness::builder().renderer(quiet_renderer())
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

#[test]
fn phone_width_shows_one_pane_at_a_time() {
    let mut h = harness_sized(FakeStore::concept(), egui::vec2(390.0, 844.0));
    assert!(h.state().1.narrow);
    h.get_by_label_contains("Tomás Rivera Rev C stencil quote");
    assert!(!h.state().1.folders_open);
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert!(h.state().1.reader_focus);
    h.get_by_label_contains("back to list");
    let img = h.render().expect("render");
    img.save(out_dir().join("phone-reader.png")).unwrap();
    h.get_by_label_contains("back to list").click();
    h.run();
    h.run();
    assert!(!h.state().1.reader_focus);
    h.get_by_label_contains("folders").click();
    h.run();
    h.run();
    assert!(h.state().1.folders_open);
    let img = h.render().expect("render");
    img.save(out_dir().join("phone-folders.png")).unwrap();
}

#[test]
fn tapping_a_row_once_opens_the_reader_at_phone_width() {
    let mut h = harness_sized(FakeStore::concept(), egui::vec2(390.0, 844.0));
    assert!(!h.state().1.reader_focus);
    h.get_by_label_contains("Tomás Rivera Rev C stencil quote").click();
    h.run();
    h.run();
    assert!(h.state().1.reader_focus);
    h.get_by_label_contains("stencil-quote-revc.pdf");
    h.get_by_label_contains("back to list").click();
    h.run();
    h.run();
    assert!(!h.state().1.reader_focus);
    h.get_by_label_contains("Tomás Rivera Rev C stencil quote");
}

#[test]
fn picking_a_folder_at_phone_width_closes_the_folder_view() {
    let mut h = harness_sized(FakeStore::concept(), egui::vec2(390.0, 844.0));
    h.get_by_label_contains("folders").click();
    h.run();
    h.run();
    assert!(h.state().1.folders_open);
    h.get_by_label("folder sent").click();
    h.run();
    h.run();
    assert!(!h.state().1.folders_open);
    assert_eq!(h.state().1.folder, Folder::Sent);
}

#[test]
fn composer_fits_at_phone_width() {
    let mut h = harness_sized(FakeStore::concept(), egui::vec2(390.0, 844.0));
    h.get_by_label_contains("Tomás Rivera Rev C stencil quote").click();
    h.run();
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    let send = h.get_by_label("send").rect();
    let discard = h.get_by_label("discard").rect();
    assert!(send.right() <= 390.0 && send.bottom() <= 844.0, "send at {send:?}");
    assert!(discard.right() <= 390.0 && discard.bottom() <= 844.0, "discard at {discard:?}");
    let img = h.render().expect("render");
    img.save(out_dir().join("phone-composer.png")).unwrap();
}

fn open_blank_composer(h: &mut Harness<'static, (bool, App)>) {
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    assert!(h.state().1.draft.is_some());
}

fn focus_body(h: &mut Harness<'static, (bool, App)>) {
    h.get_by_label("message body").click();
    h.run();
}

#[test]
fn enter_in_the_body_inserts_a_newline() {
    let mut h = harness(FakeStore::concept());
    open_blank_composer(&mut h);
    focus_body(&mut h);
    h.get_by_label("message body").type_text("a");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.get_by_label("message body").type_text("b");
    h.run();
    assert_eq!(h.state().1.draft.as_ref().unwrap().body, "a\nb");
}

#[test]
fn ctrl_enter_sends_and_does_not_insert_a_newline() {
    let mut h = harness(FakeStore::concept());
    open_blank_composer(&mut h);
    h.state_mut().1.draft.as_mut().unwrap().to = "bob <bob@example.test>".into();
    focus_body(&mut h);
    h.get_by_label("message body").type_text("hello");
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run();
    h.run();
    let app = &h.state().1;
    assert!(app.draft.is_none());
    assert_eq!(app.store.list(&Query::new(Folder::Sent), 0, 10).items.len(), 1);
}

#[test]
fn ctrl_enter_without_a_recipient_does_not_insert_a_newline() {
    let mut h = harness(FakeStore::concept());
    open_blank_composer(&mut h);
    focus_body(&mut h);
    h.get_by_label("message body").type_text("hello");
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run();
    h.run();
    assert_eq!(h.state().1.draft.as_ref().unwrap().body, "hello");
}

#[test]
fn tab_in_the_body_does_not_insert_a_tab() {
    let mut h = harness(FakeStore::concept());
    open_blank_composer(&mut h);
    focus_body(&mut h);
    h.get_by_label("message body").type_text("x");
    h.run();
    h.key_press(egui::Key::Tab);
    h.run();
    h.run();
    assert_eq!(h.state().1.draft.as_ref().unwrap().body, "x");
}

fn inbox_ids(app: &App) -> Vec<MessageId> {
    app.store.list(&Query::new(Folder::Inbox), 0, 100_000).items.iter().map(|s| s.id).collect()
}

#[test]
fn list_shows_rows_and_counts() {
    let h = harness(FakeStore::concept());
    let app = &h.state().1;
    assert_eq!(app.store.counts(&Query::new(Folder::Inbox)).all, 30);
    assert!(app.rendered_rows >= 20 && app.rendered_rows <= 30, "{}", app.rendered_rows);
    h.get_by_label_contains("Tomás Rivera Rev C stencil quote");
}

#[test]
fn ten_thousand_messages_paint_only_the_viewport() {
    let h = harness(FakeStore::bulk(10_000));
    let app = &h.state().1;
    assert_eq!(app.store.counts(&Query::new(Folder::Inbox)).all, 10_000);
    assert!(app.rendered_rows > 10 && app.rendered_rows <= 40, "{}", app.rendered_rows);
}

#[test]
fn j_and_k_move_the_selection() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::J);
    h.run();
    h.key_press(egui::Key::J);
    h.run();
    assert_eq!(h.state().1.selected, 2);
    h.key_press(egui::Key::K);
    h.run();
    assert_eq!(h.state().1.selected, 1);
}

#[test]
fn e_archives_the_selected_message() {
    let mut h = harness(FakeStore::concept());
    let first = inbox_ids(&h.state().1)[0];
    h.key_press(egui::Key::E);
    h.run();
    let app = &h.state().1;
    assert!(!inbox_ids(app).contains(&first));
    assert_eq!(app.store.counts(&Query::new(Folder::Inbox)).all, 29);
    let archived = app.store.list(&Query::new(Folder::Archive), 0, 10);
    assert_eq!(archived.items.len(), 1);
    assert_eq!(archived.items[0].id, first);
}

#[test]
fn x_toggles_the_flag() {
    let mut h = harness(FakeStore::concept());
    let id = inbox_ids(&h.state().1)[2];
    h.key_press(egui::Key::J);
    h.run();
    h.key_press(egui::Key::J);
    h.run();
    assert!(!h.state().1.store.get(id).unwrap().summary.flagged);
    h.key_press(egui::Key::X);
    h.run();
    assert!(h.state().1.store.get(id).unwrap().summary.flagged);
    h.key_press(egui::Key::X);
    h.run();
    assert!(!h.state().1.store.get(id).unwrap().summary.flagged);
}

#[test]
fn remote_images_are_blocked_until_load_once() {
    let mut h = harness(FakeStore::concept());
    h.get_by_label_contains("REMOTE IMAGES BLOCKED");
    h.get_by_label("load once").click();
    h.run();
    h.run();
    assert!(h.query_by_label_contains("REMOTE IMAGES BLOCKED").is_none());
    assert_eq!(h.state().1.images_loaded.len(), 1);
    h.key_press(egui::Key::J);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    assert!(h.query_by_label_contains("REMOTE IMAGES BLOCKED").is_none());
}

#[test]
fn c_opens_the_composer_and_reply_prefills() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    assert!(h.state().1.draft.as_ref().unwrap().to.is_empty());
    h.get_by_label("discard").click();
    h.run();
    h.run();
    assert!(h.state().1.draft.is_none());
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert_eq!(d.to, "Tomás Rivera <tomás.rivera@example.test>");
    assert_eq!(d.subject, "Re: Rev C stencil quote and lead time");
    assert!(d.body.contains("wrote:\n> "), "{}", d.body);
}

#[test]
fn sending_a_reply_files_it_in_sent() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    h.get_by_label("send").click();
    h.run();
    h.run();
    let app = &h.state().1;
    assert!(app.draft.is_none());
    assert_eq!(app.store.list(&Query::new(Folder::Sent), 0, 10).items.len(), 1);
}

#[test]
fn slash_focuses_search_and_filters() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::Slash);
    h.run();
    h.run();
    h.get_by_role(egui::accesskit::Role::TextInput).type_text("partsbin");
    h.run();
    h.run();
    let app = &h.state().1;
    assert_eq!(app.search, "partsbin");
    assert_eq!(app.store.counts(&app.query()).all, 3);
    h.key_press(egui::Key::Escape);
    h.run();
    h.run();
    assert_eq!(h.state().1.search, "");
}

#[test]
fn go_archive_chord_and_unread_filter() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::G);
    h.run();
    h.key_press(egui::Key::A);
    h.run();
    h.run();
    assert_eq!(h.state().1.folder, Folder::Archive);
    h.get_by_label_contains("Nothing in archive");
}

fn out_dir() -> PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn spoof_headers() -> Headers {
    use mailbench::spoof::SpoofWarning::*;
    Headers {
        spoof: vec![
            NameHasOtherAddress("support@paypal.com".into()),
            Lookalike { domain: "xn--ypal-43d9g.com".into(), resembles: "paypal.com".into() },
            FirstTimeSender,
        ],
        ..Default::default()
    }
}

fn spoof_reader(size: egui::Vec2, name: &str) {
    let mut store = FakeStore::concept();
    store.set_headers(0, spoof_headers());
    let mut h = harness_sized(store, size);
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    let img = h.render().expect("render");
    img.save(out_dir().join(name)).unwrap();
}

fn crypt_headers() -> Headers {
    use mailbench::crypt::{SignerCert, SmimeState, SmimeVerdict};
    Headers {
        decrypt_note: Some((false, "could not decrypt: no imported key can open this message".into())),
        smime: Some(SmimeVerdict {
            state: SmimeState::Untrusted("unable to get local issuer certificate".into()),
            signer: Some(SignerCert {
                subject: "CN=Ada Lovelace".into(),
                issuer: "CN=Example Root CA".into(),
                emails: vec!["ada@example.com".into()],
                fingerprint: "ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12".into(),
                not_before: "Jan  1 00:00:00 2026 GMT".into(),
                not_after: "Jan  1 00:00:00 2027 GMT".into(),
                expired: false,
            }),
            content: None,
        }),
        ..Default::default()
    }
}

fn crypt_reader(size: egui::Vec2, name: &str) {
    let mut store = FakeStore::concept();
    store.set_headers(0, crypt_headers());
    let mut h = harness_sized(store, size);
    h.key_press(egui::Key::Enter);
    h.run();
    h.get_by_label_contains("certificate not trusted").click();
    h.run();
    h.run();
    let img = h.render().expect("render");
    img.save(out_dir().join(name)).unwrap();
}

fn pgp_reader(size: egui::Vec2, name: &str, state: mailbench::openpgp::PgpState) {
    let mut store = FakeStore::concept();
    store.set_headers(
        0,
        Headers {
            pgp: Some(mailbench::openpgp::PgpStatus {
                state,
                signer: Some("Tomas Reyes <tomas@kestrel-instruments.example>".into()),
                fingerprint: "3B2A9C41D7E05F6A8812C4D9E7F10B35A6C2D80F".into(),
            }),
            ..Default::default()
        },
    );
    let mut h = harness_sized(store, size);
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    let img = h.render().expect("render");
    img.save(out_dir().join(name)).unwrap();
}

#[test]
fn crypt_chips_and_certificate_render_on_desktop() {
    crypt_reader(egui::vec2(1280.0, 800.0), "crypt-reader.png");
}

#[test]
fn crypt_chips_and_certificate_render_on_phone() {
    crypt_reader(egui::vec2(390.0, 844.0), "phone-crypt-reader.png");
}

#[test]
fn pgp_chips_render_on_desktop_and_phone() {
    use mailbench::openpgp::PgpState::*;
    for (state, label) in [(Good, "good"), (Bad, "bad"), (UnknownKey, "unknown")] {
        pgp_reader(egui::vec2(1280.0, 800.0), &format!("pgp-{label}.png"), state.clone());
        pgp_reader(egui::vec2(390.0, 844.0), &format!("phone-pgp-{label}.png"), state);
    }
}

#[test]
fn spoof_chips_render_on_desktop() {
    spoof_reader(egui::vec2(1280.0, 800.0), "spoof-reader.png");
}

#[test]
fn spoof_chips_render_on_phone() {
    spoof_reader(egui::vec2(390.0, 844.0), "phone-spoof-reader.png");
}

fn save(h: &mut Harness<'static, (bool, App)>, name: &str) {
    h.run();
    let img = h.render().expect("render");
    assert_eq!(img.width(), 1280);
    img.save(out_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
fn render_every_screen() {
    let mut h = harness(FakeStore::concept());
    save(&mut h, "inbox");

    h.key_press(egui::Key::Enter);
    h.run();
    h.get_by_label_contains("quoted lines hidden").click();
    h.run();
    h.run();
    assert!(h.state().1.quotes_open && h.state().1.reader_focus);
    h.get_by_label_contains("quoted lines shown").click();
    h.run();
    h.run();
    assert!(!h.state().1.quotes_open);
    save(&mut h, "reader");
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    {
        let d = h.state_mut().1.draft.as_mut().unwrap();
        d.cc = "Wren Okafor".into();
        d.body = "Thanks Tomás. Leave the panel edge unpopulated, Wren wants the test coupon there.\n\nI will pay the invoice this afternoon so it ships next Friday. Please send the tracking number when it goes out.\n\nMara".into();
        d.saved = true;
    }
    save(&mut h, "composer");

    let mut h = harness(FakeStore::empty());
    save(&mut h, "empty");

    let mut h = harness(FakeStore::failed());
    save(&mut h, "error");
}

#[test]
fn copy_has_no_em_or_en_dashes() {
    for f in [
        "src/app.rs",
        "src/data.rs",
        "src/fake.rs",
        "src/kit.rs",
        "src/mailstore.rs",
        "docs/design.md",
    ] {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(f)).unwrap();
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{f}");
    }
}

fn unread_row(app: &App) -> (usize, MessageId, String) {
    let items = app.store.list(&Query::new(Folder::Inbox), 0, 100).items;
    let i = items.iter().enumerate().skip(1).find(|(_, s)| s.unread && s.thread == 1).map(|(i, _)| i).unwrap();
    (i, items[i].id, items[i].subject.clone())
}

fn is_unread(app: &App, id: MessageId) -> bool {
    app.store.get(id).unwrap().summary.unread
}

#[test]
fn clicking_a_row_marks_it_read() {
    let mut h = harness(FakeStore::concept());
    let (i, id, subject) = unread_row(&h.state().1);
    assert!(is_unread(&h.state().1, id));
    h.get_by_label_contains(&subject).click();
    h.run();
    h.run();
    assert_eq!(h.state().1.selected, i);
    assert!(!is_unread(&h.state().1, id));
}

#[test]
fn moving_past_a_message_quickly_does_not_mark_it_read() {
    let mut h = harness(FakeStore::concept());
    let (i, id, _) = unread_row(&h.state().1);
    h.state_mut().1.selected = i - 1;
    h.run();
    h.key_press(egui::Key::J);
    h.run();
    h.key_press(egui::Key::J);
    h.run();
    assert!(is_unread(&h.state().1, id));
}

#[test]
fn resting_on_a_message_marks_it_read_after_the_dwell() {
    let mut h = harness(FakeStore::concept());
    let (i, id, _) = unread_row(&h.state().1);
    h.state_mut().1.selected = i - 1;
    h.run();
    h.key_press(egui::Key::J);
    h.run();
    assert!(is_unread(&h.state().1, id));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    h.run();
    assert!(!is_unread(&h.state().1, id));
}

#[test]
fn u_keeps_a_shown_message_unread() {
    let mut h = harness(FakeStore::concept());
    let (_, id, subject) = unread_row(&h.state().1);
    h.get_by_label_contains(&subject).click();
    h.run();
    h.run();
    assert!(!is_unread(&h.state().1, id));
    h.key_press(egui::Key::U);
    h.run();
    assert!(is_unread(&h.state().1, id));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    h.run();
    h.run();
    assert!(is_unread(&h.state().1, id));
}

#[test]
fn mark_read_never_keeps_a_clicked_message_unread() {
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.prefs.reading.mark_read = mailbench::prefs::MarkRead::Never;
    let (_, id, subject) = unread_row(&h.state().1);
    h.get_by_label_contains(&subject).click();
    h.run();
    h.run();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    h.run();
    assert!(is_unread(&h.state().1, id));
}

#[test]
fn a_three_message_thread_opens_collapsed_and_expands_an_older_message() {
    let mut h = harness(FakeStore::concept());
    let head = inbox_ids(&h.state().1)[0];
    assert_eq!(h.state().1.store.thread(head).len(), 3);
    assert_eq!(h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].thread, 3);
    h.get_by_label_contains("Quote attached");
    assert!(h.query_by_label_contains("Hi Mara, the Rev C Gerbers passed").is_none());
    let older = h.state().1.store.thread(head)[0].summary.id;
    h.get_by_label_contains("Wed 15 Nov 16:20").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.expanded, Some(older));
    h.get_by_label_contains("Hi Mara, the Rev C Gerbers passed");
}

#[test]
fn archiving_from_the_list_takes_the_whole_thread_and_the_reader_takes_one() {
    let mut h = harness(FakeStore::concept());
    let head = inbox_ids(&h.state().1)[0];
    let ids: Vec<_> = h.state().1.store.thread(head).iter().map(|t| t.summary.id).collect();
    h.key_press(egui::Key::E);
    h.run();
    for id in &ids {
        assert_eq!(h.state().1.store.get(*id).unwrap().folder, Folder::Archive);
    }
}

#[test]
fn enter_expands_the_focused_older_message_in_the_reader() {
    let mut h = harness(FakeStore::concept());
    let head = inbox_ids(&h.state().1)[0];
    let thread = h.state().1.store.thread(head);
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(h.state().1.reader_focus);
    h.key_press(egui::Key::K);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    assert_eq!(h.state().1.thread_focus, Some(thread[0].summary.id));
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert_eq!(h.state().1.expanded, Some(thread[0].summary.id));
    h.get_by_label_contains("Hi Mara, the Rev C Gerbers passed");
}

fn picker_to(path: PathBuf) -> mailbench::app::SavePicker {
    Box::new(move |_, _| Some(path.clone()))
}

#[test]
fn save_as_writes_the_attachment_bytes_to_the_chosen_path() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.pdf");
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.save_picker = picker_to(target.clone());
    h.get_by_label_contains("stencil-quote-revc.pdf").click();
    h.run();
    h.get_by_label("save as").click();
    h.run();
    assert_eq!(std::fs::read(&target).unwrap(), b"fake content of stencil-quote-revc.pdf");
}

const TINY_PDF: &str = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]/Contents 4 0 R/Resources<<>>>>endobj\n4 0 obj<</Length 30>>stream\n1 0 0 rg 0 0 100 100 re f\nendstream\nendobj\ntrailer<</Root 1 0 R/Size 5>>\n";

#[test]
fn a_pdf_attachment_shows_a_preview_image_in_the_reader() {
    let mut h = harness(FakeStore::concept());
    assert!(h.query_by_role(egui::accesskit::Role::Image).is_none());
    let mut h = harness(FakeStore::concept().with_attachment_bytes("stencil-quote-revc.pdf", TINY_PDF.as_bytes().to_vec()));
    h.run();
    assert!(h.query_by_role(egui::accesskit::Role::Image).is_some());
}

#[test]
fn shift_s_saves_every_attachment_as_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("all.zip");
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.save_picker = picker_to(target.clone());
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::S);
    h.run();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&target).unwrap()).unwrap();
    assert_eq!(zip.len(), 2);
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("revc-paste-layer.zip").unwrap(), &mut text).unwrap();
    assert_eq!(text, "fake content of revc-paste-layer.zip");
}

#[test]
fn o_writes_a_private_copy_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let open_dir = dir.path().join("open");
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut h = harness(FakeStore::concept());
    {
        let app = &mut h.state_mut().1;
        app.open_dir = open_dir.clone();
        let sink = opened.clone();
        app.opener = Box::new(move |p| {
            sink.lock().unwrap().push(p.to_path_buf());
            true
        });
    }
    h.key_press(egui::Key::O);
    h.run();
    let paths = opened.lock().unwrap().clone();
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"fake content of stencil-quote-revc.pdf");
    assert!(h.state().1.notice.as_ref().unwrap().contains("plain copy"));
}

#[test]
fn o_and_shift_s_act_on_the_focused_older_message_while_the_newest_is_shown() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("older.zip");
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = FakeStore::concept();
    let head = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    let store = store.without_attachments(head).with_older_attachment(head, 0, "older-notes.pdf");
    let mut h = harness(store);
    let older = h.state().1.store.thread(head)[0].summary.id;
    {
        let app = &mut h.state_mut().1;
        app.open_dir = dir.path().join("open");
        app.save_picker = picker_to(target.clone());
        let sink = opened.clone();
        app.opener = Box::new(move |p| {
            sink.lock().unwrap().push(p.to_path_buf());
            true
        });
    }
    h.key_press(egui::Key::O);
    h.run();
    assert!(opened.lock().unwrap().is_empty());
    h.key_press(egui::Key::Enter);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    assert_eq!(h.state().1.thread_focus, Some(older));
    assert_eq!(h.state().1.expanded, None);
    h.key_press(egui::Key::O);
    h.run();
    let paths = opened.lock().unwrap().clone();
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"fake content of older-notes.pdf");
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::S);
    h.run();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&target).unwrap()).unwrap();
    assert_eq!(zip.len(), 1);
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("older-notes.pdf").unwrap(), &mut text).unwrap();
    assert_eq!(text, "fake content of older-notes.pdf");
}

#[test]
fn an_expanded_older_message_shows_its_own_chip_and_preview() {
    let store = FakeStore::concept();
    let head = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    let store = store
        .with_older_attachment(head, 0, "older-scan.pdf")
        .with_attachment_bytes("older-scan.pdf", TINY_PDF.as_bytes().to_vec());
    let mut h = harness(store);
    assert!(h.query_by_label_contains("older-scan.pdf").is_none());
    h.get_by_label_contains("Wed 15 Nov 16:20").click();
    h.run();
    h.run();
    h.get_by_label_contains("older-scan.pdf");
    assert!(h.query_by_label_contains("stencil-quote-revc.pdf").is_none());
    assert!(h.query_by_role(egui::accesskit::Role::Image).is_some());
}

#[test]
fn o_and_save_as_act_on_the_expanded_older_message() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("older.pdf");
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = FakeStore::concept();
    let head = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    let store = store.with_older_attachment(head, 0, "older-notes.pdf");
    let mut h = harness(store);
    let older = h.state().1.store.thread(head)[0].summary.id;
    {
        let app = &mut h.state_mut().1;
        app.open_dir = dir.path().join("open");
        app.save_picker = picker_to(target.clone());
        let sink = opened.clone();
        app.opener = Box::new(move |p| {
            sink.lock().unwrap().push(p.to_path_buf());
            true
        });
    }
    h.get_by_label_contains("Wed 15 Nov 16:20").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.expanded, Some(older));
    h.get_by_label_contains("older-notes.pdf");
    h.key_press(egui::Key::Enter);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    assert_eq!(h.state().1.thread_focus, Some(older));
    h.key_press(egui::Key::O);
    h.run();
    let paths = opened.lock().unwrap().clone();
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"fake content of older-notes.pdf");
    h.get_by_label_contains("older-notes.pdf").click();
    h.run();
    h.get_by_label("save as").click();
    h.run();
    assert_eq!(std::fs::read(&target).unwrap(), b"fake content of older-notes.pdf");
}

#[test]
fn open_disabled_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let open_dir = dir.path().join("open");
    let mut h = harness(FakeStore::concept());
    {
        let app = &mut h.state_mut().1;
        app.open_dir = open_dir.clone();
        app.allow_open = false;
    }
    h.key_press(egui::Key::O);
    h.run();
    assert!(!open_dir.exists());
    assert!(h.state().1.notice.as_ref().unwrap().contains("turned off"));
}

#[test]
fn reply_quotes_the_original_with_an_attribution_line() {
    let h = harness(FakeStore::concept());
    let m = h.state().1.store.get(h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id).unwrap();
    let d = mailbench::app::Draft::reply(&m, mailbench::app::Reply::One, "", &[], Vec::new(), None);
    assert!(d.body.contains(&format!("On {}, ", m.sent)));
    assert!(d.body.contains(" wrote:\n> "));
    for line in d.body.lines().skip_while(|l| !l.ends_with("wrote:")).skip(1) {
        assert!(line.starts_with('>'), "{line}");
    }
}

#[test]
fn reply_with_a_selection_quotes_only_the_selection() {
    let h = harness(FakeStore::concept());
    let m = h.state().1.store.get(h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id).unwrap();
    let d = mailbench::app::Draft::reply(&m, mailbench::app::Reply::One, "", &[], Vec::new(), Some("just this line"));
    assert!(d.body.ends_with("wrote:\n> just this line"), "{}", d.body);
    let f = mailbench::app::Draft::reply(&m, mailbench::app::Reply::Forward, "", &[], Vec::new(), Some("just this line"));
    assert!(f.body.contains(&m.paragraphs[0]), "forward keeps the whole message");
}

fn select_in_body(h: &mut Harness<'static, (bool, App)>, from: usize, to: usize) {
    let id = egui::Id::new(mailbench::kit::PLAIN_BODY);
    h.ctx.memory_mut(|m| m.request_focus(id));
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(from), egui::text::CCursor::new(to))));
    state.store(&h.ctx, id);
    h.run();
    h.run();
}

#[test]
fn pressing_r_with_text_selected_in_the_reader_quotes_only_that_text() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    let id = h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    let first = h.state().1.store.get(id).unwrap().paragraphs[0].clone();
    let picked: String = first.chars().take(12).collect();
    select_in_body(&mut h, 0, 12);
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    let d = h.state().1.draft.clone().expect("r opens a reply while the body has focus");
    assert!(d.body.ends_with(&format!("wrote:\n> {}", picked.trim())), "{}", d.body);
    assert!(!d.body.contains(&first[12..]), "the rest of the message was quoted");
}

#[test]
fn the_reply_window_fits_the_screen_with_a_long_quote() {
    let mut store = FakeStore::concept();
    let long = (0..400).map(|n| format!("line {n} of a very long message")).collect::<Vec<_>>();
    store.set_paragraphs(0, long);
    let mut h = harness(store);
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    let send = h.get_by_label("send").rect();
    let discard = h.get_by_label("discard").rect();
    assert!(send.bottom() <= 800.0 && send.top() > 300.0, "send is at {send:?}");
    assert!(discard.bottom() <= 800.0, "discard is off screen at {discard:?}");
    let img = h.render().expect("render");
    img.save(out_dir().join("composer-long.png")).unwrap();
}

#[test]
fn chinese_punctuation_and_emoji_render_with_a_fallback_font() {
    let ctx = egui::Context::default();
    mailbench::kit::setup(&ctx);
    let mut out = ctx.run_ui(Default::default(), |_| {});
    out.textures_delta.clear();
    for c in ['\u{ff1a}', '\u{3010}', '\u{4e2d}', '\u{1f600}'] {
        let found = ctx.fonts_mut(|f| f.has_glyph(&mailbench::kit::body(14.0), c));
        assert!(found, "no glyph for {c:?}");
    }
}

#[test]
fn reply_sets_threading_headers_and_forward_does_not() {
    let h = harness(FakeStore::concept());
    let mut m = h.state().1.store.get(h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id).unwrap();
    m.message_id = Some("orig@x.test".into());
    m.references = vec!["root@x.test".into()];
    let r = mailbench::app::Draft::reply(&m, mailbench::app::Reply::One, "", &[], Vec::new(), None);
    assert_eq!(r.in_reply_to.as_deref(), Some("orig@x.test"));
    assert_eq!(r.references, vec!["root@x.test", "orig@x.test"]);
    let f = mailbench::app::Draft::reply(&m, mailbench::app::Reply::Forward, "", &[], Vec::new(), None);
    assert!(f.in_reply_to.is_none());
    assert!(f.body.contains("---------- Forwarded message ----------"));
    assert!(f.body.contains("Subject: "));
    assert!(f.body.contains("From: "));
}

#[test]
fn forward_carries_the_original_attachments() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::F);
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert!(!d.attachments.is_empty());
}

#[test]
fn signature_is_added_and_follows_the_from_identity() {
    let mut d = mailbench::app::Draft::blank_for(0, "Mara\nKestrel");
    assert!(d.body.contains("-- \nMara\nKestrel"));
    d.set_account(1, "Mara at home");
    assert!(d.body.contains("-- \nMara at home"));
    assert!(!d.body.contains("Kestrel"));
}

#[test]
fn recipients_become_chips_and_autocomplete_fills_from_seen_addresses() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    {
        let d = h.state_mut().1.draft.as_mut().unwrap();
        d.to = "a@x.test, \"Rivera, Tomás\" <t@x.test>".into();
    }
    h.run();
    h.run();
    h.get_by_label_contains("a@x.test");
    h.get_by_label_contains("Rivera, Tomás");
    assert_eq!(split_recipients("a@x.test, \"Rivera, Tomás\" <t@x.test>").len(), 2);
    assert!(!h.state().1.store.suggest("rivera", 5).is_empty());
}

#[test]
fn attachments_are_listed_with_sizes_and_can_be_removed() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    h.state_mut().1.draft.as_mut().unwrap().attach_bytes("board.pdf", vec![1; 2500]);
    h.run();
    h.run();
    h.get_by_label_contains("board.pdf");
    assert_eq!(human_size(2500), "2 KB");
    h.get_by_label("remove board.pdf").click();
    h.run();
    h.run();
    assert!(h.state().1.draft.as_ref().unwrap().attachments.is_empty());
}

#[test]
fn autosave_files_a_draft_and_resaves_in_place() {
    let mut h = harness(FakeStore::concept());
    let before = h.state().1.store.counts(&Query::new(Folder::Drafts)).all;
    h.state_mut().1.autosave = std::time::Duration::ZERO;
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    h.state_mut().1.draft.as_mut().unwrap().subject = "autosaved".into();
    h.run();
    h.run();
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Drafts)).all, before + 1);
    assert!(h.state().1.draft.as_ref().unwrap().saved);
    h.state_mut().1.draft.as_mut().unwrap().subject = "autosaved again".into();
    h.run();
    h.run();
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Drafts)).all, before + 1);
}

#[test]
fn discarding_a_saved_draft_removes_it() {
    let mut h = harness(FakeStore::concept());
    let before = h.state().1.store.counts(&Query::new(Folder::Drafts)).all;
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    h.state_mut().1.draft.as_mut().unwrap().subject = "gone".into();
    h.get_by_label("save draft").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Drafts)).all, before + 1);
    h.get_by_label("discard").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Drafts)).all, before);
}

#[test]
fn undo_send_brings_the_message_back_into_the_composer() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    h.get_by_label("send").click();
    h.run();
    h.run();
    assert!(h.state().1.draft.is_none());
    assert!(h.state().1.pending_send.is_some());
    h.get_by_label("undo").click();
    h.run();
    h.run();
    let app = &h.state().1;
    assert!(app.draft.is_some());
    assert_eq!(app.store.list(&Query::new(Folder::Sent), 0, 10).items.len(), 0);
}

#[test]
fn reply_from_a_thread_quotes_the_focused_message() {
    let mut h = harness(FakeStore::concept());
    let head = inbox_ids(&h.state().1)[0];
    let thread = h.state().1.store.thread(head);
    h.key_press(egui::Key::Enter);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    h.key_press(egui::Key::K);
    h.run();
    assert_eq!(h.state().1.thread_focus, Some(thread[0].summary.id));
    let older = h.state().1.store.get(thread[0].summary.id).unwrap();
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    assert!(d.body.contains(&format!("On {}, ", older.sent)), "{}", d.body);
    assert!(d.body.contains(&format!("> {}", older.paragraphs[0].lines().next().unwrap())), "{}", d.body);
}

#[test]
fn dropped_files_are_attached_with_their_size_and_sent() {
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coupon.bin");
    std::fs::write(&path, vec![7u8; 2500]).unwrap();
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(path)));
    h.run();
    h.run();
    h.get_by_label_contains("coupon.bin");
    let d = h.state().1.draft.clone().unwrap();
    assert_eq!(d.attachments.len(), 1);
    assert_eq!(human_size(d.attachments[0].data.len()), "2 KB");
    h.get_by_label("send").click();
    h.run();
    h.run();
    let sent = h.state().1.store.list(&Query::new(Folder::Sent), 0, 10).items[0].id;
    assert_eq!(h.state().1.store.get(sent).unwrap().attachments[0].name, "coupon.bin");
}

#[test]
fn tab_completes_an_address_from_suggestions() {
    let mut h = harness(FakeStore::concept());
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    let to = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "name or address");
    to.click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "name or address").type_text("rivera");
    h.run();
    h.run();
    h.key_press(egui::Key::Tab);
    h.run();
    h.run();
    h.key_press(egui::Key::Tab);
    h.run();
    h.run();
    let d = h.state().1.draft.clone().unwrap();
    let all = format!("{} {}", d.to, d.to_input);
    assert!(all.contains("rivera") && all.contains('@'), "{all}");
}

fn press(h: &mut Harness<'static, (bool, App)>, key: egui::Key, modifiers: egui::Modifiers) {
    h.key_press_modifiers(modifiers, key);
    h.run();
}

#[test]
fn shift_j_extends_the_marked_range_and_star_a_n_select_all_and_none() {
    let mut h = harness(FakeStore::concept());
    press(&mut h, egui::Key::J, egui::Modifiers::SHIFT);
    press(&mut h, egui::Key::J, egui::Modifiers::SHIFT);
    assert_eq!(h.state().1.marked.iter().copied().collect::<Vec<_>>(), vec![0, 1, 2]);
    press(&mut h, egui::Key::K, egui::Modifiers::SHIFT);
    assert_eq!(h.state().1.marked.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
    h.key_press(egui::Key::Escape);
    h.state_mut().1.marked.clear();
    h.event(egui::Event::Text("*".into()));
    h.run();
    h.key_press(egui::Key::A);
    h.run();
    assert_eq!(h.state().1.marked.len(), 30);
    h.event(egui::Event::Text("*".into()));
    h.run();
    h.key_press(egui::Key::N);
    h.run();
    assert!(h.state().1.marked.is_empty());
}

#[test]
fn archiving_three_marked_rows_and_undoing_brings_them_back() {
    let mut h = harness(FakeStore::concept());
    let ids = inbox_ids(&h.state().1);
    h.state_mut().1.marked = [0usize, 2, 4].into_iter().collect();
    h.key_press(egui::Key::E);
    h.run();
    let app = &h.state().1;
    assert_eq!(app.store.counts(&Query::new(Folder::Inbox)).all, 27);
    assert_eq!(app.store.counts(&Query::new(Folder::Archive)).all, 3);
    assert!(app.marked.is_empty());
    h.key_press(egui::Key::Z);
    h.run();
    let app = &h.state().1;
    assert_eq!(app.store.counts(&Query::new(Folder::Inbox)).all, 30);
    assert_eq!(app.store.counts(&Query::new(Folder::Archive)).all, 0);
    let _ = ids;
}

#[test]
fn flagging_marked_rows_is_undone_by_z() {
    let mut h = harness(FakeStore::concept());
    let before = h.state().1.store.counts(&Query::new(Folder::Inbox)).flagged;
    h.state_mut().1.marked = [1usize, 3, 5].into_iter().collect();
    h.key_press(egui::Key::X);
    h.run();
    let after = h.state().1.store.counts(&Query::new(Folder::Inbox)).flagged;
    assert_ne!(before, after);
    h.key_press(egui::Key::Z);
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Inbox)).flagged, before);
}

#[test]
fn shift_i_marks_every_message_in_the_folder_read_and_z_restores() {
    let mut h = harness(FakeStore::concept());
    let before = h.state().1.store.counts(&Query::new(Folder::Inbox)).unread;
    assert!(before > 1);
    press(&mut h, egui::Key::I, egui::Modifiers::SHIFT);
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Inbox)).unread, 0);
    h.key_press(egui::Key::Z);
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Inbox)).unread, before);
}

#[test]
fn the_folder_context_menu_has_mark_all_as_read() {
    let mut h = harness(FakeStore::concept());
    let before = h.state().1.store.counts(&Query::new(Folder::Inbox)).unread;
    assert!(before > 1);
    h.get_by_label("folder inbox").click_secondary();
    h.run();
    h.get_by_label("mark all as read").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Inbox)).unread, 0);
}

#[test]
fn snoozing_marked_rows_uses_the_picker_and_moves_them_together() {
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.marked = [0usize, 1, 2].into_iter().collect();
    h.key_press(egui::Key::S);
    h.run();
    h.get_by_label("snooze tomorrow 08:00").click();
    h.run();
    h.run();
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Snoozed)).all, 3);
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Inbox)).all, 27);
}

#[test]
fn sidebar_lists_labels_and_clicking_one_searches_it() {
    let mut h = harness(FakeStore::concept());
    h.get_by_label("label bench night");
    h.get_by_label("label invoices");
    h.get_by_label("label fab").click();
    h.run();
    h.run();
    let app = &h.state().1;
    assert_eq!(app.search, "label:fab");
    assert_eq!(Parsed::parse(&app.search).label.as_deref(), Some("fab"));
}

#[test]
fn clicking_a_label_with_spaces_quotes_it_in_the_query() {
    let mut h = harness(FakeStore::concept());
    h.get_by_label("label bench night").click();
    h.run();
    h.run();
    let app = &h.state().1;
    assert_eq!(app.search, "label:\"bench night\"");
    assert_eq!(Parsed::parse(&app.search).label.as_deref(), Some("bench night"));
}

#[test]
fn imap_style_accounts_show_the_new_folder_action() {
    let h = harness(FakeStore::concept());
    let name = h.state().1.store.accounts()[0].name.clone();
    h.get_by_label(&format!("new folder in {name}"));
}

fn html_harness(html: &str) -> Harness<'static, (bool, App)> {
    let mut store = FakeStore::concept();
    let id = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    store.set_html(id, html);
    let mut h = harness(store);
    for _ in 0..40 {
        h.run();
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h
}

fn copied(h: &Harness<'static, (bool, App)>) -> Option<String> {
    h.output().platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    })
}

#[test]
fn copy_with_no_selection_copies_the_html_text() {
    let mut h = html_harness("<p>First paragraph here</p><p>Second paragraph</p>");
    h.event(egui::Event::Copy);
    h.step();
    assert_eq!(copied(&h).as_deref(), Some("First paragraph here\nSecond paragraph"));
}

#[test]
fn dragging_selects_text_and_copy_takes_only_that() {
    let mut h = html_harness("<body style=\"margin:0\"><p style=\"margin:0\">Selectable words</p><p style=\"margin:0\">Other line below</p></body>");
    let body = h.get_by_label("message body");
    let rect = body.rect();
    let start = rect.left_top() + egui::vec2(1.0, 6.0);
    let end = rect.left_top() + egui::vec2(400.0, 8.0);
    h.hover_at(start);
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run();
    for step in 1..=8 {
        h.hover_at(start + (end - start) * (step as f32 / 8.0));
        h.run();
    }
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run();
    h.event(egui::Event::Copy);
    h.step();
    assert_eq!(copied(&h).as_deref(), Some("Selectable words"));
}

#[test]
fn trusting_a_sender_loads_images_without_asking_again() {
    let mut h = harness(FakeStore::concept());
    h.get_by_label("always from sender").click();
    h.run();
    h.run();
    assert!(h.query_by_label_contains("REMOTE IMAGES BLOCKED").is_none());
    assert_eq!(h.state().1.trusted.len(), 1);
}

#[test]
fn rebound_archive_key_archives_and_the_hint_bar_shows_it() {
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.prefs.keys.set("archive", egui::Key::B).unwrap();
    h.run();
    assert!(h.state().1.prefs.keys.hint_items().iter().any(|(k, a)| k == "b" && a == "archive"));
    let first = inbox_ids(&h.state().1)[0];
    h.key_press(egui::Key::E);
    h.run();
    assert!(inbox_ids(&h.state().1).contains(&first));
    h.key_press(egui::Key::B);
    h.run();
    assert!(!inbox_ids(&h.state().1).contains(&first));
}

#[test]
fn never_policy_hides_the_load_once_button() {
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.prefs.reading.remote_images = mailbench::prefs::RemoteImages::Never;
    h.run();
    h.run();
    h.get_by_label_contains("REMOTE IMAGES BLOCKED");
    assert!(h.query_by_label("load once").is_none());
}

#[test]
fn trusted_only_policy_hides_the_load_once_button_but_keeps_trust_choices() {
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.prefs.reading.remote_images = mailbench::prefs::RemoteImages::Trusted;
    h.run();
    h.run();
    assert!(h.query_by_label("load once").is_none());
    h.get_by_label("always from sender");
}

#[test]
fn compact_density_fits_more_rows_than_comfortable() {
    let mut h = harness(FakeStore::bulk(10_000));
    let comfortable = h.state().1.rendered_rows;
    h.state_mut().1.prefs.density = mailbench::prefs::Density::Compact;
    h.run();
    h.run();
    assert!(h.state().1.rendered_rows > comfortable, "{} vs {comfortable}", h.state().1.rendered_rows);
}

#[test]
fn default_reading_zoom_seeds_the_html_zoom() {
    let mut h = harness(FakeStore::concept());
    h.state_mut().1.html_state.default_zoom = 1500;
    h.run();
    h.run();
    assert!(h.state().1.html_state.zoom.values().all(|z| *z == 1500));
}

fn wide_harness(store: FakeStore, app_setup: impl FnOnce(&mut App)) -> Harness<'static, (bool, App)> {
    let mut app = App::with_store(Box::new(store));
    app.selected = 0;
    app_setup(&mut app);
    let mut h = Harness::builder().renderer(quiet_renderer())
        .with_size(egui::vec2(1920.0, 1080.0))
        .with_step_dt(0.02)
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
    for _ in 0..40 {
        h.run();
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h
}

fn html_store() -> FakeStore {
    let mut store = FakeStore::concept();
    let id = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    store.set_html(id, "<p>First paragraph here</p>");
    store
}

fn plain_store() -> FakeStore {
    let store = FakeStore::concept();
    let id = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    assert!(store.get(id).unwrap().html.is_empty());
    store
}

#[test]
fn full_width_reader_fills_the_pane_for_plain_and_html_mail() {
    for store in [plain_store(), html_store()] {
        let h = wide_harness(store, |_| {});
        let app = &h.state().1;
        assert!(app.reader_pane_w > 900.0, "{}", app.reader_pane_w);
        assert!(app.reader_content_w >= app.reader_pane_w - 20.0, "{} of {}", app.reader_content_w, app.reader_pane_w);
    }
}

#[test]
fn comfortable_reader_width_keeps_the_old_cap() {
    for (store, cap) in [(plain_store(), 604.0), (html_store(), 820.0)] {
        let h = wide_harness(store, |app| app.prefs.reading.reader_width = mailbench::prefs::ReaderWidth::Comfortable);
        let app = &h.state().1;
        assert!(app.reader_content_w <= cap + 0.5, "{}", app.reader_content_w);
        assert!(app.reader_content_w > cap - 40.0, "{}", app.reader_content_w);
    }
}

#[test]
fn w_toggles_the_reader_width() {
    let mut h = wide_harness(plain_store(), |_| {});
    h.key_press(egui::Key::W);
    h.run();
    h.run();
    let app = &h.state().1;
    assert_eq!(app.prefs.reading.reader_width, mailbench::prefs::ReaderWidth::Comfortable);
    assert!(app.reader_content_w < 700.0);
}

#[test]
fn dragging_the_list_divider_resizes_the_reader_and_persists() {
    let saved = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = saved.clone();
    let mut h = wide_harness(plain_store(), move |app| {
        app.prefs_saver = Box::new(move |p| *sink.borrow_mut() = Some(p.clone()));
    });
    let before = h.state().1.reader_pane_w;
    let x = 747.0;
    h.hover_at(egui::pos2(x, 500.0));
    h.run();
    h.drag_at(egui::pos2(x, 500.0));
    h.run();
    h.hover_at(egui::pos2(x + 200.0, 500.0));
    h.run();
    h.drop_at(egui::pos2(x + 200.0, 500.0));
    h.run();
    h.run();
    let after = h.state().1.reader_pane_w;
    assert!((before - after - 200.0).abs() < 6.0, "{before} -> {after}");
    let prefs = saved.borrow().clone().expect("saved");
    assert!(prefs.layout.list_w > 700, "{}", prefs.layout.list_w);

    let h2 = wide_harness(plain_store(), move |app| app.prefs = prefs);
    assert!((h2.state().1.reader_pane_w - after).abs() < 2.0);
}

#[test]
fn double_clicking_the_divider_resets_the_list_width() {
    let mut h = wide_harness(plain_store(), |app| app.prefs.layout.list_w = 700);
    let before = h.state().1.reader_pane_w;
    let x = 900.0;
    h.hover_at(egui::pos2(x, 500.0));
    h.run();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: egui::pos2(x, 500.0),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    h.step();
    h.run();
    h.run();
    let after = h.state().1.reader_pane_w;
    assert!((after - before - 160.0).abs() < 6.0, "{before} -> {after}");
    assert_eq!(h.state().1.prefs.layout.list_w, 540);
}

#[test]
fn a_wide_html_table_scrolls_instead_of_shrinking_below_legibility() {
    let mut store = FakeStore::concept();
    let id = store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id;
    store.set_html(id, "<body style=\"margin:0\"><table width=\"2400\"><tr><td>wide</td></tr></table></body>");
    let h = wide_harness(store, |app| app.prefs.reading.reader_width = mailbench::prefs::ReaderWidth::Comfortable);
    let app = &h.state().1;
    assert!(app.html_state.shown_width > app.reader_content_w + 100.0, "{} vs {}", app.html_state.shown_width, app.reader_content_w);
}
