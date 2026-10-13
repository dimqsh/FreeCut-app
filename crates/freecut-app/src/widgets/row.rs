//! The property row (ui-concept §6.2, §13.2, UI-009): label · control ·
//! gutter, in one of three shapes ([`RowShape`]):
//!
//! - **Form** rows (selects, colours, segmented choices, paired fields):
//!   a label column of 76 pt (64 Compact), then the controls across the
//!   rest, so every row of a section lines up.
//! - **Parameter** rows (numbers, switches, colours): the label hugs the
//!   left and the control hugs the right edge next to the gutter, laid out
//!   right to left; short labels leave air in the middle.
//! - **Stacked** rows (the one value a section is about): the label alone
//!   on the first line, the rail and field 4 pt below.
//!
//! The **label** (`type.body`, `text.secondary`, cut with an ellipsis):
//! - can scrub the value: dragging it moves the value one [`Scrub::step`]
//!   per 2 pt, ×10 with `Shift` and ×0.1 with `Alt`, and reports the value
//!   it asks for in [`RowOutput::scrubbed`]; `Esc` stops the scrub and
//!   reports [`RowOutput::cancelled`]. The scrub keeps its exact value from
//!   frame to frame (the stored value is rounded), so slow and fine scrubs
//!   move evenly both ways; the value shown moves by whole shown digits
//!   toward where the scrub began;
//! - resets the value on a double-click, or from its context menu
//!   ([`RowOutput::reset`]), when the value differs from its default;
//! - shows a 4 pt `accent` dot just before it while the value differs
//!   from its default (shape and colour; absent otherwise).
//!
//! The **gutter** (56 pt, 50 Compact) holds the keyframe navigator on
//! animatable rows. It keeps its width when empty, so controls line up
//! across sections.

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align, CursorIcon, Id, Key, Layout, Modifiers, Rect, Response, Sense, TextStyle,
    UiBuilder,
};

use super::keyframe::{KeyNavOutput, KeyframeNav};
use crate::theme;

/// What a row reports.
#[derive(Debug)]
pub struct RowOutput<R> {
    /// What the control closure returned.
    pub inner: R,
    /// The label's response: a scrub's drag feeds undo coalescing.
    pub label: Response,
    /// The value the label scrub asks for this frame, in shown units:
    /// `None` when it didn't move.
    pub scrubbed: Option<f64>,
    /// `Esc` stopped a scrub this frame.
    pub cancelled: bool,
    /// The label was double-clicked (or its menu's *Reset* picked) while
    /// the value differs from its default.
    pub reset: bool,
    pub keys: KeyNavOutput,
}

/// What a scrubbable label works on, in shown units.
#[derive(Debug, Clone, PartialEq)]
pub struct Scrub {
    /// The value now.
    pub value: f64,
    /// One scrub step (2 pt of label drag).
    pub step: f64,
    pub range: RangeInclusive<f64>,
    /// Digits shown after the point: the scrub moves by whole ones.
    pub decimals: usize,
}

/// A row's shape (ui-concept §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowShape {
    #[default]
    Form,
    Parameter,
    Stacked,
}

/// Where a row's parts go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowLayout {
    pub label: Rect,
    pub control: Rect,
    pub gutter: Rect,
}

/// The space between the control and the gutter, and after a parameter
/// row's label.
const GAP: f32 = 4.0;
/// A stacked row's label line.
const LABEL_LINE: f32 = 18.0;

/// The height of a row of `shape` with controls `control_h` tall.
pub fn row_height(shape: RowShape, control_h: f32) -> f32 {
    match shape {
        RowShape::Form | RowShape::Parameter => control_h,
        RowShape::Stacked => LABEL_LINE + GAP + control_h,
    }
}

/// Where the parts of a row go in `rect`: the gutter on the right; form
/// rows start their controls at the label column; parameter rows give the
/// control the room it asks for (`control_w`) flush against the gutter and
/// the label the rest; stacked rows put the label on its own line.
pub fn row_layout(
    shape: RowShape,
    rect: Rect,
    label_column: f32,
    control_w: f32,
    gutter: f32,
    control_h: f32,
) -> RowLayout {
    let gutter_left = (rect.right() - gutter).max(rect.left());
    let control_right = (gutter_left - GAP).max(rect.left());
    let line = |top: f32| egui::Rangef::new(top, top + control_h);
    match shape {
        RowShape::Form => {
            let start = (rect.left() + label_column).min(control_right);
            RowLayout {
                label: Rect::from_x_y_ranges(rect.left()..=start, rect.y_range()),
                control: Rect::from_x_y_ranges(start..=control_right, rect.y_range()),
                gutter: Rect::from_x_y_ranges(gutter_left..=rect.right(), rect.y_range()),
            }
        }
        RowShape::Parameter => {
            let start = (control_right - control_w).max(rect.left());
            RowLayout {
                label: Rect::from_x_y_ranges(
                    rect.left()..=(start - 2.0 * GAP).max(rect.left()),
                    rect.y_range(),
                ),
                control: Rect::from_x_y_ranges(start..=control_right, rect.y_range()),
                gutter: Rect::from_x_y_ranges(gutter_left..=rect.right(), rect.y_range()),
            }
        }
        RowShape::Stacked => {
            let below = line(rect.top() + LABEL_LINE + GAP);
            RowLayout {
                label: Rect::from_x_y_ranges(
                    rect.left()..=gutter_left,
                    rect.top()..=rect.top() + LABEL_LINE,
                ),
                control: Rect::from_x_y_ranges(rect.left()..=control_right, below),
                gutter: Rect::from_x_y_ranges(gutter_left..=rect.right(), below),
            }
        }
    }
}

/// Whether the row `id` showed its changed dot last frame (for tests and
/// accessibility checks).
#[allow(dead_code)] // read by the inspector's tests
pub fn shows_changed(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id.with("changed")))
        .unwrap_or(false)
}

/// The value a scrub that began at `start` and has reached `exact` asks
/// for: `exact` cut toward `start` to `decimals` digits, so the same
/// distance either way changes the value by the same amount.
pub fn scrub_value(start: f64, exact: f64, decimals: usize) -> f64 {
    // At most 15 digits: a power of ten f64 holds exactly.
    let scale = 10f64.powi(decimals.min(15) as i32);
    let moved = (exact - start) * scale;
    // A hair toward the move, so 0.3 summed from tenths is still 0.3.
    let moved = (moved + 1e-9_f64.copysign(moved)).trunc();
    start + moved / scale
}

/// A property row.
#[derive(Debug)]
pub struct PropRow<'a> {
    id: Id,
    label: &'a str,
    scrub: Option<Scrub>,
    changed: bool,
    keys: Option<KeyframeNav<'a>>,
    hint: Option<&'a str>,
    shape: RowShape,
    control_w: Option<f32>,
}

impl<'a> PropRow<'a> {
    pub fn new(id: Id, label: &'a str) -> PropRow<'a> {
        PropRow {
            id,
            label,
            scrub: None,
            changed: false,
            keys: None,
            hint: None,
            shape: RowShape::Form,
            control_w: None,
        }
    }

    /// Dragging the label scrubs the value (numeric rows).
    pub fn scrub(mut self, scrub: Scrub) -> Self {
        self.scrub = Some(scrub);
        self
    }

    /// The value differs from its default: the changed dot, and the label
    /// resets it on a double-click.
    pub fn changed(mut self, changed: bool) -> Self {
        self.changed = changed;
        self
    }

    pub fn keys(mut self, keys: KeyframeNav<'a>) -> Self {
        self.keys = Some(keys);
        self
    }

    /// Hover text on the label.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// A parameter row whose control takes `control_w` pt at the right
    /// edge; the control closure lays out right to left.
    pub fn parameter(mut self, control_w: f32) -> Self {
        self.shape = RowShape::Parameter;
        self.control_w = Some(control_w);
        self
    }

    /// A two-line row: the label, then the control across the row.
    pub fn stacked(mut self) -> Self {
        self.shape = RowShape::Stacked;
        self
    }

    pub fn show<R>(
        self,
        ui: &mut egui::Ui,
        control: impl FnOnce(&mut egui::Ui) -> R,
    ) -> RowOutput<R> {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let m = look.metrics;
        let control_h = m.row_height;
        let width = ui.available_width();
        // Allocated once the control is drawn: a taller control (the
        // gradient bar) makes the row taller instead of overlapping the next.
        let rect = Rect::from_min_size(
            ui.available_rect_before_wrap().min,
            egui::vec2(width, row_height(self.shape, control_h)),
        );
        // A form label longer than the column ("Change case") pushes its
        // own row's controls right instead of touching them.
        let label_column = if self.shape == RowShape::Form {
            let font = TextStyle::Body.resolve(ui.style());
            let label_w = ui.fonts_mut(|f| {
                f.layout_no_wrap(self.label.to_owned(), font, p.text_secondary)
                    .size()
                    .x
            });
            m.label_column.max(label_w + 2.0 * GAP)
        } else {
            m.label_column
        };
        let parts = row_layout(
            self.shape,
            rect,
            label_column,
            self.control_w.unwrap_or(0.0),
            m.row_gutter,
            control_h,
        );

        // The control first: a parameter row's label takes what is left.
        let layout = match self.shape {
            RowShape::Parameter => Layout::right_to_left(Align::Center),
            RowShape::Form | RowShape::Stacked => Layout::left_to_right(Align::Center),
        };
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id.with("control"))
                .max_rect(parts.control)
                .layout(layout),
        );
        let inner = control(&mut child);
        // Taller controls make the row taller; wider ones never widen the
        // panel.
        let bottom = rect.bottom().max(child.min_rect().bottom());
        ui.allocate_rect(
            Rect::from_x_y_ranges(rect.x_range(), rect.top()..=bottom),
            Sense::hover(),
        );
        let label_rect = match self.shape {
            // A wider control than asked pushes the label back.
            RowShape::Parameter => {
                let used = child.min_rect();
                Rect::from_x_y_ranges(
                    parts.label.left()..=(used.left() - 2.0 * GAP).max(parts.label.left()),
                    parts.label.y_range(),
                )
            }
            RowShape::Form | RowShape::Stacked => parts.label,
        };

        let scrubs = self.scrub.is_some() && ui.is_enabled();
        let sense = if scrubs {
            Sense::CLICK | Sense::DRAG
        } else {
            Sense::CLICK
        };
        let mut label = ui.interact(label_rect, self.id.with("label"), sense);
        if scrubs && (label.hovered() || label.dragged()) {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if ui.is_rect_visible(label_rect) {
            let color = if ui.is_enabled() {
                p.text_secondary
            } else {
                p.text_disabled
            };
            let mut job = egui::text::LayoutJob::simple_singleline(
                self.label.to_owned(),
                TextStyle::Body.resolve(ui.style()),
                color,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(label_rect.width().max(0.0));
            let galley = ui.painter().layout_job(job);
            let at = egui::pos2(
                label_rect.left(),
                label_rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(at, galley, color);
            if self.changed {
                // The changed dot sits in the side inset, before the label.
                ui.painter().circle_filled(
                    egui::pos2(label_rect.left() - 7.0, label_rect.center().y),
                    2.0,
                    p.accent,
                );
            }
        }
        ui.data_mut(|d| d.insert_temp(self.id.with("changed"), self.changed));
        let mut reset = self.changed && label.double_clicked();
        if self.changed {
            label.context_menu(|ui| {
                if ui.button("Reset to default").clicked() {
                    reset = true;
                    ui.close();
                }
            });
        }
        let tip = match (self.hint, self.changed) {
            (Some(hint), true) => Some(format!("{hint}\nDouble-click to reset")),
            (Some(hint), false) => Some(hint.to_owned()),
            (None, true) => Some("Double-click to reset".to_owned()),
            (None, false) => None,
        };
        if let Some(tip) = tip {
            label = label.on_hover_text(tip);
        }

        let mut scrubbed = None;
        let mut cancelled = false;
        // The scrub's start and exact value, kept across its frames.
        let held = self.id.with("scrub");
        // egui drops a drag on Esc: that frame reports it stopped.
        if let Some(scrub) = &self.scrub
            && (label.dragged() || label.drag_stopped())
        {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                ui.ctx().stop_dragging();
                cancelled = true;
            } else {
                let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
                let factor = if shift {
                    10.0
                } else if alt {
                    0.1
                } else {
                    1.0
                };
                let steps = f64::from(label.drag_delta().x) / 2.0 * factor;
                // A new scrub starts from the value now.
                let kept = if label.drag_started() {
                    None
                } else {
                    ui.data(|d| d.get_temp::<(f64, f64)>(held))
                };
                let (start, exact) = kept.unwrap_or((scrub.value, scrub.value));
                let (lo, hi) = (*scrub.range.start(), *scrub.range.end());
                let exact = (exact + steps * scrub.step).clamp(lo, hi);
                if steps != 0.0 {
                    scrubbed = Some(scrub_value(start, exact, scrub.decimals));
                }
                ui.data_mut(|d| d.insert_temp(held, (start, exact)));
            }
        }
        if label.drag_stopped() || cancelled {
            ui.data_mut(|d| d.remove::<(f64, f64)>(held));
        }

        let mut keys = KeyNavOutput::default();
        if let Some(nav) = self.keys {
            let w = nav.width(super::field::compact(ui));
            let at = Rect::from_min_size(
                egui::pos2(parts.gutter.right() - w, parts.gutter.top()),
                egui::vec2(w, parts.gutter.height()),
            );
            let mut gutter = ui.new_child(
                UiBuilder::new()
                    .id_salt(self.id.with("gutter"))
                    .max_rect(at)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            keys = nav.show(&mut gutter);
        }
        RowOutput {
            inner,
            label,
            scrubbed,
            cancelled,
            reset,
            keys,
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Event, Pos2, Vec2};

    use super::*;
    use crate::ui::drag::tests::{Frames, button, modifiers};

    /// §6.2: parameter rows put the control flush against the gutter;
    /// form rows start it at the label column; stacked rows put the label
    /// on a line of its own.
    #[test]
    fn rows_lay_out_their_parts_by_shape() {
        let rect = Rect::from_min_size(Pos2::new(14.0, 0.0), Vec2::new(300.0, 28.0));
        let p = row_layout(RowShape::Parameter, rect, 76.0, 84.0, 56.0, 28.0);
        assert_eq!(p.gutter.left(), 314.0 - 56.0);
        assert_eq!(p.control.right(), p.gutter.left() - 4.0);
        assert_eq!(p.control.width(), 84.0);
        assert_eq!(p.label.left(), 14.0);
        assert!(p.label.right() < p.control.left());
        let f = row_layout(RowShape::Form, rect, 76.0, 0.0, 56.0, 28.0);
        assert_eq!(f.control.left(), 14.0 + 76.0);
        assert_eq!(f.control.right(), p.control.right());
        assert_eq!(f.label.width(), 76.0);
        let tall = Rect::from_min_size(
            rect.min,
            Vec2::new(300.0, row_height(RowShape::Stacked, 28.0)),
        );
        let s = row_layout(RowShape::Stacked, tall, 76.0, 0.0, 56.0, 28.0);
        assert_eq!(s.label.height(), 18.0);
        assert_eq!(s.control.top(), 22.0);
        assert_eq!(s.control.left(), 14.0);
        assert_eq!(s.gutter.top(), s.control.top());
        assert_eq!(row_height(RowShape::Stacked, 28.0), 50.0);
        // Too narrow: nothing inverts.
        let narrow = Rect::from_min_size(rect.min, Vec2::new(40.0, 28.0));
        let n = row_layout(RowShape::Parameter, narrow, 76.0, 84.0, 56.0, 28.0);
        assert!(n.control.width() >= 0.0 && n.label.width() >= 0.0);
    }

    /// The label resets a changed value on a double-click, and only a
    /// changed one.
    #[test]
    fn a_double_click_on_the_label_resets_a_changed_value() {
        for changed in [true, false] {
            let mut frames = Frames::new();
            let mut resets = 0;
            let mut frame = |frames: &mut Frames, events: Vec<Event>| {
                frames.run(events, |ui| {
                    let out = PropRow::new(Id::new("row"), "Opacity")
                        .changed(changed)
                        .parameter(84.0)
                        .show(ui, |ui| ui.label("100"));
                    resets += usize::from(out.reset);
                });
            };
            frame(&mut frames, Vec::new());
            let at = frames
                .ctx
                .read_response(Id::new("row").with("label"))
                .unwrap()
                .rect
                .center();
            frame(&mut frames, vec![Event::PointerMoved(at), button(at, true)]);
            frame(&mut frames, vec![button(at, false)]);
            frame(&mut frames, vec![button(at, true)]);
            frame(&mut frames, vec![button(at, false)]);
            frame(&mut frames, Vec::new());
            assert_eq!(resets, usize::from(changed), "changed {changed}");
            assert_eq!(shows_changed(&frames.ctx, Id::new("row")), changed);
        }
    }

    #[test]
    fn a_scrub_moves_by_whole_shown_digits_toward_its_start() {
        assert_eq!(scrub_value(50.0, 49.95, 1), 50.0);
        assert_eq!(scrub_value(50.0, 50.05, 1), 50.0);
        assert_eq!(scrub_value(50.0, 0.1 + 0.1 + 0.1 + 49.7, 1), 50.0);
        assert!((scrub_value(50.0, 50.0 - 0.1 * 3.0, 1) - 49.7).abs() < 1e-9);
        assert_eq!(scrub_value(14.0, 14.5, 0), 14.0);
        assert_eq!(scrub_value(14.0, 13.5, 0), 14.0);
        assert_eq!(scrub_value(14.0, 12.9, 0), 13.0);
    }

    /// Scrubs a row's label: one 8 pt move to start the drag, then `moves`
    /// frames of `dx` pt each, with `held` modifiers. The stored value is
    /// rounded to `decimals` after every frame, as the cards store it.
    /// Returns the value at the end.
    fn scrubbed(start: f64, decimals: usize, held: Modifiers, dx: f32, moves: usize) -> f64 {
        let mut frames = Frames::new();
        let scale = 10f64.powi(decimals as i32);
        let mut value = start;
        let frame = |frames: &mut Frames, value: &mut f64, events: Vec<Event>| {
            frames.run(events, |ui| {
                let out = PropRow::new(Id::new("row"), "Opacity")
                    .scrub(Scrub {
                        value: *value,
                        step: 1.0,
                        range: 0.0..=100.0,
                        decimals,
                    })
                    .show(ui, |_| ());
                if let Some(v) = out.scrubbed {
                    *value = (v * scale).round() / scale;
                }
            });
        };
        let press = Pos2::new(20.0, 12.0);
        frame(&mut frames, &mut value, vec![modifiers(held)]);
        frame(
            &mut frames,
            &mut value,
            vec![Event::PointerMoved(press), button(press, true)],
        );
        let mut at = press + Vec2::new(8.0 * dx.signum(), 0.0);
        frame(&mut frames, &mut value, vec![Event::PointerMoved(at)]);
        for _ in 0..moves {
            at += Vec2::new(dx, 0.0);
            frame(&mut frames, &mut value, vec![Event::PointerMoved(at)]);
        }
        frame(&mut frames, &mut value, vec![button(at, false)]);
        value
    }

    /// S0: a fine (Alt) scrub of 1 pt per frame moves a per-mille row as
    /// far left as right (40 pt more: 2 %), and a slow normal scrub of
    /// 0.5 pt per frame keeps every half step.
    #[test]
    fn slow_and_fine_scrubs_keep_their_fraction_both_ways() {
        let left = scrubbed(50.0, 1, Modifiers::ALT, -1.0, 40);
        let right = scrubbed(50.0, 1, Modifiers::ALT, 1.0, 40);
        // 48 pt in all: 24 steps of 0.1 %.
        assert!((left - 47.6).abs() < 1e-9, "left {left}");
        assert!((right - 52.4).abs() < 1e-9, "right {right}");
        let slow = scrubbed(50.0, 1, Modifiers::NONE, -0.5, 40);
        // 28 pt: 14 steps of 1 %.
        assert!((slow - 36.0).abs() < 1e-9, "slow {slow}");
    }

    /// S0: whole-number rows scrub evenly both ways: one step per 2 pt.
    #[test]
    fn whole_number_rows_scrub_as_far_left_as_right() {
        let left = scrubbed(14.0, 0, Modifiers::NONE, -1.0, 12);
        let right = scrubbed(14.0, 0, Modifiers::NONE, 1.0, 12);
        assert_eq!((left, right), (4.0, 24.0));
    }
}
