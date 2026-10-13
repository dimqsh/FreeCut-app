//! The number field (ui-concept §13.19, UI-009): a painted `bg.field`
//! frame (`bg.hover` while hovered or focused) holding egui's `DragValue`,
//! an optional leading prefix letter (X, Y, W, H) and an optional trailing
//! **unit segment** (`%`, `°`, `s`, `px` …) in `bg.well` behind a hairline
//! seam, so the value is typed bare and its unit said once. Typed values
//! are committed on `Enter` or when the field lets go; a value that differs
//! across the selection shows a dash (UI-010).
//!
//! **Mixed values.** While a mixed field is typed in, it holds a stand-in
//! value no clip has, so whatever is typed counts as a change, even the
//! first clip's own value, and goes to every selected clip. Its id is
//! recorded under `key`, so the stand-in stays until the typing is
//! committed, however focus left.

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align, Color32, CornerRadius, DragValue, Id, Layout, Rect, Response, Sense, Stroke,
    StrokeKind, UiBuilder,
};

use crate::theme::{self, Type};

/// What a mixed field holds while typed in: no stored value is this.
const STAND_IN: f64 = f64::MAX;

/// UI-010's dash for a mixed value.
pub const MIXED: &str = "—";

/// A field's width (§13.19): 64 pt bare and 84 pt with a unit; Compact
/// 56 and 72.
pub fn field_width(has_unit: bool, compact: bool) -> f32 {
    match (has_unit, compact) {
        (false, false) => 64.0,
        (true, false) => 84.0,
        (false, true) => 56.0,
        (true, true) => 72.0,
    }
}

/// The unit segment is as wide as its text plus 7 pt on each side.
pub fn unit_segment_width(text_w: f32) -> f32 {
    text_w + 14.0
}

/// The parts of a field of `rect`: the value's area and the unit segment
/// (empty when there is no unit), for a prefix `prefix_w` wide and a unit
/// segment `unit_w` wide.
pub fn field_parts(rect: Rect, prefix_w: f32, unit_w: f32) -> (Rect, Rect) {
    let unit_w = unit_w.clamp(0.0, rect.width());
    let seam = rect.right() - unit_w;
    let value = Rect::from_min_max(
        egui::pos2((rect.left() + prefix_w).min(seam), rect.top()),
        egui::pos2(seam, rect.bottom()),
    );
    let unit = Rect::from_min_max(egui::pos2(seam, rect.top()), rect.max);
    (value, unit)
}

/// Whether the current density is Compact (24 pt controls).
pub(crate) fn compact(ui: &egui::Ui) -> bool {
    theme::look(ui.ctx()).metrics.control_height < 28.0
}

/// What a field reports this frame.
#[derive(Debug)]
pub struct FieldOutput {
    pub response: Response,
    /// The value asked for (the input value when nothing changed).
    pub value: f64,
    pub changed: bool,
}

/// A numeric field.
#[derive(Debug, Clone)]
pub struct NumberField<'a> {
    key: Id,
    value: f64,
    range: RangeInclusive<f64>,
    speed: f64,
    decimals: usize,
    prefix: &'a str,
    suffix: &'a str,
    unit: &'a str,
    mixed: bool,
    size: Option<egui::Vec2>,
}

impl<'a> NumberField<'a> {
    /// A field for `value`; `key` is the row's stable id.
    pub fn new(key: Id, value: f64, range: RangeInclusive<f64>) -> NumberField<'a> {
        NumberField {
            key,
            value,
            range,
            speed: 1.0,
            decimals: 1,
            prefix: "",
            suffix: "",
            unit: "",
            mixed: false,
            size: None,
        }
    }

    /// The drag speed, per point.
    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    /// A leading letter (X, Y, W, H) in `type.caption` `text.tertiary`.
    pub fn prefix(mut self, prefix: &'a str) -> Self {
        self.prefix = prefix;
        self
    }

    /// Text after the value inside the value itself; prefer [`Self::unit`].
    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = suffix;
        self
    }

    /// The unit segment's text (`%`, `°`, `px` …); empty for none.
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit.trim();
        self
    }

    /// The selected clips differ: show a dash (UI-010).
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    pub fn size(mut self, size: egui::Vec2) -> Self {
        self.size = Some(size);
        self
    }

    /// Only the width; the height is the density's control height.
    pub fn width(mut self, width: f32) -> Self {
        let height = self.size.map_or(0.0, |s| s.y);
        self.size = Some(egui::vec2(width, height));
        self
    }

    /// The id of the `DragValue` drawn last under `key` (its focus).
    pub fn field_id(ctx: &egui::Context, key: Id) -> Option<Id> {
        ctx.data(|d| d.get_temp::<Id>(key.with("field-id")))
    }

    pub fn show(self, ui: &mut egui::Ui) -> FieldOutput {
        let typing_key = self.key.with("typing");
        let was_typing = ui.data(|d| d.get_temp::<bool>(typing_key)).unwrap_or(false);
        let focused = NumberField::field_id(ui.ctx(), self.key)
            .is_some_and(|id| ui.memory(|m| m.has_focus(id)));
        let stand_in = self.mixed && (focused || was_typing);
        let mut value = if stand_in { STAND_IN } else { self.value };

        let mut field = DragValue::new(&mut value)
            .range(self.range.clone())
            .speed(self.speed)
            .max_decimals(self.decimals)
            .suffix(self.suffix)
            .update_while_editing(false);
        if self.mixed {
            field = field.custom_formatter(|_, _| MIXED.to_owned());
        }
        // Only what the user enters is clamped: a stored value outside the
        // range (an In longer than its clip leaves room for) shows as it
        // is, and drawing never edits it.
        field = field.clamp_existing_to_range(false);
        let frame = Framed {
            key: self.key,
            unit: self.unit,
            prefix: self.prefix,
            size: self.size,
            corners: None,
            focused,
        };
        let response = frame.show(ui, |ui| ui.add(field));

        let typing = self.mixed && response.has_focus();
        ui.data_mut(|d| {
            d.insert_temp(self.key.with("field-id"), response.id);
            d.insert_temp(typing_key, typing);
        });
        // A text edit reports a change per key typed; the value changes
        // only when the typing is committed.
        let held = if stand_in { STAND_IN } else { self.value };
        let changed = response.changed() && value != held;
        FieldOutput {
            value: if changed { value } else { self.value },
            changed,
            response,
        }
    }
}

/// The painted frame of a number field around any one-line widget (the
/// field's `DragValue`, the Volume field's decibels).
#[derive(Debug, Clone, Copy)]
pub struct Framed<'a> {
    /// The field's stable id (its hover fade).
    pub key: Id,
    pub unit: &'a str,
    pub prefix: &'a str,
    /// The size; the density's width and height when `None` (a zero
    /// height also means the control height).
    pub size: Option<egui::Vec2>,
    pub corners: Option<CornerRadius>,
    /// The widget inside has keyboard focus (lights the frame at once).
    pub focused: bool,
}

impl<'a> Framed<'a> {
    pub fn new(key: Id, unit: &'a str) -> Framed<'a> {
        Framed {
            key,
            unit: unit.trim(),
            prefix: "",
            size: None,
            corners: None,
            focused: false,
        }
    }

    /// Paints the frame, the prefix and the unit segment, and adds the
    /// widget `add` makes in the value's area, styled frameless. Returns the
    /// widget's response.
    pub fn show(self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> Response) -> Response {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let height = look.metrics.control_height;
        let mut size = self
            .size
            .unwrap_or_else(|| egui::vec2(field_width(!self.unit.is_empty(), compact(ui)), height));
        if size.y <= 0.0 {
            size.y = height;
        }
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let corners = self.corners.unwrap_or(CornerRadius::same(theme::R_MD));
        let caption = Type::Caption.style(ui.style()).resolve(ui.style());
        let unit_galley = (!self.unit.is_empty()).then(|| {
            ui.painter()
                .layout_no_wrap(self.unit.to_owned(), caption.clone(), p.text_tertiary)
        });
        let prefix_galley = (!self.prefix.trim().is_empty()).then(|| {
            ui.painter().layout_no_wrap(
                self.prefix.trim().to_owned(),
                caption.clone(),
                p.text_tertiary,
            )
        });
        let unit_w = unit_galley
            .as_ref()
            .map_or(0.0, |g| unit_segment_width(g.size().x));
        let prefix_w = prefix_galley.as_ref().map_or(0.0, |g| g.size().x + 8.0);
        let (value_rect, unit_rect) = field_parts(rect, prefix_w, unit_w);

        let enabled = ui.is_enabled();
        let hovered = enabled && ui.rect_contains_pointer(rect);
        let lit = ui.ctx().animate_bool_with_time(
            self.key.with("lit"),
            hovered || self.focused,
            ui.style().animation_time,
        );
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let opacity = if enabled { 1.0 } else { 0.4 };
            painter.rect_filled(
                rect,
                corners,
                p.bg_field
                    .lerp_to_gamma(p.bg_hover, lit)
                    .gamma_multiply(opacity),
            );
            if let Some(galley) = unit_galley {
                let unit_corners = CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: corners.ne,
                    se: corners.se,
                };
                painter.rect_filled(unit_rect, unit_corners, p.bg_well.gamma_multiply(opacity));
                painter.vline(
                    unit_rect.left(),
                    unit_rect.y_range(),
                    Stroke::new(1.0, p.border_subtle),
                );
                let color = if enabled {
                    p.text_tertiary
                } else {
                    p.text_disabled
                };
                painter.galley(unit_rect.center() - galley.size() / 2.0, galley, color);
            }
            if let Some(galley) = prefix_galley {
                let at = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
                painter.galley(at, galley, p.text_tertiary);
            }
        }

        let pad = if prefix_w > 0.0 { 2.0 } else { 8.0 };
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(value_rect)
                // egui's DragValue centres its value; typing is centred too,
                // so the text doesn't jump when the field is clicked.
                .layout(
                    Layout::left_to_right(Align::Center)
                        .with_main_align(Align::Center)
                        .with_main_justify(true),
                ),
        );
        frameless(child.style_mut(), pad, value_rect.size());
        let response = add(&mut child);

        // The frame shows focus: a 1 pt accent border while typing.
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                corners,
                Stroke::new(1.0, p.accent),
                StrokeKind::Inside,
            );
        } else {
            super::control_outline(ui, rect, theme::R_MD);
        }
        response
    }
}

/// Styles a child `Ui` so stock widgets inside a painted frame (the
/// field's `DragValue`, the hex field's `TextEdit`) draw no frame of their
/// own: transparent fills, no outlines, text `pad` pt from the left.
pub(crate) fn frameless(style: &mut egui::Style, pad: f32, size: egui::Vec2) {
    style.spacing.button_padding = egui::vec2(pad, 0.0);
    style.spacing.interact_size = size;
    let v = &mut style.visuals;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = Color32::TRANSPARENT;
        w.weak_bg_fill = Color32::TRANSPARENT;
        w.bg_stroke = Stroke::NONE;
        w.expansion = 0.0;
    }
    v.text_edit_bg_color = Some(Color32::TRANSPARENT);
    v.extreme_bg_color = Color32::TRANSPARENT;
    // Width 0: no focus frame; the colour still paints selected text.
    v.selection.stroke.width = 0.0;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Key, Modifiers};

    use super::*;
    use crate::ui::drag::tests::{Frames, button, key};

    /// Types `text` into a field showing `value` (mixed or not) and presses
    /// `Enter`. Returns what each frame reported as a change.
    fn typed(value: f64, mixed: bool, text: &str) -> Vec<f64> {
        let mut frames = Frames::new();
        let mut changes = Vec::new();
        let key_id = Id::new("field");
        let mut frame = |frames: &mut Frames, events: Vec<Event>| {
            frames.run(events, |ui| {
                let out = NumberField::new(key_id, value, 0.0..=100.0)
                    .mixed(mixed)
                    .unit("%")
                    .show(ui);
                if out.changed {
                    changes.push(out.value);
                }
            });
        };
        frame(&mut frames, Vec::new());
        // A click puts the field in typing mode with its text selected.
        let id = NumberField::field_id(&frames.ctx, key_id).unwrap();
        let at = frames.ctx.read_response(id).unwrap().rect.center();
        frame(&mut frames, vec![Event::PointerMoved(at), button(at, true)]);
        frame(&mut frames, vec![button(at, false)]);
        frame(&mut frames, Vec::new());
        frame(&mut frames, vec![Event::Text(text.to_owned())]);
        frame(&mut frames, vec![key(Key::Enter, Modifiers::NONE)]);
        frame(&mut frames, Vec::new());
        changes
    }

    /// S1 (TX-054 with several clips): typing the first clip's own value
    /// into a mixed field is a change, so it reaches every clip.
    #[test]
    fn typing_into_a_mixed_field_always_reports_the_value() {
        assert_eq!(typed(64.0, true, "64"), [64.0]);
        assert_eq!(typed(64.0, true, "30"), [30.0]);
        // Not mixed: the same value changes nothing.
        assert_eq!(typed(64.0, false, "64"), Vec::<f64>::new());
        assert_eq!(typed(64.0, false, "30"), [30.0]);
    }

    /// §13.19: 64 / 84 pt (56 / 72 Compact); the unit segment is its text
    /// plus 14 pt and sits on the trailing edge.
    #[test]
    fn widths_and_parts_follow_the_concept() {
        assert_eq!(field_width(false, false), 64.0);
        assert_eq!(field_width(true, false), 84.0);
        assert_eq!(field_width(false, true), 56.0);
        assert_eq!(field_width(true, true), 72.0);
        assert_eq!(unit_segment_width(9.0), 23.0);
        let rect = Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(84.0, 28.0));
        let (value, unit) = field_parts(rect, 0.0, 23.0);
        assert_eq!(unit.left(), 71.0);
        assert_eq!(unit.right(), rect.right());
        assert_eq!(value.right(), unit.left());
        assert_eq!(value.left(), 10.0);
        let (value, unit) = field_parts(rect, 14.0, 0.0);
        assert_eq!(value.left(), 24.0);
        assert_eq!(unit.width(), 0.0);
        // A unit wider than the field never inverts the parts.
        let (value, unit) = field_parts(rect, 0.0, 200.0);
        assert!(value.width() >= 0.0 && unit.width() <= rect.width());
    }
}
