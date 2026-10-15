//! The toolbar round's timeline commands on the open [`Project`] (UI-011):
//! cut away before / after the playhead (TL-026), the blade (TL-024),
//! markers (TL-037), *Follow main track* (TL-029) and linked selection
//! (TL-057). Each command plans one request and commits it as one undo
//! entry; the toolbar, the menus, the keys and the timeline's gestures all
//! call these, so they behave the same everywhere and are tested here
//! without a window.

use std::collections::BTreeSet;

use freecut_timeline::edit::{
    Cut, EditRequest, MarkerOwner, MarkerPatch, MarkerSelector, SplitReq, SplitTargets,
    TrimTargets, trim_to_playhead_available,
};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::{ClipId, SeqId};
use freecut_timeline::model::{
    FollowKind, FollowKinds, MarkerRef, MarkerScope, PaletteColor, Sequence, SequenceFlags,
    next_marker, prev_marker,
};
use freecut_timeline::select::{Expand, expand_selection};
use freecut_timeline::time::Tick;

use crate::session::{Project, unavailable_reason};

/// Who a marker added now would belong to (TL-037's `M` rule), for the
/// flag button's tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerTarget {
    Timeline,
    Clip(ClipId),
}

/// Why *Split at playhead* has nothing to cut with nothing selected
/// (TL-023).
pub const NO_CLIP_AT_PLAYHEAD: &str = "No clip at the playhead";

/// How a click picks clips (TL-015, TL-057).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// The clip (and its partners) replace the selection.
    Replace,
    /// Added to the selection, or removed with its partners when selected.
    Toggle,
}

/// The labels of TL-029's *Item kinds…* list, in its order.
pub fn follow_kind_label(kind: FollowKind) -> &'static str {
    match kind {
        FollowKind::VideoOverlay => "Video overlays",
        FollowKind::Text => "Text",
        FollowKind::Sticker => "Stickers & shapes",
        FollowKind::Effect => "Effects",
        FollowKind::Adjustment => "Filters & adjustments",
        FollowKind::Music => "Music",
        FollowKind::SoundEffect => "Sound effects",
        FollowKind::Speech => "Voice & speech",
        FollowKind::OtherAudio => "Other audio",
    }
}

/// The *Follow main track* tooltip's summary of the checked kinds (TL-029):
/// "All kinds", "None", or a short list.
pub fn follow_summary(kinds: FollowKinds) -> String {
    if kinds == FollowKinds::ALL {
        return "All kinds".to_owned();
    }
    if kinds.is_empty() {
        return "None".to_owned();
    }
    let names: Vec<&str> = kinds.iter().map(follow_kind_label).collect();
    if names.len() <= 3 {
        names.join(", ")
    } else {
        format!("{}, and {} more", names[..2].join(", "), names.len() - 2)
    }
}

/// The colour *Add marker with next colour* uses: the one after the last
/// colour used (Blue before any).
pub fn next_marker_color(last: Option<PaletteColor>) -> PaletteColor {
    last.unwrap_or_default().next()
}

/// The palette slot's name, for the editor's swatches.
pub fn color_name(color: PaletteColor) -> &'static str {
    match color {
        PaletteColor::Blue => "Blue",
        PaletteColor::Teal => "Teal",
        PaletteColor::Green => "Green",
        PaletteColor::Yellow => "Yellow",
        PaletteColor::Orange => "Orange",
        PaletteColor::Red => "Red",
        PaletteColor::Pink => "Pink",
        PaletteColor::Purple => "Purple",
    }
}

/// TL-057's selection rule for one click on `clip`: with `linked` on and
/// `single` (Alt) off, the clip brings its link partners and group members
/// on unlocked tracks; Toggle on a selected clip removes the whole set.
pub fn picked_selection(
    seq: &Sequence,
    current: &[ClipId],
    clip: ClipId,
    pick: Pick,
    linked: bool,
    single: bool,
) -> Vec<ClipId> {
    let set: BTreeSet<ClipId> = if single {
        BTreeSet::from([clip])
    } else {
        expand_selection(
            seq,
            &BTreeSet::from([clip]),
            Expand {
                groups: true,
                links: linked,
            },
        )
    };
    match pick {
        Pick::Replace => {
            // The clicked clip first: commands that act on "the" clip read
            // the first one.
            let mut out = vec![clip];
            out.extend(set.into_iter().filter(|c| *c != clip));
            out
        }
        Pick::Toggle if current.contains(&clip) => current
            .iter()
            .copied()
            .filter(|c| !set.contains(c))
            .collect(),
        Pick::Toggle => {
            let mut out = current.to_vec();
            for c in set {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            out
        }
    }
}

impl Project {
    fn root_seq(&self) -> SeqId {
        self.doc().root()
    }

    fn root(&self) -> Option<&Sequence> {
        self.doc().sequence(self.root_seq())
    }

    /// The root sequence's flags (the defaults when there is none).
    pub fn flags(&self) -> SequenceFlags {
        self.root().map(|s| s.flags).unwrap_or_default()
    }

    // ------------------------------------------------------------ TL-026

    /// The main-track clip under the playhead (on the frame grid).
    fn main_under_playhead(&self) -> Option<ClipId> {
        let rate = self.doc().settings().frame_rate;
        let at = rate.quantize(self.playhead).unwrap_or(self.playhead);
        let main = self.root()?.main_track()?;
        main.clip_at(at).map(|c| c.id)
    }

    /// The main-track clip under the playhead and, with `linked` (TL-057),
    /// its linked partners and group members that cover the playhead too,
    /// the main clip first: what Split and the cut-aways act on with
    /// nothing selected.
    fn main_with_partners(&self, linked: bool) -> Option<Vec<ClipId>> {
        let main = self.main_under_playhead()?;
        let mut out = vec![main];
        if linked && let Some(seq) = self.root() {
            let rate = self.doc().settings().frame_rate;
            let at = rate.quantize(self.playhead).unwrap_or(self.playhead);
            let rules = Expand {
                groups: true,
                links: true,
            };
            let covers = |id: &ClipId| {
                self.doc()
                    .clip(*id)
                    .is_some_and(|c| c.range.strictly_contains(at))
            };
            out.extend(
                expand_selection(seq, &BTreeSet::from([main]), rules)
                    .into_iter()
                    .filter(|id| *id != main && covers(id)),
            );
        }
        Some(out)
    }

    /// What a cut-away acts on: the selection, or with nothing selected the
    /// main-track clip under the playhead, with its linked partners when
    /// `linked` (TL-057) so they stay in sync.
    pub fn cut_targets(&self, linked: bool) -> TrimTargets {
        if !self.selection.is_empty() {
            return TrimTargets::Clips(self.selection.clone());
        }
        match self.main_with_partners(linked) {
            Some(clips) if clips.len() > 1 => TrimTargets::Clips(clips),
            _ => TrimTargets::MainUnderPlayhead,
        }
    }

    /// Whether the cut-away buttons and keys can act now; the reason when
    /// they can't (TL-026's tooltips).
    pub fn cut_available(&self, linked: bool) -> Result<(), &'static str> {
        trim_to_playhead_available(
            self.doc(),
            self.root_seq(),
            &self.cut_targets(linked),
            self.playhead,
        )
        .map_err(unavailable_reason)
    }

    /// Cuts away the part of the targets before (`Head`, Q) or after
    /// (`Tail`, W) the playhead, as one undo entry. `ripple_others` is the
    /// `Shift` variant that ripples non-main tracks; `linked` is the
    /// *Select linked clips together* preference (TL-057). Returns whether
    /// the playhead moved (a head cut on the magnetic main track).
    pub fn cut_away(
        &mut self,
        cut: Cut,
        ripple_others: bool,
        linked: bool,
    ) -> Result<bool, String> {
        self.cut_available(linked)?;
        let before = self.playhead;
        let req = EditRequest::TrimToPlayhead {
            seq: self.root_seq(),
            targets: self.cut_targets(linked),
            at: self.playhead,
            cut,
            ripple_others,
        };
        self.apply(&req, Coalesce::Never)?;
        Ok(self.playhead != before)
    }

    // ------------------------------------------------------------ TL-023

    /// What *Split at playhead* cuts (TL-023): the selection, or with
    /// nothing selected the main-track clip under the playhead (with its
    /// linked partners when `linked`, TL-057). Refused with the reason when
    /// there is none.
    pub fn split_targets(&self, linked: bool) -> Result<SplitTargets, &'static str> {
        if !self.selection.is_empty() {
            return Ok(SplitTargets::Clips(self.selection.clone()));
        }
        self.main_with_partners(linked)
            .map(SplitTargets::Clips)
            .ok_or(NO_CLIP_AT_PLAYHEAD)
    }

    /// *Split at playhead* (`Mod+B`), or with `all` *Split all*
    /// (`Mod+Shift+B`: every clip under the playhead on unlocked tracks),
    /// as one `SplitClips` entry.
    pub fn split_at_playhead(&mut self, all: bool, linked: bool) -> Result<(), String> {
        let targets = if all {
            SplitTargets::AllTracks
        } else {
            self.split_targets(linked)?
        };
        let req = EditRequest::Split(SplitReq {
            seq: self.root_seq(),
            at: self.playhead,
            targets,
        });
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    // ------------------------------------------------------------ TL-024

    /// A blade click at `at` (already snapped): cuts `clip`, or with
    /// `all_tracks` every unlocked track, as one `SplitClips` entry.
    pub fn blade(&mut self, clip: ClipId, at: Tick, all_tracks: bool) -> Result<(), String> {
        let targets = if all_tracks {
            SplitTargets::AllTracks
        } else {
            SplitTargets::Clips(vec![clip])
        };
        let req = EditRequest::Split(SplitReq {
            seq: self.root_seq(),
            at,
            targets,
        });
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    // ------------------------------------------------------------ TL-037

    /// Where `M` would put a marker now: on the one selected clip when the
    /// playhead is inside it, else on the timeline.
    pub fn marker_target(&self) -> MarkerTarget {
        let rate = self.doc().settings().frame_rate;
        let at = rate.quantize(self.playhead).unwrap_or(self.playhead);
        match self.selection.as_slice() {
            [id] if self.doc().clip(*id).is_some_and(|c| c.range.contains(at)) => {
                MarkerTarget::Clip(*id)
            }
            _ => MarkerTarget::Timeline,
        }
    }

    /// Adds a marker at the playhead by the `M` rule, in `color` (Blue when
    /// `None`), and selects it.
    pub fn add_marker(&mut self, color: Option<PaletteColor>) -> Result<MarkerRef, String> {
        let req = EditRequest::AddMarker {
            seq: self.root_seq(),
            at: self.playhead,
            owner: MarkerOwner::Auto(self.selection.clone()),
            color,
        };
        self.apply(&req, Coalesce::Never)?;
        self.markers
            .iter()
            .next()
            .copied()
            .ok_or_else(|| "FreeCut couldn't add the marker".to_owned())
    }

    /// Moves the playhead to the next (or previous) marker start among the
    /// timeline markers and the clip markers on visible tracks; at the end
    /// there is nothing to jump to and the status says so (TL-037).
    pub fn jump_to_marker(&mut self, forward: bool) -> Result<(), String> {
        let seq = self.root().ok_or("No more markers")?;
        let found = if forward {
            next_marker(seq, self.playhead, MarkerScope::VisibleTracks)
        } else {
            prev_marker(seq, self.playhead, MarkerScope::VisibleTracks)
        };
        let t = found.ok_or("No more markers")?;
        self.playhead = t;
        Ok(())
    }

    /// Deletes markers as one entry (TL-037's bulk deletes need no
    /// confirmation: undo brings them back).
    pub fn delete_markers(&mut self, which: MarkerSelector) -> Result<(), String> {
        let req = EditRequest::DeleteMarkers {
            seq: self.root_seq(),
            which,
        };
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    /// Deletes the selected markers; `false` when none is selected.
    pub fn delete_selected_markers(&mut self, coalesce: Coalesce) -> Result<bool, String> {
        if self.markers.is_empty() {
            return Ok(false);
        }
        let req = EditRequest::DeleteMarkers {
            seq: self.root_seq(),
            which: MarkerSelector::Refs(self.markers.iter().copied().collect()),
        };
        self.apply(&req, coalesce)?;
        self.markers.clear();
        Ok(true)
    }

    /// Whether *Delete markers on this clip* has something to do: exactly
    /// one clip selected, carrying markers.
    pub fn clip_with_markers(&self) -> Option<ClipId> {
        match self.selection.as_slice() {
            [id] if self.doc().clip(*id).is_some_and(|c| !c.markers.is_empty()) => Some(*id),
            _ => None,
        }
    }

    /// Whether the sequence has timeline markers.
    pub fn has_timeline_markers(&self) -> bool {
        self.root().is_some_and(|s| !s.markers.is_empty())
    }

    /// Moves a marker to start at sequence time `to` (quantized and, on a
    /// clip, kept inside it by the op).
    pub fn move_marker(&mut self, marker: MarkerRef, to: Tick) -> Result<(), String> {
        let req = EditRequest::MoveMarker {
            seq: self.root_seq(),
            marker,
            to,
        };
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    /// Changes a marker's name, note, colour or duration as one entry.
    pub fn edit_marker(&mut self, marker: MarkerRef, patch: MarkerPatch) -> Result<(), String> {
        let req = EditRequest::EditMarker {
            seq: self.root_seq(),
            marker,
            patch,
        };
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    // ------------------------------------------------------------ TL-029

    /// Turns *Follow main track* on or off: one entry carrying the
    /// attachments it made or cleared; nothing moves.
    pub fn toggle_follow_main(&mut self) -> Result<(), String> {
        let on = !self.flags().follow_main;
        let req = EditRequest::SetFollowMain {
            seq: self.root_seq(),
            on,
        };
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    /// Checks or unchecks the kinds that follow, as one entry.
    pub fn set_follow_kinds(&mut self, kinds: FollowKinds) -> Result<(), String> {
        let req = EditRequest::SetFollowKinds {
            seq: self.root_seq(),
            kinds,
        };
        self.apply(&req, Coalesce::Never).map(|_| ())
    }

    // ------------------------------------------------------------ TL-057

    /// Selects by TL-057's rule (see [`picked_selection`]). Picking clips
    /// clears the marker selection.
    pub fn pick(&mut self, clip: ClipId, pick: Pick, linked: bool, single: bool) {
        let Some(seq) = self.root() else { return };
        let next = picked_selection(seq, &self.selection, clip, pick, linked, single);
        self.selection = next;
        self.markers.clear();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::edit::{InsertReq, InsertTarget, NewClip, NewMedia};
    use freecut_timeline::model::{
        AudioStreamInfo, ClipContent, MediaInfo, MediaKind, ProjectSettings, Rotation, StoredPath,
        TextClipData, TrackFlags, TrackHeader, VideoStreamInfo,
    };
    use freecut_timeline::time::TimeRange;

    use super::*;

    pub(crate) fn s(n: i64) -> Tick {
        Tick::from_secs(n).unwrap()
    }

    fn video_info(secs: Tick) -> MediaInfo {
        MediaInfo {
            container: "mp4".into(),
            default_video: Some(0),
            duration: secs,
            video: vec![VideoStreamInfo {
                index: 0,
                codec: "h264".into(),
                width: 640,
                height: 360,
                sar: (1, 1),
                rotation: Rotation::None,
                mirrored: false,
                rate: None,
                variable_rate: false,
                has_alpha: false,
                bit_depth: 8,
                start: Tick::ZERO,
                duration: secs,
            }],
            audio: vec![AudioStreamInfo {
                index: 1,
                codec: "aac".into(),
                sample_rate: 48_000,
                channels: 2,
                start: Tick::ZERO,
                duration: secs,
                duration_exact: true,
                default: true,
                language: None,
                title: None,
                layout: None,
            }],
            still: None,
        }
    }

    /// Two 4 s videos with sound on the magnetic main track (0–4 s and
    /// 4–8 s) and a 2 s title over the first one at 1 s; nothing selected.
    pub(crate) fn talking_head() -> (Project, [ClipId; 3]) {
        let mut p = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let media = NewMedia {
            path: StoredPath {
                absolute: "/m/talk.mp4".into(),
                relative: None,
            },
            info: video_info(s(10)),
            fingerprint: None,
            display_name: None,
        };
        let req = EditRequest::ImportMedia(vec![media]);
        let plan = p.editor.plan(&p.ctx(), &req).unwrap();
        let id = plan.imported()[0];
        p.editor
            .commit(plan, freecut_timeline::CommitMeta::default())
            .unwrap();
        let main = p.root().unwrap().main_track().unwrap().id;
        let mut ids = Vec::new();
        for at in [Tick::ZERO, s(4)] {
            let source = TimeRange::new(at, s(4)).unwrap();
            let req = EditRequest::Insert(InsertReq {
                seq: p.doc().root(),
                target: InsertTarget::Track(main),
                at,
                clips: vec![NewClip::media(id, MediaKind::Video, source)],
            });
            p.apply(&req, Coalesce::Never).unwrap();
            ids.push(p.selection[0]);
        }
        let req = EditRequest::Insert(InsertReq {
            seq: p.doc().root(),
            target: InsertTarget::NewTrack,
            at: s(1),
            clips: vec![NewClip::text(TextClipData::with_content("Hi"), s(2))],
        });
        p.apply(&req, Coalesce::Never).unwrap();
        ids.push(p.selection[0]);
        p.selection.clear();
        (p, [ids[0], ids[1], ids[2]])
    }

    fn range(p: &Project, clip: ClipId) -> (Tick, Tick) {
        let c = p.doc().clip(clip).unwrap();
        (c.start(), c.end())
    }

    #[test]
    fn cut_away_before_the_playhead_ripples_the_main_track_and_moves_the_playhead() {
        let (mut p, [a, b, title]) = talking_head();
        // On a clip's first frame there is nothing before the playhead.
        p.playhead = s(4);
        assert_eq!(
            p.cut_available(true),
            Err("Move the playhead inside the clip")
        );
        p.playhead = s(2);
        assert_eq!(p.cut_available(true), Ok(()));
        let entries = p.editor.history().len();
        let moved = p.cut_away(Cut::Head, false, true).unwrap();
        assert!(moved, "Q moves the playhead to the clip's start");
        assert_eq!(p.playhead, Tick::ZERO);
        assert_eq!(range(&p, a), (Tick::ZERO, s(2)), "the head went");
        assert_eq!(range(&p, b), (s(2), s(6)), "later clips closed up");
        assert_eq!(p.editor.history().len(), entries + 1, "one entry");
        let label = p.editor.history().undo_label().unwrap();
        assert_eq!(
            crate::session::op_name(label.op),
            "Cut Away Before Playhead"
        );
        // The title was over a's cut-away part: it follows its anchor.
        assert!(p.doc().clip(title).is_some());
        p.undo().unwrap();
        assert_eq!(range(&p, a), (Tick::ZERO, s(4)));
        assert_eq!(range(&p, b), (s(4), s(8)));
        p.redo().unwrap();
        assert_eq!(range(&p, b), (s(2), s(6)));
    }

    #[test]
    fn cut_away_after_the_playhead_keeps_the_playhead_and_acts_on_the_selection() {
        let (mut p, [_, b, title]) = talking_head();
        p.selection = vec![title];
        p.playhead = s(2);
        let moved = p.cut_away(Cut::Tail, false, true).unwrap();
        assert!(!moved, "W never moves the playhead");
        assert_eq!(range(&p, title), (s(1), s(2)), "the title lost its tail");
        assert_eq!(range(&p, b), (s(4), s(8)), "an overlay cut doesn't ripple");
        // Nothing selected and the playhead past every clip: the reason.
        p.selection.clear();
        p.playhead = s(9);
        assert_eq!(
            p.cut_available(true),
            Err("Select a clip or put the playhead over the main track")
        );
        assert!(p.cut_away(Cut::Tail, false, true).is_err());
    }

    #[test]
    fn cut_away_on_a_locked_track_says_why() {
        let (mut p, [_, _, title]) = talking_head();
        let seq = p.doc().root();
        let (_, track) = p.root().unwrap().find_clip(title).unwrap();
        let header = TrackHeader {
            name: None,
            flags: TrackFlags {
                locked: true,
                ..track.header.flags
            },
        };
        let track = track.id;
        p.apply(
            &EditRequest::SetTrackHeader { seq, track, header },
            Coalesce::Never,
        )
        .unwrap();
        p.selection = vec![title];
        p.playhead = s(2);
        assert_eq!(
            p.cut_available(true),
            Err("The selected clip is on a locked track")
        );
    }

    #[test]
    fn the_blade_cuts_one_clip_or_every_track() {
        let (mut p, [a, _, title]) = talking_head();
        let entries = p.editor.history().len();
        p.blade(a, s(2), false).unwrap();
        assert_eq!(p.editor.history().len(), entries + 1);
        assert_eq!(range(&p, a), (Tick::ZERO, s(2)));
        assert_eq!(range(&p, title), (s(1), s(3)), "only the clicked clip");
        p.undo().unwrap();
        p.blade(a, s(2), true).unwrap();
        assert_eq!(range(&p, a), (Tick::ZERO, s(2)));
        assert_eq!(range(&p, title), (s(1), s(2)), "Shift cuts every track");
        assert_eq!(p.editor.history().len(), entries + 1, "one entry");
    }

    /// Detaches `clip`'s sound and returns the new audio clip; nothing
    /// stays selected.
    fn detach(p: &mut Project, clip: ClipId) -> ClipId {
        let req = EditRequest::DetachAudio {
            seq: p.doc().root(),
            clips: vec![clip],
        };
        p.apply(&req, Coalesce::Never).unwrap();
        let sound = p.selection[0];
        p.selection.clear();
        sound
    }

    #[test]
    fn a_cut_away_with_nothing_selected_cuts_the_linked_sound_too() {
        let (mut p, [a, _, _]) = talking_head();
        let sound = detach(&mut p, a);
        p.playhead = s(2);
        p.cut_away(Cut::Head, false, true).unwrap();
        let seq = p.doc().root();
        let offsets = freecut_timeline::edit::sync_offsets(p.doc(), seq);
        assert!(offsets.is_empty(), "in sync: {offsets:?}");
        assert_eq!(range(&p, sound), (Tick::ZERO, s(2)));
        // With the preference off only the main clip is cut.
        p.undo().unwrap();
        p.playhead = s(2);
        p.cut_away(Cut::Head, false, false).unwrap();
        assert_eq!(range(&p, sound), (Tick::ZERO, s(4)));
    }

    #[test]
    fn split_with_nothing_selected_cuts_the_main_clip_and_split_all_every_track() {
        let (mut p, [a, _, title]) = talking_head();
        let sound = detach(&mut p, a);
        p.playhead = s(2);
        let entries = p.editor.history().len();
        p.split_at_playhead(false, true).unwrap();
        assert_eq!(p.editor.history().len(), entries + 1, "one entry");
        assert_eq!(range(&p, a), (Tick::ZERO, s(2)), "the main clip is cut");
        assert_eq!(range(&p, sound), (Tick::ZERO, s(2)), "with its sound");
        assert_eq!(range(&p, title), (s(1), s(3)), "the title is left alone");
        p.undo().unwrap();
        p.selection.clear();
        p.split_at_playhead(true, true).unwrap();
        assert_eq!(range(&p, title), (s(1), s(2)), "Split all cuts every track");
        // Past the main track's end there's nothing to split.
        p.selection.clear();
        p.playhead = s(9);
        assert_eq!(
            p.split_at_playhead(false, true),
            Err(NO_CLIP_AT_PLAYHEAD.to_owned())
        );
        assert_eq!(
            crate::ui::timeline_toolbar::split_available(&p, true),
            Err(NO_CLIP_AT_PLAYHEAD)
        );
    }

    #[test]
    fn markers_go_on_the_timeline_or_the_selected_clip_and_are_selected() {
        let (mut p, [a, _, _]) = talking_head();
        p.playhead = s(2);
        assert_eq!(p.marker_target(), MarkerTarget::Timeline);
        let first = p.add_marker(None).unwrap();
        assert!(matches!(first, MarkerRef::Timeline(_)));
        assert_eq!(p.markers, BTreeSet::from([first]), "the new marker");
        let seq = p.root().unwrap();
        let m = seq.markers.get(first.id()).unwrap();
        assert_eq!((m.name.as_str(), m.color), ("Marker 1", PaletteColor::Blue));

        p.selection = vec![a];
        assert_eq!(p.marker_target(), MarkerTarget::Clip(a));
        let on_clip = p.add_marker(Some(next_marker_color(None))).unwrap();
        assert!(matches!(on_clip, MarkerRef::Clip(c, _) if c == a));
        let m = p.doc().clip(a).unwrap().markers.as_slice()[0].clone();
        assert_eq!((m.name.as_str(), m.color), ("Marker 1", PaletteColor::Teal));
        assert_eq!(p.clip_with_markers(), Some(a));
        // The playhead outside the one selected clip: a timeline marker.
        p.playhead = s(6);
        assert_eq!(p.marker_target(), MarkerTarget::Timeline);

        // Undo removes the marker and its selection.
        p.undo().unwrap();
        assert!(p.markers.is_empty());
        assert!(p.doc().clip(a).unwrap().markers.is_empty());
    }

    #[test]
    fn marker_navigation_stops_at_the_ends() {
        let (mut p, _) = talking_head();
        for t in [s(2), s(5)] {
            p.playhead = t;
            p.add_marker(None).unwrap();
        }
        p.playhead = Tick::ZERO;
        p.jump_to_marker(true).unwrap();
        assert_eq!(p.playhead, s(2));
        p.jump_to_marker(true).unwrap();
        assert_eq!(p.playhead, s(5));
        assert_eq!(p.jump_to_marker(true), Err("No more markers".to_owned()));
        assert_eq!(p.playhead, s(5), "no wrap-around");
        p.jump_to_marker(false).unwrap();
        assert_eq!(p.playhead, s(2));
        assert!(p.jump_to_marker(false).is_err());
    }

    #[test]
    fn markers_move_edit_and_delete_as_one_entry_each() {
        let (mut p, [a, _, _]) = talking_head();
        p.playhead = s(1);
        let m = p.add_marker(None).unwrap();
        let entries = p.editor.history().len();
        p.move_marker(m, s(3)).unwrap();
        let start = |p: &Project| p.root().unwrap().markers.get(m.id()).unwrap().at.start();
        assert_eq!(start(&p), s(3));
        p.edit_marker(
            m,
            MarkerPatch {
                name: Some("Intro".into()),
                color: Some(PaletteColor::Red),
                ..MarkerPatch::default()
            },
        )
        .unwrap();
        assert_eq!(p.editor.history().len(), entries + 2);
        let marker = p.root().unwrap().markers.get(m.id()).unwrap().clone();
        assert_eq!(
            (marker.name.as_str(), marker.color),
            ("Intro", PaletteColor::Red)
        );

        // Delete key: the selected markers, one entry.
        p.markers = BTreeSet::from([m]);
        assert!(p.delete_selected_markers(Coalesce::Never).unwrap());
        assert!(!p.has_timeline_markers());
        assert!(p.markers.is_empty());
        p.undo().unwrap();
        assert!(p.has_timeline_markers());

        // Bulk deletes: on the clip, and every timeline marker.
        p.selection = vec![a];
        p.playhead = s(2);
        p.add_marker(None).unwrap();
        p.delete_markers(MarkerSelector::OnClip(a)).unwrap();
        assert_eq!(p.clip_with_markers(), None);
        p.delete_markers(MarkerSelector::AllTimeline).unwrap();
        assert!(!p.has_timeline_markers());
    }

    #[test]
    fn follow_main_track_toggles_in_one_entry_and_moves_nothing() {
        let (mut p, [a, b, title]) = talking_head();
        assert!(p.flags().follow_main, "on by default");
        let before = (range(&p, a), range(&p, b), range(&p, title));
        let entries = p.editor.history().len();
        p.toggle_follow_main().unwrap();
        assert!(!p.flags().follow_main);
        assert_eq!(p.editor.history().len(), entries + 1);
        assert_eq!((range(&p, a), range(&p, b), range(&p, title)), before);
        assert!(p.doc().clip(title).unwrap().attach.is_none(), "cleared");
        p.undo().unwrap();
        assert!(p.flags().follow_main);
        assert!(p.doc().clip(title).unwrap().attach.is_some(), "restored");

        let kinds = FollowKinds::ALL.with(FollowKind::Text, false);
        p.set_follow_kinds(kinds).unwrap();
        assert_eq!(p.flags().follow_kinds, kinds);
        assert!(p.doc().clip(title).unwrap().attach.is_none());
        assert_eq!(
            follow_summary(kinds),
            "Video overlays, Stickers & shapes, and 6 more"
        );
        assert_eq!(follow_summary(FollowKinds::ALL), "All kinds");
        assert_eq!(follow_summary(FollowKinds::NONE), "None");
        assert_eq!(
            follow_summary(FollowKinds::NONE.with(FollowKind::Text, true)),
            "Text"
        );
    }

    #[test]
    fn linked_clips_are_picked_together_unless_alt_is_held() {
        let (mut p, [a, b, _]) = talking_head();
        p.selection = vec![a];
        let req = EditRequest::DetachAudio {
            seq: p.doc().root(),
            clips: vec![a],
        };
        p.apply(&req, Coalesce::Never).unwrap();
        let sound = p.selection[0];
        assert!(matches!(
            p.doc().clip(sound).unwrap().content,
            ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            }
        ));
        p.pick(a, Pick::Replace, true, false);
        assert_eq!(p.selection, vec![a, sound], "the clicked clip first");
        p.pick(sound, Pick::Replace, true, true);
        assert_eq!(p.selection, vec![sound], "Alt picks one");
        p.pick(a, Pick::Replace, false, false);
        assert_eq!(p.selection, vec![a], "with the option off, TL-030's rule");
        // Toggle adds the set and removes it whole.
        p.pick(b, Pick::Toggle, true, false);
        assert_eq!(p.selection, vec![a, b]);
        p.selection = vec![b];
        p.pick(a, Pick::Toggle, true, false);
        assert_eq!(p.selection, vec![b, a, sound]);
        p.pick(sound, Pick::Toggle, true, false);
        assert_eq!(p.selection, vec![b], "the whole linked set goes");
    }

    #[test]
    fn next_colour_follows_the_last_one_used() {
        assert_eq!(next_marker_color(None), PaletteColor::Teal);
        assert_eq!(
            next_marker_color(Some(PaletteColor::Purple)),
            PaletteColor::Blue
        );
        assert_eq!(color_name(PaletteColor::Orange), "Orange");
    }
}
