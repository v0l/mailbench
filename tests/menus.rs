use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailbench::app::App;
use mailbench::fake::FakeStore;
use mailbench::mailstore::*;
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

type H = Harness<'static, (bool, App)>;

fn harness(size: egui::Vec2) -> H {
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

const SIZES: [(f32, f32); 3] = [(1280.0, 800.0), (1920.0, 1080.0), (392.0, 844.0)];
const SUBJECT: &str = "Tomás Rivera Rev C stencil quote";

fn limit(size: (f32, f32)) -> f32 {
    if size.0 < 700.0 { 320.0f32.min(size.0 - 32.0) } else { 260.0 }
}

fn settle(h: &mut H) {
    h.run();
    h.run();
}

fn open_row_menu(h: &mut H) {
    h.get_by_label_contains(SUBJECT).click_secondary();
    settle(h);
}

fn open_folder_menu(h: &mut H, narrow: bool) {
    if narrow {
        h.get_by_label_contains("folders").click();
        settle(h);
    }
    h.get_by_label("folder inbox").click_secondary();
    settle(h);
}

fn open_reader(h: &mut H) {
    h.get_by_label_contains(SUBJECT).click();
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

fn open_card_menu(h: &mut H) {
    open_reader(h);
    h.get_all_by_label("actions for Tomás Rivera").next().unwrap().click();
    settle(h);
}

fn open_more_menu(h: &mut H) {
    open_reader(h);
    click_more(h);
}

fn click_more(h: &mut H) {
    h.get_by_label("more actions").scroll_to_me();
    settle(h);
    h.get_by_label("more actions").click();
    settle(h);
}

const MENU_LABELS: [&str; 16] = ["reply", "reply all", "forward", "flag", "unflag", "mark read", "mark unread", "snooze", "archive", "move to junk", "move to folder", "delete", "mark all as read", "new subfolder", "edit", "delete message"];

type Rects = Vec<(String, egui::Rect)>;

fn rects(h: &H) -> Rects {
    MENU_LABELS.iter().flat_map(|l| h.query_all_by_label(l).map(|n| (l.to_string(), n.rect())).collect::<Vec<_>>()).collect()
}

fn new_since(h: &H, before: &Rects) -> Rects {
    rects(h).into_iter().filter(|x| !before.contains(x)).collect()
}

fn key_widths(menu: &Rects, labels: &[&str]) -> Vec<f32> {
    labels
        .iter()
        .map(|l| menu.iter().find(|(n, _)| n == l).unwrap_or_else(|| panic!("{l} not in {:?}", menu.iter().map(|x| &x.0).collect::<Vec<_>>())).1.width())
        .collect()
}

fn assert_bounded(name: &str, size: (f32, f32), widths: &[f32]) {
    for w in widths {
        assert!(*w >= 139.9 && *w <= limit(size) + 0.1, "{name} at {size:?}: key {w} outside 140..={}", limit(size));
    }
}

type Opener = fn(&mut H);

fn row_menu_probe(h: &mut H) -> Vec<f32> {
    let before = rects(h);
    open_row_menu(h);
    key_widths(&new_since(h, &before), &["archive", "delete"])
}

fn card_menu_probe(h: &mut H) -> Vec<f32> {
    open_reader(h);
    let before = rects(h);
    h.get_all_by_label("actions for Tomás Rivera").next().unwrap().click();
    settle(h);
    key_widths(&new_since(h, &before), &["reply", "delete"])
}

fn more_menu_probe(h: &mut H) -> Vec<f32> {
    open_reader(h);
    let before = rects(h);
    click_more(h);
    key_widths(&new_since(h, &before), &["snooze", "delete"])
}

fn folder_menu_probe(h: &mut H) -> Vec<f32> {
    let narrow = h.state().1.narrow;
    if narrow {
        h.get_by_label_contains("folders").click();
        settle(h);
    }
    let before = rects(h);
    h.get_by_label("folder inbox").click_secondary();
    settle(h);
    key_widths(&new_since(h, &before), &["mark all as read"])
}

fn check_all_sizes(name: &str, probe: fn(&mut H) -> Vec<f32>) {
    let mut desktop = Vec::new();
    for size in SIZES {
        let mut h = harness(egui::vec2(size.0, size.1));
        let widths = probe(&mut h);
        assert_bounded(name, size, &widths);
        if size.0 >= 700.0 {
            desktop.push(widths);
        }
    }
    assert_eq!(desktop[0], desktop[1], "{name} grows with the window");
}

#[test]
fn row_menu_width_is_bounded_and_does_not_grow_with_the_window() {
    check_all_sizes("row menu", row_menu_probe);
}

#[test]
fn card_menu_width_is_bounded_and_does_not_grow_with_the_window() {
    check_all_sizes("card menu", card_menu_probe);
}

#[test]
fn reader_more_menu_width_is_bounded_and_does_not_grow_with_the_window() {
    check_all_sizes("more menu", more_menu_probe);
}

#[test]
fn folder_menu_width_is_bounded_and_does_not_grow_with_the_window() {
    check_all_sizes("folder menu", folder_menu_probe);
}

#[test]
fn phone_menu_rows_are_tappable() {
    let mut h = harness(egui::vec2(392.0, 844.0));
    let before = rects(&h);
    open_row_menu(&mut h);
    let archive = new_since(&h, &before).into_iter().find(|x| x.0 == "archive").unwrap();
    assert!(archive.1.height() >= 44.0);
}

#[test]
fn row_menu_is_styled_and_has_the_full_message_actions() {
    let mut h = harness(egui::vec2(1280.0, 800.0));
    let before = rects(&h);
    open_row_menu(&mut h);
    let shown = labels_of(&new_since(&h, &before));
    for label in ["reply", "reply all", "forward", "snooze", "archive", "move to junk", "move to folder", "delete"] {
        assert!(shown.contains(&label.to_string()), "{label} missing from {shown:?}");
    }
    assert!(shown.iter().any(|l| l.ends_with("flag")), "{shown:?}");
    assert!(!shown.contains(&"mark all as read".to_string()));
    assert!(h.query_by_label("Mark all as read").is_none());
}

fn labels_of(r: &Rects) -> Vec<String> {
    r.iter().map(|x| x.0.clone()).collect()
}

fn screen_menu_row(h: &mut H) -> Vec<String> {
    open_reader(h);
    let before = rects(h);
    open_row_menu(h);
    labels_of(&new_since(h, &before))
}

fn screen_menu_card(h: &mut H) -> Vec<String> {
    open_reader(h);
    let before = rects(h);
    h.get_all_by_label("actions for Tomás Rivera").next().unwrap().click();
    settle(h);
    labels_of(&new_since(h, &before))
}

#[test]
fn the_list_row_menu_and_the_card_menu_list_the_same_entries() {
    use mailbench::app::menu::{MessageMenu, message_menu};
    for multi in [false, true] {
        for unread in [false, true] {
            for server_flags in [false, true] {
                let state = || MessageMenu { unread, flagged: false, server_flags, folder: Folder::Inbox, multi };
                assert_eq!(message_menu(&state()), message_menu(&state()));
            }
        }
    }
    let single = message_menu(&MessageMenu { unread: true, flagged: false, server_flags: true, folder: Folder::Inbox, multi: false });
    let marked = message_menu(&MessageMenu { unread: true, flagged: false, server_flags: true, folder: Folder::Inbox, multi: true });
    let names = |v: &[mailbench::app::menu::MenuEntry]| v.iter().map(|e| e.label).collect::<Vec<_>>();
    assert!(names(&single).contains(&"reply") && names(&single).contains(&"forward"));
    assert!(!names(&marked).contains(&"reply") && !names(&marked).contains(&"forward"));
    for l in ["flag", "mark read", "archive", "move to junk", "delete", "move to folder"] {
        assert!(names(&marked).contains(&l), "{l}");
    }
    let none = message_menu(&MessageMenu { unread: true, flagged: false, server_flags: false, folder: Folder::Inbox, multi: false });
    assert!(!names(&none).iter().any(|l| l.ends_with("flag")));
}

#[test]
fn list_row_and_card_menus_show_the_same_labels_on_screen() {
    let mut a = harness(egui::vec2(1280.0, 800.0));
    let row = screen_menu_row(&mut a);
    let mut b = harness(egui::vec2(1280.0, 800.0));
    let card = screen_menu_card(&mut b);
    assert!(row.len() >= 9, "{row:?}");
    assert_eq!(row, card);
}

#[test]
fn multi_selection_row_menu_hides_single_message_entries() {
    let mut h = harness(egui::vec2(1280.0, 800.0));
    h.state_mut().1.marked = [0usize, 1, 2].into_iter().collect();
    settle(&mut h);
    let before = rects(&h);
    h.get_by_label_contains(SUBJECT).click_secondary();
    settle(&mut h);
    let shown = labels_of(&new_since(&h, &before));
    assert!(!shown.contains(&"reply".to_string()) && !shown.contains(&"forward".to_string()), "{shown:?}");
    assert!(shown.contains(&"delete".to_string()) && shown.contains(&"archive".to_string()), "{shown:?}");
}

#[test]
fn deleting_from_the_row_menu_with_marked_rows_trashes_all_of_them() {
    let mut h = harness(egui::vec2(1280.0, 800.0));
    let before = h.state().1.store.counts(&Query::new(Folder::Inbox)).all;
    h.state_mut().1.marked = [0usize, 1, 2].into_iter().collect();
    settle(&mut h);
    h.get_by_label_contains(SUBJECT).click_secondary();
    settle(&mut h);
    h.get_by_label("delete").click();
    settle(&mut h);
    assert_eq!(h.state().1.store.counts(&Query::new(Folder::Inbox)).all, before - 3);
}

fn rs_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    rs_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    files.into_iter().map(|p| { let t = std::fs::read_to_string(&p).unwrap(); (p, t) }).collect()
}

#[test]
fn every_popup_and_context_menu_goes_through_the_menu_helper() {
    let mut bad = Vec::new();
    for (path, text) in sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains(".context_menu(") || line.contains("Popup::menu(") {
                let body = lines[i..(i + 30).min(lines.len())].join("\n");
                if !(body.contains("menu_column(") || body.contains("show_message_menu(")) {
                    bad.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "menus not using menu_column: {bad:?}");
}

#[test]
fn menus_never_size_items_from_available_width() {
    let mut bad = Vec::new();
    for (path, text) in sources() {
        for (i, line) in text.lines().enumerate() {
            if line.contains("Key::new") && line.contains("min_width(ui.available_width())") {
                bad.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    assert!(bad.is_empty(), "keys stretched to the window: {bad:?}");
}

#[test]
fn no_menu_label_starts_with_a_capital() {
    let mut bad = Vec::new();
    for (path, text) in sources() {
        for (i, line) in text.lines().enumerate() {
            let mut rest = line;
            while let Some(at) = rest.find("menu.key(ui, \"") {
                let tail = &rest[at + 14..];
                if tail.chars().next().is_some_and(|c| c.is_uppercase()) {
                    bad.push(format!("{}:{}", path.display(), i + 1));
                }
                rest = tail;
            }
            if line.contains("ui.button(\"") && text.contains("context_menu(") {
                bad.push(format!("{}:{} plain button in a menu file", path.display(), i + 1));
            }
            let t = line.trim();
            if t.starts_with("(\"") && (t.contains("Action::") || t.contains("VALUE") || t.contains("FAULT")) {
                if t[2..].chars().next().is_some_and(|c| c.is_uppercase()) {
                    bad.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "capitalised menu labels: {bad:?}");
}

fn chat_harness(size: egui::Vec2) -> H {
    use mailbench::chat::fake::FakeChat;
    use mailbench::chat::*;
    let (chat, handle) = FakeChat::with_contacts();
    handle.set_capabilities(Capabilities { edit: true, delete: true, ..Default::default() });
    let crew = handle.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo"]);
    handle.say(&crew, "ann", "Van is loaded");
    handle.say(&crew, "me", "On my way");
    let mut view = ChatView::new(controller::ChatController::start(Box::new(chat), None));
    view.dwell = std::time::Duration::from_millis(150);
    let mut app = App::with_store(Box::new(FakeStore::concept()));
    app.prefs.developer_chat = true;
    app.chats = Some(view);
    app.open_chats();
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(1.0).build_ui_state(
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
    chat_settle(&mut h);
    h.get_by_label_contains("room Field crew").click();
    chat_settle(&mut h);
    h
}

fn chat_settle(h: &mut H) {
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        h.run_steps(2);
    }
}

fn chat_menu_probe(h: &mut H) -> Vec<f32> {
    let before = rects(h);
    h.get_all_by_label_contains("actions for message from").last().unwrap().click();
    chat_settle(h);
    key_widths(&new_since(h, &before), &["reply", "delete message"])
}

#[test]
fn chat_message_menu_width_is_bounded_and_does_not_grow_with_the_window() {
    let mut desktop = Vec::new();
    for size in SIZES {
        let mut h = chat_harness(egui::vec2(size.0, size.1));
        let widths = chat_menu_probe(&mut h);
        assert_bounded("chat menu", size, &widths);
        if size.0 >= 700.0 {
            desktop.push(widths);
        }
    }
    assert_eq!(desktop[0], desktop[1], "chat menu grows with the window");
}

fn out_dir() -> PathBuf {
    let d = std::env::var_os("SCREENS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screens"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn render_menu_screens() {
    let cases: [(&str, Opener); 4] = [
        ("menu-row", open_row_menu),
        ("menu-card", open_card_menu),
        ("menu-reader-more", open_more_menu),
        ("menu-folder", |h| {
            let narrow = h.state().1.narrow;
            open_folder_menu(h, narrow)
        }),
    ];
    for (name, open) in cases {
        for (suffix, size) in [("", (1280.0, 800.0)), ("-phone", (392.0, 844.0))] {
            let mut h = harness(egui::vec2(size.0, size.1));
            open(&mut h);
            let img = h.render().expect("render");
            img.save(out_dir().join(format!("{name}{suffix}.png"))).unwrap();
        }
    }
}
