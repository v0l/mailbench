#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InsetsPx {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub fn merge_insets(bars: InsetsPx, cutout: InsetsPx) -> InsetsPx {
    let side = |a: f32, b: f32| a.max(b).max(0.0);
    InsetsPx {
        left: side(bars.left, cutout.left),
        top: side(bars.top, cutout.top),
        right: side(bars.right, cutout.right),
        bottom: side(bars.bottom, cutout.bottom),
    }
}

pub fn inset_rect(rect: egui::Rect, insets: InsetsPx, pixels_per_point: f32) -> egui::Rect {
    let ppp = pixels_per_point.max(0.1);
    let min_x = rect.min.x + insets.left / ppp;
    let min_y = rect.min.y + insets.top / ppp;
    egui::Rect::from_min_max(
        egui::pos2(min_x, min_y),
        egui::pos2((rect.max.x - insets.right / ppp).max(min_x), (rect.max.y - insets.bottom / ppp).max(min_y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_bar_sits_below_status_bar() {
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 800.0));
        let r = inset_rect(full, InsetsPx { top: 120.0, bottom: 144.0, ..Default::default() }, 3.0);
        assert_eq!(r.min.y, 40.0);
        assert_eq!(r.max.y, 752.0);
        assert_eq!(r.width(), 400.0);
    }

    #[test]
    fn side_insets_shrink_width() {
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 400.0));
        let r = inset_rect(full, InsetsPx { left: 60.0, right: 30.0, ..Default::default() }, 3.0);
        assert_eq!(r.min.x, 20.0);
        assert_eq!(r.max.x, 790.0);
    }

    #[test]
    fn cutout_larger_than_bar_wins() {
        let m = merge_insets(InsetsPx { left: 0.0, top: 60.0, right: 0.0, bottom: 100.0 }, InsetsPx { left: 128.0, top: 40.0, right: 0.0, bottom: 0.0 });
        assert_eq!(m, InsetsPx { left: 128.0, top: 60.0, right: 0.0, bottom: 100.0 });
    }

    #[test]
    fn cutout_on_right_after_rotation() {
        let m = merge_insets(InsetsPx { bottom: 48.0, ..Default::default() }, InsetsPx { right: 128.0, ..Default::default() });
        assert_eq!(m, InsetsPx { right: 128.0, bottom: 48.0, ..Default::default() });
    }

    #[test]
    fn merge_ignores_negative_and_keeps_oversized() {
        let m = merge_insets(InsetsPx { left: -5.0, top: 9000.0, right: 3.0, bottom: -1.0 }, InsetsPx { left: -9.0, top: 1.0, right: -2.0, bottom: -3.0 });
        assert_eq!(m, InsetsPx { left: 0.0, top: 9000.0, right: 3.0, bottom: 0.0 });
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let r = inset_rect(full, m, 1.0);
        assert!(r.width() >= 0.0 && r.height() >= 0.0);
    }

    #[test]
    fn oversized_insets_do_not_invert() {
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let r = inset_rect(full, InsetsPx { left: 300.0, top: 300.0, right: 300.0, bottom: 300.0 }, 1.0);
        assert!(r.width() >= 0.0 && r.height() >= 0.0);
    }
}
