//! The curve editor (CL-014): a square graph of one curve, painted with
//! egui's painter. Click the curve area to add a point, drag a point to
//! move it (Shift keeps it on its vertical), double-click a point or drag it
//! out of the graph to remove it. The endpoints stay; the model's
//! [`Curve`] methods keep every rule (16 points at most, x at least 0.01
//! apart, the endpoint ranges), so the widget only turns pointer positions
//! into curve coordinates and back.
//!
//! The drawn line is the monotone cubic the renderer bakes
//! (`freecut_render::color::CurveTable`), so what the editor shows is what
//! the picture gets. Its histogram waits for CL-021.

use eframe::egui::{self, Color32, Id, Pos2, Rect, Response, Sense, Shape, Stroke, WidgetInfo};
use freecut_render::color::{CURVE_TABLE_SIZE, CurveTable};
use freecut_timeline::model::{Curve, CurveError, CurvePoint};

use crate::theme;

/// How close (in points) the pointer must be to grab a point.
const GRAB: f32 = 9.0;
/// How far past the graph's edge a dragged inner point is removed.
const TEAR_OFF: f32 = 28.0;
/// A point's radius.
const DOT: f32 = 4.5;

/// What the editor did this frame.
#[derive(Debug)]
pub struct CurveOutput {
    /// The curve after this frame's click or drag; `None` when unchanged.
    pub curve: Option<Curve>,
    /// The graph's response: drives undo coalescing (one entry per drag).
    pub response: Response,
    /// Why a click did nothing (a 17th point).
    pub refused: Option<&'static str>,
}

/// `pos` as curve coordinates in `rect` (x right, y up), each limited to
/// 0–10000.
pub fn to_curve(rect: Rect, pos: Pos2) -> CurvePoint {
    let one = f32::from(Curve::ONE);
    let fx = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let fy = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
    // In 0..=10000 after the clamps.
    CurvePoint::new((fx * one).round() as u16, (fy * one).round() as u16)
}

/// Where curve point `p` is drawn in `rect`.
pub fn to_screen(rect: Rect, p: CurvePoint) -> Pos2 {
    let one = f32::from(Curve::ONE);
    egui::pos2(
        rect.left() + f32::from(p.x) / one * rect.width(),
        rect.bottom() - f32::from(p.y) / one * rect.height(),
    )
}

/// The point of `curve` nearest `pos` within [`GRAB`] points.
pub fn grabbed(curve: &Curve, rect: Rect, pos: Pos2) -> Option<usize> {
    curve
        .points()
        .iter()
        .enumerate()
        .map(|(i, p)| (i, to_screen(rect, *p).distance(pos)))
        .filter(|(_, d)| *d <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// `curve` with a point added at `pos`, and the new point's index; the
/// reason when the curve can't take one there.
pub fn add_at(curve: &Curve, rect: Rect, pos: Pos2) -> Result<(Curve, usize), CurveError> {
    let p = to_curve(rect, pos);
    let added = curve.with_point(p)?;
    let index = added.points().iter().position(|q| *q == p).unwrap_or(0);
    Ok((added, index))
}

/// The editor of `curve`, drawn in `colour`, filling the available width
/// up to `max_side` points.
pub fn curve_editor(
    ui: &mut egui::Ui,
    id: Id,
    curve: &Curve,
    colour: Color32,
    max_side: f32,
) -> CurveOutput {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let side = ui.available_width().min(max_side).max(120.0);
    let (outer, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    // Interacted under `id`, so tests and focus can find it.
    let response = ui.interact(outer, id, Sense::click_and_drag());
    // A margin keeps the endpoint dots inside the frame.
    let rect = outer.shrink(DOT + 2.0);
    response.widget_info(|| {
        WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), "Curve editor")
    });
    let drag_key = id.with("dragging");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_key));
    let mut out = CurveOutput {
        curve: None,
        response: response.clone(),
        refused: None,
    };
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let enabled = ui.is_enabled();

    if enabled && response.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
        if let Some(at) = origin {
            dragging = grabbed(curve, rect, at);
            if dragging.is_none() {
                match add_at(curve, rect, at) {
                    Ok((added, index)) => {
                        dragging = Some(index);
                        out.curve = Some(added);
                    }
                    Err(CurveError::TooManyPoints) => out.refused = Some(FULL),
                    Err(_) => {}
                }
            }
        }
    }
    if enabled && response.dragged() {
        let base = out.curve.clone().unwrap_or_else(|| curve.clone());
        if let (Some(i), Some(at)) = (dragging, pointer) {
            let last = base.points().len().saturating_sub(1);
            let outside = !rect.expand(TEAR_OFF).contains(at);
            if outside && i > 0 && i < last {
                // Dragged out of the graph: the point goes (CL-014).
                if let Ok(fewer) = base.without_point(i) {
                    out.curve = Some(fewer);
                }
                dragging = None;
            } else {
                let mut to = to_curve(rect, at);
                if ui.input(|i| i.modifiers.shift)
                    && let Some(q) = base.points().get(i)
                {
                    to.x = q.x;
                }
                let moved = base.moved(i, to);
                if moved != *curve {
                    out.curve = Some(moved);
                }
            }
        }
    }
    if response.drag_stopped() {
        dragging = None;
    }
    if enabled && response.double_clicked() {
        if let Some(at) = pointer
            && let Some(i) = grabbed(curve, rect, at)
            && let Ok(fewer) = curve.without_point(i)
        {
            out.curve = Some(fewer);
        }
    } else if enabled
        && response.clicked()
        && let Some(at) = pointer
        && grabbed(curve, rect, at).is_none()
    {
        match add_at(curve, rect, at) {
            Ok((added, _)) => out.curve = Some(added),
            Err(CurveError::TooManyPoints) => out.refused = Some(FULL),
            Err(_) => {}
        }
    }
    ui.data_mut(|d| {
        if let Some(i) = dragging {
            d.insert_temp(drag_key, i);
        } else {
            d.remove::<usize>(drag_key);
        }
    });

    if ui.is_rect_visible(outer) {
        let shown = out.curve.as_ref().unwrap_or(curve);
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, theme::R_MD, p.bg_well);
        crate::widgets::control_outline(ui, outer, theme::R_MD);
        let grid = Stroke::new(1.0, p.border_subtle);
        for k in 1..4 {
            let f = k as f32 / 4.0;
            let x = rect.left() + f * rect.width();
            let y = rect.top() + f * rect.height();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                grid,
            );
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                grid,
            );
        }
        painter.line_segment(
            [rect.left_bottom(), rect.right_top()],
            Stroke::new(1.0, p.text_disabled),
        );
        let table = CurveTable::new(shown);
        let values = table.values();
        let steps = 128;
        let line: Vec<Pos2> = (0..=steps)
            .map(|k| {
                let f = k as f32 / steps as f32;
                let i = ((f * (CURVE_TABLE_SIZE - 1) as f32).round() as usize)
                    .min(CURVE_TABLE_SIZE - 1);
                egui::pos2(
                    rect.left() + f * rect.width(),
                    rect.bottom() - values[i].clamp(0.0, 1.0) * rect.height(),
                )
            })
            .collect();
        painter.add(Shape::line(line, Stroke::new(2.0, colour)));
        let hovered = pointer
            .filter(|_| response.hovered())
            .and_then(|at| grabbed(shown, rect, at));
        for (i, point) in shown.points().iter().enumerate() {
            let c = to_screen(rect, *point);
            let active = dragging == Some(i) || hovered == Some(i);
            let r = if active { DOT + 1.5 } else { DOT };
            painter.circle(c, r, p.bg_card, Stroke::new(2.0, colour));
        }
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, outer, theme::R_MD);
        }
    }
    out
}

/// The tooltip of a click that would add a 17th point (CL-014).
pub const FULL: &str = "A curve holds at most 16 points; remove one first";

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 200.0))
    }

    #[test]
    fn coordinates_round_trip_and_clamp() {
        let r = rect();
        assert_eq!(to_curve(r, r.left_bottom()), CurvePoint::new(0, 0));
        assert_eq!(to_curve(r, r.right_top()), CurvePoint::new(10_000, 10_000));
        assert_eq!(to_curve(r, r.center()), CurvePoint::new(5_000, 5_000));
        assert_eq!(
            to_curve(r, egui::pos2(-50.0, 900.0)),
            CurvePoint::new(0, 0),
            "outside the graph is its edge"
        );
        let p = CurvePoint::new(2_500, 7_500);
        assert_eq!(to_curve(r, to_screen(r, p)), p);
    }

    #[test]
    fn grabbing_and_adding_follow_the_model_rules() {
        let r = rect();
        let c = Curve::identity();
        assert_eq!(
            grabbed(&c, r, r.left_bottom() + egui::vec2(3.0, -3.0)),
            Some(0)
        );
        assert_eq!(grabbed(&c, r, r.center()), None);
        let (added, i) = add_at(&c, r, r.center() + egui::vec2(0.0, -40.0)).unwrap();
        assert_eq!((added.points().len(), i), (3, 1));
        assert_eq!(added.points()[1], CurvePoint::new(5_000, 7_000));
        // A click on an endpoint's column adds nothing.
        assert!(add_at(&c, r, r.left_bottom()).is_err());
        // The 17th point is refused.
        let mut full = Curve::identity();
        for k in 1..15u16 {
            full = full.with_point(CurvePoint::new(k * 600, 100)).unwrap();
        }
        assert_eq!(
            add_at(&full, r, to_screen(r, CurvePoint::new(9_500, 5_000))).err(),
            Some(CurveError::TooManyPoints)
        );
    }
}
