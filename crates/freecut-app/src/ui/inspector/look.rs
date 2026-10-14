//! The Look lens (CL-043, ui-concept §6.3): the colour stack of video,
//! image and adjustment clips.
//!
//! Top to bottom: the grade header (the **Presets** select of saved looks,
//! CL-026, the master switch, CL-023, and Amount, CL-035, labelled Mix on
//! adjustment clips), the page strip (Basic, HSL, Curves, Wheels, Masks; a
//! dot marks a page with changes), the page, and the footer (Save look…,
//! CL-026, and Reset all).
//!
//! Basic holds the Auto (CL-019), Match (CL-020), Light (CL-007), Color
//! (CL-008, with Skin protection, CL-030) and LUT (CL-017) sections; HSL
//! (CL-013) the eight band swatches and the chosen band's Hue, Saturation
//! and Lightness; Curves (CL-014) the Luma, Red, Green and Blue tabs over
//! the curve editor; Wheels (CL-015) Strength, the four wheels and their
//! luminance sliders. Every section has its on/off switch, reset badge,
//! folded summary, `...` menu with the card key commands, and a keyframe
//! navigator per keyable control (CL-028; curves take no keys). Masks
//! (CL-031) lists the clip's colour masks as chips with **Add mask**, and
//! the chosen one's shape, combine mode, invert and values
//! ([`super::masks`]); the Stage shows its outline and handles.
//!
//! The LUT section's select lists None, FreeCut's bundled LUTs, the
//! library's imported ones and Import LUT… (which asks the app to run the
//! import, CL-016); a LUT whose file is missing says so under the select.
//!
//! Auto shows its button (with what to analyse and the style) until a run
//! stores a correction, then the offsets it adds as read-only rows,
//! Strength, Re-run and Apply to sliders. Match shows **Match color…**,
//! which enters pick mode (a click on a clip on the timeline picks the
//! reference frame; **Use frame at playhead** and Cancel sit in the
//! section), then the reference, Intensity, Re-match and Change
//! reference…. The analyses run on the app's pool
//! ([`super::ColorRequest`]); while one runs its section shows a spinner
//! and Cancel.
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
    AutoSource, AutoStyle, Clip, ClipContent, ColorPage, ColorParamId, ColorStack, Curve,
    CurveChannel, GeneratedKind, HslChannel, HueBand, LutInterp, LutSlot, MatchRef, PropGroup,
    PropPath, Section, SkinScope, WheelId,
};
use freecut_timeline::time::Tick;

use super::lens::{CardId, Detail, widget_id};
use super::{Env, KeyActions, card_keys, key_menu, key_steps, status};
use crate::luts::LutChoices;
use crate::theme::{self, icons};
use crate::ui::drag::Held;
use crate::ui::look::{
    self as words, SKIN_HINT, SKIN_LABEL, band_label, curve_label, interp_hint, interp_label,
    page_changed, page_label, param_hint, row_label, section_label, section_summary,
    skin_switch_hint, wheel_hint, wheel_label,
};
use crate::ui::props::{self, Edit, Merge, Steps};
use crate::ui::rows::{self, At, Numeric};
use crate::widgets::card::{self as card_widget, CardMemory, PropertyCard};
use crate::widgets::color_wheel::color_wheel;
use crate::widgets::curve_editor::curve_editor;
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
                (Section::Auto, CardId::Auto),
                (Section::Match, CardId::Match),
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
        ColorPage::Masks => section_card(ui, env, &shown, Section::Masks, CardId::Masks),
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
            | ColorParamId::AutoStrength
            | ColorParamId::MatchIntensity
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
            presets_row(ui, kind, &mut steps, &env.project.looks);
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

/// The **Presets** select (CL-026, CL-043): picking a saved look applies
/// its sections to the lens's clips, as one undo entry.
fn presets_row(
    ui: &mut egui::Ui,
    kind: super::lens::SelectionKind,
    steps: &mut Steps,
    looks: &crate::looks::SavedLooks,
) {
    let names: Vec<String> = looks.looks.iter().map(|l| l.name.clone()).collect();
    let options: Vec<SelectOption<'_>> = names.iter().map(|n| SelectOption::new(n)).collect();
    let id = widget_id(kind, CardId::Grade, "presets");
    let row = PropRow::new(id, "Presets")
        .hint("Applies a saved look's sections to the selected clips")
        .show(ui, |ui| {
            let text = if looks.loading {
                "Reading saved looks…"
            } else if looks.looks.is_empty() {
                "No saved looks yet"
            } else {
                "Apply a saved look"
            };
            let enabled = !looks.looks.is_empty();
            ui.add_enabled_ui(enabled, |ui| {
                Select::new(id.with("select"), &options, None)
                    .text(text)
                    .fill_width()
                    .show(ui)
                    .picked
            })
            .inner
        });
    if let Some(look) = row.inner.and_then(|i| looks.looks.get(i)) {
        steps.edit(
            Edit::Color(ColorEdit::ApplySections(std::sync::Arc::clone(
                &look.snapshot,
            ))),
            Merge::Own,
        );
    }
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
    // A LUT, Auto or Match section with nothing chosen shows its action
    // and no switch (CL-043).
    let switchable = match section {
        Section::Lut => stacks.iter().any(|s| s.lut.is_some()),
        Section::Auto => stacks.iter().any(|s| s.auto.is_some()),
        Section::Match => stacks.iter().any(|s| s.match_xf.is_some()),
        _ => true,
    };
    let analysis = matches!(section, Section::Auto | Section::Match)
        .then(|| AnalysisView::of(env, ids, stack));
    let mut ask = None;
    let mut import = false;
    // The Masks page edits one clip's masks (CL-031): its stored stack, so
    // whole-mask edits never write values evaluated at the playhead.
    let single = match ids {
        [one] => Some(*one),
        _ => None,
    };
    let stored = single
        .and_then(|id| env.doc.clip(id))
        .map(|c| ColorStack::of(c.color.as_ref()))
        .unwrap_or_default();
    let mut focus = None;
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
                Section::Masks => {
                    focus = super::masks::masks_body(ui, at, &mut steps, single, stack, &stored);
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
                Section::Auto => {
                    if let Some(view) = &analysis {
                        ask = auto_body(ui, at, &mut steps, shown, view);
                    }
                    None
                }
                Section::Match => {
                    if let Some(view) = &analysis {
                        ask = match_body(ui, at, &mut steps, shown, view);
                    }
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
    // The Stage draws the chosen mask's outline and handles (TR-019).
    if let (Some(clip), Some(id)) = (single, focus) {
        env.project.mask_focus = Some(crate::session::MaskFocus { clip, id });
    }
    if import {
        env.out.import_luts = true;
    }
    if let (Some(ask), Some(first)) = (ask, ids.first()) {
        answer(env, *first, stack, ask);
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
        let masks: usize = stacks.iter().map(|s| s.masks.len()).sum();
        let sentence = if section == Section::Masks && masks > 0 {
            Some(format!(
                "Removed {}. Undo brings them back",
                words::masks_count(masks)
            ))
        } else {
            reset_sentence(&format!("Reset {title}"), removed)
        };
        status(env, sentence);
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

/// What the Auto and Match sections know beyond the stacks.
struct AnalysisView {
    /// The analysis running for the lens's clip: Auto (`true`) or Match.
    running: Option<bool>,
    /// Pick mode for the lens's clip, with its "graded" choice.
    picking: Option<bool>,
    /// Why the sections can't measure now (an adjustment clip, several
    /// clips, a locked track).
    blocked: Option<&'static str>,
    /// The match's reference as words, and whether it can be measured
    /// again (its clip still exists).
    reference: Option<(String, bool)>,
}

impl AnalysisView {
    fn of(env: &Env<'_, '_>, ids: &[ClipId], stack: &ColorStack) -> AnalysisView {
        let first = ids.first().copied();
        let clip = first.and_then(|id| env.doc.clip(id));
        let blocked = if ids.len() > 1 {
            Some(words::ONE_AT_A_TIME)
        } else if clip.is_some_and(|c| {
            matches!(
                c.content,
                ClipContent::Generated {
                    kind: GeneratedKind::Adjustment
                }
            )
        }) {
            Some(words::NO_FOOTAGE)
        } else if first.is_some_and(|id| props::on_locked_track(&env.doc, id)) {
            Some("The clip's track is locked")
        } else {
            None
        };
        let running = env
            .project
            .color_job
            .as_ref()
            .filter(|j| Some(j.clip) == first)
            .map(|j| j.auto);
        let picking = env
            .project
            .match_pick
            .filter(|p| Some(p.target) == first)
            .map(|p| p.graded);
        let reference = stack
            .match_xf
            .as_ref()
            .map(|m| reference_words(env, &m.reference));
        AnalysisView {
            running,
            picking,
            blocked,
            reference,
        }
    }
}

/// A match reference as words ("Beach · 0:03.12", "sunset.jpg") and
/// whether Re-match can measure it again.
fn reference_words(env: &Env<'_, '_>, reference: &MatchRef) -> (String, bool) {
    let rate = env.doc.settings().frame_rate;
    let when = |at: Tick| {
        let secs = at.as_secs_f64().max(0.0);
        let fps = f64::from(rate.num()) / f64::from(rate.den().max(1));
        // Whole minutes, seconds and frames of a non-negative time.
        let (m, s) = ((secs / 60.0).floor() as u64, (secs % 60.0).floor() as u64);
        let f = ((secs.fract()) * fps).floor() as u64;
        format!("{m}:{s:02}.{f:02}")
    };
    match reference {
        MatchRef::Clip { clip, at, graded } => match env.doc.clip(*clip) {
            Some(c) => {
                let name = crate::ui::timeline::clip_label(&env.doc, &c.content, c.name.as_deref());
                let still = matches!(c.content, ClipContent::Image { .. });
                let mut words = if still {
                    name
                } else {
                    format!("{name} · {}", when(*at))
                };
                if !graded {
                    words.push_str(" · ungraded");
                }
                (words, true)
            }
            None => ("A deleted clip".to_owned(), false),
        },
        MatchRef::Media { media, .. } => match env.doc.library().get(*media) {
            Some(item) => (crate::ui::timeline::media_name(item), true),
            None => ("A removed file".to_owned(), false),
        },
    }
}

/// What the Auto or Match section asked for.
enum Ask {
    Auto(AutoStyle, AutoSource),
    Cancel,
    Pick,
    CancelPick,
    Graded(bool),
    MatchPlayhead,
    Rematch,
}

/// Carries out what a section asked: pick mode is the project's; an
/// analysis is the app's ([`super::ColorRequest`]).
fn answer(env: &mut Env<'_, '_>, clip: ClipId, stack: &ColorStack, ask: Ask) {
    use super::ColorRequest as R;
    use crate::session::MatchPick;
    match ask {
        Ask::Auto(style, source) => {
            env.out.color = Some(R::Auto {
                clip,
                style,
                source,
            })
        }
        Ask::Cancel => env.out.color = Some(R::CancelJob),
        Ask::Pick => {
            env.project.match_pick = Some(MatchPick {
                target: clip,
                graded: true,
            });
            status(env, Some(words::PICK_SENTENCE.to_owned()));
        }
        Ask::CancelPick => env.project.match_pick = None,
        Ask::Graded(graded) => {
            if let Some(p) = &mut env.project.match_pick {
                p.graded = graded;
            }
        }
        Ask::MatchPlayhead => {
            let playhead = env.project.playhead;
            let graded = env.project.match_pick.is_none_or(|p| p.graded);
            let found = crate::jobs::color::clip_under_playhead(&env.doc, clip, playhead)
                .and_then(|r| Some((r, env.doc.clip(r)?.content_time_at(playhead)?)));
            match found {
                Some((r, at)) => {
                    env.project.match_pick = None;
                    env.out.color = Some(R::Match {
                        clip,
                        reference: MatchRef::Clip {
                            clip: r,
                            at,
                            graded,
                        },
                    });
                }
                None => status(
                    env,
                    Some("No clip under the playhead to match to".to_owned()),
                ),
            }
        }
        Ask::Rematch => {
            if let Some(m) = &stack.match_xf {
                env.out.color = Some(R::Match {
                    clip,
                    reference: m.reference.clone(),
                });
            }
        }
    }
}

/// A button with a known id (`at.id(name)`), so the keyboard, screen
/// readers and tests find the same widget every frame: drawn like an egui
/// button.
pub(super) fn id_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    enabled: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let pad = ui.spacing().button_padding;
    let size = egui::vec2(
        galley.size().x + 2.0 * pad.x,
        (galley.size().y + 2.0 * pad.y).max(ui.spacing().interact_size.y),
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let label = galley.text().to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    if ui.is_rect_visible(rect) {
        let visuals = if enabled {
            ui.style().interact(&response)
        } else {
            &ui.visuals().widgets.noninteractive
        };
        ui.painter().rect(
            rect.expand(visuals.expansion),
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let colour = if enabled {
            visuals.text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let at = egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, colour);
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, rect, theme::R_SM);
        }
    }
    response
}

/// A spinner with its sentence and Cancel, while an analysis runs.
fn running_row(ui: &mut egui::Ui, at: At<'_>, sentence: &str) -> bool {
    let p = theme::look(ui.ctx()).palette;
    let mut cancel = false;
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(egui::RichText::new(sentence).color(p.text_secondary));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel = id_button(ui, at.id("cancel_job"), true, "Cancel")
                .on_hover_text("Stops measuring; nothing changes")
                .clicked();
        });
    });
    cancel
}

/// A select row remembered in egui's memory (view state): Auto's source
/// and style.
fn remembered_select(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    hint: &str,
    options: &[&str],
    hints: &[&str],
) -> usize {
    let current = remembered(ui, id, options.len());
    let opts: Vec<SelectOption<'_>> = options.iter().map(|o| SelectOption::new(o)).collect();
    let row = PropRow::new(id, label)
        .hint(hints.get(current).copied().unwrap_or(hint))
        .show(ui, |ui| {
            Select::new(id.with("select"), &opts, Some(current))
                .fill_width()
                .show(ui)
                .picked
        });
    let picked = row.inner.unwrap_or(current);
    remember(ui, id, picked);
    picked
}

/// Auto (CL-019): what to analyse and the style, the Auto (Re-run)
/// button; with a correction, its offsets as read-only rows, Strength and
/// Apply to sliders.
fn auto_body(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    view: &AnalysisView,
) -> Option<Ask> {
    let p = theme::look(ui.ctx()).palette;
    let auto = shown.stack.auto;
    let mut ask = None;
    if let Some(a) = &auto {
        for (id, name, value) in words::auto_rows(a) {
            let manual = if shown.stack.sections_enabled.is_on(id.section()?) {
                shown.stack.get(id)
            } else {
                0
            };
            let limited = words::auto_limited(a, id, manual);
            let row_id = at.id("auto_row").with(id.as_str());
            PropRow::new(row_id, name)
                .hint(if limited {
                    "More than the slider's range allows on top of your own value: it stops at \
                     the end of the range"
                } else {
                    "What Auto adds to this slider at Strength 100 %"
                })
                .show(ui, |ui| {
                    let text = if limited {
                        format!("{value} · limited")
                    } else {
                        value
                    };
                    ui.label(egui::RichText::new(text).color(p.text_secondary));
                });
        }
        slider(ui, at, steps, shown, ColorParamId::AutoStrength, false);
    }
    if view.running == Some(true) {
        if running_row(ui, at, "Measuring the footage…") {
            ask = Some(Ask::Cancel);
        }
        return ask;
    }
    let sources = AutoSource::ALL.map(words::auto_source_label);
    let source_hints = AutoSource::ALL.map(words::auto_source_hint);
    let source = remembered_select(
        ui,
        at.id("auto_source"),
        "Analyze",
        "What Auto measures",
        &sources,
        &source_hints,
    );
    let styles = AutoStyle::ALL.map(words::auto_style_label);
    let style_hints = AutoStyle::ALL.map(words::auto_style_hint);
    let style = remembered_select(
        ui,
        at.id("auto_style"),
        "Style",
        "How Auto corrects",
        &styles,
        &style_hints,
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let enabled = view.blocked.is_none() && view.running.is_none();
        let label = if auto.is_some() {
            words::RERUN
        } else {
            words::AUTO_BUTTON
        };
        let text = crate::widgets::icon_then_text(ui, icons::ADJUST, label);
        let run = id_button(ui, at.id("auto_run"), enabled, text)
            .on_hover_text(words::AUTO_HINT)
            .on_disabled_hover_text(view.blocked.unwrap_or("Another analysis is running"));
        if run.clicked() {
            ask = Some(Ask::Auto(AutoStyle::ALL[style], AutoSource::ALL[source]));
        }
        if auto.is_some()
            && id_button(ui, at.id("auto_apply"), true, words::APPLY_TO_SLIDERS)
                .on_hover_text(words::APPLY_HINT)
                .clicked()
        {
            steps.edit(Edit::Color(ColorEdit::ApplyAutoToSliders), Merge::Own);
        }
    });
    ask
}

/// Match (CL-020): **Match color…** (pick mode), or the reference,
/// Intensity, Re-match and Change reference….
fn match_body(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    shown: &Shown<'_>,
    view: &AnalysisView,
) -> Option<Ask> {
    let p = theme::look(ui.ctx()).palette;
    let mut ask = None;
    if let Some((name, _)) = &view.reference {
        PropRow::new(at.id("reference"), "Reference")
            .hint("The frame this clip's colours were matched to")
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(name).color(p.text_secondary)).truncate(),
                );
            });
        slider(ui, at, steps, shown, ColorParamId::MatchIntensity, false);
    }
    if view.running == Some(false) {
        if running_row(ui, at, "Matching…") {
            ask = Some(Ask::Cancel);
        }
        return ask;
    }
    if let Some(graded) = view.picking {
        egui::Frame::new()
            .fill(p.bg_well)
            .stroke(egui::Stroke::new(1.0, p.accent))
            .corner_radius(theme::R_SM)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(words::PICK_SENTENCE).color(p.text_primary));
                let mut ungraded = !graded;
                if ui
                    .checkbox(&mut ungraded, words::UNGRADED)
                    .on_hover_text(words::UNGRADED_HINT)
                    .changed()
                {
                    ask = Some(Ask::Graded(!ungraded));
                }
                ui.horizontal(|ui| {
                    if id_button(ui, at.id("match_playhead"), true, words::USE_PLAYHEAD)
                        .on_hover_text(words::USE_PLAYHEAD_HINT)
                        .clicked()
                    {
                        ask = Some(Ask::MatchPlayhead);
                    }
                    if id_button(ui, at.id("cancel_pick"), true, "Cancel")
                        .on_hover_text("Leaves pick mode without changes (Esc)")
                        .clicked()
                    {
                        ask = Some(Ask::CancelPick);
                    }
                });
            });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ask = Some(Ask::CancelPick);
        }
        return ask;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let enabled = view.blocked.is_none() && view.running.is_none();
        let why = view.blocked.unwrap_or("Another analysis is running");
        match &view.reference {
            None => {
                let text = crate::widgets::icon_then_text(ui, icons::PALETTE, words::MATCH_BUTTON);
                if id_button(ui, at.id("match_pick"), enabled, text)
                    .on_hover_text(words::MATCH_HINT)
                    .on_disabled_hover_text(why)
                    .clicked()
                {
                    ask = Some(Ask::Pick);
                }
            }
            Some((_, again)) => {
                if id_button(ui, at.id("rematch"), enabled && *again, words::REMATCH)
                    .on_hover_text("Measures this clip and the same reference again")
                    .on_disabled_hover_text(if *again {
                        why
                    } else {
                        "The reference is gone, so the match stays as it is"
                    })
                    .clicked()
                {
                    ask = Some(Ask::Rematch);
                }
                if id_button(ui, at.id("match_change"), enabled, words::CHANGE_REFERENCE)
                    .on_hover_text("Pick another frame to match to")
                    .on_disabled_hover_text(why)
                    .clicked()
                {
                    ask = Some(Ask::Pick);
                }
            }
        }
    });
    ask
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

/// The footer: **Save look…** (CL-026: the first clip's grade at the
/// playhead, through the app's sheet) and **Reset all** (CL-024).
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
    let first = clips.first().map(|(id, stack, _)| (*id, stack.clone()));
    let savable = first
        .as_ref()
        .is_some_and(|(_, stack)| crate::looks::has_look_values(stack));
    let mut save = false;
    // CL-024, CL-031: with colour masks, Reset all asks first, in the
    // footer's own row, and names how many masks it removes.
    let masks: usize = ids
        .iter()
        .filter_map(|id| env.doc.clip(*id))
        .map(|c| c.color.as_ref().map_or(0, |s| s.masks.len()))
        .sum();
    let kind = env.kind;
    let asking_id = widget_id(kind, CardId::Grade, "reset-all-asking");
    let mut asking = ui.data(|d| d.get_temp::<bool>(asking_id)).unwrap_or(false) && masks > 0;
    ui.horizontal(|ui| {
        if asking {
            ui.label(words::reset_all_masks_question(masks))
                .on_hover_text("Every colour value back to its default; undo brings it all back");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cancel = widget_id(kind, CardId::Grade, "reset-all-cancel");
                if id_button(ui, cancel, true, "Cancel").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    asking = false;
                }
                let sure = widget_id(kind, CardId::Grade, "reset-all-sure");
                if id_button(ui, sure, true, "Reset all").clicked() {
                    reset = true;
                    asking = false;
                }
            });
            return;
        }
        save = ui
            .add_enabled(savable, egui::Button::new("Save look…"))
            .on_hover_text("Saves this grade to reuse on other clips and in other projects")
            .on_disabled_hover_text(crate::ui::save_look::NOTHING_TO_SAVE)
            .clicked();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = crate::widgets::icon_then_text(ui, icons::RESET, "Reset all");
            let id = widget_id(kind, CardId::Grade, "reset-all");
            let clicked = id_button(ui, id, changed && !locked, label)
                .on_hover_text(
                    "Every colour value back to its default, Amount to 100 %, and the \
                     colour keyframes removed",
                )
                .on_disabled_hover_text("Nothing to reset: the colour is at its defaults")
                .clicked();
            if clicked && masks > 0 {
                asking = true;
            } else {
                reset = clicked;
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(asking_id, asking));
    if save && let Some((id, stack)) = first {
        let keyed = env
            .doc
            .clip(id)
            .is_some_and(|c| c.keys.tracks().iter().any(|t| is_color_prop(t.prop())));
        env.out.save_look = Some(super::SaveLookRequest { stack, keyed });
    }
    if reset {
        let mut steps = Steps::default();
        steps.edit(Edit::Color(ColorEdit::ResetAll), Merge::Own);
        let refused = props::commit(env.project, env.drag, &ids, steps, Held::Up);
        let result = refused.or_else(|| {
            if masks > 0 {
                Some(format!(
                    "Reset all colour removed {}{}. Undo brings them back",
                    words::masks_count(masks),
                    match removed {
                        0 => String::new(),
                        1 => " and 1 keyframe".to_owned(),
                        n => format!(" and {n} keyframes"),
                    }
                ))
            } else {
                reset_sentence("Reset all colour", removed)
            }
        });
        status(env, result);
    }
}
