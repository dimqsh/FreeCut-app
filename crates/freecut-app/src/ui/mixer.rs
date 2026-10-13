//! The Mixer panel (AU-013) and track solo (AU-012).
//!
//! One strip per track that carries sound (video tracks, idle when none of
//! their clips plays sound, and audio tracks), then the master strip. A
//! strip has a fader (−∞, −60 to +12 dB), pan (−100 to +100), mute and solo.
//! Double-click resets a control, the fields under it take typed values,
//! and a drag is one undo entry (TL-043). Solo is view state: it changes
//! what preview plays, never the document.

use std::collections::BTreeSet;

use eframe::egui::{self, DragValue, Response, RichText};
use freecut_timeline::edit::EditRequest;
use freecut_timeline::ids::{SeqId, TrackId};
use freecut_timeline::model::{Document, GainDb, MasterAudio, Pan, TrackAudio, TrackHeader};

use super::drag::DragGesture;
use super::timeline::track_name;
use crate::clip_sound::{DETENT, UNITY, format_db, gain_db, gain_from_db, parse_db};
use crate::session::Project;

/// Fader length in points.
const FADER_LEN: f32 = 110.0;
const STRIP_W: f32 = 84.0;

/// The panel's state: whether it shows, and its drag.
#[derive(Debug, Default)]
pub struct MixerState {
    pub open: bool,
    drag: DragGesture,
    /// Where each fader was drawn last frame, strips in order then master
    /// (for tests that drag them).
    #[cfg(test)]
    faders: Vec<egui::Rect>,
}

impl MixerState {
    /// Ends a drag whose widget went away. Call once per frame.
    pub fn settle(&mut self, project: Option<&mut Project>, pointer_down: bool) {
        self.drag.settle(project, pointer_down);
    }

    /// Forgets a drag of a project that is no longer open.
    pub fn reset(&mut self) {
        self.drag.reset();
    }
}

/// One mixer strip: a track that carries sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    pub track: TrackId,
    pub name: String,
    pub mix: TrackAudio,
    pub muted: bool,
    pub soloed: bool,
    /// No clip on the track plays sound (a video track whose sound is all
    /// detached, or an empty track): shown as idle (AU-013).
    pub idle: bool,
}

/// The strips of `seq`, in timeline order: video tracks top to bottom, then
/// audio tracks top to bottom.
pub fn strips(doc: &Document, seq: SeqId, solo: &BTreeSet<TrackId>) -> Vec<Strip> {
    let Some(sequence) = doc.sequence(seq) else {
        return Vec::new();
    };
    sequence
        .visual_tracks()
        .iter()
        .rev()
        .chain(sequence.audio_tracks())
        .filter(|t| t.kind.carries_sound())
        .map(|t| Strip {
            track: t.id,
            name: track_name(doc, seq, t.id),
            mix: t.mix,
            muted: t.header.flags.muted,
            soloed: solo.contains(&t.id),
            idle: t.clips().iter().all(|c| c.audio.active().is_none()),
        })
        .collect()
}

/// The solo set after clicking a track's S (AU-012): a click solos only
/// that track (or, when it already is the only soloed one, un-solos it);
/// with `add` (Shift-click) the track joins or leaves the set.
pub fn solo_after_click(
    current: &BTreeSet<TrackId>,
    track: TrackId,
    add: bool,
) -> BTreeSet<TrackId> {
    let mut next = current.clone();
    if add {
        if !next.remove(&track) {
            next.insert(track);
        }
    } else if current.len() == 1 && current.contains(&track) {
        next.clear();
    } else {
        next = BTreeSet::from([track]);
    }
    next
}

/// The fader position (0..=1) showing `gain`: below `DETENT` is −∞,
/// `DETENT` is −60 dB, `UNITY` is 0 dB and 1 is +12 dB. Below 0 dB the
/// scale is quadratic like the clip Volume slider, so the range near 0 dB
/// gets the most travel.
pub fn fader_from_gain(gain: GainDb) -> f64 {
    if gain.is_silent() {
        return 0.0;
    }
    let db = gain_db(gain).min(top_db());
    if db >= 0.0 {
        UNITY + db / top_db() * (1.0 - UNITY)
    } else {
        UNITY - (-db / 60.0).sqrt() * (UNITY - DETENT)
    }
}

/// The gain at fader position `u`, on a 0.1 dB grid, at most +12 dB.
pub fn gain_from_fader(u: f64) -> GainDb {
    if u.is_nan() || u < DETENT {
        return GainDb::SILENT;
    }
    let db = if u >= UNITY {
        (u.min(1.0) - UNITY) / (1.0 - UNITY) * top_db()
    } else {
        let s = (UNITY - u) / (UNITY - DETENT);
        -60.0 * s * s
    };
    TrackAudio::clamp_gain(gain_from_db((db * 10.0).round() / 10.0))
}

/// The loudest fader value in dB (AU-010, AU-013: the same +12 dB).
fn top_db() -> f64 {
    gain_db(TrackAudio::MAX_GAIN)
}

/// "C", "L30", "R100".
pub fn pan_label(percent: f64) -> String {
    let p = percent.round() as i64;
    match p {
        0 => "C".to_owned(),
        p if p < 0 => format!("L{}", -p),
        p => format!("R{p}"),
    }
}

/// A typed pan: "C", "L30", "R 30", "-30" or "30", clamped to ±100.
pub fn parse_pan(text: &str) -> Option<f64> {
    let t = text.trim().replace('−', "-").to_uppercase();
    if matches!(t.as_str(), "C" | "CENTRE" | "CENTER") {
        return Some(0.0);
    }
    let (sign, digits) = if let Some(rest) = t.strip_prefix('L') {
        (-1.0, rest)
    } else if let Some(rest) = t.strip_prefix('R') {
        (1.0, rest)
    } else {
        (1.0, t.as_str())
    };
    let v: f64 = digits.trim().parse().ok()?;
    v.is_finite()
        .then(|| (sign * v).round().clamp(-100.0, 100.0))
}

/// The compact volume readout on a track header (AU-013): "0.0 dB",
/// "−6.0 dB", "−∞ dB".
pub fn track_readout(gain: GainDb) -> String {
    format_db(gain_db(gain))
}

/// Draws the mixer and carries out its edits. Returns why an edit failed.
pub fn show(ui: &mut egui::Ui, project: &mut Project, state: &mut MixerState) -> Option<String> {
    let doc = project.doc().clone();
    let seq = doc.root();
    let list = strips(&doc, seq, project.solo());
    let mut status = None;
    let shift = ui.input(|i| i.modifiers.shift);
    #[cfg(test)]
    state.faders.clear();
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal_top(|ui| {
            if list.is_empty() {
                ui.weak("No track carries sound yet.");
            }
            for strip in &list {
                ui.push_id(("mixer-strip", strip.track), |ui| {
                    strip_frame(ui, |ui| {
                        if let Some(e) = track_strip(ui, project, state, seq, strip, shift) {
                            status = Some(e);
                        }
                    });
                });
            }
            ui.separator();
            strip_frame(ui, |ui| {
                if let Some(e) = master_strip(ui, project, state, seq) {
                    status = Some(e);
                }
            });
        });
    });
    status
}

fn strip_frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(STRIP_W);
        ui.vertical_centered(add);
    });
}

/// What a strip control asks for this frame.
enum StripEdit {
    Gain(GainDb),
    Pan(Pan),
    Mute(bool),
    Solo,
}

fn track_strip(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MixerState,
    seq: SeqId,
    strip: &Strip,
    shift: bool,
) -> Option<String> {
    let name = RichText::new(&strip.name).strong();
    ui.add(egui::Label::new(name).truncate())
        .on_hover_text(&strip.name);
    if strip.idle {
        ui.weak("idle")
            .on_hover_text("No clip on this track plays sound");
    } else {
        ui.weak(" ");
    }
    let mut edits: Vec<(StripEdit, Response)> = Vec::new();
    if let Some((gain, r)) = fader(ui, strip.mix.gain, state, project) {
        edits.push((StripEdit::Gain(gain), r));
    }
    if let Some((pan, r)) = pan_control(ui, strip.mix.pan, &mut state.drag, project) {
        edits.push((StripEdit::Pan(pan), r));
    }
    ui.horizontal(|ui| {
        let m = ui
            .selectable_label(strip.muted, "M")
            .on_hover_text("Mute track");
        if m.clicked() {
            edits.push((StripEdit::Mute(!strip.muted), m));
        }
        let s = ui.selectable_label(strip.soloed, "S").on_hover_text(
            "Solo: hear only this track (Shift-click to add or remove). Preview only",
        );
        if s.clicked() {
            edits.push((StripEdit::Solo, s));
        }
    });
    let mut status = None;
    for (edit, response) in edits {
        let req = match edit {
            StripEdit::Gain(gain) => EditRequest::SetTrackGain {
                seq,
                track: strip.track,
                gain,
            },
            StripEdit::Pan(pan) => EditRequest::SetTrackPan {
                seq,
                track: strip.track,
                pan,
            },
            StripEdit::Mute(muted) => {
                let Some((_, track)) = project
                    .doc()
                    .sequence(seq)
                    .and_then(|s| s.find_track(strip.track))
                else {
                    continue;
                };
                let mut flags = track.header.flags;
                flags.muted = muted;
                EditRequest::SetTrackHeader {
                    seq,
                    track: strip.track,
                    header: TrackHeader {
                        name: track.header.name.clone(),
                        flags,
                    },
                }
            }
            StripEdit::Solo => {
                let next = solo_after_click(project.solo(), strip.track, shift);
                project.set_solo(next);
                continue;
            }
        };
        let coalesce = state.drag.coalesce(project, &response);
        if let Err(e) = project.apply(&req, coalesce) {
            status = Some(e);
        }
        state.drag.finish(project, &response);
    }
    status
}

fn master_strip(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MixerState,
    seq: SeqId,
) -> Option<String> {
    ui.add(egui::Label::new(RichText::new("Master").strong()));
    ui.weak("whole mix");
    let gain = project
        .doc()
        .sequence(seq)
        .map_or(GainDb::ZERO, |s| s.master.gain);
    let (wanted, response) = fader(ui, gain, state, project)?;
    let req = EditRequest::SetMasterGain {
        seq,
        gain: MasterAudio::clamp_gain(wanted),
    };
    let coalesce = state.drag.coalesce(project, &response);
    let status = project.apply(&req, coalesce).err();
    state.drag.finish(project, &response);
    status
}

/// A vertical fader with a typed-value field under it. Returns the gain
/// asked for and the widget that asked, when it differs from `gain`.
fn fader(
    ui: &mut egui::Ui,
    gain: GainDb,
    state: &mut MixerState,
    project: &mut Project,
) -> Option<(GainDb, Response)> {
    let mut u = fader_from_gain(gain);
    let slider = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = FADER_LEN;
            ui.add(
                egui::Slider::new(&mut u, 0.0..=1.0)
                    .vertical()
                    .show_value(false),
            )
        })
        .inner
        .on_hover_text("Volume: −∞ or −60 to +12 dB · double-click for 0 dB");
    #[cfg(test)]
    state.faders.push(slider.rect);
    let mut db = gain_db(gain);
    let field = ui.add(
        DragValue::new(&mut db)
            .speed(0.1)
            .update_while_editing(false)
            .custom_formatter(|v, _| format_db(v))
            .custom_parser(parse_db),
    );
    let from_slider = if slider.double_clicked() {
        Some(GainDb::ZERO)
    } else if slider.changed() {
        Some(gain_from_fader(u))
    } else {
        None
    };
    let from_field = field
        .changed()
        .then(|| TrackAudio::clamp_gain(gain_from_db(db)));
    pick(
        gain,
        [(from_slider, slider), (from_field, field)],
        &mut state.drag,
        project,
    )
}

/// A pan slider with a typed-value field. Returns the pan asked for and the
/// widget that asked, when it differs from `pan`.
fn pan_control(
    ui: &mut egui::Ui,
    pan: Pan,
    drag: &mut DragGesture,
    project: &mut Project,
) -> Option<(Pan, Response)> {
    let mut p = i32::from(pan.percent());
    let slider = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = STRIP_W - 12.0;
            ui.add(egui::Slider::new(&mut p, -100..=100).show_value(false))
        })
        .inner
        .on_hover_text("Pan: left −100 to right +100 · double-click for centre");
    let mut value = f64::from(pan.percent());
    let field = ui.add(
        DragValue::new(&mut value)
            .speed(1.0)
            .range(-100.0..=100.0)
            .update_while_editing(false)
            .custom_formatter(|v, _| pan_label(v))
            .custom_parser(parse_pan),
    );
    let from_slider = if slider.double_clicked() {
        Some(Pan::CENTRE)
    } else if slider.changed() {
        Some(Pan::new(p))
    } else {
        None
    };
    let from_field = field.changed().then(|| Pan::new(value.round() as i32));
    pick(
        pan,
        [(from_slider, slider), (from_field, field)],
        drag,
        project,
    )
}

/// The first control that asks for a value other than `current`; the drags
/// of the others end when they report their release.
fn pick<T: PartialEq>(
    current: T,
    asks: [(Option<T>, Response); 2],
    drag: &mut DragGesture,
    project: &mut Project,
) -> Option<(T, Response)> {
    let mut chosen = None;
    for (wanted, response) in asks {
        match wanted {
            Some(v) if v != current && chosen.is_none() => chosen = Some((v, response)),
            _ => drag.finish(project, &response),
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::TrackKind;

    use super::*;
    use crate::ui::drag::tests::{Frames, button, project_with_a_sound};

    #[test]
    fn a_click_solos_one_track_and_shift_click_adds_or_removes() {
        let id = |n| TrackId::from_raw(std::num::NonZeroU64::new(n).unwrap());
        let [a, b, c] = [id(1), id(2), id(3)];
        let none = BTreeSet::new();
        let only_a = solo_after_click(&none, a, false);
        assert_eq!(only_a, BTreeSet::from([a]));
        // Clicking the only soloed track un-solos it.
        assert!(solo_after_click(&only_a, a, false).is_empty());
        let ab = solo_after_click(&only_a, b, true);
        assert_eq!(ab, BTreeSet::from([a, b]));
        // A plain click on one of several: only that one.
        assert_eq!(solo_after_click(&ab, b, false), BTreeSet::from([b]));
        assert_eq!(solo_after_click(&ab, c, false), BTreeSet::from([c]));
        // Shift-click removes a soloed track.
        assert_eq!(solo_after_click(&ab, a, true), BTreeSet::from([b]));
    }

    #[test]
    fn the_fader_spans_silence_to_plus_12_db_on_a_tenth_db_grid() {
        assert_eq!(gain_from_fader(0.0), GainDb::SILENT);
        assert_eq!(gain_from_fader(DETENT / 2.0), GainDb::SILENT);
        assert_eq!(gain_from_fader(f64::NAN), GainDb::SILENT);
        assert_eq!(gain_from_fader(DETENT), GainDb::MIN);
        assert_eq!(gain_from_fader(UNITY), GainDb::ZERO);
        assert_eq!(gain_from_fader(1.0), TrackAudio::MAX_GAIN);
        assert_eq!(gain_from_fader(3.0), TrackAudio::MAX_GAIN);
        assert_eq!(fader_from_gain(GainDb::SILENT), 0.0);
        assert_eq!(fader_from_gain(GainDb::ZERO), UNITY);
        assert!((fader_from_gain(TrackAudio::MAX_GAIN) - 1.0).abs() < 1e-12);
        // Above the fader's top (a clip-range value) pins to the top.
        assert!((fader_from_gain(GainDb::MAX) - 1.0).abs() < 1e-12);
        let mut last = GainDb::SILENT;
        for i in 0..=10_000 {
            let g = gain_from_fader(f64::from(i) / 10_000.0);
            assert!(g >= last, "monotonic at {i}");
            assert!(g.is_silent() || g.hundredths() % 10 == 0, "0.1 dB grid");
            last = g;
        }
        for h in (-6_000..=1_200).step_by(10) {
            let g = GainDb::from_hundredths(h);
            assert_eq!(gain_from_fader(fader_from_gain(g)), g, "{h}");
        }
    }

    #[test]
    fn pan_reads_and_parses_left_centre_right() {
        assert_eq!(pan_label(0.0), "C");
        assert_eq!(pan_label(-30.0), "L30");
        assert_eq!(pan_label(100.0), "R100");
        assert_eq!(parse_pan("c"), Some(0.0));
        assert_eq!(parse_pan("L30"), Some(-30.0));
        assert_eq!(parse_pan(" r 45 "), Some(45.0));
        assert_eq!(parse_pan("−20"), Some(-20.0));
        assert_eq!(parse_pan("250"), Some(100.0));
        assert_eq!(parse_pan("L250"), Some(-100.0));
        assert_eq!(parse_pan("left"), None);
        assert_eq!(parse_pan(""), None);
        assert_eq!(track_readout(GainDb::from_hundredths(-600)), "−6.0 dB");
        assert_eq!(track_readout(GainDb::SILENT), "−∞ dB");
    }

    /// One frame of the mixer, settled like the app does.
    fn mixer_frame(
        frames: &mut Frames,
        project: &mut Project,
        state: &mut MixerState,
        events: Vec<egui::Event>,
    ) {
        frames.run(events, |ui| {
            assert_eq!(show(ui, project, state), None);
        });
        let down = frames.ctx.input(|i| i.pointer.any_down());
        state.settle(Some(project), down);
    }

    #[test]
    fn a_fader_drag_is_one_undo_entry() {
        let (mut project, _) = project_with_a_sound();
        let seq = project.doc().root();
        let audio = project.doc().sequence(seq).unwrap().audio_tracks()[0].id;
        let track_gain = |p: &Project| {
            p.doc()
                .sequence(seq)
                .unwrap()
                .find_track(audio)
                .unwrap()
                .1
                .mix
                .gain
        };
        let mut state = MixerState::default();
        let mut frames = Frames::new();
        mixer_frame(&mut frames, &mut project, &mut state, Vec::new());
        // Strips: main (idle), Audio 1, then master.
        assert_eq!(state.faders.len(), 3);
        let fader = state.faders[1];
        let entries = project.editor.history().len();
        let x = fader.center().x;
        let at = |u: f32| egui::Pos2::new(x, fader.bottom() - fader.height() * u);
        let start = at(0.75);
        let mut events = vec![egui::Event::PointerMoved(start), button(start, true)];
        for step in 1..=6 {
            events.push(egui::Event::PointerMoved(at(0.75 - 0.05 * step as f32)));
            mixer_frame(&mut frames, &mut project, &mut state, events);
            events = Vec::new();
        }
        let end = at(0.3);
        mixer_frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![egui::Event::PointerMoved(end), button(end, false)],
        );
        mixer_frame(&mut frames, &mut project, &mut state, Vec::new());

        let dragged = track_gain(&project);
        assert!(dragged < GainDb::from_hundredths(-1_000), "{dragged:?}");
        assert!(!project.editor.gesture_open(), "the drag ended");
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert_eq!(track_gain(&project), GainDb::ZERO, "one undo: back to 0 dB");
    }

    #[test]
    fn strips_list_sound_tracks_in_timeline_order_with_their_state() {
        let (mut project, clip) = project_with_a_sound();
        let seq = project.doc().root();
        let doc = project.doc().clone();
        let s = doc.sequence(seq).unwrap();
        let main = s.main_track().unwrap().id;
        let audio = s.audio_tracks()[0].id;
        // An empty main track is idle; the audio track plays.
        let list = strips(&doc, seq, &BTreeSet::new());
        assert_eq!(
            list.iter().map(|s| s.track).collect::<Vec<_>>(),
            [main, audio]
        );
        assert!(list[0].idle);
        assert!(!list[1].idle);
        assert_eq!(list[1].name, "Audio 1");
        assert_eq!(list[1].mix, TrackAudio::default());

        // A text track has no strip.
        let text = crate::ui::text::insert_text(
            &mut project,
            freecut_timeline::edit::InsertTarget::FirstFree,
            freecut_timeline::time::Tick::ZERO,
        )
        .unwrap();
        let doc = project.doc().clone();
        let (_, text_track) = doc.sequence(seq).unwrap().find_clip(text).unwrap();
        assert_eq!(text_track.kind, TrackKind::Text);
        assert_eq!(strips(&doc, seq, &BTreeSet::new()).len(), 2);

        // Track volume, pan, mute and solo show on the strip.
        for req in [
            EditRequest::SetTrackGain {
                seq,
                track: audio,
                gain: GainDb::from_hundredths(-600),
            },
            EditRequest::SetTrackPan {
                seq,
                track: audio,
                pan: Pan::new(-40),
            },
        ] {
            project.apply(&req, Coalesce::Never).unwrap();
        }
        project.set_solo(BTreeSet::from([audio]));
        let list = strips(project.doc(), seq, project.solo());
        let strip = &list[1];
        assert_eq!(strip.mix.gain, GainDb::from_hundredths(-600));
        assert_eq!(strip.mix.pan, Pan::new(-40));
        assert!(strip.soloed && !list[0].soloed);
        let _ = clip;
    }
}
