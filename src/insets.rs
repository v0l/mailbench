#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InsetsPx {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
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
    fn oversized_insets_do_not_invert() {
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let r = inset_rect(full, InsetsPx { left: 300.0, top: 300.0, right: 300.0, bottom: 300.0 }, 1.0);
        assert!(r.width() >= 0.0 && r.height() >= 0.0);
    }
}
