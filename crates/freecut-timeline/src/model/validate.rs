//! Document invariants (TL-004, TL-005, TL-006, TL-011).
//!
//! `validate` reports every violation it finds. It runs at the end of every
//! transaction, so a planner bug becomes a refused edit rather than a corrupt
//! document, and after every step of the property tests.

use std::collections::{BTreeMap, BTreeSet};

use core::num::NonZeroU64;

use super::{
    Clip, ClipContent, Document, EmbeddedAudio, Library, Markers, MediaKind, MediaSource, PropPath,
    Sequence, TextClipData, Track, TrackKind, slot_policy,
};
use crate::edit::Limits;
use crate::ids::{ClipId, GroupId, LinkId, MediaId, SeqId, TrackId};
use crate::time::Tick;

/// Inputs `validate` needs from outside the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidateCtx {
    pub limits: Limits,
    /// Every id must be below this; `None` skips the check.
    pub id_high_water: Option<NonZeroU64>,
}

/// One broken invariant, with its location.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvariantViolation {
    #[error("root sequence {0} is missing")]
    RootMissing(SeqId),
    #[error("{seq}: the main track must exist and be a video track")]
    MainTrack { seq: SeqId },
    #[error("{seq}: track {track} of kind {kind:?} is in the wrong lane")]
    WrongLane {
        seq: SeqId,
        track: TrackId,
        kind: TrackKind,
    },
    #[error("{seq}: track {track} breaks the layering band order")]
    BandOrder { seq: SeqId, track: TrackId },
    #[error("{seq}: {count} tracks exceed the limit of {max}")]
    TooManyTracks {
        seq: SeqId,
        count: usize,
        max: usize,
    },
    #[error("{seq}: the magnetic main track has a gap or doesn't start at 0")]
    MainNotContiguous { seq: SeqId },
    #[error("track {track}: clips are out of order or overlap at {clip}")]
    Overlap { track: TrackId, clip: ClipId },
    #[error("clip {clip} starts before 0")]
    NegativeStart { clip: ClipId },
    #[error("clip {clip} doesn't belong on track {track}")]
    KindMismatch { track: TrackId, clip: ClipId },
    #[error("clip {clip}: source range and clip length disagree")]
    SourceLength { clip: ClipId },
    #[error("clip {clip}: source range is outside the media")]
    SourceBounds { clip: ClipId },
    #[error("id {0} is used more than once")]
    DuplicateId(u64),
    #[error("id {0} is not below the allocator's next id")]
    IdAboveHighWater(u64),
    #[error("group {group} has fewer than 2 members")]
    SmallGroup { group: GroupId },
    #[error("link {link} has fewer than 2 members")]
    SmallLink { link: LinkId },
    #[error("clip {clip} plays media that isn't in the library")]
    MissingMedia { clip: ClipId },
    #[error("clip {clip} has sound settings its content can't carry")]
    AudioMismatch { clip: ClipId },
    #[error("clip {clip} has a negative fade")]
    NegativeFade { clip: ClipId },
    #[error("clip {clip}: a transform value is out of range")]
    TransformRange { clip: ClipId },
    #[error("clip {clip}: a text style or layout value is out of range")]
    TextRange { clip: ClipId },
    #[error("clip {clip} has both a background box and a bubble switched on")]
    TextBoxAndBubble { clip: ClipId },
    #[error("clip {clip} has keyframes its content can't carry")]
    Keys { clip: ClipId },
    #[error("clip {clip} has animations its kind can't carry or out of range")]
    Animations { clip: ClipId },
    #[error("media {media} is extracted audio but holds more than its one stream")]
    DerivedMedia { media: MediaId },
    #[error("the canvas size {width}×{height} is invalid")]
    InvalidCanvas { width: u32, height: u32 },
    #[error("the clip index is out of date for {clip}")]
    StaleIndex { clip: ClipId },
    #[error("the clip index holds {indexed} clips, the document {actual}")]
    IndexSize { indexed: usize, actual: usize },
    #[error("{seq}: timeline markers are out of order or invalid")]
    SeqMarkers { seq: SeqId },
    #[error("clip {clip}: its markers are out of order or invalid")]
    ClipMarkers { clip: ClipId },
    #[error("clip {clip}: its attachment names no main-track clip of its sequence")]
    Attachment { clip: ClipId },
    #[error("clip {clip} has a colour stack its content can't carry or out of range")]
    Color { clip: ClipId },
}

impl Document {
    /// Checks every invariant and returns all violations.
    pub fn validate(&self, ctx: &ValidateCtx) -> Result<(), Vec<InvariantViolation>> {
        let mut out = Vec::new();
        if self.sequence(self.root()).is_none() {
            out.push(InvariantViolation::RootMissing(self.root()));
        }
        check_settings(self, &mut out);
        check_library(self.library(), &mut out);
        let above = |raw: u64| ctx.id_high_water.is_some_and(|hw| raw >= hw.get());
        let mut ids: BTreeSet<u64> = self.library().items().map(|m| m.id.get()).collect();
        for raw in &ids {
            if above(*raw) {
                out.push(InvariantViolation::IdAboveHighWater(*raw));
            }
        }
        // An extracted item's parent id stays reserved even when the parent
        // is gone.
        for item in self.library().items() {
            if let MediaSource::AudioOf { source, .. } = item.source
                && above(source.get())
            {
                out.push(InvariantViolation::IdAboveHighWater(source.get()));
            }
        }
        let mut relation_ids = BTreeSet::new();
        let mut clip_count = 0;
        for seq in self.sequences() {
            let mut entity = vec![seq.id.get()];
            entity.extend(seq.markers.iter().map(|m| m.id.get()));
            for (_, track) in seq.tracks() {
                entity.push(track.id.get());
                for clip in track.clips() {
                    entity.push(clip.id.get());
                    entity.extend(clip.markers.iter().map(|m| m.id.get()));
                    clip_count += 1;
                    let indexed = self.index_entry(clip.id);
                    let expected = Some((seq.id, track.id, clip.start()));
                    if indexed.map(|e| (e.seq, e.track, e.start)) != expected {
                        out.push(InvariantViolation::StaleIndex { clip: clip.id });
                    }
                    relation_ids.extend(clip.group.map(GroupId::get));
                    relation_ids.extend(clip.link.map(LinkId::get));
                    relation_ids.extend(clip.audio.detached().map(LinkId::get));
                }
            }
            for raw in entity {
                if !ids.insert(raw) {
                    out.push(InvariantViolation::DuplicateId(raw));
                }
                if above(raw) {
                    out.push(InvariantViolation::IdAboveHighWater(raw));
                }
            }
            check_structure(seq, ctx, &mut out);
            for (_, track) in seq.tracks() {
                check_track(track, self.library(), &mut out);
            }
            check_relations(seq, &mut out);
            check_attachments(seq, &mut out);
        }
        // Group and link ids are shared by their members, but come from the
        // same id space as everything else.
        for raw in relation_ids {
            if ids.contains(&raw) {
                out.push(InvariantViolation::DuplicateId(raw));
            }
            if above(raw) {
                out.push(InvariantViolation::IdAboveHighWater(raw));
            }
        }
        if clip_count != self.clip_count() {
            out.push(InvariantViolation::IndexSize {
                indexed: self.clip_count(),
                actual: clip_count,
            });
        }
        if out.is_empty() { Ok(()) } else { Err(out) }
    }

    /// Checks the invariants an edit with this scope could have broken: the
    /// structure of touched sequences, the clips of touched tracks, new ids,
    /// and groups and links when the edit involved them.
    pub(crate) fn validate_scope(
        &self,
        ctx: &ValidateCtx,
        scope: &Scope,
    ) -> Result<(), Vec<InvariantViolation>> {
        let mut out = Vec::new();
        if scope.settings {
            check_settings(self, &mut out);
        }
        if scope.library {
            check_library(self.library(), &mut out);
            // Media changed: every clip must still find its media in range.
            for seq in self.sequences() {
                for (_, track) in seq.tracks() {
                    check_track(track, self.library(), &mut out);
                }
            }
        }
        for &seq_id in &scope.seqs {
            let Some(seq) = self.sequence(seq_id) else {
                continue;
            };
            check_structure(seq, ctx, &mut out);
            if scope.relations {
                check_relations(seq, &mut out);
            }
            if scope.attachments {
                check_attachments(seq, &mut out);
            }
        }
        for &(seq_id, track_id) in &scope.tracks {
            if let Some((_, track)) = self.sequence(seq_id).and_then(|s| s.find_track(track_id)) {
                check_track(track, self.library(), &mut out);
            }
        }
        for &raw in &scope.new_ids {
            if ctx.id_high_water.is_some_and(|hw| raw >= hw.get()) {
                out.push(InvariantViolation::IdAboveHighWater(raw));
            }
        }
        if out.is_empty() { Ok(()) } else { Err(out) }
    }
}

/// What an edit touched, collected from its changes.
#[derive(Debug, Default)]
pub(crate) struct Scope {
    pub(crate) seqs: BTreeSet<SeqId>,
    pub(crate) tracks: BTreeSet<(SeqId, TrackId)>,
    pub(crate) new_ids: Vec<u64>,
    /// Some changed clip belongs to a group or link.
    pub(crate) relations: bool,
    /// Some changed clip is attached, or a clip (maybe an owner) left.
    pub(crate) attachments: bool,
    pub(crate) settings: bool,
    pub(crate) library: bool,
}

impl Scope {
    pub(crate) fn track(&mut self, seq: SeqId, track: TrackId) {
        self.seqs.insert(seq);
        self.tracks.insert((seq, track));
    }

    pub(crate) fn structure(&mut self, seq: SeqId) {
        self.seqs.insert(seq);
    }

    pub(crate) fn new_id(&mut self, raw: u64) {
        self.new_ids.push(raw);
    }

    pub(crate) fn note_relations(&mut self, has: bool) {
        self.relations |= has;
    }

    pub(crate) fn note_attachments(&mut self, has: bool) {
        self.attachments |= has;
    }
}

/// Track list, lanes, bands, track count and the magnetic main track.
fn check_structure(seq: &Sequence, ctx: &ValidateCtx, out: &mut Vec<InvariantViolation>) {
    let sid = seq.id;
    match seq.main_track() {
        Some(main) if main.kind == TrackKind::Video => {}
        _ => out.push(InvariantViolation::MainTrack { seq: sid }),
    }
    for track in seq.visual_tracks() {
        if track.kind.is_audio() {
            out.push(InvariantViolation::WrongLane {
                seq: sid,
                track: track.id,
                kind: track.kind,
            });
        }
    }
    for track in seq.audio_tracks() {
        if !track.kind.is_audio() {
            out.push(InvariantViolation::WrongLane {
                seq: sid,
                track: track.id,
                kind: track.kind,
            });
        }
    }
    let mut prev_band = None;
    for track in seq.visual_tracks() {
        let band = track.kind.band();
        if band < prev_band {
            out.push(InvariantViolation::BandOrder {
                seq: sid,
                track: track.id,
            });
        }
        prev_band = prev_band.max(band);
    }
    if !markers_ok(&seq.markers, false) {
        out.push(InvariantViolation::SeqMarkers { seq: sid });
    }
    if seq.track_count() > ctx.limits.max_tracks {
        out.push(InvariantViolation::TooManyTracks {
            seq: sid,
            count: seq.track_count(),
            max: ctx.limits.max_tracks,
        });
    }
    if seq.flags.main_magnet
        && let Some(main) = seq.main_track()
    {
        let mut expected = Tick::ZERO;
        for clip in main.clips() {
            if clip.start() != expected {
                out.push(InvariantViolation::MainNotContiguous { seq: sid });
                break;
            }
            expected = clip.end();
        }
    }
}

/// The canvas size is usable.
fn check_settings(doc: &Document, out: &mut Vec<InvariantViolation>) {
    let s = doc.settings();
    if !s.canvas_is_valid() {
        out.push(InvariantViolation::InvalidCanvas {
            width: s.width,
            height: s.height,
        });
    }
}

/// Order, overlap, placement, kind, media references, media bounds and
/// transform and text value ranges of one track's clips.
fn check_track(track: &Track, library: &Library, out: &mut Vec<InvariantViolation>) {
    let mut prev_end: Option<Tick> = None;
    let mut prev_key = None;
    for clip in track.clips() {
        let key = (clip.start(), clip.id);
        if prev_key.is_some_and(|p| p >= key) || prev_end.is_some_and(|e| clip.start() < e) {
            out.push(InvariantViolation::Overlap {
                track: track.id,
                clip: clip.id,
            });
        }
        prev_key = Some(key);
        prev_end = Some(clip.end());
        if clip.start().is_negative() {
            out.push(InvariantViolation::NegativeStart { clip: clip.id });
        }
        if !track.kind.accepts(&clip.content) {
            out.push(InvariantViolation::KindMismatch {
                track: track.id,
                clip: clip.id,
            });
        }
        if let Some(asset) = clip.content.asset()
            && library.get(asset).is_none()
        {
            out.push(InvariantViolation::MissingMedia { clip: clip.id });
        }
        check_audio(clip, out);
        check_motion(clip, out);
        if !markers_ok(&clip.markers, true) {
            out.push(InvariantViolation::ClipMarkers { clip: clip.id });
        }
        if !clip.transform.is_valid() {
            out.push(InvariantViolation::TransformRange { clip: clip.id });
        }
        // CL-001: only video, images and adjustment clips carry a stack,
        // stored as `None` while it is the default one, values in range.
        if let Some(stack) = &clip.color
            && (!clip.content.takes_color()
                || !stack.is_valid()
                || **stack == crate::model::ColorStack::default())
        {
            out.push(InvariantViolation::Color { clip: clip.id });
        }
        if let ClipContent::Text(data) = &clip.content {
            // TX-054: at most one of the two box layers is on.
            let both = data.style.background.on && data.style.bubble.on;
            if both {
                out.push(InvariantViolation::TextBoxAndBubble { clip: clip.id });
            }
            let ranges_ok = if both {
                let mut probe = TextClipData::clone(data);
                probe.style.background.on = false;
                probe.is_valid()
            } else {
                data.is_valid()
            };
            if !ranges_ok {
                out.push(InvariantViolation::TextRange { clip: clip.id });
            }
        }
        if let ClipContent::Media { asset, source, .. } = &clip.content {
            if source.duration() != clip.duration() {
                out.push(InvariantViolation::SourceLength { clip: clip.id });
            }
            let too_long = library
                .duration(*asset)
                .is_some_and(|len| source.end() > len);
            if source.start().is_negative() || too_long {
                out.push(InvariantViolation::SourceBounds { clip: clip.id });
            }
        }
    }
}

/// Sound settings fit the content (AU-002, TL-030): none on images, text
/// and generated items, always active on audio clips, detached only on video
/// clips (and then carrying no other link), and fades never negative.
fn check_audio(clip: &Clip, out: &mut Vec<InvariantViolation>) {
    let fits = match (&clip.content, &clip.audio) {
        (
            ClipContent::Image { .. } | ClipContent::Text(_) | ClipContent::Generated { .. },
            audio,
        ) => *audio == EmbeddedAudio::None,
        (
            ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            },
            audio,
        ) => matches!(audio, EmbeddedAudio::Active(_)),
        (_, EmbeddedAudio::Detached(link)) => clip.link.is_none_or(|l| l == *link),
        (_, EmbeddedAudio::None | EmbeddedAudio::Active(_)) => true,
    };
    if !fits {
        out.push(InvariantViolation::AudioMismatch { clip: clip.id });
    }
    if let EmbeddedAudio::Active(audio) = &clip.audio
        && (audio.fade_in.is_negative() || audio.fade_out.is_negative())
    {
        out.push(InvariantViolation::NegativeFade { clip: clip.id });
    }
}

/// Keys only on properties the content has a place for (transform keys on
/// visual clips, text keys on text clips), each value of its property's
/// kind (TR-027); animations only on visual clips, within the slot policy
/// of the clip's kind and in range (FX-018, FX-019).
fn check_motion(clip: &Clip, out: &mut Vec<InvariantViolation>) {
    let text = clip.content.text().is_some();
    let keys_ok = clip.keys.tracks().iter().all(|t| {
        let fits = match t.prop() {
            PropPath::Transform(_) => clip.content.is_visual(),
            PropPath::Text(_) | PropPath::FillStopColor(_) | PropPath::Reveal => text,
            PropPath::Color(_) | PropPath::ColorPuck(_) | PropPath::ColorMask { .. } => {
                clip.content.takes_color()
            }
        };
        let kind = t.prop().kind();
        fits && !t.keys().is_empty()
            && t.keys().iter().all(|k| Some(k.value.kind()) == kind)
            && t.keys().windows(2).all(|w| w[0].at < w[1].at)
    });
    if !keys_ok {
        out.push(InvariantViolation::Keys { clip: clip.id });
    }
    let anims_ok = match slot_policy(&clip.content) {
        Some(policy) => clip.animations.is_valid(policy),
        None => clip.animations.is_empty(),
    };
    if !anims_ok {
        out.push(InvariantViolation::Animations { clip: clip.id });
    }
}

/// Extracted-audio items hold exactly their one stream and no picture
/// (AU-004).
fn check_library(library: &Library, out: &mut Vec<InvariantViolation>) {
    for item in library.items() {
        if let MediaSource::AudioOf { source, stream } = item.source {
            let info = &item.info;
            let ok = source != item.id
                && info.video.is_empty()
                && info.default_video.is_none()
                && info.still.is_none()
                && info.audio.len() == 1
                && info.audio_stream(stream).is_some();
            if !ok {
                out.push(InvariantViolation::DerivedMedia { media: item.id });
            }
        }
    }
}

/// Sorted by `(start, id)` with unique ids, names and notes within limits,
/// and beats only on clips (TL-037).
fn markers_ok(markers: &Markers, on_clip: bool) -> bool {
    markers.is_sorted() && markers.iter().all(|m| m.is_valid(on_clip))
}

/// Attached items sit off the main track, in a sequence that follows, and
/// name a clip on its main track (TL-029).
fn check_attachments(seq: &Sequence, out: &mut Vec<InvariantViolation>) {
    let owners: BTreeSet<ClipId> = seq
        .main_track()
        .map(|m| m.clips().iter().map(|c| c.id).collect())
        .unwrap_or_default();
    for (slot, track) in seq.tracks() {
        for clip in track.clips() {
            let Some(attach) = clip.attach else { continue };
            if slot.is_main() || !seq.flags.follow_main || !owners.contains(&attach.owner) {
                out.push(InvariantViolation::Attachment { clip: clip.id });
            }
        }
    }
}

/// Groups and links have at least two members.
fn check_relations(seq: &Sequence, out: &mut Vec<InvariantViolation>) {
    let mut groups: BTreeMap<GroupId, usize> = BTreeMap::new();
    let mut links: BTreeMap<LinkId, usize> = BTreeMap::new();
    for (_, track) in seq.tracks() {
        for clip in track.clips() {
            if let Some(g) = clip.group {
                *groups.entry(g).or_default() += 1;
            }
            if let Some(l) = clip.link {
                *links.entry(l).or_default() += 1;
            }
        }
    }
    for (group, n) in groups {
        if n < 2 {
            out.push(InvariantViolation::SmallGroup { group });
        }
    }
    for (link, n) in links {
        if n < 2 {
            out.push(InvariantViolation::SmallLink { link });
        }
    }
}
