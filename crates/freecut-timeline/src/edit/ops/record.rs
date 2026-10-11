//! Placing a recorded voiceover take (AU-027).

use core::time::Duration;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::delete::{DeleteReq, delete_in};
use super::library::NewMedia;
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, SeqId, TrackId};
use crate::model::{
    Clip, ClipContent, GeneratedMediaKind, MediaItem, MediaKind, MediaOrigin, MediaSource,
    Sequence, TrackKind,
};
use crate::time::{Rounding, SampleRate, TICKS_PER_SECOND, Tick, TimeRange, div_round};

/// Where a take goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RecordTarget {
    /// The first unlocked audio track free over the take, top down; a new
    /// audio track at the bottom when none is.
    #[default]
    FirstFree,
    /// Always a new audio track at the top of the audio band.
    NewTrack,
}

/// Where a take starts on the timeline: the start position, earlier by the
/// device's input latency and later by the user's offset preference, on
/// the take's sample grid (nearest sample, halves to the earlier one).
/// Pure; saturates instead of overflowing. It can be negative: placement
/// then drops the part before 0.
pub fn recorded_start(
    start: Tick,
    input_latency: Duration,
    user_offset_ms: i32,
    rate: SampleRate,
) -> Tick {
    let latency = div_round(
        i128::try_from(input_latency.as_nanos())
            .unwrap_or(i128::MAX / 2)
            .saturating_mul(i128::from(TICKS_PER_SECOND)),
        1_000_000_000,
        Rounding::NearestTiesDown,
    );
    let offset = i128::from(user_offset_ms) * i128::from(TICKS_PER_SECOND) / 1_000;
    let exact = i128::from(start.raw())
        .saturating_sub(latency)
        .saturating_add(offset);
    let sample = i128::from(rate.sample_ticks().raw());
    let snapped = div_round(exact, sample, Rounding::NearestTiesDown).saturating_mul(sample);
    let clamped = snapped.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    Tick::from_raw(i64::try_from(clamped).unwrap_or(0))
}

/// The first unlocked audio track (top down) with nothing over `range`.
pub fn first_free_audio_track(seq: &Sequence, range: TimeRange) -> Option<TrackId> {
    seq.audio_tracks()
        .iter()
        .find(|t| {
            t.kind == TrackKind::Audio
                && !t.is_locked()
                && policy::fits(t, &[range], &BTreeSet::new())
        })
        .map(|t| t.id)
}

/// Adds the recorded file as a voiceover media item and places it as one
/// entry: the item, the clip (selected), any new track and, for *Redo last
/// take*, the removal of `replace`.
///
/// The clip starts at `at` on the frame grid; its in-point is `handle_in`
/// (the pre-roll) plus the rounding, so the audio stays where `at` put it.
/// The part before 0 is dropped.
pub(crate) fn insert_recorded_clip(
    mut tx: Tx<'_>,
    seq: SeqId,
    media: &NewMedia,
    at: Tick,
    handle_in: Tick,
    target: RecordTarget,
    replace: Option<ClipId>,
) -> Result<Plan, EditError> {
    tx.seq(seq)?;
    let rate = tx.rate();
    let frame = rate.frame_ticks();
    let mut start = rate.quantize(at)?;
    let mut source_in = handle_in.checked_add(start.checked_sub(at)?)?;
    if start.is_negative() {
        source_in = source_in.checked_sub(start)?;
        start = Tick::ZERO;
    }
    if source_in.is_negative() {
        let bump = rate.round_to_grid(source_in.checked_neg()?, Rounding::Ceil)?;
        start = start.checked_add(bump)?;
        source_in = source_in.checked_add(bump)?;
    }
    let available = media.info.duration.checked_sub(source_in)?;
    let duration = rate.round_to_grid(available, Rounding::Floor)?;
    if duration < frame {
        return Err(EditError::TooShort);
    }

    let media_id = tx.ids.media()?;
    if !media.info.has_audio() {
        return Err(EditError::NoSound(media_id));
    }
    tx.push(Change::InsertMedia {
        item: Arc::new(MediaItem {
            id: media_id,
            source: MediaSource::File,
            origin: MediaOrigin::Generated {
                kind: GeneratedMediaKind::Voiceover,
            },
            path: media.path.clone(),
            info: media.info.clone(),
            fingerprint: media.fingerprint,
            display_name: media.display_name.clone(),
        }),
    })?;
    tx.created_media(media_id);

    if let Some(old) = replace {
        let loc = tx.locate(old)?;
        if loc.seq != seq {
            return Err(EditError::StaleClip(old));
        }
        delete_in(
            &mut tx,
            &DeleteReq {
                seq,
                clips: vec![old],
                ripple: false,
            },
        )?;
    }

    let range = TimeRange::new(start, duration)?;
    let clip_id = tx.ids.clip()?;
    let clip = Clip::new(
        clip_id,
        range,
        ClipContent::Media {
            asset: media_id,
            kind: MediaKind::Audio,
            source: TimeRange::new(source_in, duration)?,
            stream: None,
        },
    );
    let track = match target {
        RecordTarget::FirstFree => match first_free_audio_track(tx.seq(seq)?, range) {
            Some(t) => t,
            None => {
                let slot = policy::slot_for_new_track(tx.seq(seq)?, TrackKind::Audio, None);
                policy::add_track_at(&mut tx, seq, TrackKind::Audio, slot)?
            }
        },
        RecordTarget::NewTrack => {
            let slot = policy::slot_for_new_track(tx.seq(seq)?, TrackKind::Audio, Some(0));
            policy::add_track_at(&mut tx, seq, TrackKind::Audio, slot)?
        }
    };
    tx.push(Change::InsertClip {
        seq,
        track,
        clip: Arc::new(clip),
    })?;
    tx.select(vec![clip_id]);
    tx.finish(Label {
        op: OpKind::InsertRecordedClip,
        count: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate() -> SampleRate {
        SampleRate::new(48_000).unwrap()
    }

    #[test]
    fn latency_moves_the_take_earlier_and_the_offset_later() {
        let start = Tick::from_secs(10).unwrap();
        let st = rate().sample_ticks().raw();
        let none = recorded_start(start, Duration::ZERO, 0, rate());
        assert_eq!(none, start);
        // 20 ms of latency = 960 samples at 48 kHz.
        let late = recorded_start(start, Duration::from_millis(20), 0, rate());
        assert_eq!(late.raw(), start.raw() - 960 * st);
        // +5 ms of user offset brings back 240 samples.
        let both = recorded_start(start, Duration::from_millis(20), 5, rate());
        assert_eq!(both.raw(), start.raw() - 720 * st);
    }

    #[test]
    fn rounding_goes_to_the_nearest_sample_with_halves_earlier() {
        let st = rate().sample_ticks().raw();
        let ns_per_tick = 1_000_000_000f64 / TICKS_PER_SECOND as f64;
        // Just under half a sample of latency rounds to no shift; just over
        // rounds a whole sample earlier; an exact half goes earlier too.
        let half = Duration::from_nanos((st as f64 / 2.0 * ns_per_tick).round() as u64);
        let r = recorded_start(Tick::ZERO, half, 0, rate());
        assert!(r.raw() == 0 || r.raw() == -st, "{r:?}");
        let over = Duration::from_nanos((st as f64 * 0.6 * ns_per_tick) as u64);
        assert_eq!(recorded_start(Tick::ZERO, over, 0, rate()).raw(), -st);
        let under = Duration::from_nanos((st as f64 * 0.4 * ns_per_tick) as u64);
        assert_eq!(recorded_start(Tick::ZERO, under, 0, rate()).raw(), 0);
        for t in [-7, 0, 13, 1_000_003] {
            let r = recorded_start(
                Tick::from_raw(t * st),
                Duration::from_micros(333),
                -17,
                rate(),
            );
            assert_eq!(r.raw().rem_euclid(st), 0, "on the sample grid");
        }
    }

    #[test]
    fn extreme_inputs_saturate() {
        let r = recorded_start(Tick::MAX, Duration::ZERO, i32::MAX, rate());
        assert!(r.raw() > 0);
        let r = recorded_start(Tick::from_raw(i64::MIN), Duration::MAX, i32::MIN, rate());
        assert!(r.raw() < 0);
    }
}
