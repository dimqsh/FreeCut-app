//! The edit model as a [`Schedule`] (AU-010).
//!
//! [`DocSchedule`] answers every block's window with TL's mix view
//! ([`mix_sources`]), which already decides what sounds: clip mute (AU-007),
//! disabled clips (TL-036), muted tracks (TL-008, AU-011), detached sound
//! (TL-030) and which streams a clip plays (AU-038). On top of that it
//! applies track solo (AU-012), a playback parameter that isn't in the
//! model. Everything else is a change of units:
//!
//! - **Key.** A clip playing one stream is keyed by its clip id, so it keeps
//!   its reader across moves, trims and stream changes. A clip playing "All
//!   streams mixed" has one input per stream, keyed by [`stream_input_key`].
//! - **Source.** The library item and the container index of the stream.
//! - **Volume.** A static [`GainDb`] becomes linear amplitude
//!   (`10^(dB/20)`, −∞ as 0); keyframes (AU-008) become a [`GainKeys`]
//!   envelope at the same source times, so they stick to the footage through
//!   head trims and splits (the mixer evaluates it at each sample's media
//!   time, on the axis of `source_start`).
//! - **Fades** keep their stored length and curve (AU-006); the mixer clamps
//!   them.
//! - **Streams.** Each of the `n` streams of "All streams mixed" gets the
//!   gain `1/√n` (AU-038).
//! - **Track stage.** The track's volume and pan (AU-013), with the track id
//!   for monitoring.

use std::collections::BTreeSet;
use std::sync::Arc;

use freecut_timeline::ids::{ClipId, SeqId, TrackId};
use freecut_timeline::mix::{MixSource, mix_sources};
use freecut_timeline::model::{
    Animatable, Document, FadeCurve as ModelCurve, GainDb, Interp, Pan, TrackAudio,
};
use freecut_timeline::time::TimeRange;

use crate::envelope::{GainEnvelope, GainKey, GainKeys, KeyInterp};
use crate::gain::{Fade, FadeCurve, gain_amplitude, stream_mix_factor};
use crate::source::{MixInput, Schedule, SourceKey, TrackStage};

/// Set on the keys of "All streams mixed" inputs. Clip ids never reach it
/// (they stay at or below `freecut_timeline::ids::MAX_ID`, 2^53).
const STREAM_KEY_FLAG: u64 = 1 << 63;
/// Where the stream index goes in such a key: above every clip id bit.
const STREAM_KEY_SHIFT: u32 = 54;
/// The stream index bits kept in such a key (9 bits, indices 0..=511).
const STREAM_KEY_MASK: u64 = (1 << (63 - STREAM_KEY_SHIFT)) - 1;

/// The mixer key of one stream of a clip that plays "All streams mixed"
/// (AU-038). Stable across edits, distinct from every clip id (which keys a
/// clip playing one stream) and from the clip's other streams.
///
/// Container indices are kept modulo 512; two audio streams of one file 512
/// indices apart would share a key, which no real file has.
pub fn stream_input_key(clip: ClipId, stream: u32) -> u64 {
    STREAM_KEY_FLAG | ((u64::from(stream) & STREAM_KEY_MASK) << STREAM_KEY_SHIFT) | clip.get()
}

/// What one sequence of a document snapshot plays.
///
/// The snapshot never changes: after an edit, build a new schedule from the
/// new document and hand it to the engine with the *same* opener, so the
/// readers of clips that are still scheduled stay open. Cloning is cheap.
#[derive(Debug, Clone)]
pub struct DocSchedule {
    doc: Arc<Document>,
    seq: SeqId,
    /// The soloed tracks (AU-012); `None` when nothing is soloed.
    solo: Option<Arc<BTreeSet<TrackId>>>,
}

impl DocSchedule {
    /// The sound of sequence `seq` of `doc`, without solo. An unknown
    /// sequence is silent.
    pub fn new(doc: impl Into<Arc<Document>>, seq: SeqId) -> DocSchedule {
        DocSchedule {
            doc: doc.into(),
            seq,
            solo: None,
        }
    }

    /// The sound of `doc`'s root sequence, without solo.
    pub fn root(doc: impl Into<Arc<Document>>) -> DocSchedule {
        let doc = doc.into();
        let seq = doc.root();
        DocSchedule::new(doc, seq)
    }

    /// The same schedule with track solo (AU-012), a playback parameter the
    /// app keeps in its view state: when `solo` names any track of the
    /// sequence, only those tracks are heard. A muted track stays silent
    /// even when soloed (the mix view never lists it).
    ///
    /// `None`, an empty set, or a set naming no track of the sequence (say,
    /// a soloed track that was removed) means nothing is soloed: everything
    /// is heard.
    ///
    /// Solo is for monitoring; [`OfflineMix`](crate::OfflineMix) ignores it
    /// unless asked to export as heard.
    pub fn with_solo(mut self, solo: Option<BTreeSet<TrackId>>) -> DocSchedule {
        let tracks = self.doc.sequence(self.seq).map(|s| {
            s.tracks()
                .map(|(_, t)| t.id)
                .filter(|id| solo.as_ref().is_some_and(|set| set.contains(id)))
                .collect::<BTreeSet<_>>()
        });
        self.solo = tracks.filter(|set| !set.is_empty()).map(Arc::new);
        self
    }

    /// The same schedule without solo: everything that isn't muted is heard.
    pub fn without_solo(&self) -> DocSchedule {
        DocSchedule {
            solo: None,
            ..self.clone()
        }
    }

    /// The soloed tracks in effect, if any (see [`DocSchedule::with_solo`]).
    pub fn solo(&self) -> Option<&BTreeSet<TrackId>> {
        self.solo.as_deref()
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn sequence(&self) -> SeqId {
        self.seq
    }

    /// The sequence's master volume as linear amplitude (AU-010); unity for
    /// an unknown sequence.
    pub fn master_amplitude(&self) -> f32 {
        self.doc
            .sequence(self.seq)
            .map_or(1.0, |s| gain_amplitude(s.master.gain))
    }

    fn sources(&self, window: TimeRange) -> Vec<MixSource> {
        let mut sources = mix_sources(&self.doc, self.seq, window);
        if let Some(solo) = &self.solo {
            sources.retain(|s| solo.contains(&s.track));
        }
        sources
    }
}

impl Schedule for DocSchedule {
    fn inputs(&self, window: TimeRange) -> Vec<MixInput> {
        self.sources(window).iter().map(MixInput::from).collect()
    }

    fn export_inputs(&self, window: TimeRange) -> Vec<MixInput> {
        self.without_solo().inputs(window)
    }
}

/// One stream of one audible clip as a mixer input (see the module docs).
impl From<&MixSource> for MixInput {
    fn from(source: &MixSource) -> MixInput {
        let key = if source.mixed > 1 {
            stream_input_key(source.clip, source.stream)
        } else {
            source.clip.get()
        };
        let audio = &source.audio;
        MixInput {
            clip: source.clip.get(),
            gain: envelope(&audio.gain),
            fade_in: Fade::new(audio.fade_in, curve(audio.fade_in_curve)),
            fade_out: Fade::new(audio.fade_out, curve(audio.fade_out_curve)),
            stream_mix: stream_mix_factor(usize::try_from(source.mixed).unwrap_or(usize::MAX)),
            track: track_stage(source.track, &source.track_audio),
            ..MixInput::new(
                key,
                SourceKey {
                    asset: source.asset.get(),
                    stream: source.stream,
                },
                source.range,
                source.source_start,
            )
        }
    }
}

/// A clip volume as the mixer's envelope: static amplitude, or the keys at
/// their source times with their interpolation.
fn envelope(gain: &Animatable<GainDb>) -> GainEnvelope {
    let keys: Vec<GainKey> = gain
        .keys()
        .iter()
        .map(|k| {
            let interp = match k.interp {
                Interp::Linear => KeyInterp::Linear,
                Interp::Hold => KeyInterp::Hold,
                eased => KeyInterp::Eased(eased),
            };
            GainKey::new(k.at, decibels(k.value), interp)
        })
        .collect();
    match GainKeys::new(keys) {
        Some(keys) => GainEnvelope::Keyframes(keys),
        None => GainEnvelope::Static(gain_amplitude(*gain.static_value())),
    }
}

/// A stored gain in decibels, −∞ for silence.
fn decibels(gain: GainDb) -> f32 {
    if gain.is_silent() {
        f32::NEG_INFINITY
    } else {
        (f64::from(gain.hundredths()) / 100.0) as f32
    }
}

fn curve(curve: ModelCurve) -> FadeCurve {
    match curve {
        ModelCurve::Linear => FadeCurve::Linear,
        ModelCurve::Smooth => FadeCurve::Smooth,
        ModelCurve::Fast => FadeCurve::Fast,
        ModelCurve::Slow => FadeCurve::Slow,
    }
}

/// A pan in percent as the mixer's −1..=1.
fn pan(pan: Pan) -> f32 {
    f32::from(pan.percent()) / 100.0
}

fn track_stage(track: TrackId, audio: &TrackAudio) -> TrackStage {
    TrackStage {
        id: track.get(),
        gain: gain_amplitude(audio.gain),
        pan: pan(audio.pan),
    }
}

#[cfg(test)]
mod tests;
