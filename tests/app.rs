use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::FakeStore;
use mailbench::mailstore::*;
use std::path::PathBuf;

fn harness(store: FakeStore) -> Harness<'static, (bool, App)> {
    let mut app = App::with_store(Box::new(store));
    app.selected = 0;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
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
    assert_eq!(d.to, "Tomás Rivera");
    assert_eq!(d.subject, "Re: Rev C stencil quote and lead time");
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
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
    std::fs::create_dir_all(&d).unwrap();
    d
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

    h.state_mut().1.reader_focus = true;
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
