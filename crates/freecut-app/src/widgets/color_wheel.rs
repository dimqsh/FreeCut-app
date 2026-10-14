//! A colour wheel (CL-015): a disk of hues around a grey centre and a puck
//! on it. Dragging moves the puck with the pointer (Shift moves it ten
//! times slower, for fine work); pressing away from the puck first brings
//! it there; double-clicking centres it. The model keeps the puck on the
//! disk ([`freecut_timeline::model::Wheel`]); the widget only turns pointer
//! movement into puck units (−1000 to +1000, x along the blue colour
//! difference, y along the red one, as on a vectorscope).
//!
//! The rim shows the colour a puck in that direction adds, worked out by
//! the renderer's own [`puck_vector`], so the disk and the picture agree.

use eframe::egui::{self, Color32, Id, Pos2, Rect, Response, Sense, Stroke, WidgetInfo};
use freecut_render::color::puck_vector;
use freecut_timeline::model::PUCK_RADIUS;

use crate::theme;

/// How many slices the disk is drawn with.
const SLICES: usize = 72;
/// How strongly the rim shows its colour (a puck at the rim adds far
/// less; the disk exaggerates it so the hues read).
const RIM_TINT: f32 = 0.7;
/// Fine drag: this share of the pointer's movement.
const FINE: f32 = 0.1;

/// What the wheel did this frame.
#[derive(Debug)]
pub struct WheelOutput {
    /// The puck after this frame's drag or double-click; `None` when
    /// unchanged.
    pub puck: Option<(i16, i16)>,
    /// Drives undo coalescing: one entry per drag.
    pub response: Response,
}

/// Puck units per point of a disk of radius `r` points.
fn units_per_point(r: f32) -> f32 {
    f32::from(PUCK_RADIUS) / r.max(1.0)
}

/// Where puck `(x, y)` is drawn on a disk centred at `c` with radius `r`
/// (y up).
pub fn puck_pos(c: Pos2, r: f32, (x, y): (i16, i16)) -> Pos2 {
    let k = 1.0 / units_per_point(r);
    egui::pos2(c.x + f32::from(x) * k, c.y - f32::from(y) * k)
}

/// The puck at screen point `p` of the disk (not yet pulled onto it).
pub fn puck_at(c: Pos2, r: f32, p: Pos2) -> (f32, f32) {
    let k = units_per_point(r);
    ((p.x - c.x) * k, (c.y - p.y) * k)
}

/// `v` rounded into the `i16` range.
fn unit(v: f32) -> i16 {
    // Clamped to the i16 range first, so the cast is exact.
    v.round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

/// The display colour of a puck at `(x, y)` added to mid-grey.
fn tint(x: f32, y: f32, strength: f32) -> Color32 {
    let v = puck_vector(unit(x), unit(y));
    let [r, g, b] = v.map(|c| ((0.5 + c * strength).clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// A wheel of side `side` points showing `puck`.
pub fn color_wheel(
    ui: &mut egui::Ui,
    id: Id,
    label: &str,
    puck: (i16, i16),
    side: f32,
) -> WheelOutput {
    let p = theme::look(ui.ctx()).palette;
    let (outer, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    // Interacted under `id`, so tests and focus can find it.
    let response = ui.interact(outer, id, Sense::click_and_drag());
    let c = outer.center();
    let r = side / 2.0 - 3.0;
    response.widget_info(|| WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), label));
    let mut out = WheelOutput {
        puck: None,
        response: response.clone(),
    };
    let held_key = id.with("held");
    let enabled = ui.is_enabled();
    if enabled && response.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin());
        let current = puck_pos(c, r, puck);
        let start = match origin {
            // Pressed away from the puck: it jumps there first.
            Some(at) if at.distance(current) > 8.0 => {
                let (x, y) = puck_at(c, r, at);
                on_disk_f(x, y)
            }
            _ => (f32::from(puck.0), f32::from(puck.1)),
        };
        ui.data_mut(|d| d.insert_temp(held_key, start));
        if start != (f32::from(puck.0), f32::from(puck.1)) {
            out.puck = Some((unit(start.0), unit(start.1)));
        }
    }
    if enabled && response.dragged() {
        let held: Option<(f32, f32)> = ui.data(|d| d.get_temp(held_key));
        if let Some((hx, hy)) = held {
            let delta = response.drag_delta();
            let k = units_per_point(r)
                * if ui.input(|i| i.modifiers.shift) {
                    FINE
                } else {
                    1.0
                };
            let next = on_disk_f(hx + delta.x * k, hy - delta.y * k);
            ui.data_mut(|d| d.insert_temp(held_key, next));
            let wanted = (unit(next.0), unit(next.1));
            if wanted != puck {
                out.puck = Some(wanted);
            }
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<(f32, f32)>(held_key));
    }
    if enabled && response.double_clicked() && puck != (0, 0) {
        out.puck = Some((0, 0));
    }

    if ui.is_rect_visible(outer) {
        let painter = ui.painter_at(outer.expand(2.0));
        let mut mesh = egui::Mesh::default();
        let grey = Color32::from_gray(128);
        mesh.colored_vertex(c, grey);
        let rim = f32::from(PUCK_RADIUS);
        for k in 0..=SLICES {
            let a = k as f32 / SLICES as f32 * std::f32::consts::TAU;
            let (s, co) = a.sin_cos();
            let at = egui::pos2(c.x + co * r, c.y - s * r);
            mesh.colored_vertex(at, tint(co * rim, s * rim, RIM_TINT));
        }
        for k in 0..SLICES as u32 {
            mesh.add_triangle(0, k + 1, k + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
        let ring = if response.hovered() || response.dragged() {
            p.border_strong
        } else {
            p.border_control
        };
        painter.circle_stroke(c, r, Stroke::new(1.0, ring));
        // Cross hairs through the centre.
        let hair = Stroke::new(1.0, Color32::from_black_alpha(60));
        painter.line_segment([egui::pos2(c.x - r, c.y), egui::pos2(c.x + r, c.y)], hair);
        painter.line_segment([egui::pos2(c.x, c.y - r), egui::pos2(c.x, c.y + r)], hair);
        let shown = out.puck.unwrap_or(puck);
        let at = clamp_to(c, r, puck_pos(c, r, shown));
        painter.circle(
            at,
            5.5,
            tint(f32::from(shown.0), f32::from(shown.1), 0.8),
            Stroke::new(2.0, Color32::WHITE),
        );
        painter.circle_stroke(at, 7.0, Stroke::new(1.0, Color32::from_black_alpha(140)));
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(
                ui,
                Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r)),
                255,
            );
        }
    }
    out
}

/// `(x, y)` in puck units pulled onto the disk, so dragging back from past
/// the rim moves the puck at once.
fn on_disk_f(x: f32, y: f32) -> (f32, f32) {
    let rim = f32::from(PUCK_RADIUS);
    let len = x.hypot(y);
    if len <= rim {
        (x, y)
    } else {
        (x * rim / len, y * rim / len)
    }
}

/// `p` pulled onto the disk at `c` of radius `r`.
fn clamp_to(c: Pos2, r: f32, p: Pos2) -> Pos2 {
    let d = p - c;
    if d.length() <= r {
        p
    } else {
        c + d.normalized() * r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puck_positions_round_trip_with_y_up() {
        let c = egui::pos2(100.0, 100.0);
        let r = 50.0;
        assert_eq!(puck_pos(c, r, (1000, 0)), egui::pos2(150.0, 100.0));
        assert_eq!(puck_pos(c, r, (0, 1000)), egui::pos2(100.0, 50.0));
        let (x, y) = puck_at(c, r, egui::pos2(75.0, 125.0));
        assert_eq!((unit(x), unit(y)), (-500, -500));
        assert_eq!(unit(1e9), i16::MAX);
    }

    #[test]
    fn the_rim_shows_the_colour_a_puck_adds() {
        // +x adds blue, +y adds red (as on a vectorscope).
        let blue = tint(1000.0, 0.0, RIM_TINT);
        assert!(blue.b() > blue.r() && blue.b() > blue.g());
        let red = tint(0.0, 1000.0, RIM_TINT);
        assert!(red.r() > red.g() && red.r() > red.b());
        assert_eq!(tint(0.0, 0.0, RIM_TINT), Color32::from_gray(128));
    }
}
