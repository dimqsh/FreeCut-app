//! Attached items (TL-029): items over a main-track clip belong to it and
//! follow it while the sequence's *Follow main track* flag is on. Pure data
//! and queries; the ops apply them (`edit::follow`).

use super::clip::{Clip, ClipContent, MediaKind};
use super::library::{GeneratedMediaKind, Library, MediaOrigin, MediaSource};
use super::sequence::{Sequence, TrackSlot};
use super::track::{Track, TrackKind};
use crate::ids::ClipId;
use crate::time::Tick;

/// What an item hangs on: the main-track clip under its start and the
/// content frame of that clip it starts on (source time for media).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Attachment {
    pub owner: ClipId,
    pub owner_source_time: Tick,
}

/// Which kinds of items follow (TL-029's *Item kinds…* list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FollowKind {
    /// Clips and images on non-main video tracks.
    VideoOverlay,
    Text,
    Sticker,
    Effect,
    /// Filters and adjustments.
    Adjustment,
    Music,
    SoundEffect,
    /// Voiceover recordings and generated speech.
    Speech,
    /// Detached, extracted and other-origin audio.
    OtherAudio,
}

impl FollowKind {
    pub const ALL: [FollowKind; 9] = [
        FollowKind::VideoOverlay,
        FollowKind::Text,
        FollowKind::Sticker,
        FollowKind::Effect,
        FollowKind::Adjustment,
        FollowKind::Music,
        FollowKind::SoundEffect,
        FollowKind::Speech,
        FollowKind::OtherAudio,
    ];

    const fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// A set of [`FollowKind`]s. The default is every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FollowKinds(u16);

impl FollowKinds {
    pub const NONE: FollowKinds = FollowKinds(0);
    pub const ALL: FollowKinds = FollowKinds((1 << FollowKind::ALL.len()) - 1);

    /// From stored bits; unknown bits are dropped.
    pub const fn from_bits(bits: u16) -> FollowKinds {
        FollowKinds(bits & FollowKinds::ALL.0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, kind: FollowKind) -> bool {
        self.0 & kind.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The set with `kind` checked or unchecked.
    pub const fn with(self, kind: FollowKind, on: bool) -> FollowKinds {
        if on {
            FollowKinds(self.0 | kind.bit())
        } else {
            FollowKinds(self.0 & !kind.bit())
        }
    }

    pub fn iter(self) -> impl Iterator<Item = FollowKind> {
        FollowKind::ALL
            .into_iter()
            .filter(move |k| self.contains(*k))
    }
}

impl Default for FollowKinds {
    fn default() -> Self {
        FollowKinds::ALL
    }
}

impl FromIterator<FollowKind> for FollowKinds {
    fn from_iter<I: IntoIterator<Item = FollowKind>>(iter: I) -> Self {
        iter.into_iter()
            .fold(FollowKinds::NONE, |set, k| set.with(k, true))
    }
}

/// The follow kind of `clip` on `track`. Audio reads the media item's origin
/// (PM-041) as plain data: voiceovers and generated speech are `Speech`;
/// everything else is `OtherAudio` until AU-001's music and sound-effect
/// origins exist.
pub fn follow_kind(clip: &Clip, track: &Track, library: &Library) -> FollowKind {
    match track.kind {
        TrackKind::Video => FollowKind::VideoOverlay,
        TrackKind::Text => FollowKind::Text,
        TrackKind::Sticker => FollowKind::Sticker,
        TrackKind::Effect => FollowKind::Effect,
        TrackKind::Adjustment => FollowKind::Adjustment,
        TrackKind::Audio => {
            let item = match &clip.content {
                ClipContent::Media {
                    asset,
                    kind: MediaKind::Audio,
                    ..
                } => library.get(*asset),
                _ => None,
            };
            match item {
                Some(m)
                    if m.source == MediaSource::File
                        && matches!(
                            m.origin,
                            MediaOrigin::Generated {
                                kind: GeneratedMediaKind::Voiceover | GeneratedMediaKind::Tts
                            }
                        ) =>
                {
                    FollowKind::Speech
                }
                _ => FollowKind::OtherAudio,
            }
        }
    }
}

/// The attachment `clip` on `track` (at `slot`) gets from its position in
/// `seq`: the main-track clip under its start, while following is on and its
/// kind is checked. `None` for main-track clips and items over nothing.
pub fn attachment_for(
    seq: &Sequence,
    slot: TrackSlot,
    track: &Track,
    clip: &Clip,
    library: &Library,
) -> Option<Attachment> {
    let flags = seq.flags;
    if !flags.follow_main || slot.is_main() {
        return None;
    }
    if !flags
        .follow_kinds
        .contains(follow_kind(clip, track, library))
    {
        return None;
    }
    let owner = seq.main_track()?.clip_at(clip.start())?;
    Some(Attachment {
        owner: owner.id,
        owner_source_time: owner.content_time_at(clip.start())?,
    })
}

/// Every non-main clip of `seq` with the attachment its position gives it
/// (TL-029, pure): what turning following on computes.
pub fn attach_all(seq: &Sequence, library: &Library) -> Vec<(ClipId, Option<Attachment>)> {
    seq.tracks()
        .filter(|(slot, _)| !slot.is_main())
        .flat_map(|(slot, track)| {
            track
                .clips()
                .iter()
                .map(move |c| (c.id, attachment_for(seq, slot, track, c, library)))
        })
        .collect()
}

/// Where an attached item should start: its owner's position of the anchor
/// frame. `None` when the owner is gone or off the main track.
pub fn expected_start(seq: &Sequence, attach: Attachment) -> Option<Tick> {
    let owner = seq
        .main_track()?
        .clips()
        .iter()
        .find(|c| c.id == attach.owner)?;
    owner.sequence_time_of(attach.owner_source_time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_sets_check_and_uncheck() {
        let all = FollowKinds::default();
        assert_eq!(all, FollowKinds::ALL);
        assert!(FollowKind::ALL.iter().all(|k| all.contains(*k)));
        let no_text = all.with(FollowKind::Text, false);
        assert!(!no_text.contains(FollowKind::Text));
        assert!(no_text.contains(FollowKind::Sticker));
        assert_eq!(no_text.iter().count(), 8);
        assert_eq!(FollowKinds::from_bits(u16::MAX), FollowKinds::ALL);
        let two: FollowKinds = [FollowKind::Music, FollowKind::Speech]
            .into_iter()
            .collect();
        assert_eq!(
            two.iter().collect::<Vec<_>>(),
            vec![FollowKind::Music, FollowKind::Speech]
        );
        assert!(FollowKinds::NONE.is_empty());
    }
}
