//! Load-time repairs (PM-013, TL-004). Pure functions over decoded parts.
//!
//! Repairs fix what can be fixed without guessing at the user's intent, and
//! each one is reported. A repaired project opens with unsaved changes; the
//! repair itself is not an undoable edit (README §5.11).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use freecut_timeline::ids::{ClipId, GroupId, LinkId, TrackId};
use freecut_timeline::model::{Clip, ClipContent, MediaKind, SequenceFlags, Track, TrackKind};

/// What [`repair_main_track`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MainRepair {
    /// The lowest video track, found at `from`, was moved to index 0, and
    /// the other video tracks moved down after it.
    Promoted { track: TrackId, from: usize },
    /// There was no video track; an empty one was inserted at index 0.
    Created { track: TrackId },
}

/// Makes `visual[0]` a video track (the main track, README C31), or inserts
/// an empty one with an id from `fresh` when there is none.
///
/// Promotion moves the whole video band down, not just the lowest video
/// track: the video tracks go first and the upper-band tracks (text,
/// stickers, effects, adjustments) after them, each group in its own order.
/// Moving one track alone could leave another video track above an upper
/// track, which breaks the band order (TL-004) and gets the file refused.
/// Returns `None` when nothing was wrong, and `Err` when `fresh` has no id
/// to give.
pub(crate) fn repair_main_track<E>(
    visual: &mut Vec<Arc<Track>>,
    fresh: impl FnOnce() -> Result<TrackId, E>,
) -> Result<Option<MainRepair>, E> {
    if visual.first().is_some_and(|t| t.kind == TrackKind::Video) {
        return Ok(None);
    }
    if let Some(from) = visual.iter().position(|t| t.kind == TrackKind::Video) {
        let id = visual[from].id;
        let (mut video, upper): (Vec<_>, Vec<_>) =
            visual.drain(..).partition(|t| t.kind == TrackKind::Video);
        video.extend(upper);
        *visual = video;
        return Ok(Some(MainRepair::Promoted { track: id, from }));
    }
    let id = fresh()?;
    visual.insert(0, Arc::new(Track::new(id, TrackKind::Video)));
    Ok(Some(MainRepair::Created { track: id }))
}

/// Clears the link of every clip that can't belong to it: a link holding
/// the detached sound of a video (TL-030) has only that video (whose sound
/// is `Detached` under it) and audio clips as members. An image, a text or a
/// video that plays its own sound carrying such a link (a hand-edited or
/// foreign file) would be taken for that sound, and restoring it would
/// delete the clip. Other links (format 1's, which held no detached sound)
/// are left alone. Returns how many clips lost their link; run it before
/// [`dissolve_small_relations`], which then dissolves what is left too
/// small.
pub(crate) fn clear_stray_links(visual: &mut [Arc<Track>], audio: &mut [Arc<Track>]) -> usize {
    let detached: BTreeSet<LinkId> = visual
        .iter()
        .chain(audio.iter())
        .flat_map(|t| t.clips().iter())
        .filter_map(|c| c.audio.detached())
        .collect();
    // A video detached under another link than its own is not guessed at:
    // validation refuses it.
    let stray = |clip: &Clip| {
        let sound = matches!(
            clip.content,
            ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            }
        );
        clip.link.is_some_and(|l| detached.contains(&l))
            && !sound
            && clip.audio.detached().is_none()
    };
    let mut cleared = 0;
    for track in visual.iter_mut().chain(audio.iter_mut()) {
        if !track.clips().iter().any(|c| stray(c)) {
            continue;
        }
        let clips = track
            .clips()
            .iter()
            .map(|clip| {
                if !stray(clip) {
                    return Arc::clone(clip);
                }
                cleared += 1;
                let mut clip = (**clip).clone();
                clip.link = None;
                Arc::new(clip)
            })
            .collect();
        *track = rebuilt(track, clips);
    }
    cleared
}

/// How many groups and links [`dissolve_small_relations`] dissolved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Dissolved {
    pub groups: usize,
    pub links: usize,
}

impl Dissolved {
    pub(crate) fn any(self) -> bool {
        self.groups + self.links > 0
    }
}

/// Clears the group and link ids that fewer than two clips of one sequence
/// carry, as every edit does after deleting a member (TL-011: "the loader
/// drops it from the link and dissolves the link if fewer than 2 members
/// remain"; TL-039 for groups). Nothing else about the clips changes; in
/// particular a video whose detached sound lost its audio clips stays
/// detached, so restoring it later gives the sound back (TL-030).
pub(crate) fn dissolve_small_relations(
    visual: &mut [Arc<Track>],
    audio: &mut [Arc<Track>],
) -> Dissolved {
    let mut groups: BTreeMap<GroupId, usize> = BTreeMap::new();
    let mut links: BTreeMap<LinkId, usize> = BTreeMap::new();
    for track in visual.iter().chain(audio.iter()) {
        for clip in track.clips() {
            if let Some(g) = clip.group {
                *groups.entry(g).or_default() += 1;
            }
            if let Some(l) = clip.link {
                *links.entry(l).or_default() += 1;
            }
        }
    }
    let lonely_groups: BTreeSet<GroupId> = lonely(groups);
    let lonely_links: BTreeSet<LinkId> = lonely(links);
    let is_lonely = |clip: &Clip| {
        clip.group.is_some_and(|g| lonely_groups.contains(&g))
            || clip.link.is_some_and(|l| lonely_links.contains(&l))
    };
    for track in visual.iter_mut().chain(audio.iter_mut()) {
        if !track.clips().iter().any(|c| is_lonely(c)) {
            continue;
        }
        let clips = track
            .clips()
            .iter()
            .map(|clip| {
                if !is_lonely(clip) {
                    return Arc::clone(clip);
                }
                let mut clip = (**clip).clone();
                clip.group = clip.group.filter(|g| !lonely_groups.contains(g));
                clip.link = clip.link.filter(|l| !lonely_links.contains(l));
                Arc::new(clip)
            })
            .collect();
        *track = rebuilt(track, clips);
    }
    Dissolved {
        groups: lonely_groups.len(),
        links: lonely_links.len(),
    }
}

/// Clears every attachment that can't hold (TL-029): one on the main track
/// (`visual[0]`, which holds the owners), one in a sequence that doesn't
/// follow, and one whose owner is not a clip of the main track. A
/// hand-edited file, or a main track repaired by [`repair_main_track`],
/// can leave such attachments; clearing one leaves its item where it is,
/// as turning *Follow main track* off would. Run it after
/// [`repair_main_track`]. Returns how many clips lost their attachment.
pub(crate) fn clear_broken_attachments(
    flags: SequenceFlags,
    visual: &mut [Arc<Track>],
    audio: &mut [Arc<Track>],
) -> usize {
    let owners: BTreeSet<ClipId> = visual
        .first()
        .map(|main| main.clips().iter().map(|c| c.id).collect())
        .unwrap_or_default();
    let mut cleared = 0;
    for (i, track) in visual.iter_mut().chain(audio.iter_mut()).enumerate() {
        let on_main = i == 0 && track.kind == TrackKind::Video;
        let broken = |clip: &Clip| {
            clip.attach
                .is_some_and(|a| on_main || !flags.follow_main || !owners.contains(&a.owner))
        };
        if !track.clips().iter().any(|c| broken(c)) {
            continue;
        }
        let clips = track
            .clips()
            .iter()
            .map(|clip| {
                if !broken(clip) {
                    return Arc::clone(clip);
                }
                cleared += 1;
                let mut clip = (**clip).clone();
                clip.attach = None;
                Arc::new(clip)
            })
            .collect();
        *track = rebuilt(track, clips);
    }
    cleared
}

/// `track` with `clips` in place of its own. Everything else is kept: the
/// header, and the track volume and pan (AU-013), which
/// [`Track::from_parts`] would reset.
fn rebuilt(track: &Track, clips: Vec<Arc<Clip>>) -> Arc<Track> {
    let mut t = Track::from_parts(track.id, track.kind, track.header.clone(), clips);
    t.mix = track.mix;
    Arc::new(t)
}

/// The ids counted fewer than twice.
fn lonely<T: Ord>(counts: BTreeMap<T, usize>) -> BTreeSet<T> {
    counts
        .into_iter()
        .filter(|(_, n)| *n < 2)
        .map(|(id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU64;

    use freecut_timeline::ids::{ClipId, MediaId};
    use freecut_timeline::model::{ClipContent, EmbeddedAudio, MediaKind};
    use freecut_timeline::time::{Tick, TimeRange};

    fn track(id: u64, kind: TrackKind) -> Arc<Track> {
        Arc::new(Track::new(
            TrackId::from_raw(NonZeroU64::new(id).unwrap()),
            kind,
        ))
    }

    fn ids(visual: &[Arc<Track>]) -> Vec<u64> {
        visual.iter().map(|t| t.id.get()).collect()
    }

    fn fresh() -> Result<TrackId, ()> {
        Ok(TrackId::from_raw(NonZeroU64::new(99).unwrap()))
    }

    #[test]
    fn a_video_main_track_is_left_alone() {
        let mut visual = vec![track(1, TrackKind::Video), track(2, TrackKind::Text)];
        assert_eq!(repair_main_track(&mut visual, fresh), Ok(None));
        assert_eq!(ids(&visual), vec![1, 2]);
    }

    #[test]
    fn the_lowest_video_track_is_promoted_with_the_video_band() {
        let mut visual = vec![
            track(1, TrackKind::Text),
            track(2, TrackKind::Video),
            track(3, TrackKind::Sticker),
            track(4, TrackKind::Video),
        ];
        let repair = repair_main_track(&mut visual, fresh).unwrap();
        assert_eq!(
            repair,
            Some(MainRepair::Promoted {
                track: visual[0].id,
                from: 1
            })
        );
        // Video band first, then the upper band; each keeps its order.
        assert_eq!(ids(&visual), vec![2, 4, 1, 3]);
    }

    #[test]
    fn a_missing_main_track_is_created_empty() {
        let mut visual = vec![track(1, TrackKind::Text)];
        let repair = repair_main_track(&mut visual, fresh).unwrap();
        assert!(matches!(repair, Some(MainRepair::Created { .. })));
        assert_eq!(ids(&visual), vec![99, 1]);
        assert_eq!(visual[0].kind, TrackKind::Video);
        assert!(visual[0].is_empty());

        let mut empty = Vec::new();
        repair_main_track(&mut empty, fresh).unwrap();
        assert_eq!(ids(&empty), vec![99]);
    }

    fn video_clip(id: u64, at: i64) -> Clip {
        let range = TimeRange::new(Tick::from_raw(at), Tick::from_raw(10)).unwrap();
        Clip::new(
            ClipId::from_raw(NonZeroU64::new(id).unwrap()),
            range,
            ClipContent::Media {
                asset: MediaId::from_raw(NonZeroU64::new(99).unwrap()),
                kind: MediaKind::Video,
                source: range,
                stream: None,
            },
        )
    }

    fn with_clips(id: u64, kind: TrackKind, clips: Vec<Clip>) -> Arc<Track> {
        Arc::new(Track::from_parts(
            TrackId::from_raw(NonZeroU64::new(id).unwrap()),
            kind,
            Default::default(),
            clips.into_iter().map(Arc::new).collect(),
        ))
    }

    #[test]
    fn lonely_groups_and_links_dissolve_and_a_detached_video_stays_detached() {
        let group = |n| Some(GroupId::from_raw(NonZeroU64::new(n).unwrap()));
        let link = |n| LinkId::from_raw(NonZeroU64::new(n).unwrap());
        let mut lone = video_clip(1, 0);
        lone.link = Some(link(50));
        lone.audio = EmbeddedAudio::Detached(link(50));
        lone.group = group(60);
        let mut paired = video_clip(2, 20);
        paired.group = group(61);
        let mut partner = video_clip(3, 40);
        partner.group = group(61);
        let untouched = with_clips(11, TrackKind::Video, vec![paired, partner]);
        let mut visual = vec![
            with_clips(10, TrackKind::Video, vec![lone]),
            Arc::clone(&untouched),
        ];
        let dissolved = dissolve_small_relations(&mut visual, &mut []);
        assert_eq!(
            dissolved,
            Dissolved {
                groups: 1,
                links: 1
            }
        );
        let lone = &visual[0].clips()[0];
        assert_eq!((lone.group, lone.link), (None, None));
        assert_eq!(lone.audio, EmbeddedAudio::Detached(link(50)));
        assert!(Arc::ptr_eq(&visual[1], &untouched), "pairs are left alone");

        assert!(!dissolve_small_relations(&mut visual, &mut []).any());
    }

    #[test]
    fn repaired_tracks_keep_their_volume_and_pan() {
        use freecut_timeline::model::{GainDb, Pan, TrackAudio};
        let link = |n| LinkId::from_raw(NonZeroU64::new(n).unwrap());
        let mix = TrackAudio {
            gain: GainDb::from_hundredths(-300),
            pan: Pan::new(-40),
        };
        let with_mix = |track: Arc<Track>| {
            let mut t = (*track).clone();
            t.mix = mix;
            Arc::new(t)
        };
        // A lonely group.
        let mut lone = video_clip(1, 0);
        lone.group = Some(GroupId::from_raw(NonZeroU64::new(60).unwrap()));
        let mut visual = vec![with_mix(with_clips(10, TrackKind::Video, vec![lone]))];
        assert!(dissolve_small_relations(&mut visual, &mut []).any());
        assert_eq!(visual[0].mix, mix);
        // A stray link: a video playing its own sound in a detached link.
        let mut detached = video_clip(2, 0);
        detached.link = Some(link(50));
        detached.audio = EmbeddedAudio::Detached(link(50));
        let mut stray = video_clip(3, 20);
        stray.link = Some(link(50));
        let mut visual = vec![
            with_clips(11, TrackKind::Video, vec![detached]),
            with_mix(with_clips(12, TrackKind::Video, vec![stray])),
        ];
        assert_eq!(clear_stray_links(&mut visual, &mut []), 1);
        assert_eq!(visual[1].clips()[0].link, None);
        assert_eq!(visual[1].mix, mix);
    }

    #[test]
    fn attachments_that_cannot_hold_are_cleared() {
        use freecut_timeline::model::Attachment;
        let anchor = |owner: u64| {
            Some(Attachment {
                owner: ClipId::from_raw(NonZeroU64::new(owner).unwrap()),
                owner_source_time: Tick::from_raw(3),
            })
        };
        // 1 is the main-track owner; 2 is on the main track itself.
        let owner = video_clip(1, 0);
        let mut on_main = video_clip(2, 20);
        on_main.attach = anchor(1);
        let mut good = video_clip(3, 0);
        good.attach = anchor(1);
        let mut gone = video_clip(4, 20);
        gone.attach = anchor(9);
        // An owner that is a clip, but not on the main track.
        let mut sideways = video_clip(5, 40);
        sideways.attach = anchor(3);
        let overlay = with_clips(11, TrackKind::Video, vec![good, gone, sideways]);
        let mut visual = vec![
            with_clips(10, TrackKind::Video, vec![owner, on_main]),
            overlay,
        ];
        let flags = SequenceFlags::default();
        assert_eq!(clear_broken_attachments(flags, &mut visual, &mut []), 3);
        let attached: Vec<(u64, bool)> = visual
            .iter()
            .flat_map(|t| t.clips())
            .map(|c| (c.id.get(), c.attach.is_some()))
            .collect();
        assert_eq!(
            attached,
            vec![(1, false), (2, false), (3, true), (4, false), (5, false)]
        );
        assert_eq!(clear_broken_attachments(flags, &mut visual, &mut []), 0);

        // A sequence that doesn't follow keeps no attachments.
        let off = SequenceFlags {
            follow_main: false,
            ..flags
        };
        assert_eq!(clear_broken_attachments(off, &mut visual, &mut []), 1);
        assert!(
            visual
                .iter()
                .flat_map(|t| t.clips())
                .all(|c| c.attach.is_none())
        );
    }

    #[test]
    fn running_out_of_ids_is_reported() {
        let mut visual = vec![track(1, TrackKind::Text)];
        assert_eq!(repair_main_track(&mut visual, || Err("full")), Err("full"));
        assert_eq!(ids(&visual), vec![1], "nothing changed");
    }
}
