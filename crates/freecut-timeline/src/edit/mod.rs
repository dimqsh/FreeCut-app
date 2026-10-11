//! Planning edits.
//!
//! Every edit is a pure planner `(&Document, &EditCtx, request) -> Plan`. A
//! plan holds the primitive changes, the resulting document (for ghost
//! previews), and notices for the user. Nothing changes until
//! [`crate::Editor::commit`] installs the plan and records it in history.

mod change;
mod error;
mod follow;
mod ops;
mod policy;
mod tx;

use core::num::NonZeroU64;

pub(crate) use change::apply_all;
pub use change::{Change, ChangeSet};
pub use error::{ApplyError, EditError};
pub use ops::{
    AddTrackReq, AudioEdit, ColorEdit, ColorOp, Cut, DeleteReq, Edge, GainKeyEdit, InsertReq,
    InsertTarget, KeyEdit, MarkerOwner, MarkerPatch, MarkerSelector, MoveReq, NewClip, NewMedia,
    RecordTarget, ResetScope, SplitReq, SplitTargets, SyncOffset, TextLayoutEdit, TextStyleEdit,
    TrackPosition, TransformEdit, TrimReq, TrimTargets, Unavailable, change_case,
    first_free_audio_track, has_color, is_color_prop, linked_audio, recorded_start, stream_doubled_by,
    sync_offset, sync_offsets, trim_to_playhead_available,
};

use crate::ids::{ClipId, MediaId, SeqId, TrackId};
use crate::model::{
    AnimationSpec, Document, Fingerprint, FollowKinds, GainDb, MarkerRef, MediaInfo, PaletteColor,
    Pan, ProjectSettings, PropPath, RichText, Slot, StoredPath, StreamSel, TrackHeader,
};
use crate::time::Tick;

/// Structural limits (TL-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_tracks: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_tracks: 100 }
    }
}

/// Everything a planner needs besides the document. The frame rate and
/// media lengths come from the document itself (its settings and library).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditCtx {
    pub limits: Limits,
}

/// What an edit did, for menus ("Undo Split 3 clips") and the history panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpKind {
    AddTrack,
    RemoveTrack,
    SetTrackHeader,
    SetMagnet,
    InsertClips,
    MoveClips,
    TrimClip,
    SplitClips,
    DeleteClips,
    RippleDeleteClips,
    Conform,
    ImportMedia,
    RemoveMedia,
    RelinkMedia,
    UpdateMediaInfo,
    SetProjectSettings,
    DetachAudio,
    RestoreAudio,
    SetClipAudio,
    ExtractAudio,
    SetMasterGain,
    SetTextContent,
    SetTextStyle,
    SetTextLayout,
    ResetText,
    SetTransform,
    EditGainKeys,
    /// Keyframes of transform and text properties (TR-028, TX-021).
    EditKeys,
    /// In, Out and Loop animations (FX-018, FX-019, TX-022).
    SetClipAnimation,
    SetTrackGain,
    SetTrackPan,
    SetAudioStream,
    AddStreamAsClip,
    Resync,
    /// Cut away the part before (`Head`) or after (`Tail`) the playhead
    /// (TL-026): "Cut away before playhead" / "Cut away after playhead".
    TrimToPlayhead(Cut),
    AddMarker,
    EditMarker,
    MoveMarker,
    DeleteMarkers,
    /// *Follow main track* on or off (TL-029).
    SetFollowMain,
    /// The kinds that follow (TL-029).
    SetFollowKinds,
    /// A voiceover take placed on the timeline (AU-027).
    InsertRecordedClip,
    /// Colour sliders, switches, bypass or a reset (CL-007, CL-008,
    /// CL-023, CL-024, CL-035).
    EditColor(ColorOp),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label {
    pub op: OpKind,
    pub count: u32,
}

/// Side effects worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Switching the magnet on closed this many gaps (TL-005).
    ClosedGaps(u32),
    /// An item didn't fit and went onto a new track (TL-006).
    CreatedTrack(TrackId),
    /// Tracks emptied by the edit were removed (TL-006).
    RemovedEmptyTracks(u32),
    /// This extracted audio item kept its old file: the relinked original
    /// has no matching audio stream (AU-004). It can be relinked on its own.
    ExtractedAudioNotRelinked(MediaId),
    /// An imported item has more than one audio stream (AU-038): how many,
    /// and the container index of the one that plays by default.
    MultipleAudioStreams {
        media: MediaId,
        count: u32,
        default_stream: u32,
    },
    /// Move the playhead here (a head cut on the magnetic main track,
    /// TL-026).
    PlayheadTo(Tick),
    /// Clips the edit left alone, with the reason for each (TL-053).
    Skipped(Vec<(ClipId, Unavailable)>),
    /// Items attached to deleted main-track clips went with them (TL-025,
    /// TL-029).
    AlsoRemovedAttached(u32),
    /// The marker this edit added, to select (TL-037).
    AddedMarker(MarkerRef),
}

/// An edit request. One typed planner per variant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditRequest {
    AddTrack(AddTrackReq),
    RemoveTrack {
        seq: SeqId,
        track: TrackId,
    },
    SetTrackHeader {
        seq: SeqId,
        track: TrackId,
        header: TrackHeader,
    },
    SetMagnet {
        seq: SeqId,
        on: bool,
    },
    Insert(InsertReq),
    Move(MoveReq),
    Trim(TrimReq),
    Split(SplitReq),
    Delete(DeleteReq),
    /// Rounds every clip start and end onto the frame grid after a rate
    /// change (TL-001).
    Conform {
        seq: SeqId,
    },
    /// Adds probed files to the library (PM-030).
    ImportMedia(Vec<NewMedia>),
    /// Removes media; `with_clips` also deletes the clips using it (PM-058).
    RemoveMedia {
        media: Vec<MediaId>,
        with_clips: bool,
    },
    /// Points media at another file (PM-091).
    RelinkMedia {
        media: MediaId,
        path: StoredPath,
        info: MediaInfo,
        fingerprint: Option<Fingerprint>,
    },
    /// Records a corrected probe result, such as the exact length (PM-093).
    UpdateMediaInfo {
        media: MediaId,
        info: MediaInfo,
    },
    /// Changes the project settings, optionally conforming every sequence to
    /// the new frame grid (PM-004, PM-005, PM-011).
    SetProjectSettings {
        settings: ProjectSettings,
        conform: bool,
    },
    /// Moves the sound of video clips to new linked audio clips (TL-030,
    /// AU-003). Clips without active sound are skipped; refused when none
    /// has any. The new audio clips are selected afterward.
    DetachAudio {
        seq: SeqId,
        clips: Vec<ClipId>,
    },
    /// Gives detached video clips their sound back and removes the audio
    /// clips still linked to them (TL-030, AU-003). Clips without detached
    /// sound are skipped; refused when none has any. [`linked_audio`] lists
    /// what would be removed, for the confirmation.
    RestoreAudio {
        seq: SeqId,
        clips: Vec<ClipId>,
    },
    /// Changes one sound setting on every listed clip that plays its own
    /// sound (AU-005, AU-006, AU-007); others are skipped, and the edit is
    /// refused when none plays any.
    SetClipAudio {
        seq: SeqId,
        clips: Vec<ClipId>,
        edit: AudioEdit,
    },
    /// Adds a library item playing one audio stream of `media` (AU-004,
    /// PM-041); `None` takes the default stream. Extracting the same stream
    /// again changes nothing and reports the existing item in
    /// [`Plan::imported`].
    ExtractAudio {
        media: MediaId,
        stream: Option<u32>,
    },
    /// Sets a sequence's master volume (AU-010). Gains above +12 dB are
    /// lowered to +12 dB; setting the current value changes nothing.
    SetMasterGain {
        seq: SeqId,
        gain: GainDb,
    },
    /// Replaces the content of one text clip (TX-002). The app sends one
    /// per edit session, on commit; unchanged content changes nothing.
    /// Refused for a clip that isn't text.
    SetTextContent {
        seq: SeqId,
        clip: ClipId,
        content: RichText,
    },
    /// Changes one character-style value on every listed text clip (TX-003,
    /// TX-004, TX-008 to TX-016, TX-055), or switches an optional layer
    /// (TX-054); other clips are skipped, and the edit is refused when none
    /// is text.
    SetTextStyle {
        seq: SeqId,
        clips: Vec<ClipId>,
        edit: TextStyleEdit,
    },
    /// Changes one paragraph-layout value on every listed text clip
    /// (TX-005); other clips are skipped, and the edit is refused when none
    /// is text.
    SetTextLayout {
        seq: SeqId,
        clips: Vec<ClipId>,
        edit: TextLayoutEdit,
    },
    /// **Reset card** / **Reset lens** (TX-054): the scope's properties go
    /// back to the factory default text style on every listed text clip, as
    /// one entry. Content and the optional layers' on/off states are kept.
    /// Other clips are skipped; refused when none is text.
    ResetText {
        seq: SeqId,
        clips: Vec<ClipId>,
        scope: ResetScope,
    },
    /// Changes one transform value on every listed visual clip (TR-001,
    /// TR-003, TR-015); audio clips are skipped, and the edit is refused
    /// when none is visual.
    SetTransform {
        seq: SeqId,
        clips: Vec<ClipId>,
        edit: TransformEdit,
    },
    /// Adds, changes, moves or removes volume keyframes (AU-008, TR-027) on
    /// every listed clip that plays its own sound; see [`GainKeyEdit`] for
    /// which clips each edit skips. One undo entry.
    EditGainKeys {
        seq: SeqId,
        clips: Vec<ClipId>,
        edit: GainKeyEdit,
    },
    /// Adds, removes, moves or re-eases keyframes of a transform or text
    /// property, or keys or clears a whole card (TR-028 to TR-031, TX-021),
    /// on every listed clip that has the property; see [`KeyEdit`] for
    /// which clips each edit skips. `prop` is ignored by the card edits.
    /// One undo entry ("Key this card" included).
    EditKeys {
        seq: SeqId,
        clips: Vec<ClipId>,
        prop: PropPath,
        edit: KeyEdit,
    },
    /// Chooses, changes or clears (`None`) the In, Out or Loop animation of
    /// every listed visual clip (FX-018, FX-019, TX-022), with the slot
    /// policy of each clip's kind; audio clips are skipped, and the edit is
    /// refused when none is visual. Lengths are snapped and limited as
    /// FX-018 says; with Pace on a text clip the length comes from the
    /// text.
    SetClipAnimation {
        seq: SeqId,
        clips: Vec<ClipId>,
        slot: Slot,
        spec: Option<AnimationSpec>,
    },
    /// Sets a track's volume (AU-013): −∞ or −60 to +12 dB; louder values
    /// become +12 dB. Refused on tracks that carry no sound; setting the
    /// current value changes nothing.
    SetTrackGain {
        seq: SeqId,
        track: TrackId,
        gain: GainDb,
    },
    /// Sets a track's pan (AU-013). As [`EditRequest::SetTrackGain`].
    SetTrackPan {
        seq: SeqId,
        track: TrackId,
        pan: Pan,
    },
    /// Chooses the audio stream (or all streams mixed) of every listed media
    /// clip that plays its own sound (AU-038); `None` goes back to the
    /// media's default stream. Clips whose sound is detached or whose media
    /// lacks the stream are skipped; refused when none is left, with the
    /// reason.
    SetAudioStream {
        seq: SeqId,
        clips: Vec<ClipId>,
        sel: Option<StreamSel>,
    },
    /// Puts audio stream `stream` of a video clip's file on its own audio
    /// clip, linked to the video (AU-038). Refused for the stream the clip
    /// already plays (offer Detach audio instead) and for media with a
    /// single audio stream. The new clip is selected afterward.
    AddStreamAsClip {
        seq: SeqId,
        clip: ClipId,
        stream: u32,
    },
    /// Moves out-of-sync linked audio clips back in sync with their video
    /// (TL-030); see [`sync_offset`]. Listed video clips resync their audio
    /// partners. Refused when nothing listed is out of sync.
    Resync {
        seq: SeqId,
        clips: Vec<ClipId>,
    },
    /// Cuts away the part of each target before or after `at` (TL-026); see
    /// [`trim_to_playhead_available`] for when it can act.
    TrimToPlayhead {
        seq: SeqId,
        targets: TrimTargets,
        at: Tick,
        cut: Cut,
        /// `Shift`: ripple the targets' tracks off the magnetic main track.
        ripple_others: bool,
    },
    /// Adds a marker at `at` (TL-037); `color: None` is Blue.
    AddMarker {
        seq: SeqId,
        at: Tick,
        owner: MarkerOwner,
        color: Option<PaletteColor>,
    },
    /// Changes a marker's name, note, colour or duration (TL-037).
    EditMarker {
        seq: SeqId,
        marker: MarkerRef,
        patch: MarkerPatch,
    },
    /// Moves a marker to start at sequence time `to` (TL-037).
    MoveMarker {
        seq: SeqId,
        marker: MarkerRef,
        to: Tick,
    },
    /// Deletes markers (TL-037).
    DeleteMarkers {
        seq: SeqId,
        which: MarkerSelector,
    },
    /// *Follow main track* on or off (TL-029).
    SetFollowMain {
        seq: SeqId,
        on: bool,
    },
    /// Which kinds of items follow (TL-029).
    SetFollowKinds {
        seq: SeqId,
        kinds: FollowKinds,
    },
    /// Imports a recorded take and places it (AU-027); see
    /// [`recorded_start`] for `at`. `handle_in` is the pre-roll kept before
    /// the in-point; `replace` is the take *Redo last take* removes.
    InsertRecordedClip {
        seq: SeqId,
        media: NewMedia,
        at: Tick,
        handle_in: Tick,
        target: RecordTarget,
        replace: Option<ClipId>,
    },
    /// Changes the colour stacks of every listed clip that takes one
    /// (video, images, adjustment clips; CL-001); see [`ColorEdit`]. Other
    /// clips are skipped; refused when none takes one. One undo entry; a
    /// reset removes the keys it resets in the same entry (CL-024).
    EditColor {
        seq: SeqId,
        clips: Vec<ClipId>,
        edit: ColorEdit,
    },
}

/// Which editor state a plan was made against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanStamp {
    pub(crate) editor: u64,
    pub(crate) revision: u64,
}

/// A planned edit, ready to preview or commit. Read-only: what gets
/// recorded in history is exactly what produced the previewed document.
///
/// A plan's changes can't be swapped before committing:
///
/// ```compile_fail
/// fn tamper(plan: &mut freecut_timeline::edit::Plan) {
///     plan.changes = Default::default();
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Plan {
    label: Label,
    changes: ChangeSet,
    notices: Vec<Notice>,
    /// The selection after the edit, when the edit decides it (TL-015).
    selection_after: Option<Vec<ClipId>>,
    result: Document,
    imported: Vec<MediaId>,
    pub(crate) id_high_water: NonZeroU64,
    pub(crate) stamp: Option<PlanStamp>,
}

impl Plan {
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn label(&self) -> Label {
        self.label
    }

    pub fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    pub fn selection_after(&self) -> Option<&[ClipId]> {
        self.selection_after.as_deref()
    }

    /// Media items this plan adds to the library, in request order. For an
    /// extract that reuses an existing item, that item.
    pub fn imported(&self) -> &[MediaId] {
        &self.imported
    }

    /// The document as it would be after this edit, for ghost previews.
    pub fn preview(&self) -> &Document {
        &self.result
    }

    pub(crate) fn new(
        label: Label,
        changes: ChangeSet,
        notices: Vec<Notice>,
        selection_after: Option<Vec<ClipId>>,
        result: Document,
        imported: Vec<MediaId>,
        id_high_water: NonZeroU64,
    ) -> Plan {
        Plan {
            label,
            changes,
            notices,
            selection_after,
            result,
            imported,
            id_high_water,
            stamp: None,
        }
    }

    pub(crate) fn into_result(self) -> (Document, Label, ChangeSet, Option<Vec<ClipId>>) {
        (self.result, self.label, self.changes, self.selection_after)
    }
}

/// Plans `req` against `doc`. `ids` is where new ids start (the editor's
/// allocator position). Only [`crate::Editor`] and [`crate::Planner`] call
/// this, so every plan carries the stamp of the state it was made against.
pub(crate) fn plan(
    doc: &Document,
    ctx: &EditCtx,
    ids: crate::ids::IdCursor,
    req: &EditRequest,
) -> Result<Plan, EditError> {
    let tx = tx::Tx::new(doc, ctx, ids);
    match req {
        EditRequest::AddTrack(r) => ops::add_track(tx, r),
        EditRequest::RemoveTrack { seq, track } => ops::remove_track(tx, *seq, *track),
        EditRequest::SetTrackHeader { seq, track, header } => {
            ops::set_track_header(tx, *seq, *track, header)
        }
        EditRequest::SetMagnet { seq, on } => ops::set_magnet(tx, *seq, *on),
        EditRequest::Insert(r) => ops::insert(tx, r),
        EditRequest::Move(r) => ops::move_clips(tx, r),
        EditRequest::Trim(r) => ops::trim(tx, r),
        EditRequest::Split(r) => ops::split(tx, r),
        EditRequest::Delete(r) => ops::delete(tx, r),
        EditRequest::Conform { seq } => ops::conform(tx, *seq),
        EditRequest::ImportMedia(items) => ops::import_media(tx, items),
        EditRequest::RemoveMedia { media, with_clips } => ops::remove_media(tx, media, *with_clips),
        EditRequest::RelinkMedia {
            media,
            path,
            info,
            fingerprint,
        } => ops::update_media(tx, *media, Some(path), info, *fingerprint),
        EditRequest::UpdateMediaInfo { media, info } => {
            ops::update_media(tx, *media, None, info, None)
        }
        EditRequest::SetProjectSettings { settings, conform } => {
            ops::set_project_settings(tx, settings, *conform)
        }
        EditRequest::DetachAudio { seq, clips } => ops::detach_audio(tx, *seq, clips),
        EditRequest::RestoreAudio { seq, clips } => ops::restore_audio(tx, *seq, clips),
        EditRequest::SetClipAudio { seq, clips, edit } => {
            ops::set_clip_audio(tx, *seq, clips, *edit)
        }
        EditRequest::ExtractAudio { media, stream } => ops::extract_audio(tx, *media, *stream),
        EditRequest::SetMasterGain { seq, gain } => ops::set_master_gain(tx, *seq, *gain),
        EditRequest::SetTextContent { seq, clip, content } => {
            ops::set_text_content(tx, *seq, *clip, content)
        }
        EditRequest::SetTextStyle { seq, clips, edit } => {
            ops::set_text_style(tx, *seq, clips, edit)
        }
        EditRequest::SetTextLayout { seq, clips, edit } => {
            ops::set_text_layout(tx, *seq, clips, *edit)
        }
        EditRequest::ResetText { seq, clips, scope } => ops::reset_text(tx, *seq, clips, *scope),
        EditRequest::SetTransform { seq, clips, edit } => {
            ops::set_transform(tx, *seq, clips, *edit)
        }
        EditRequest::EditGainKeys { seq, clips, edit } => {
            ops::edit_gain_keys(tx, *seq, clips, edit)
        }
        EditRequest::EditKeys {
            seq,
            clips,
            prop,
            edit,
        } => ops::edit_keys(tx, *seq, clips, *prop, *edit),
        EditRequest::SetClipAnimation {
            seq,
            clips,
            slot,
            spec,
        } => ops::set_clip_animation(tx, *seq, clips, *slot, spec.as_ref()),
        EditRequest::SetTrackGain { seq, track, gain } => {
            ops::set_track_gain(tx, *seq, *track, *gain)
        }
        EditRequest::SetTrackPan { seq, track, pan } => ops::set_track_pan(tx, *seq, *track, *pan),
        EditRequest::SetAudioStream { seq, clips, sel } => {
            ops::set_audio_stream(tx, *seq, clips, *sel)
        }
        EditRequest::AddStreamAsClip { seq, clip, stream } => {
            ops::add_stream_as_clip(tx, *seq, *clip, *stream)
        }
        EditRequest::Resync { seq, clips } => ops::resync(tx, *seq, clips),
        EditRequest::TrimToPlayhead {
            seq,
            targets,
            at,
            cut,
            ripple_others,
        } => ops::trim_to_playhead(tx, *seq, targets, *at, *cut, *ripple_others),
        EditRequest::AddMarker {
            seq,
            at,
            owner,
            color,
        } => ops::add_marker(tx, *seq, *at, owner, *color),
        EditRequest::EditMarker { seq, marker, patch } => {
            ops::edit_marker(tx, *seq, *marker, patch)
        }
        EditRequest::MoveMarker { seq, marker, to } => ops::move_marker(tx, *seq, *marker, *to),
        EditRequest::DeleteMarkers { seq, which } => ops::delete_markers(tx, *seq, which),
        EditRequest::SetFollowMain { seq, on } => ops::set_follow_main(tx, *seq, *on),
        EditRequest::SetFollowKinds { seq, kinds } => ops::set_follow_kinds(tx, *seq, *kinds),
        EditRequest::InsertRecordedClip {
            seq,
            media,
            at,
            handle_in,
            target,
            replace,
        } => ops::insert_recorded_clip(tx, *seq, media, *at, *handle_in, *target, *replace),
        EditRequest::EditColor { seq, clips, edit } => ops::edit_color(tx, *seq, clips, edit),
    }
}
