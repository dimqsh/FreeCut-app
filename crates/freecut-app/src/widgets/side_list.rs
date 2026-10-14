//! The side list (ui-concept §8.2, §13.22): group headings with a
//! chevron, rows with right-aligned counts, the hairline splitter that
//! resizes it, and the chip row it turns into in narrow panels (§13.9).
//!
//! The numbers live in pure functions ([`clamp_side_width`],
//! [`side_list_visible`]); the painters only call them.

use eframe::egui::{
    self, Align2, CursorIcon, Id, Key, Pos2, Rangef, Rect, Response, Sense, Stroke, StrokeKind,
    Vec2,
};

use super::icon_button::keyboard_focus;
use super::{control_outline, focus_ring};
use crate::theme::{self, Type, fonts, icons};

/// The side list's width before the user drags its hairline.
pub const DEFAULT_WIDTH: f32 = 112.0;
/// The narrowest side list.
pub const MIN_WIDTH: f32 = 80.0;
/// Below this panel width the side list gives way to a chip row.
pub const NARROW_BELOW: f32 = 360.0;
/// Group heading and row heights.
pub const HEADING_H: f32 = 26.0;
pub const ROW_H: f32 = 24.0;
/// Padding around the list and between rows.
pub const PADDING: f32 = 4.0;
pub const ROW_GAP: f32 = 1.0;
/// Rows start past the heading's chevron.
const ROW_INDENT: f32 = 18.0;
/// The splitter's grab area on each side of its hairline.
const GRAB: f32 = 3.0;

/// The side list's width for a wanted width `w` in a panel `panel_w`
/// wide: at least [`MIN_WIDTH`], at most half the panel.
pub fn clamp_side_width(w: f32, panel_w: f32) -> f32 {
    let max = (panel_w / 2.0).max(MIN_WIDTH);
    if w.is_finite() {
        w.clamp(MIN_WIDTH, max)
    } else {
        DEFAULT_WIDTH.clamp(MIN_WIDTH, max)
    }
}

/// Whether a panel `panel_w` wide shows the side list (else the chip row).
pub fn side_list_visible(panel_w: f32) -> bool {
    panel_w >= NARROW_BELOW
}

/// A group heading: chevron and title, full width; a click folds it.
pub fn heading(ui: &mut egui::Ui, id: Id, title: &str, open: bool) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADING_H), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            hovered,
            ui.style().animation_time,
        );
        if t > 0.0 {
            ui.painter()
                .rect_filled(rect, theme::R_SM, p.bg_field.gamma_multiply(t));
        }
        let color = if hovered {
            p.text_primary
        } else {
            p.text_secondary
        };
        let chevron = if open {
            icons::CARET_DOWN
        } else {
            icons::CARET_RIGHT
        };
        ui.painter().text(
            Pos2::new(rect.left() + 6.0, rect.center().y),
            Align2::LEFT_CENTER,
            chevron,
            fonts::icon_font(ui.ctx(), 10.0, false),
            color,
        );
        let font = Type::Caption.style(ui.style()).resolve(ui.style());
        let galley = one_line(ui, title, font, color, rect.width() - 20.0 - 6.0);
        ui.painter().galley(
            Pos2::new(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        if keyboard_focus(ui, &response) {
            focus_ring(ui, rect, theme::R_SM);
        }
    }
    let label = title.to_owned();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, &label)
    });
    response
}

/// A row: label at the row indent, the count right-aligned (hidden when
/// zero). Idle transparent, hover `bg.field`, current `bg.active`.
pub fn row(ui: &mut egui::Ui, id: Id, label: &str, count: usize, current: bool) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let t =
            ui.ctx()
                .animate_bool_with_time(id.with("hover"), hovered, ui.style().animation_time);
        if current {
            ui.painter().rect_filled(rect, theme::R_SM, p.bg_active);
        } else if t > 0.0 {
            ui.painter()
                .rect_filled(rect, theme::R_SM, p.bg_field.gamma_multiply(t));
        }
        let color = if current || hovered {
            p.text_primary
        } else {
            p.text_secondary
        };
        let mut right = rect.right() - 6.0;
        if count > 0 {
            let font = Type::Micro.style(ui.style()).resolve(ui.style());
            let galley = ui
                .painter()
                .layout_no_wrap(count.to_string(), font, p.text_tertiary);
            let at = Pos2::new(
                right - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            );
            right = at.x - 6.0;
            ui.painter().galley(at, galley, p.text_tertiary);
        }
        let font = egui::TextStyle::Small.resolve(ui.style());
        let left = rect.left() + ROW_INDENT;
        let galley = one_line(ui, label, font, color, (right - left).max(8.0));
        ui.painter().galley(
            Pos2::new(left, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        if keyboard_focus(ui, &response) {
            focus_ring(ui, rect, theme::R_SM);
        }
    }
    // Accessibility: "Video, 6 items" (§13.22).
    let name = match count {
        0 => format!("{label}, no items"),
        1 => format!("{label}, 1 item"),
        n => format!("{label}, {n} items"),
    };
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &name));
    response
}

/// Moves the current row with `Up` / `Down` while one of the rows has
/// focus: returns the row to pick and focuses it.
pub fn key_step(ui: &egui::Ui, focused: bool, current: usize, len: usize) -> Option<usize> {
    if !focused || len == 0 {
        return None;
    }
    let (up, down) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
    if down && current + 1 < len {
        Some(current + 1)
    } else if up && current > 0 {
        Some(current - 1)
    } else {
        None
    }
}

/// The hairline between the side list and the grid at `x`, over `span`:
/// 1 pt `border.subtle`, `accent` on hover or drag, with a ±3 pt grab
/// area. Dragging changes `width` (kept within [`clamp_side_width`] of
/// `panel_w`); a double-click resets it.
pub fn splitter(ui: &mut egui::Ui, id: Id, x: f32, span: Rangef, width: &mut f32, panel_w: f32) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let grab = Rect::from_x_y_ranges(Rangef::new(x - GRAB, x + GRAB), span);
    let response = ui
        .interact(grab, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal);
    if response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        *width = clamp_side_width(*width + response.drag_delta().x, panel_w);
    }
    if response.double_clicked() {
        *width = clamp_side_width(DEFAULT_WIDTH, panel_w);
    }
    let hot = response.hovered() || response.dragged();
    let t = ui
        .ctx()
        .animate_bool_with_time(id.with("hot"), hot, ui.style().animation_time);
    let line = Rect::from_x_y_ranges(Rangef::new(x - 0.5, x + 0.5), span);
    ui.painter().rect_filled(line, 0.0, p.border_subtle);
    if t > 0.0 {
        ui.painter()
            .rect_filled(line, 0.0, p.accent.gamma_multiply(t));
    }
}

/// A chip (§13.9): a 24 pt pill in `bg.field` with a `type.caption`
/// label; selected = `overlay.selection` with `accent.text`.
pub fn chip(ui: &mut egui::Ui, label: &str, selected: bool) -> Response {
    chip_with(ui, None, label, selected)
}

/// [`chip`] under its own `id`, so it keeps its state (and tests find it)
/// however the row around it changes.
pub fn chip_at(ui: &mut egui::Ui, id: egui::Id, label: &str, selected: bool) -> Response {
    chip_with(ui, Some(id), label, selected)
}

fn chip_with(ui: &mut egui::Ui, id: Option<egui::Id>, label: &str, selected: bool) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let font = Type::Caption.style(ui.style()).resolve(ui.style());
    let color = if selected {
        p.accent_text
    } else {
        p.text_secondary
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    let size = Vec2::new(galley.size().x + 20.0, 24.0);
    let (rect, response) = match id {
        None => ui.allocate_exact_size(size, Sense::click()),
        Some(id) => {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            (rect, ui.interact(rect, id, Sense::click()))
        }
    };
    if ui.is_rect_visible(rect) {
        let radius = 12;
        let fill = if selected {
            p.overlay_selection()
        } else if response.hovered() {
            p.bg_hover
        } else {
            p.bg_field
        };
        ui.painter().rect_filled(rect, radius, fill);
        control_outline(ui, rect, radius);
        let text = if response.hovered() && !selected {
            p.text_primary
        } else {
            color
        };
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, text);
        if keyboard_focus(ui, &response) {
            ui.painter().rect_stroke(
                rect.expand(1.0),
                radius,
                Stroke::new(p.focus_width, p.accent),
                StrokeKind::Outside,
            );
        }
    }
    let name = label.to_owned();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &name)
    });
    response
}

/// `text` on one line, cut with "…" to fit `width`.
pub fn one_line(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_width_stays_between_80_and_half_the_panel() {
        assert_eq!(clamp_side_width(112.0, 400.0), 112.0);
        assert_eq!(clamp_side_width(40.0, 400.0), MIN_WIDTH);
        assert_eq!(clamp_side_width(300.0, 400.0), 200.0);
        // A panel too narrow for half to reach the minimum keeps the
        // minimum (the list is hidden there anyway).
        assert_eq!(clamp_side_width(300.0, 120.0), MIN_WIDTH);
        assert_eq!(clamp_side_width(f32::NAN, 400.0), DEFAULT_WIDTH);
    }

    #[test]
    fn side_list_hides_below_360_pt() {
        assert!(side_list_visible(360.0));
        assert!(side_list_visible(800.0));
        assert!(!side_list_visible(359.9));
        assert!(!side_list_visible(0.0));
    }
}
