//! The Sound lens (AU-002, AU-005 … AU-008, AU-038): the Volume card with
//! its keyframe navigator (AU-008, TR-027) and the mute switch, the Fades
//! card with the fade curves, the sound source, and for several clips the
//! Mute and Detach commands. The footer resets every sound setting as one
//! undo entry.

use eframe::egui::{self, DragValue, Key, Modifiers, Response};
use freecut_timeline::edit::{AudioEdit, EditRequest, GainKeyEdit};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Clip, ClipAudio, ClipContent, EmbeddedAudio, FadeCurve, GainDb, MediaKind, StreamSel,
};
use freecut_timeline::time::Tick;

use super::lens::{CardId, Detail, widget_id};
use super::{Env, InspectorOutput, card};
use crate::clip_keys::{self, KeyButton, Step};
use crate::clip_sound::{
    ClipCommand, SoundCommand, fade_from_secs, fade_limit, format_db, gain_db, gain_from_db,
    gain_from_slider, gain_percent, mute_command, parse_db, slider_from_gain, sound_command,
    step_gain,
};
use crate::clip_streams::{self, StreamPicker};
use crate::session::Project;
use crate::ui::drag::DragGesture;
use crate::widgets::card::{self as card_widget, PropertyCard};
use crate::widgets::field::{Framed, compact, field_width};
use crate::widgets::keyframe::{KeyState, KeyframeNav};
use crate::widgets::row::PropRow;
use crate::widgets::select::{Select, SelectOption};
use crate::widgets::slider::SliderField;

const OFF_CLIP: &str = "Move the playhead onto this clip to change its volume keyframes";

/// The Sound lens's cards for the selection.
pub(super) fn cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let doc = env.doc.clone();
    let selection = env.selection.clone();
    let [id] = selection.as_slice() else {
        several(ui, env, &selection);
        return;
    };
    let Some(clip) = doc.clip(*id) else {
        return;
    };
    let is_video = matches!(
        clip.content,
        ClipContent::Media {
            kind: MediaKind::Video,
            ..
        }
    );
    match &clip.audio {
        EmbeddedAudio::Active(audio) => {
            volume_card(ui, env, clip, audio);
            fades_card(ui, env, clip, audio);
        }
        EmbeddedAudio::Detached(_) => {
            let kind = env.kind;
            let mut restore = false;
            card(
                ui,
                env.workspace,
                kind,
                CardId::SoundState,
                "Sound",
                "detached".to_owned(),
                |ui, _| {
                    ui.label("Audio is detached");
                    // AU-038: the stream belongs to the detached clips.
                    stream_row(ui, env.project, *id, env.out);
                    restore = ui
                        .button("Restore audio")
                        .on_hover_text("Give this clip its sound back")
                        .clicked();
                },
            );
            if restore {
                env.out.command = Some(ClipCommand::Restore);
            }
        }
        EmbeddedAudio::None if is_video => {
            card(
                ui,
                env.workspace,
                env.kind,
                CardId::SoundState,
                "Sound",
                "none".to_owned(),
                |ui, _| {
                    ui.weak("This clip has no sound");
                },
            );
        }
        EmbeddedAudio::None => {}
    }
}

/// Mute and Detach / Restore for several clips (AU-007, TL-030).
fn several(ui: &mut egui::Ui, env: &mut Env<'_, '_>, clips: &[ClipId]) {
    let doc = env.doc.clone();
    let out = &mut *env.out;
    card(
        ui,
        env.workspace,
        env.kind,
        CardId::SoundCommands,
        "Sound",
        String::new(),
        |ui, _| {
            ui.horizontal(|ui| {
                mute_button(ui, &doc, clips, out);
                sound_command_button(ui, &doc, clips, out);
            });
        },
    );
}

/// The footer of a clip's Sound lens: Detach audio for video clips, and
/// Reset lens (every sound setting, one entry).
pub(super) fn footer(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let doc = env.doc.clone();
    let selection = env.selection.clone();
    let [id] = selection.as_slice() else {
        return;
    };
    let is_video = doc.clip(*id).is_some_and(|c| {
        matches!(
            c.content,
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            }
        )
    });
    let mut reset = false;
    ui.horizontal(|ui| {
        if is_video {
            sound_command_button(ui, &doc, &selection, env.out);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            reset = ui
                .button("Reset lens")
                .on_hover_text("Volume 0 dB without keyframes, unmuted, no fades")
                .clicked();
        });
    });
    if reset && let Err(e) = reset_sound(env.project, env.drag, *id) {
        env.out.status = Some(e);
    }
}

/// One change the sound cards ask for this frame.
enum Edit {
    /// The Volume slider or field: the static volume, or the key at the
    /// playhead (TR-027).
    Volume(GainDb),
    Audio(AudioEdit),
}

/// Volume (with its keyframes), the mute switch, and the sound source
/// behind More.
fn volume_card(ui: &mut egui::Ui, env: &mut Env<'_, '_>, clip: &Clip, audio: &ClipAudio) {
    let kind = env.kind;
    let id = clip.id;
    let playhead = env.project.playhead;
    let keyframed = audio.gain.is_animated();
    // TR-027: with keys, Volume shows (and sets) the value at the playhead.
    let gain = if keyframed {
        clip_keys::volume_at_playhead(clip, audio, playhead)
    } else {
        *audio.gain.static_value()
    };
    let editable = !keyframed || clip_keys::source_at(clip, playhead).is_some();
    let everything = env.workspace.detail == Detail::Everything;
    let mut clear_keys = false;
    let mut toggle_key = false;
    let mut stream_pick = None;
    let mut edits: Vec<(Edit, Response)> = Vec::new();
    let mut finished: Vec<Response> = Vec::new();
    let mut cancelled = false;
    let summary = if audio.muted {
        "muted".to_owned()
    } else {
        format_db(gain_db(gain))
    };
    let memory = env.workspace.card(kind, CardId::Volume);
    let project = &mut *env.project;
    let shown = PropertyCard::new(widget_id(kind, CardId::Volume, "card"), "Volume")
        .summary(summary)
        .switch(!audio.muted, "Sound on or off (mute)")
        .show(ui, memory, |ui, memory| {
            // AU-008: the keyframe diamond and the arrows to the previous
            // and next keyframe.
            let button = clip_keys::key_button(clip, audio, playhead);
            let prev = clip_keys::adjacent_key(clip, audio, playhead, Step::Previous);
            let next = clip_keys::adjacent_key(clip, audio, playhead, Step::Next);
            let state = if keyframed {
                KeyState::Animated {
                    on_key: matches!(button, KeyButton::Remove(_)),
                    prev: prev.is_some(),
                    next: next.is_some(),
                }
            } else {
                KeyState::NotAnimated
            };
            let nav = KeyframeNav {
                property: "volume",
                state,
                disabled: match button {
                    KeyButton::Disabled(reason) => Some(reason),
                    _ => None,
                },
            };
            let row_id = widget_id(kind, CardId::Volume, "volume");
            let row = PropRow::new(row_id, "Volume")
                .changed(!keyframed && gain != GainDb::ZERO)
                .keys(nav)
                .stacked()
                .show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        // The rail takes what the dB field leaves.
                        let field_w = field_width(true, compact(ui));
                        let rail_w = (ui.available_width() - field_w - 8.0).max(48.0);
                        let rail = ui
                            .allocate_ui(egui::vec2(rail_w, ui.available_height()), |ui| {
                                SliderField::new(
                                    row_id,
                                    "Volume",
                                    slider_from_gain(gain),
                                    0.0..=1.0,
                                )
                                .without_field()
                                .show(ui)
                            })
                            .inner;
                        ui.add_space(8.0 - ui.spacing().item_spacing.x);
                        let (typed, field) = volume_field(ui, gain);
                        (rail, typed, field)
                    })
                    .inner
                });
            let (rail, typed, field) = row.inner;
            let rail_response = rail
                .response
                .clone()
                .on_hover_text(format!(
                    "{:.0}% of the original level · double-click for 0 dB",
                    gain_percent(gain)
                ))
                .on_disabled_hover_text(OFF_CLIP);
            if rail.cancelled {
                cancelled = true;
            } else if rail.double_clicked {
                edits.push((Edit::Volume(GainDb::ZERO), rail_response));
            } else if rail.changed {
                let wanted = gain_from_slider(rail.value);
                if wanted != gain {
                    edits.push((Edit::Volume(wanted), rail_response));
                } else {
                    finished.push(rail_response);
                }
            } else {
                finished.push(rail_response);
            }
            let field = field.on_disabled_hover_text(OFF_CLIP);
            if row.reset {
                edits.push((Edit::Volume(GainDb::ZERO), field.clone()));
            }
            if typed != gain {
                edits.push((Edit::Volume(typed), field));
            } else {
                finished.push(field);
            }
            toggle_key = row.keys.toggle;
            if row.keys.prev {
                env.out.seek = prev;
            }
            if row.keys.next {
                env.out.seek = next;
            }
            if keyframed {
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "{} keyframes · value at the playhead",
                        audio.gain.keys().len()
                    ));
                    clear_keys = ui
                        .small_button("Remove all")
                        .on_hover_text("Go back to one volume for the whole clip")
                        .clicked();
                });
            }
            if let Some(picker) = clip_streams::stream_picker(project.doc(), id) {
                card_widget::more(
                    ui,
                    widget_id(kind, CardId::Volume, "more"),
                    memory,
                    everything,
                    1,
                    |ui| {
                        PropRow::new(widget_id(kind, CardId::Volume, "source"), "Sound source")
                            .show(ui, |ui| {
                                stream_pick = stream_combo(ui, &picker);
                            });
                    },
                );
            }
        });
    let drag = &mut *env.drag;
    if let Some(on) = shown.switched {
        drag.end(project);
        let req = EditRequest::SetClipAudio {
            seq: project.doc().root(),
            clips: vec![id],
            edit: AudioEdit::Muted(!on),
        };
        if let Err(e) = project.apply(&req, Coalesce::Never) {
            env.out.status = Some(e);
        }
    }
    if cancelled && let Err(e) = drag.cancel(project) {
        env.out.status = Some(e);
    }
    if clear_keys {
        drag.end(project);
        let req = EditRequest::EditGainKeys {
            seq: project.doc().root(),
            clips: vec![id],
            edit: GainKeyEdit::Clear,
        };
        if let Err(e) = project.apply(&req, Coalesce::Never) {
            env.out.status = Some(e);
        }
    }
    if let Some(sel) = stream_pick {
        drag.end(project);
        if let Err(e) = clip_streams::set_stream(project, &[id], sel) {
            env.out.status = Some(e);
        }
    }
    if toggle_key {
        drag.end(project);
        if let Err(e) = clip_keys::toggle_key(project, id) {
            env.out.status = Some(e);
        }
    }
    apply_edits(project, drag, id, edits, env.out);
    for response in finished {
        drag.finish(project, &response);
    }
}

/// Fade in and out with their curves (AU-006).
fn fades_card(ui: &mut egui::Ui, env: &mut Env<'_, '_>, clip: &Clip, audio: &ClipAudio) {
    let kind = env.kind;
    let id = clip.id;
    let duration = clip.duration();
    let mut edits: Vec<(Edit, Response)> = Vec::new();
    let mut finished: Vec<Response> = Vec::new();
    let summary = format!(
        "in {:.1} s · out {:.1} s",
        audio.fade_in.as_secs_f64(),
        audio.fade_out.as_secs_f64()
    );
    card(
        ui,
        env.workspace,
        kind,
        CardId::Fades,
        "Fades",
        summary,
        |ui, _| {
            let fade_in_max = fade_limit(duration, audio.fade_out);
            let fade_out_max = fade_limit(duration, audio.fade_in);
            for (row, label, value, max, make, curve, make_curve) in [
                (
                    "in",
                    "Fade in",
                    audio.fade_in,
                    fade_in_max,
                    AudioEdit::FadeIn as fn(Tick) -> AudioEdit,
                    audio.fade_in_curve,
                    AudioEdit::FadeInCurve as fn(FadeCurve) -> AudioEdit,
                ),
                (
                    "out",
                    "Fade out",
                    audio.fade_out,
                    fade_out_max,
                    AudioEdit::FadeOut,
                    audio.fade_out_curve,
                    AudioEdit::FadeOutCurve,
                ),
            ] {
                let row_id = widget_id(kind, CardId::Fades, row);
                let options = FadeCurve::ALL.map(|c| SelectOption::new(curve_name(c)));
                let current = FadeCurve::ALL.iter().position(|c| *c == curve);
                let shown = PropRow::new(row_id, label)
                    .changed(value != Tick::ZERO)
                    .show(ui, |ui| {
                        let mut secs = value.as_secs_f64();
                        let key = row_id.with("field");
                        let field = ui
                            .push_id(key, |ui| {
                                Framed::new(key, "s").show(ui, |ui| {
                                    ui.add(
                                        DragValue::new(&mut secs)
                                            .range(0.0..=max.as_secs_f64())
                                            .clamp_existing_to_range(false)
                                            .speed(0.05)
                                            .fixed_decimals(1)
                                            .update_while_editing(false),
                                    )
                                })
                            })
                            .inner
                            .on_hover_text(format!(
                                "Up to {:.1} s, so both fades fit in the clip",
                                max.as_secs_f64()
                            ));
                        // AU-006: the fade's shape.
                        let select = Select::new(row_id.with("curve"), &options, current)
                            .fill_width()
                            .show(ui);
                        let picked = select
                            .picked
                            .and_then(|i| FadeCurve::ALL.get(i).copied())
                            .unwrap_or(curve);
                        let combo = select.response.on_hover_text("The shape of the fade");
                        (field, secs, combo, picked)
                    });
                let (field, secs, combo, picked) = shown.inner;
                if shown.reset {
                    edits.push((Edit::Audio(make(Tick::ZERO)), combo.clone()));
                }
                if field.changed() {
                    let ticks = fade_from_secs(secs, max);
                    if ticks != value {
                        edits.push((Edit::Audio(make(ticks)), field));
                    } else {
                        finished.push(field);
                    }
                } else {
                    finished.push(field);
                }
                if picked != curve {
                    edits.push((Edit::Audio(make_curve(picked)), combo));
                }
            }
        },
    );
    let project = &mut *env.project;
    let drag = &mut *env.drag;
    apply_edits(project, drag, id, edits, env.out);
    for response in finished {
        drag.finish(project, &response);
    }
}

fn apply_edits(
    project: &mut Project,
    drag: &mut DragGesture,
    id: ClipId,
    edits: Vec<(Edit, Response)>,
    out: &mut InspectorOutput,
) {
    for (edit, response) in edits {
        let result = match edit {
            Edit::Volume(gain) => {
                let coalesce = drag.coalesce(project, &response);
                clip_keys::set_volume(project, id, gain, coalesce)
            }
            Edit::Audio(edit) => commit(project, drag, &response, id, edit),
        };
        if let Err(e) = result {
            out.status = Some(e);
        }
        drag.finish(project, &response);
    }
}

/// Mute / Unmute for the selection (AU-007's mixed-selection rule).
fn mute_button(
    ui: &mut egui::Ui,
    doc: &freecut_timeline::model::Document,
    clips: &[ClipId],
    out: &mut InspectorOutput,
) {
    match mute_command(doc, clips) {
        Ok(mute) => {
            let label = if mute { "Mute audio" } else { "Unmute audio" };
            if ui.button(label).clicked() {
                out.command = Some(ClipCommand::SetMuted(mute));
            }
        }
        Err(reason) => {
            ui.add_enabled(false, egui::Button::new("Mute audio"))
                .on_disabled_hover_text(reason);
        }
    }
}

/// Detach audio / Restore audio for the selection (TL-030).
fn sound_command_button(
    ui: &mut egui::Ui,
    doc: &freecut_timeline::model::Document,
    clips: &[ClipId],
    out: &mut InspectorOutput,
) {
    match sound_command(doc, clips) {
        SoundCommand::Detach(Ok(())) => {
            if ui
                .button("Detach audio")
                .on_hover_text("Move the sound to its own clip on an audio track")
                .clicked()
            {
                out.command = Some(ClipCommand::Detach);
            }
        }
        SoundCommand::Detach(Err(reason)) => {
            ui.add_enabled(false, egui::Button::new("Detach audio"))
                .on_disabled_hover_text(reason);
        }
        SoundCommand::Restore => {
            if ui.button("Restore audio").clicked() {
                out.command = Some(ClipCommand::Restore);
            }
        }
    }
}

/// The Sound source picker (AU-038). Returns the new choice, if one was
/// made.
fn stream_combo(ui: &mut egui::Ui, picker: &StreamPicker) -> Option<Option<StreamSel>> {
    let mut picked = picker.current;
    let current = picker
        .choices
        .get(picker.current)
        .map_or("", |c| c.label.as_str());
    let options: Vec<SelectOption<'_>> = picker
        .choices
        .iter()
        // A stream a linked clip already plays shows disabled, with the
        // reason (AU-038).
        .map(|c| SelectOption::new(&c.label).blocked(c.blocked))
        .collect();
    let inner = ui.add_enabled_ui(picker.disabled.is_none(), |ui| {
        let out = Select::new(
            egui::Id::new("sound-source"),
            &options,
            Some(picker.current),
        )
        .text(current)
        .fill_width()
        .show(ui);
        if let Some(i) = out.picked {
            picked = i;
        }
        out.response
    });
    let response = inner
        .inner
        .on_hover_text("Which of the file's audio streams plays");
    if let Some(reason) = picker.disabled {
        response.on_disabled_hover_text(reason);
    }
    (picked != picker.current).then(|| picker.choices.get(picked).and_then(|c| c.sel))
}

/// The Sound source row of a clip whose sound is detached: shown, disabled,
/// with the reason (AU-038).
fn stream_row(ui: &mut egui::Ui, project: &Project, clip: ClipId, out: &mut InspectorOutput) {
    let Some(picker) = clip_streams::stream_picker(project.doc(), clip) else {
        return;
    };
    let row = PropRow::new(egui::Id::new("freecut-detached-source"), "Sound source")
        .show(ui, |ui| stream_combo(ui, &picker));
    if row.inner.is_some() {
        out.status = Some("Choose the stream on the detached audio clips".to_owned());
    }
}

/// The Volume field (AU-005): typed or dragged decibels in a number
/// field with a `dB` unit segment, and while it has focus Arrow Up/Down
/// step ±1 dB, ±0.1 dB with Shift. Returns the gain it shows after this
/// frame's input.
fn volume_field(ui: &mut egui::Ui, gain: GainDb) -> (GainDb, Response) {
    let key = egui::Id::new("freecut-volume-field");
    let mut shown = gain;
    let field = Framed::new(key, "dB").show(ui, |ui| {
        // The field's own arrow steps follow its drag speed (0.1 dB) and
        // ignore Shift, so it never sees the arrow keys.
        let id = ui.next_auto_id();
        let mut stepped = gain;
        if ui.memory(|m| m.has_focus(id)) {
            let (coarse, fine) = ui.input_mut(|i| {
                let mut count = |m, k| i.count_and_consume_key(m, k) as i32;
                // Shift first: a plain pattern also matches a shifted key.
                let fine =
                    count(Modifiers::SHIFT, Key::ArrowUp) - count(Modifiers::SHIFT, Key::ArrowDown);
                let coarse =
                    count(Modifiers::NONE, Key::ArrowUp) - count(Modifiers::NONE, Key::ArrowDown);
                (coarse, fine)
            });
            stepped = step_gain(
                gain,
                coarse
                    .saturating_mul(100)
                    .saturating_add(fine.saturating_mul(10)),
            );
        }
        let mut db = gain_db(stepped);
        let field = ui.add(
            DragValue::new(&mut db)
                .speed(0.1)
                .update_while_editing(false)
                .custom_formatter(|v, _| format_db(v).trim_end_matches(" dB").to_owned())
                .custom_parser(parse_db),
        );
        shown = if field.changed() {
            gain_from_db(db)
        } else {
            stepped
        };
        field
    });
    (shown, field)
}

/// Commits one sound edit. While `response` is being dragged the edits of
/// that drag merge into one undo entry; anything else is its own entry.
fn commit(
    project: &mut Project,
    drag: &mut DragGesture,
    response: &Response,
    clip: ClipId,
    edit: AudioEdit,
) -> Result<(), String> {
    let coalesce = drag.coalesce(project, response);
    let req = EditRequest::SetClipAudio {
        seq: project.doc().root(),
        clips: vec![clip],
        edit,
    };
    project.apply(&req, coalesce).map(|_| ())
}

/// Every sound setting back to its default, as one undo entry.
fn reset_sound(project: &mut Project, drag: &mut DragGesture, clip: ClipId) -> Result<(), String> {
    drag.end(project);
    let defaults = ClipAudio::default();
    let g = project.begin_gesture();
    let seq = project.doc().root();
    let keyframed = project
        .doc()
        .clip(clip)
        .and_then(|c| c.audio.active())
        .is_some_and(|a| a.gain.is_animated());
    let mut requests = Vec::new();
    if keyframed {
        requests.push(EditRequest::EditGainKeys {
            seq,
            clips: vec![clip],
            edit: GainKeyEdit::Clear,
        });
    }
    for edit in [
        AudioEdit::Gain(*defaults.gain.static_value()),
        AudioEdit::Muted(defaults.muted),
        AudioEdit::FadeIn(defaults.fade_in),
        AudioEdit::FadeOut(defaults.fade_out),
        AudioEdit::FadeInCurve(defaults.fade_in_curve),
        AudioEdit::FadeOutCurve(defaults.fade_out_curve),
    ] {
        requests.push(EditRequest::SetClipAudio {
            seq,
            clips: vec![clip],
            edit,
        });
    }
    let mut result = Ok(());
    for req in requests {
        if let Err(e) = project.apply(&req, Coalesce::Gesture(g)) {
            result = Err(e);
            break;
        }
    }
    project.end_gesture(g);
    result
}

/// A fade shape's name in the inspector (AU-006).
fn curve_name(curve: FadeCurve) -> &'static str {
    match curve {
        FadeCurve::Linear => "Linear",
        FadeCurve::Smooth => "Smooth",
        FadeCurve::Fast => "Fast",
        FadeCurve::Slow => "Slow",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Key, Modifiers, Rect};
    use freecut_timeline::model::GainDb;

    use super::volume_field;
    use crate::ui::drag::tests::{Frames, button, key};

    #[test]
    fn the_focused_volume_field_steps_1_db_and_0_1_db_with_shift() {
        let mut frames = Frames::new();
        let mut gain = GainDb::from_hundredths(-600);
        let frame = |frames: &mut Frames, events: Vec<Event>, gain: &mut GainDb| {
            let mut rect = Rect::NOTHING;
            frames.run(events, |ui| {
                let (shown, field) = volume_field(ui, *gain);
                *gain = shown;
                rect = field.rect;
            });
            rect
        };
        let rect = frame(&mut frames, Vec::new(), &mut gain);
        // A click puts the field in focus.
        let at = rect.center();
        frame(
            &mut frames,
            vec![Event::PointerMoved(at), button(at, true)],
            &mut gain,
        );
        frame(&mut frames, vec![button(at, false)], &mut gain);
        frame(&mut frames, Vec::new(), &mut gain);
        assert_eq!(gain, GainDb::from_hundredths(-600));

        frame(
            &mut frames,
            vec![key(Key::ArrowUp, Modifiers::NONE)],
            &mut gain,
        );
        assert_eq!(gain, GainDb::from_hundredths(-500), "Arrow Up: +1 dB");
        frame(
            &mut frames,
            vec![key(Key::ArrowDown, Modifiers::SHIFT)],
            &mut gain,
        );
        assert_eq!(
            gain,
            GainDb::from_hundredths(-510),
            "Shift+Arrow Down: -0.1 dB"
        );
        frame(
            &mut frames,
            vec![
                key(Key::ArrowUp, Modifiers::SHIFT),
                key(Key::ArrowUp, Modifiers::SHIFT),
                key(Key::ArrowDown, Modifiers::NONE),
            ],
            &mut gain,
        );
        assert_eq!(gain, GainDb::from_hundredths(-590));
    }

    #[test]
    fn arrow_keys_do_nothing_while_the_volume_field_is_not_focused() {
        let mut frames = Frames::new();
        let mut gain = GainDb::ZERO;
        for _ in 0..2 {
            frames.run(vec![key(Key::ArrowUp, Modifiers::NONE)], |ui| {
                gain = volume_field(ui, gain).0;
            });
        }
        assert_eq!(gain, GainDb::ZERO);
    }
}
