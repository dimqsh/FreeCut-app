//! The timeline's sequence tabs (ui-concept §13.26): the panel's header, a
//! `bg.well` strip with a hairline at its bottom and one tab per sequence
//! standing on it, the current one in `bg.panel` so it joins the toolbar
//! below.
//!
//! A project has one sequence today and the timeline model has no
//! operations to add, rename, reorder or close sequences, so the strip is
//! read-only: one tab, no `+` and no close button (R5 plan). Those come
//! with TL's sequence operations, each through the undo stack.

use eframe::egui::{self, CornerRadius, Rect, Sense, Stroke, Vec2};

use crate::theme;

/// Room left of the first tab, and between the label and the tab's edges.
const PAD: f32 = 8.0;
const LABEL_INSET: f32 = 10.0;
/// Labels are cut at this width.
const MAX_LABEL: f32 = 140.0;

/// The name the one tab shows.
pub const SEQUENCE_NAME: &str = "Sequence 1";

/// The tab rectangle for a label `label_w` wide in a strip at `strip`: it
/// stands on the strip's floor, 2 pt shorter than the strip.
pub fn tab_rect(strip: Rect, label_w: f32) -> Rect {
    let w = label_w.min(MAX_LABEL) + 2.0 * LABEL_INSET;
    let h = (strip.height() - 2.0).max(0.0);
    Rect::from_min_size(
        egui::pos2(strip.left() + PAD, strip.bottom() - h),
        Vec2::new(w, h),
    )
}

/// Paints the strip in `rect` with its one tab.
pub fn show(ui: &mut egui::Ui, rect: Rect) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, p.bg_well);
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, p.border_subtle),
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = painter.layout_no_wrap(SEQUENCE_NAME.to_owned(), font, p.text_primary);
    let tab = tab_rect(rect, galley.size().x);
    let radius = CornerRadius {
        nw: theme::R_SM,
        ne: theme::R_SM,
        sw: 0,
        se: 0,
    };
    // The current tab covers the hairline: it is one surface with the
    // toolbar.
    painter.rect_filled(tab, radius, p.bg_panel);
    let border = look.control_border();
    if border != Stroke::NONE {
        painter.rect_stroke(tab, radius, border, egui::StrokeKind::Inside);
    }
    let text_clip =
        Rect::from_min_max(tab.min, egui::pos2(tab.right() - LABEL_INSET, tab.bottom()));
    painter.with_clip_rect(text_clip).galley(
        egui::pos2(
            tab.left() + LABEL_INSET,
            tab.center().y - galley.size().y / 2.0,
        ),
        galley,
        p.text_primary,
    );
    let response = ui.interact(tab, ui.id().with("sequence-tab"), Sense::hover());
    response.on_hover_text("The project's timeline");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact geometry

    use super::*;

    #[test]
    fn the_tab_stands_on_the_strip_floor_and_cuts_long_labels() {
        let strip = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(600.0, 32.0));
        let tab = tab_rect(strip, 60.0);
        assert_eq!(tab.bottom(), 32.0);
        assert_eq!(tab.height(), 30.0);
        assert_eq!(tab.left(), 8.0);
        assert_eq!(tab.width(), 80.0);
        assert_eq!(tab_rect(strip, 500.0).width(), 160.0, "cut at 140 pt");
    }
}
