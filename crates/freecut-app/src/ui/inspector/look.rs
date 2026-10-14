//! The Look lens (CL-043, ui-concept §6.3): the colour stack of video,
//! image and adjustment clips.
//!
//! Top to bottom: the grade header (the master switch, CL-023, and Amount,
//! CL-035, labelled Mix on adjustment clips), the page strip (Basic, HSL,
//! Curves, Wheels, Masks; a dot marks a page with changes), the page, and
//! the footer (Save look…, still a placeholder, and Reset all).
//!
//! Basic holds the Light (CL-007), Color (CL-008, with Skin protection,
//! CL-030) and LUT (CL-017) sections; HSL
//! (CL-013) the eight band swatches and the chosen band's Hue, Saturation
//! and Lightness; Curves (CL-014) the Luma, Red, Green and Blue tabs over
//! the curve editor; Wheels (CL-015) Strength, the four wheels and their
//! luminance sliders. Every section has its on/off switch, reset badge,
//! folded summary, `...` menu with the card key commands, and a keyframe
//! navigator per keyable control (CL-028; curves take no keys). Masks
//! shows what it will hold.
//!
//! The LUT section's select lists None, FreeCut's bundled LUTs, the
//! library's imported ones and Import LUT… (which asks the app to run the
//! import, CL-016); a LUT whose file is missing says so under the select.
//!
//! Every edit goes through the cards' [`Steps`]: a slider drag is one undo
//! entry and `Esc` reverts it (UI-009); a slider that has keys sets the key
//! at the playhead (TR-027). With several clips selected the values shown
//! are the first's and a value that differs shows a dash (UI-010); clips
//! that take no colour (titles, sound) are left alone by the planner.

use eframe::egui;
use freecut_timeline::edit::{ColorEdit, is_color_prop};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Clip, ClipContent, ColorPage, ColorParamId, ColorStack, Curve, CurveChannel, GeneratedKind,
    HslChannel, HueBand, LutInterp, LutSlot, PropGroup, PropPath, Section, SkinScope, WheelId,
};
use freecut_timeline::time::Tick;

use super::lens::{CardId, Detail, widget_id};
use super::{Env, KeyActions, card_keys, key_menu, key_steps, status};
use crate::luts::LutChoices;
use crate::theme::{self, icons};
use crate::ui::drag::Held;
use crate::ui::look::{
    SKIN_HINT, SKIN_LABEL, band_label, curve_label, interp_hint, interp_label, page_changed,
    page_empty_state, page_label, param_hint, row_label, section_label, section_summary,
    skin_switch_hint, wheel_hint, wheel_label,
};
use crate::ui::props::{self, Edit, Merge, Steps};
use crate::ui::rows::{self, At, Numeric};
use crate::widgets::card::{self as card_widget, CardMemory, PropertyCard};
use crate::widgets::color_wheel::color_wheel;
use crate::widgets::curve_editor::curve_editor;
use crate::widgets::empty::empty_state;
use crate::widgets::row::PropRow;
use crate::widgets::segmented::{Segment, Segmented, segment_rects};
use crate::widgets::select::{Select, SelectOption};
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
                (Section::Lut, CardId::Lut),
            ] {
                section_card(ui, env, &shown, section, card);
            }
        }
        ColorPage::Hsl => section_card(ui, env, &shown, Section::Hsl, CardId::Hsl),
        ColorPage::Curves => section_card(ui, env, &shown, Section::Curves, CardId::Curves),
        ColorPage::Wheels => section_card(ui, env, &shown, Section::Wheels, CardId::Wheels),
        ColorPage::Masks => {
            if let Some((title, detail)) = page_empty_state(ColorPage::Masks) {
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

/// A slider row of `id` at `at`: −100 to +100 (Amount and Strength 0 to
/// 100 %, Amount reading Mix on adjustment clips).
fn slider(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    id: ColorParamId,
    headline: bool,
) {
    let label = if id == ColorParamId::Amount && shown.adjustment {
        "Mix"
    } else {
        row_label(id)
    };
    slider_labelled(ui, at, steps, shown, id, label, headline);
}

/// [`slider`] with its own row label.
fn slider_labelled(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    id: ColorParamId,
    label: &str,
    headline: bool,
) {
    let spec = id.spec();
    let stored = shown.stack.get(id);
    let is_mixed = mixed(shown.stacks, id);
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
    if matches!(
        id,
        ColorParamId::Amount
            | ColorParamId::WheelStrength
            | ColorParamId::LutIntensity
            | ColorParamId::SkinAmount(_)
    ) {
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
        .filter(|t| match section {
            Some(s) => t.prop().group() == Some(PropGroup::Color(s)),
            None => is_color_prop(t.prop()),
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
    let colour_keys = section_keys(env, ids, None);
    let luts = std::sync::Arc::clone(&env.project.luts);
    let summary = match (section, &stack.lut) {
        (Section::Lut, Some(slot)) if on => lut_summary(slot, &luts),
        _ => section_summary(stack, section),
    };
    // A LUT section with no LUT shows its select and no switch (CL-043).
    let switchable = section != Section::Lut || stacks.iter().any(|s| s.lut.is_some());
    let mut import = false;
    let memory = env.workspace.card(kind, card);
    let mut header = PropertyCard::new(widget_id(kind, card, "card"), title)
        .summary(summary)
        .keyed(keys.is_some_and(|k| k.animated()));
    if switchable {
        header = header.switch(
            on,
            "Turn the section off and on; its values and keyframes stay",
        );
    }
    let card_out = header
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
            let said = match section {
                Section::Hsl => {
                    hsl_body(ui, at, &mut steps, shown);
                    None
                }
                Section::Curves => curves_body(ui, at, &mut steps, shown, colour_keys > 0),
                Section::Wheels => {
                    wheels_body(ui, at, &mut steps, shown);
                    None
                }
                Section::Color => {
                    color_body(ui, at, &mut steps, shown);
                    None
                }
                Section::Lut => {
                    import = lut_body(ui, at, &mut steps, shown, &luts, memory, everything);
                    None
                }
                _ => {
                    sliders_body(ui, at, &mut steps, shown, section, memory, everything);
                    None
                }
            };
            props::commit(env.project, env.drag, ids, steps, props::held(ui)).or(said)
        });
    status(env, card_out.inner.flatten());
    if import {
        env.out.import_luts = true;
    }
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

/// Light and Color: their sliders, Pivot under More.
fn sliders_body(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    section: Section,
    memory: &mut CardMemory,
    everything: bool,
) {
    let (essentials, more): (Vec<ColorParamId>, Vec<ColorParamId>) = section
        .params()
        .iter()
        .partition(|id| **id != ColorParamId::ContrastPivot);
    for id in essentials {
        slider(ui, at, steps, shown, id, false);
    }
    if !more.is_empty() {
        card_widget::more(ui, at.id("more"), memory, everything, more.len(), |ui| {
            for id in &more {
                slider(ui, at, steps, shown, *id, false);
            }
        });
    }
}

/// The LUT section's folded summary: the LUT's name, its intensity below
/// 100 %, skin protection, and "missing" when its file is.
fn lut_summary(slot: &LutSlot, luts: &LutChoices) -> String {
    let mut parts = vec![luts.name_of(&slot.lut)];
    if slot.intensity != 100 {
        parts.push(format!("{} %", slot.intensity));
    }
    if slot.skin.on {
        parts.push(SKIN_LABEL.to_owned());
    }
    if luts.is_missing(&slot.lut) {
        parts.push("missing".to_owned());
    }
    parts.join(" · ")
}

/// A Skin protection switch row (CL-030) and, while it is on, its Amount.
fn skin_rows(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    scope: SkinScope,
) {
    let all_on = shown.stacks.iter().all(|s| s.skin(scope).on);
    let any_on = shown.stacks.iter().any(|s| s.skin(scope).on);
    let row_id = at.id(match scope {
        SkinScope::Color => "skin",
        SkinScope::Lut => "lut_skin",
    });
    let row = PropRow::new(row_id, SKIN_LABEL)
        .hint(skin_switch_hint(scope))
        .changed(any_on)
        .show(ui, |ui| {
            let mut on = all_on;
            let response =
                toggle(ui, row_id.with("switch"), &mut on, SKIN_LABEL).on_hover_text(SKIN_HINT);
            (response, on)
        });
    let (response, on) = row.inner;
    let edit = response
        .changed()
        .then_some(Edit::Color(ColorEdit::SetSkin { scope, on }));
    steps.button(response, edit);
    if any_on {
        slider(ui, at, steps, shown, ColorParamId::SkinAmount(scope), false);
    }
}

/// Color (CL-008): its sliders, then Skin protection (CL-030).
fn color_body(ui: &mut egui::Ui, at: At<'_>, steps: &mut Steps, shown: &Shown<'_>) {
    for id in ColorParamId::COLOR {
        slider(ui, at, steps, shown, id, false);
    }
    skin_rows(ui, at, steps, shown, SkinScope::Color);
}

/// LUT (CL-017): the select (None, Bundled, Imported, Import LUT…), and
/// with a LUT its Intensity, Skin protection, and the interpolation under
/// More. Returns whether Import LUT… was picked.
fn lut_body(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    luts: &LutChoices,
    memory: &mut CardMemory,
    everything: bool,
) -> bool {
    let p = theme::look(ui.ctx()).palette;
    let current = shown.stack.lut.as_ref();
    let mixed = shown
        .stacks
        .iter()
        .any(|s| s.lut.as_ref().map(|l| &l.lut) != current.map(|l| &l.lut));
    // The options, with what picking each does.
    enum Pick {
        None,
        Lut(freecut_timeline::model::LutRef),
        Heading,
        Import,
    }
    let mut picks: Vec<(String, Option<String>, Pick)> =
        vec![("None".to_owned(), None, Pick::None)];
    let group =
        |title: &str, hint: &str, entries: &[crate::luts::LutChoice], picks: &mut Vec<_>| {
            if entries.is_empty() {
                return;
            }
            picks.push((title.to_owned(), Some(hint.to_owned()), Pick::Heading));
            for e in entries {
                picks.push((e.name.clone(), None, Pick::Lut(e.lut.clone())));
            }
        };
    group("Bundled", "FreeCut's own LUTs", &luts.bundled, &mut picks);
    group(
        "Imported",
        "LUT files you imported",
        &luts.imported,
        &mut picks,
    );
    // A LUT the lists don't hold (missing, or removed from the library)
    // still shows as the current choice.
    if let Some(slot) = current
        && !picks
            .iter()
            .any(|(_, _, pk)| matches!(pk, Pick::Lut(l) if l.hash == slot.lut.hash))
    {
        picks.push((luts.name_of(&slot.lut), None, Pick::Lut(slot.lut.clone())));
    }
    picks.push(("Import LUT…".to_owned(), None, Pick::Import));
    let options: Vec<SelectOption<'_>> = picks
        .iter()
        .map(|(label, heading, _)| SelectOption::new(label).blocked(heading.as_deref()))
        .collect();
    let index = match current {
        None => Some(0),
        Some(slot) => picks
            .iter()
            .position(|(_, _, pk)| matches!(pk, Pick::Lut(l) if l.hash == slot.lut.hash)),
    };
    let id = at.id("lut");
    let row = PropRow::new(id, "LUT")
        .hint("A colour look from a .cube or .3dl file")
        .changed(current.is_some())
        .show(ui, |ui| {
            let mut select = Select::new(id.with("select"), &options, index).fill_width();
            if mixed {
                select = select.text("\u{2014}");
            }
            select.show(ui).picked
        });
    let mut import = false;
    match row.inner.and_then(|i| picks.get(i)).map(|(_, _, pk)| pk) {
        Some(Pick::None) if current.is_some() || mixed => {
            steps.edit(Edit::Color(ColorEdit::SetLut(None)), Merge::Own);
        }
        Some(Pick::Lut(lut)) if mixed || current.is_none_or(|c| c.lut.hash != lut.hash) => {
            steps.edit(
                Edit::Color(ColorEdit::SetLut(Some(LutSlot::new(lut.clone())))),
                Merge::Own,
            );
        }
        Some(Pick::Import) => import = true,
        _ => {}
    }
    let Some(slot) = current else {
        return import;
    };
    if luts.is_missing(&slot.lut) {
        ui.label(
            egui::RichText::new(format!(
                "{} can't be found, so the LUT does nothing. Import it again to bring it back",
                slot.lut.file_name
            ))
            .color(p.danger),
        );
    }
    slider(ui, at, steps, shown, ColorParamId::LutIntensity, false);
    skin_rows(ui, at, steps, shown, SkinScope::Lut);
    card_widget::more(ui, at.id("more"), memory, everything, 1, |ui| {
        let options: Vec<SelectOption<'_>> = LutInterp::ALL
            .iter()
            .map(|i| SelectOption::new(interp_label(*i)))
            .collect();
        let current = LutInterp::ALL.iter().position(|i| *i == slot.interp);
        let id = at.id("interp");
        let row = PropRow::new(id, "Interpolation")
            .hint(interp_hint(slot.interp))
            .changed(slot.interp != LutInterp::default())
            .show(ui, |ui| {
                Select::new(id.with("select"), &options, current)
                    .fill_width()
                    .show(ui)
                    .picked
            });
        if let Some(interp) = row.inner.and_then(|i| LutInterp::ALL.get(i).copied())
            && interp != slot.interp
        {
            steps.edit(Edit::Color(ColorEdit::SetLutInterp(interp)), Merge::Own);
        }
    });
    import
}

/// A band's swatch colour: its centre hue, fairly saturated.
fn band_colour(band: HueBand) -> egui::Color32 {
    egui::ecolor::HsvaGamma {
        h: f32::from(band.center_degrees()) / 360.0,
        s: 0.78,
        v: 0.92,
        a: 1.0,
    }
    .into()
}

/// The view state a page keeps in egui's memory (which band, which
/// curve): view state only, never in the model (CL-013, CL-014).
fn remembered(ui: &egui::Ui, id: egui::Id, len: usize) -> usize {
    ui.data_mut(|d| *d.get_persisted_mut_or_default::<usize>(id)) % len
}

fn remember(ui: &egui::Ui, id: egui::Id, value: usize) {
    ui.data_mut(|d| d.insert_persisted(id, value));
}

/// HSL (CL-013): eight swatches (a dot on edited ones; right-click resets
/// one), then the chosen band's Hue, Saturation and Lightness.
fn hsl_body(ui: &mut egui::Ui, at: At<'_>, steps: &mut Steps, shown: &Shown<'_>) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let band_id = at.id("band");
    let mut chosen = remembered(ui, band_id, HueBand::ALL.len());
    let height = 34.0;
    let (strip, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let cell = strip.width() / HueBand::ALL.len() as f32;
    let radius = (cell / 2.0 - 4.0).clamp(7.0, 11.0);
    for (i, band) in HueBand::ALL.into_iter().enumerate() {
        let centre = egui::pos2(
            strip.left() + cell * (i as f32 + 0.5),
            strip.center().y - 2.0,
        );
        let rect = egui::Rect::from_center_size(centre, egui::vec2(cell, height));
        let response = ui
            .interact(rect, at.id("swatch").with(i), egui::Sense::click())
            .on_hover_text(format!(
                "{}: pick to edit; right-click to reset",
                band_label(band)
            ));
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                ui.is_enabled(),
                i == chosen,
                band_label(band),
            )
        });
        if response.clicked() {
            chosen = i;
        }
        let edited = shown.stacks.iter().any(|s| !s.hsl.band(band).is_default());
        response.context_menu(|ui| {
            let label = format!("Reset {}", band_label(band).to_lowercase());
            if ui
                .add_enabled(edited, egui::Button::new(label))
                .on_hover_text("This band's hue, saturation and lightness back to 0")
                .clicked()
            {
                steps.edit(Edit::Color(ColorEdit::ResetBand(band)), Merge::Own);
                ui.close();
            }
        });
        let painter = ui.painter();
        if i == chosen {
            painter.circle_stroke(centre, radius + 3.0, egui::Stroke::new(2.0, p.accent));
        } else if response.hovered() {
            painter.circle_stroke(
                centre,
                radius + 3.0,
                egui::Stroke::new(1.0, p.border_strong),
            );
        }
        painter.circle_filled(centre, radius, band_colour(band));
        if edited {
            painter.circle_filled(egui::pos2(centre.x, centre.y + radius + 7.0), 2.0, p.accent);
        }
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, rect.shrink(2.0), theme::R_SM);
        }
    }
    remember(ui, band_id, chosen);
    let band = HueBand::ALL[chosen];
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(band_label(band))
            .color(p.text_secondary)
            .strong(),
    );
    for channel in HslChannel::ALL {
        slider(
            ui,
            at,
            steps,
            shown,
            ColorParamId::Hsl { band, channel },
            false,
        );
    }
}

/// A curve's line colour in the editor.
fn curve_colour(channel: CurveChannel, p: &theme::Palette) -> egui::Color32 {
    match channel {
        CurveChannel::Luma => p.text_primary,
        CurveChannel::Red => egui::Color32::from_rgb(232, 84, 84),
        CurveChannel::Green => egui::Color32::from_rgb(76, 186, 96),
        CurveChannel::Blue => egui::Color32::from_rgb(80, 140, 240),
    }
}

/// Curves (CL-014): the Luma, Red, Green and Blue tabs (a dot on changed
/// ones), the editor, and the curve's reset. Returns a sentence for the
/// status line (a refused 17th point).
fn curves_body(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    keyed: bool,
) -> Option<String> {
    let p = theme::look(ui.ctx()).palette;
    let tab_id = at.id("curve_tab");
    let index = remembered(ui, tab_id, CurveChannel::ALL.len());
    let items: Vec<Segment<'_>> = CurveChannel::ALL
        .iter()
        .map(|c| Segment::new(None, curve_label(*c)))
        .collect();
    let out = Segmented::new(tab_id.with("strip"), &items, index)
        .fill_width()
        .show(ui);
    let cells = segment_rects(out.response.rect, &[1.0; 4]);
    for (channel, cell) in CurveChannel::ALL.into_iter().zip(cells) {
        if shown
            .stacks
            .iter()
            .any(|s| !s.curves.get(channel).is_default())
        {
            let centre = egui::pos2(cell.right() - 8.0, cell.top() + 7.0);
            ui.painter().circle_filled(centre, 2.0, p.accent);
        }
    }
    let index = out.picked.unwrap_or(index);
    remember(ui, tab_id, index);
    let channel = CurveChannel::ALL[index];
    ui.add_space(8.0);
    let curve = shown.stack.curves.get(channel);
    let edited = curve_editor(
        ui,
        at.id("curve_editor").with(index),
        curve,
        curve_colour(channel, &p),
        320.0,
    );
    let response = edited.response.clone();
    let response = response.on_hover_text(
        "Click to add a point, drag to move it (Shift: up and down only), \
         double-click or drag it out to remove it",
    );
    if rows::esc_stops_drag(ui, &response) {
        steps.cancel();
    } else {
        let edit = edited
            .curve
            .map(|curve| Edit::Color(ColorEdit::SetCurve { channel, curve }));
        steps.button(response, edit);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let differs = shown
            .stacks
            .iter()
            .any(|s| !s.curves.get(channel).is_default());
        let label = crate::widgets::icon_then_text(ui, icons::RESET, "Reset curve");
        if ui
            .add_enabled(differs, egui::Button::new(label))
            .on_hover_text(format!(
                "The {} curve back to the straight diagonal",
                curve_label(channel).to_lowercase()
            ))
            .on_disabled_hover_text("This curve is already the straight diagonal")
            .clicked()
        {
            steps.edit(
                Edit::Color(ColorEdit::SetCurve {
                    channel,
                    curve: Curve::identity(),
                }),
                Merge::Own,
            );
        }
    });
    if keyed {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Curves take no keyframes: they hold for the whole clip")
                .color(p.text_tertiary)
                .small(),
        );
    }
    edited.refused.map(str::to_owned)
}

/// Wheels (CL-015): Strength, the four wheels two by two (each with its
/// reset and its puck's keyframe diamond), then their luminance sliders.
fn wheels_body(ui: &mut egui::Ui, at: At<'_>, steps: &mut Steps, shown: &Shown<'_>) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    slider(ui, at, steps, shown, ColorParamId::WheelStrength, false);
    ui.add_space(6.0);
    let gap = 12.0;
    let column = ((ui.available_width() - gap) / 2.0).max(80.0);
    let side = (column - 8.0).clamp(72.0, 150.0);
    for pair in WheelId::ALL.chunks(2) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for wheel in pair {
                let wheel = *wheel;
                ui.allocate_ui_with_layout(
                    egui::vec2(column, side + 34.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| wheel_cell(ui, at, steps, shown, wheel, column, side, &p),
                );
            }
        });
        ui.add_space(6.0);
    }
    ui.label(
        egui::RichText::new("Luminance")
            .color(p.text_secondary)
            .strong(),
    );
    for wheel in WheelId::ALL {
        slider_labelled(
            ui,
            at,
            steps,
            shown,
            ColorParamId::WheelLuma(wheel),
            wheel_label(wheel),
            false,
        );
    }
}

/// One wheel: its name, reset and puck keyframe diamond over the disk.
#[allow(clippy::too_many_arguments)]
fn wheel_cell(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    wheel: WheelId,
    column: f32,
    side: f32,
    p: &theme::Palette,
) {
    let name = wheel_label(wheel);
    let prop = PropPath::ColorPuck(wheel);
    ui.allocate_ui_with_layout(
        egui::vec2(column, 24.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(egui::RichText::new(name).color(p.text_secondary))
                .on_hover_text(wheel_hint(wheel));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = shown
                    .stacks
                    .iter()
                    .any(|s| !s.wheels.wheel(wheel).is_default());
                let reset = crate::widgets::icon_button::IconButton::new(
                    icons::RESET,
                    "Centre the puck and set the luminance to 0; removes their keyframes",
                )
                .enabled(dirty && ui.is_enabled())
                .show(ui);
                if reset.clicked() {
                    steps.edit(Edit::Color(ColorEdit::ResetWheel(wheel)), Merge::Own);
                }
                if let Some((prop, nav)) = at.nav(name, Some(prop)) {
                    // The navigator reads left to right inside this
                    // right-to-left row.
                    let w = nav.width(crate::widgets::field::compact(ui));
                    let out = ui
                        .allocate_ui_with_layout(
                            egui::vec2(w, 24.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| nav.show(ui),
                        )
                        .inner;
                    at.keys_clicked(prop, out, steps);
                }
            });
        },
    );
    let mixed_puck = {
        let mut pucks = shown.stacks.iter().map(|s| s.puck(wheel));
        pucks.next().is_some_and(|first| pucks.any(|q| q != first))
    };
    let puck = shown.stack.puck(wheel);
    let out = color_wheel(ui, at.id("wheel").with(wheel.as_str()), name, puck, side);
    let response = out.response.clone().on_hover_text(if mixed_puck {
        "The selected clips differ here; dragging gives them all this puck"
    } else {
        wheel_hint(wheel)
    });
    if rows::esc_stops_drag(ui, &response) {
        steps.cancel();
    } else {
        let edit = out
            .puck
            .map(|(x, y)| Edit::Color(ColorEdit::SetPuck { wheel, x, y }));
        steps.button(response, edit);
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
