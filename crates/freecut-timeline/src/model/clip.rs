//! Clips (TL-011).

use std::sync::Arc;

use super::anim::ClipAnimations;
use super::audio::EmbeddedAudio;
use super::color::ColorStack;
use super::follow::Attachment;
use super::keys::ClipKeys;
use super::marker::Markers;
use super::text::TextClipData;
use super::transform::Transform;
use crate::ids::{ClipId, GroupId, LinkId, MediaId};
use crate::time::{Tick, TimeError, TimeRange};

/// The kind of time-based media a clip plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// Video, with any embedded audio.
    Video,
    /// Audio only.
    Audio,
}

/// Which audio streams of its file a media clip plays (AU-038, TL-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamSel {
    /// The audio stream with this container index.
    Index(u32),
    /// Every audio stream, summed at 1/√n each (mixed in `freecut-audio`).
    AllMixed,
}

/// Content FreeCut generates itself rather than reading from a file. Text
/// has its own payload ([`ClipContent::Text`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratedKind {
    Sticker,
    Effect,
    Adjustment,
}

/// What a clip shows or plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipContent {
    /// Time-based media. `source` is the part of the file shown, in source
    /// time (0 = first presentable frame). At normal speed its duration equals
    /// the clip's timeline duration.
    Media {
        asset: MediaId,
        kind: MediaKind,
        source: TimeRange,
        /// The audio stream selection (AU-038); `None` plays the media's
        /// default stream (the one flagged default, else the first).
        stream: Option<StreamSel>,
    },
    /// A still picture from a file. Any length.
    Image { asset: MediaId },
    /// A text clip (TX-001). Any length. Shared, since splits and undo
    /// snapshots copy clips far more often than text changes.
    Text(Arc<TextClipData>),
    /// Stickers, effects and adjustments. Any length.
    Generated { kind: GeneratedKind },
}

impl ClipContent {
    /// The source range, for content that has one.
    pub fn source(&self) -> Option<TimeRange> {
        match self {
            ClipContent::Media { source, .. } => Some(*source),
            ClipContent::Image { .. } | ClipContent::Text(_) | ClipContent::Generated { .. } => {
                None
            }
        }
    }

    pub fn asset(&self) -> Option<MediaId> {
        match self {
            ClipContent::Media { asset, .. } | ClipContent::Image { asset } => Some(*asset),
            ClipContent::Text(_) | ClipContent::Generated { .. } => None,
        }
    }

    /// The text payload, for text clips.
    pub fn text(&self) -> Option<&Arc<TextClipData>> {
        match self {
            ClipContent::Text(data) => Some(data),
            _ => None,
        }
    }

    /// Whether the content may carry a colour stack (CL-001): video, still
    /// images and adjustment clips. Text, audio, stickers and effects may
    /// not.
    pub fn takes_color(&self) -> bool {
        matches!(
            self,
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            } | ClipContent::Image { .. }
                | ClipContent::Generated {
                    kind: GeneratedKind::Adjustment
                }
        )
    }

    /// Whether the content draws on the canvas (everything but audio).
    pub fn is_visual(&self) -> bool {
        !matches!(
            self,
            ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            }
        )
    }
}

/// An item on a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    pub id: ClipId,
    /// Placement in sequence time.
    pub range: TimeRange,
    pub content: ClipContent,
    pub enabled: bool,
    /// User-chosen name; `None` shows the media or generated default.
    pub name: Option<String>,
    pub group: Option<GroupId>,
    pub link: Option<LinkId>,
    /// The clip's sound (AU-002, TL-030): always `Active` on audio clips,
    /// `None` on images, text and generated items.
    pub audio: EmbeddedAudio,
    /// Placement on the canvas (TR-001, TR-003, TR-015). Audio clips keep
    /// the default. The static values of keyed properties.
    pub transform: Transform,
    /// Keyframes of the transform and text properties (TR-027, TX-021),
    /// in the clip's content time.
    pub keys: ClipKeys,
    /// In, Out and Loop animations (FX-018, FX-019, TX-022). Empty on
    /// audio clips.
    pub animations: ClipAnimations,
    /// Clip markers in content time, sorted (TL-037). Markers outside the
    /// shown range are kept, hidden.
    pub markers: Markers,
    /// The main-track clip this item follows (TL-029). Always `None` on
    /// the main track.
    pub attach: Option<Attachment>,
    /// The colour stack's static values (CL-001), on content that
    /// [takes one](ClipContent::takes_color); `None` is the default stack.
    /// Shared, so splits and copies are cheap; a copy is never linked.
    pub color: Option<Arc<ColorStack>>,
}

impl Clip {
    /// A clip with default settings. Media clips get default sound
    /// settings; an inserter clears them for video without an audio stream.
    pub fn new(id: ClipId, range: TimeRange, content: ClipContent) -> Clip {
        let audio = match content {
            ClipContent::Media { .. } => EmbeddedAudio::Active(Default::default()),
            ClipContent::Image { .. } | ClipContent::Text(_) | ClipContent::Generated { .. } => {
                EmbeddedAudio::None
            }
        };
        Clip {
            id,
            range,
            content,
            enabled: true,
            name: None,
            group: None,
            link: None,
            audio,
            transform: Transform::default(),
            keys: ClipKeys::default(),
            animations: ClipAnimations::default(),
            markers: Markers::default(),
            attach: None,
            color: None,
        }
    }

    pub const fn start(&self) -> Tick {
        self.range.start()
    }

    pub const fn end(&self) -> Tick {
        self.range.end()
    }

    pub const fn duration(&self) -> Tick {
        self.range.duration()
    }

    /// The id a colour match's reference names (CL-020), if any: reserved
    /// like a link id, so it is never handed out again.
    pub fn match_reference_id(&self) -> Option<u64> {
        let xf = self.color.as_ref()?.match_xf.as_ref()?;
        Some(xf.reference.raw_id())
    }

    /// The same clip placed at `start`.
    pub fn moved_to(&self, start: Tick) -> Result<Clip, TimeError> {
        Ok(Clip {
            range: self.range.with_start(start)?,
            ..self.clone()
        })
    }

    /// Moves the in-point by `delta` (positive shortens) while the start stays:
    /// the content slides under a fixed head. Used by ripple trims.
    pub fn with_head_trimmed_in_place(&self, delta: Tick) -> Result<Clip, TimeError> {
        let duration = self.duration().checked_sub(delta)?;
        let mut clip = Clip {
            range: self.range.with_duration(duration)?,
            ..self.clone()
        };
        clip.content = shift_source_start(&self.content, delta, duration)?;
        // Keys stick to the content (TR-027): without a source range the
        // content time origin moves with the in-point.
        if self.content.source().is_none() {
            clip.keys = self
                .keys
                .with_origin(self.keys.origin().checked_add(delta)?);
        }
        Ok(clip)
    }

    /// Moves the in-point and the start by `delta` (positive shortens): the
    /// tail stays where it is. Used by plain trims.
    pub fn with_head_trimmed(&self, delta: Tick) -> Result<Clip, TimeError> {
        let start = self.start().checked_add(delta)?;
        let mut clip = self.with_head_trimmed_in_place(delta)?;
        clip.range = clip.range.with_start(start)?;
        Ok(clip)
    }

    /// Moves the out-point by `delta` (positive lengthens).
    pub fn with_tail_trimmed(&self, delta: Tick) -> Result<Clip, TimeError> {
        let duration = self.duration().checked_add(delta)?;
        let mut clip = Clip {
            range: self.range.with_duration(duration)?,
            ..self.clone()
        };
        if let ClipContent::Media {
            asset,
            kind,
            source,
            stream,
        } = &self.content
        {
            clip.content = ClipContent::Media {
                asset: *asset,
                kind: *kind,
                source: source.with_duration(duration)?,
                stream: *stream,
            };
        }
        Ok(clip)
    }

    /// Splits at sequence time `at`, which must lie strictly inside the clip.
    /// The left part keeps this clip's id; the right part gets `right_id`.
    /// Both keep the sound settings, except that the new inner edges have
    /// no fade (AU-006), and every keyframe (TR-027's one split rule). In
    /// stays on the left part and Out on the right; a Loop continues on the
    /// right part with its phase advanced (FX-018, FX-019, TX-022). Clip
    /// markers go to the part whose content range holds them; hidden ones
    /// to the nearer part (TL-037).
    pub fn split_at(&self, at: Tick, right_id: ClipId) -> Result<(Clip, Clip), TimeError> {
        let left_len = at.checked_sub(self.start())?;
        let right_len = self.end().checked_sub(at)?;
        let mut left = self.with_tail_trimmed(left_len.checked_sub(self.duration())?)?;
        let mut right = self.with_head_trimmed(left_len)?;
        debug_assert_eq!(right.duration(), right_len);
        right.id = right_id;
        if let EmbeddedAudio::Active(audio) = &self.audio {
            left.audio = EmbeddedAudio::Active(audio.clone().left_of_split());
            right.audio = EmbeddedAudio::Active(audio.clone().right_of_split());
        }
        let policy = super::anim::slot_policy(&self.content);
        let (l_anim, r_anim) = self.animations.split(left_len, self.duration(), policy);
        left.animations = l_anim;
        right.animations = r_anim;
        if !self.markers.is_empty() {
            let cut = self.content_time_at(at).ok_or(TimeError::Overflow)?;
            left.markers = self.markers.without(|m| m.at.start() >= cut);
            right.markers = self.markers.without(|m| m.at.start() < cut);
        }
        Ok((left, right))
    }

    /// The source time playing at sequence time `at` (TR time map at normal
    /// speed): where keyframes of the clip are anchored. `None` for content
    /// without a source range.
    pub fn source_time_at(&self, at: Tick) -> Option<Tick> {
        let source = self.content.source()?;
        let offset = at.checked_sub(self.start()).ok()?;
        source.start().checked_add(offset).ok()
    }
}

fn shift_source_start(
    content: &ClipContent,
    delta: Tick,
    new_duration: Tick,
) -> Result<ClipContent, TimeError> {
    Ok(match content {
        ClipContent::Media {
            asset,
            kind,
            source,
            stream,
        } => ClipContent::Media {
            asset: *asset,
            kind: *kind,
            source: TimeRange::new(source.start().checked_add(delta)?, new_duration)?,
            stream: *stream,
        },
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU64;

    fn id(n: u64) -> ClipId {
        ClipId::from_raw(NonZeroU64::new(n).unwrap())
    }

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    fn media(start: i64, dur: i64, src: i64) -> Clip {
        Clip::new(
            id(1),
            TimeRange::new(t(start), t(dur)).unwrap(),
            ClipContent::Media {
                asset: MediaId::from_raw(NonZeroU64::new(7).unwrap()),
                kind: MediaKind::Video,
                source: TimeRange::new(t(src), t(dur)).unwrap(),
                stream: Some(StreamSel::AllMixed),
            },
        )
    }

    #[test]
    fn head_trim_moves_start_and_source_together() {
        let c = media(100, 50, 10).with_head_trimmed(t(20)).unwrap();
        assert_eq!((c.start(), c.duration()), (t(120), t(30)));
        assert_eq!(c.content.source().unwrap().start(), t(30));
    }

    #[test]
    fn in_place_head_trim_keeps_start() {
        let c = media(100, 50, 10)
            .with_head_trimmed_in_place(t(-5))
            .unwrap();
        assert_eq!((c.start(), c.duration()), (t(100), t(55)));
        assert_eq!(
            c.content.source().unwrap(),
            TimeRange::new(t(5), t(55)).unwrap()
        );
    }

    #[test]
    fn split_preserves_output_and_ids() {
        let (l, r) = media(100, 50, 10).split_at(t(130), id(2)).unwrap();
        assert_eq!((l.id, l.start(), l.end()), (id(1), t(100), t(130)));
        assert_eq!((r.id, r.start(), r.end()), (id(2), t(130), t(150)));
        assert_eq!(
            l.content.source().unwrap(),
            TimeRange::new(t(10), t(30)).unwrap()
        );
        assert_eq!(
            r.content.source().unwrap(),
            TimeRange::new(t(40), t(20)).unwrap()
        );
    }

    #[test]
    fn media_clips_start_with_default_sound_and_others_without() {
        assert_eq!(
            media(0, 10, 0).audio,
            EmbeddedAudio::Active(Default::default())
        );
        let text = Clip::new(
            id(2),
            TimeRange::new(t(0), t(10)).unwrap(),
            ClipContent::Text(Arc::default()),
        );
        assert_eq!(text.audio, EmbeddedAudio::None);
        assert_eq!(text.transform, Transform::IDENTITY);
    }

    #[test]
    fn split_and_trims_keep_text_and_transform() {
        use crate::model::{Angle, Scale};
        let mut clip = Clip::new(
            id(1),
            TimeRange::new(t(100), t(50)).unwrap(),
            ClipContent::Text(Arc::new(TextClipData::with_content("Hi"))),
        );
        clip.transform.scale = Scale::from_percent(150);
        clip.transform.rotation = Angle::from_degrees(30);
        clip.transform.flip_h = true;
        let (l, r) = clip.split_at(t(120), id(2)).unwrap();
        for part in [&l, &r] {
            assert_eq!(part.content, clip.content);
            assert_eq!(part.transform, clip.transform);
            assert_eq!(part.content.source(), None);
        }
        let trimmed = clip.with_head_trimmed(t(10)).unwrap();
        assert_eq!(
            (trimmed.content.clone(), trimmed.transform),
            (clip.content.clone(), clip.transform)
        );
    }

    #[test]
    fn split_keeps_sound_settings_but_drops_the_inner_fades() {
        use crate::model::{Animatable, ClipAudio, FadeCurve, GainDb};
        let mut clip = media(100, 50, 10);
        let audio = ClipAudio {
            gain: Animatable::new(GainDb::from_hundredths(-600))
                .with_key(t(0), GainDb::ZERO)
                .with_key(t(500), GainDb::SILENT),
            muted: true,
            fade_in: t(5),
            fade_out: t(7),
            fade_in_curve: FadeCurve::Slow,
            fade_out_curve: FadeCurve::Smooth,
        };
        clip.audio = EmbeddedAudio::Active(audio.clone());
        let (l, r) = clip.split_at(t(130), id(2)).unwrap();
        let (la, ra) = (l.audio.active().unwrap(), r.audio.active().unwrap());
        assert_eq!(
            (&la.gain, la.muted, la.fade_in, la.fade_out),
            (&audio.gain, true, t(5), t(0))
        );
        assert_eq!(
            (&ra.gain, ra.muted, ra.fade_in, ra.fade_out),
            (&audio.gain, true, t(0), t(7))
        );
        assert_eq!(
            (la.fade_in_curve, ra.fade_out_curve),
            (FadeCurve::Slow, FadeCurve::Smooth)
        );
        assert_eq!(
            (l.content.clone(), r.content.clone()),
            (
                ClipContent::Media {
                    asset: MediaId::from_raw(NonZeroU64::new(7).unwrap()),
                    kind: MediaKind::Video,
                    source: TimeRange::new(t(10), t(30)).unwrap(),
                    stream: Some(StreamSel::AllMixed),
                },
                ClipContent::Media {
                    asset: MediaId::from_raw(NonZeroU64::new(7).unwrap()),
                    kind: MediaKind::Video,
                    source: TimeRange::new(t(40), t(20)).unwrap(),
                    stream: Some(StreamSel::AllMixed),
                }
            ),
            "both pieces keep the stream selection"
        );
        // The same envelope on both sides of the cut (source-anchored).
        for s in [0, 39, 40, 41, 200] {
            assert_eq!(la.gain_at(t(s)), ra.gain_at(t(s)));
        }
        assert_eq!(l.source_time_at(t(120)), Some(t(30)));
        assert_eq!(r.source_time_at(t(130)), Some(t(40)));
        // Trims keep the stored values.
        let trimmed = clip.with_tail_trimmed(t(-45)).unwrap();
        assert_eq!(trimmed.audio, clip.audio);
    }

    #[test]
    fn trimming_to_nothing_fails() {
        assert!(media(0, 50, 0).with_tail_trimmed(t(-50)).is_err());
        assert!(media(0, 50, 0).with_head_trimmed(t(50)).is_err());
    }
}
