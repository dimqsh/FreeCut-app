//! Property rows for the inspector's sections (ui-concept §6.2, §13.2),
//! built from the widgets: numeric rows (a number field with its unit
//! segment hugging the gutter, optional −/+ steppers, and for the one value
//! a section is about a two-line row with a rail), colour, choice and
//! switch rows. Each pushes what its widgets asked for
//! onto the card's [`Steps`]; the card commits them after drawing, so a
//! drag is one undo entry and `Esc` during it reverts with none (UI-009).
//!
//! Values are stored fixed-point ([`PerMille`], [`SignedPerMille`],
//! [`DeciDegrees`]); the rows show them as percentages and degrees, the
//! float conversion happening here at the UI edge.
//!
//! A row whose value differs across the selected clips ([`At::mixed`],
//! UI-010) shows a dash and lights no segment, shows its changed dot (and
//! its label resets it), and sends whatever is picked or typed, even the
//! first clip's own value, so every clip ends up with it (TX-008, TX-054).
//!
//! **Keyframes (TR-027, TR-028, TX-021, ui-concept §13.6).** With one clip
//! shown ([`At::keys`]), a row whose edit sets a property that takes keys
//! gets the keyframe navigator in its gutter: the property is read from the
//! edit the row makes ([`edit_prop`]), so every row of every card finds its
//! own. The diamond adds or removes a key at the playhead (an
//! [`Edit::Keys`] step) and the arrows move the playhead
//! ([`KeyView::seek`]). With several clips selected the gutters are empty
//! (ui-concept §6.4).

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::sync::Arc;

use eframe::egui::{self, Id, Key, Modifiers, Response};
use freecut_timeline::model::{
    Clip, DeciDegrees, PerMille, PropPath, PropValue, RefPx, Rgba8, SignedPerMille,
};
use freecut_timeline::time::Tick;

use super::inspector::lens::{CardId, SelectionKind, widget_id};
use super::props::{Edit, Merge, Steps};
use crate::prop_keys::{self, KeyNav};
use crate::widgets::colour::ColourField;
use crate::widgets::field::{NumberField, compact, field_width};
use crate::widgets::keyframe::{KeyNavOutput, KeyframeNav};
use crate::widgets::row::{PropRow, RowOutput, Scrub};
use crate::widgets::segmented::{Segment, Segmented};
use crate::widgets::slider::SliderField;
use crate::widgets::stepper::{StepperOutput, stepped, stepper};
use crate::widgets::toggle::toggle;

/// The one clip the inspector shows, seen from the playhead: what the rows'
/// keyframe navigators read (TR-028). Built once per frame.
#[derive(Debug)]
pub(crate) struct KeyView {
    pub clip: Arc<Clip>,
    pub playhead: Tick,
    /// Where a navigator arrow asked the playhead to go this frame.
    pub seek: Cell<Option<Tick>>,
}

impl KeyView {
    pub fn new(clip: Arc<Clip>, playhead: Tick) -> KeyView {
        KeyView {
            clip,
            playhead,
            seek: Cell::new(None),
        }
    }

    /// The navigator of `prop` at the playhead.
    pub fn nav(&self, prop: PropPath) -> KeyNav {
        prop_keys::key_nav(&self.clip, prop, self.playhead)
    }
}

/// Where rows sit: the selection kind and the card, for stable widget ids
/// (ui-concept §16.3), whether the next row's value is mixed, and the clip
/// whose keys the navigators show.
#[derive(Debug, Clone, Copy)]
pub(crate) struct At<'a> {
    pub kind: SelectionKind,
    pub card: CardId,
    /// The row's value differs across the selected clips (UI-010).
    pub mixed: bool,
    /// One clip shown: its keys (none with several clips, ui-concept §6.4).
    pub keys: Option<&'a KeyView>,
}

impl<'a> At<'a> {
    pub fn new(kind: SelectionKind, card: CardId) -> At<'a> {
        At {
            kind,
            card,
            mixed: false,
            keys: None,
        }
    }

    /// The same place with keyframe navigators for `keys`' clip.
    pub fn with_keys(self, keys: Option<&'a KeyView>) -> At<'a> {
        At { keys, ..self }
    }

    /// The same place, for a row whose value is `mixed` or not.
    pub fn mixed(self, mixed: bool) -> At<'a> {
        At { mixed, ..self }
    }

    pub fn id(self, row: &str) -> Id {
        widget_id(self.kind, self.card, row)
    }

    /// The navigator for a row labelled `label` that sets `prop`: `None`
    /// without one clip shown or when the property takes no keys.
    pub fn nav(
        self,
        label: &'a str,
        prop: Option<PropPath>,
    ) -> Option<(PropPath, KeyframeNav<'a>)> {
        let view = self.keys?;
        let prop = prop.filter(|p| p.is_animatable())?;
        let nav = view.nav(prop);
        let row = KeyframeNav {
            property: label,
            state: nav.state,
            disabled: prop_keys::add_blocked(&nav),
        };
        Some((prop, row))
    }

    /// What a navigator asked for this frame: a key added or removed at the
    /// playhead (its own undo entry), or the playhead moved to the previous
    /// or next key.
    pub fn keys_clicked(self, prop: PropPath, out: KeyNavOutput, steps: &mut Steps) {
        let Some(view) = self.keys else {
            return;
        };
        let nav = view.nav(prop);
        if out.toggle {
            let edit = prop_keys::toggle_edit(&nav, view.playhead);
            steps.edit(Edit::Keys { prop, edit }, Merge::Own);
        }
        if out.prev && nav.prev.is_some() {
            view.seek.set(nav.prev);
        }
        if out.next && nav.next.is_some() {
            view.seek.set(nav.next);
        }
    }
}

/// The keyable property an inspector edit sets, if any (TX-021, TR-027).
pub(crate) fn edit_prop(edit: &Edit) -> Option<(PropPath, Option<PropValue>)> {
    match edit {
        Edit::Style(e) => prop_keys::style_prop(e).map(|(p, v)| (p, Some(v))),
        Edit::Layout(e) => prop_keys::layout_prop(e).map(|(p, v)| (p, Some(v))),
        Edit::Transform(e) => prop_keys::transform_prop(e).map(|(p, v)| (p, Some(v))),
        Edit::Keys { prop, .. } => Some((*prop, None)),
        Edit::Each(edits) => edits.first().and_then(|(_, e)| edit_prop(e)),
        Edit::Color(e) => prop_keys::color_prop(e).map(|(p, v)| (p, Some(v))),
        Edit::Reset(_) | Edit::Animation { .. } => None,
    }
}

/// A numeric row, in the units it shows.
#[derive(Debug, Clone)]
pub(crate) struct Numeric<'a> {
    pub row: &'a str,
    pub label: &'a str,
    pub value: f64,
    pub default: f64,
    /// What typing may reach.
    pub range: RangeInclusive<f64>,
    /// What the rail covers, when narrower.
    pub soft: Option<RangeInclusive<f64>>,
    pub suffix: &'a str,
    pub decimals: usize,
    /// One label-scrub step, in shown units.
    pub scrub: f64,
    /// −/+ steppers adding this much per click (TX-054).
    pub stepper: Option<f64>,
    pub logarithmic: bool,
    /// Show the keyframe navigator when the row's edit sets a property
    /// that takes keys (TX-021).
    pub keys: bool,
    /// The one value its section is about: a two-line row with a rail
    /// (ui-concept §6.2).
    pub headline: bool,
    /// The field's drag speed, per point.
    pub speed: f64,
    /// The label's tooltip.
    pub hint: Option<&'a str>,
}

impl<'a> Numeric<'a> {
    pub fn new(
        row: &'a str,
        label: &'a str,
        value: f64,
        default: f64,
        range: RangeInclusive<f64>,
    ) -> Numeric<'a> {
        Numeric {
            row,
            label,
            value,
            default,
            range,
            soft: None,
            suffix: "",
            decimals: 1,
            scrub: 1.0,
            stepper: None,
            logarithmic: false,
            keys: true,
            headline: false,
            speed: 1.0,
            hint: None,
        }
    }

    /// A tooltip on the label saying what the value does.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    /// The one value its section is about: label above, rail and field.
    pub fn headline(mut self) -> Self {
        self.headline = true;
        self
    }

    pub fn soft(mut self, soft: RangeInclusive<f64>) -> Self {
        self.soft = Some(soft);
        self
    }

    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = suffix;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn stepper(mut self, step: f64) -> Self {
        self.stepper = Some(step);
        self
    }

    pub fn logarithmic(mut self) -> Self {
        self.logarithmic = true;
        self
    }

    pub fn no_keys(mut self) -> Self {
        self.keys = false;
        self
    }
}

/// The width of the −/+ steppers: two 24 pt buttons and their gap.
const STEPPERS: f32 = 48.0 + crate::widgets::stepper::GAP;

/// What a numeric row's rail and field did this frame, together.
pub(crate) struct Slid {
    pub value: f64,
    pub changed: bool,
    /// Rail and field: drives undo coalescing.
    pub response: Response,
    /// `Esc` stopped a drag of the rail or the field.
    pub cancelled: bool,
    /// The rail's track (0 % to 100 %), when there is a rail.
    pub track: Option<egui::Rect>,
}

/// The number field of a row keyed `id`, in shown units, with its unit
/// segment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn number(
    ui: &mut egui::Ui,
    id: Id,
    value: f64,
    range: RangeInclusive<f64>,
    unit: &str,
    decimals: usize,
    speed: f64,
    mixed: bool,
) -> crate::widgets::field::FieldOutput {
    let key = id.with("field");
    ui.push_id(key, |ui| {
        NumberField::new(key, value, range)
            .decimals(decimals)
            .speed(speed)
            .unit(unit)
            .mixed(mixed)
            .show(ui)
    })
    .inner
}

/// `Esc` while `response` is dragged stops the drag: the caller cancels
/// the gesture (UI-009). egui drops a drag on Esc itself, so that frame
/// reports it stopped.
pub(crate) fn esc_stops_drag(ui: &egui::Ui, response: &Response) -> bool {
    if (response.dragged() || response.drag_stopped())
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        ui.ctx().stop_dragging();
        return true;
    }
    false
}

/// The width a parameter row's control asks for: the field, and the
/// steppers before it.
pub(crate) fn parameter_width(ui: &egui::Ui, unit: &str, steppers: bool) -> f32 {
    let field = field_width(!unit.trim().is_empty(), compact(ui));
    let spacing = ui.spacing().item_spacing.x;
    if steppers {
        field + spacing + STEPPERS
    } else {
        field
    }
}

/// A numeric row's control: (rail,) steppers and field. Parameter rows
/// draw right to left (the field first, against the gutter); headline rows
/// left to right on their second line.
fn controls(
    ui: &mut egui::Ui,
    id: Id,
    n: &Numeric<'_>,
    mixed: bool,
) -> (Slid, Option<StepperOutput>) {
    let unit = n.suffix.trim();
    let speed = n.speed;
    let steppers = |ui: &mut egui::Ui| {
        n.stepper.map(|_| {
            ui.allocate_ui_with_layout(
                egui::vec2(STEPPERS, ui.available_height()),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| stepper(ui, id.with("stepper"), "Less", "More"),
            )
            .inner
        })
    };
    if n.headline {
        let field_w = field_width(!unit.is_empty(), compact(ui));
        let spacing = ui.spacing().item_spacing.x;
        let reserved = field_w
            + 8.0
            + if n.stepper.is_some() {
                STEPPERS + spacing
            } else {
                0.0
            };
        let rail_w = (ui.available_width() - reserved).max(48.0);
        let rail = ui
            .allocate_ui_with_layout(
                egui::vec2(rail_w, ui.available_height()),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let mut s = SliderField::new(id, n.label, n.value, n.range.clone())
                        .mixed(mixed)
                        .without_field();
                    if let Some(soft) = &n.soft {
                        s = s.soft(soft.clone());
                    }
                    if n.logarithmic {
                        s = s.logarithmic();
                    }
                    s.show(ui)
                },
            )
            .inner;
        ui.add_space(8.0 - spacing);
        let stepped = steppers(ui);
        let field = number(
            ui,
            id,
            n.value,
            n.range.clone(),
            unit,
            n.decimals,
            speed,
            mixed,
        );
        let cancelled = rail.cancelled || esc_stops_drag(ui, &field.response);
        let (value, changed) = if field.changed {
            (field.value, true)
        } else {
            (rail.value, rail.changed)
        };
        let slid = Slid {
            value,
            changed: changed && !cancelled,
            response: rail.response.union(field.response),
            cancelled,
            track: Some(rail.track),
        };
        (slid, stepped)
    } else {
        let field = number(
            ui,
            id,
            n.value,
            n.range.clone(),
            unit,
            n.decimals,
            speed,
            mixed,
        );
        let stepped = steppers(ui);
        let cancelled = esc_stops_drag(ui, &field.response);
        let slid = Slid {
            value: field.value,
            changed: field.changed && !cancelled,
            response: field.response,
            cancelled,
            track: None,
        };
        (slid, stepped)
    }
}

/// One frame of a numeric row. `to_edit` turns a wanted value (shown
/// units) into an edit, or `None` when it leaves the stored value as it is.
/// Returns the rail's track on a headline row.
pub(crate) fn numeric(
    ui: &mut egui::Ui,
    at: At<'_>,
    n: Numeric<'_>,
    steps: &mut Steps,
    to_edit: impl Fn(f64) -> Option<Edit>,
) -> Option<egui::Rect> {
    let id = at.id(n.row);
    let mut row = PropRow::new(id, n.label)
        .scrub(Scrub {
            value: n.value,
            step: n.scrub,
            range: n.range.clone(),
            decimals: n.decimals,
        })
        .changed(at.mixed || n.value != n.default);
    if let Some(hint) = n.hint {
        row = row.hint(hint);
    }
    row = if n.headline {
        row.stacked()
    } else {
        row.parameter(parameter_width(ui, n.suffix, n.stepper.is_some()))
    };
    // The property the row sets: from an edit it would make (one value
    // of the default and the two ends differs from the stored one).
    let prop = if n.keys && at.keys.is_some() {
        [n.default, *n.range.start(), *n.range.end()]
            .into_iter()
            .find_map(&to_edit)
            .and_then(|e| edit_prop(&e))
            .map(|(p, _)| p)
    } else {
        None
    };
    let nav = at.nav(n.label, prop);
    if let Some((_, nav)) = nav {
        row = row.keys(nav);
    }
    let range = n.range.clone();
    let out = row.show(ui, |ui| controls(ui, id, &n, at.mixed));
    let RowOutput {
        inner: (slid, stepped_out),
        label,
        scrubbed,
        cancelled,
        reset,
        keys,
    } = out;
    if let Some((prop, _)) = nav {
        at.keys_clicked(prop, keys, steps);
    }
    let track = slid.track;
    if cancelled || slid.cancelled {
        steps.cancel();
        return track;
    }
    if reset && let Some(edit) = to_edit(n.default) {
        steps.edit(edit, Merge::Own);
    }
    let clamp = |v: f64| v.clamp(*range.start(), *range.end());
    let scrubbed = scrubbed.and_then(|v| to_edit(clamp(v)));
    steps.button(label, scrubbed);
    let slid_edit = slid.changed.then(|| to_edit(clamp(slid.value))).flatten();
    steps.button(slid.response, slid_edit);
    if let (Some(s), Some(step)) = (stepped_out, n.stepper) {
        let wanted =
            (s.steps != 0).then(|| stepped(n.value, s.steps, step, *range.start(), *range.end()));
        let edit = wanted.and_then(&to_edit);
        let (clicked, other) = if s.steps < 0 {
            (s.minus, s.plus)
        } else {
            (s.plus, s.minus)
        };
        steps.button(clicked, edit);
        steps.finish(other);
    }
    track
}

/// A per-mille value shown as a percentage, `lo..=hi` per-mille.
#[allow(clippy::too_many_arguments)]
pub(crate) fn percent(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    stored: PerMille,
    default: PerMille,
    (lo, hi): (u16, u16),
    make: impl Fn(PerMille) -> Edit,
) {
    let n = Numeric::new(
        row,
        label,
        stored.as_f64() * 100.0,
        default.as_f64() * 100.0,
        f64::from(lo) / 10.0..=f64::from(hi) / 10.0,
    )
    .suffix(" %");
    numeric(ui, at, n, steps, |v| {
        let wanted = per_mille_in(v * 10.0, lo, hi);
        (at.mixed || wanted != stored).then(|| make(wanted))
    });
}

/// A signed per-mille value shown in tenths (−100 to +100 for the curve
/// and the tail bend, or a percentage with `suffix` " %").
#[allow(clippy::too_many_arguments)]
pub(crate) fn signed(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    n: Numeric<'_>,
    stored: SignedPerMille,
    (lo, hi): (i16, i16),
    make: impl Fn(SignedPerMille) -> Edit,
) {
    numeric(ui, at, n, steps, |v| {
        let wanted = signed_per_mille_in(v * 10.0, lo, hi);
        (at.mixed || wanted != stored).then(|| make(wanted))
    });
}

/// An angle in degrees, 0–359.9°, clockwise from the right (TX-011).
#[allow(clippy::too_many_arguments)]
pub(crate) fn angle(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    stored: DeciDegrees,
    default: DeciDegrees,
    make: impl Fn(DeciDegrees) -> Edit,
) {
    let n = Numeric::new(row, label, stored.degrees(), default.degrees(), 0.0..=359.9)
        .suffix("°")
        .decimals(1);
    numeric(ui, at, n, steps, |v| {
        let wanted = DeciDegrees::new(round_i32(v * 10.0));
        (at.mixed || wanted != stored).then(|| make(wanted))
    });
}

/// A small whole number (Shout's spikes, Thought's trail circles).
#[allow(clippy::too_many_arguments)]
pub(crate) fn count(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    stored: u8,
    default: u8,
    (lo, hi): (u8, u8),
    make: impl Fn(u8) -> Edit,
) {
    let n = Numeric::new(
        row,
        label,
        f64::from(stored),
        f64::from(default),
        f64::from(lo)..=f64::from(hi),
    )
    .decimals(0)
    .no_keys();
    numeric(ui, at, n, steps, |v| {
        // Clamped to a u8 range first, so the cast is exact.
        let wanted = round_i32(v).clamp(i32::from(lo), i32::from(hi)) as u8;
        (at.mixed || wanted != stored).then(|| make(wanted))
    });
}

/// A colour row (UI-009's colour field, ui-concept §13.5): the label,
/// then the hex field and the swatch with its popover against the gutter.
/// Changes while the pointer is held in the picker merge into one entry; a
/// typed hex or a palette swatch is one entry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn color(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    stored: Rgba8,
    default: Rgba8,
    make: impl Fn(Rgba8) -> Edit,
) {
    let prop = edit_prop(&make(stored)).map(|(p, _)| p);
    let nav = at.nav(label, prop);
    let mut row_w = PropRow::new(at.id(row), label)
        .changed(at.mixed || stored != default)
        .parameter(COLOUR_W);
    if let Some((_, n)) = nav {
        row_w = row_w.keys(n);
    }
    let out = row_w.show(ui, |ui| {
        colour_field(ui, at.id(row), stored, at.mixed, label)
    });
    if let Some((prop, _)) = nav {
        at.keys_clicked(prop, out.keys, steps);
    }
    if out.reset {
        steps.edit(make(default), Merge::Own);
    }
    if let Some(wanted) = out.inner
        && (at.mixed || wanted != stored)
    {
        steps.edit(make(wanted), Merge::WhileHeld);
    }
}

/// The colour field of a row keyed `id`, laid out left to right (hex, then
/// swatch) in a parameter row's right-to-left control area. Returns the
/// colour asked for this frame.
pub(crate) fn colour_field(
    ui: &mut egui::Ui,
    id: Id,
    stored: Rgba8,
    mixed: bool,
    name: &str,
) -> Option<Rgba8> {
    let rgba = [stored.r, stored.g, stored.b, stored.a];
    let picked = ui
        .allocate_ui_with_layout(
            egui::vec2(COLOUR_W, ui.available_height()),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ColourField::new(id.with("colour"), rgba, name)
                    .mixed(mixed)
                    .show(ui)
                    .picked
            },
        )
        .inner;
    picked.map(|[r, g, b, a]| Rgba8::new(r, g, b, a))
}

/// The colour field's width: the hex field, 4 pt, the swatch.
pub(crate) const COLOUR_W: f32 = 86.0 + 4.0 + 24.0;

/// A choice among a few values as a segmented control. A stored value
/// that isn't among `values`, or a mixed one, lights no segment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    items: &[Segment<'_>],
    values: &[T],
    stored: T,
    default: T,
    icons_only: bool,
    make: impl Fn(T) -> Edit,
) {
    let id = at.id(row);
    let current = values
        .iter()
        .position(|v| *v == stored)
        .filter(|_| !at.mixed)
        .unwrap_or(values.len());
    let mut row = PropRow::new(id, label).changed(at.mixed || stored != default);
    // Too many words for the form column: the choice gets a line of its own.
    if Segmented::new(id, items, current)
        .icons_only(icons_only)
        .natural(ui)
        > form_room(ui)
    {
        row = row.stacked();
    }
    let out = row.show(ui, |ui| {
        Segmented::new(id, items, current)
            .icons_only(icons_only)
            .fill_width()
            .show(ui)
    });
    if out.reset {
        steps.edit(make(default), Merge::Own);
    }
    let picked = out.inner.picked.and_then(|i| values.get(i).copied());
    steps.button(
        out.inner.response,
        picked.filter(|v| at.mixed || *v != stored).map(&make),
    );
}

/// The width a form row leaves its controls (ui-concept §6.2): the row
/// minus the label column, the gutter and the gap before it.
pub(crate) fn form_room(ui: &egui::Ui) -> f32 {
    let m = crate::theme::look(ui.ctx()).metrics;
    ui.available_width() - m.label_column - m.row_gutter - 4.0
}

/// An on/off row with a toggle switch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn switch(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    stored: bool,
    default: bool,
    make: impl Fn(bool) -> Edit,
) {
    let id = at.id(row);
    let out = PropRow::new(id, label)
        .changed(at.mixed || stored != default)
        .parameter(32.0)
        .show(ui, |ui| {
            let mut on = stored;
            toggle(ui, id.with("toggle"), &mut on, label)
        });
    if out.reset {
        steps.edit(make(default), Merge::Own);
    }
    let response = out.inner;
    let edit = response.changed().then(|| make(!stored));
    steps.button(response, edit);
}

/// A read-only row (ui-concept §6.2): the label, and `value` against the
/// gutter in `text.primary` (the monospace face for timecodes).
pub(crate) fn readout(ui: &mut egui::Ui, id: Id, label: &str, value: &str, mono: bool) {
    let p = crate::theme::look(ui.ctx()).palette;
    let font = if mono {
        egui::TextStyle::Monospace
    } else {
        egui::TextStyle::Body
    }
    .resolve(ui.style());
    let width = ui
        .painter()
        .layout_no_wrap(value.to_owned(), font.clone(), p.text_primary)
        .size()
        .x;
    PropRow::new(id, label).parameter(width).show(ui, |ui| {
        ui.label(egui::RichText::new(value).font(font).color(p.text_primary));
    });
}

/// "#FFFFFF", with the alpha when not opaque: "#FFFFFF · 50 %".
pub(crate) fn hex(c: Rgba8) -> String {
    let hex = format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b);
    if c.a == 255 {
        hex
    } else {
        format!("{hex} · {:.0} %", f64::from(c.a) / 255.0 * 100.0)
    }
}

/// `v` rounded to the nearest integer, saturating at the `i32` range.
pub(crate) fn round_i32(v: f64) -> i32 {
    let r = v.round();
    if r.is_nan() {
        0
    } else {
        // Clamped into range first, so the cast is exact.
        r.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
}

/// A length typed in ref px, as stored (1/100 ref px).
pub(crate) fn ref_px(px: f64) -> RefPx {
    RefPx::from_hundredths(round_i32(px * 100.0))
}

/// A per-mille value, rounded and clamped to `lo..=hi`.
pub(crate) fn per_mille_in(v: f64, lo: u16, hi: u16) -> PerMille {
    // Clamped to a u16 range first, so the cast is exact.
    PerMille::new(round_i32(v).clamp(i32::from(lo), i32::from(hi)) as u16)
}

/// A signed per-mille value, rounded and clamped to `lo..=hi`.
pub(crate) fn signed_per_mille_in(v: f64, lo: i16, hi: i16) -> SignedPerMille {
    // Clamped to an i16 range first, so the cast is exact.
    SignedPerMille::new(round_i32(v).clamp(i32::from(lo), i32::from(hi)) as i16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_values_round_to_the_stored_steps() {
        assert_eq!(ref_px(64.004), RefPx::from_hundredths(6400));
        assert_eq!(per_mille_in(1234.0, 0, 1000), PerMille::ONE);
        assert_eq!(per_mille_in(-3.0, 0, 1000), PerMille::ZERO);
        assert_eq!(per_mille_in(50.0, 100, 2000).get(), 100);
        assert_eq!(signed_per_mille_in(-555.4, -200, 1000).get(), -200);
        assert_eq!(signed_per_mille_in(123.6, -1000, 1000).get(), 124);
        assert_eq!(round_i32(f64::NAN), 0);
        assert_eq!(round_i32(1e20), i32::MAX);
        assert_eq!(hex(Rgba8::new(255, 0, 16, 255)), "#FF0010");
        assert_eq!(hex(Rgba8::new(0, 0, 0, 128)), "#000000 · 50 %");
    }
}
