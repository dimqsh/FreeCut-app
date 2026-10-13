//! The Shelf (ui-concept §8.1, §13.10): the asset panel's header, a strip
//! of tabs with an icon over a one-word label, grouped by intent with thin
//! rules. Tabs share the width; when they don't fit at their minimum they
//! keep it and the strip scrolls with an end button at each side that has
//! more behind it. In Compact the labels drop and tooltips name the tabs.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Key, Mesh, Pos2, Rangef, Rect, Response, Sense, Vec2,
};

use super::icon_button::keyboard_focus;
use super::{control_outline, focus_ring};
use crate::theme::{self, Type, fonts, icons};

/// Between two tabs.
pub const TAB_GAP: f32 = 2.0;
/// A group rule's slot: the 1 pt rule and 4 pt on each side.
pub const RULE_SLOT: f32 = 9.0;
/// The end buttons' overlay width.
const END_W: f32 = 22.0;
const ICON: f32 = 16.0;

/// One tab.
#[derive(Debug, Clone, Copy)]
pub struct ShelfTab<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    /// Tabs of one intent share a group; a rule separates groups.
    pub group: u8,
}

/// Each tab's width for `n` tabs in `available` pt (gaps included, rules
/// already taken off): equal shares, or `min` and `true` (overflow) when a
/// share would be narrower than `min`.
pub fn shelf_tab_widths(available: f32, n: usize, min: f32) -> (f32, bool) {
    if n == 0 {
        return (0.0, false);
    }
    let gaps = (n as f32 - 1.0) * TAB_GAP;
    let share = (available - gaps) / n as f32;
    if share < min {
        (min, true)
    } else {
        (share, false)
    }
}

/// The smallest tab: 56 pt, 40 pt in Compact.
pub fn min_tab_width(compact: bool) -> f32 {
    if compact { 40.0 } else { 56.0 }
}

/// The tab strip's scroll offset after a click on an end button:
/// 60 % of the visible width further, within `0..=content - visible`.
pub fn end_step(offset: f32, visible: f32, content: f32, forward: bool) -> f32 {
    let step = visible * 0.6;
    let max = (content - visible).max(0.0);
    let to = if forward {
        offset + step
    } else {
        offset - step
    };
    to.clamp(0.0, max)
}

/// What the Shelf did this frame.
#[derive(Debug, Default)]
pub struct ShelfOutput {
    /// A tab was chosen.
    pub picked: Option<usize>,
    /// `Down` or `Enter` on the current tab: move into the Drawer.
    pub enter: bool,
}

/// Draws the Shelf across `ui`'s width as the panel header. `locked`
/// keeps every tab but the current one from being chosen (UI-004: no
/// switching while something dragged from the panel is in flight).
pub fn shelf(
    ui: &mut egui::Ui,
    id: Id,
    tabs: &[ShelfTab<'_>],
    current: usize,
    compact: bool,
    locked: bool,
) -> ShelfOutput {
    let mut out = ShelfOutput::default();
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), look.metrics.panel_header),
        Sense::hover(),
    );
    // The hairline along the bottom, full width.
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, p.border_subtle),
    );
    let inner = Rect::from_min_max(
        rect.min + Vec2::new(8.0, 4.0),
        rect.max - Vec2::new(8.0, 5.0),
    );
    if tabs.is_empty() || inner.width() <= 0.0 {
        return out;
    }
    let rules = tabs.windows(2).filter(|w| w[0].group != w[1].group).count() as f32;
    let (tab_w, overflow) = shelf_tab_widths(
        inner.width() - rules * RULE_SLOT,
        tabs.len(),
        min_tab_width(compact),
    );
    let content =
        tab_w * tabs.len() as f32 + TAB_GAP * (tabs.len() as f32 - 1.0) + rules * RULE_SLOT;

    // The scroll offset: a target moved by the end buttons and the wheel,
    // animated over dur.base.
    let target_id = id.with("target");
    let max_offset = (content - inner.width()).max(0.0);
    let mut target = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(target_id))
        .unwrap_or(0.0)
        .clamp(0.0, max_offset);
    if overflow && ui.rect_contains_pointer(inner) {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        let d = if delta.x.abs() > delta.y.abs() {
            delta.x
        } else {
            delta.y
        };
        if d != 0.0 {
            target = (target - d).clamp(0.0, max_offset);
        }
    }
    let dur_base = ui.style().animation_time * (160.0 / 90.0);
    let offset = if overflow {
        ui.ctx()
            .animate_value_with_time(id.with("offset"), target, dur_base)
    } else {
        0.0
    };

    let clip = ui.painter().with_clip_rect(inner.intersect(ui.clip_rect()));
    let mut x = inner.left() - offset;
    let mut rects = Vec::with_capacity(tabs.len());
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            if tabs[i - 1].group != tab.group {
                let rule_x = x + RULE_SLOT / 2.0 - TAB_GAP / 2.0;
                clip.vline(
                    rule_x,
                    Rangef::new(inner.center().y - 10.0, inner.center().y + 10.0),
                    egui::Stroke::new(1.0, p.border_subtle),
                );
                x += RULE_SLOT;
            } else {
                x += TAB_GAP;
            }
        }
        let tab_rect = Rect::from_x_y_ranges(Rangef::new(x, x + tab_w), inner.y_range());
        x += tab_w;
        rects.push(tab_rect);
        let visible = tab_rect.intersect(inner);
        if visible.width() <= 0.0 {
            continue;
        }
        let enabled = !locked || i == current;
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(visible, id.with(i), sense);
        paint_tab(
            ui,
            &clip,
            tab_rect,
            tab,
            i == current,
            enabled,
            &response,
            compact,
        );
        let response = if compact {
            response.on_hover_text(tab.label)
        } else {
            response
        };
        if enabled && response.clicked() {
            out.picked = Some(i);
        }
        if response.has_focus() {
            let (left, right, down) = ui.input(|inp| {
                (
                    inp.key_pressed(Key::ArrowLeft),
                    inp.key_pressed(Key::ArrowRight),
                    inp.key_pressed(Key::ArrowDown) || inp.key_pressed(Key::Enter),
                )
            });
            let step = if left && i > 0 {
                Some(i - 1)
            } else if right && i + 1 < tabs.len() {
                Some(i + 1)
            } else {
                None
            };
            if let Some(to) = step.filter(|_| !locked) {
                out.picked = Some(to);
                ui.memory_mut(|m| m.request_focus(id.with(to)));
            }
            out.enter |= down && i == current;
        }
    }
    // Keep the chosen tab in view.
    if let Some(to) = out.picked
        && overflow
        && let Some(r) = rects.get(to)
    {
        let left = r.left() + offset - inner.left();
        let right = r.right() + offset - inner.left();
        if left < target {
            target = left;
        } else if right > target + inner.width() {
            target = right - inner.width();
        }
    }

    if overflow {
        if offset > 0.5 {
            let r = Rect::from_x_y_ranges(
                Rangef::new(inner.left(), inner.left() + END_W),
                inner.y_range(),
            );
            if end_button(ui, id.with("back"), r, false).clicked() {
                target = end_step(target, inner.width(), content, false);
            }
        }
        if offset < max_offset - 0.5 {
            let r = Rect::from_x_y_ranges(
                Rangef::new(inner.right() - END_W, inner.right()),
                inner.y_range(),
            );
            if end_button(ui, id.with("forward"), r, true).clicked() {
                target = end_step(target, inner.width(), content, true);
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(target_id, target));
    out
}

#[allow(clippy::too_many_arguments)] // one tab's whole state
fn paint_tab(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    tab: &ShelfTab<'_>,
    current: bool,
    enabled: bool,
    response: &Response,
    compact: bool,
) {
    let p = theme::look(ui.ctx()).palette;
    let hovered = enabled && response.hovered();
    let t = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        hovered,
        ui.style().animation_time,
    );
    if current {
        painter.rect_filled(rect, theme::R_SM, p.bg_field);
    }
    let ink = if !enabled {
        p.text_disabled
    } else if current {
        p.text_primary
    } else {
        p.text_secondary.lerp_to_gamma(p.text_primary, t)
    };
    let icon_font = fonts::icon_font(ui.ctx(), ICON, false);
    if compact {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tab.icon,
            icon_font,
            ink,
        );
    } else {
        let font = Type::Caption.style(ui.style()).resolve(ui.style());
        let label = super::side_list::one_line(ui, tab.label, font, ink, rect.width() - 8.0);
        let block = ICON + 2.0 + label.size().y;
        let top = rect.center().y - block / 2.0;
        painter.text(
            Pos2::new(rect.center().x, top),
            Align2::CENTER_TOP,
            tab.icon,
            icon_font,
            ink,
        );
        painter.galley(
            Pos2::new(rect.center().x - label.size().x / 2.0, top + ICON + 2.0),
            label,
            ink,
        );
    }
    if current {
        control_outline(ui, rect, theme::R_SM);
    }
    if keyboard_focus(ui, response) {
        focus_ring(ui, rect.shrink(1.0), theme::R_SM);
    }
    let label = tab.label.to_owned();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, current, &label)
    });
}

/// An end button: a fade from the panel colour over the strip's end with a
/// 16 pt chevron square.
fn end_button(ui: &egui::Ui, id: Id, rect: Rect, forward: bool) -> Response {
    let p = theme::look(ui.ctx()).palette;
    // The fade: opaque over the outer 60 %, then to transparent.
    let mut mesh = Mesh::default();
    let solid = rect.width() * 0.6;
    let (outer, edge, inner) = if forward {
        (rect.right(), rect.right() - solid, rect.left())
    } else {
        (rect.left(), rect.left() + solid, rect.right())
    };
    let clear = Color32::TRANSPARENT;
    let quad = |mesh: &mut Mesh, x0: f32, x1: f32, c0: Color32, c1: Color32| {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(x0, rect.top()), c0);
        mesh.colored_vertex(Pos2::new(x1, rect.top()), c1);
        mesh.colored_vertex(Pos2::new(x1, rect.bottom()), c1);
        mesh.colored_vertex(Pos2::new(x0, rect.bottom()), c0);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    };
    quad(&mut mesh, outer, edge, p.bg_panel, p.bg_panel);
    quad(&mut mesh, edge, inner, p.bg_panel, clear);
    ui.painter().add(mesh);
    let square = Rect::from_center_size(
        Pos2::new(
            if forward {
                rect.right() - 8.0
            } else {
                rect.left() + 8.0
            },
            rect.center().y,
        ),
        Vec2::splat(16.0),
    );
    let response = ui
        .interact(square.expand(3.0), id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let fill = if response.hovered() {
        p.bg_hover
    } else {
        p.bg_field
    };
    ui.painter().rect_filled(square, theme::R_XS, fill);
    ui.painter().text(
        square.center(),
        Align2::CENTER_CENTER,
        if forward {
            icons::CARET_RIGHT
        } else {
            icons::CARET_LEFT
        },
        fonts::icon_font(ui.ctx(), 10.0, false),
        p.text_primary,
    );
    let label = if forward { "More tabs" } else { "Earlier tabs" };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_the_width_or_overflow_at_their_minimum() {
        let (w, over) = shelf_tab_widths(3.0 * 100.0 + 2.0 * TAB_GAP, 3, 56.0);
        assert!((w - 100.0).abs() < 1e-4);
        assert!(!over);
        let (w, over) = shelf_tab_widths(150.0, 3, 56.0);
        assert_eq!((w, over), (56.0, true));
        // Compact's minimum lets more tabs fit.
        assert!(!shelf_tab_widths(150.0, 3, 40.0).1);
        assert_eq!(shelf_tab_widths(100.0, 0, 56.0), (0.0, false));
        assert_eq!(min_tab_width(false), 56.0);
        assert_eq!(min_tab_width(true), 40.0);
    }

    #[test]
    fn end_buttons_move_by_60_percent_within_the_content() {
        assert!((end_step(0.0, 100.0, 300.0, true) - 60.0).abs() < 1e-4);
        assert_eq!(end_step(180.0, 100.0, 300.0, true), 200.0);
        assert_eq!(end_step(30.0, 100.0, 300.0, false), 0.0);
        assert_eq!(end_step(0.0, 400.0, 300.0, true), 0.0);
    }
}
