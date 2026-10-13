//! The panel shell (ui-concept §3.4, §13.16, §13.17): rounded panels
//! floating on the ground with gutters between them, the panel header, the
//! focus line and the gutter's resize line.
//!
//! Every rule with a number in it is a pure function ([`ground_margin`],
//! [`corner_mask_points`], [`inset_span`]) so it can be tested without a
//! window; the painters only call them.

use eframe::egui::{
    self, Color32, Frame, Margin, Mesh, Pos2, Rangef, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::theme::{self, Look, ThemeKind, Type};

/// What lies beyond one side of a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beyond {
    /// The window's edge: a full gutter of ground.
    Window,
    /// Another panel: half a gutter here, half on its side.
    Panel,
    /// Ground that is already there (the top bar, the status line).
    Ground,
}

/// The ground around a panel, in the order left, right, top, bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides {
    pub left: Beyond,
    pub right: Beyond,
    pub top: Beyond,
    pub bottom: Beyond,
}

/// The ground margin of a panel whose neighbours are `sides`, for a
/// `gutter` of ground between panels: neighbours add up to one gutter,
/// window edges get a whole one.
pub fn ground_margin(sides: Sides, gutter: f32) -> Margin {
    let side = |b: Beyond| -> i8 {
        let pt = match b {
            Beyond::Window => gutter,
            Beyond::Panel => gutter / 2.0,
            Beyond::Ground => 0.0,
        };
        // Gutters are a few points (4 or 8): far inside i8.
        pt.round() as i8
    };
    Margin {
        left: side(sides.left),
        right: side(sides.right),
        top: side(sides.top),
        bottom: side(sides.bottom),
    }
}

/// The regions' smallest cards (ui-concept §3.2, UI-001): the asset panel
/// and the inspector wide, the Stage wide, the timeline and the upper area
/// (the row of asset panel, Stage and inspector) tall.
pub const ASSETS_MIN: f32 = 240.0;
pub const INSPECTOR_MIN: f32 = 280.0;
pub const STAGE_MIN: f32 = 320.0;
pub const TIMELINE_MIN: f32 = 160.0;
pub const UPPER_MIN: f32 = 240.0;

/// The outer sizes a resizable panel may take (§13.17: splits are
/// clamped to the regions' minimums): its card at least `min_card` plus
/// `ground` points of gutter along the split's axis, and at most what is
/// left of `room` after `others`, the space the regions beyond it need. A
/// room too small for both keeps the panel's minimum.
pub fn split_range(min_card: f32, ground: f32, room: f32, others: f32) -> Rangef {
    let min = min_card + ground;
    Rangef::new(min, (room - others).max(min))
}

/// The ground around a panel: `bg.app` with the gutter margins.
pub fn ground_frame(look: &Look, sides: Sides) -> Frame {
    Frame::NONE
        .fill(look.palette.bg_app)
        .inner_margin(ground_margin(sides, look.metrics.gutter))
}

/// A panel's card: `bg.panel`, `r.lg`, no outline (High contrast draws
/// one after the content, see [`show_panel`]).
pub fn panel_frame(look: &Look, inner: Margin) -> Frame {
    Frame::NONE
        .fill(look.palette.bg_panel)
        .corner_radius(theme::R_LG)
        .inner_margin(inner)
}

/// Shows `add` in a panel card filling `ui`, then masks the four corners
/// with the ground so full-bleed content can't square them off, and draws
/// High contrast's outline. Returns the card's rect.
pub fn show_panel<R>(
    ui: &mut egui::Ui,
    inner: Margin,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let look = theme::look(ui.ctx());
    let card = ui.available_rect_before_wrap();
    let response = panel_frame(&look, inner).show(ui, |ui| {
        ui.set_min_size((card.size() - inner.sum()).max(Vec2::ZERO));
        add(ui)
    });
    let rect = response.response.rect;
    let painter = ui.painter().with_clip_rect(rect);
    round_corners(&painter, rect, f32::from(theme::R_LG), look.palette.bg_app);
    if look.kind == ThemeKind::HighContrast {
        painter.rect_stroke(
            rect,
            theme::R_LG,
            Stroke::new(1.0, look.palette.border_subtle),
            StrokeKind::Inside,
        );
    }
    response
}

/// The four corner masks of a `rect` rounded by `r`: for each corner, the
/// corner point followed by `n + 1` points along the quarter arc, from one
/// edge to the other. Order: top-left, top-right, bottom-right,
/// bottom-left.
pub fn corner_mask_points(rect: Rect, r: f32, n: usize) -> [Vec<Pos2>; 4] {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let n = n.max(1);
    // (corner, arc centre, start angle) per corner; angles in radians,
    // y down, so 180° points left and 270° points up.
    let corners = [
        (
            rect.left_top(),
            rect.left_top() + Vec2::new(r, r),
            std::f32::consts::PI,
        ),
        (
            rect.right_top(),
            rect.right_top() + Vec2::new(-r, r),
            1.5 * std::f32::consts::PI,
        ),
        (
            rect.right_bottom(),
            rect.right_bottom() + Vec2::new(-r, -r),
            0.0,
        ),
        (
            rect.left_bottom(),
            rect.left_bottom() + Vec2::new(r, -r),
            0.5 * std::f32::consts::PI,
        ),
    ];
    corners.map(|(corner, centre, start)| {
        let mut points = Vec::with_capacity(n + 2);
        points.push(corner);
        for i in 0..=n {
            let a = start + std::f32::consts::FRAC_PI_2 * i as f32 / n as f32;
            points.push(centre + r * Vec2::new(a.cos(), a.sin()));
        }
        points
    })
}

/// Paints the ground over the parts of `rect`'s corners outside a radius
/// `r` (ui-concept §16.3's corner masks): exact, because the ground is
/// flat.
pub fn round_corners(painter: &egui::Painter, rect: Rect, r: f32, ground: Color32) {
    let mut mesh = Mesh::default();
    for fan in corner_mask_points(rect, r, 8) {
        let base = mesh.vertices.len() as u32;
        for p in &fan {
            mesh.colored_vertex(*p, ground);
        }
        // A fan from the corner point over consecutive arc points.
        for i in 1..(fan.len() as u32 - 1) {
            mesh.add_triangle(base, base + i, base + i + 1);
        }
    }
    painter.add(mesh);
}

/// `range` with `inset` taken off both ends (never inverted): the span of
/// the focus line and of the gutter's resize line, which stay clear of the
/// rounded corners.
pub fn inset_span(range: Rangef, inset: f32) -> Rangef {
    if range.span() <= 2.0 * inset {
        let mid = range.center();
        Rangef::new(mid, mid)
    } else {
        Rangef::new(range.min + inset, range.max - inset)
    }
}

/// The focused place's 2 pt accent line along the straight part of the
/// panel's top edge (§3.4).
pub fn focus_line(painter: &egui::Painter, card: Rect, look: &Look) {
    let span = inset_span(card.x_range(), f32::from(theme::R_LG));
    let rect = Rect::from_x_y_ranges(span, Rangef::new(card.top(), card.top() + 2.0));
    painter.rect_filled(rect, 1.0, look.palette.accent);
}

/// A gutter's resize line (§13.17): 2 pt of accent along `along` at
/// `across` (x for a vertical gutter, y for a horizontal one), inset 12 pt
/// from both ends, at `opacity` (its hover fade).
pub fn gutter_line(
    painter: &egui::Painter,
    vertical: bool,
    across: f32,
    along: Rangef,
    look: &Look,
    opacity: f32,
) {
    if opacity <= 0.0 {
        return;
    }
    let span = inset_span(along, f32::from(theme::R_LG));
    let thick = Rangef::new(across - 1.0, across + 1.0);
    let rect = if vertical {
        Rect::from_x_y_ranges(thick, span)
    } else {
        Rect::from_x_y_ranges(span, thick)
    };
    painter.rect_filled(rect, 1.0, look.palette.accent.gamma_multiply(opacity));
}

/// A panel header (§13.16): `panel_header` pt tall across the panel, the
/// title in `type.label` at the side inset, an optional readout in
/// `type.small` `text.tertiary` after it, `trailing` laid out right to
/// left from the far edge, and a hairline along the bottom.
pub fn pane_header<R>(
    ui: &mut egui::Ui,
    title: &str,
    readout: Option<&str>,
    trailing: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let look = theme::look(ui.ctx());
    let m = look.metrics;
    let p = look.palette;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), m.panel_header),
        Sense::hover(),
    );
    let inner = rect.shrink2(egui::vec2(m.side_inset, 0.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let label_style = Type::Label.style(child.style());
    let title_resp = child.add(
        egui::Label::new(
            egui::RichText::new(title)
                .text_style(label_style)
                .color(p.text_primary),
        )
        .truncate(),
    );
    let after_title = title_resp.rect.right() + 12.0;
    let r = child
        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let r = trailing(ui);
            if let Some(text) = readout {
                ui.add_space(8.0);
                // The readout sits right before the trailing controls and
                // gives way to the title when the panel is narrow.
                let room = (ui.available_rect_before_wrap().right() - after_title).max(0.0);
                if room > 24.0 {
                    let height = ui.available_height();
                    ui.allocate_ui_with_layout(
                        egui::vec2(room, height),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text)
                                        .text_style(egui::TextStyle::Small)
                                        .color(p.text_tertiary),
                                )
                                .truncate(),
                            );
                        },
                    );
                }
            }
            r
        })
        .inner;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, p.border_subtle),
    );
    egui::InnerResponse::new(r, ui.interact(rect, ui.id().with(title), Sense::hover()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §13.17: a split stops where the regions beyond it reach their
    /// minimums, and never goes below its own.
    #[test]
    fn splits_stop_at_the_other_regions_minimums() {
        let r = split_range(ASSETS_MIN, 12.0, 1440.0, 600.0);
        assert_eq!((r.min, r.max), (252.0, 840.0));
        // A room too small for everyone: the panel keeps its minimum.
        let r = split_range(INSPECTOR_MIN, 12.0, 500.0, 400.0);
        assert_eq!((r.min, r.max), (292.0, 292.0));
    }

    #[test]
    fn ground_margins_add_up_to_one_gutter() {
        let left = Sides {
            left: Beyond::Window,
            right: Beyond::Panel,
            top: Beyond::Ground,
            bottom: Beyond::Panel,
        };
        let centre = Sides {
            left: Beyond::Panel,
            right: Beyond::Panel,
            top: Beyond::Ground,
            bottom: Beyond::Panel,
        };
        for gutter in [8.0, 4.0] {
            let a = ground_margin(left, gutter);
            let b = ground_margin(centre, gutter);
            assert_eq!(f32::from(a.right) + f32::from(b.left), gutter);
            assert_eq!(f32::from(a.left), gutter, "the window edge");
            assert_eq!(a.top, 0, "the top bar is ground already");
        }
    }

    #[test]
    fn corner_masks_follow_the_arc_inside_the_rect() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(200.0, 100.0));
        let r = 12.0;
        let fans = corner_mask_points(rect, r, 8);
        let centres = [
            rect.left_top() + Vec2::new(r, r),
            rect.right_top() + Vec2::new(-r, r),
            rect.right_bottom() + Vec2::new(-r, -r),
            rect.left_bottom() + Vec2::new(r, -r),
        ];
        let corners = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
        for ((fan, centre), corner) in fans.iter().zip(centres).zip(corners) {
            assert_eq!(fan.len(), 10);
            assert_eq!(fan[0], corner);
            for p in &fan[1..] {
                assert!(((*p - centre).length() - r).abs() < 0.01, "{p:?}");
                assert!(rect.expand(0.01).contains(*p), "{p:?}");
            }
            // Each arc runs from one edge to the other.
            let (first, last) = (fan[1], fan[fan.len() - 1]);
            let on_edge = |p: Pos2| {
                (p.x - rect.left()).abs() < 0.01
                    || (p.x - rect.right()).abs() < 0.01
                    || (p.y - rect.top()).abs() < 0.01
                    || (p.y - rect.bottom()).abs() < 0.01
            };
            assert!(on_edge(first) && on_edge(last));
        }
        // A rect smaller than two radii rounds by half its side.
        let small = Rect::from_min_size(Pos2::ZERO, Vec2::new(10.0, 40.0));
        let fans = corner_mask_points(small, 12.0, 4);
        assert!(((fans[0][1] - Pos2::new(5.0, 5.0)).length() - 5.0).abs() < 0.01);
    }

    #[test]
    fn lines_stay_clear_of_the_corners() {
        let span = inset_span(Rangef::new(0.0, 100.0), 12.0);
        assert_eq!(span, Rangef::new(12.0, 88.0));
        let tiny = inset_span(Rangef::new(0.0, 20.0), 12.0);
        assert_eq!(tiny, Rangef::new(10.0, 10.0));
    }
}
