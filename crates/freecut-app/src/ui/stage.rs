//! The Stage's text handles (TX-005, TX-055): geometry of the side handles
//! that set a text box's width and of the bubble's tail-tip handle, as
//! pure functions over the preview's [`LayerBox`] (the renderer's own
//! layout, never one made on the UI thread). Floats only: this is the UI
//! edge. The player draws the handles and turns drags into edits.
//!
//! A text layer's box is `(0, 0)..size` in layout pixels (canvas pixels at
//! 100 % scale); `to_canvas` places it. One ref px is `scale_px` layout
//! pixels ([`freecut_timeline::plan::ref_px_scale`]).

use freecut_timeline::glam::DVec2;
use freecut_timeline::model::{RefPx, TailTip, TextClipData, TextLayout, TextStyle};

use super::box_cards::kind_has_tail;
use super::rows::ref_px;
use crate::preview::LayerBox;

/// How far the tail tip may go from the box centre (TX-055), in em.
pub const TIP_REACH_EM: f64 = 40.0;

/// A handle on a selected text clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// The box's left edge: width, the right edge stays (TX-005).
    Left,
    /// The box's right edge.
    Right,
    /// The bubble's tail tip (TX-055).
    Tail,
}

/// Where a text clip's handles are, in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextHandles {
    /// The side handles; `None` while the text is curved (fixed width is
    /// paused, TX-005).
    pub sides: Option<[DVec2; 2]>,
    /// The tail tip, while a bubble with a tail (or Thought's trail) shows.
    pub tail: Option<DVec2>,
}

/// The handles of a text clip showing `data` in `layer`.
pub fn text_handles(layer: &LayerBox, data: &TextClipData, scale_px: f64) -> TextHandles {
    let (w, h) = layer.size;
    let m = layer.to_canvas;
    let curved = data.style.is_curved();
    let sides = (!curved).then(|| {
        [
            m.transform_point2(DVec2::new(0.0, h / 2.0)),
            m.transform_point2(DVec2::new(w, h / 2.0)),
        ]
    });
    let b = &data.style.bubble;
    let tail = (b.on
        && b.value.spec.tail.on
        && kind_has_tail(b.value.spec.kind)
        && !curved
        && !data.content.is_empty())
    .then(|| {
        let tip = b.value.spec.tail.value.tip;
        let local = DVec2::new(w, h) / 2.0 + DVec2::new(tip.x.as_f64(), tip.y.as_f64()) * scale_px;
        m.transform_point2(local)
    });
    TextHandles { sides, tail }
}

/// The handle within `reach` canvas pixels of `p`, the tail first (it may
/// sit on an edge).
pub fn handle_at(handles: &TextHandles, p: DVec2, reach: f64) -> Option<Handle> {
    let near = |q: DVec2| q.distance(p) <= reach;
    if handles.tail.is_some_and(near) {
        return Some(Handle::Tail);
    }
    let [left, right] = handles.sides?;
    if near(left) {
        Some(Handle::Left)
    } else if near(right) {
        Some(Handle::Right)
    } else {
        None
    }
}

/// Where `handle` is, in canvas pixels.
pub fn handle_point(handles: &TextHandles, handle: Handle) -> Option<DVec2> {
    match handle {
        Handle::Tail => handles.tail,
        Handle::Left => handles.sides.map(|[l, _]| l),
        Handle::Right => handles.sides.map(|[_, r]| r),
    }
}

/// What a side-handle drag sets: the fixed width, and how far the clip's
/// position moves (canvas pixels) so the opposite edge stays put.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidthDrag {
    pub width: RefPx,
    pub shift: DVec2,
}

/// The width a drag of `handle` to canvas point `pointer` gives the box
/// that was `start` when the drag began (TX-005): the opposite edge stays,
/// or with `symmetric` (Alt) the centre does. The width is clamped to
/// 20–10,000 ref px. `None` for the tail or a degenerate box.
pub fn width_drag(
    start: &LayerBox,
    handle: Handle,
    pointer: DVec2,
    symmetric: bool,
    scale_px: f64,
) -> Option<WidthDrag> {
    let m = start.to_canvas;
    let det = m.determinant();
    if !det.is_finite() || det.abs() < 1e-12 || scale_px <= 0.0 {
        return None;
    }
    let local = m.inverse().transform_point2(pointer);
    let w = start.size.0;
    let wanted = match (handle, symmetric) {
        (Handle::Tail, _) => return None,
        (_, true) => 2.0 * (local.x - w / 2.0).abs(),
        (Handle::Right, false) => local.x,
        (Handle::Left, false) => w - local.x,
    };
    let width = TextLayout::clamp_box_width(Some(ref_px(wanted / scale_px)))?;
    let new_w = width.as_f64() * scale_px;
    let dx = match (handle, symmetric) {
        (_, true) | (Handle::Tail, _) => 0.0,
        (Handle::Right, false) => (new_w - w) / 2.0,
        (Handle::Left, false) => (w - new_w) / 2.0,
    };
    let shift = m.transform_vector2(DVec2::new(dx, 0.0));
    Some(WidthDrag { width, shift })
}

/// The tail tip for the pointer at canvas point `pointer`, on the box that
/// was `start` when the drag began: ref px from the box centre in the
/// clip's frame, at most 40 em away (TX-055). `None` for a degenerate box.
pub fn tail_tip_at(
    start: &LayerBox,
    pointer: DVec2,
    scale_px: f64,
    size: RefPx,
) -> Option<TailTip> {
    let m = start.to_canvas;
    let det = m.determinant();
    if !det.is_finite() || det.abs() < 1e-12 || scale_px <= 0.0 {
        return None;
    }
    let local = m.inverse().transform_point2(pointer);
    let mut v = (local - DVec2::new(start.size.0, start.size.1) / 2.0) / scale_px;
    let reach = TIP_REACH_EM * size.as_f64();
    if v.length() > reach {
        v *= reach / v.length();
    }
    Some(TailTip {
        x: ref_px(v.x),
        y: ref_px(v.y),
    })
}

/// TX-055's default tip for text of `text` ref px (laid-out width and
/// height) in `style`'s bubble: a quarter of the body width left of the
/// centre, 1.2 em below the body's bottom edge. The body is the text plus
/// the bubble's padding, at least 2 em × 1.5 em.
pub fn default_tail_tip(text: (f64, f64), style: &TextStyle) -> TailTip {
    let em = style.size.as_f64();
    let b = &style.bubble.value;
    let body_w = (text.0 + 2.0 * b.pad_x.as_f64() * em).max(2.0 * em);
    let body_h = (text.1 + 2.0 * b.pad_y.as_f64() * em).max(1.5 * em);
    TailTip {
        x: ref_px(-body_w / 4.0),
        y: ref_px(body_h / 2.0 + 1.2 * em),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::num::NonZeroU64;

    use freecut_timeline::glam::DMat3;
    use freecut_timeline::ids::{ClipId, TrackId};
    use freecut_timeline::model::{
        Angle, CanvasPos, Scale, SignedPerMille, TextLayerKind, Transform,
    };
    use freecut_timeline::plan::clip_placement;

    use super::*;

    /// A box of `size` layout px, placed by `t` on a 1920 × 1080 canvas
    /// with its origin at the box centre.
    fn layer(t: &Transform, size: (f64, f64)) -> LayerBox {
        let raw = NonZeroU64::MIN;
        LayerBox {
            clip: ClipId::from_raw(raw),
            track: TrackId::from_raw(raw),
            to_canvas: clip_placement(t, (1920, 1080))
                * DMat3::from_translation(-DVec2::new(size.0, size.1) / 2.0),
            size,
            opacity: 1.0,
        }
    }

    fn close(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-6
    }

    #[test]
    fn side_handles_sit_mid_edge_and_pause_while_curved() {
        let mut data = TextClipData::default();
        let l = layer(&Transform::IDENTITY, (400.0, 100.0));
        let h = text_handles(&l, &data, 1.0);
        let [left, right] = h.sides.unwrap();
        assert!(close(left, DVec2::new(760.0, 540.0)));
        assert!(close(right, DVec2::new(1160.0, 540.0)));
        assert_eq!(h.tail, None, "no bubble");
        assert_eq!(
            handle_at(&h, DVec2::new(1165.0, 538.0), 10.0),
            Some(Handle::Right)
        );
        assert_eq!(handle_at(&h, DVec2::new(960.0, 540.0), 10.0), None);
        data.style = data.style.with_layer(TextLayerKind::Curve, true);
        data.style.curve.value = SignedPerMille::new(400);
        assert_eq!(text_handles(&l, &data, 1.0).sides, None);
    }

    #[test]
    fn a_right_drag_keeps_the_left_edge_and_alt_keeps_the_centre() {
        let l = layer(&Transform::IDENTITY, (400.0, 100.0));
        // Right edge from 1160 to 1260 at scale 1: 500 px wide, centre
        // moves 50 px right, so the left edge stays at 760.
        let d = width_drag(&l, Handle::Right, DVec2::new(1260.0, 540.0), false, 1.0).unwrap();
        assert_eq!(d.width, RefPx::from_px(500));
        assert!(close(d.shift, DVec2::new(50.0, 0.0)));
        let d = width_drag(&l, Handle::Left, DVec2::new(860.0, 500.0), false, 1.0).unwrap();
        assert_eq!(d.width, RefPx::from_px(300));
        assert!(close(d.shift, DVec2::new(50.0, 0.0)), "right edge stays");
        let d = width_drag(&l, Handle::Right, DVec2::new(1260.0, 540.0), true, 1.0).unwrap();
        assert_eq!(d.width, RefPx::from_px(600));
        assert!(close(d.shift, DVec2::ZERO));
        // Past the left edge: clamped to 20 ref px.
        let d = width_drag(&l, Handle::Right, DVec2::new(0.0, 540.0), false, 2.0).unwrap();
        assert_eq!(d.width, TextLayout::MIN_BOX_WIDTH);
        assert!(close(d.shift, DVec2::new((40.0 - 400.0) / 2.0, 0.0)));
        assert_eq!(width_drag(&l, Handle::Tail, DVec2::ZERO, false, 1.0), None);
    }

    #[test]
    fn width_drags_follow_rotation_and_scale() {
        // Turned 90° clockwise and doubled: the box's right edge points
        // down on screen.
        let t = Transform {
            rotation: Angle::from_degrees(90),
            scale: Scale::from_percent(200),
            position: CanvasPos::new(0, 0),
            ..Transform::IDENTITY
        };
        let l = layer(&t, (200.0, 50.0));
        let [_, right] = text_handles(&l, &TextClipData::default(), 1.0)
            .sides
            .unwrap();
        assert!(close(right, DVec2::new(960.0, 540.0 + 200.0)));
        // 40 screen px further down is 20 layout px wider.
        let d = width_drag(&l, Handle::Right, right + DVec2::new(0.0, 40.0), false, 1.0).unwrap();
        assert_eq!(d.width, RefPx::from_px(220));
        assert!(close(d.shift, DVec2::new(0.0, 20.0)));
    }

    #[test]
    fn the_tail_handle_follows_the_tip_and_stops_at_forty_em() {
        let mut data = TextClipData::default();
        data.style = data.style.with_layer(TextLayerKind::Bubble, true);
        data.style.bubble.value.spec.tail.value.tip = TailTip {
            x: RefPx::from_px(-50),
            y: RefPx::from_px(100),
        };
        let l = layer(&Transform::IDENTITY, (400.0, 100.0));
        // Scale 2: 1 ref px is 2 layout px.
        let tail = text_handles(&l, &data, 2.0).tail.unwrap();
        assert!(close(tail, DVec2::new(960.0 - 100.0, 540.0 + 200.0)));
        let tip = tail_tip_at(&l, tail, 2.0, data.style.size).unwrap();
        assert_eq!(tip, data.style.bubble.value.spec.tail.value.tip);
        // Far away: 40 em of 64 ref px along the same direction.
        let far = tail_tip_at(&l, DVec2::new(960.0, 1e7), 1.0, data.style.size).unwrap();
        assert_eq!((far.x, far.y), (RefPx::from_px(0), RefPx::from_px(2560)));
        // Kinds without a tail, a tail switched off, or curved text: none.
        data.style.bubble.value.spec.tail.on = false;
        assert_eq!(text_handles(&l, &data, 1.0).tail, None);
    }

    #[test]
    fn the_default_tip_is_below_and_left_of_the_body() {
        let style = TextStyle::default();
        // 300 × 80 ref px of text at 64 px: body 357.6 × 118.4.
        let tip = default_tail_tip((300.0, 80.0), &style);
        assert_eq!(tip.x, RefPx::from_hundredths(-8940));
        assert_eq!(tip.y, RefPx::from_hundredths(5920 + 7680));
        // Tiny text gets the minimum body: 2 em × 1.5 em.
        let tip = default_tail_tip((1.0, 1.0), &style);
        assert_eq!(tip.x, RefPx::from_px(-32));
    }
}
