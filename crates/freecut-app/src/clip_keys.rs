//! Volume keyframes and the on-clip volume line, seen from the app
//! (AU-008, AU-009, TR-027): where keys sit in sequence time, what the
//! inspector's diamond and arrows do at the playhead, the line's dB ↔ pixel
//! mapping and hit tests, and the edits behind the gestures. The maths is
//! pure; the edit functions only build requests for [`Project::apply`].

use freecut_timeline::edit::{AudioEdit, EditRequest, GainKeyEdit};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{Animatable, Clip, ClipAudio, FadeCurve, GainDb, Segment};
use freecut_timeline::time::Tick;

use crate::clip_sound::{gain_db, gain_from_slider, slider_from_db, slider_from_gain};
use crate::session::Project;

// ------------------------------------------------------------ time map

/// The source time under sequence time `at`, when `at` lies on the clip
/// (its end included, so a playhead parked at the end still reaches it).
pub fn source_at(clip: &Clip, at: Tick) -> Option<Tick> {
    if at < clip.start() || at > clip.end() {
        return None;
    }
    clip.source_time_at(at)
}

/// The sequence time showing source time `source` of the clip.
pub fn seq_time_of(clip: &Clip, source: Tick) -> Option<Tick> {
    let start = clip.content.source()?.start();
    let offset = source.checked_sub(start).ok()?;
    clip.start().checked_add(offset).ok()
}

/// A key the timeline can show: its index in the key list, its source time
/// and where it sits in sequence time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleKey {
    pub index: usize,
    pub at: Tick,
    pub seq: Tick,
}

/// The keys inside the clip's trimmed range, in time order. Keys outside
/// it are kept by the model (TR-027) but not drawn or reachable.
pub fn visible_keys(clip: &Clip, audio: &ClipAudio) -> Vec<VisibleKey> {
    audio
        .gain
        .keys()
        .iter()
        .enumerate()
        .filter_map(|(index, k)| {
            let seq = seq_time_of(clip, k.at)?;
            (seq >= clip.start() && seq <= clip.end()).then_some(VisibleKey {
                index,
                at: k.at,
                seq,
            })
        })
        .collect()
}

/// Which way the inspector's keyframe arrows go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

/// Where the inspector's previous / next keyframe arrow moves the playhead:
/// the nearest visible key strictly before or after it, in sequence time.
pub fn adjacent_key(clip: &Clip, audio: &ClipAudio, playhead: Tick, step: Step) -> Option<Tick> {
    let keys = visible_keys(clip, audio);
    match step {
        Step::Previous => keys.iter().rev().find(|k| k.seq < playhead).map(|k| k.seq),
        Step::Next => keys.iter().find(|k| k.seq > playhead).map(|k| k.seq),
    }
}

/// The volume the inspector shows (TR-027): the value at the playhead,
/// or at the nearer clip edge when the playhead is off the clip.
pub fn volume_at_playhead(clip: &Clip, audio: &ClipAudio, playhead: Tick) -> GainDb {
    let at = playhead.clamp(clip.start(), clip.end().max(clip.start()));
    match clip.source_time_at(at) {
        Some(source) => audio.gain_at(source),
        None => *audio.gain.static_value(),
    }
}

/// What the keyframe diamond next to Volume does (AU-008, TR-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyButton {
    /// Adds a key at this source time with the volume there.
    Add(Tick),
    /// The playhead sits on this key: removes it.
    Remove(Tick),
    /// Unavailable, with the reason.
    Disabled(&'static str),
}

pub fn key_button(clip: &Clip, audio: &ClipAudio, playhead: Tick) -> KeyButton {
    match source_at(clip, playhead) {
        None => KeyButton::Disabled("Move the playhead onto this clip to add a volume keyframe"),
        Some(at) if audio.gain.key_at(at).is_some() => KeyButton::Remove(at),
        Some(at) => KeyButton::Add(at),
    }
}

// ----------------------------------------------------------- the line

/// The volume line's level at source time `at`, in dB (−∞ for silence):
/// the keyframe envelope as the engine plays it, so a segment touching −∞
/// ramps in linear amplitude instead of jumping at its midpoint (AU-008).
pub fn line_db_at(gain: &Animatable<GainDb>, at: Tick) -> f64 {
    match gain.segment_at(at) {
        Segment::Static(v) => gain_db(*v),
        Segment::Key(k) => gain_db(k.value),
        Segment::Ramp { from, to } => {
            let span = (to.at.raw() as f64) - (from.at.raw() as f64);
            let p = if span > 0.0 {
                ((at.raw() as f64) - (from.at.raw() as f64)) / span
            } else {
                0.0
            };
            // TR-031: the segment's own curve, as the engine plays it.
            let p = from.interp.ease(p);
            let (a, b) = (gain_db(from.value), gain_db(to.value));
            if a.is_finite() && b.is_finite() {
                a + (b - a) * p
            } else {
                let amp = |db: f64| {
                    if db.is_finite() {
                        10f64.powf(db / 20.0)
                    } else {
                        0.0
                    }
                };
                let v = (amp(a) + (amp(b) - amp(a)) * p).max(0.0);
                if v > 0.0 {
                    20.0 * v.log10()
                } else {
                    f64::NEG_INFINITY
                }
            }
        }
    }
}

/// A fade's gain `p` of the way through it (0 at the silent end), the
/// shapes the engine plays (AU-006).
fn fade_gain(curve: FadeCurve, p: f64) -> f64 {
    let curve = match curve {
        FadeCurve::Linear => freecut_audio::FadeCurve::Linear,
        FadeCurve::Smooth => freecut_audio::FadeCurve::Smooth,
        FadeCurve::Fast => freecut_audio::FadeCurve::Fast,
        FadeCurve::Slow => freecut_audio::FadeCurve::Slow,
    };
    curve.gain(p)
}

/// The fades' combined gain at sequence time `at` of the clip (1 outside
/// them), as the line draws them on top of the volume (AU-009).
pub fn fades_at(clip: &Clip, audio: &ClipAudio, at: Tick) -> f64 {
    let (fade_in, fade_out) = audio.effective_fades(clip.duration());
    let secs = |t: Tick| t.as_secs_f64();
    let mut g = 1.0;
    if fade_in > Tick::ZERO {
        let p = (secs(at) - secs(clip.start())) / secs(fade_in);
        if p < 1.0 {
            g *= fade_gain(audio.fade_in_curve, p);
        }
    }
    if fade_out > Tick::ZERO {
        let p = (secs(clip.end()) - secs(at)) / secs(fade_out);
        if p < 1.0 {
            g *= fade_gain(audio.fade_out_curve, p);
        }
    }
    g
}

/// The line's level at sequence time `at`: volume and keyframes, times the
/// fades. dB, −∞ for silence.
pub fn envelope_db_at(clip: &Clip, audio: &ClipAudio, at: Tick) -> f64 {
    let at = at.clamp(clip.start(), clip.end().max(clip.start()));
    let source = clip.source_time_at(at).unwrap_or(Tick::ZERO);
    let db = line_db_at(&audio.gain, source);
    let fade = fades_at(clip, audio, at);
    if fade >= 1.0 {
        db
    } else if fade <= 0.0 {
        f64::NEG_INFINITY
    } else {
        db + 20.0 * fade.log10()
    }
}

/// The y of `db` decibels in a band from `top` to `bottom` (AU-009): the
/// volume slider's scale (AU-005) stood upright, so 0 dB sits a quarter
/// down and −∞ on the bottom edge.
pub fn y_of_db(db: f64, top: f32, bottom: f32) -> f32 {
    bottom - (slider_from_db(db) as f32) * (bottom - top)
}

pub fn y_of_gain(gain: GainDb, top: f32, bottom: f32) -> f32 {
    y_of_db(gain_db(gain), top, bottom)
}

/// The gain at height `y` of the band, on the slider's 0.1 dB grid: the
/// inverse of [`y_of_gain`].
#[cfg(test)]
pub fn gain_from_y(y: f32, top: f32, bottom: f32) -> GainDb {
    let h = bottom - top;
    if h <= 0.0 {
        return GainDb::ZERO;
    }
    gain_from_slider(f64::from((bottom - y) / h))
}

/// `gain` raised by `du` of the band's height (negative lowers): how a drag
/// of the line or of a dot changes a level. Silence stays silent while
/// dragged down.
pub fn shifted(gain: GainDb, du: f64) -> GainDb {
    if gain.is_silent() && du <= 0.0 {
        return gain;
    }
    gain_from_slider(slider_from_gain(gain) + du)
}

/// The dot nearest to `p` within `radius` pixels, if any.
pub fn hit_dot(dots: &[(f32, f32)], p: (f32, f32), radius: f32) -> Option<usize> {
    dots.iter()
        .enumerate()
        .map(|(i, d)| (i, (d.0 - p.0).hypot(d.1 - p.1)))
        .filter(|(_, dist)| *dist <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// How close (in pixels) a press must be to the line to grab it: generous
/// on short bands, so the line stays easy to catch on small tracks.
pub fn line_tolerance(band_height: f32) -> f32 {
    (band_height * 0.2).clamp(3.0, 6.0)
}

/// What a drag of the line at source time `at` moves (AU-008): the static
/// volume without keys, else the segment under the pointer: the two keys
/// around it, or the first / last key before / after them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinePart {
    Static(GainDb),
    /// Keys by index, with their time and value when the drag began.
    Keys(Vec<(usize, Tick, GainDb)>),
}

pub fn line_part(gain: &Animatable<GainDb>, at: Tick) -> LinePart {
    let keys = gain.keys();
    if keys.is_empty() {
        return LinePart::Static(*gain.static_value());
    }
    let after = keys.partition_point(|k| k.at <= at);
    let picked: Vec<usize> = match after {
        0 => vec![0],
        n if n == keys.len() => vec![n - 1],
        n => vec![n - 1, n],
    };
    LinePart::Keys(
        picked
            .into_iter()
            .map(|i| (i, keys[i].at, keys[i].value))
            .collect(),
    )
}

// --------------------------------------------------------------- edits

fn audio_of(project: &Project, clip: ClipId) -> Result<ClipAudio, String> {
    project
        .doc()
        .clip(clip)
        .and_then(|c| c.audio.active().cloned())
        .ok_or_else(|| "This clip has no sound of its own".to_owned())
}

fn keys_edit(
    project: &mut Project,
    clip: ClipId,
    edit: GainKeyEdit,
    coalesce: Coalesce,
) -> Result<(), String> {
    let req = EditRequest::EditGainKeys {
        seq: project.doc().root(),
        clips: vec![clip],
        edit,
    };
    project.apply(&req, coalesce).map(|_| ())
}

/// The diamond (AU-008): adds a key at the playhead with the volume there,
/// or removes the key the playhead sits on.
pub fn toggle_key(project: &mut Project, clip: ClipId) -> Result<(), String> {
    let c = project
        .doc()
        .clip(clip)
        .cloned()
        .ok_or("That clip is gone")?;
    let audio = audio_of(project, clip)?;
    match key_button(&c, &audio, project.playhead) {
        KeyButton::Add(at) => keys_edit(
            project,
            clip,
            GainKeyEdit::Set { at, gain: None },
            Coalesce::Never,
        ),
        KeyButton::Remove(at) => keys_edit(
            project,
            clip,
            GainKeyEdit::Remove(vec![at]),
            Coalesce::Never,
        ),
        KeyButton::Disabled(reason) => Err(reason.to_owned()),
    }
}

/// A key at source time `at` with the line's value there (Alt/Option-click
/// on the line). Returns the key's time.
pub fn add_key(project: &mut Project, clip: ClipId, at: Tick) -> Result<Tick, String> {
    keys_edit(
        project,
        clip,
        GainKeyEdit::Set { at, gain: None },
        Coalesce::Never,
    )?;
    Ok(at)
}

/// The inspector's Volume (TR-027): without keys it sets the static
/// volume; with keys it sets or creates the key at the playhead, and is
/// refused while the playhead is off the clip.
pub fn set_volume(
    project: &mut Project,
    clip: ClipId,
    gain: GainDb,
    coalesce: Coalesce,
) -> Result<(), String> {
    let c = project
        .doc()
        .clip(clip)
        .cloned()
        .ok_or("That clip is gone")?;
    let audio = audio_of(project, clip)?;
    if !audio.gain.is_animated() {
        let req = EditRequest::SetClipAudio {
            seq: project.doc().root(),
            clips: vec![clip],
            edit: AudioEdit::Gain(gain),
        };
        return project.apply(&req, coalesce).map(|_| ());
    }
    let at = source_at(&c, project.playhead)
        .ok_or("Move the playhead onto this clip to change its volume keyframes")?;
    keys_edit(
        project,
        clip,
        GainKeyEdit::Set {
            at,
            gain: Some(gain),
        },
        coalesce,
    )
}

/// Moves key `index` toward source time `to` (clamped between its
/// neighbours by the model) and gives it `gain`: one step of a dot drag.
/// Returns where the key ended up.
pub fn move_key(
    project: &mut Project,
    clip: ClipId,
    index: usize,
    to: Tick,
    gain: GainDb,
    coalesce: Coalesce,
) -> Result<Tick, String> {
    let audio = audio_of(project, clip)?;
    let key = audio
        .gain
        .keys()
        .get(index)
        .cloned()
        .ok_or("That volume keyframe is gone")?;
    if key.at != to {
        keys_edit(
            project,
            clip,
            GainKeyEdit::Move { from: key.at, to },
            coalesce,
        )?;
    }
    let at = audio_of(project, clip)?
        .gain
        .keys()
        .get(index)
        .map_or(key.at, |k| k.at);
    if key.value != gain {
        keys_edit(
            project,
            clip,
            GainKeyEdit::Set {
                at,
                gain: Some(gain),
            },
            coalesce,
        )?;
    }
    Ok(at)
}

/// Removes the keys at `times` (Delete on selected dots).
pub fn remove_keys(project: &mut Project, clip: ClipId, times: &[Tick]) -> Result<(), String> {
    keys_edit(
        project,
        clip,
        GainKeyEdit::Remove(times.to_vec()),
        Coalesce::Never,
    )
}

/// One step of a line drag: the part grabbed, raised by `du` of the band's
/// height from where it was when the drag began.
pub fn shift_line(
    project: &mut Project,
    clip: ClipId,
    part: &LinePart,
    du: f64,
    coalesce: Coalesce,
) -> Result<(), String> {
    let audio = audio_of(project, clip)?;
    match part {
        LinePart::Static(from) => {
            let gain = shifted(*from, du);
            if *audio.gain.static_value() == gain || audio.gain.is_animated() {
                return Ok(());
            }
            let req = EditRequest::SetClipAudio {
                seq: project.doc().root(),
                clips: vec![clip],
                edit: AudioEdit::Gain(gain),
            };
            project.apply(&req, coalesce).map(|_| ())
        }
        LinePart::Keys(keys) => {
            for (_, at, from) in keys {
                let gain = shifted(*from, du);
                let now = audio.gain.key_at(*at).map(|k| k.value);
                if now.is_none() || now == Some(gain) {
                    continue;
                }
                keys_edit(
                    project,
                    clip,
                    GainKeyEdit::Set {
                        at: *at,
                        gain: Some(gain),
                    },
                    coalesce,
                )?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)] // test code

    use freecut_timeline::time::TICKS_PER_SECOND;

    use super::*;
    use crate::ui::drag::tests::project_with_a_sound;

    fn secs(n: f64) -> Tick {
        Tick::from_raw((n * TICKS_PER_SECOND as f64).round() as i64)
    }

    fn db(h: i32) -> GainDb {
        GainDb::from_hundredths(h)
    }

    fn keys(project: &Project, clip: ClipId) -> Vec<(Tick, GainDb)> {
        audio_of(project, clip)
            .unwrap()
            .gain
            .keys()
            .iter()
            .map(|k| (k.at, k.value))
            .collect()
    }

    #[test]
    fn db_and_y_map_both_ways_on_the_slider_scale() {
        let (top, bottom) = (100.0, 140.0);
        assert_eq!(y_of_gain(GainDb::SILENT, top, bottom), bottom);
        assert_eq!(y_of_gain(GainDb::MAX, top, bottom), top);
        assert_eq!(
            y_of_gain(GainDb::ZERO, top, bottom),
            110.0,
            "0 dB a quarter down"
        );
        assert!(y_of_gain(db(-600), top, bottom) > 110.0);
        assert_eq!(gain_from_y(110.0, top, bottom), GainDb::ZERO);
        assert_eq!(gain_from_y(bottom, top, bottom), GainDb::SILENT);
        assert_eq!(gain_from_y(top - 30.0, top, bottom), GainDb::MAX);
        for h in (-6_000..=2_000).step_by(100) {
            let y = y_of_gain(db(h), top, bottom);
            assert_eq!(gain_from_y(y, top, bottom), db(h), "{h}");
        }
        // Shifts follow the scale; silence stays put going down.
        assert_eq!(shifted(GainDb::ZERO, 0.0), GainDb::ZERO);
        assert!(shifted(GainDb::ZERO, -0.1) < GainDb::ZERO);
        assert_eq!(shifted(GainDb::SILENT, -0.2), GainDb::SILENT);
        assert_eq!(shifted(GainDb::SILENT, 0.75), GainDb::ZERO);
        assert_eq!(shifted(GainDb::ZERO, 5.0), GainDb::MAX);
    }

    #[test]
    fn dots_are_hit_within_their_radius_nearest_first() {
        let dots = [(10.0, 10.0), (16.0, 10.0), (40.0, 30.0)];
        assert_eq!(hit_dot(&dots, (11.0, 11.0), 6.0), Some(0));
        assert_eq!(hit_dot(&dots, (14.0, 10.0), 6.0), Some(1));
        assert_eq!(hit_dot(&dots, (40.0, 25.0), 6.0), Some(2));
        assert_eq!(hit_dot(&dots, (28.0, 20.0), 6.0), None);
        assert_eq!(hit_dot(&[], (0.0, 0.0), 6.0), None);
        assert_eq!(line_tolerance(4.0), 3.0);
        assert_eq!(line_tolerance(100.0), 6.0);
    }

    #[test]
    fn the_line_follows_keys_fades_and_ramps_out_of_silence_smoothly() {
        let gain = Animatable::new(db(-600))
            .with_key(secs(1.0), GainDb::ZERO)
            .with_key(secs(2.0), db(-1_200))
            .with_key(secs(3.0), GainDb::SILENT);
        assert_eq!(line_db_at(&gain, secs(0.0)), 0.0, "first key before it");
        assert_eq!(line_db_at(&gain, secs(1.5)), -6.0, "dB-linear between");
        assert_eq!(line_db_at(&gain, secs(9.0)), f64::NEG_INFINITY);
        // Toward −∞: linear amplitude, half way is half the level.
        let half = line_db_at(&gain, secs(2.5));
        let want = 20.0 * (0.5 * 10f64.powf(-12.0 / 20.0)).log10();
        assert!((half - want).abs() < 1e-9, "{half} vs {want}");
        assert_eq!(
            line_db_at(&Animatable::new(db(-300)), secs(5.0)),
            -3.0,
            "static"
        );
    }

    /// TR-031: an eased volume segment is drawn on its curve, as the
    /// engine plays it.
    #[test]
    fn the_line_follows_an_eased_segment() {
        use freecut_timeline::model::{EaseCurve, EaseDir, Interp};
        let quad_in = Interp::Ease {
            curve: EaseCurve::Quad,
            dir: EaseDir::In,
        };
        let gain = Animatable::new(GainDb::ZERO)
            .with_key(secs(0.0), GainDb::ZERO)
            .with_key(secs(1.0), db(-2_000));
        let gain = gain.with_interp(secs(0.0), quad_in).unwrap();
        assert!((line_db_at(&gain, secs(0.5)) - -5.0).abs() < 1e-9);
        assert_eq!(line_db_at(&gain, secs(1.0)), -20.0);
    }

    #[test]
    fn line_parts_pick_the_segment_under_the_pointer() {
        assert_eq!(
            line_part(&Animatable::new(db(-300)), secs(1.0)),
            LinePart::Static(db(-300))
        );
        let gain = Animatable::new(GainDb::ZERO)
            .with_key(secs(1.0), db(-100))
            .with_key(secs(2.0), db(-200));
        assert_eq!(
            line_part(&gain, secs(0.5)),
            LinePart::Keys(vec![(0, secs(1.0), db(-100))])
        );
        assert_eq!(
            line_part(&gain, secs(1.5)),
            LinePart::Keys(vec![(0, secs(1.0), db(-100)), (1, secs(2.0), db(-200))])
        );
        assert_eq!(
            line_part(&gain, secs(3.0)),
            LinePart::Keys(vec![(1, secs(2.0), db(-200))])
        );
    }

    #[test]
    fn the_inspector_reads_and_steps_keys_at_the_playhead() {
        let (mut project, clip) = project_with_a_sound();
        project.playhead = secs(1.0);
        let c = project.doc().clip(clip).cloned().unwrap();
        let audio = audio_of(&project, clip).unwrap();
        assert_eq!(key_button(&c, &audio, secs(1.0)), KeyButton::Add(secs(1.0)));
        assert!(matches!(
            key_button(&c, &audio, secs(5.0)),
            KeyButton::Disabled(_)
        ));
        assert_eq!(volume_at_playhead(&c, &audio, secs(1.0)), GainDb::ZERO);

        // The diamond captures the static volume.
        toggle_key(&mut project, clip).unwrap();
        assert_eq!(keys(&project, clip), [(secs(1.0), GainDb::ZERO)]);
        // Editing Volume elsewhere adds a key there (TR-027).
        project.playhead = secs(3.0);
        set_volume(&mut project, clip, db(-1_200), Coalesce::Never).unwrap();
        assert_eq!(
            keys(&project, clip),
            [(secs(1.0), GainDb::ZERO), (secs(3.0), db(-1_200))]
        );
        let c = project.doc().clip(clip).cloned().unwrap();
        let audio = audio_of(&project, clip).unwrap();
        assert_eq!(volume_at_playhead(&c, &audio, secs(2.0)), db(-600));
        assert_eq!(volume_at_playhead(&c, &audio, secs(9.0)), db(-1_200));
        assert_eq!(
            key_button(&c, &audio, secs(3.0)),
            KeyButton::Remove(secs(3.0))
        );
        assert_eq!(
            adjacent_key(&c, &audio, secs(2.0), Step::Previous),
            Some(secs(1.0))
        );
        assert_eq!(
            adjacent_key(&c, &audio, secs(2.0), Step::Next),
            Some(secs(3.0))
        );
        assert_eq!(adjacent_key(&c, &audio, secs(3.0), Step::Next), None);
        assert_eq!(adjacent_key(&c, &audio, secs(1.0), Step::Previous), None);

        // Off the clip the keyed volume can't be edited.
        project.playhead = secs(9.0);
        assert!(set_volume(&mut project, clip, db(-300), Coalesce::Never).is_err());
        // The diamond on a key removes it.
        project.playhead = secs(3.0);
        toggle_key(&mut project, clip).unwrap();
        assert_eq!(keys(&project, clip), [(secs(1.0), GainDb::ZERO)]);
    }

    #[test]
    fn keys_are_added_moved_and_deleted_through_the_app_paths_with_undo() {
        let (mut project, clip) = project_with_a_sound();
        let entries = project.editor.history().len();
        add_key(&mut project, clip, secs(1.0)).unwrap();
        add_key(&mut project, clip, secs(2.0)).unwrap();
        add_key(&mut project, clip, secs(3.0)).unwrap();
        assert_eq!(project.editor.history().len(), entries + 3);

        // A dot drag: time and level, clamped by its neighbours, one entry.
        let g = project.begin_gesture();
        for step in 1..=4 {
            let to = secs(2.0 + 0.5 * f64::from(step));
            move_key(
                &mut project,
                clip,
                1,
                to,
                db(-100 * step),
                Coalesce::Gesture(g),
            )
            .unwrap();
        }
        project.end_gesture(g);
        assert_eq!(project.editor.history().len(), entries + 4);
        let k = keys(&project, clip);
        assert_eq!(k[1].1, db(-400));
        assert!(k[0].0 < k[1].0 && k[1].0 < k[2].0, "never reorders: {k:?}");
        assert_eq!(k[1].0, Tick::from_raw(secs(3.0).raw() - 1));

        // A segment drag between keys 0 and 1 moves both, one entry.
        let gain = audio_of(&project, clip).unwrap().gain;
        let part = line_part(&gain, secs(1.5));
        let g = project.begin_gesture();
        for du in [-0.05, -0.1, -0.2] {
            shift_line(&mut project, clip, &part, du, Coalesce::Gesture(g)).unwrap();
        }
        project.end_gesture(g);
        assert_eq!(project.editor.history().len(), entries + 5);
        let k = keys(&project, clip);
        assert_eq!(k[0].1, shifted(GainDb::ZERO, -0.2));
        assert_eq!(k[1].1, shifted(db(-400), -0.2));
        assert_eq!(k[2].1, GainDb::ZERO, "the next key stays");

        // Delete the first two dots; the clip stays.
        let times = [k[0].0, k[1].0];
        remove_keys(&mut project, clip, &times).unwrap();
        assert_eq!(keys(&project, clip), [(secs(3.0), GainDb::ZERO)]);
        assert!(project.doc().clip(clip).is_some());

        // Undo walks back one gesture at a time.
        project.undo().unwrap();
        project.undo().unwrap();
        assert_eq!(keys(&project, clip)[1].1, db(-400));
        project.undo().unwrap();
        assert_eq!(keys(&project, clip)[1], (secs(2.0), GainDb::ZERO));
    }

    #[test]
    fn a_static_line_drag_sets_the_volume_as_one_entry() {
        let (mut project, clip) = project_with_a_sound();
        let entries = project.editor.history().len();
        let part = line_part(&audio_of(&project, clip).unwrap().gain, secs(1.0));
        let g = project.begin_gesture();
        for du in [-0.1, -0.2, -0.3] {
            shift_line(&mut project, clip, &part, du, Coalesce::Gesture(g)).unwrap();
        }
        project.end_gesture(g);
        assert_eq!(project.editor.history().len(), entries + 1);
        let gain = *audio_of(&project, clip).unwrap().gain.static_value();
        assert_eq!(gain, shifted(GainDb::ZERO, -0.3));
        project.undo().unwrap();
        assert_eq!(
            *audio_of(&project, clip).unwrap().gain.static_value(),
            GainDb::ZERO
        );
    }

    #[test]
    fn fades_bend_the_line_down_to_silence_at_the_edges() {
        let (mut project, clip) = project_with_a_sound();
        let req = EditRequest::SetClipAudio {
            seq: project.doc().root(),
            clips: vec![clip],
            edit: AudioEdit::FadeIn(secs(1.0)),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        let c = project.doc().clip(clip).cloned().unwrap();
        let audio = audio_of(&project, clip).unwrap();
        assert_eq!(envelope_db_at(&c, &audio, secs(0.0)), f64::NEG_INFINITY);
        let mid = envelope_db_at(&c, &audio, secs(0.5));
        assert!((mid - 20.0 * 0.5f64.log10()).abs() < 1e-9, "{mid}");
        assert_eq!(envelope_db_at(&c, &audio, secs(2.0)), 0.0);
    }
}
