pub fn inset_rect(rect: egui::Rect, top_px: f32, bottom_px: f32, pixels_per_point: f32) -> egui::Rect {
    let ppp = pixels_per_point.max(0.1);
    let min_y = rect.min.y + top_px / ppp;
    egui::Rect::from_min_max(
        egui::pos2(rect.min.x, min_y),
        egui::pos2(rect.max.x, (rect.max.y - bottom_px / ppp).max(min_y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_bar_sits_below_status_bar() {
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 800.0));
        let r = inset_rect(full, 120.0, 144.0, 3.0);
        assert_eq!(r.min.y, 40.0);
        assert_eq!(r.max.y, 752.0);
        assert_eq!(r.width(), 400.0);
    }

    #[test]
    fn oversized_insets_do_not_invert() {
        let full = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let r = inset_rect(full, 300.0, 300.0, 1.0);
        assert!(r.height() >= 0.0);
    }
}
