//! Edit errors.

use crate::ids::{ClipId, MarkerId, MediaId, SeqId, TrackId};
use crate::model::{InvariantViolation, Section};
use crate::time::{Tick, TimeError};

/// Why a planned edit was refused. A refused edit leaves the document
/// untouched. The app maps these to user-facing messages.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    #[error("track is locked")]
    TrackLocked { track: TrackId },
    #[error("the main track is locked")]
    MainTrackLocked,
    #[error("would overlap another clip")]
    Collision { track: TrackId },
    #[error("would start before 0")]
    BeforeZero,
    #[error("this item can't go on that track")]
    KindMismatch { track: TrackId },
    #[error("the source media ends here")]
    SourceLimit { clip: ClipId },
    #[error("the requested range is outside the media")]
    SourceOutOfRange,
    #[error("these items need different kinds of track")]
    MixedKinds,
    #[error("media {0} isn't in the project")]
    UnknownMedia(MediaId),
    #[error("media {0} is still used on the timeline")]
    MediaInUse(MediaId),
    #[error("the canvas size is invalid")]
    InvalidCanvas,
    #[error("a clip must be at least one frame long")]
    TooShort,
    #[error("track limit of {max} reached")]
    TrackLimit { max: usize },
    #[error("there is no track there")]
    TrackOutOfRange,
    #[error("the main track can't be removed")]
    MainTrackRemoval,
    #[error("selected main-track clips must be adjacent")]
    NonContiguousMain,
    #[error("nothing to edit")]
    NothingSelected,
    #[error("this video has no sound")]
    NoSound(MediaId),
    #[error("media {media} has no audio stream {stream}")]
    UnknownStream { media: MediaId, stream: u32 },
    #[error("none of the selected clips has sound to detach")]
    NothingToDetach,
    #[error("none of the selected clips has detached sound")]
    NothingToRestore,
    #[error("none of the selected clips has sound")]
    NoClipAudio,
    #[error("a fade can't be negative")]
    NegativeFade,
    #[error("none of the selected clips is a text clip")]
    NoTextClip,
    #[error("none of the selected clips shows a picture")]
    NoVisualClip,
    /// CL-001: only video, images and adjustment clips take colour
    /// adjustments.
    #[error("none of the selected clips takes colour adjustments")]
    NoColorClip,
    /// CL-019: **Apply to sliders** would change the picture, because this
    /// section (Auto, or Light or Color taking an offset) is switched off.
    #[error("switch the {section:?} section on to apply Auto to the sliders")]
    SectionSwitchedOff { section: Section },
    #[error("the volume has keyframes: change the keyframes instead")]
    VolumeKeyframed,
    #[error("none of the selected clips has a volume keyframe at {at:?}")]
    NoKeyframe { at: Tick },
    /// No listed clip has a keyframe of the property at that time (TR-028,
    /// TR-029).
    #[error("none of the selected clips has a keyframe there")]
    NoKey { at: Tick },
    #[error("this property can't take keyframes")]
    NotAnimatable,
    #[error("the keyframe value doesn't fit the property")]
    WrongValueKind,
    /// TR-028: diamonds work only on clips under the playhead.
    #[error("move the playhead over the clip")]
    NotUnderPlayhead,
    #[error("this track carries no sound")]
    TrackHasNoSound { track: TrackId },
    #[error("the sound of the selected clips is detached")]
    SoundDetached,
    #[error("this clip isn't a video clip")]
    NotVideo { clip: ClipId },
    #[error("media {0} has only one audio stream")]
    SingleStream(MediaId),
    /// The clip already plays this stream: offer Detach audio instead
    /// (AU-038).
    #[error("stream {stream} is already playing in this clip")]
    StreamAlreadyPlaying { clip: ClipId, stream: u32 },
    /// Choosing that stream for `clip` would play container stream `stream`
    /// twice: clip `by`, linked to it and overlapping it in time, already
    /// plays it (AU-038).
    #[error("stream {stream} already plays on a linked clip")]
    StreamPlaysElsewhere {
        clip: ClipId,
        stream: u32,
        by: ClipId,
    },
    #[error("none of the selected clips is out of sync")]
    NothingToResync,
    /// No target of a cut-away has the playhead inside it (TL-026).
    #[error("move the playhead inside the clip")]
    NothingToTrim,
    #[error("marker {0} no longer exists")]
    StaleMarker(MarkerId),
    #[error("time value out of range")]
    Time(#[from] TimeError),
    #[error("sequence {0} no longer exists")]
    StaleSequence(SeqId),
    #[error("track {0} no longer exists")]
    StaleTrack(TrackId),
    #[error("clip {0} no longer exists")]
    StaleClip(ClipId),
    #[error("no more ids")]
    IdSpaceExhausted,
    /// A planner produced a change that doesn't apply: a bug.
    #[error("internal edit error: {0}")]
    Internal(ApplyError),
    /// A planner produced an invalid document: a bug.
    #[error("internal invariant violated: {0:?}")]
    Invariant(Vec<InvariantViolation>),
}

/// A primitive change didn't match the document it was applied to. Always a
/// bug in a planner or in history bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("sequence {0} is missing")]
    SequenceMissing(SeqId),
    #[error("track {0} is missing")]
    TrackMissing(TrackId),
    #[error("track {0} already exists")]
    TrackExists(TrackId),
    #[error("clip {0} is missing")]
    ClipMissing(ClipId),
    #[error("clip {0} already exists")]
    ClipExists(ClipId),
    #[error("media {0} is missing")]
    MediaMissing(MediaId),
    #[error("media {0} already exists")]
    MediaExists(MediaId),
    #[error("the recorded 'before' value doesn't match the document")]
    BeforeMismatch,
    #[error("a replace changed the clip id")]
    IdMismatch,
    #[error("track slot is out of range")]
    SlotOutOfRange,
    #[error(transparent)]
    Time(#[from] TimeError),
}
