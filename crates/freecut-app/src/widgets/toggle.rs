//! The toggle switch (ui-concept §13.8): a 32 × 18 pt pill. Off: a
//! `bg.active` fill with a hollow thumb (High contrast adds the
//! `border.control` outline). On: an `accent` fill with a `text.on-accent`
//! thumb holding a small check, so the state never rests on colour alone.
//! The thumb slides over `dur.fast`.

use eframe::egui::{self, Id, Response, Sense, Shape, Stroke, StrokeKind, WidgetInfo, WidgetType};

use crate::theme;

/// Draws a switch for `on`; a click (or `Space`/`Enter` while focused)
/// flips it and marks the response changed. `label` names it for screen
/// readers.
pub fn toggle(ui: &mut egui::Ui, id: Id, on: &mut bool, label: &str) -> Response {
    let size = egui::vec2(32.0, 18.0);
    // The hit area is at least 24 pt high (UI-043).
    let (outer, _) = ui.allocate_exact_size(egui::vec2(size.x, size.y.max(24.0)), Sense::hover());
    let mut response = ui.interact(outer, id, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let label = label.to_owned();
    let state = *on;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, state, &label));
    if ui.is_rect_visible(outer) {
        let p = theme::look(ui.ctx()).palette;
        let rect = egui::Rect::from_center_size(outer.center(), size);
        let how_on =
            ui.ctx()
                .animate_bool_with_time(id.with("anim"), *on, ui.style().animation_time);
        let radius = rect.height() / 2.0;
        let enabled = ui.is_enabled();
        let (fill, stroke) = if !enabled {
            (
                egui::Color32::TRANSPARENT,
                Stroke::new(1.0, p.text_disabled),
            )
        } else if *on {
            let fill = if response.hovered() {
                p.accent_hover
            } else {
                p.accent
            };
            (fill, Stroke::NONE)
        } else {
            let fill = if response.hovered() {
                p.bg_active.lerp_to_gamma(p.text_tertiary, 0.15)
            } else {
                p.bg_active
            };
            (fill, theme::look(ui.ctx()).control_border())
        };
        ui.painter()
            .rect(rect, radius, fill, stroke, StrokeKind::Inside);
        let thumb_r = radius - 3.0;
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = egui::pos2(x, rect.center().y);
        if *on && enabled {
            ui.painter()
                .circle_filled(center, thumb_r, p.text_on_accent);
            // The check inside the thumb (shape cue).
            let s = thumb_r * 0.55;
            ui.painter().add(Shape::line(
                vec![
                    center + egui::vec2(-s, 0.0),
                    center + egui::vec2(-s * 0.3, s * 0.7),
                    center + egui::vec2(s, -s * 0.6),
                ],
                Stroke::new(1.5, p.accent),
            ));
        } else {
            let color = if enabled {
                p.text_secondary
            } else {
                p.text_disabled
            };
            ui.painter()
                .circle_stroke(center, thumb_r, Stroke::new(1.5, color));
        }
        if super::icon_button::keyboard_focus(ui, &response) {
            super::focus_ring(ui, rect, theme::R_SM.max(9));
        }
    }
    response
}
