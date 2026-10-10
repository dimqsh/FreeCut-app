//! Clip audio settings (AU-005, AU-006, AU-007, AU-008), the embedded audio
//! of video clips (AU-002, TL-030) and the track stage (AU-013).
//!
//! Plain integer data: gain in hundredths of a decibel, fade lengths in
//! ticks, pan in percent. Conversion to linear amplitude happens only in
//! `freecut-audio` and at the UI edge.

use super::animate::{Animatable, Interpolate, Progress, lerp_i64};
use crate::ids::LinkId;
use crate::time::Tick;

/// A gain in hundredths of a decibel, with a sentinel for −∞ (silence).
///
/// Every value is either [`GainDb::SILENT`] or within
/// [`GainDb::MIN`]..=[`GainDb::MAX`]; the constructor clamps. Ordering puts
/// silence below every finite gain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GainDb(i32);

impl GainDb {
    /// 0.0 dB: the level unchanged.
    pub const ZERO: GainDb = GainDb(0);
    /// −∞ dB: silence.
    pub const SILENT: GainDb = GainDb(i32::MIN);
    /// −60.0 dB, the quietest finite gain (AU-005).
    pub const MIN: GainDb = GainDb(-6_000);
    /// +20.0 dB, the loudest gain (AU-005).
    pub const MAX: GainDb = GainDb(2_000);

    /// `hundredths` of a decibel, clamped into [`GainDb::MIN`]..=
    /// [`GainDb::MAX`]. `i32::MIN` (the stored form of −∞) stays silent.
    pub const fn from_hundredths(hundredths: i32) -> GainDb {
        if hundredths == i32::MIN {
            GainDb::SILENT
        } else if hundredths < GainDb::MIN.0 {
            GainDb::MIN
        } else if hundredths > GainDb::MAX.0 {
            GainDb::MAX
        } else {
            GainDb(hundredths)
        }
    }

    /// The stored value: hundredths of a decibel, `i32::MIN` for −∞.
    pub const fn hundredths(self) -> i32 {
        self.0
    }

    pub const fn is_silent(self) -> bool {
        self.0 == i32::MIN
    }
}

/// Gain interpolates in the dB domain (AU-008), rounded to 0.01 dB when
/// evaluated.
///
/// A segment that touches −∞ is played in linear amplitude, so it doesn't
/// jump; `freecut-audio` evaluates such segments itself (see
/// [`Animatable::segment_at`]). This model-side value, used for drawing and
/// read-outs, approximates them by the nearer key: silence before the
/// midpoint, the other key's level from the midpoint on (where the linear
/// ramp is within 6.02 dB of it).
impl Interpolate for GainDb {
    fn interpolate(a: &GainDb, b: &GainDb, p: Progress) -> GainDb {
        if a.is_silent() || b.is_silent() {
            return if p.is_past_half() { *b } else { *a };
        }
        let v = lerp_i64(i64::from(a.0), i64::from(b.0), p);
        // Between two finite gains, so within the clamped range.
        GainDb::from_hundredths(i32::try_from(v).unwrap_or(a.0))
    }
}

/// A stereo balance from −100 (left) through 0 (centre) to +100 (right)
/// (AU-013, AU-014).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pan(i8);

impl Pan {
    pub const CENTRE: Pan = Pan(0);
    pub const LEFT: Pan = Pan(-100);
    pub const RIGHT: Pan = Pan(100);

    /// `percent`, clamped into −100..=100.
    pub fn new(percent: i32) -> Pan {
        // Clamped into i8 range first.
        Pan(i8::try_from(percent.clamp(-100, 100)).unwrap_or(0))
    }

    pub const fn percent(self) -> i8 {
        self.0
    }
}

/// Pan interpolates linearly, rounded when evaluated.
impl Interpolate for Pan {
    fn interpolate(a: &Pan, b: &Pan, p: Progress) -> Pan {
        let v = lerp_i64(i64::from(a.0), i64::from(b.0), p);
        Pan::new(i32::try_from(v).unwrap_or(0))
    }
}

/// The shape of a fade (AU-006). `p` runs from 0 to 1 across a fade-in; a
/// fade-out mirrors it. Evaluated in `freecut-audio`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FadeCurve {
    /// Linear amplitude: `p`.
    #[default]
    Linear,
    /// S-curve: `½ − ½·cos(π·p)`.
    Smooth,
    /// Equal-power quarter sine, quick at first: `sin(π·p/2)`.
    Fast,
    /// Gradual at first: `1 − cos(π·p/2)`.
    Slow,
}

impl FadeCurve {
    pub const ALL: [FadeCurve; 4] = [
        FadeCurve::Linear,
        FadeCurve::Smooth,
        FadeCurve::Fast,
        FadeCurve::Slow,
    ];
}

/// The track stage of a track that carries sound (AU-013): volume and pan,
/// after every clip of the track. Tracks without sound (text, stickers,
/// effects, adjustments) keep the defaults, which nothing reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TrackAudio {
    /// −∞, or −60.0 to +12.0 dB. Default 0.0 dB.
    pub gain: GainDb,
    /// Balance law of AU-013. Default centre.
    pub pan: Pan,
}

impl TrackAudio {
    /// +12.0 dB, the loudest track volume (AU-013).
    pub const MAX_GAIN: GainDb = GainDb::from_hundredths(1_200);

    /// `gain` limited to the track range.
    pub fn clamp_gain(gain: GainDb) -> GainDb {
        gain.min(TrackAudio::MAX_GAIN)
    }
}

/// The master stage of a sequence (AU-010): the master volume, applied to
/// the whole mix before the safety limiter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MasterAudio {
    /// Master volume: −∞, or −60.0 to +12.0 dB. Default 0.0 dB.
    pub gain: GainDb,
}

impl MasterAudio {
    /// +12.0 dB, the loudest master volume (AU-010).
    pub const MAX_GAIN: GainDb = GainDb::from_hundredths(1_200);

    /// `gain` limited to the master range: anything above
    /// [`MasterAudio::MAX_GAIN`] becomes it.
    pub fn clamp_gain(gain: GainDb) -> GainDb {
        gain.min(MasterAudio::MAX_GAIN)
    }
}

/// The sound settings of a clip: an audio clip's own, or the embedded audio
/// of a video clip. They move between the two on detach and restore (AU-003),
/// keyframes included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ClipAudio {
    /// Clip volume (AU-005), with optional keyframes in source time
    /// (AU-008). With keys, the envelope replaces the static volume.
    pub gain: Animatable<GainDb>,
    /// Silences the clip but keeps every other setting (AU-007).
    pub muted: bool,
    /// Fade lengths on the timeline, never negative (AU-006). They may be
    /// longer than the clip: the stored values survive trims, and
    /// [`ClipAudio::effective_fades`] clamps them when evaluated.
    pub fade_in: Tick,
    pub fade_out: Tick,
    /// Fade shapes (AU-006). Default linear.
    pub fade_in_curve: FadeCurve,
    pub fade_out_curve: FadeCurve,
}

impl ClipAudio {
    /// Settings with static volume `gain` and every other value default.
    pub fn with_gain(gain: GainDb) -> ClipAudio {
        ClipAudio {
            gain: Animatable::new(gain),
            ..ClipAudio::default()
        }
    }

    /// The volume at source time `source` (AU-008, AU-009): the static
    /// volume, or the keyframe envelope. See [`GainDb`]'s `Interpolate` for
    /// segments touching −∞. Fades and mute are not included.
    pub fn gain_at(&self, source: Tick) -> GainDb {
        self.gain.value_at(source)
    }

    /// The fades a clip of `clip_duration` actually plays (AU-006): the
    /// fade-in first, then the fade-out in what is left.
    pub fn effective_fades(&self, clip_duration: Tick) -> (Tick, Tick) {
        let duration = clip_duration.max(Tick::ZERO);
        let fade_in = self.fade_in.max(Tick::ZERO).min(duration);
        // Both are non-negative and fade_in <= duration: no overflow.
        let rest = Tick::from_raw(duration.raw() - fade_in.raw());
        let fade_out = self.fade_out.max(Tick::ZERO).min(rest);
        (fade_in, fade_out)
    }

    /// The settings of the left piece of a split: the new right edge has no
    /// fade, so its curve goes back to the default (AU-006). Keys are all
    /// kept (TR-027).
    pub fn left_of_split(self) -> ClipAudio {
        ClipAudio {
            fade_out: Tick::ZERO,
            fade_out_curve: FadeCurve::default(),
            ..self
        }
    }

    /// The settings of the right piece of a split: the new left edge has no
    /// fade, so its curve goes back to the default (AU-006). Keys are all
    /// kept (TR-027).
    pub fn right_of_split(self) -> ClipAudio {
        ClipAudio {
            fade_in: Tick::ZERO,
            fade_in_curve: FadeCurve::default(),
            ..self
        }
    }
}

/// The sound a clip carries (AU-002, TL-030).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum EmbeddedAudio {
    /// No sound: images, generated items, and video without an audio stream.
    #[default]
    None,
    /// The clip plays its own sound with these settings. Audio clips are
    /// always `Active`.
    Active(ClipAudio),
    /// The sound was detached to the audio clips carrying this link
    /// (TL-030); the video clip itself is silent. Only on video clips.
    Detached(LinkId),
}

impl EmbeddedAudio {
    /// The settings, when the clip plays its own sound.
    pub const fn active(&self) -> Option<&ClipAudio> {
        match self {
            EmbeddedAudio::Active(audio) => Some(audio),
            EmbeddedAudio::None | EmbeddedAudio::Detached(_) => None,
        }
    }

    /// The link of the detached sound, if any.
    pub const fn detached(&self) -> Option<LinkId> {
        match self {
            EmbeddedAudio::Detached(link) => Some(*link),
            EmbeddedAudio::None | EmbeddedAudio::Active(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    #[test]
    fn gain_clamps_into_range_but_keeps_silence() {
        assert_eq!(GainDb::from_hundredths(-7_000), GainDb::MIN);
        assert_eq!(GainDb::from_hundredths(2_001), GainDb::MAX);
        assert_eq!(GainDb::from_hundredths(-350).hundredths(), -350);
        assert_eq!(GainDb::from_hundredths(i32::MIN), GainDb::SILENT);
        assert!(GainDb::SILENT.is_silent() && !GainDb::MIN.is_silent());
        assert_eq!(GainDb::default(), GainDb::ZERO);
    }

    #[test]
    fn silence_sorts_below_every_finite_gain() {
        assert!(GainDb::SILENT < GainDb::MIN);
        assert!(GainDb::MIN < GainDb::ZERO && GainDb::ZERO < GainDb::MAX);
    }

    #[test]
    fn default_clip_audio_is_unity_unmuted_without_fades() {
        let a = ClipAudio::default();
        assert_eq!(
            (*a.gain.static_value(), a.muted, a.fade_in, a.fade_out),
            (GainDb::ZERO, false, Tick::ZERO, Tick::ZERO)
        );
        assert!(!a.gain.is_animated());
        assert_eq!(
            (a.fade_in_curve, a.fade_out_curve),
            (FadeCurve::Linear, FadeCurve::Linear)
        );
        assert_eq!(a, ClipAudio::with_gain(GainDb::ZERO));
    }

    fn p(num: i64, den: i64) -> Progress {
        Progress::new(num, den).unwrap()
    }

    #[test]
    fn gain_interpolates_in_db_and_steps_at_the_midpoint_into_silence() {
        let lerp = |a, b, p| GainDb::interpolate(&GainDb(a), &GainDb(b), p);
        assert_eq!(lerp(-600, 0, p(1, 2)), GainDb(-300));
        assert_eq!(lerp(-600, 0, p(1, 3)), GainDb(-400));
        assert_eq!(lerp(0, -1, p(1, 2)), GainDb(0), "halves to even");
        let silent = GainDb::SILENT;
        assert_eq!(GainDb::interpolate(&silent, &GainDb(0), p(1, 3)), silent);
        assert_eq!(GainDb::interpolate(&silent, &GainDb(0), p(1, 2)), GainDb(0));
        assert_eq!(GainDb::interpolate(&GainDb(0), &silent, p(2, 3)), silent);
    }

    #[test]
    fn gain_at_reads_the_envelope_in_source_time() {
        let mut a = ClipAudio::with_gain(GainDb(-1_000));
        assert_eq!(a.gain_at(t(5)), GainDb(-1_000));
        a.gain = a
            .gain
            .with_key(t(10), GainDb(-600))
            .with_key(t(20), GainDb(0));
        assert_eq!(a.gain_at(t(0)), GainDb(-600));
        assert_eq!(a.gain_at(t(15)), GainDb(-300));
        assert_eq!(a.gain_at(t(40)), GainDb(0));
    }

    #[test]
    fn pan_clamps_and_interpolates() {
        assert_eq!(Pan::new(-300), Pan::LEFT);
        assert_eq!(Pan::new(101), Pan::RIGHT);
        assert_eq!(Pan::default(), Pan::CENTRE);
        assert_eq!(
            Pan::interpolate(&Pan::LEFT, &Pan::RIGHT, p(1, 4)).percent(),
            -50
        );
        assert_eq!(Pan::interpolate(&Pan(0), &Pan(3), p(1, 2)).percent(), 2);
    }

    #[test]
    fn track_audio_defaults_to_unity_centre() {
        let a = TrackAudio::default();
        assert_eq!((a.gain, a.pan), (GainDb::ZERO, Pan::CENTRE));
        assert_eq!(TrackAudio::clamp_gain(GainDb::MAX), TrackAudio::MAX_GAIN);
        assert_eq!(TrackAudio::clamp_gain(GainDb::SILENT), GainDb::SILENT);
    }

    #[test]
    fn effective_fades_fit_the_fade_in_first() {
        let a = ClipAudio {
            fade_in: t(30),
            fade_out: t(50),
            ..ClipAudio::default()
        };
        assert_eq!(a.effective_fades(t(100)), (t(30), t(50)));
        assert_eq!(a.effective_fades(t(60)), (t(30), t(30)));
        assert_eq!(a.effective_fades(t(20)), (t(20), t(0)));
        assert_eq!(a.effective_fades(t(0)), (t(0), t(0)));
    }

    #[test]
    fn split_pieces_lose_the_fade_on_the_new_edge() {
        let a = ClipAudio {
            gain: Animatable::new(GainDb(-100)).with_key(t(9), GainDb(-200)),
            fade_in: t(3),
            fade_out: t(4),
            fade_in_curve: FadeCurve::Smooth,
            fade_out_curve: FadeCurve::Fast,
            ..ClipAudio::default()
        };
        let (l, r) = (a.clone().left_of_split(), a.clone().right_of_split());
        assert_eq!(
            (l.fade_in, l.fade_out, l.fade_in_curve, l.fade_out_curve),
            (t(3), t(0), FadeCurve::Smooth, FadeCurve::Linear)
        );
        assert_eq!(
            (r.fade_in, r.fade_out, r.fade_in_curve, r.fade_out_curve),
            (t(0), t(4), FadeCurve::Linear, FadeCurve::Fast)
        );
        assert_eq!((&l.gain, &r.gain), (&a.gain, &a.gain), "all keys kept");
    }

    #[test]
    fn embedded_audio_accessors() {
        let link = LinkId::from_raw(core::num::NonZeroU64::MIN);
        assert_eq!(EmbeddedAudio::Detached(link).detached(), Some(link));
        assert_eq!(EmbeddedAudio::Detached(link).active(), None);
        assert_eq!(
            EmbeddedAudio::Active(ClipAudio::default()).active(),
            Some(&ClipAudio::default())
        );
        assert_eq!(EmbeddedAudio::None.active(), None);
    }
}
