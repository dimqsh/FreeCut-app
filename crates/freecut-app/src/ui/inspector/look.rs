//! The Look lens (CL-043, ui-concept §6.3): the colour stack of video,
//! image and adjustment clips.
//!
//! Top to bottom: the grade header (the master switch, CL-023, and Amount,
//! CL-035, labelled Mix on adjustment clips), the page strip (Basic, HSL,
//! Curves, Wheels, Masks; a dot marks a page with changes), the page, and
//! the footer (Save look…, still a placeholder, and Reset all).
//!
//! Basic holds the Light (CL-007) and Color (CL-008) sections, each with
//! its on/off switch, reset badge, folded summary, `...` menu with the
//! card key commands, and a keyframe navigator per slider (CL-028). The
//! pages later slices fill show what they will hold.
//!
//! Every edit goes through the cards' [`Steps`]: a slider drag is one undo
//! entry and `Esc` reverts it (UI-009); a slider that has keys sets the key
//! at the playhead (TR-027). With several clips selected the values shown
//! are the first's and a value that differs shows a dash (UI-010); clips
//! that take no colour (titles, sound) are left alone by the planner.

use eframe::egui;
use freecut_timeline::edit::ColorEdit;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Clip, ClipContent, ColorPage, ColorParamId, ColorStack, GeneratedKind, PropGroup, PropPath,
    Section,
};
use freecut_timeline::time::Tick;

use super::lens::{CardId, Detail, widget_id};
use super::{Env, KeyActions, card_keys, key_menu, key_steps, status};
use crate::theme::{self, icons};
use crate::ui::drag::Held;
use crate::ui::look::{
    page_changed, page_empty_state, page_label, param_hint, param_label, section_label,
    section_summary,
};
use crate::ui::props::{self, Edit, Merge, Steps};
use crate::ui::rows::{self, At, Numeric};
use crate::widgets::card::{self as card_widget, PropertyCard};
use crate::widgets::empty::empty_state;
use crate::widgets::row::PropRow;
use crate::widgets::segmented::{Segment, Segmented, segment_rects};
use crate::widgets::toggle::toggle;

/// The stack `clip` shows at the playhead: keyed sliders evaluated
/// (CL-028), the static values elsewhere.
pub(crate) fn shown_color(clip: &Clip, playhead: Tick) -> ColorStack {
    let at = clip.content_time_at(playhead);
    let evaluated = match at {
        Some(ct) if !clip.keys.is_empty() => clip.color_at(ct),
        _ => clip.color.clone(),
    };
    ColorStack::of(evaluated.as_ref())
}

/// The selected clips that take a colour stack, in selection order, with
/// the stacks they show.
fn graded(env: &Env<'_, '_>) -> Vec<(ClipId, ColorStack, bool)> {
    let playhead = env.project.playhead;
    env.selection
        .iter()
        .filter_map(|id| env.doc.clip(*id))
        .filter(|c| c.content.takes_color())
        .map(|c| {
            let adjustment = matches!(
                c.content,
                ClipContent::Generated {
                    kind: GeneratedKind::Adjustment
                }
            );
            (c.id, shown_color(c, playhead), adjustment)
        })
        .collect()
}

pub(super) fn cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let clips = graded(env);
    let Some((_, stack, adjustment)) = clips.first().cloned() else {
        return;
    };
    let ids: Vec<ClipId> = clips.iter().map(|(id, _, _)| *id).collect();
    let stacks: Vec<ColorStack> = clips.into_iter().map(|(_, s, _)| s).collect();
    let shown = Shown {
        ids: &ids,
        stack: &stack,
        stacks: &stacks,
        adjustment,
    };
    grade_header(ui, env, &shown);
    let page = page_strip(ui, env, &stack);
    match page {
        ColorPage::Basic => {
            for (section, card) in [
                (Section::Light, CardId::Light),
                (Section::Color, CardId::ColorBalance),
            ] {
                section_card(ui, env, &shown, section, card);
            }
        }
        other => {
            if let Some((title, detail)) = page_empty_state(other) {
                let inset = theme::look(ui.ctx()).metrics.side_inset as i8;
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(inset, 12))
                    .show(ui, |ui| empty_state(ui, icons::LENS_LOOK, title, detail));
            }
        }
    }
}

/// What the lens shows: the clips it edits, the first one's stack and
/// every clip's stack (for mixed values), at the playhead.
struct Shown<'s> {
    ids: &'s [ClipId],
    stack: &'s ColorStack,
    stacks: &'s [ColorStack],
    /// The first clip is an adjustment clip: Amount reads Mix (CL-035).
    adjustment: bool,
}

/// Whether `id`'s value differs across `stacks` (a mixed value, UI-010).
fn mixed(stacks: &[ColorStack], id: ColorParamId) -> bool {
    let mut values = stacks.iter().map(|s| s.get(id));
    values
        .next()
        .is_some_and(|first| values.any(|v| v != first))
}

/// A slider row of `id` at `at`: −100 to +100 (Amount 0 to 100 %, Mix on
/// adjustment clips).
fn slider(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    id: ColorParamId,
    headline: bool,
) {
    let spec = id.spec();
    let stored = shown.stack.get(id);
    let is_mixed = mixed(shown.stacks, id);
    let label = if id == ColorParamId::Amount && shown.adjustment {
        "Mix"
    } else {
        param_label(id)
    };
    let at = at.mixed(is_mixed);
    let mut n = Numeric::new(
        id.as_str(),
        label,
        f64::from(stored),
        f64::from(spec.default),
        f64::from(spec.min)..=f64::from(spec.max),
    )
    .decimals(0)
    .speed(0.5)
    .hint(param_hint(id));
    if id == ColorParamId::Amount {
        n = n.suffix("%");
    }
    if headline {
        n = n.headline();
    }
    rows::numeric(ui, at, n, steps, |v| {
        // In range after the clamp, so the cast is exact.
        let wanted = v.round().clamp(f64::from(spec.min), f64::from(spec.max)) as i16;
        (is_mixed || wanted != stored).then_some(Edit::Color(ColorEdit::SetParam(id, wanted)))
    });
}

/// The grade header: the master switch and Amount (Mix).
fn grade_header(ui: &mut egui::Ui, env: &mut Env<'_, '_>, shown: &Shown<'_>) {
    let kind = env.kind;
    let look = theme::look(ui.ctx());
    let mut steps = Steps::default();
    let at = At::new(kind, CardId::Grade).with_keys(env.keys);
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: look.metrics.card_padding as i8,
            right: look.metrics.card_padding as i8,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = look.metrics.row_gap;
            let bypassed = shown.stacks.iter().all(|s| s.bypass);
            let row = PropRow::new(widget_id(kind, CardId::Grade, "on"), "Grade")
                .hint("Turns the whole grade off and on; every value and keyframe stays")
                .show(ui, |ui| {
                    let mut on = !bypassed;
                    let response = toggle(
                        ui,
                        widget_id(kind, CardId::Grade, "switch"),
                        &mut on,
                        "Grade on",
                    )
                    .on_hover_text(if bypassed {
                        "The grade is off: the clip shows its footage as it is"
                    } else {
                        "The grade is on"
                    });
                    (response, on)
                });
            let (response, on) = row.inner;
            let edit = response
                .changed()
                .then_some(Edit::Color(ColorEdit::SetBypass(!on)));
            steps.button(response, edit);
            slider(ui, at, &mut steps, shown, ColorParamId::Amount, true);
        });
    let result = props::commit(env.project, env.drag, shown.ids, steps, props::held(ui));
    status(env, result);
    hairline(ui);
}

/// A hairline across the panel.
fn hairline(ui: &mut egui::Ui) {
    let p = theme::look(ui.ctx()).palette;
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        y + 0.5,
        egui::Stroke::new(1.0, p.border_subtle),
    );
}

/// The page strip; returns the page to show (remembered per selection
/// kind, CL-043).
fn page_strip(ui: &mut egui::Ui, env: &mut Env<'_, '_>, stack: &ColorStack) -> ColorPage {
    let kind = env.kind;
    let look = theme::look(ui.ctx());
    let current = env.workspace.look_page(kind);
    let items: Vec<Segment<'_>> = ColorPage::ALL
        .iter()
        .map(|p| Segment::new(None, page_label(*p)))
        .collect();
    let index = ColorPage::ALL
        .iter()
        .position(|p| *p == current)
        .unwrap_or(0);
    let mut picked = None;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let out = Segmented::new(widget_id(kind, CardId::Grade, "pages"), &items, index)
                .fill_width()
                .show(ui);
            picked = out.picked;
            // A dot after the label of a page holding changes (CL-043).
            let cells = segment_rects(out.response.rect, &[1.0; 5]);
            let font = egui::TextStyle::Body.resolve(ui.style());
            for (page, cell) in ColorPage::ALL.into_iter().zip(cells) {
                if !page_changed(stack, page) {
                    continue;
                }
                let text_w = ui.fonts_mut(|f| {
                    f.layout_no_wrap(
                        page_label(page).to_owned(),
                        font.clone(),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
                });
                let centre = egui::pos2(cell.center().x + text_w / 2.0 + 6.0, cell.center().y);
                ui.painter().circle_filled(centre, 2.0, look.palette.accent);
            }
        });
    let page = picked
        .and_then(|i| ColorPage::ALL.get(i).copied())
        .unwrap_or(current);
    env.workspace.set_look_page(kind, page);
    hairline(ui);
    page
}

/// How many keys of `section`'s sliders the clips hold (for the reset's
/// sentence).
fn section_keys(env: &Env<'_, '_>, ids: &[ClipId], section: Option<Section>) -> usize {
    ids.iter()
        .filter_map(|id| env.doc.clip(*id))
        .flat_map(|c| c.keys.tracks().iter())
        .filter(|t| match (t.prop(), section) {
            (PropPath::Color(id), Some(s)) => id.section() == Some(s),
            (PropPath::Color(_), None) => true,
            _ => false,
        })
        .map(|t| t.keys().len())
        .sum()
}

/// What a reset says once done: the keyframes it removed, which undo
/// brings back (TR-027, CL-024).
fn reset_sentence(what: &str, keys: usize) -> Option<String> {
    (keys > 0).then(|| {
        let s = if keys == 1 { "" } else { "s" };
        format!("{what} removed {keys} keyframe{s}. Undo brings them back")
    })
}

fn section_card(
    ui: &mut egui::Ui,
    env: &mut Env<'_, '_>,
    shown: &Shown<'_>,
    section: Section,
    card: CardId,
) {
    let (ids, stack, stacks) = (shown.ids, shown.stack, shown.stacks);
    let kind = env.kind;
    let everything = env.workspace.detail == Detail::Everything;
    let keys = card_keys(env, PropGroup::Color(section));
    let mut key_actions = KeyActions::default();
    let mut reset = false;
    let on = stack.sections_enabled.is_on(section);
    let dirty = stacks.iter().any(|s| !s.section_is_default(section));
    let at = At::new(kind, card).with_keys(env.keys);
    let title = section_label(section);
    let removed = section_keys(env, ids, Some(section));
    let memory = env.workspace.card(kind, card);
    let card_out = PropertyCard::new(widget_id(kind, card, "card"), title)
        .summary(section_summary(stack, section))
        .keyed(keys.is_some_and(|k| k.animated()))
        .switch(
            on,
            "Turn the section off and on; its values and keyframes stay",
        )
        .reset(
            dirty || removed > 0,
            "Every value of the section back to its default; its keyframes are removed",
        )
        .menu(|ui| {
            if ui
                .button(crate::widgets::icon_then_text(
                    ui,
                    icons::RESET,
                    "Reset section",
                ))
                .on_hover_text("Every value back to its default; removes the section's keyframes")
                .clicked()
            {
                reset = true;
                ui.close();
            }
            if let Some(keys) = &keys {
                key_menu(ui, keys, &mut key_actions);
            }
        })
        .show(ui, memory, |ui, memory| {
            let mut steps = Steps::default();
            let (essentials, more): (Vec<ColorParamId>, Vec<ColorParamId>) = section
                .params()
                .iter()
                .partition(|id| **id != ColorParamId::ContrastPivot);
            for id in essentials {
                slider(ui, at, &mut steps, shown, id, false);
            }
            if !more.is_empty() {
                card_widget::more(
                    ui,
                    widget_id(kind, card, "more"),
                    memory,
                    everything,
                    more.len(),
                    |ui| {
                        for id in &more {
                            slider(ui, at, &mut steps, shown, *id, false);
                        }
                    },
                );
            }
            props::commit(env.project, env.drag, ids, steps, props::held(ui))
        });
    status(env, card_out.inner.flatten());
    let mut steps = Steps::default();
    if let Some(on) = card_out.switched {
        steps.edit(
            Edit::Color(ColorEdit::SetSectionEnabled(section, on)),
            Merge::Own,
        );
    }
    let resetting = reset || card_out.reset;
    if resetting {
        steps.edit(Edit::Color(ColorEdit::ResetSection(section)), Merge::Own);
    }
    let refused = props::commit(env.project, env.drag, ids, steps, Held::Up);
    if refused.is_some() {
        status(env, refused);
    } else if resetting {
        status(env, reset_sentence(&format!("Reset {title}"), removed));
    }
    if let Some(keys) = &keys {
        let result = key_steps(env, keys, &key_actions);
        status(env, result);
    }
}

/// The footer: **Save look…** (saved looks arrive with the Adjust tab,
/// CL-026) and **Reset all** (CL-024).
pub(super) fn footer(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let clips = graded(env);
    let ids: Vec<ClipId> = clips.iter().map(|(id, _, _)| *id).collect();
    let removed = section_keys(env, &ids, None);
    let changed = clips.iter().any(|(id, _, _)| {
        env.doc
            .clip(*id)
            .is_some_and(|c| freecut_timeline::edit::has_color(c))
    });
    let locked = matches!(env.selection.as_slice(), [id] if props::on_locked_track(&env.doc, *id));
    let mut reset = false;
    ui.horizontal(|ui| {
        ui.add_enabled(false, egui::Button::new("Save look…"))
            .on_disabled_hover_text("Saving a grade as a look arrives with the Adjust tab");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = crate::widgets::icon_then_text(ui, icons::RESET, "Reset all");
            reset = ui
                .add_enabled(changed && !locked, egui::Button::new(label))
                .on_hover_text(
                    "Every colour value back to its default, Amount to 100 %, and the \
                     colour keyframes removed",
                )
                .on_disabled_hover_text("Nothing to reset: the colour is at its defaults")
                .clicked();
        });
    });
    if reset {
        let mut steps = Steps::default();
        steps.edit(Edit::Color(ColorEdit::ResetAll), Merge::Own);
        let refused = props::commit(env.project, env.drag, &ids, steps, Held::Up);
        let result = refused.or_else(|| reset_sentence("Reset all colour", removed));
        status(env, result);
    }
}
