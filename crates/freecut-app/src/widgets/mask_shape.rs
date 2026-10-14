//! FreeCut's own glyphs for the mask shapes (CL-031, TR-017), painted
//! with strokes so they need no icon font, and the shape picker built
//! from them: a segmented track (§13.4's look) with one glyph per shape.
//!
//! The glyphs: Linear is an edge with the kept side shaded above it, Band
//! two parallel edges with the strip between shaded, Ellipse an oval and
//! Rectangle a box with rounded corners.

use eframe::egui::{self, Id, Rect, Sense, Stroke, StrokeKind, WidgetInfo, WidgetType};
use freecut_timeline::model::MaskShapeKind;

use crate::theme;
use crate::ui::look::{mask_shape_hint, mask_shape_label};

/// Paints `kind`'s glyph centred in `rect` in `color`.
pub fn paint_glyph(painter: &egui::Painter, rect: Rect, kind: MaskShapeKind, color: egui::Color32) {
    let side = rect.width().min(rect.height());
    let g = Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let shade = color.gamma_multiply(0.3);
    let stroke = Stroke::new(1.5, color);
    match kind {
        MaskShapeKind::Linear => {
            let mid = g.center().y + side * 0.1;
            let kept = Rect::from_min_max(
                egui::pos2(g.left(), g.top() + side * 0.15),
                egui::pos2(g.right(), mid),
            );
            painter.rect_filled(kept, 1.0, shade);
            painter.line_segment(
                [egui::pos2(g.left(), mid), egui::pos2(g.right(), mid)],
                stroke,
            );
        }
        MaskShapeKind::Band => {
            let (top, bottom) = (g.top() + side * 0.3, g.bottom() - side * 0.3);
            painter.rect_filled(Rect::from_x_y_ranges(g.x_range(), top..=bottom), 0.0, shade);
            for y in [top, bottom] {
                painter.line_segment([egui::pos2(g.left(), y), egui::pos2(g.right(), y)], stroke);
            }
        }
        MaskShapeKind::Ellipse => {
            let radius = egui::vec2(side * 0.5, side * 0.36);
            painter.add(egui::Shape::ellipse_filled(g.center(), radius, shade));
            painter.add(egui::Shape::ellipse_stroke(g.center(), radius, stroke));
        }
        MaskShapeKind::Rectangle => {
            let r = Rect::from_center_size(g.center(), egui::vec2(side, side * 0.72));
            painter.rect(r, side * 0.18, shade, stroke, StrokeKind::Inside);
        }
    }
}

/// What the picker did this frame.
pub struct ShapePicked {
    /// A shape other than the current one was clicked.
    pub picked: Option<MaskShapeKind>,
}

/// The shape picker: four glyph cells in a well, the current one on the
/// thumb; each names its shape in its tooltip and to screen readers.
pub fn shape_picker(ui: &mut egui::Ui, id: Id, current: MaskShapeKind) -> ShapePicked {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let height = look.metrics.control_height;
    let width = ui.available_width().min(4.0 * 44.0);
    let (track, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::RadioGroup,
            ui.is_enabled(),
            mask_shape_label(current),
        )
    });
    let painter = ui.painter().clone();
    painter.rect(
        track,
        theme::R_MD,
        p.bg_well,
        Stroke::new(1.0, p.border_subtle),
        StrokeKind::Inside,
    );
    super::control_outline(ui, track, theme::R_MD);
    let cells = super::segmented::segment_rects(track, &[1.0; 4]);
    let enabled = ui.is_enabled();
    let mut picked = None;
    for (kind, cell) in MaskShapeKind::ALL.into_iter().zip(cells) {
        let r = ui.interact(cell, id.with(kind.as_str()), Sense::click());
        let selected = kind == current;
        if selected {
            if look.kind == theme::ThemeKind::Light {
                painter.rect(
                    cell,
                    theme::R_SM,
                    p.bg_card,
                    Stroke::new(1.0, p.border_subtle),
                    StrokeKind::Inside,
                );
            } else {
                painter.rect_filled(cell, theme::R_SM, p.bg_hover);
            }
            super::control_outline(ui, cell, theme::R_SM);
        }
        let color = if !enabled {
            p.text_disabled
        } else if selected || r.hovered() {
            p.text_primary
        } else {
            p.text_secondary
        };
        paint_glyph(&painter, cell.shrink2(egui::vec2(8.0, 5.0)), kind, color);
        let label = mask_shape_label(kind);
        r.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, label));
        let r = r.on_hover_text(format!("{label} — {}", mask_shape_hint(kind)));
        if r.clicked() && !selected {
            picked = Some(kind);
        }
    }
    ShapePicked { picked }
}
