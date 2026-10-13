//! The colour field (ui-concept §13.5): an editable **hex field** and a
//! **swatch** that opens the colour popover.
//!
//! - **Hex field:** 86 × 24 pt, `r.sm`, `bg.field`, a 20 pt leading `#`
//!   segment in `bg.well` behind a hairline seam, six digits in the
//!   monospace face. It takes 3 or 6 digits ([`parse_hex`]) and commits on
//!   `Enter` or when it lets go; text that isn't a colour keeps the old
//!   value, and `Esc` puts the old text back. Alpha is kept: opacity has a
//!   row of its own. While focused: a 1 pt `accent` border.
//! - **Swatch:** 24 × 24 pt, `r.sm`, a 1 pt `border.subtle` outline
//!   (`border.strong` on hover), over a checkerboard of 6 pt cells so a
//!   translucent colour reads as one. A click opens the popover 4 pt below,
//!   right edges aligned, 240 pt wide: the saturation / value square with
//!   the hue and alpha strips (egui's picker), then a row of FreeCut's own
//!   palette swatches.
//! - **Mixed** selections (UI-010) show "—" and a hatched swatch.

use eframe::egui::{
    self, Color32, CornerRadius, Id, Key, Popup, PopupCloseBehavior, Rect, RectAlign, Response,
    Sense, Shape, Stroke, StrokeKind, UiBuilder,
    color_picker::{Alpha, color_picker_hsva_2d},
    ecolor::Hsva,
    text::{CCursor, CCursorRange},
};

use super::field::MIXED;
use crate::theme;

/// The hex field's size and its `#` segment.
const HEX_W: f32 = 86.0;
const HEX_H: f32 = 24.0;
const HASH_W: f32 = 20.0;
/// The swatch's side.
const SWATCH: f32 = 24.0;
/// The popover's width and its palette swatches.
const POPOVER_W: f32 = 240.0;
const CHIP: f32 = 20.0;

/// The colour typed as `text`: 3 or 6 hex digits, with or without a
/// leading `#`, any case; `alpha` is kept. `None` when it isn't a colour.
pub fn parse_hex(text: &str, alpha: u8) -> Option<[u8; 4]> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = |s: &str| u8::from_str_radix(s, 16).ok();
    match digits.len() {
        3 => {
            let mut out = [0, 0, 0, alpha];
            for (i, c) in digits.chars().enumerate() {
                let v = value(&c.to_string())?;
                out[i] = v * 17;
            }
            Some(out)
        }
        6 => Some([
            value(&digits[0..2])?,
            value(&digits[2..4])?,
            value(&digits[4..6])?,
            alpha,
        ]),
        _ => None,
    }
}

/// "FFFFFF": the six digits shown in the field.
pub fn hex6([r, g, b, _]: [u8; 4]) -> String {
    format!("{r:02X}{g:02X}{b:02X}")
}

/// A colour field for `value` (unmultiplied sRGBA).
#[derive(Debug, Clone, Copy)]
pub struct ColourField<'a> {
    id: Id,
    value: [u8; 4],
    mixed: bool,
    name: &'a str,
}

/// What the colour field reports.
#[derive(Debug)]
pub struct ColourOutput {
    /// The colour asked for this frame.
    pub picked: Option<[u8; 4]>,
}

impl<'a> ColourField<'a> {
    /// `name` names it for screen readers ("Fill colour").
    pub fn new(id: Id, value: [u8; 4], name: &'a str) -> ColourField<'a> {
        ColourField {
            id,
            value,
            mixed: false,
            name,
        }
    }

    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> ColourOutput {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (_, typed) = self.hex_field(ui);
            ui.add_space(4.0);
            let (_, popover) = self.swatch(ui);
            ColourOutput {
                picked: popover.or(typed),
            }
        })
        .inner
    }

    /// The editable hex field: the colour typed and committed this frame.
    fn hex_field(&self, ui: &mut egui::Ui) -> (Response, Option<[u8; 4]>) {
        let p = theme::look(ui.ctx()).palette;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(HEX_W, HEX_H), Sense::hover());
        let edit_id = self.id.with("hex");
        let draft_id = self.id.with("draft");
        let focused = ui.memory(|m| m.has_focus(edit_id));
        let shown = if self.mixed {
            MIXED.to_owned()
        } else {
            hex6(self.value)
        };
        let mut text = if focused {
            ui.data(|d| d.get_temp::<String>(draft_id))
                .unwrap_or_else(|| shown.clone())
        } else {
            shown.clone()
        };
        let enabled = ui.is_enabled();
        let lit = ui.ctx().animate_bool_with_time(
            self.id.with("hex-lit"),
            enabled && (focused || ui.rect_contains_pointer(rect)),
            ui.style().animation_time,
        );
        let hash = Rect::from_min_size(rect.min, egui::vec2(HASH_W, rect.height()));
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            painter.rect_filled(rect, theme::R_SM, p.bg_field.lerp_to_gamma(p.bg_hover, lit));
            let corners = CornerRadius {
                nw: theme::R_SM,
                sw: theme::R_SM,
                ne: 0,
                se: 0,
            };
            painter.rect_filled(hash, corners, p.bg_well);
            painter.vline(
                hash.right(),
                hash.y_range(),
                Stroke::new(1.0, p.border_subtle),
            );
            painter.text(
                hash.center(),
                egui::Align2::CENTER_CENTER,
                "#",
                egui::TextStyle::Monospace.resolve(ui.style()),
                p.text_tertiary,
            );
        }
        let value_rect = Rect::from_min_max(egui::pos2(hash.right(), rect.top()), rect.max);
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(value_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        super::field::frameless(child.style_mut(), 6.0, value_rect.size());
        let output = egui::TextEdit::singleline(&mut text)
            .id(edit_id)
            .font(egui::TextStyle::Monospace)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::symmetric(6, 0))
            .vertical_align(egui::Align::Center)
            .desired_width(value_rect.width() - 12.0)
            .char_limit(7)
            .show(&mut child);
        let response = output.response.response.clone();
        if response.gained_focus() {
            // Typing replaces the whole value.
            let mut state = output.state.clone();
            let all = CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), edit_id);
        }
        let mut picked = None;
        if response.has_focus() {
            ui.data_mut(|d| d.insert_temp(draft_id, text.clone()));
            ui.painter().rect_stroke(
                rect,
                theme::R_SM,
                Stroke::new(1.0, p.accent),
                StrokeKind::Inside,
            );
        } else {
            super::control_outline(ui, rect, theme::R_SM);
        }
        if response.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(draft_id));
            let escaped = ui.input(|i| i.key_pressed(Key::Escape));
            if !escaped
                && let Some(c) = parse_hex(&text, self.value[3])
                && (self.mixed || c != self.value)
            {
                picked = Some(c);
            }
        }
        let name = format!("{} hex", self.name);
        let response = response.on_hover_text("Type 3 or 6 hex digits, then Enter");
        response.widget_info(|| egui::WidgetInfo::text_edit(enabled, &name, &shown, &text));
        (response, picked)
    }

    /// The swatch and its popover: the colour picked there this frame.
    fn swatch(&self, ui: &mut egui::Ui) -> (Response, Option<[u8; 4]>) {
        let p = theme::look(ui.ctx()).palette;
        let (outer, _) = ui.allocate_exact_size(egui::vec2(SWATCH, SWATCH), Sense::hover());
        let enabled = ui.is_enabled();
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(outer, self.id.with("swatch"), sense);
        if ui.is_rect_visible(outer) {
            let painter = ui.painter().with_clip_rect(outer);
            if self.mixed {
                hatch(&painter, outer, p.bg_field, p.text_tertiary);
            } else {
                checkers(&painter, outer);
                let [r, g, b, a] = self.value;
                painter.rect_filled(
                    outer,
                    theme::R_SM,
                    // The colour being edited (data, not a theme colour).
                    Color32::from(egui::Rgba::from_srgba_unmultiplied(r, g, b, a)),
                );
            }
            let border = if enabled && response.hovered() {
                p.border_strong
            } else {
                p.border_subtle
            };
            ui.painter().rect_stroke(
                outer,
                theme::R_SM,
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
            super::control_outline(ui, outer, theme::R_SM);
            if super::icon_button::keyboard_focus(ui, &response) {
                super::focus_ring(ui, outer, theme::R_SM);
            }
        }
        let name = self.name.to_owned();
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, enabled, &name)
        });
        let response = response.on_hover_text("Pick a colour");

        let popup_id = self.id.with("popover");
        let toggle = response
            .clicked()
            .then(|| egui::SetOpenCommand::Bool(!Popup::is_id_open(ui.ctx(), popup_id)));
        let hsva_id = self.id.with("hsva");
        let mut picked = None;
        Popup::new(popup_id, ui.ctx().clone(), outer, response.layer_id)
            .open_memory(toggle)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .align(RectAlign::BOTTOM_END)
            .gap(4.0)
            .width(POPOVER_W)
            .frame(super::select::list_frame(ui).inner_margin(egui::Margin::same(8)))
            .show(|ui| {
                ui.set_width(POPOVER_W - 16.0);
                ui.spacing_mut().slider_width = POPOVER_W - 16.0;
                // The picker keeps its hue while the colour is grey.
                let mut hsva = ui
                    .data(|d| d.get_temp::<(Hsva, [u8; 4])>(hsva_id))
                    .filter(|(_, rgba)| *rgba == self.value)
                    .map_or_else(|| Hsva::from_srgba_unmultiplied(self.value), |(h, _)| h);
                if color_picker_hsva_2d(ui, &mut hsva, Alpha::OnlyBlend) {
                    let rgba = hsva.to_srgba_unmultiplied();
                    if rgba != self.value || self.mixed {
                        picked = Some(rgba);
                    }
                    ui.data_mut(|d| d.insert_temp(hsva_id, (hsva, rgba)));
                }
                ui.add_space(6.0);
                if let Some(c) = palette(ui, self.id, self.value[3]) {
                    picked = Some(c);
                }
            });
        (response, picked)
    }
}

/// The palette rows: 8 swatches a row. Returns the colour clicked, with
/// `alpha` kept.
fn palette(ui: &mut egui::Ui, id: Id, alpha: u8) -> Option<[u8; 4]> {
    let p = theme::look(ui.ctx()).palette;
    let mut picked = None;
    for row in theme::surface::SWATCHES.chunks(8) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            for (i, colour) in row.iter().enumerate() {
                let [r, g, b, _] = colour.to_array();
                let (rect, _) = ui.allocate_exact_size(egui::vec2(CHIP, CHIP), Sense::hover());
                let response = ui.interact(rect, id.with(("palette", r, g, b, i)), Sense::click());
                ui.painter().rect_filled(rect, theme::R_XS, *colour);
                let border = if response.hovered() {
                    p.border_strong
                } else {
                    p.border_subtle
                };
                ui.painter().rect_stroke(
                    rect,
                    theme::R_XS,
                    Stroke::new(1.0, border),
                    StrokeKind::Inside,
                );
                let hex = format!("#{}", hex6([r, g, b, 255]));
                if response.on_hover_text(hex).clicked() {
                    picked = Some([r, g, b, alpha]);
                }
            }
        });
    }
    picked
}

/// A checkerboard of 6 pt cells under a translucent colour.
fn checkers(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, theme::R_SM, theme::surface::CHECKER_LIGHT);
    let cell = 6.0;
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left() + if row % 2 == 0 { 0.0 } else { cell };
        while x < rect.right() {
            let r = Rect::from_min_size(egui::pos2(x, y), egui::vec2(cell, cell)).intersect(rect);
            painter.rect_filled(r, 0.0, theme::surface::CHECKER_DARK);
            x += 2.0 * cell;
        }
        y += cell;
        row += 1;
    }
}

/// Diagonal hatching for a mixed colour.
fn hatch(painter: &egui::Painter, rect: Rect, fill: Color32, line: Color32) {
    painter.rect_filled(rect, theme::R_SM, fill);
    let step = 5.0;
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        painter.add(Shape::line_segment(
            [
                egui::pos2(x, rect.bottom()),
                egui::pos2(x + rect.height(), rect.top()),
            ],
            Stroke::new(1.0, line),
        ));
        x += step;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Modifiers, Pos2};

    use super::*;
    use crate::ui::drag::tests::{Frames, button, key};

    #[test]
    fn hex_takes_three_or_six_digits_and_keeps_alpha() {
        assert_eq!(parse_hex("abc", 255), Some([0xAA, 0xBB, 0xCC, 255]));
        assert_eq!(parse_hex("#A1B2C3", 128), Some([0xA1, 0xB2, 0xC3, 128]));
        assert_eq!(parse_hex(" ffffff ", 7), Some([255, 255, 255, 7]));
        assert_eq!(parse_hex("12345", 255), None);
        assert_eq!(parse_hex("ggg", 255), None);
        assert_eq!(parse_hex("", 255), None);
        assert_eq!(parse_hex("#+12345", 255), None);
        assert_eq!(hex6([255, 0, 16, 9]), "FF0010");
    }

    /// Types `text` into the hex field and ends with `end`. Returns what was
    /// picked.
    fn typed(text: &str, end: Key) -> Vec<[u8; 4]> {
        let mut frames = Frames::new();
        let mut picked = Vec::new();
        let id = Id::new("colour");
        let value = [255, 255, 255, 128];
        let mut frame = |frames: &mut Frames, events: Vec<Event>| {
            frames.run(events, |ui| {
                ui.horizontal(|ui| {
                    if let Some(c) = ColourField::new(id, value, "Colour").show(ui).picked {
                        picked.push(c);
                    }
                });
            });
        };
        frame(&mut frames, Vec::new());
        let at: Pos2 = frames
            .ctx
            .read_response(id.with("hex"))
            .unwrap()
            .rect
            .center();
        frame(&mut frames, vec![Event::PointerMoved(at), button(at, true)]);
        frame(&mut frames, vec![button(at, false)]);
        frame(&mut frames, Vec::new());
        frame(&mut frames, vec![Event::Text(text.to_owned())]);
        frame(&mut frames, vec![key(end, Modifiers::NONE)]);
        frame(&mut frames, Vec::new());
        picked
    }

    #[test]
    fn a_typed_hex_commits_on_enter_and_esc_keeps_the_old_colour() {
        assert_eq!(typed("f00", Key::Enter), [[255, 0, 0, 128]]);
        assert_eq!(typed("00FF00", Key::Enter), [[0, 255, 0, 128]]);
        assert!(typed("f00", Key::Escape).is_empty());
        assert!(typed("nope", Key::Enter).is_empty(), "not a colour");
        assert!(typed("FFFFFF", Key::Enter).is_empty(), "no change");
    }
}
