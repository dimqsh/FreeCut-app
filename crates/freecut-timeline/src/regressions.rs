//! In-crate regression tests from the M1 review (they need `Document::apply`).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use core::num::NonZeroU64;
use std::sync::Arc;

use crate::Editor;
use crate::edit::{Change, Limits};
use crate::history::HistoryLimits;
use crate::ids::{ClipId, GroupId, IdAlloc, LinkId, SeqId, TrackId};
use crate::model::{
    Clip, ClipContent, Document, InvariantViolation, Track, TrackKind, TrackSlot, ValidateCtx,
};
use crate::time::{Tick, TimeRange};

fn nz(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

fn doc_with_relation(group: bool, rel: u64) -> Document {
    let (seq, main, text) = (
        SeqId::from_raw(nz(1)),
        TrackId::from_raw(nz(2)),
        TrackId::from_raw(nz(3)),
    );
    let mut doc = Document::new(
        seq,
        main,
        crate::model::ProjectSettings::new_default().unwrap(),
    );
    doc.apply(&Change::InsertTrack {
        seq,
        slot: TrackSlot::visual(1),
        track: Arc::new(Track::new(text, TrackKind::Text)),
    })
    .unwrap();
    for (id, start) in [(4, 0), (5, 10)] {
        let mut clip = Clip::new(
            ClipId::from_raw(nz(id)),
            TimeRange::new(Tick::from_raw(start), Tick::from_raw(10)).unwrap(),
            ClipContent::Text(Arc::default()),
        );
        if group {
            clip.group = Some(GroupId::from_raw(nz(rel)));
        } else {
            clip.link = Some(LinkId::from_raw(nz(rel)));
        }
        doc.apply(&Change::InsertClip {
            seq,
            track: text,
            clip: Arc::new(clip),
        })
        .unwrap();
    }
    doc
}

fn vctx(hw: u64) -> ValidateCtx {
    ValidateCtx {
        limits: Limits::default(),
        id_high_water: Some(nz(hw)),
    }
}

#[test]
fn loading_moves_the_allocator_past_group_and_link_ids() {
    for group in [true, false] {
        let doc = doc_with_relation(group, 1000);
        let editor = Editor::from_document(doc, IdAlloc::default(), HistoryLimits::default());
        assert!(
            editor.ids().next().get() > 1000,
            "group={group}: next id {} reuses relation 1000's id space",
            editor.ids().next()
        );
    }
}

#[test]
fn validate_reports_group_and_link_ids_at_or_above_high_water() {
    for group in [true, false] {
        let errs = doc_with_relation(group, 1000)
            .validate(&vctx(6))
            .err()
            .unwrap_or_default();
        assert!(
            errs.contains(&InvariantViolation::IdAboveHighWater(1000)),
            "group={group}: validate with high water 6 missed id 1000: {errs:?}"
        );
    }
}

#[test]
fn validate_reports_group_and_link_ids_colliding_with_entity_ids() {
    for group in [true, false] {
        let errs = doc_with_relation(group, 4)
            .validate(&vctx(6))
            .err()
            .unwrap_or_default();
        assert!(
            errs.contains(&InvariantViolation::DuplicateId(4)),
            "group={group}: relation id 4 collides with clip 4: {errs:?}"
        );
    }
}

#[test]
fn shared_group_and_link_ids_are_not_duplicates_of_themselves() {
    for group in [true, false] {
        assert_eq!(
            doc_with_relation(group, 6).validate(&vctx(7)),
            Ok(()),
            "group={group}"
        );
    }
}

/// TX-054: validate refuses a text clip with both Background and Bubble
/// on, independently of the range check.
#[test]
fn validate_rejects_background_and_bubble_both_on() {
    use crate::model::{SignedPerMille, TextClipData};
    let (seq, main, text) = (
        SeqId::from_raw(nz(1)),
        TrackId::from_raw(nz(2)),
        TrackId::from_raw(nz(3)),
    );
    let doc_with = |data: TextClipData| {
        let mut doc = Document::new(
            seq,
            main,
            crate::model::ProjectSettings::new_default().unwrap(),
        );
        doc.apply(&Change::InsertTrack {
            seq,
            slot: TrackSlot::visual(1),
            track: Arc::new(Track::new(text, TrackKind::Text)),
        })
        .unwrap();
        let clip = Clip::new(
            ClipId::from_raw(nz(4)),
            TimeRange::new(Tick::from_raw(0), Tick::from_raw(10)).unwrap(),
            ClipContent::Text(Arc::new(data)),
        );
        doc.apply(&Change::InsertClip {
            seq,
            track: text,
            clip: Arc::new(clip),
        })
        .unwrap();
        doc
    };
    let clip = ClipId::from_raw(nz(4));
    let mut both = TextClipData::default();
    both.style.background.on = true;
    both.style.bubble.on = true;
    assert_eq!(
        doc_with(both.clone()).validate(&vctx(10)).unwrap_err(),
        vec![InvariantViolation::TextBoxAndBubble { clip }]
    );
    both.style.letter_spacing = SignedPerMille::new(-999);
    assert_eq!(
        doc_with(both.clone()).validate(&vctx(10)).unwrap_err(),
        vec![
            InvariantViolation::TextBoxAndBubble { clip },
            InvariantViolation::TextRange { clip }
        ]
    );
    both.style.bubble.on = false;
    assert_eq!(
        doc_with(both).validate(&vctx(10)).unwrap_err(),
        vec![InvariantViolation::TextRange { clip }]
    );
    let mut one = TextClipData::default();
    one.style.bubble.on = true;
    assert!(doc_with(one).validate(&vctx(10)).is_ok());
}

/// F25/F26 from the M1 review: a one-clip ripple trim and a select-all
/// nudge must cost work linear in the clips moved. The clip lookups count
/// the clips they visit (`crate::probe`), so the check doesn't depend on
/// the machine's load: 4x the clips must cost at most about 4x the visits
/// (quadratic work would cost 16x).
mod scaling {
    use super::nz;
    use crate::Editor;
    use crate::edit::{
        Edge, EditCtx, EditRequest, InsertReq, InsertTarget, Limits, MoveReq, NewClip, TrimReq,
    };
    use crate::history::HistoryLimits;
    use crate::ids::{ClipId, IdAlloc, SeqId, TrackId};
    use crate::model::{Document, ProjectSettings, TextClipData};
    use crate::time::Tick;

    fn ctx() -> EditCtx {
        EditCtx {
            limits: Limits::default(),
        }
    }

    /// An editor with `n` one-frame text clips on a text track.
    fn texts(n: usize) -> (Editor, SeqId, Vec<ClipId>) {
        let seq = SeqId::from_raw(nz(1));
        let settings = ProjectSettings::new_default().unwrap();
        let frame = settings.frame_rate.frame_ticks();
        let doc = Document::new(seq, TrackId::from_raw(nz(2)), settings);
        let mut editor = Editor::open(doc, IdAlloc::default(), HistoryLimits::default()).unwrap();
        let req = EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::NewTrack,
            at: Tick::ZERO,
            clips: (0..n)
                .map(|_| NewClip::text(TextClipData::default(), frame))
                .collect(),
        });
        let plan = editor.plan(&ctx(), &req).unwrap();
        editor.commit(plan, crate::CommitMeta::default()).unwrap();
        let ids = editor.document().sequence(seq).unwrap().visual_tracks()[1]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();
        (editor, seq, ids)
    }

    /// The clips visited while planning `req`.
    fn visits(editor: &Editor, req: &EditRequest) -> u64 {
        crate::probe::take();
        editor.plan(&ctx(), req).unwrap();
        crate::probe::take()
    }

    fn assert_linear(small: u64, large: u64) {
        assert!(small > 0, "the lookups are counted");
        assert!(
            large <= 4 * small + 64,
            "4x the clips visited {large} clips against {small}"
        );
    }

    #[test]
    fn select_all_nudge_scales_linearly() {
        let cost = |n| {
            let (editor, seq, all) = texts(n);
            let frame = editor.document().settings().frame_rate.frame_ticks();
            let nudge = EditRequest::Move(MoveReq {
                seq,
                clips: all,
                delta: frame,
                track_shift: 0,
                followers: Vec::new(),
            });
            visits(&editor, &nudge)
        };
        assert_linear(cost(2_000), cost(8_000));
    }

    #[test]
    fn a_one_clip_ripple_trim_scales_linearly() {
        let cost = |n| {
            let (editor, seq, all) = texts(n);
            let frame = editor.document().settings().frame_rate.frame_ticks();
            let trim = EditRequest::Trim(TrimReq {
                seq,
                clip: all[0],
                edge: Edge::Out,
                delta: frame,
                ripple: true,
                partners: Vec::new(),
            });
            visits(&editor, &trim)
        };
        assert_linear(cost(2_000), cost(8_000));
    }
}
