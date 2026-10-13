//! The help badge and the reset badge (ui-concept §13.21): 16 pt circles
//! in `bg.field` that sit in a section header (or a row) and take their own
//! clicks, so pressing one never folds the section.
//!
//! - **Help** `?`: on hover the circle turns `overlay.selection`, the mark
//!   `accent.text`, the cursor the help cursor, and a 260 pt-wide
//!   explanation pops over beside it without moving anything.
//! - **Reset**: a counter-clockwise arrow that resets the section (one
//!   undo entry, the caller's). With nothing to reset it stays, at 40 %,
//!   and explains why: never hidden, so the header never shifts.

use eframe::egui::{
    self, Align2, CursorIcon, Id, Response, Sense, Stroke, StrokeKind, WidgetInfo, WidgetType,
};

use crate::theme::{self, Type, fonts, icons};

/// The badge's diameter.
pub const BADGE: f32 = 16.0;

/// The width of the explanation popover.
const HELP_WIDTH: f32 = 260.0;

/// A badge's hit area: the circle with 4 pt around it, at least 24 pt tall
/// (UI-043's target size along the header).
fn allocate(ui: &mut egui::Ui, id: Id, sense: Sense) -> (egui::Rect, Response) {
    let (outer, _) = ui.allocate_exact_size(egui::vec2(BADGE + 4.0, 24.0), Sense::hover());
    let response = ui.interact(outer, id, sense);
    (outer, response)
}

/// A help badge explaining `text`. Returns its response.
pub fn help_badge(ui: &mut egui::Ui, id: Id, text: &str) -> Response {
    let (outer, response) = allocate(ui, id, Sense::hover() | Sense::FOCUSABLE);
    let p = theme::look(ui.ctx()).palette;
    let hovered = response.hovered() || response.has_focus();
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::Help);
    }
    if ui.is_rect_visible(outer) {
        let t = ui
            .ctx()
            .animate_bool_with_time(id.with("lit"), hovered, ui.style().animation_time);
        let fill = p.bg_field.lerp_to_gamma(p.overlay_selection(), t);
        let mark = p.text_secondary.lerp_to_gamma(p.accent_text, t);
        let c = outer.center();
        ui.painter().circle_filled(c, BADGE / 2.0, fill);
        outline(ui, c);
        ui.painter().text(
            c,
            Align2::CENTER_CENTER,
            "?",
            Type::Micro.style(ui.style()).resolve(ui.style()),
            mark,
        );
        if super::icon_button::keyboard_focus(ui, &response) {
            super::focus_ring(ui, outer.shrink(2.0), 8);
        }
    }
    let label = text.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, &label));
    let text = text.to_owned();
    response.on_hover_ui(|ui| {
        ui.set_max_width(HELP_WIDTH);
        let p = theme::look(ui.ctx()).palette;
        ui.label(
            egui::RichText::new(text)
                .text_style(egui::TextStyle::Small)
                .color(p.text_primary),
        );
    })
}

/// A reset badge for a section: clickable only when `dirty` (something
/// differs from the defaults); otherwise drawn at 40 % with a tooltip
/// saying there is nothing to reset. Returns whether it was clicked.
pub fn reset_badge(ui: &mut egui::Ui, id: Id, dirty: bool, tooltip: &str) -> bool {
    let enabled = dirty && ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (outer, response) = allocate(ui, id, sense);
    let p = theme::look(ui.ctx()).palette;
    if ui.is_rect_visible(outer) {
        let hovered = enabled && response.hovered();
        let t = ui
            .ctx()
            .animate_bool_with_time(id.with("lit"), hovered, ui.style().animation_time);
        let c = outer.center();
        let opacity = if enabled { 1.0 } else { 0.4 };
        let fill = p.bg_field.lerp_to_gamma(p.bg_hover, t);
        ui.painter()
            .circle_filled(c, BADGE / 2.0, fill.gamma_multiply(opacity));
        outline(ui, c);
        let glyph = p.text_secondary.lerp_to_gamma(p.text_primary, t);
        ui.painter().text(
            c,
            Align2::CENTER_CENTER,
            icons::RESET,
            fonts::icon_font(ui.ctx(), 10.0, false),
            glyph.gamma_multiply(opacity),
        );
        if super::icon_button::keyboard_focus(ui, &response) {
            super::focus_ring(ui, outer.shrink(2.0), 8);
        }
    }
    let label = tooltip.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label));
    let response = if enabled {
        response.on_hover_text(tooltip)
    } else {
        response.on_hover_text("Nothing to reset: every setting here is at its default")
    };
    enabled && response.clicked()
}

/// High contrast's outline around a badge (UI-014).
fn outline(ui: &egui::Ui, c: egui::Pos2) {
    let border = theme::look(ui.ctx()).control_border();
    if border != Stroke::NONE {
        let rect = egui::Rect::from_center_size(c, egui::vec2(BADGE, BADGE));
        ui.painter()
            .rect_stroke(rect, 8, border, StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Pos2};

    use super::*;
    use crate::ui::drag::tests::{Frames, button};

    /// A reset badge answers clicks only while something differs.
    #[test]
    fn a_reset_badge_clicks_only_when_dirty() {
        for dirty in [false, true] {
            let mut frames = Frames::new();
            let mut clicked = false;
            let mut frame = |frames: &mut Frames, events: Vec<Event>| {
                frames.run(events, |ui| {
                    clicked |= reset_badge(ui, Id::new("reset"), dirty, "Reset");
                });
            };
            frame(&mut frames, Vec::new());
            frame(&mut frames, Vec::new());
            let rect = frames.ctx.read_response(Id::new("reset")).unwrap().rect;
            let at: Pos2 = rect.center();
            frame(&mut frames, vec![Event::PointerMoved(at), button(at, true)]);
            frame(&mut frames, vec![button(at, false)]);
            assert_eq!(clicked, dirty);
        }
    }
}
