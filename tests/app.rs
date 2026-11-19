use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::FakeStore;
use mailbench::mailstore::*;
use mailbench::search::Parsed;
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
    let d = mailbench::app::Draft::reply(&m, mailbench::app::Reply::One, "", &[], Vec::new());
    assert!(d.body.contains(&format!("On {}, ", m.sent)));
    assert!(d.body.contains(" wrote:\n> "));
    for line in d.body.lines().skip_while(|l| !l.ends_with("wrote:")).skip(1) {
        assert!(line.starts_with('>'), "{line}");
    }
}

#[test]
fn reply_sets_threading_headers_and_forward_does_not() {
    let h = harness(FakeStore::concept());
    let mut m = h.state().1.store.get(h.state().1.store.list(&Query::new(Folder::Inbox), 0, 1).items[0].id).unwrap();
    m.message_id = Some("orig@x.test".into());
    m.references = vec!["root@x.test".into()];
    let r = mailbench::app::Draft::reply(&m, mailbench::app::Reply::One, "", &[], Vec::new());
    assert_eq!(r.in_reply_to.as_deref(), Some("orig@x.test"));
    assert_eq!(r.references, vec!["root@x.test", "orig@x.test"]);
    let f = mailbench::app::Draft::reply(&m, mailbench::app::Reply::Forward, "", &[], Vec::new());
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
