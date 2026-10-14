//! The Stage's handles: geometry of a text box's side handles and bubble
//! tail tip (TX-005, TX-055), and of a colour mask's outline and handles
//! (CL-031, TR-019), as pure functions over the preview's [`LayerBox`]
//! (the renderer's own layout, never one made on the UI thread). Floats
//! only: this is the UI edge. The player draws the handles and turns
//! drags into edits.
//!
//! A text layer's box is `(0, 0)..size` in layout pixels (canvas pixels at
//! 100 % scale); `to_canvas` places it. One ref px is `scale_px` layout
//! pixels ([`freecut_timeline::plan::ref_px_scale`]).

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::model::{
    Angle, ColorMask, ColorMaskParam, MASK_FEATHER_SPAN, Mask, MaskParam, MaskPoint, MaskShape,
    MaskShapeKind, PerMille, PropValue, RefPx, TailTip, TextClipData, TextLayout, TextStyle,
};

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

// ---------------------------------------------------------------------------
// Colour masks (CL-031, TR-019)
// ---------------------------------------------------------------------------

/// The gap between a mask's edge and its feather handle, and the reach of
/// the rotation knob past the shape, in shorter sides of the item.
const FEATHER_GAP: f64 = 0.05;
const KNOB_GAP: f64 = 0.1;

/// Where a mask's frame sits on the canvas (TR-019): mask units (shorter
/// sides of the item from its centre, +y down, as the renderer reads them)
/// mapped to canvas pixels. A clip's masks live in its display frame, so
/// the frame follows the clip's transform; an adjustment clip's live on
/// the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskFrame {
    to_canvas: DMat3,
}

impl MaskFrame {
    /// The frame of a picture clip whose box is `layer`.
    pub fn of_layer(layer: &LayerBox) -> MaskFrame {
        let (w, h) = layer.size;
        MaskFrame::of_box(layer.to_canvas, (w, h))
    }

    /// The canvas itself, `size` pixels (an adjustment clip, CL-031).
    pub fn canvas(size: (u32, u32)) -> MaskFrame {
        MaskFrame::of_box(DMat3::IDENTITY, (f64::from(size.0), f64::from(size.1)))
    }

    fn of_box(to_canvas: DMat3, (w, h): (f64, f64)) -> MaskFrame {
        let s = w.min(h).max(1e-9);
        MaskFrame {
            to_canvas: to_canvas
                * DMat3::from_translation(DVec2::new(w / 2.0, h / 2.0))
                * DMat3::from_scale(DVec2::splat(s)),
        }
    }

    pub fn canvas_point(&self, p: DVec2) -> DVec2 {
        self.to_canvas.transform_point2(p)
    }

    /// The mask point under canvas point `p`; `None` for a degenerate
    /// frame (a clip scaled to nothing).
    pub fn mask_point(&self, p: DVec2) -> Option<DVec2> {
        let det = self.to_canvas.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        Some(self.to_canvas.inverse().transform_point2(p))
    }
}

/// A handle of the mask being edited (TR-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskHandle {
    /// The centre: moves the mask.
    Center,
    /// The right edge: the width.
    Width,
    /// The bottom edge: the height.
    Height,
    /// The bottom-right corner: width and height together.
    Corner,
    /// The knob past the shape: the rotation.
    Rotate,
    /// The small handle outside the edge: the feather.
    Feather,
    /// A band's edge: its width.
    BandEdge,
}

/// A mask's outline and handles on the canvas, in canvas pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskHandles {
    /// The edge as the renderer draws it (Expand included); open
    /// polylines for a line and a band, one closed loop otherwise.
    pub outline: Vec<Vec<DVec2>>,
    /// Whether the outline's polylines are closed loops.
    pub closed: bool,
    /// The two edges of the feather band, when there is a feather.
    pub feather: Vec<Vec<DVec2>>,
    pub handles: Vec<(MaskHandle, DVec2)>,
    /// The line from the centre to the rotation knob.
    pub stem: [DVec2; 2],
}

/// A mask's own frame in the item: local (along its turned axes) to mask
/// units.
fn local_to_mask(m: &Mask, l: DVec2) -> DVec2 {
    let (cx, cy) = m.center.to_f64();
    let (s, c) = m.rotation.degrees().to_radians().sin_cos();
    DVec2::new(cx + c * l.x - s * l.y, cy + s * l.x + c * l.y)
}

fn mask_to_local(m: &Mask, p: DVec2) -> DVec2 {
    let (cx, cy) = m.center.to_f64();
    let (s, c) = m.rotation.degrees().to_radians().sin_cos();
    let d = p - DVec2::new(cx, cy);
    DVec2::new(c * d.x + s * d.y, -s * d.x + c * d.y)
}

/// The feather's width in shorter sides.
fn feather_width(m: &Mask) -> f64 {
    m.feather.as_f64() * MASK_FEATHER_SPAN
}

/// Half of a band's width in shorter sides.
fn band_half(m: &Mask) -> f64 {
    match m.shape {
        MaskShape::Band { width } => width.as_f64() / 2.0,
        _ => 0.0,
    }
}

/// The local point of `handle` on `m`.
fn handle_local(m: &Mask, handle: MaskHandle) -> DVec2 {
    let (w, h) = m.size.to_f64();
    let (bx, by) = (w / 2.0, h / 2.0);
    let f = feather_width(m) / 2.0 + FEATHER_GAP;
    let sized = m.kind().has_size();
    match handle {
        MaskHandle::Center => DVec2::ZERO,
        MaskHandle::Width => DVec2::new(bx, 0.0),
        MaskHandle::Height => DVec2::new(0.0, by),
        MaskHandle::Corner => DVec2::new(bx, by),
        MaskHandle::Rotate if sized => DVec2::new(0.0, -by - KNOB_GAP),
        // A line and a band turn by a knob along their middle.
        MaskHandle::Rotate => DVec2::new(0.25, 0.0),
        MaskHandle::Feather if sized => DVec2::new(-bx - f, 0.0),
        MaskHandle::Feather => DVec2::new(0.0, band_half(m) + f),
        MaskHandle::BandEdge => DVec2::new(0.0, -band_half(m)),
    }
}

/// The handles `m`'s shape has.
fn handles_of(m: &Mask) -> &'static [MaskHandle] {
    use MaskHandle as H;
    match m.kind() {
        MaskShapeKind::Linear => &[H::Center, H::Rotate, H::Feather],
        MaskShapeKind::Band => &[H::Center, H::BandEdge, H::Rotate, H::Feather],
        MaskShapeKind::Ellipse | MaskShapeKind::Rectangle => &[
            H::Center,
            H::Width,
            H::Height,
            H::Corner,
            H::Rotate,
            H::Feather,
        ],
    }
}

/// `m`'s edge in local coordinates, `grow` outside the shape (negative:
/// inside), as polylines.
fn edge_local(m: &Mask, grow: f64) -> Vec<Vec<DVec2>> {
    let (w, h) = m.size.to_f64();
    let reach = 4.0;
    match m.shape {
        MaskShape::Linear => vec![vec![DVec2::new(-reach, grow), DVec2::new(reach, grow)]],
        MaskShape::Band { .. } => {
            let half = (band_half(m) + grow).max(0.0);
            vec![
                vec![DVec2::new(-reach, -half), DVec2::new(reach, -half)],
                vec![DVec2::new(-reach, half), DVec2::new(reach, half)],
            ]
        }
        MaskShape::Ellipse => {
            let (rx, ry) = ((w / 2.0 + grow).max(0.0), (h / 2.0 + grow).max(0.0));
            vec![
                (0..72)
                    .map(|i| {
                        let t = f64::from(i) * std::f64::consts::TAU / 72.0;
                        DVec2::new(rx * t.cos(), ry * t.sin())
                    })
                    .collect(),
            ]
        }
        MaskShape::Rectangle { roundness } => {
            let (bx, by) = ((w / 2.0 + grow).max(0.0), (h / 2.0 + grow).max(0.0));
            let r = (roundness.as_f64() * (w / 2.0).min(h / 2.0) + grow).clamp(0.0, bx.min(by));
            let mut out = Vec::with_capacity(40);
            for (corner, start) in [
                (DVec2::new(bx - r, by - r), 0.0),
                (DVec2::new(-bx + r, by - r), 90.0),
                (DVec2::new(-bx + r, -by + r), 180.0),
                (DVec2::new(bx - r, -by + r), 270.0),
            ] {
                for k in 0..=8 {
                    let t = (start + f64::from(k) * 90.0 / 8.0f64).to_radians();
                    out.push(corner + DVec2::new(t.cos(), t.sin()) * r);
                }
            }
            vec![out]
        }
    }
}

/// Mask `cm`'s outline and handles in `frame`.
pub fn mask_handles(frame: &MaskFrame, cm: &ColorMask) -> MaskHandles {
    let m = &cm.mask;
    let place = |l: DVec2| frame.canvas_point(local_to_mask(m, l));
    let lines = |grow: f64| -> Vec<Vec<DVec2>> {
        edge_local(m, grow)
            .into_iter()
            .map(|line| line.into_iter().map(place).collect())
            .collect()
    };
    let grow = f64::from(cm.expand) / 1000.0;
    let f = feather_width(m) / 2.0;
    let feather = if f > 0.0 && !m.is_empty_area() {
        let mut both = lines(grow + f);
        both.extend(lines(grow - f));
        both
    } else {
        Vec::new()
    };
    MaskHandles {
        outline: lines(grow),
        closed: m.kind().has_size(),
        feather,
        handles: handles_of(m)
            .iter()
            .map(|h| (*h, place(handle_local(m, *h))))
            .collect(),
        stem: [
            place(DVec2::ZERO),
            place(handle_local(m, MaskHandle::Rotate)),
        ],
    }
}

/// The handle within `reach` canvas pixels of `p`: the small ones first,
/// the centre last.
pub fn mask_handle_at(handles: &MaskHandles, p: DVec2, reach: f64) -> Option<(MaskHandle, DVec2)> {
    handles
        .handles
        .iter()
        .filter(|(_, q)| q.distance(p) <= reach)
        .min_by(|a, b| {
            let rank = |h: MaskHandle| u8::from(h == MaskHandle::Center);
            rank(a.0)
                .cmp(&rank(b.0))
                .then(a.1.distance(p).total_cmp(&b.1.distance(p)))
        })
        .copied()
}

/// What dragging `handle` of `start` (the mask as the drag began) to
/// canvas point `pointer` sets: one value, before clamping. `press` is
/// where the drag began; `shift` locks the move to one axis, keeps a
/// corner's ratio and turns in 15° steps (TR-007's modifiers). `None` for
/// a degenerate frame.
pub fn mask_drag(
    frame: &MaskFrame,
    start: &ColorMask,
    handle: MaskHandle,
    press: DVec2,
    pointer: DVec2,
    shift: bool,
) -> Option<(ColorMaskParam, PropValue)> {
    let m = &start.mask;
    let at = frame.mask_point(pointer)?;
    let q = mask_to_local(m, at);
    let (w0, h0) = m.size.to_f64();
    let point = |x: f64, y: f64| PropValue::MaskPoint(MaskPoint::from_f64(x, y));
    let ratio = |v: f64| {
        // Saturated: the stack clamps into the value's range.
        PropValue::Ratio(PerMille::new(
            (v * 1000.0).round().clamp(0.0, f64::from(u16::MAX)) as u16,
        ))
    };
    let shape = ColorMaskParam::Shape;
    Some(match handle {
        MaskHandle::Center => {
            let mut d = at - frame.mask_point(press)?;
            if shift {
                if d.x.abs() >= d.y.abs() {
                    d.y = 0.0;
                } else {
                    d.x = 0.0;
                }
            }
            let (cx, cy) = m.center.to_f64();
            (shape(MaskParam::Center), point(cx + d.x, cy + d.y))
        }
        MaskHandle::Width | MaskHandle::Height | MaskHandle::Corner => {
            let (mut w, mut h) = (w0, h0);
            match handle {
                MaskHandle::Width => w = 2.0 * q.x.abs(),
                MaskHandle::Height => h = 2.0 * q.y.abs(),
                _ => {
                    w = 2.0 * q.x.abs();
                    h = 2.0 * q.y.abs();
                }
            }
            // Linked (or Shift on the corner): the ratio stays.
            let keep = m.linked || (shift && handle == MaskHandle::Corner);
            if keep && w0 > 0.0 && h0 > 0.0 {
                let k = match handle {
                    MaskHandle::Width => w / w0,
                    MaskHandle::Height => h / h0,
                    _ => (w / w0).max(h / h0),
                };
                (w, h) = (w0 * k, h0 * k);
            }
            (shape(MaskParam::Size), point(w, h))
        }
        MaskHandle::Rotate => {
            let (cx, cy) = m.center.to_f64();
            let d = at - DVec2::new(cx, cy);
            let base = handle_local(m, MaskHandle::Rotate);
            let mut deg = (d.y.atan2(d.x) - base.y.atan2(base.x)).to_degrees();
            // The turn nearest the one the drag started from.
            let was = m.rotation.degrees();
            deg += ((was - deg) / 360.0).round() * 360.0;
            if shift {
                deg = (deg / 15.0).round() * 15.0;
            }
            let hundredths = (deg * 100.0).round().clamp(-1e9, 1e9) as i32;
            (
                shape(MaskParam::Rotation),
                PropValue::Angle(Angle::from_hundredths(hundredths)),
            )
        }
        MaskHandle::Feather => {
            let out = if m.kind().has_size() {
                -q.x - w0 / 2.0
            } else {
                q.y.abs() - band_half(m)
            };
            let f = (2.0 * (out - FEATHER_GAP)).max(0.0);
            (shape(MaskParam::Feather), ratio(f / MASK_FEATHER_SPAN))
        }
        MaskHandle::BandEdge => (shape(MaskParam::BandWidth), ratio(2.0 * q.y.abs())),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::num::NonZeroU64;

    use freecut_timeline::glam::DMat3;
    use freecut_timeline::ids::{ClipId, TrackId};
    use freecut_timeline::model::{CanvasPos, Scale, SignedPerMille, TextLayerKind, Transform};
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

    fn video_box(t: &Transform) -> LayerBox {
        let raw = NonZeroU64::MIN;
        // A 1920 × 1080 picture filling a 1920 × 1080 canvas at identity.
        LayerBox {
            clip: ClipId::from_raw(raw),
            track: TrackId::from_raw(raw),
            to_canvas: clip_placement(t, (1920, 1080))
                * DMat3::from_translation(-DVec2::new(1920.0, 1080.0) / 2.0),
            size: (1920.0, 1080.0),
            opacity: 1.0,
        }
    }

    fn ellipse() -> ColorMask {
        ColorMask::new(1, Mask::new(MaskShapeKind::Ellipse))
    }

    #[test]
    fn mask_handles_sit_on_the_shape_and_follow_the_clip() {
        let frame = MaskFrame::of_layer(&video_box(&Transform::IDENTITY));
        let h = mask_handles(&frame, &ellipse());
        let at = |which| h.handles.iter().find(|(k, _)| *k == which).unwrap().1;
        // 50 % of the 1080 px shorter side across, at the centre.
        assert!(close(at(MaskHandle::Center), DVec2::new(960.0, 540.0)));
        assert!(close(
            at(MaskHandle::Width),
            DVec2::new(960.0 + 270.0, 540.0)
        ));
        assert!(close(
            at(MaskHandle::Height),
            DVec2::new(960.0, 540.0 + 270.0)
        ));
        assert!(h.closed && h.feather.is_empty());
        // A half-size clip moved right: the mask moves and shrinks with it.
        let t = Transform {
            scale: Scale::from_percent(50),
            position: CanvasPos::new(250_000, 0),
            ..Transform::IDENTITY
        };
        let small = mask_handles(&MaskFrame::of_layer(&video_box(&t)), &ellipse());
        let centre = small.handles[0].1;
        assert!(
            close(centre, DVec2::new(960.0 + 480.0, 540.0)),
            "{centre:?}"
        );
        let w = small
            .handles
            .iter()
            .find(|(k, _)| *k == MaskHandle::Width)
            .unwrap()
            .1;
        assert!(close(w, centre + DVec2::new(135.0, 0.0)));
        // An adjustment clip's frame is the canvas.
        let canvas = mask_handles(&MaskFrame::canvas((1920, 1080)), &ellipse());
        assert!(close(canvas.handles[0].1, DVec2::new(960.0, 540.0)));
        assert_eq!(
            mask_handle_at(&h, DVec2::new(1232.0, 541.0), 10.0).map(|(k, _)| k),
            Some(MaskHandle::Width)
        );
        assert_eq!(mask_handle_at(&h, DVec2::new(100.0, 100.0), 10.0), None);
    }

    #[test]
    fn mask_drags_set_one_value_each() {
        let frame = MaskFrame::of_layer(&video_box(&Transform::IDENTITY));
        let e = ellipse();
        let c = DVec2::new(960.0, 540.0);
        // The centre moves by the pointer's travel, in shorter sides.
        let (p, v) = mask_drag(
            &frame,
            &e,
            MaskHandle::Center,
            c,
            c + DVec2::new(108.0, 54.0),
            false,
        )
        .unwrap();
        assert_eq!(p, ColorMaskParam::Shape(MaskParam::Center));
        assert_eq!(v, PropValue::MaskPoint(MaskPoint::new(1_000, 500)));
        // Shift keeps one axis.
        let (_, v) = mask_drag(
            &frame,
            &e,
            MaskHandle::Center,
            c,
            c + DVec2::new(108.0, 54.0),
            true,
        )
        .unwrap();
        assert_eq!(v, PropValue::MaskPoint(MaskPoint::new(1_000, 0)));
        // A linked ellipse keeps its ratio from the width handle.
        let (p, v) = mask_drag(
            &frame,
            &e,
            MaskHandle::Width,
            c,
            c + DVec2::new(540.0, 0.0),
            false,
        )
        .unwrap();
        assert_eq!(p, ColorMaskParam::Shape(MaskParam::Size));
        assert_eq!(v, PropValue::MaskPoint(MaskPoint::new(10_000, 10_000)));
        // The knob sits above: pulled to the right it turns 90° clockwise.
        let (p, v) = mask_drag(
            &frame,
            &e,
            MaskHandle::Rotate,
            c,
            c + DVec2::new(300.0, 0.0),
            false,
        )
        .unwrap();
        assert_eq!(p, ColorMaskParam::Shape(MaskParam::Rotation));
        assert_eq!(v, PropValue::Angle(Angle::from_degrees(90)));
        // Feather: pulled out from the left edge past the gap.
        let edge = c - DVec2::new(270.0 + 0.05 * 1080.0, 0.0);
        let (p, v) = mask_drag(
            &frame,
            &e,
            MaskHandle::Feather,
            edge,
            edge - DVec2::new(0.0625 * 1080.0, 0.0),
            false,
        )
        .unwrap();
        assert_eq!(p, ColorMaskParam::Shape(MaskParam::Feather));
        assert_eq!(v, PropValue::Ratio(PerMille::new(500)));
        // A band's edge sets its width.
        let band = ColorMask::new(2, Mask::new(MaskShapeKind::Band));
        let (p, v) = mask_drag(
            &frame,
            &band,
            MaskHandle::BandEdge,
            c,
            c - DVec2::new(0.0, 216.0),
            false,
        )
        .unwrap();
        assert_eq!(p, ColorMaskParam::Shape(MaskParam::BandWidth));
        assert_eq!(v, PropValue::Ratio(PerMille::new(400)));
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
