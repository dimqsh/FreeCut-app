//! The slider with field (ui-concept §13.3, UI-009), used only for the one
//! value a section is about: a 4 pt rail in `bg.well` with a 1 pt
//! `border.subtle` edge (`r.xs`), the part up to the thumb in `accent`, a
//! 14 pt `text.primary` thumb with a black-40 % rim that grows to 16 pt on
//! hover and drag, the rail's ends inset by half the thumb; then, 8 pt
//! later, the number field with its unit segment. The rail covers the
//! *soft* range; typing may go up to the *hard* range. A press on the rail
//! jumps there and then drags; `Shift` fine-tunes ×0.1.
//!
//! **Gestures.** The composite reports one [`Response`] (rail and field
//! together), whose drag start, drag and release feed
//! [`crate::ui::drag::DragGesture`], so a whole drag is one undo entry.
//! `Esc` during a drag stops it and reports [`SliderOutput::cancelled`]:
//! the caller cancels the gesture, which restores the value and records
//! nothing (UI-009).
//!
//! A value that differs across the selection ([`SliderField::mixed`])
//! shows a dash in the field and no thumb (UI-010).

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Color32, Id, Key, Modifiers, Rect, Response, Sense, Stroke, StrokeKind, WidgetInfo,
};

use super::field::{NumberField, compact, field_width};
use crate::theme;

/// The thumb's diameter at rest and while hovered or dragged.
const THUMB: f32 = 14.0;
const THUMB_HOT: f32 = 16.0;
/// Between the rail and the field.
const FIELD_GAP: f32 = 8.0;

/// The rail's track in a rail rectangle: the ends inset by half the hot
/// thumb, so the thumb never leaves the rail.
pub fn rail_track(rail: Rect) -> Rect {
    rail.shrink2(egui::vec2(THUMB_HOT / 2.0, 0.0))
}

/// Where `value` sits on a rail over `soft`, from 0 to 1 (clamped).
/// Logarithmic rails need a positive range.
pub fn to_fraction(value: f64, soft: &RangeInclusive<f64>, logarithmic: bool) -> f64 {
    let (lo, hi) = (*soft.start(), *soft.end());
    if hi.is_nan() || lo.is_nan() || hi <= lo || !value.is_finite() {
        return 0.0;
    }
    let f = if logarithmic && lo > 0.0 {
        (value.max(lo) / lo).ln() / (hi / lo).ln()
    } else {
        (value - lo) / (hi - lo)
    };
    f.clamp(0.0, 1.0)
}

/// The value at `fraction` (clamped to 0..=1) of a rail over `soft`.
pub fn from_fraction(fraction: f64, soft: &RangeInclusive<f64>, logarithmic: bool) -> f64 {
    let (lo, hi) = (*soft.start(), *soft.end());
    let f = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if logarithmic && lo > 0.0 && hi > lo {
        lo * (hi / lo).powf(f)
    } else {
        lo + (hi - lo) * f
    }
}

/// What the slider reports this frame.
#[derive(Debug)]
pub struct SliderOutput {
    /// The value after this frame's input (the input value when nothing
    /// changed or the drag was cancelled).
    pub value: f64,
    pub changed: bool,
    /// Rail and field together: drives undo coalescing.
    pub response: Response,
    /// The rail's track: 0 % at its left edge, 100 % at its right.
    pub track: Rect,
    /// `Esc` stopped a drag this frame.
    pub cancelled: bool,
    pub double_clicked: bool,
}

/// A slider with field.
#[derive(Debug, Clone)]
pub struct SliderField<'a> {
    id: Id,
    value: f64,
    soft: RangeInclusive<f64>,
    hard: RangeInclusive<f64>,
    logarithmic: bool,
    decimals: usize,
    suffix: &'a str,
    unit: &'a str,
    speed: f64,
    field: bool,
    label: &'a str,
    mixed: bool,
}

impl<'a> SliderField<'a> {
    /// A slider for `value` whose rail and field both cover `range`.
    pub fn new(id: Id, label: &'a str, value: f64, range: RangeInclusive<f64>) -> SliderField<'a> {
        SliderField {
            id,
            value,
            hard: range.clone(),
            soft: range,
            logarithmic: false,
            decimals: 1,
            suffix: "",
            unit: "",
            speed: 1.0,
            field: true,
            label,
            mixed: false,
        }
    }

    /// The selected clips differ (UI-010): a dash, no thumb.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// The rail covers `soft` only; typing reaches the full range.
    pub fn soft(mut self, soft: RangeInclusive<f64>) -> Self {
        self.soft = soft;
        self
    }

    pub fn logarithmic(mut self) -> Self {
        self.logarithmic = true;
        self
    }

    /// The field's unit segment (e.g. "dB").
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    /// A rail without its own field (the caller draws a special one).
    pub fn without_field(mut self) -> Self {
        self.field = false;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> SliderOutput {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let height = look.metrics.control_height;
        let field_core = field_width(!self.unit.trim().is_empty(), compact(ui));
        let field_w = if self.field {
            field_core + FIELD_GAP
        } else {
            0.0
        };
        let rail_w = (ui.available_width() - field_w).max(48.0);
        let (rail_rect, _) = ui.allocate_exact_size(egui::vec2(rail_w, height), Sense::hover());
        if self.field {
            ui.add_space(FIELD_GAP - ui.spacing().item_spacing.x);
        }
        let enabled = ui.is_enabled();
        let rail_sense = if enabled {
            Sense::CLICK | Sense::DRAG
        } else {
            Sense::hover()
        };
        let rail = ui.interact(rail_rect, self.id.with("rail"), rail_sense);
        let track = rail_track(rail_rect);
        let mut value = self.value;
        let mut changed = false;

        // The rail: jump, then drag (Shift fine-tunes ×0.1).
        let frac_id = self.id.with("fraction");
        let pointer_frac = rail
            .interact_pointer_pos()
            .map(|pos| f64::from((pos.x - track.left()) / track.width().max(1.0)));
        if rail.drag_started() || rail.clicked() {
            if let Some(f) = pointer_frac {
                ui.data_mut(|d| d.insert_temp(frac_id, f.clamp(0.0, 1.0)));
                value = from_fraction(f, &self.soft, self.logarithmic);
                changed = true;
            }
        } else if rail.dragged() || rail.drag_stopped() {
            let shift = ui.input(|i| i.modifiers.shift);
            let stored = ui.data(|d| d.get_temp::<f64>(frac_id));
            let f = match (shift, stored, pointer_frac) {
                (true, Some(prev), _) => {
                    let dx = f64::from(rail.drag_delta().x / track.width().max(1.0));
                    prev + dx * 0.1
                }
                (_, _, Some(f)) => f,
                (_, Some(prev), None) => prev,
                (_, None, None) => to_fraction(value, &self.soft, self.logarithmic),
            }
            .clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(frac_id, f));
            let wanted = from_fraction(f, &self.soft, self.logarithmic);
            if wanted != value {
                value = wanted;
                changed = true;
            }
        }

        // The field: typed values reach the hard range.
        let mut response = rail.clone();
        if self.field {
            let key = self.id.with("field");
            let field = ui
                .push_id(key, |ui| {
                    NumberField::new(key, self.value, self.hard.clone())
                        .speed(self.speed)
                        .decimals(self.decimals)
                        .suffix(self.suffix)
                        .unit(self.unit)
                        .mixed(self.mixed)
                        .size(egui::vec2(field_core, height))
                        .show(ui)
                })
                .inner;
            if field.changed {
                value = field.value;
                changed = true;
            }
            response = response.union(field.response);
        }

        // Esc during a drag: stop, keep the value the caller had.
        let mut cancelled = false;
        // egui itself drops a drag on Esc, so the Esc frame reports the
        // drag as stopped.
        if (response.dragged() || response.drag_stopped())
            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            ui.ctx().stop_dragging();
            cancelled = true;
            changed = false;
            value = self.value;
        }

        if ui.is_rect_visible(rail_rect) {
            let shown = to_fraction(value, &self.soft, self.logarithmic) as f32;
            let y = rail_rect.center().y;
            let line = Rect::from_min_max(
                egui::pos2(track.left(), y - 2.0),
                egui::pos2(track.right(), y + 2.0),
            );
            let (fill_color, thumb) = if enabled {
                (p.accent, p.text_primary)
            } else {
                (p.text_disabled, p.text_disabled)
            };
            ui.painter().rect(
                line,
                theme::R_XS,
                p.bg_well,
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Outside,
            );
            super::control_outline(ui, line, theme::R_XS);
            // Mixed values have no one place on the rail until changed.
            if !self.mixed || changed {
                let x = track.left() + track.width() * shown;
                let filled = Rect::from_min_max(line.min, egui::pos2(x, line.max.y));
                ui.painter().rect_filled(filled, theme::R_XS, fill_color);
                let hot = enabled && (rail.hovered() || rail.dragged());
                let t = ui.ctx().animate_bool_with_time(
                    self.id.with("thumb"),
                    hot,
                    ui.style().animation_time,
                );
                let r = egui::lerp(THUMB..=THUMB_HOT, t) / 2.0;
                ui.painter().circle(
                    egui::pos2(x, y),
                    r,
                    thumb,
                    Stroke::new(1.0, Color32::from_black_alpha(102)),
                );
            }
            if super::icon_button::keyboard_focus(ui, &rail) {
                super::focus_ring(ui, rail_rect, theme::R_SM);
            }
        }
        let label = self.label.to_owned();
        rail.widget_info(|| WidgetInfo::slider(enabled, value, &label));
        let double_clicked = rail.double_clicked();
        SliderOutput {
            value,
            changed,
            response,
            track,
            cancelled,
            double_clicked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_map_both_ways_on_linear_and_log_rails() {
        let r = 0.0..=100.0;
        assert_eq!(to_fraction(25.0, &r, false), 0.25);
        assert_eq!(from_fraction(0.25, &r, false), 25.0);
        assert_eq!(to_fraction(250.0, &r, false), 1.0, "past the soft range");
        assert_eq!(to_fraction(f64::NAN, &r, false), 0.0);
        let log = 4.0..=1000.0;
        for v in [4.0, 10.0, 64.0, 500.0, 1000.0] {
            let back = from_fraction(to_fraction(v, &log, true), &log, true);
            assert!((back - v).abs() < 1e-9, "{v} -> {back}");
        }
        assert!((to_fraction(64.0, &log, true) - 0.502).abs() < 0.01);
        assert_eq!(from_fraction(2.0, &r, false), 100.0, "clamped");
    }

    #[test]
    fn the_track_is_inset_by_half_the_hot_thumb() {
        let rail = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 28.0));
        let track = rail_track(rail);
        assert_eq!((track.left(), track.right()), (8.0, 192.0));
    }
}
