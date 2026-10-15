//! What the clip sound commands can do for a selection, and the unit
//! conversions behind the inspector's sound controls (AU-005, AU-006,
//! AU-007, TL-030). Pure functions of the document; the panels only draw
//! their results and send requests.

use freecut_timeline::edit::linked_audio;
use freecut_timeline::ids::{ClipId, SeqId};
use freecut_timeline::model::{ClipContent, Document, EmbeddedAudio, GainDb, MediaKind, StreamSel};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

/// A clip command from a menu or the inspector, carried out by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipCommand {
    Split,
    Delete,
    /// Mute (true) or unmute every selected clip that plays its own sound.
    SetMuted(bool),
    Detach,
    Restore,
    /// Play this stream of the file (AU-038); `None` follows the file's
    /// default.
    SetStream(Option<StreamSel>),
    /// Put the file's stream at this position (0-based) on its own linked
    /// audio clip (AU-038).
    AddStream(usize),
    /// Move out-of-sync linked sound back in sync (TL-030).
    Resync,
}

/// The one sound command a selection offers: detach when any clip can
/// detach, else restore when any clip is detached (TL-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCommand {
    /// Detach, or why not.
    Detach(Result<(), &'static str>),
    Restore,
}

/// The mute toggle for `clips` (AU-007): `Ok(true)` mutes, `Ok(false)`
/// unmutes, `Err` says why it's unavailable. A mixed selection mutes: if
/// any clip with sound is unmuted, every one becomes muted.
pub fn mute_command(doc: &Document, clips: &[ClipId]) -> Result<bool, &'static str> {
    let audios: Vec<_> = clips
        .iter()
        .filter_map(|id| doc.clip(*id))
        .map(|c| &c.audio)
        .collect();
    let active: Vec<_> = audios.iter().filter_map(|a| a.active()).collect();
    if active.is_empty() {
        return Err(
            if audios
                .iter()
                .any(|a| matches!(a, EmbeddedAudio::Detached(_)))
            {
                "The sound of this clip is detached; mute its audio clip instead"
            } else if clips.len() > 1 {
                "None of these clips has sound"
            } else {
                "This clip has no sound"
            },
        );
    }
    Ok(active.iter().any(|a| !a.muted))
}

/// Whether a clip can detach its sound: a video clip that plays its own
/// sound from media that has an audio stream.
fn can_detach(doc: &Document, id: ClipId) -> bool {
    doc.clip(id).is_some_and(|c| {
        matches!(
            c.content,
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            }
        ) && c.audio.active().is_some()
            && c.content
                .asset()
                .and_then(|a| doc.library().get(a))
                .is_some_and(|m| m.info.has_audio())
    })
}

/// Detach or restore for `clips`, with the reason when neither applies.
pub fn sound_command(doc: &Document, clips: &[ClipId]) -> SoundCommand {
    if clips.iter().any(|id| can_detach(doc, *id)) {
        return SoundCommand::Detach(Ok(()));
    }
    let found: Vec<_> = clips.iter().filter_map(|id| doc.clip(*id)).collect();
    if found.iter().any(|c| c.audio.detached().is_some()) {
        return SoundCommand::Restore;
    }
    let all_audio = !found.is_empty()
        && found.iter().all(|c| {
            matches!(
                c.content,
                ClipContent::Media {
                    kind: MediaKind::Audio,
                    ..
                }
            )
        });
    SoundCommand::Detach(Err(if all_audio {
        "Audio clips are already separate from any video"
    } else if clips.len() > 1 {
        "None of these clips has sound to detach"
    } else {
        "This clip has no sound"
    }))
}

/// The audio clips a restore of `clips` would remove, and whether to ask
/// first (TL-030): when a video has more than one linked clip, or a linked
/// clip no longer lines up with its video (it was moved, trimmed or
/// split). Volume, mute and fade edits move back with the restore, so they
/// alone don't need a question.
pub fn restore_removals(doc: &Document, seq: SeqId, clips: &[ClipId]) -> (Vec<ClipId>, bool) {
    let mut removed: Vec<ClipId> = Vec::new();
    let mut ask = false;
    for id in clips {
        let Some(video) = doc.clip(*id) else { continue };
        let linked = linked_audio(doc, seq, *id);
        if linked.len() > 1 {
            ask = true;
        }
        for audio_id in linked {
            if let Some(audio) = doc.clip(audio_id)
                && (audio.range != video.range || audio.content.source() != video.content.source())
            {
                ask = true;
            }
            if !removed.contains(&audio_id) {
                removed.push(audio_id);
            }
        }
    }
    (removed, ask)
}

// ------------------------------------------------------------- volume

/// Slider positions (0..=1) of the volume slider's landmarks: below
/// `DETENT` is −∞, `DETENT` is −60 dB, `UNITY` is 0 dB, 1 is +20 dB.
pub const DETENT: f64 = 0.02;
pub const UNITY: f64 = 0.75;

/// The gain at slider position `u` (AU-005), on a 0.1 dB grid. Below 0 dB
/// the scale is quadratic, so the range near 0 dB gets the most travel;
/// above, it is linear.
pub fn gain_from_slider(u: f64) -> GainDb {
    if u.is_nan() || u < DETENT {
        return GainDb::SILENT;
    }
    let db = if u >= UNITY {
        (u.min(1.0) - UNITY) / (1.0 - UNITY) * 20.0
    } else {
        let s = (UNITY - u) / (UNITY - DETENT);
        -60.0 * s * s
    };
    gain_from_db_step(db, 0.1)
}

/// The slider position showing `gain`; the inverse of
/// [`gain_from_slider`] on its grid.
pub fn slider_from_gain(gain: GainDb) -> f64 {
    slider_from_db(gain_db(gain))
}

/// The slider position of `db` decibels (−∞ for silence), for values off
/// the gain grid too (the volume line, AU-009). Clamped to 0..=1.
pub fn slider_from_db(db: f64) -> f64 {
    if db.is_nan() || db == f64::NEG_INFINITY || db < -60.0 {
        return 0.0;
    }
    let db = db.min(20.0);
    if db >= 0.0 {
        UNITY + db / 20.0 * (1.0 - UNITY)
    } else {
        UNITY - (-db / 60.0).sqrt() * (UNITY - DETENT)
    }
}

/// `db` rounded to a multiple of `step` dB and clamped into the gain range;
/// −∞ is silence.
fn gain_from_db_step(db: f64, step: f64) -> GainDb {
    if db == f64::NEG_INFINITY {
        return GainDb::SILENT;
    }
    if db.is_nan() {
        return GainDb::ZERO;
    }
    let steps = (db / step).round() * step;
    let hundredths = (steps * 100.0).round().clamp(-1e6, 1e6) as i32;
    GainDb::from_hundredths(hundredths)
}

/// The gain for a value typed in decibels, clamped into range (AU-005);
/// −∞ means silence.
pub fn gain_from_db(db: f64) -> GainDb {
    gain_from_db_step(db, 0.01)
}

/// The gain in decibels for display; −∞ for silence.
pub fn gain_db(gain: GainDb) -> f64 {
    if gain.is_silent() {
        f64::NEG_INFINITY
    } else {
        f64::from(gain.hundredths()) / 100.0
    }
}

/// `gain` moved by `hundredths` of a decibel (AU-005's arrow steps, AU-036),
/// within the gain range. Stepping up from −∞ starts at −60 dB; stepping
/// down stops there.
pub fn step_gain(gain: GainDb, hundredths: i32) -> GainDb {
    if hundredths == 0 {
        gain
    } else if gain.is_silent() {
        if hundredths > 0 { GainDb::MIN } else { gain }
    } else {
        GainDb::from_hundredths(gain.hundredths().saturating_add(hundredths))
    }
}

/// "−6.0 dB", "+3.5 dB", "−∞ dB".
pub fn format_db(db: f64) -> String {
    if db == f64::NEG_INFINITY {
        "−∞ dB".to_owned()
    } else if db > 0.0 {
        format!("+{db:.1} dB")
    } else if db < 0.0 {
        format!("−{:.1} dB", -db)
    } else {
        "0.0 dB".to_owned()
    }
}

/// The volume as a percentage of the original level, for the hover hint.
pub fn gain_percent(gain: GainDb) -> f64 {
    let db = gain_db(gain);
    if db == f64::NEG_INFINITY {
        0.0
    } else {
        10f64.powf(db / 20.0) * 100.0
    }
}

/// Parses what the user typed into the volume field (AU-005): decibels
/// ("-6", "+3.5", "−6 dB"), a percentage of the original level ("50%"), or
/// "-inf". Returns decibels; −∞ for silence.
pub fn parse_db(text: &str) -> Option<f64> {
    let t = text.trim().replace('−', "-").to_lowercase();
    let t = t.trim_end_matches("db").trim();
    if matches!(t, "-inf" | "-∞" | "inf" | "∞" | "silent") {
        return Some(f64::NEG_INFINITY);
    }
    if let Some(pct) = t.strip_suffix('%') {
        let p: f64 = pct.trim().parse().ok()?;
        if p.is_nan() || p < 0.0 || !p.is_finite() {
            return None;
        }
        return Some(if p == 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * (p / 100.0).log10()
        });
    }
    let v: f64 = t.trim_start_matches('+').parse().ok()?;
    v.is_finite().then_some(v)
}

// -------------------------------------------------------------- fades

/// Ticks in a tenth of a second (the fade fields' step, AU-006).
const TENTH: i64 = TICKS_PER_SECOND / 10;

/// A fade length in seconds typed or dragged in the inspector, as ticks:
/// rounded to 0.1 s, never negative, and at most `max` (so fade-in plus
/// fade-out stays within the clip while editing, AU-006).
pub fn fade_from_secs(secs: f64, max: Tick) -> Tick {
    let tenths = if secs.is_finite() {
        (secs * 10.0).round().clamp(0.0, 1e12) as i64
    } else {
        0
    };
    let ticks = Tick::from_raw(tenths.saturating_mul(TENTH));
    ticks.min(max.max(Tick::ZERO))
}

/// The longest a fade may be while the other edge's fade is `other`.
pub fn fade_limit(duration: Tick, other: Tick) -> Tick {
    let duration = duration.max(Tick::ZERO);
    let other = other.max(Tick::ZERO).min(duration);
    // Both non-negative, other <= duration: no overflow.
    Tick::from_raw(duration.raw() - other.raw())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)] // test code

    use freecut_timeline::edit::{AudioEdit, EditRequest, InsertReq, InsertTarget, NewClip};
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::{
        AudioStreamInfo, MediaInfo, ProjectSettings, StoredPath, VideoStreamInfo,
    };
    use freecut_timeline::time::TimeRange;

    use super::*;
    use crate::session::Project;

    #[test]
    fn slider_landmarks() {
        assert_eq!(gain_from_slider(0.0), GainDb::SILENT);
        assert_eq!(gain_from_slider(DETENT / 2.0), GainDb::SILENT);
        assert_eq!(gain_from_slider(f64::NAN), GainDb::SILENT);
        assert_eq!(gain_from_slider(DETENT), GainDb::MIN);
        assert_eq!(gain_from_slider(UNITY), GainDb::ZERO);
        assert_eq!(gain_from_slider(1.0), GainDb::MAX);
        assert_eq!(gain_from_slider(2.0), GainDb::MAX);
        assert_eq!(slider_from_gain(GainDb::SILENT), 0.0);
        assert_eq!(slider_from_gain(GainDb::ZERO), UNITY);
        assert!((slider_from_gain(GainDb::MIN) - DETENT).abs() < 1e-12);
        assert!((slider_from_gain(GainDb::MAX) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn slider_is_monotonic_on_a_tenth_db_grid_and_round_trips() {
        let mut last = GainDb::SILENT;
        for i in 0..=10_000 {
            let g = gain_from_slider(f64::from(i) / 10_000.0);
            assert!(g >= last, "monotonic at {i}");
            assert!(g.is_silent() || g.hundredths() % 10 == 0, "0.1 dB grid");
            last = g;
        }
        // Every 0.1 dB value maps back to itself.
        for h in (-6_000..=2_000).step_by(10) {
            let g = GainDb::from_hundredths(h);
            assert_eq!(gain_from_slider(slider_from_gain(g)), g, "{h}");
        }
    }

    #[test]
    fn the_range_near_unity_gets_more_travel() {
        let near = slider_from_gain(GainDb::ZERO) - slider_from_gain(GainDb::from_hundredths(-600));
        let far = slider_from_gain(GainDb::from_hundredths(-5_400))
            - slider_from_gain(GainDb::from_hundredths(-6_000));
        assert!(near > 2.0 * far, "{near} vs {far}");
    }

    #[test]
    fn typed_volume_accepts_db_percent_and_silence() {
        assert_eq!(parse_db("-6"), Some(-6.0));
        assert_eq!(parse_db("+3.5"), Some(3.5));
        assert_eq!(parse_db(" −6.0 dB "), Some(-6.0));
        assert_eq!(parse_db("-inf"), Some(f64::NEG_INFINITY));
        assert_eq!(parse_db("0%"), Some(f64::NEG_INFINITY));
        assert_eq!(gain_from_db(parse_db("50%").unwrap()).hundredths(), -602);
        assert_eq!(gain_from_db(parse_db("200%").unwrap()).hundredths(), 602);
        assert_eq!(parse_db("loud"), None);
        assert_eq!(parse_db("-5%"), None);
        // Out of range is clamped.
        assert_eq!(gain_from_db(-90.0), GainDb::MIN);
        assert_eq!(gain_from_db(35.0), GainDb::MAX);
        assert_eq!(gain_from_db(f64::NEG_INFINITY), GainDb::SILENT);
        assert_eq!(format_db(gain_db(GainDb::SILENT)), "−∞ dB");
        assert_eq!(format_db(-6.02), "−6.0 dB");
        assert_eq!(format_db(3.5), "+3.5 dB");
        assert!((gain_percent(GainDb::from_hundredths(-602)) - 50.0).abs() < 0.1);
    }

    #[test]
    fn fades_round_to_tenths_and_respect_the_limit() {
        let s = |n: i64| Tick::from_secs(n).unwrap();
        assert_eq!(fade_from_secs(0.04, s(10)), Tick::ZERO);
        assert_eq!(fade_from_secs(1.26, s(10)), Tick::from_raw(13 * TENTH));
        assert_eq!(fade_from_secs(-3.0, s(10)), Tick::ZERO);
        assert_eq!(fade_from_secs(f64::NAN, s(10)), Tick::ZERO);
        assert_eq!(fade_from_secs(12.0, s(10)), s(10));
        assert_eq!(fade_limit(s(4), s(1)), s(3));
        assert_eq!(fade_limit(s(4), s(9)), Tick::ZERO);
        assert_eq!(fade_limit(s(4), Tick::from_raw(-5)), s(4));
    }

    /// A project with a video clip (with sound), a silent video clip and an
    /// audio clip, all on their own tracks.
    fn project() -> (Project, [ClipId; 3]) {
        let mut p = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let secs = Tick::from_secs(4).unwrap();
        let video = VideoStreamInfo {
            index: 0,
            codec: "h264".into(),
            width: 640,
            height: 360,
            sar: (1, 1),
            rotation: freecut_timeline::model::Rotation::None,
            mirrored: false,
            rate: None,
            variable_rate: false,
            has_alpha: false,
            bit_depth: 8,
            start: Tick::ZERO,
            duration: secs,
        };
        let audio = AudioStreamInfo {
            index: 1,
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: secs,
            duration_exact: true,
            default: true,
            language: None,
            title: None,
            layout: None,
        };
        let info = |v: bool, a: bool| MediaInfo {
            container: "mp4".into(),
            default_video: v.then_some(0),
            duration: secs,
            video: if v { vec![video.clone()] } else { vec![] },
            audio: if a { vec![audio.clone()] } else { vec![] },
            still: None,
        };
        let media = |name: &str, info: MediaInfo| freecut_timeline::edit::NewMedia {
            path: StoredPath {
                absolute: format!("/m/{name}"),
                relative: None,
            },
            info,
            fingerprint: None,
            display_name: None,
        };
        let req = EditRequest::ImportMedia(vec![
            media("a.mp4", info(true, true)),
            media("b.mp4", info(true, false)),
            media("c.wav", info(false, true)),
        ]);
        let plan = p.editor.plan(&p.ctx(), &req).unwrap();
        let ids = plan.imported().to_vec();
        p.editor
            .commit(plan, freecut_timeline::CommitMeta::default())
            .unwrap();
        let source = TimeRange::new(Tick::ZERO, secs).unwrap();
        let mut clips = Vec::new();
        for (id, kind) in ids
            .iter()
            .zip([MediaKind::Video, MediaKind::Video, MediaKind::Audio])
        {
            let req = EditRequest::Insert(InsertReq {
                seq: p.doc().root(),
                target: InsertTarget::NewTrack,
                at: Tick::ZERO,
                clips: vec![NewClip::media(*id, kind, source)],
            });
            p.apply(&req, Coalesce::Never).unwrap();
            clips.push(p.selection[0]);
        }
        (p, [clips[0], clips[1], clips[2]])
    }

    #[test]
    fn mute_toggles_by_the_mixed_selection_rule() {
        let (mut p, [video, silent, audio]) = project();
        let doc = p.doc().clone();
        assert_eq!(mute_command(&doc, &[video, audio]), Ok(true));
        assert_eq!(mute_command(&doc, &[silent]), Err("This clip has no sound"));
        let req = EditRequest::SetClipAudio {
            seq: doc.root(),
            clips: vec![video],
            edit: AudioEdit::Muted(true),
        };
        p.apply(&req, Coalesce::Never).unwrap();
        let doc = p.doc().clone();
        // One unmuted clip in the selection: mute them all.
        assert_eq!(mute_command(&doc, &[video, audio]), Ok(true));
        assert_eq!(mute_command(&doc, &[video, silent]), Ok(false));
    }

    #[test]
    fn detach_restore_availability_and_confirmation() {
        let (mut p, [video, silent, audio]) = project();
        let doc = p.doc().clone();
        assert_eq!(
            sound_command(&doc, &[video, silent]),
            SoundCommand::Detach(Ok(()))
        );
        assert_eq!(
            sound_command(&doc, &[silent]),
            SoundCommand::Detach(Err("This clip has no sound"))
        );
        assert!(matches!(
            sound_command(&doc, &[audio]),
            SoundCommand::Detach(Err(_))
        ));

        let seq = doc.root();
        p.apply(
            &EditRequest::DetachAudio {
                seq,
                clips: vec![video],
            },
            Coalesce::Never,
        )
        .unwrap();
        let detached_to = p.selection.clone();
        let doc = p.doc().clone();
        assert_eq!(sound_command(&doc, &[video]), SoundCommand::Restore);
        assert_eq!(
            mute_command(&doc, &[video]),
            Err("The sound of this clip is detached; mute its audio clip instead")
        );
        // One linked clip, still lined up with its video: no question.
        assert_eq!(
            restore_removals(&doc, seq, &[video]),
            (detached_to.clone(), false)
        );

        // Moving the audio clip makes the restore ask first.
        p.apply(
            &EditRequest::Move(freecut_timeline::edit::MoveReq {
                seq,
                clips: detached_to.clone(),
                delta: Tick::from_secs(1).unwrap(),
                track_shift: 0,
                followers: Vec::new(),
            }),
            Coalesce::Never,
        )
        .unwrap();
        let doc = p.doc().clone();
        assert_eq!(restore_removals(&doc, seq, &[video]), (detached_to, true));
        // Not a detached video: nothing to remove.
        assert_eq!(restore_removals(&doc, seq, &[audio]), (vec![], false));
    }

    #[test]
    fn volume_steps_stay_in_range_and_leave_silence_at_minus_60_db() {
        let db = GainDb::from_hundredths;
        assert_eq!(step_gain(db(-602), 100), db(-502));
        assert_eq!(step_gain(db(-602), -10), db(-612));
        assert_eq!(step_gain(db(1_950), 100), GainDb::MAX);
        assert_eq!(step_gain(db(-5_950), -100), GainDb::MIN);
        assert_eq!(step_gain(GainDb::SILENT, 10), GainDb::MIN);
        assert_eq!(step_gain(GainDb::SILENT, -100), GainDb::SILENT);
        assert_eq!(step_gain(db(300), 0), db(300));
    }
}
