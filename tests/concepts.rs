use egui_kittest::Harness;
use mailbench::app::State;
use mailbench::{Concept, Screen};
use std::path::PathBuf;

fn render(concept: Concept, screen: Screen) -> image::RgbaImage {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            |ui, (ready, state): &mut (bool, State)| {
                if !*ready {
                    mailbench::kit::setup(ui.ctx());
                    *ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                state.show(ui);
            },
            (false, State::new(concept, screen)),
        );
    h.run();
    h.run();
    h.render().expect("render")
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/concepts");
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn render_every_concept_screen() {
    for c in Concept::ALL {
        for s in Screen::ALL {
            let img = render(c, s);
            assert_eq!(img.width(), 1280);
            let path = out_dir().join(format!("{}-{}.png", c.slug(), s.slug()));
            img.save(&path).unwrap();
        }
    }
}

#[test]
fn copy_has_no_em_or_en_dashes() {
    for f in ["src/data.rs", "src/kit.rs", "src/concept_a.rs", "src/concept_b.rs", "src/concept_c.rs", "docs/design.md"] {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(f)).unwrap();
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{f}");
    }
}
