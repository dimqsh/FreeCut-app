//! The thin scrollbar for painted areas (ui-concept §13.24): an 8 pt
//! gutter with no track fill and a 4 pt thumb (2 pt inset, `r.xs`, at least
//! 24 pt long) in `border.strong`, `text.tertiary` while hovered or dragged;
//! hidden when nothing overflows. Pressing the gutter jumps the thumb there
//! and keeps dragging. The timeline's lanes use it, since they are painted
//! rather than laid out in a `ScrollArea`.

use eframe::egui::{self, Id, Pos2, Rect, Sense, Vec2};

use crate::theme;

/// The shortest a thumb gets.
pub const MIN_THUMB: f32 = 24.0;
/// The thumb's thickness and its inset from the gutter's edges.
const THUMB: f32 = 4.0;
const INSET: f32 = 2.0;

/// Which way the bar runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// The thumb's start and length along a gutter `track` pt long, for
/// `content` pt of which `view` show from `offset`. `None` when nothing
/// overflows.
pub fn thumb_span(track: f32, content: f32, view: f32, offset: f32) -> Option<(f32, f32)> {
    if content <= view + 0.5 || track <= 0.0 || view <= 0.0 {
        return None;
    }
    let len = (track * view / content).clamp(MIN_THUMB.min(track), track);
    let room = (content - view).max(f32::EPSILON);
    let start = (track - len) * (offset / room).clamp(0.0, 1.0);
    Some((start, len))
}

/// The offset that puts the thumb's start at `thumb_start` pt along the
/// gutter (the inverse of [`thumb_span`]), within `0..=content − view`.
pub fn offset_for(track: f32, content: f32, view: f32, thumb_start: f32) -> f32 {
    let Some((_, len)) = thumb_span(track, content, view, 0.0) else {
        return 0.0;
    };
    let free = (track - len).max(f32::EPSILON);
    let room = (content - view).max(0.0);
    (thumb_start / free).clamp(0.0, 1.0) * room
}

/// Shows the bar in `gutter` and applies a drag to `offset`. Returns
/// whether the offset changed.
pub fn thin_scrollbar(
    ui: &mut egui::Ui,
    id: Id,
    gutter: Rect,
    axis: Axis,
    content: f32,
    view: f32,
    offset: &mut f32,
) -> bool {
    let along = |p: Pos2| match axis {
        Axis::Horizontal => p.x - gutter.left(),
        Axis::Vertical => p.y - gutter.top(),
    };
    let track = match axis {
        Axis::Horizontal => gutter.width(),
        Axis::Vertical => gutter.height(),
    };
    let Some((start, len)) = thumb_span(track, content, view, *offset) else {
        return false;
    };
    let response = ui.interact(gutter, id, Sense::click_and_drag());
    let before = *offset;
    // Where in the thumb the pointer holds it, kept for the whole drag.
    let grab_id = id.with("grab");
    if response.drag_started() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            let at = along(p);
            let grab = if at >= start && at <= start + len {
                at - start
            } else {
                // Outside the thumb: jump so the thumb's middle is there.
                len / 2.0
            };
            ui.data_mut(|d| d.insert_temp(grab_id, grab));
            *offset = offset_for(track, content, view, at - grab);
        }
    } else if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        let grab = ui.data(|d| d.get_temp::<f32>(grab_id)).unwrap_or(len / 2.0);
        *offset = offset_for(track, content, view, along(p) - grab);
    }
    if ui.is_rect_visible(gutter) {
        let p = theme::look(ui.ctx()).palette;
        let hot = response.hovered() || response.dragged();
        let color = if hot {
            p.text_tertiary
        } else {
            p.border_strong
        };
        let (start, len) = thumb_span(track, content, view, *offset).unwrap_or((start, len));
        let thumb = match axis {
            Axis::Horizontal => Rect::from_min_size(
                Pos2::new(gutter.left() + start, gutter.center().y - THUMB / 2.0),
                Vec2::new(len, THUMB),
            ),
            Axis::Vertical => Rect::from_min_size(
                Pos2::new(gutter.center().x - THUMB / 2.0, gutter.top() + start),
                Vec2::new(THUMB, len),
            ),
        };
        let thumb = match axis {
            Axis::Horizontal => thumb.shrink2(Vec2::new(INSET, 0.0)),
            Axis::Vertical => thumb.shrink2(Vec2::new(0.0, INSET)),
        };
        ui.painter().rect_filled(thumb, theme::R_XS, color);
    }
    (*offset - before).abs() > f32::EPSILON
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact geometry

    use super::*;

    #[test]
    fn the_thumb_shows_only_when_something_overflows() {
        assert_eq!(thumb_span(200.0, 300.0, 300.0, 0.0), None);
        assert_eq!(thumb_span(200.0, 100.0, 300.0, 0.0), None);
        let (start, len) = thumb_span(200.0, 400.0, 100.0, 0.0).unwrap();
        assert_eq!((start, len), (0.0, 50.0));
        // At the end the thumb touches the far end.
        let (start, len) = thumb_span(200.0, 400.0, 100.0, 300.0).unwrap();
        assert_eq!(start + len, 200.0);
        // Never shorter than 24 pt.
        let (_, len) = thumb_span(200.0, 1.0e6, 100.0, 0.0).unwrap();
        assert_eq!(len, MIN_THUMB);
        // Offsets past the end are held at it.
        let (start, _) = thumb_span(200.0, 400.0, 100.0, 9_999.0).unwrap();
        assert_eq!(start, 150.0);
    }

    #[test]
    fn offsets_and_thumb_positions_round_trip() {
        for offset in [0.0, 75.0, 150.0, 300.0] {
            let (start, _) = thumb_span(200.0, 400.0, 100.0, offset).unwrap();
            assert!((offset_for(200.0, 400.0, 100.0, start) - offset).abs() < 1e-3);
        }
        assert_eq!(offset_for(200.0, 400.0, 100.0, -50.0), 0.0);
        assert_eq!(offset_for(200.0, 400.0, 100.0, 1_000.0), 300.0);
        assert_eq!(offset_for(200.0, 50.0, 100.0, 10.0), 0.0, "no overflow");
    }
}
