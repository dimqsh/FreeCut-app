//! Text clips (TX-001, TX-002, TX-004, TX-005): insertion and placement on
//! `Text` tracks, the text edit requests, and how other edits carry text.
//! Every edit goes through `Fixture::apply`, which checks plan purity,
//! validity and the undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    AddTrackReq, EditError, EditRequest, InsertReq, InsertTarget, MoveReq, NewClip, OpKind,
    SplitReq, SplitTargets, TextLayoutEdit, TextStyleEdit, TrackPosition,
};
use freecut_timeline::ids::{ClipId, TrackId};
use freecut_timeline::model::{
    ClipContent, Fill, FontRef, HAlign, PerMille, ProjectSettings, RefPx, Rgba8, RichText,
    TextClipData, TextLayout, TextStyle, TrackFlags, TrackHeader, TrackKind, TrackSlot,
    default_text_duration,
};
use freecut_timeline::time::FrameRate;
use pretty_assertions::assert_eq;

fn insert_text(fx: &mut Fixture, target: InsertTarget, at: i64, len: i64) -> ClipId {
    insert_data(fx, target, at, len, TextClipData::default())
}

fn insert_data(
    fx: &mut Fixture,
    target: InsertTarget,
    at: i64,
    len: i64,
    data: TextClipData,
) -> ClipId {
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target,
        at: f(at),
        clips: vec![NewClip::text(data, f(len))],
    });
    match fx.apply(req).unwrap() {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!("insert was a no-op"),
    }
}

fn text_of(fx: &Fixture, clip: ClipId) -> Arc<TextClipData> {
    Arc::clone(fx.doc().clip(clip).unwrap().content.text().unwrap())
}

fn track_of(fx: &Fixture, clip: ClipId) -> TrackId {
    fx.doc().find_clip(clip).unwrap().track
}

fn kinds(fx: &Fixture) -> Vec<TrackKind> {
    fx.seq().visual_tracks().iter().map(|t| t.kind).collect()
}

fn add_track(fx: &mut Fixture, kind: TrackKind) -> TrackId {
    fx.apply(EditRequest::AddTrack(AddTrackReq {
        seq: fx.seq_id(),
        kind,
        position: TrackPosition::Default,
    }))
    .unwrap();
    fx.seq()
        .tracks()
        .map(|(_, t)| t)
        .filter(|t| t.kind == kind)
        .max_by_key(|t| t.id)
        .unwrap()
        .id
}

fn lock(fx: &mut Fixture, track: TrackId) {
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                locked: true,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
}

fn style(
    fx: &mut Fixture,
    clips: Vec<ClipId>,
    edit: TextStyleEdit,
) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetTextStyle {
        seq: fx.seq_id(),
        clips,
        edit,
    })
}

fn layout(
    fx: &mut Fixture,
    clips: Vec<ClipId>,
    edit: TextLayoutEdit,
) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetTextLayout {
        seq: fx.seq_id(),
        clips,
        edit,
    })
}

fn content(fx: &mut Fixture, clip: ClipId, text: &str) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetTextContent {
        seq: fx.seq_id(),
        clip,
        content: RichText::new(text),
    })
}

// ---------------------------------------------------------------- insert

#[test]
fn a_new_text_clip_gets_the_factory_defaults_and_three_seconds() {
    let mut fx = Fixture::new();
    let len = default_text_duration(fx.doc().settings().frame_rate).unwrap();
    assert_eq!(len, f(75));
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: f(10),
        clips: vec![NewClip::text(TextClipData::default(), len)],
    });
    fx.apply(req).unwrap();
    // Allowed on an empty main track (TL-004): a new Text track above it.
    assert_eq!(fx.render(), "V1: 6@10+75\nV0*:");
    let clip = fx
        .doc()
        .clip(ClipId::from_raw(6.try_into().unwrap()))
        .unwrap();
    assert_eq!(clip.content.text().unwrap().content.as_str(), "Your text");
    assert_eq!(clip.transform, Default::default());
    assert_eq!(kinds(&fx), vec![TrackKind::Video, TrackKind::Text]);
}

#[test]
fn text_lengths_are_rounded_to_the_frame_grid() {
    let mut fx = Fixture::new();
    fx.set_rate(FrameRate::new(24_000, 1001).unwrap());
    let rate = fx.doc().settings().frame_rate;
    let len = default_text_duration(rate).unwrap();
    assert_eq!(len, rate.frames(72).unwrap());
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: rate.frames(3).unwrap(),
        clips: vec![NewClip::text(
            TextClipData::default(),
            freecut_timeline::model::DEFAULT_TEXT_DURATION,
        )],
    });
    let Committed::Applied { selection, .. } = fx.apply(req).unwrap() else {
        panic!("no-op");
    };
    let clip = fx.doc().clip(selection.unwrap()[0]).unwrap();
    assert_eq!(clip.duration(), len);
}

#[test]
fn first_free_uses_a_free_text_track_then_creates_one_on_top_of_the_band() {
    let mut fx = Fixture::new();
    // An overlay video track and a sticker track already exist.
    fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    add_track(&mut fx, TrackKind::Sticker);
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 20);
    let t1 = track_of(&fx, a);
    // New Text tracks go on top of the upper band.
    assert_eq!(
        kinds(&fx),
        vec![
            TrackKind::Video,
            TrackKind::Video,
            TrackKind::Sticker,
            TrackKind::Text
        ]
    );
    // Free over the whole span: same track.
    let b = insert_text(&mut fx, InsertTarget::FirstFree, 20, 5);
    assert_eq!(track_of(&fx, b), t1);
    // Busy: a second Text track, again on top.
    let c = insert_text(&mut fx, InsertTarget::FirstFree, 15, 10);
    let t2 = track_of(&fx, c);
    assert_ne!(t2, t1);
    assert_eq!(fx.seq().find_track(t2).unwrap().0, TrackSlot::visual(4));
    // Free on the lower Text track again: the lowest free one wins.
    let d = insert_text(&mut fx, InsertTarget::FirstFree, 40, 5);
    assert_eq!(track_of(&fx, d), t1);
    // Busy on the first, free on the second.
    let e = insert_text(&mut fx, InsertTarget::FirstFree, 2, 5);
    assert_eq!(track_of(&fx, e), t2);
}

#[test]
fn first_free_skips_locked_text_tracks() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let t1 = track_of(&fx, a);
    lock(&mut fx, t1);
    let b = insert_text(&mut fx, InsertTarget::FirstFree, 50, 10);
    assert_ne!(track_of(&fx, b), t1);
    assert_eq!(
        kinds(&fx),
        vec![TrackKind::Video, TrackKind::Text, TrackKind::Text]
    );
}

#[test]
fn text_dropped_on_a_video_track_is_redirected_to_a_text_track() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 30)]);
    let a = insert_text(&mut fx, InsertTarget::Track(main), 5, 10);
    let track = fx.seq().find_track(track_of(&fx, a)).unwrap().1;
    assert_eq!(track.kind, TrackKind::Text);
    // The main track is untouched.
    assert_eq!(fx.seq().main_track().unwrap().clips().len(), 1);
    // Dropping on that Text track itself keeps it there.
    let t = track.id;
    let b = insert_text(&mut fx, InsertTarget::Track(t), 20, 5);
    assert_eq!(track_of(&fx, b), t);
}

#[test]
fn a_text_insert_at_the_track_limit_is_refused() {
    let mut fx = Fixture::new();
    while fx.seq().track_count() < 100 {
        add_track(&mut fx, TrackKind::Sticker);
    }
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: f(0),
        clips: vec![NewClip::text(TextClipData::default(), f(5))],
    });
    assert_eq!(
        fx.apply(req).unwrap_err(),
        EditError::TrackLimit { max: 100 }
    );
}

// ---------------------------------------------------------------- content

#[test]
fn set_text_content_replaces_the_text_and_keeps_the_rest() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let before = fx.doc().clip(a).unwrap().clone();
    content(&mut fx, a, "Hello\nwörld 👋🏽").unwrap();
    let after = fx.doc().clip(a).unwrap();
    assert_eq!(
        after.content.text().unwrap().content.as_str(),
        "Hello\nwörld 👋🏽"
    );
    assert_eq!(after.range, before.range);
    assert_eq!(after.transform, before.transform);
    assert_eq!(
        after.content.text().unwrap().style,
        before.content.text().unwrap().style
    );
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::SetTextContent
    );
    // Empty content is allowed (TX-002).
    content(&mut fx, a, "").unwrap();
    assert!(text_of(&fx, a).content.is_empty());
}

#[test]
fn unchanged_content_records_nothing() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    assert_eq!(content(&mut fx, a, "Your text").unwrap(), Committed::NoOp);
}

#[test]
fn content_edits_need_a_text_clip_on_an_unlocked_track() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    assert_eq!(content(&mut fx, v, "x").unwrap_err(), EditError::NoTextClip);
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let t = track_of(&fx, a);
    lock(&mut fx, t);
    assert_eq!(
        content(&mut fx, a, "x").unwrap_err(),
        EditError::TrackLocked { track: t }
    );
}

// ---------------------------------------------------------------- style

#[test]
fn style_edits_apply_to_every_selected_text_clip_and_skip_others() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let b = insert_text(&mut fx, InsertTarget::FirstFree, 20, 10);
    let all = vec![v, a, b];

    style(
        &mut fx,
        all.clone(),
        TextStyleEdit::Font(FontRef::new("Noto Serif")),
    )
    .unwrap();
    style(
        &mut fx,
        all.clone(),
        TextStyleEdit::Size(RefPx::from_px(120)),
    )
    .unwrap();
    let red = Rgba8::new(255, 0, 0, 128);
    style(&mut fx, all.clone(), TextStyleEdit::Fill(red.into())).unwrap();
    style(&mut fx, all.clone(), TextStyleEdit::Bold(false)).unwrap();
    style(&mut fx, all.clone(), TextStyleEdit::Italic(true)).unwrap();
    let expected = TextStyle {
        font: FontRef::new("Noto Serif"),
        size: RefPx::from_hundredths(12_000),
        fill: Fill::Solid(red),
        bold: false,
        italic: true,
        ..TextStyle::default()
    };
    for clip in [a, b] {
        assert_eq!(text_of(&fx, clip).style, expected);
        assert_eq!(text_of(&fx, clip).content.as_str(), "Your text");
    }
    assert_eq!(fx.editor.history().undo_label().unwrap().count, 2);
    // The video clip is untouched.
    assert!(matches!(
        fx.doc().clip(v).unwrap().content,
        ClipContent::Media { .. }
    ));
}

#[test]
fn style_sizes_are_clamped_and_repeats_record_nothing() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    style(&mut fx, vec![a], TextStyleEdit::Size(RefPx::from_px(5000))).unwrap();
    assert_eq!(text_of(&fx, a).style.size, TextStyle::MAX_SIZE);
    assert_eq!(
        style(&mut fx, vec![a], TextStyleEdit::Size(RefPx::from_px(1001))).unwrap(),
        Committed::NoOp
    );
    style(&mut fx, vec![a], TextStyleEdit::Size(RefPx::from_px(-3))).unwrap();
    assert_eq!(text_of(&fx, a).style.size, TextStyle::MIN_SIZE);
    assert_eq!(
        style(&mut fx, vec![a], TextStyleEdit::Bold(true)).unwrap(),
        Committed::NoOp,
        "bold is already the default"
    );
}

#[test]
fn style_and_layout_edits_without_text_clips_are_refused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    assert_eq!(
        style(&mut fx, vec![v], TextStyleEdit::Italic(true)).unwrap_err(),
        EditError::NoTextClip
    );
    assert_eq!(
        layout(&mut fx, vec![v], TextLayoutEdit::Align(HAlign::Left)).unwrap_err(),
        EditError::NoTextClip
    );
    assert_eq!(
        style(&mut fx, vec![], TextStyleEdit::Italic(true)).unwrap_err(),
        EditError::NothingSelected
    );
}

#[test]
fn a_locked_text_clip_refuses_the_whole_style_edit() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let b = insert_text(&mut fx, InsertTarget::FirstFree, 5, 10);
    let tb = track_of(&fx, b);
    lock(&mut fx, tb);
    assert_eq!(
        style(&mut fx, vec![a, b], TextStyleEdit::Italic(true)).unwrap_err(),
        EditError::TrackLocked { track: tb }
    );
    assert!(!text_of(&fx, a).style.italic);
}

// ---------------------------------------------------------------- layout

#[test]
fn layout_edits_set_alignment_box_width_and_line_spacing() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    layout(&mut fx, vec![a], TextLayoutEdit::Align(HAlign::Right)).unwrap();
    layout(
        &mut fx,
        vec![a],
        TextLayoutEdit::BoxWidth(Some(RefPx::from_px(600))),
    )
    .unwrap();
    layout(
        &mut fx,
        vec![a],
        TextLayoutEdit::LineSpacing(PerMille::new(1500)),
    )
    .unwrap();
    assert_eq!(
        text_of(&fx, a).layout,
        TextLayout {
            align: HAlign::Right,
            box_width: Some(RefPx::from_px(600)),
            line_spacing: PerMille::new(1500),
            ..TextLayout::default()
        }
    );
    // Clamps (TX-005, TX-009).
    layout(
        &mut fx,
        vec![a],
        TextLayoutEdit::BoxWidth(Some(RefPx::from_px(1))),
    )
    .unwrap();
    assert_eq!(
        text_of(&fx, a).layout.box_width,
        Some(TextLayout::MIN_BOX_WIDTH)
    );
    layout(
        &mut fx,
        vec![a],
        TextLayoutEdit::LineSpacing(PerMille::new(9999)),
    )
    .unwrap();
    assert_eq!(
        text_of(&fx, a).layout.line_spacing,
        TextLayout::MAX_LINE_SPACING
    );
    // Back to auto width.
    layout(&mut fx, vec![a], TextLayoutEdit::BoxWidth(None)).unwrap();
    assert_eq!(text_of(&fx, a).layout.box_width, None);
    assert_eq!(
        layout(&mut fx, vec![a], TextLayoutEdit::BoxWidth(None)).unwrap(),
        Committed::NoOp
    );
}

// ---------------------------------------------------------------- other edits

#[test]
fn splitting_a_text_clip_keeps_its_data_on_both_parts() {
    let mut fx = Fixture::new();
    let data = TextClipData::with_content("Split me");
    let a = insert_data(&mut fx, InsertTarget::FirstFree, 0, 20, data.clone());
    style(&mut fx, vec![a], TextStyleEdit::Italic(true)).unwrap();
    let Committed::Applied { selection, .. } = fx
        .apply(EditRequest::Split(SplitReq {
            seq: fx.seq_id(),
            at: f(8),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap()
    else {
        panic!("no-op");
    };
    let right = selection.unwrap()[0];
    assert_eq!(text_of(&fx, a), text_of(&fx, right));
    assert_eq!(text_of(&fx, right).content.as_str(), "Split me");
    assert!(text_of(&fx, right).style.italic);
    assert_eq!(track_of(&fx, a), track_of(&fx, right));
}

#[test]
fn text_moves_between_text_tracks_but_never_onto_video_tracks() {
    let mut fx = Fixture::new();
    fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let b = insert_text(&mut fx, InsertTarget::FirstFree, 0, 10);
    let (ta, tb) = (track_of(&fx, a), track_of(&fx, b));
    assert_ne!(ta, tb);
    let data = text_of(&fx, b);
    // Down one track: the lower Text track, free from frame 30.
    fx.apply(EditRequest::Move(MoveReq {
        seq: fx.seq_id(),
        clips: vec![b],
        delta: f(30),
        track_shift: -1,
        followers: Vec::new(),
    }))
    .unwrap();
    assert_eq!(track_of(&fx, b), ta);
    assert_eq!(text_of(&fx, b), data);
    // Down again: the video overlay track refuses it.
    assert_eq!(
        fx.apply(EditRequest::Move(MoveReq {
            seq: fx.seq_id(),
            clips: vec![b],
            delta: f(0),
            track_shift: -1,
            followers: Vec::new(),
        }))
        .unwrap_err(),
        EditError::KindMismatch {
            track: fx.track_at(TrackSlot::visual(1))
        }
    );
}

#[test]
fn conforming_keeps_text_data() {
    let mut fx = Fixture::new();
    let a = insert_data(
        &mut fx,
        InsertTarget::FirstFree,
        3,
        10,
        TextClipData::with_content("keep"),
    );
    let settings = ProjectSettings {
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..*fx.doc().settings()
    };
    fx.apply(EditRequest::SetProjectSettings {
        settings,
        conform: true,
    })
    .unwrap();
    assert_eq!(text_of(&fx, a).content.as_str(), "keep");
}

// ------------------------------------------------------------- paste style

/// TX-020: Paste style puts another clip's whole style (layer switches
/// included) on the targets in one undo step; their content and layout
/// stay, and Fixture::apply checks undo and redo.
#[test]
fn paste_style_copies_the_whole_style_and_keeps_content_and_layout() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, InsertTarget::FirstFree, 0, 20);
    let b = insert_text(&mut fx, InsertTarget::FirstFree, 20, 20);
    style(&mut fx, vec![a], TextStyleEdit::Bold(true)).unwrap();
    style(
        &mut fx,
        vec![a],
        TextStyleEdit::Size(RefPx::from_hundredths(9_600)),
    )
    .unwrap();
    style(
        &mut fx,
        vec![a],
        TextStyleEdit::SetLayer {
            layer: freecut_timeline::model::TextLayerKind::Outline,
            on: true,
        },
    )
    .unwrap();
    content(&mut fx, b, "Keep me").unwrap();
    layout(&mut fx, vec![b], TextLayoutEdit::Align(HAlign::Right)).unwrap();
    let source = text_of(&fx, a).style.clone();
    let before = text_of(&fx, b);
    assert_ne!(before.style, source);

    style(
        &mut fx,
        vec![b],
        TextStyleEdit::Paste(Box::new(source.clone())),
    )
    .unwrap();
    let after = text_of(&fx, b);
    assert_eq!(after.style, source);
    assert_eq!(after.content, before.content);
    assert_eq!(after.layout, before.layout);
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::SetTextStyle
    );
    // Pasting the same style again changes nothing.
    assert!(matches!(
        style(&mut fx, vec![b], TextStyleEdit::Paste(Box::new(source))).unwrap(),
        Committed::NoOp
    ));
}
