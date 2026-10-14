//! The Look lens's Masks page (CL-031): the chips of the clip's colour
//! masks, **Add mask** with the four shapes, and the chosen mask's rows.
//!
//! One clip at a time: masks are numbered per clip, so with several clips
//! selected the page says so instead. The chosen mask is view state (egui
//! memory, per clip), never part of the document; the Stage draws its
//! outline and handles while the page shows it (TR-019).
//!
//! Every row edits through the card's [`Steps`]: a drag is one undo entry,
//! `Esc` reverts it, and a keyed value sets its key at the playhead
//! (TR-027) because [`ColorEdit::SetMaskParam`] names its property. Shape,
//! combine mode, invert and the link change the stored mask whole
//! ([`ColorEdit::SetMask`]), never the keyed values at the playhead.

use eframe::egui;
use freecut_timeline::edit::ColorEdit;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Angle, ColorMask, ColorMaskParam, ColorStack, EXPAND_LIMIT, MASK_BAND_MAX, MASK_CENTER_LIMIT,
    MASK_RATIO_MAX, MASK_ROTATION_LIMIT, MASK_SIZE_MAX, MAX_COLOR_MASKS, Mask, MaskCombine,
    MaskParam, MaskPoint, MaskShape, MaskShapeKind, PerMille, PropValue,
};

use crate::theme::{self, icons};
use crate::ui::look::{
    MASKS_FULL, NO_MASKS, ONE_CLIP_FOR_MASKS, mask_chip_label, mask_mode_hint, mask_mode_label,
    mask_param_hint, mask_param_label, mask_shape_hint, mask_shape_label,
};
use crate::ui::props::{Edit, Merge, Steps};
use crate::ui::rows::{self, At, Numeric};
use crate::widgets::icon_button::IconButton;
use crate::widgets::mask_shape::{paint_glyph, shape_picker};
use crate::widgets::row::PropRow;
use crate::widgets::select::{Select, SelectOption};
use crate::widgets::side_list::chip_at;
use crate::widgets::toggle::toggle;

/// The memory key of the chosen mask of `clip`.
fn chosen_key(clip: ClipId) -> egui::Id {
    egui::Id::new(("freecut-color-mask", clip))
}

/// The mask of `stack` the page shows for `clip`: the one chosen before
/// while it still exists, else the last.
pub(super) fn chosen(ui: &egui::Ui, clip: ClipId, stack: &ColorStack) -> Option<u16> {
    let remembered: Option<u16> = ui.data(|d| d.get_temp(chosen_key(clip)));
    remembered
        .filter(|id| stack.mask(*id).is_some())
        .or_else(|| stack.masks.last().map(|m| m.id))
}

fn choose(ui: &egui::Ui, clip: ClipId, id: u16) {
    remember_chosen(ui.ctx(), clip, id);
}

/// Makes mask `id` the one the page shows for `clip` (view state).
pub(crate) fn remember_chosen(ctx: &egui::Context, clip: ClipId, id: u16) {
    ctx.data_mut(|d| d.insert_temp(chosen_key(clip), id));
}

/// The id the next added mask takes: the smallest the stack doesn't use
/// (the model's own rule).
fn next_id(stack: &ColorStack) -> Option<u16> {
    (1..=u16::MAX).find(|id| stack.mask(*id).is_none())
}

/// The Masks section's body. `shown` is the stack at the playhead (keyed
/// values evaluated), `stored` the clip's own (static values). Returns
/// the mask the Stage should show, when one clip is edited.
pub(super) fn masks_body(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    clip: Option<ClipId>,
    shown: &ColorStack,
    stored: &ColorStack,
) -> Option<u16> {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let Some(clip) = clip else {
        ui.label(egui::RichText::new(ONE_CLIP_FOR_MASKS).color(p.text_secondary));
        return None;
    };
    let mut current = chosen(ui, clip, shown);
    // The chips and Add mask, wrapping in a narrow panel.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        let count = shown.masks.len();
        for (i, m) in shown.masks.iter().enumerate() {
            let selected = current == Some(m.id);
            let chip_id = at.id("mask-chip").with(m.id);
            let response = chip_at(ui, chip_id, &mask_chip_label(i, m.mask.kind()), selected)
                .on_hover_text(mask_mode_hint(m.mode));
            if response.clicked() {
                current = Some(m.id);
                choose(ui, clip, m.id);
            }
            response.context_menu(|ui| {
                if ui
                    .add_enabled(i > 0, egui::Button::new("Move earlier"))
                    .clicked()
                {
                    steps.edit(
                        Edit::Color(ColorEdit::ReorderMask {
                            id: m.id,
                            to: i - 1,
                        }),
                        Merge::Own,
                    );
                    ui.close();
                }
                if ui
                    .add_enabled(i + 1 < count, egui::Button::new("Move later"))
                    .clicked()
                {
                    steps.edit(
                        Edit::Color(ColorEdit::ReorderMask {
                            id: m.id,
                            to: i + 1,
                        }),
                        Merge::Own,
                    );
                    ui.close();
                }
                if ui.button("Delete mask").clicked() {
                    steps.edit(Edit::Color(ColorEdit::RemoveMask(m.id)), Merge::Own);
                    ui.close();
                }
            });
        }
        let full = count >= MAX_COLOR_MASKS;
        let label = crate::widgets::icon_then_text(ui, icons::PLUS, "Add mask");
        let add = super::look::id_button(ui, at.id("add-mask"), !full, label)
            .on_hover_text("Adds a shape; the grade then applies only inside the masks")
            .on_disabled_hover_text(MASKS_FULL);
        egui::Popup::menu(&add).show(|ui| {
            for kind in MaskShapeKind::ALL {
                if add_row(ui, at, kind).clicked() {
                    steps.edit(Edit::Color(ColorEdit::AddMask(Mask::new(kind))), Merge::Own);
                    if let Some(id) = next_id(stored) {
                        choose(ui, clip, id);
                    }
                    ui.close();
                }
            }
        });
    });
    let (Some(id), Some(m)) = (current, current.and_then(|id| shown.mask(id)).copied()) else {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(NO_MASKS).color(p.text_secondary));
        return None;
    };
    // Shape, mode, invert and link change the stored mask whole.
    let base = stored.mask(id).copied().unwrap_or(m);
    ui.add_space(4.0);
    let picker = PropRow::new(at.id("mask-shape"), "Shape")
        .hint("The mask's shape; centre, size, turn and feather stay when it changes")
        .show(ui, |ui| {
            shape_picker(ui, at.id("mask-shapes"), m.mask.kind())
        });
    if let Some(kind) = picker.inner.picked {
        let next = ColorMask {
            mask: base.mask.with_kind(kind),
            ..base
        };
        steps.edit(Edit::Color(ColorEdit::SetMask(next)), Merge::Own);
    }
    let modes: Vec<SelectOption<'_>> = MaskCombine::ALL
        .iter()
        .map(|mode| SelectOption::new(mask_mode_label(*mode)))
        .collect();
    let index = MaskCombine::ALL.iter().position(|x| *x == m.mode);
    let select_id = at.id("mask-mode");
    let mode = PropRow::new(select_id, "Combine")
        .hint(mask_mode_hint(m.mode))
        .show(ui, |ui| {
            Select::new(select_id.with("select"), &modes, index)
                .fill_width()
                .show(ui)
                .picked
        });
    if let Some(mode) = mode.inner.and_then(|i| MaskCombine::ALL.get(i).copied())
        && mode != m.mode
    {
        steps.edit(Edit::Color(ColorEdit::SetMaskMode { id, mode }), Merge::Own);
    }
    switch_row(
        ui,
        at,
        steps,
        "mask-invert",
        "Invert",
        base.mask.invert,
        |on| ColorMask {
            mask: Mask {
                invert: on,
                ..base.mask
            },
            ..base
        },
        "Grades outside the shape instead of inside",
    );
    mask_rows(ui, at, steps, id, &m, &base);
    // Order and delete.
    let index = shown.masks.iter().position(|x| x.id == id).unwrap_or(0);
    ui.horizontal(|ui| {
        let earlier = IconButton::new(icons::CARET_LEFT, "Move earlier in the list")
            .enabled(index > 0)
            .show(ui);
        if earlier.clicked() {
            steps.edit(
                Edit::Color(ColorEdit::ReorderMask { id, to: index - 1 }),
                Merge::Own,
            );
        }
        let later = IconButton::new(icons::CARET_RIGHT, "Move later in the list")
            .enabled(index + 1 < shown.masks.len())
            .show(ui);
        if later.clicked() {
            steps.edit(
                Edit::Color(ColorEdit::ReorderMask { id, to: index + 1 }),
                Merge::Own,
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = crate::widgets::icon_then_text(ui, icons::DELETE, "Delete mask");
            if super::look::id_button(ui, at.id("mask-delete"), true, label)
                .on_hover_text("Removes this mask and its keyframes")
                .clicked()
            {
                steps.edit(Edit::Color(ColorEdit::RemoveMask(id)), Merge::Own);
            }
        });
    });
    Some(id)
}

/// One row of the Add mask menu: the shape's glyph and name.
fn add_row(ui: &mut egui::Ui, at: At<'_>, kind: MaskShapeKind) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let label = mask_shape_label(kind);
    let id = at.id("add-mask").with(kind.as_str());
    let response = super::look::id_button(ui, id, true, format!("       {label}"))
        .on_hover_text(mask_shape_hint(kind));
    let glyph = egui::Rect::from_min_size(
        response.rect.left_center() + egui::vec2(8.0, -8.0),
        egui::vec2(16.0, 16.0),
    );
    paint_glyph(ui.painter(), glyph, kind, p.text_primary);
    response
}

/// A switch row whose change replaces the stored mask with `with(on)`.
#[allow(clippy::too_many_arguments)]
fn switch_row(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    row: &str,
    label: &str,
    value: bool,
    with: impl Fn(bool) -> ColorMask,
    hint: &str,
) {
    let id = at.id(row);
    let out = PropRow::new(id, label).hint(hint).show(ui, |ui| {
        let mut on = value;
        let response = toggle(ui, id.with("switch"), &mut on, label);
        (response, on)
    });
    let (response, on) = out.inner;
    let edit = response
        .changed()
        .then(|| Edit::Color(ColorEdit::SetMask(with(on))));
    steps.button(response, edit);
}

/// The value rows of mask `id`: position, size (and its link), rotation,
/// feather, the shape's own value and Expand.
fn mask_rows(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    id: u16,
    m: &ColorMask,
    base: &ColorMask,
) {
    let set = |param: MaskParam, value: PropValue| {
        Some(Edit::Color(ColorEdit::SetMaskParam {
            id,
            param: ColorMaskParam::Shape(param),
            value,
        }))
    };
    let pct = |v: i32| f64::from(v) / 100.0;
    let units = |shown: f64| (shown * 100.0).round();
    let (cx, cy) = (m.mask.center.x, m.mask.center.y);
    let limit = pct(MASK_CENTER_LIMIT);
    let row = |key: &'static str, label: &'static str| (key, label);
    let (k, l) = row("mask-x", "Position X");
    rows::numeric(
        ui,
        at,
        Numeric::new(k, l, pct(cx), 0.0, -limit..=limit)
            .suffix("%")
            .speed(0.2)
            .hint(mask_param_hint(ColorMaskParam::Shape(MaskParam::Center))),
        steps,
        |v| {
            let x = units(v) as i32;
            (x != cx)
                .then(|| {
                    set(
                        MaskParam::Center,
                        PropValue::MaskPoint(MaskPoint::new(x, cy)),
                    )
                })
                .flatten()
        },
    );
    let (k, l) = row("mask-y", "Position Y");
    rows::numeric(
        ui,
        at,
        Numeric::new(k, l, pct(cy), 0.0, -limit..=limit)
            .suffix("%")
            .speed(0.2)
            .no_keys(),
        steps,
        |v| {
            let y = units(v) as i32;
            (y != cy)
                .then(|| {
                    set(
                        MaskParam::Center,
                        PropValue::MaskPoint(MaskPoint::new(cx, y)),
                    )
                })
                .flatten()
        },
    );
    if m.mask.kind().has_size() {
        let (w, h) = (m.mask.size.x, m.mask.size.y);
        let default = m.mask.kind().default_size();
        let top = pct(MASK_SIZE_MAX);
        let linked = base.mask.linked;
        // Linked: the other side follows, keeping the ratio.
        let scaled = |from: i32, to: i32, other: i32| {
            if linked && from > 0 {
                (i64::from(other) * i64::from(to) / i64::from(from)) as i32
            } else {
                other
            }
        };
        rows::numeric(
            ui,
            at,
            Numeric::new("mask-w", "Width", pct(w), pct(default.x), 0.0..=top)
                .suffix("%")
                .speed(0.2)
                .hint(mask_param_hint(ColorMaskParam::Shape(MaskParam::Size))),
            steps,
            |v| {
                let nw = units(v) as i32;
                (nw != w)
                    .then(|| {
                        set(
                            MaskParam::Size,
                            PropValue::MaskPoint(MaskPoint::new(nw, scaled(w, nw, h))),
                        )
                    })
                    .flatten()
            },
        );
        rows::numeric(
            ui,
            at,
            Numeric::new("mask-h", "Height", pct(h), pct(default.y), 0.0..=top)
                .suffix("%")
                .speed(0.2)
                .no_keys(),
            steps,
            |v| {
                let nh = units(v) as i32;
                (nh != h)
                    .then(|| {
                        set(
                            MaskParam::Size,
                            PropValue::MaskPoint(MaskPoint::new(scaled(h, nh, w), nh)),
                        )
                    })
                    .flatten()
            },
        );
        switch_row(
            ui,
            at,
            steps,
            "mask-link",
            "Keep ratio",
            linked,
            |on| ColorMask {
                mask: Mask {
                    linked: on,
                    ..base.mask
                },
                ..*base
            },
            "Width and height change together",
        );
    }
    let deg = m.mask.rotation.degrees();
    let turn = f64::from(MASK_ROTATION_LIMIT) / 100.0;
    rows::numeric(
        ui,
        at,
        Numeric::new("mask-rotation", "Rotation", deg, 0.0, -turn..=turn)
            .suffix("°")
            .speed(0.5)
            .hint(mask_param_hint(ColorMaskParam::Shape(MaskParam::Rotation))),
        steps,
        |v| {
            let a = Angle::from_hundredths((v * 100.0).round() as i32);
            (a != m.mask.rotation)
                .then(|| set(MaskParam::Rotation, PropValue::Angle(a)))
                .flatten()
        },
    );
    let ratio_row = |ui: &mut egui::Ui,
                     steps: &mut Steps,
                     key: &'static str,
                     param: MaskParam,
                     value: PerMille,
                     default: f64,
                     max: u16| {
        let label = mask_param_label(ColorMaskParam::Shape(param));
        rows::numeric(
            ui,
            at,
            Numeric::new(
                key,
                label,
                value.as_f64() * 100.0,
                default,
                0.0..=f64::from(max) / 10.0,
            )
            .suffix("%")
            .speed(0.3)
            .hint(mask_param_hint(ColorMaskParam::Shape(param))),
            steps,
            |v| {
                let r = PerMille::new((v * 10.0).round().clamp(0.0, f64::from(max)) as u16);
                (r != value)
                    .then(|| set(param, PropValue::Ratio(r)))
                    .flatten()
            },
        );
    };
    ratio_row(
        ui,
        steps,
        "mask-feather",
        MaskParam::Feather,
        m.mask.feather,
        0.0,
        MASK_RATIO_MAX,
    );
    match m.mask.shape {
        MaskShape::Rectangle { roundness } => ratio_row(
            ui,
            steps,
            "mask-roundness",
            MaskParam::Roundness,
            roundness,
            0.0,
            MASK_RATIO_MAX,
        ),
        MaskShape::Band { width } => ratio_row(
            ui,
            steps,
            "mask-band",
            MaskParam::BandWidth,
            width,
            50.0,
            MASK_BAND_MAX,
        ),
        _ => {}
    }
    let limit = f64::from(EXPAND_LIMIT);
    rows::numeric(
        ui,
        at,
        Numeric::new(
            "mask-expand",
            "Expand",
            f64::from(m.expand),
            0.0,
            -limit..=limit,
        )
        .decimals(0)
        .speed(0.5)
        .hint(mask_param_hint(ColorMaskParam::Expand)),
        steps,
        |v| {
            let e = v.round() as i16;
            (e != m.expand).then_some(Edit::Color(ColorEdit::SetMaskParam {
                id,
                param: ColorMaskParam::Expand,
                value: PropValue::Level(e),
            }))
        },
    );
}
