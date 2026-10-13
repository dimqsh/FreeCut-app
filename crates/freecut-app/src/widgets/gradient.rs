//! The gradient stop bar (TX-014, ui-concept §16): the gradient painted
//! left to right, with a marker under each colour stop. Clicking a marker
//! selects its stop; dragging it moves the stop. A stop never passes its
//! neighbours while dragged (two stops may meet, making a hard edge), so
//! stop indices stay put during a drag. `Esc` during a drag reports
//! [`GradientBarOutput::cancelled`], like the slider.
//!
//! The marker is picked where the press happened (egui reports a drag only
//! once the pointer has moved a few points), and it keeps its offset from
//! the pointer, so it never jumps. Of stops that met, the drag's direction
//! picks: the upper one moving right, the lower one moving left, so met
//! stops can always be pulled apart.

use eframe::egui::{
    self, Color32, Id, Key, Mesh, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
};

use crate::theme;

/// The bar's height, and the marker strip's below it.
const BAR: f32 = 18.0;
const MARKERS: f32 = 12.0;
/// How near (pt) a press must be to a marker to grab it.
const GRAB: f32 = 8.0;

/// What the bar reports this frame.
#[derive(Debug)]
pub struct GradientBarOutput {
    /// Bar and markers: a marker drag drives undo coalescing.
    pub response: Response,
    /// A marker was pressed: select that stop.
    pub selected: Option<usize>,
    /// A marker was dragged: stop index and its new position, 0..=1.
    pub moved: Option<(usize, f64)>,
    /// `Esc` stopped a drag this frame.
    pub cancelled: bool,
}

/// One stop as the bar shows it: position 0..=1 and its sRGB colour with
/// straight alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarStop {
    pub at: f64,
    pub color: [u8; 4],
}

/// The stop under `x` (pt from the bar's left edge, bar `width` wide):
/// the nearest marker within the grab distance. Of stops at the same
/// place, the highest index when the pointer heads right (`toward` > 0),
/// else the lowest.
pub fn stop_at(stops: &[BarStop], x: f32, width: f32, toward: f32) -> Option<usize> {
    let nearest = stops
        .iter()
        .enumerate()
        .map(|(i, s)| (i, (s.at as f32 * width - x).abs()))
        .filter(|(_, d)| *d <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)?;
    let at = stops.get(nearest)?.at;
    let met = stops
        .iter()
        .enumerate()
        .filter(|(_, s)| s.at == at)
        .map(|(i, _)| i);
    if toward > 0.0 { met.max() } else { met.min() }
}

/// Draws the bar for `stops` (sorted) with stop `selected` highlighted.
pub fn gradient_bar(
    ui: &mut egui::Ui,
    id: Id,
    stops: &[BarStop],
    selected: usize,
) -> GradientBarOutput {
    let p = theme::look(ui.ctx()).palette;
    let width = ui.available_width().max(60.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, BAR + MARKERS), Sense::hover());
    let sense = if ui.is_enabled() {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let bar = Rect::from_min_size(rect.min, egui::vec2(width, BAR));
    let grabbed_id = id.with("grabbed");
    let x_of = |pos: Pos2| pos.x - bar.left();

    let mut selected_now = None;
    let mut moved = None;
    if response.drag_started() || response.clicked() {
        let now = response.interact_pointer_pos();
        // A drag is reported once the pointer has moved: grab where the
        // press was (a click's release is where it was pressed).
        let press = ui.input(|i| i.pointer.press_origin()).or(now);
        let toward = match (press, now) {
            (Some(a), Some(b)) => b.x - a.x,
            _ => 0.0,
        };
        // The stop and the marker's offset from the press.
        let grabbed = press.and_then(|pos| {
            let i = stop_at(stops, x_of(pos), width, toward)?;
            Some((i, stops.get(i)?.at as f32 * width - x_of(pos)))
        });
        ui.data_mut(|d| d.insert_temp(grabbed_id, grabbed));
        selected_now = grabbed.map(|(i, _)| i);
    }
    let grabbed = ui.data(|d| d.get_temp::<Option<(usize, f32)>>(grabbed_id).flatten());
    if (response.dragged() || response.drag_stopped())
        && let (Some((i, offset)), Some(pos)) = (grabbed, response.interact_pointer_pos())
    {
        let lo = i
            .checked_sub(1)
            .and_then(|j| stops.get(j))
            .map_or(0.0, |s| s.at);
        let hi = stops.get(i + 1).map_or(1.0, |s| s.at);
        let at = f64::from((x_of(pos) + offset) / width).clamp(lo, hi);
        if stops.get(i).is_some_and(|s| s.at != at) {
            moved = Some((i, at));
        }
    }
    let mut cancelled = false;
    if (response.dragged() || response.drag_stopped())
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        ui.ctx().stop_dragging();
        cancelled = true;
        moved = None;
    }
    if response.drag_stopped() || cancelled {
        ui.data_mut(|d| d.remove::<Option<(usize, f32)>>(grabbed_id));
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(bar, theme::R_SM, p.bg_well);
        painter.add(gradient_mesh(bar, stops));
        painter.rect_stroke(
            bar,
            theme::R_SM,
            Stroke::new(1.0, p.border_control),
            StrokeKind::Inside,
        );
        for (i, s) in stops.iter().enumerate() {
            let x = bar.left() + s.at as f32 * width;
            let top = bar.bottom() + 1.0;
            let tri = vec![
                egui::pos2(x, top),
                egui::pos2(x - 5.0, top + MARKERS - 2.0),
                egui::pos2(x + 5.0, top + MARKERS - 2.0),
            ];
            let stroke = if i == selected {
                Stroke::new(2.0, p.accent)
            } else {
                Stroke::new(1.0, p.border_control)
            };
            let [r, g, b, _] = s.color;
            painter.add(egui::Shape::convex_polygon(
                tri,
                Color32::from_rgb(r, g, b),
                stroke,
            ));
        }
    }
    GradientBarOutput {
        response,
        selected: selected_now,
        moved,
        cancelled,
    }
}

/// The gradient across `bar`: one quad per stop interval, the end colours
/// extended to the edges.
fn gradient_mesh(bar: Rect, stops: &[BarStop]) -> Mesh {
    let mut mesh = Mesh::default();
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return mesh;
    };
    let color = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let mut points: Vec<(f32, Color32)> = vec![(0.0, color(first.color))];
    points.extend(stops.iter().map(|s| (s.at as f32, color(s.color))));
    points.push((1.0, color(last.color)));
    for pair in points.windows(2) {
        let [(a, ca), (b, cb)] = [pair[0], pair[1]];
        if b <= a {
            continue;
        }
        let x0 = bar.left() + a * bar.width();
        let x1 = bar.left() + b * bar.width();
        // At most 10 quads: the index fits a u32.
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x0, bar.top()), ca);
        mesh.colored_vertex(egui::pos2(x1, bar.top()), cb);
        mesh.colored_vertex(egui::pos2(x1, bar.bottom()), cb);
        mesh.colored_vertex(egui::pos2(x0, bar.bottom()), ca);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_grabs_the_nearest_marker_within_reach() {
        let stops = [
            BarStop {
                at: 0.0,
                color: [0, 0, 0, 255],
            },
            BarStop {
                at: 0.5,
                color: [255; 4],
            },
            BarStop {
                at: 0.52,
                color: [255, 0, 0, 255],
            },
        ];
        assert_eq!(stop_at(&stops, 3.0, 200.0, 0.0), Some(0));
        assert_eq!(stop_at(&stops, 99.0, 200.0, 0.0), Some(1));
        assert_eq!(stop_at(&stops, 104.5, 200.0, 0.0), Some(2));
        assert_eq!(stop_at(&stops, 150.0, 200.0, 0.0), None);
    }

    use eframe::egui::{Event, Pos2, Vec2};

    use crate::ui::drag::tests::{Frames, button};

    fn stops(at: &[f64]) -> Vec<BarStop> {
        at.iter()
            .map(|&at| BarStop {
                at,
                color: [255; 4],
            })
            .collect()
    }

    /// Drives the bar over a press at `from` and the pointer at each of
    /// `path`, then a release; applies every move to `stops` the way the
    /// Fill card does. Returns the grabbed stop.
    fn drag(stops: &mut [BarStop], from_at: f64, offset: f32, path: &[f32]) -> Option<usize> {
        /// One frame; returns the bar's rectangle and the stop it selected.
        fn frame(
            frames: &mut Frames,
            stops: &mut [BarStop],
            events: Vec<Event>,
        ) -> (Rect, Option<usize>) {
            let mut shown = (Rect::NOTHING, None);
            frames.run(events, |ui| {
                let out = gradient_bar(ui, Id::new("bar"), stops, 0);
                shown = (out.response.rect, out.selected);
                if let Some((i, at)) = out.moved {
                    stops[i].at = at;
                }
            });
            shown
        }
        let mut frames = Frames::new();
        let (rect, _) = frame(&mut frames, stops, Vec::new());
        let press = Pos2::new(
            rect.left() + from_at as f32 * rect.width() + offset,
            rect.top() + 4.0,
        );
        let (_, mut grabbed) = frame(
            &mut frames,
            stops,
            vec![Event::PointerMoved(press), button(press, true)],
        );
        let mut last = press;
        for dx in path {
            last = press + Vec2::new(*dx, 0.0);
            let (_, picked) = frame(&mut frames, stops, vec![Event::PointerMoved(last)]);
            grabbed = grabbed.or(picked);
        }
        frame(&mut frames, stops, vec![button(last, false)]);
        grabbed
    }

    /// S1: a quick flick (12 pt in one frame, past egui's 6 pt click
    /// distance and the 8 pt grab reach) still moves the marker pressed on,
    /// by exactly the pointer's travel.
    #[test]
    fn a_quick_drag_moves_the_marker_that_was_pressed() {
        let mut s = stops(&[0.0, 0.5, 1.0]);
        assert_eq!(drag(&mut s, 0.5, 0.0, &[12.0]), Some(1));
        assert!(s[1].at > 0.5, "moved right: {}", s[1].at);
        // Pressed 3 pt off the marker: it keeps that offset, no jump.
        let mut t = stops(&[0.0, 0.5, 1.0]);
        drag(&mut t, 0.5, 3.0, &[12.0]);
        assert!(
            (t[1].at - s[1].at).abs() < 1e-6,
            "{} vs {}",
            t[1].at,
            s[1].at
        );
    }

    /// S1: with two stops 7 pt apart, the one pressed on moves, not its
    /// neighbour.
    #[test]
    fn dragging_one_of_two_close_stops_moves_that_one() {
        let mut s = stops(&[0.0, 0.5, 0.52, 1.0]);
        assert_eq!(drag(&mut s, 0.5, 0.0, &[-12.0]), Some(1));
        assert!(s[1].at < 0.5, "stop 1 moved left: {}", s[1].at);
        assert_eq!(s[2].at, 0.52, "its neighbour stays");
    }

    /// S1: stops that met at 0 % come apart by dragging right (the upper
    /// one), and at 100 % by dragging left (the lower one).
    #[test]
    fn met_stops_can_be_pulled_apart() {
        let mut s = stops(&[0.0, 0.0, 1.0]);
        assert_eq!(drag(&mut s, 0.0, 0.0, &[4.0, 20.0]), Some(1));
        assert!(s[1].at > 0.0 && s[0].at == 0.0, "{s:?}");
        let mut s = stops(&[0.0, 1.0, 1.0]);
        assert_eq!(drag(&mut s, 1.0, 0.0, &[-4.0, -20.0]), Some(1));
        assert!(s[1].at < 1.0 && s[2].at == 1.0, "{s:?}");
    }
}
