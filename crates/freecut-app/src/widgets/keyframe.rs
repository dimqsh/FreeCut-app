//! The keyframe navigator (ui-concept §13.6, TR-028, UI-009): `<` previous
//! key · diamond · `>` next key, in a property row's gutter: three 18 × 24
//! pt targets (16 Compact) 1 pt apart, each with a `bg.field` hover chip.
//! The diamond's state is told by shape: an outline in `text.tertiary` when
//! the property isn't animated, an outline with a centre dot when it is
//! animated but has no key at the playhead, filled `accent` on a key. The
//! arrows are always there on animatable rows and grey out
//! (`text.disabled`) when there is no key that way ([`arrows`]), so the
//! gutter never changes width.

use eframe::egui::{self, Align2, Response, Sense, Shape, Stroke, WidgetInfo, WidgetType};

use crate::theme::{self, fonts, icons};

/// A property's keyframes as seen from the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    NotAnimated,
    Animated {
        /// A key sits at the playhead.
        on_key: bool,
        /// There is a key before / after the playhead.
        prev: bool,
        next: bool,
    },
}

/// What the navigator was asked to do this frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KeyNavOutput {
    /// Add or remove the key at the playhead.
    pub toggle: bool,
    pub prev: bool,
    pub next: bool,
}

/// A keyframe navigator for one property.
#[derive(Debug, Clone, Copy)]
pub struct KeyframeNav<'a> {
    pub property: &'a str,
    pub state: KeyState,
    /// Why keys can't be added now (the diamond is disabled with this
    /// tooltip; the arrows still work).
    pub disabled: Option<&'a str>,
}

/// Whether the previous / next arrows can be used in `state`: never while
/// the property isn't animated, else when a key lies that way.
pub fn arrows(state: KeyState) -> (bool, bool) {
    match state {
        KeyState::NotAnimated => (false, false),
        KeyState::Animated { prev, next, .. } => (prev, next),
    }
}

/// One target's width: 18 pt, 16 in Compact.
pub fn target_width(compact: bool) -> f32 {
    if compact { 16.0 } else { 18.0 }
}

/// The navigator's width: three targets 1 pt apart (56 pt, 50 Compact:
/// the row gutter).
pub fn nav_width(compact: bool) -> f32 {
    3.0 * target_width(compact) + 2.0 * GAP
}

impl KeyframeNav<'_> {
    /// The width it takes in the gutter.
    pub fn width(&self, compact: bool) -> f32 {
        nav_width(compact)
    }

    /// Draws the navigator left to right in `ui`.
    pub fn show(self, ui: &mut egui::Ui) -> KeyNavOutput {
        let mut out = KeyNavOutput::default();
        let p = theme::look(ui.ctx()).palette;
        let w = target_width(super::field::compact(ui));
        let (prev, next) = arrows(self.state);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            out.prev = arrow(ui, w, icons::CARET_LEFT, prev, "Previous keyframe", &p).clicked();
            out.toggle = self.diamond(ui, w).clicked();
            out.next = arrow(ui, w, icons::CARET_RIGHT, next, "Next keyframe", &p).clicked();
        });
        out
    }

    fn diamond(&self, ui: &mut egui::Ui, w: f32) -> Response {
        let p = theme::look(ui.ctx()).palette;
        let enabled = self.disabled.is_none() && ui.is_enabled();
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(w, 24.0), sense);
        if ui.is_rect_visible(rect) {
            hover_chip(ui, &response, rect, enabled);
            let color = if !enabled {
                p.text_disabled
            } else if matches!(self.state, KeyState::NotAnimated) {
                if response.hovered() {
                    p.text_primary
                } else {
                    p.text_tertiary
                }
            } else {
                p.accent
            };
            let c = rect.center();
            let r = 5.0;
            let points = vec![
                c + egui::vec2(0.0, -r),
                c + egui::vec2(r, 0.0),
                c + egui::vec2(0.0, r),
                c + egui::vec2(-r, 0.0),
            ];
            let on_key = matches!(self.state, KeyState::Animated { on_key: true, .. });
            let fill = if on_key {
                color
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter()
                .add(Shape::convex_polygon(points, fill, Stroke::new(1.5, color)));
            if matches!(self.state, KeyState::Animated { on_key: false, .. }) {
                ui.painter().circle_filled(c, 1.5, color);
            }
            if super::icon_button::keyboard_focus(ui, &response) {
                super::focus_ring(ui, rect, theme::R_XS);
            }
        }
        let description = match self.state {
            KeyState::NotAnimated => format!("Keyframe for {}: not animated", self.property),
            KeyState::Animated { on_key: true, .. } => {
                format!("Keyframe for {}: key at the playhead", self.property)
            }
            KeyState::Animated { on_key: false, .. } => {
                format!("Keyframe for {}: none at the playhead", self.property)
            }
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &description));
        match self.disabled {
            Some(reason) => response.on_hover_text(reason),
            None => {
                let tip = match self.state {
                    KeyState::Animated { on_key: true, .. } => {
                        format!("Remove the {} keyframe at the playhead", self.property)
                    }
                    _ => format!("Add a {} keyframe at the playhead", self.property),
                };
                response.on_hover_text(tip)
            }
        }
    }
}

/// The space between the targets.
const GAP: f32 = 1.0;

/// The `bg.field` chip under a hovered target (`r.xs`).
fn hover_chip(ui: &egui::Ui, response: &Response, rect: egui::Rect, enabled: bool) {
    let t = ui.ctx().animate_bool_with_time(
        response.id.with("chip"),
        enabled && response.hovered(),
        ui.style().animation_time,
    );
    if t > 0.0 {
        let p = theme::look(ui.ctx()).palette;
        ui.painter()
            .rect_filled(rect, theme::R_XS, p.bg_field.gamma_multiply(t));
    }
}

fn arrow(
    ui: &mut egui::Ui,
    w: f32,
    icon: &str,
    enabled: bool,
    tip: &str,
    p: &theme::Palette,
) -> Response {
    let enabled = enabled && ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w, 24.0), sense);
    if ui.is_rect_visible(rect) {
        hover_chip(ui, &response, rect, enabled);
        let color = if !enabled {
            p.text_disabled
        } else if response.hovered() {
            p.text_primary
        } else {
            p.text_secondary
        };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            fonts::icon_font(ui.ctx(), 11.0, false),
            color,
        );
    }
    let tip = tip.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &tip));
    if enabled {
        response.on_hover_text(tip)
    } else {
        response.on_hover_text("No keyframe that way")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_grey_out_without_a_key_that_way_and_the_width_never_changes() {
        assert_eq!(arrows(KeyState::NotAnimated), (false, false));
        let s = |prev, next| KeyState::Animated {
            on_key: false,
            prev,
            next,
        };
        assert_eq!(arrows(s(true, false)), (true, false));
        assert_eq!(arrows(s(false, true)), (false, true));
        assert_eq!(nav_width(false), 56.0);
        assert_eq!(nav_width(true), 50.0);
        let nav = |state| KeyframeNav {
            property: "Size",
            state,
            disabled: None,
        };
        assert_eq!(
            nav(KeyState::NotAnimated).width(false),
            nav(s(true, true)).width(false)
        );
    }
}
