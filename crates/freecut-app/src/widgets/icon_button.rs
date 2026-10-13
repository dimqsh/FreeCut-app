//! The icon button and tool button (ui-concept §13.18): a 26 pt chip
//! (24 Compact), transparent at rest, with checked, danger, accent and
//! disabled states, hover fades of `dur.fast` and the focus ring on
//! keyboard focus only.

use eframe::egui::{self, Align2, Color32, Id, Key, Response, Sense};

use crate::theme::{self, fonts};

/// Whether keyboard focus is being moved by the keyboard (Tab, arrows,
/// F6) rather than by pointer presses: the focus ring shows only then, so
/// a mouse click doesn't leave a ring on a button.
pub fn keyboard_navigating(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(nav_id())).unwrap_or(false)
}

fn nav_id() -> Id {
    Id::new("freecut-keyboard-navigating")
}

/// Notes how focus moves this frame: a pointer press ends keyboard
/// navigation, Tab / arrows / F6 start it. Call once per frame.
pub fn track_keyboard_navigation(ctx: &egui::Context) {
    let (pressed, keys) = ctx.input(|i| {
        let keys = [
            Key::Tab,
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::F6,
        ]
        .iter()
        .any(|k| i.key_pressed(*k));
        (i.pointer.any_pressed(), keys)
    });
    if pressed || keys {
        ctx.data_mut(|d| d.insert_temp(nav_id(), keys && !pressed));
    }
}

/// Whether `response` should show the focus ring.
pub fn keyboard_focus(ui: &egui::Ui, response: &Response) -> bool {
    response.has_focus() && keyboard_navigating(ui.ctx())
}

/// An icon button. Build with [`IconButton::new`], then [`IconButton::show`].
#[derive(Debug, Clone, Copy)]
pub struct IconButton<'a> {
    icon: &'a str,
    tooltip: &'a str,
    glyph: f32,
    checked: bool,
    danger: bool,
    accent: bool,
    enabled: bool,
    /// The chip's side; the density's `icon_chip` when `None`.
    side: Option<f32>,
}

impl<'a> IconButton<'a> {
    /// A toolbar-sized button (16 pt glyph) with a one-sentence tooltip.
    pub fn new(icon: &'a str, tooltip: &'a str) -> IconButton<'a> {
        IconButton {
            icon,
            tooltip,
            glyph: 16.0,
            checked: false,
            danger: false,
            accent: false,
            enabled: true,
            side: None,
        }
    }

    /// A chip `side` pt square instead of the density's (24 pt in track
    /// headers).
    pub fn side(mut self, side: f32) -> Self {
        self.side = Some(side);
        self
    }

    /// The glyph size: 16 in toolbars and the transport, 14 in headers.
    pub fn glyph(mut self, size: f32) -> Self {
        self.glyph = size;
        self
    }

    /// A mode that is on: a well chip with the glyph in `accent.text` and
    /// Phosphor's Fill weight.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Destructive: the chip turns `danger` on hover.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// The primary action of a group (play): an `accent` glyph and an
    /// `overlay.selection` hover chip.
    pub fn accent(mut self, accent: bool) -> Self {
        self.accent = accent;
        self
    }

    /// Disabled buttons show at 40 % and still explain themselves.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> Response {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let side = self.side.unwrap_or(look.metrics.icon_chip);
        let enabled = self.enabled && ui.is_enabled();
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), sense);
        if ui.is_rect_visible(rect) {
            let hovered = enabled && response.hovered();
            let pressed = enabled && response.is_pointer_button_down_on();
            let t = ui.ctx().animate_bool_with_time(
                response.id.with("hover"),
                hovered,
                ui.style().animation_time,
            );
            let (rest, hover_fill) = if self.checked {
                (p.bg_well, p.bg_hover)
            } else if self.danger {
                (Color32::TRANSPARENT, p.danger.gamma_multiply(0.14))
            } else if self.accent {
                (Color32::TRANSPARENT, p.overlay_selection())
            } else {
                (Color32::TRANSPARENT, p.bg_field)
            };
            let fill = if pressed && !self.accent && !self.danger {
                p.bg_active
            } else {
                rest.lerp_to_gamma(hover_fill, t)
            };
            let glyph_rest = if self.checked {
                p.accent_text
            } else if self.accent {
                p.accent
            } else {
                p.text_secondary
            };
            let glyph_hover = if self.danger {
                p.danger
            } else if self.checked || self.accent {
                glyph_rest
            } else {
                p.text_primary
            };
            let mut glyph = glyph_rest.lerp_to_gamma(glyph_hover, t);
            let mut fill = fill;
            if !enabled {
                glyph = glyph.gamma_multiply(0.4);
                fill = fill.gamma_multiply(0.4);
            }
            let painter = ui.painter();
            if fill != Color32::TRANSPARENT {
                painter.rect_filled(rect, theme::R_SM, fill);
            }
            if enabled {
                super::control_outline(ui, rect, theme::R_SM);
            } else {
                // High contrast's outline dims with the rest of the button.
                let border = look.control_border();
                if border != egui::Stroke::NONE {
                    let dim = egui::Stroke::new(border.width, border.color.gamma_multiply(0.4));
                    painter.rect_stroke(rect, theme::R_SM, dim, egui::StrokeKind::Inside);
                }
            }
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                self.icon,
                fonts::icon_font(ui.ctx(), self.glyph, self.checked),
                glyph,
            );
            if keyboard_focus(ui, &response) {
                super::focus_ring(ui, rect, theme::R_SM);
            }
        }
        let label = self.tooltip.to_owned();
        let selected = self.checked;
        response.widget_info(|| {
            if selected {
                egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, true, &label)
            } else {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label)
            }
        });
        response.on_hover_text(self.tooltip)
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, Event, Pos2, RawInput, Rect};

    use super::*;

    #[test]
    fn a_pointer_press_ends_keyboard_navigation_and_tab_starts_it() {
        let ctx = Context::default();
        let run = |events: Vec<Event>| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 100.0))),
                events,
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| track_keyboard_navigation(ui.ctx()));
            // Nothing renders here: drop the font atlas uploads.
            output.textures_delta.clear();
        };
        run(Vec::new());
        assert!(!keyboard_navigating(&ctx));
        run(vec![Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(keyboard_navigating(&ctx));
        run(vec![Event::PointerButton {
            pos: Pos2::new(10.0, 10.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(!keyboard_navigating(&ctx));
    }
}
