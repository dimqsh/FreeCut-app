//! The read-only mix view (AU-010, AU-011, TL-008): which clips sound in a
//! stretch of a sequence, what they read, and with which settings. Pure
//! data; the mixer in `freecut-audio` decodes and sums it.

use crate::ids::{ClipId, MediaId, SeqId, TrackId};
use crate::model::{ClipAudio, ClipContent, Document, TrackAudio};
use crate::time::{Tick, TimeRange};

/// One stream of one clip the mixer plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixSource {
    pub clip: ClipId,
    pub track: TrackId,
    /// The library item whose file is read.
    pub asset: MediaId,
    /// Container index of the audio stream to read.
    pub stream: u32,
    /// How many streams of this clip are summed (AU-038): 1, or every
    /// stream of the file for "All streams mixed", when the clip is listed
    /// once per stream and the mixer scales each by 1/√n. Sources of one
    /// clip are adjacent and share everything but `stream`.
    pub mixed: u32,
    /// The clip's placement in sequence time.
    pub range: TimeRange,
    /// The source time playing at `range.start()`.
    pub source_start: Tick,
    /// The clip's settings. Keyframes are shared, so this is a cheap copy.
    pub audio: ClipAudio,
    /// The track stage the clip goes through (AU-013): track volume and pan.
    pub track_audio: TrackAudio,
}

/// Every clip that sounds somewhere in `window` of sequence `seq`, by track
/// (visual bottom-up, then audio top-down), then by start.
///
/// A clip sounds when it plays media with active sound settings, its library
/// item has an audio stream, and it isn't disabled (TL-036), muted (AU-007)
/// or on a muted track (TL-008, AU-011). Hidden tracks still sound: hiding is
/// a picture flag (TL-008). Video clips whose sound is detached are silent;
/// their audio clips play instead (TL-030).
///
/// The streams read are the clip's selection (AU-038): the chosen stream,
/// every stream for "All streams mixed" (one source each, see
/// [`MixSource::mixed`]), or the item's default stream (the one flagged
/// default, else the first; an extracted item has only its own), which is
/// also the fallback for a chosen stream the file no longer has.
///
/// Cheap enough for every mixed block: a binary search per track plus the
/// clips found. An unknown sequence has no sources.
pub fn mix_sources(doc: &Document, seq: SeqId, window: TimeRange) -> Vec<MixSource> {
    let Some(sequence) = doc.sequence(seq) else {
        return Vec::new();
    };
    let library = doc.library();
    let mut out = Vec::new();
    for (_, track) in sequence.tracks() {
        if track.header.flags.muted {
            continue;
        }
        for clip in track.clips_in_range(window) {
            let ClipContent::Media {
                asset,
                source,
                stream,
                ..
            } = &clip.content
            else {
                continue;
            };
            let Some(audio) = clip.audio.active().filter(|a| clip.enabled && !a.muted) else {
                continue;
            };
            let Some(item) = library.get(*asset) else {
                continue;
            };
            let streams = item.info.played_streams(*stream);
            let mixed = u32::try_from(streams.len()).unwrap_or(u32::MAX);
            for s in streams {
                out.push(MixSource {
                    clip: clip.id,
                    track: track.id,
                    asset: *asset,
                    stream: s.index,
                    mixed,
                    range: clip.range,
                    source_start: source.start(),
                    audio: audio.clone(),
                    track_audio: track.mix,
                });
            }
        }
    }
    out
}
