//! The edit operations. Each is a planner over a [`Tx`].

mod anim;
mod audio;
mod color;
mod conform;
mod cut;
mod delete;
mod follow;
mod insert;
mod keys;
mod library;
mod markers;
mod move_;
mod prop_keys;
mod props;
mod record;
mod split;
mod stream;
mod sync;
mod text;
mod track;
mod trim;

use std::collections::BTreeSet;
use std::sync::Arc;

pub use audio::{AudioEdit, linked_audio};
pub use color::{ColorEdit, ColorOp, has_color, is_color_prop};
pub use cut::{Cut, TrimTargets, Unavailable, trim_to_playhead_available};
pub use delete::DeleteReq;
pub use insert::{InsertReq, InsertTarget, NewClip};
pub use keys::GainKeyEdit;
pub use library::NewMedia;
pub use markers::{MarkerOwner, MarkerPatch, MarkerSelector};
pub use move_::MoveReq;
pub use prop_keys::KeyEdit;
pub use props::TransformEdit;
pub use record::{RecordTarget, first_free_audio_track, recorded_start};
pub use split::{SplitReq, SplitTargets};
pub use stream::stream_doubled_by;
pub use sync::{SyncOffset, sync_offset, sync_offsets};
pub use text::{ResetScope, TextLayoutEdit, TextStyleEdit, change_case};
pub use track::{AddTrackReq, TrackPosition};
pub use trim::{Edge, TrimReq};

pub(crate) use anim::set_clip_animation;
pub(crate) use audio::{detach_audio, restore_audio, set_clip_audio, set_master_gain};
pub(crate) use color::edit_color;
pub(crate) use cut::trim_to_playhead;
pub(crate) use delete::delete;
pub(crate) use follow::{set_follow_kinds, set_follow_main};
pub(crate) use insert::insert;
pub(crate) use keys::edit_gain_keys;
pub(crate) use library::{
    extract_audio, import_media, remove_media, set_project_settings, update_media,
};
pub(crate) use markers::{add_marker, delete_markers, edit_marker, move_marker};
pub(crate) use move_::move_clips;
pub(crate) use prop_keys::edit_keys;
pub(crate) use props::set_transform;
pub(crate) use record::insert_recorded_clip;
pub(crate) use split::split;
pub(crate) use stream::{add_stream_as_clip, set_audio_stream};
pub(crate) use sync::resync;
pub(crate) use text::{reset_text, set_text_content, set_text_layout, set_text_style};
pub(crate) use track::{
    add_track, remove_track, set_magnet, set_track_gain, set_track_header, set_track_pan,
};
pub(crate) use trim::trim;

use super::EditError;
use super::tx::Tx;
use crate::ids::{ClipId, SeqId};
use crate::model::{Clip, ClipLoc, Lane, TrackSlot};

pub(crate) use conform::conform;

/// A clip found in the working document.
#[derive(Debug, Clone)]
pub(super) struct Resolved {
    pub(super) loc: ClipLoc,
    pub(super) clip: Arc<Clip>,
}

/// Resolves clip ids in `seq`, deduplicated, in a deterministic order: by
/// track (visual bottom-up, then audio top-down), then by start. Uses the
/// document's clip index: O(k log n) for k ids.
pub(super) fn resolve_clips(
    tx: &Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
) -> Result<Vec<Resolved>, EditError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        if !seen.insert(id) {
            continue;
        }
        let loc = tx.locate(id)?;
        if loc.seq != seq {
            return Err(EditError::StaleClip(id));
        }
        out.push(Resolved {
            loc,
            clip: tx.clip(id)?,
        });
    }
    out.sort_by_key(|r| (lane_order(r.loc.slot), r.clip.start(), r.clip.id));
    Ok(out)
}

pub(super) fn lane_order(slot: TrackSlot) -> (u8, usize) {
    match slot.lane {
        Lane::Visual => (0, slot.index),
        Lane::Audio => (1, slot.index),
    }
}
