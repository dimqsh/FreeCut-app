//! Geometry of the player's on-canvas interaction (TR-007 subset): which
//! clip is under the pointer, where a clip's box is, and where a move drag
//! puts it. Pure functions over the preview's [`LayerBox`]es; floats only,
//! this is the UI edge.

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::CanvasPos;

use crate::preview::LayerBox;

/// The canvas as the player shows it: its size, and where each visible
/// clip landed in the latest preview frame (bottom to top).
#[derive(Debug, Clone, Copy)]
pub struct CanvasView<'a> {
    pub canvas: (u32, u32),
    pub layers: &'a [LayerBox],
}

impl CanvasView<'_> {
    /// The box of `clip`, if it was visible in the latest frame.
    pub fn layer(&self, clip: ClipId) -> Option<&LayerBox> {
        self.layers.iter().rev().find(|l| l.clip == clip)
    }
}

/// Maps a canvas point into `layer`'s own coordinates (`None` when the
/// layer is degenerate, for example at 1 % scale rounded to nothing).
fn to_layer(layer: &LayerBox, p: DVec2) -> Option<DVec2> {
    let det = layer.to_canvas.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    Some(layer.to_canvas.inverse().transform_point2(p))
}

/// Whether canvas point `p` falls inside `layer`'s transformed box.
pub fn contains(layer: &LayerBox, p: DVec2) -> bool {
    to_layer(layer, p)
        .is_some_and(|q| q.x >= 0.0 && q.y >= 0.0 && q.x <= layer.size.0 && q.y <= layer.size.1)
}

/// The topmost layer under canvas point `p` that `pickable` accepts
/// (TR-007: top track first; transparent clips and clips on locked tracks
/// are left out by the caller's filter).
pub fn hit_test(
    layers: &[LayerBox],
    p: DVec2,
    pickable: impl Fn(&LayerBox) -> bool,
) -> Option<ClipId> {
    layers
        .iter()
        .rev()
        .filter(|l| l.opacity > 0.0 && pickable(l))
        .find(|l| contains(l, p))
        .map(|l| l.clip)
}

/// The four corners of `layer`'s box in canvas pixels, clockwise from the
/// top-left of the untransformed box.
pub fn corners(layer: &LayerBox) -> [DVec2; 4] {
    let (w, h) = layer.size;
    let m: DMat3 = layer.to_canvas;
    [
        m.transform_point2(DVec2::new(0.0, 0.0)),
        m.transform_point2(DVec2::new(w, 0.0)),
        m.transform_point2(DVec2::new(w, h)),
        m.transform_point2(DVec2::new(0.0, h)),
    ]
}

/// A screen point in canvas pixels, for a canvas of `canvas` pixels shown
/// in the screen rectangle `min .. min + size`.
pub fn screen_to_canvas(screen: DVec2, min: DVec2, size: DVec2, canvas: (u32, u32)) -> DVec2 {
    let scale = DVec2::new(
        f64::from(canvas.0) / size.x.max(1e-9),
        f64::from(canvas.1) / size.y.max(1e-9),
    );
    (screen - min) * scale
}

/// The inverse of [`screen_to_canvas`].
pub fn canvas_to_screen(p: DVec2, min: DVec2, size: DVec2, canvas: (u32, u32)) -> DVec2 {
    min + p * DVec2::new(
        size.x / f64::from(canvas.0.max(1)),
        size.y / f64::from(canvas.1.max(1)),
    )
}

/// Where a move drag puts a clip that started at `start`, after the
/// pointer moved `delta` canvas pixels. With `constrain` (Shift, TR-007)
/// only the larger of the two axes moves.
pub fn moved_position(
    start: CanvasPos,
    delta: DVec2,
    canvas: (u32, u32),
    constrain: bool,
) -> CanvasPos {
    let delta = if constrain {
        if delta.x.abs() >= delta.y.abs() {
            DVec2::new(delta.x, 0.0)
        } else {
            DVec2::new(0.0, delta.y)
        }
    } else {
        delta
    };
    let (x, y) = start.to_px(canvas.0, canvas.1);
    CanvasPos::from_px(x + delta.x, y + delta.y, canvas.0, canvas.1)
}

#[cfg(test)]
mod tests {
    use freecut_timeline::ids::{ClipId, TrackId};
    use freecut_timeline::model::{Angle, Scale, Transform};
    use freecut_timeline::plan::clip_placement;

    use super::*;

    fn ids(n: u64) -> (ClipId, TrackId) {
        let raw = std::num::NonZeroU64::new(n).unwrap();
        (ClipId::from_raw(raw), TrackId::from_raw(raw))
    }

    /// A text-like box of `size` centred on its clip's position.
    fn text_box(n: u64, transform: &Transform, size: (f64, f64)) -> LayerBox {
        let (clip, track) = ids(n);
        LayerBox {
            clip,
            track,
            to_canvas: clip_placement(transform, (1920, 1080))
                * DMat3::from_translation(-DVec2::new(size.0, size.1) / 2.0),
            size,
            opacity: 1.0,
        }
    }

    #[test]
    fn the_topmost_box_under_the_pointer_wins() {
        let lower = text_box(1, &Transform::IDENTITY, (400.0, 100.0));
        let upper = text_box(2, &Transform::IDENTITY, (100.0, 100.0));
        let layers = [lower, upper];
        let centre = DVec2::new(960.0, 540.0);
        assert_eq!(hit_test(&layers, centre, |_| true), Some(upper.clip));
        // Inside the lower box only.
        assert_eq!(
            hit_test(&layers, DVec2::new(800.0, 540.0), |_| true),
            Some(lower.clip)
        );
        assert_eq!(hit_test(&layers, DVec2::new(10.0, 10.0), |_| true), None);
        // A filtered-out (locked) or transparent upper layer is skipped.
        assert_eq!(
            hit_test(&layers, centre, |l| l.clip != upper.clip),
            Some(lower.clip)
        );
        let clear = LayerBox {
            opacity: 0.0,
            ..upper
        };
        assert_eq!(
            hit_test(&[lower, clear], centre, |_| true),
            Some(lower.clip)
        );
    }

    #[test]
    fn hit_testing_follows_rotation_scale_and_position() {
        // A wide box turned 90°: tall on screen.
        let turned = Transform {
            rotation: Angle::from_degrees(90),
            ..Transform::IDENTITY
        };
        let layer = text_box(1, &turned, (400.0, 50.0));
        assert!(contains(&layer, DVec2::new(960.0, 540.0 + 180.0)));
        assert!(!contains(&layer, DVec2::new(960.0 + 180.0, 540.0)));

        // Moved right by a quarter canvas and doubled.
        let moved = Transform {
            position: CanvasPos::new(250_000, 0),
            scale: Scale::from_percent(200),
            ..Transform::IDENTITY
        };
        let layer = text_box(1, &moved, (100.0, 100.0));
        assert!(contains(&layer, DVec2::new(1440.0 + 95.0, 540.0)));
        assert!(!contains(&layer, DVec2::new(1440.0 + 105.0, 540.0)));
        assert!(!contains(&layer, DVec2::new(960.0, 540.0)));
        let c = corners(&layer);
        assert!((c[0] - DVec2::new(1340.0, 440.0)).length() < 1e-9);
        assert!((c[2] - DVec2::new(1540.0, 640.0)).length() < 1e-9);
    }

    #[test]
    fn a_degenerate_box_is_never_hit() {
        let (clip, track) = ids(1);
        let flat = LayerBox {
            clip,
            track,
            to_canvas: DMat3::from_scale(DVec2::new(0.0, 1.0)),
            size: (10.0, 10.0),
            opacity: 1.0,
        };
        assert!(!contains(&flat, DVec2::ZERO));
    }

    #[test]
    fn screen_and_canvas_points_round_trip() {
        let (min, size) = (DVec2::new(100.0, 50.0), DVec2::new(960.0, 540.0));
        let p = screen_to_canvas(DVec2::new(580.0, 320.0), min, size, (1920, 1080));
        assert!((p - DVec2::new(960.0, 540.0)).length() < 1e-9);
        let back = canvas_to_screen(p, min, size, (1920, 1080));
        assert!((back - DVec2::new(580.0, 320.0)).length() < 1e-9);
    }

    #[test]
    fn a_move_drag_adds_pixels_and_shift_keeps_one_axis() {
        let canvas = (1920, 1080);
        let start = CanvasPos::new(100_000, 0); // 192 px right
        let moved = moved_position(start, DVec2::new(96.0, -54.0), canvas, false);
        assert_eq!(moved, CanvasPos::new(150_000, -50_000));
        let h = moved_position(start, DVec2::new(96.0, -54.0), canvas, true);
        assert_eq!(h, CanvasPos::new(150_000, 0));
        let v = moved_position(start, DVec2::new(10.0, -54.0), canvas, true);
        assert_eq!(v, CanvasPos::new(100_000, -50_000));
        // Clamped to ±4 canvas sizes.
        let far = moved_position(start, DVec2::new(1e9, 0.0), canvas, false);
        assert_eq!(far.x, CanvasPos::LIMIT);
    }
}
