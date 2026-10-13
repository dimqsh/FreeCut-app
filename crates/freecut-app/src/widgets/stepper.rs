//! The **−** / **+** stepper (TX-054): two small buttons beside a slider
//! that nudge its value one step per click, ten with `Shift`. Each click is
//! its own undo entry; the caller clamps the result. A small gap keeps
//! them two buttons where controls are outlined (High contrast).

use eframe::egui::{self, Id, Response};

use super::icon_button;
use crate::theme::icons;

/// What a stepper reports this frame.
#[derive(Debug)]
pub struct StepperOutput {
    /// Steps to add: −1 or +1 per click, ×10 with `Shift`; 0 without a
    /// click.
    pub steps: i32,
    pub minus: Response,
    pub plus: Response,
}

/// The steps one click adds: one, or ten with `Shift`, in the clicked
/// direction.
pub fn click_steps(up: bool, shift: bool) -> i32 {
    let n = if shift { 10 } else { 1 };
    if up { n } else { -n }
}

/// `value` moved `steps` steps of `step`, clamped to `min..=max`.
pub fn stepped(value: f64, steps: i32, step: f64, min: f64, max: f64) -> f64 {
    (value + f64::from(steps) * step).clamp(min, max)
}

/// The gap between the − and + buttons: their outlines never touch.
pub const GAP: f32 = 2.0;

/// Draws the two buttons; `less` and `more` are their tooltips
/// ("Smaller", "Larger").
pub fn stepper(ui: &mut egui::Ui, id: Id, less: &str, more: &str) -> StepperOutput {
    let shift = ui.input(|i| i.modifiers.shift);
    let (minus, plus) = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            let minus = ui
                .push_id(id.with("minus"), |ui| {
                    icon_button(ui, icons::MINUS, 18.0, &format!("{less} (Shift: ×10)"))
                })
                .inner;
            let plus = ui
                .push_id(id.with("plus"), |ui| {
                    icon_button(ui, icons::PLUS, 18.0, &format!("{more} (Shift: ×10)"))
                })
                .inner;
            (minus, plus)
        })
        .inner;
    let steps = if minus.clicked() {
        click_steps(false, shift)
    } else if plus.clicked() {
        click_steps(true, shift)
    } else {
        0
    };
    StepperOutput { steps, minus, plus }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::drag::tests::Frames;

    /// High contrast outlines every control: the two buttons keep a gap
    /// so they don't read as one box.
    #[test]
    fn the_buttons_stand_apart() {
        let mut frames = Frames::new();
        let mut rects = None;
        frames.run(Vec::new(), |ui| {
            ui.horizontal(|ui| {
                let out = stepper(ui, Id::new("s"), "Smaller", "Larger");
                rects = Some((out.minus.rect, out.plus.rect));
            });
        });
        let (minus, plus) = rects.unwrap();
        assert!(
            plus.left() - minus.right() >= GAP - 0.01,
            "{minus:?} {plus:?}"
        );
    }

    #[test]
    fn shift_steps_ten_times_further_and_results_clamp() {
        assert_eq!(click_steps(true, false), 1);
        assert_eq!(click_steps(false, false), -1);
        assert_eq!(click_steps(true, true), 10);
        assert_eq!(click_steps(false, true), -10);
        // Size (TX-054): ±1 ref px, ±10 with Shift, clamped to 4–1000.
        assert_eq!(stepped(64.0, 10, 1.0, 4.0, 1000.0), 74.0);
        assert_eq!(stepped(6.0, -10, 1.0, 4.0, 1000.0), 4.0);
        assert_eq!(stepped(995.0, 10, 1.0, 4.0, 1000.0), 1000.0);
        // Spacing: ±1 %, ±10 % with Shift.
        assert_eq!(stepped(-15.0, -10, 1.0, -20.0, 100.0), -20.0);
    }
}
