//! The edit document: sequences of tracks of clips.
//!
//! The document is `Arc`-structured so that cloning it is a handful of pointer
//! copies and an edit copies only the tracks it touches. There are no public
//! mutators: every change goes through [`crate::Editor`].

mod anim;
mod animate;
mod apply;
mod audio;
mod clip;
mod color;
mod follow;
mod keys;
mod library;
mod marker;
pub mod mask;
mod sequence;
mod settings;
mod text;
mod track;
mod transform;
mod validate;
mod values;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub use anim::{
    ActiveSlot, AnimOrder, AnimParams, AnimUnit, AnimWindows, AnimationSpec, ClipAnimations,
    Direction, LoopSeams, PresetId, Slot, SlotPolicy, SlotWindow, Travel, slot_policy,
};
pub use animate::{
    Animatable, EaseCurve, EaseDir, Interp, Interpolate, Keyframe, KeyframeError, Progress,
    Segment, lerp_i64,
};
pub use audio::{ClipAudio, EmbeddedAudio, FadeCurve, GainDb, MasterAudio, Pan, TrackAudio};
pub use clip::{Clip, ClipContent, GeneratedKind, MediaKind, StreamSel};
pub use color::{
    AutoCorrection, AutoOffsets, AutoSource, AutoStyle, BasicAdjust, ColorMask, ColorMaskParam,
    ColorPage, ColorParamId, ColorParamSpec, ColorStack, ColorWheels, Curve, CurveChannel,
    CurveError, CurvePoint, Curves, EXPAND_LIMIT, Hsl, HslBand, HslChannel, HueBand, LutInterp,
    LutRef, LutSlot, MATCH_GAIN_MAX, MATCH_GAIN_MIN, MATCH_OFFSET_LIMIT, MATCH_ONE,
    MAX_COLOR_MASKS, MatchRef, MatchTransform, PUCK_RADIUS, SKIN_AMOUNT_DEFAULT, Section,
    SectionMask, SectionSnapshot, SkinProtect, SkinScope, Wheel, WheelId, color_param_spec,
    hash_hex, on_disk, parse_hash_hex, scaled_offset,
};
pub use follow::{
    Attachment, FollowKind, FollowKinds, attach_all, attachment_for, expected_start, follow_kind,
};
pub use keys::{
    ClipKeys, KeyTrack, KeysError, PropGroup, PropPath, PropValue, TransformProp, ValueKind,
};
pub use library::{
    AudioStreamInfo, Fingerprint, GeneratedMediaKind, Library, MediaInfo, MediaItem, MediaOrigin,
    MediaSource, Rotation, StoredPath, VideoStreamInfo,
};
pub(crate) use marker::marker_points_except;
pub use marker::{
    MAX_MARKER_NAME_CHARS, MAX_MARKER_NOTE_CHARS, Marker, MarkerOrigin, MarkerRef, MarkerScope,
    Markers, PaletteColor, next_marker, prev_marker,
};
pub use mask::{
    MASK_BAND_MAX, MASK_CENTER_LIMIT, MASK_FEATHER_SPAN, MASK_RATIO_MAX, MASK_ROTATION_LIMIT,
    MASK_SIZE_MAX, MASK_UNIT, Mask, MaskCombine, MaskParam, MaskPoint, MaskShape, MaskShapeKind,
    MaskValue,
};
pub use sequence::{Lane, Sequence, SequenceFlags, TrackSlot};
pub use settings::{MAX_CANVAS_SIDE, ProjectSettings};
pub use text::{
    Border, BoxHeight, BoxShape, BubbleKind, BubbleSpec, CaseTransform, DEFAULT_TEXT_DURATION,
    DeciDegrees, Decoration, Fill, FillCenter, FillExtent, FillKind, FontRef, Glow, GlowDirection,
    GradientError, GradientStop, GradientStops, HAlign, LangTag, Layer, LinearGradient,
    RadialGradient, RefPx, Rgba8, RichText, Shadow, Side, SignedPerMille, Stroke, StrokePosition,
    Tail, TailTip, TextBox, TextBubble, TextCard, TextClipData, TextLayerKind, TextLayout,
    TextLens, TextOrigin, TextPropPath, TextStyle, VAnchor, card_props, case,
    default_text_duration,
};
pub use track::{Band, Track, TrackFlags, TrackHeader, TrackKind};
pub use transform::{Angle, CanvasPos, PerMille, Scale, Transform};
pub(crate) use validate::Scope;
pub use validate::{InvariantViolation, ValidateCtx};

use crate::ids::{ClipId, SeqId, TrackId};
use crate::time::Tick;

/// Where a clip lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipLoc {
    pub seq: SeqId,
    pub track: TrackId,
    pub slot: TrackSlot,
}

/// A derived index entry: enough to find a clip with one binary search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndexEntry {
    pub(crate) seq: SeqId,
    pub(crate) track: TrackId,
    pub(crate) start: Tick,
}

/// The whole editable timeline state of a project.
#[derive(Debug, Clone)]
pub struct Document {
    root: SeqId,
    settings: ProjectSettings,
    library: Arc<Library>,
    sequences: BTreeMap<SeqId, Arc<Sequence>>,
    /// Derived from `sequences`; never part of equality. Only lookups go
    /// through it, so iteration order never reaches output.
    index: Arc<HashMap<ClipId, IndexEntry>>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.settings == other.settings
            && self.library == other.library
            && self.sequences == other.sequences
    }
}

impl Eq for Document {}

impl Document {
    /// A document with an empty library and one empty root sequence.
    pub fn new(root: SeqId, main_track: TrackId, settings: ProjectSettings) -> Document {
        let mut sequences = BTreeMap::new();
        sequences.insert(root, Arc::new(Sequence::new(root, main_track)));
        Document {
            root,
            settings,
            library: Arc::default(),
            sequences,
            index: Arc::default(),
        }
    }

    /// Builds a document from stored parts (project loading). Refuses a
    /// missing root and repeated sequence or clip ids; every other invariant
    /// is checked by [`Document::validate`], which a loader runs next.
    pub fn from_parts(
        root: SeqId,
        settings: ProjectSettings,
        library: Library,
        sequences: Vec<Arc<Sequence>>,
    ) -> Result<Document, InvariantViolation> {
        let mut map = BTreeMap::new();
        for seq in sequences {
            let id = seq.id;
            if map.insert(id, seq).is_some() {
                return Err(InvariantViolation::DuplicateId(id.get()));
            }
        }
        if !map.contains_key(&root) {
            return Err(InvariantViolation::RootMissing(root));
        }
        let mut index = HashMap::new();
        for seq in map.values() {
            for (_, track) in seq.tracks() {
                for clip in track.clips() {
                    let entry = IndexEntry {
                        seq: seq.id,
                        track: track.id,
                        start: clip.start(),
                    };
                    if index.insert(clip.id, entry).is_some() {
                        return Err(InvariantViolation::DuplicateId(clip.id.get()));
                    }
                }
            }
        }
        Ok(Document {
            root,
            settings,
            library: Arc::new(library),
            sequences: map,
            index: Arc::new(index),
        })
    }

    pub const fn root(&self) -> SeqId {
        self.root
    }

    pub const fn settings(&self) -> &ProjectSettings {
        &self.settings
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn sequence(&self, id: SeqId) -> Option<&Sequence> {
        self.sequences.get(&id).map(|s| &**s)
    }

    pub fn sequences(&self) -> impl Iterator<Item = &Sequence> {
        self.sequences.values().map(|s| &**s)
    }

    /// Number of clips in every sequence.
    pub fn clip_count(&self) -> usize {
        self.index.len()
    }

    /// Where clip `id` lives. Constant time plus a scan of one sequence's
    /// track list.
    pub fn find_clip(&self, id: ClipId) -> Option<ClipLoc> {
        crate::probe::visited(1);
        let entry = self.index.get(&id)?;
        let (slot, _) = self.sequence(entry.seq)?.find_track(entry.track)?;
        Some(ClipLoc {
            seq: entry.seq,
            track: entry.track,
            slot,
        })
    }

    /// Clip `id`, found through the index and one binary search.
    pub fn clip(&self, id: ClipId) -> Option<&Arc<Clip>> {
        crate::probe::visited(1);
        let entry = self.index.get(&id)?;
        let (_, track) = self.sequence(entry.seq)?.find_track(entry.track)?;
        let i = track.index_of(entry.start, id)?;
        track.clips().get(i)
    }

    /// The largest id in use: media, sequences, tracks, clips, groups,
    /// links and markers (0 for none), including the parent an extracted
    /// item names. An id allocator must start above it.
    pub fn max_id(&self) -> u64 {
        let mut max = 0;
        for item in self.library.items() {
            max = max.max(item.id.get());
            // The parent may be gone; a new item under its id would be
            // taken for it (and relinked with it).
            if let MediaSource::AudioOf { source, .. } = item.source {
                max = max.max(source.get());
            }
        }
        for seq in self.sequences() {
            max = max.max(seq.id.get());
            for m in &seq.markers {
                max = max.max(m.id.get());
            }
            for (_, track) in seq.tracks() {
                max = max.max(track.id.get());
                for clip in track.clips() {
                    max = max.max(clip.id.get());
                    max = max.max(clip.group.map_or(0, |g| g.get()));
                    max = max.max(clip.link.map_or(0, |l| l.get()));
                    // A detached video keeps its link id after the link
                    // dissolves; it must never be handed out again.
                    max = max.max(clip.audio.detached().map_or(0, |l| l.get()));
                    // So does the clip or media a colour match names.
                    max = max.max(clip.match_reference_id().unwrap_or(0));
                    for m in &clip.markers {
                        max = max.max(m.id.get());
                    }
                }
            }
        }
        max
    }

    pub(crate) fn index_entry(&self, id: ClipId) -> Option<IndexEntry> {
        self.index.get(&id).copied()
    }

    pub(crate) fn index_mut(&mut self) -> &mut HashMap<ClipId, IndexEntry> {
        Arc::make_mut(&mut self.index)
    }

    pub(crate) fn settings_mut(&mut self) -> &mut ProjectSettings {
        &mut self.settings
    }

    pub(crate) fn library_mut(&mut self) -> &mut Library {
        Arc::make_mut(&mut self.library)
    }

    pub(crate) fn sequence_mut(&mut self, id: SeqId) -> Option<&mut Sequence> {
        self.sequences.get_mut(&id).map(Arc::make_mut)
    }
}
