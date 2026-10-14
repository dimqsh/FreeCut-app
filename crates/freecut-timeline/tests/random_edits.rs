//! Property test: random edit sequences keep every invariant, and undoing
//! everything restores the original document exactly (TL-043).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::edit::{
    AddTrackReq, AudioEdit, DeleteReq, Edge, EditRequest, GainKeyEdit, InsertReq, InsertTarget,
    KeyEdit, MoveReq, NewClip, ResetScope, SplitReq, SplitTargets, TextLayoutEdit, TextStyleEdit,
    TrackPosition, TransformEdit, TrimReq, change_case, sync_offsets,
};
use freecut_timeline::edit::{
    Cut, MarkerOwner, MarkerPatch, MarkerSelector, NewMedia, OpKind, RecordTarget, TrimTargets,
};
use freecut_timeline::history::{Coalesce, CoalesceKey, GestureId, HistoryLimits};
use freecut_timeline::model::{
    AnimOrder, AnimUnit, AnimationSpec, Direction, PresetId, PropGroup, PropPath, PropValue, Slot,
    TextPropPath, TransformProp, Travel,
};
use freecut_timeline::model::{FollowKinds, MarkerRef, PaletteColor, expected_start};
use freecut_timeline::motion::PresetTable;
use freecut_timeline::plan::{AnimContext, PlanInputs, PlanNode, frame_plan_with};
use freecut_timeline::time::Tick;
use std::collections::BTreeSet;
use std::sync::Arc;

use freecut_timeline::ids::IdAlloc;
use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::model::{
    Angle, CanvasPos, Document, FadeCurve, FontRef, GainDb, HAlign, Interp, PerMille,
    ProjectSettings, RefPx, Rgba8, RichText, Scale, StoredPath, StreamSel,
};
use freecut_timeline::model::{
    BoxHeight, BoxShape, BubbleKind, CaseTransform, DeciDegrees, Fill, FillCenter, FillExtent,
    FillKind, GlowDirection, GradientStop, GradientStops, LangTag, Side, SignedPerMille,
    StrokePosition, TailTip, TextCard, TextLayerKind, TextLens, TextStyle, VAnchor,
};
use freecut_timeline::model::{
    Clip, ClipContent, MediaKind, MediaSource, TextClipData, TrackFlags, TrackHeader, TrackKind,
};
use freecut_timeline::time::FrameRate;
use freecut_timeline::time::TimeRange;
use freecut_timeline::{CommitMeta, Committed, Editor};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Insert {
        asset: u8,
        src: i64,
        len: i64,
        at: i64,
        track: u8,
    },
    InsertText {
        at: i64,
        len: i64,
        track: u8,
    },
    /// One of the text requests: content, a style value, a layout value, a
    /// reset or Change case.
    SetText {
        pick: u16,
        count: u8,
        which: u8,
        value: i32,
    },
    /// Switches an optional text layer on every text clip (TX-054), with
    /// Background and Bubble weighted up for the exclusivity rule.
    TextLayer {
        pick: u16,
        layer: u8,
        on: bool,
    },
    SetTransform {
        pick: u16,
        count: u8,
        which: u8,
        value: i32,
    },
    /// CL-007, CL-008, CL-013, CL-014, CL-015, CL-023, CL-024, CL-035.
    Color {
        pick: u16,
        count: u8,
        which: u8,
        value: i32,
    },
    Move {
        pick: u16,
        count: u8,
        delta: i64,
        shift: i8,
    },
    Trim {
        pick: u16,
        out: bool,
        delta: i64,
        ripple: bool,
    },
    Split {
        pick: u16,
        at: i64,
        all: bool,
    },
    Delete {
        pick: u16,
        count: u8,
        ripple: bool,
    },
    AddTrack {
        kind: u8,
    },
    Lock {
        track: u8,
        locked: bool,
    },
    Magnet {
        on: bool,
    },
    Conform,
    Import {
        frames: i64,
        video: bool,
        sound: bool,
        /// Audio streams when `sound`, and which is flagged default.
        streams: u32,
        default: Option<usize>,
    },
    /// One of the volume keyframe edits (AU-008).
    GainKeys {
        pick: u16,
        count: u8,
        which: u8,
        at: i64,
        to: i64,
        value: i32,
    },
    /// One of the transform and text keyframe edits (TR-028 to TR-031,
    /// TX-021), card commands included.
    Keys {
        pick: u16,
        count: u8,
        prop: u8,
        which: u8,
        at: i64,
        to: i64,
        value: i32,
    },
    /// Sets or clears an animation slot (FX-018, FX-019, TX-022).
    Anim {
        pick: u16,
        count: u8,
        slot: u8,
        preset: u8,
        len: i64,
        value: i32,
    },
    /// The master volume (AU-010), sometimes above its +12 dB ceiling.
    MasterGain {
        value: i32,
    },
    /// Track volume or pan (AU-013).
    TrackStage {
        track: u8,
        pan: bool,
        value: i32,
    },
    /// The stream picker (AU-038): 0 = default, 1 = all mixed, else an
    /// index (sometimes missing).
    Stream {
        pick: u16,
        count: u8,
        which: u8,
    },
    AddStream {
        pick: u16,
        stream: u8,
    },
    Resync {
        pick: u16,
        count: u8,
    },
    /// Moves one linked audio clip on its own, putting it out of sync.
    Slip {
        pick: u16,
        delta: i64,
    },
    Detach {
        pick: u16,
        count: u8,
    },
    Restore {
        pick: u16,
        count: u8,
    },
    SetAudio {
        pick: u16,
        count: u8,
        which: u8,
        value: i64,
    },
    Extract {
        pick: u8,
        default_stream: bool,
    },
    RemoveMedia {
        pick: u8,
        with_clips: bool,
    },
    Relink {
        pick: u8,
        frames: i64,
    },
    /// Switches the sequence rate between 25 and 30 fps (positions keep
    /// their ticks, so clips go off the grid).
    RateChange,
    /// Cut away before/after the playhead (TL-026).
    CutAway {
        pick: u16,
        count: u8,
        at: i64,
        head: bool,
        ripple: bool,
    },
    /// Add, edit, move or delete markers (TL-037).
    Marker {
        pick: u16,
        which: u8,
        at: i64,
        value: i32,
    },
    /// *Follow main track* on/off, or a kind set (TL-029).
    Follow {
        on: bool,
        kinds: Option<u16>,
    },
    /// A recorded take (AU-027).
    Record {
        at: i64,
        handle: i64,
        frames: i64,
        new_track: bool,
        replace: bool,
    },
    Undo,
    Redo,
    BeginGesture,
    EndGesture,
    CancelGesture,
    Save,
}

/// How an edit is committed when no gesture is open.
#[derive(Debug, Clone, Copy)]
enum Mode {
    Never,
    /// A keyboard repeat of command `key`, `dt_ms` after the previous one.
    Repeat {
        key: u8,
        dt_ms: u64,
    },
}

fn mode() -> impl Strategy<Value = Mode> {
    prop_oneof![
        3 => Just(Mode::Never),
        1 => (0u8..2, 0u64..1_500).prop_map(|(key, dt_ms)| Mode::Repeat { key, dt_ms }),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0u8..6, 0i64..60, 1i64..40, 0i64..80, 0u8..4)
            .prop_map(|(asset, src, len, at, track)| Op::Insert { asset, src, len, at, track }),
        2 => (0i64..80, 1i64..30, 0u8..8).prop_map(|(at, len, track)| Op::InsertText { at, len, track }),
        4 => (any::<u16>(), 1u8..3, 0u8..(TEXT_OPS + STYLE_EDITS), any::<i32>())
            .prop_map(|(pick, count, which, value)| Op::SetText { pick, count, which, value }),
        3 => (any::<u16>(), 0u8..10, prop::bool::weighted(0.7))
            .prop_map(|(pick, layer, on)| Op::TextLayer { pick, layer, on }),
        2 => (any::<u16>(), 1u8..3, 0u8..7, any::<i32>())
            .prop_map(|(pick, count, which, value)| Op::SetTransform { pick, count, which, value }),
        4 => (any::<u16>(), 1u8..3, 0u8..19, any::<i32>())
            .prop_map(|(pick, count, which, value)| Op::Color { pick, count, which, value }),
        4 => (any::<u16>(), 1u8..3, -30i64..30, -1i8..2)
            .prop_map(|(pick, count, delta, shift)| Op::Move { pick, count, delta, shift }),
        3 => (any::<u16>(), any::<bool>(), -20i64..20, any::<bool>())
            .prop_map(|(pick, out, delta, ripple)| Op::Trim { pick, out, delta, ripple }),
        3 => (any::<u16>(), 0i64..120, any::<bool>()).prop_map(|(pick, at, all)| Op::Split { pick, at, all }),
        2 => (any::<u16>(), 1u8..3, any::<bool>()).prop_map(|(pick, count, ripple)| Op::Delete { pick, count, ripple }),
        1 => (0u8..3).prop_map(|kind| Op::AddTrack { kind }),
        1 => (0u8..4, any::<bool>()).prop_map(|(track, locked)| Op::Lock { track, locked }),
        1 => any::<bool>().prop_map(|on| Op::Magnet { on }),
        1 => Just(Op::Conform),
        1 => (1i64..250, any::<bool>(), prop::bool::weighted(0.8), 1u32..4, prop::option::of(0usize..3))
            .prop_map(|(frames, video, sound, streams, default)| Op::Import { frames, video, sound, streams, default }),
        2 => (any::<u16>(), 1u8..3).prop_map(|(pick, count)| Op::Detach { pick, count }),
        2 => (any::<u16>(), 1u8..3).prop_map(|(pick, count)| Op::Restore { pick, count }),
        2 => (any::<u16>(), 1u8..3, 0u8..6, -10i64..40)
            .prop_map(|(pick, count, which, value)| Op::SetAudio { pick, count, which, value }),
        2 => (any::<u16>(), 1u8..3, 0u8..6, -20i64..120, -20i64..120, any::<i32>())
            .prop_map(|(pick, count, which, at, to, value)| Op::GainKeys { pick, count, which, at, to, value }),
        1 => (0u8..5, any::<bool>(), any::<i32>())
            .prop_map(|(track, pan, value)| Op::TrackStage { track, pan, value }),
        6 => (any::<u16>(), 1u8..3, 0u8..23, 0u8..8, -20i64..120, -20i64..120, any::<i32>())
            .prop_map(|(pick, count, prop, which, at, to, value)| Op::Keys { pick, count, prop, which, at, to, value }),
        3 => (any::<u16>(), 1u8..3, 0u8..3, 0u8..7, -2i64..80, any::<i32>())
            .prop_map(|(pick, count, slot, preset, len, value)| Op::Anim { pick, count, slot, preset, len, value }),
        1 => (-7000i32..2000).prop_map(|value| Op::MasterGain { value }),
        1 => (any::<u16>(), 1u8..3, 0u8..6).prop_map(|(pick, count, which)| Op::Stream { pick, count, which }),
        2 => (any::<u16>(), 0u8..5).prop_map(|(pick, stream)| Op::AddStream { pick, stream }),
        2 => (any::<u16>(), 1u8..3).prop_map(|(pick, count)| Op::Resync { pick, count }),
        3 => (any::<u16>(), -20i64..20).prop_map(|(pick, delta)| Op::Slip { pick, delta }),
        1 => (any::<u8>(), any::<bool>())
            .prop_map(|(pick, default_stream)| Op::Extract { pick, default_stream }),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(pick, with_clips)| Op::RemoveMedia { pick, with_clips }),
        1 => (any::<u8>(), 1i64..250).prop_map(|(pick, frames)| Op::Relink { pick, frames }),
        1 => Just(Op::RateChange),
        3 => (any::<u16>(), 0u8..3, -5i64..90, any::<bool>(), prop::bool::weighted(0.3))
            .prop_map(|(pick, count, at, head, ripple)| Op::CutAway { pick, count, at, head, ripple }),
        4 => (any::<u16>(), 0u8..10, -5i64..120, any::<i32>())
            .prop_map(|(pick, which, at, value)| Op::Marker { pick, which, at, value }),
        1 => (prop::bool::weighted(0.6), prop::option::of(any::<u16>()))
            .prop_map(|(on, kinds)| Op::Follow { on, kinds }),
        1 => (-5i64..120, 0i64..5, 1i64..60, any::<bool>(), any::<bool>())
            .prop_map(|(at, handle, frames, new_track, replace)| Op::Record { at, handle, frames, new_track, replace }),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
        1 => Just(Op::BeginGesture),
        1 => Just(Op::EndGesture),
        1 => Just(Op::CancelGesture),
        1 => Just(Op::Save),
    ]
}

fn all_clips(fx: &Fixture) -> Vec<ClipId> {
    fx.seq()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter().map(|c| c.id))
        .collect()
}

fn pick(fx: &Fixture, pick: u16, count: u8) -> Vec<ClipId> {
    let clips = all_clips(fx);
    if clips.is_empty() {
        return Vec::new();
    }
    (0..usize::from(count))
        .map(|i| clips[(usize::from(pick) + i * 7) % clips.len()])
        .collect()
}

fn request(fx: &Fixture, op: &Op) -> Option<EditRequest> {
    let seq = fx.seq_id();
    let tracks: Vec<_> = fx.seq().tracks().map(|(_, t)| t.id).collect();
    Some(match op {
        Op::Insert {
            asset,
            src,
            len,
            at,
            track,
        } => {
            // Any item in the library, including ones imported mid-run.
            let items: Vec<_> = fx.doc().library().items().cloned().collect();
            let item = items.get(usize::from(*asset) % items.len().max(1))?;
            let kind = if item.info.has_video() {
                MediaKind::Video
            } else {
                MediaKind::Audio
            };
            let max = item.info.duration.raw() / f(1).raw();
            if max < 1 {
                return None;
            }
            let asset = item.id;
            let src = (*src).min(max - 1);
            let len = (*len).min(max - src);
            let target = match tracks.get(usize::from(*track)) {
                Some(t) => InsertTarget::Track(*t),
                None if track % 2 == 0 => InsertTarget::FirstFree,
                None => InsertTarget::NewTrack,
            };
            EditRequest::Insert(InsertReq {
                seq,
                target,
                at: f(*at),
                clips: vec![NewClip::media(
                    asset,
                    kind,
                    TimeRange::new(f(src), f(len)).ok()?,
                )],
            })
        }
        Op::InsertText { at, len, track } => EditRequest::Insert(InsertReq {
            seq,
            // Any track (redirected when it isn't a Text track), else
            // first free or a new track.
            target: match tracks.get(usize::from(*track)) {
                Some(t) if track % 3 != 0 => InsertTarget::Track(*t),
                _ if track % 2 == 0 => InsertTarget::FirstFree,
                _ => InsertTarget::NewTrack,
            },
            at: f(*at),
            clips: vec![NewClip::text(TextClipData::default(), f(*len))],
        }),
        Op::SetText {
            pick: p,
            count,
            which,
            value,
        } => {
            // Mostly text clips, else any clips.
            let texts: Vec<ClipId> = fx
                .seq()
                .tracks()
                .flat_map(|(_, t)| t.clips().iter())
                .filter(|c| c.content.text().is_some())
                .map(|c| c.id)
                .collect();
            let clips = if texts.is_empty() || p % 5 == 0 {
                pick(fx, *p, *count)
            } else {
                vec![texts[usize::from(*p) % texts.len()]]
            };
            let v = *value;
            let small = v.rem_euclid(4);
            let style = |edit| EditRequest::SetTextStyle {
                seq,
                clips: clips.clone(),
                edit,
            };
            let layout = |edit| EditRequest::SetTextLayout {
                seq,
                clips: clips.clone(),
                edit,
            };
            match which {
                0 => EditRequest::SetTextContent {
                    seq,
                    clip: *clips.first()?,
                    content: RichText::new(
                        ["", "Your text", "straße\nın İzmir. ok", "مرحبا 👋🏽"][small as usize],
                    ),
                },
                1 => style(TextStyleEdit::Font(FontRef::new(
                    ["Noto Sans", "Noto Serif", "", "Missing"][small as usize],
                ))),
                2 => style(TextStyleEdit::Size(RefPx::from_hundredths(v))),
                3 => style(TextStyleEdit::Fill(
                    Rgba8::new(v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8).into(),
                )),
                4 => style(TextStyleEdit::Bold(v % 2 == 0)),
                5 => style(TextStyleEdit::Italic(v % 2 == 0)),
                6 => layout(TextLayoutEdit::Align(
                    [HAlign::Left, HAlign::Center, HAlign::Right, HAlign::Center][small as usize],
                )),
                7 => layout(TextLayoutEdit::BoxWidth(
                    (small != 0).then_some(RefPx::from_hundredths(v)),
                )),
                8 => layout(TextLayoutEdit::LineSpacing(PerMille::new(v as u16))),
                9 => layout(TextLayoutEdit::Justify(v % 2 == 0)),
                10 => layout(TextLayoutEdit::VAnchor(
                    [
                        VAnchor::Top,
                        VAnchor::Middle,
                        VAnchor::Bottom,
                        VAnchor::Middle,
                    ][small as usize],
                )),
                11 => {
                    let cards = TextCard::ALL;
                    let lenses = [TextLens::Content, TextLens::Style, TextLens::BoxAndBubble];
                    let i = v.rem_euclid(14) as usize;
                    EditRequest::ResetText {
                        seq,
                        clips: clips.clone(),
                        scope: match cards.get(i) {
                            Some(card) => ResetScope::Card(*card),
                            None => ResetScope::Lens(lenses[i - cards.len()]),
                        },
                    }
                }
                12 => {
                    let clip = *clips.first()?;
                    let len = fx.doc().clip(clip)?.content.text()?.content.len();
                    let a = (v as u16 as usize) % (len + 2);
                    let b = ((v >> 16) as u16 as usize) % (len + 2);
                    let case = [
                        CaseTransform::Upper,
                        CaseTransform::Lower,
                        CaseTransform::Title,
                        CaseTransform::Sentence,
                    ][small as usize];
                    let tr = LangTag::new("tr");
                    let lang = (v % 3 == 0).then_some(&tr);
                    change_case(fx.doc(), seq, clip, a.min(b)..a.max(b), case, lang).ok()?
                }
                k => style(style_edit(k - TEXT_OPS, v)),
            }
        }
        Op::TextLayer { pick: p, layer, on } => {
            let texts: Vec<ClipId> = fx
                .seq()
                .tracks()
                .flat_map(|(_, t)| t.clips().iter())
                .filter(|c| c.content.text().is_some())
                .map(|c| c.id)
                .collect();
            // Usually one clip, sometimes every text clip.
            let clips = match texts.len() {
                0 => return None,
                n if p % 4 == 0 => texts[..n].to_vec(),
                n => vec![texts[usize::from(*p) % n]],
            };
            EditRequest::SetTextStyle {
                seq,
                clips,
                edit: TextStyleEdit::SetLayer {
                    layer: LAYERS
                        .get(usize::from(*layer))
                        .copied()
                        .unwrap_or(if layer % 2 == 0 {
                            TextLayerKind::Background
                        } else {
                            TextLayerKind::Bubble
                        }),
                    on: *on,
                },
            }
        }
        Op::Color {
            pick: p,
            count,
            which,
            value,
        } => {
            use freecut_timeline::edit::ColorEdit as E;
            use freecut_timeline::model::{
                ColorParamId as C, CurveChannel, CurvePoint, HueBand, Section as S, WheelId,
            };
            let v = *value;
            let param = C::ALL[(v as u32 as usize) % C::ALL.len()];
            let section = S::ALL[(v as u32 as usize) % S::ALL.len()];
            let band = HueBand::ALL[(v as u32 as usize) % 8];
            let wheel = WheelId::ALL[(v as u32 as usize) % 4];
            let channel = CurveChannel::ALL[(v as u32 as usize >> 3) % 4];
            // Points from the bits of `v`, often refused by `Curve::new`
            // (then the identity curve).
            let curve = {
                let n = 2 + (v as u32 as usize >> 5) % 5;
                let points = (0..n)
                    .map(|i| {
                        let x = (i as u32 * 10_000 / (n as u32 - 1)) as u16;
                        let y = ((v as u32).rotate_left(i as u32 * 7) % 10_001) as u16;
                        CurvePoint::new(x, y)
                    })
                    .collect();
                freecut_timeline::model::Curve::new(points).unwrap_or_default()
            };
            EditRequest::EditColor {
                seq,
                clips: pick(fx, *p, *count),
                edit: match which {
                    0 | 1 => E::SetParam(param, (v >> 8) as i16),
                    2 => E::SetParams(vec![(param, (v >> 4) as i16), (C::Hue, v as i16)]),
                    3 => E::SetSectionEnabled(section, v % 2 == 0),
                    4 => E::SetBypass(v % 2 == 0),
                    5 | 6 => E::ResetSection(section),
                    8 => E::SetCurve { channel, curve },
                    9 => E::SetPuck {
                        wheel,
                        x: (v >> 16) as i16,
                        y: v as i16,
                    },
                    10 => E::ResetBand(band),
                    11 => E::ResetWheel(wheel),
                    12 => E::SetParam(C::HSL[(v as u32 as usize) % 24], (v >> 3) as i16),
                    14 => E::SetLut(Some(freecut_timeline::model::LutSlot {
                        intensity: (v >> 8) as u8,
                        ..freecut_timeline::model::LutSlot::new(
                            freecut_timeline::model::LutRef::new(
                                [(v & 3) as u8; 16],
                                format!("l{}.cube", v & 3),
                            ),
                        )
                    })),
                    15 => E::SetLut(None),
                    16 => E::SetSkin {
                        scope: freecut_timeline::model::SkinScope::ALL[(v as u32 as usize) % 2],
                        on: v % 3 != 0,
                    },
                    17 => E::SetLutInterp(
                        freecut_timeline::model::LutInterp::ALL[(v as u32 as usize) % 2],
                    ),
                    18 => E::SetParam(C::LUT[(v as u32 as usize) % 2], (v >> 2) as i16),
                    _ => E::ResetAll,
                },
            }
        }
        Op::SetTransform {
            pick: p,
            count,
            which,
            value,
        } => {
            let v = *value;
            EditRequest::SetTransform {
                seq,
                clips: pick(fx, *p, *count),
                edit: match which {
                    0 => TransformEdit::Position(CanvasPos::new(v, v.rotate_left(7))),
                    1 => TransformEdit::Scale(Scale::from_basis_points(v)),
                    2 => TransformEdit::Rotation(Angle::from_hundredths(v)),
                    3 => TransformEdit::FlipH(v % 2 == 0),
                    4 => TransformEdit::FlipV(v % 2 == 0),
                    5 => TransformEdit::Opacity(PerMille::new(v as u16)),
                    _ => TransformEdit::Reset,
                },
            }
        }
        Op::Move {
            pick: p,
            count,
            delta,
            shift,
        } => EditRequest::Move(MoveReq {
            seq,
            clips: pick(fx, *p, *count),
            delta: f(*delta),
            track_shift: i32::from(*shift),
        }),
        Op::Trim {
            pick: p,
            out,
            delta,
            ripple,
        } => EditRequest::Trim(TrimReq {
            seq,
            clip: *pick(fx, *p, 1).first()?,
            edge: if *out { Edge::Out } else { Edge::In },
            delta: f(*delta),
            ripple: *ripple,
        }),
        Op::Split { pick: p, at, all } => EditRequest::Split(SplitReq {
            seq,
            at: f(*at),
            targets: if *all {
                SplitTargets::AllTracks
            } else {
                SplitTargets::Clips(pick(fx, *p, 1))
            },
        }),
        Op::Delete {
            pick: p,
            count,
            ripple,
        } => EditRequest::Delete(DeleteReq {
            seq,
            clips: pick(fx, *p, *count),
            ripple: *ripple,
        }),
        Op::AddTrack { kind } => EditRequest::AddTrack(AddTrackReq {
            seq,
            kind: [TrackKind::Video, TrackKind::Audio, TrackKind::Sticker][usize::from(*kind)],
            position: TrackPosition::Default,
        }),
        Op::Lock { track, locked } => {
            let track = *tracks.get(usize::from(*track))?;
            EditRequest::SetTrackHeader {
                seq,
                track,
                header: TrackHeader {
                    name: None,
                    flags: TrackFlags {
                        locked: *locked,
                        ..TrackFlags::default()
                    },
                },
            }
        }
        Op::Magnet { on } => EditRequest::SetMagnet { seq, on: *on },
        Op::Conform => EditRequest::Conform { seq },
        Op::Detach { pick: p, count } => EditRequest::DetachAudio {
            seq,
            clips: pick(fx, *p, *count),
        },
        Op::Restore { pick: p, count } => {
            // Mostly clips whose sound is detached, else any clips.
            let detached: Vec<ClipId> = fx
                .seq()
                .tracks()
                .flat_map(|(_, t)| t.clips().iter())
                .filter(|c| c.audio.detached().is_some())
                .map(|c| c.id)
                .collect();
            let clips = if detached.is_empty() || p % 4 == 0 {
                pick(fx, *p, *count)
            } else {
                vec![detached[usize::from(*p) % detached.len()]]
            };
            EditRequest::RestoreAudio { seq, clips }
        }
        Op::SetAudio {
            pick: p,
            count,
            which,
            value,
        } => EditRequest::SetClipAudio {
            seq,
            clips: pick(fx, *p, *count),
            edit: match which {
                0 => AudioEdit::Gain(GainDb::from_hundredths(i32::try_from(*value * 100).ok()?)),
                1 => AudioEdit::Muted(*value % 2 == 0),
                2 => AudioEdit::FadeIn(f(*value)),
                3 => AudioEdit::FadeOut(f(*value)),
                4 => AudioEdit::FadeInCurve(FadeCurve::ALL[value.rem_euclid(4) as usize]),
                _ => AudioEdit::FadeOutCurve(FadeCurve::ALL[value.rem_euclid(4) as usize]),
            },
        },
        Op::GainKeys {
            pick: p,
            count,
            which,
            at,
            to,
            value,
        } => {
            let clips = pick(fx, *p, *count);
            // Times near the first clip's source, or one of its keys.
            let first = fx.doc().clip(*clips.first()?)?;
            let base = first.content.source().map_or(f(0), |s| s.start());
            let near = |n: i64| base.checked_add(f(n)).ok();
            let keys: Vec<_> = first
                .audio
                .active()
                .map(|a| a.gain.keys().iter().map(|k| k.at).collect())
                .unwrap_or_default();
            let existing = |n: i64| {
                if keys.is_empty() || n % 5 == 0 {
                    near(n)
                } else {
                    Some(keys[n.rem_euclid(keys.len() as i64) as usize])
                }
            };
            let gain = GainDb::from_hundredths(if value % 7 == 0 {
                i32::MIN
            } else {
                value % 3_000
            });
            let edit = match which {
                0 => GainKeyEdit::Set {
                    at: near(*at)?,
                    gain: Some(gain),
                },
                1 => GainKeyEdit::Set {
                    at: near(*at)?,
                    gain: None,
                },
                2 => GainKeyEdit::Interp {
                    at: existing(*at)?,
                    interp: if value % 2 == 0 {
                        Interp::Hold
                    } else {
                        Interp::Linear
                    },
                },
                3 => GainKeyEdit::Move {
                    from: existing(*at)?,
                    to: near(*to)?,
                },
                4 => GainKeyEdit::Remove(vec![existing(*at)?, existing(*to)?]),
                _ => GainKeyEdit::Clear,
            };
            EditRequest::EditGainKeys { seq, clips, edit }
        }
        Op::Keys {
            pick: p,
            count,
            prop,
            which,
            at,
            to,
            value,
        } => {
            // Mostly text clips (they take every property), else any clips.
            let texts: Vec<ClipId> = fx
                .seq()
                .tracks()
                .flat_map(|(_, t)| t.clips().iter())
                .filter(|c| c.content.text().is_some())
                .map(|c| c.id)
                .collect();
            let clips = if texts.is_empty() || p % 3 == 0 {
                pick(fx, *p, *count)
            } else {
                vec![texts[usize::from(*p) % texts.len()]]
            };
            let first = fx.doc().clip(*clips.first()?)?.clone();
            let prop = key_prop(*prop, *value);
            // Times mostly inside the first clip, else near it, or one of
            // its keys.
            let frames = (first.duration().raw() / f(1).raw()).max(1);
            let near = |n: i64| {
                let n = if n % 5 == 0 { n } else { n.rem_euclid(frames) };
                first.start().checked_add(f(n)).ok()
            };
            let keyed: Vec<_> = first
                .keys
                .keys(prop)
                .iter()
                .filter_map(|k| first.sequence_time_of(k.at))
                .collect();
            let existing = |n: i64| {
                if keyed.is_empty() || n % 4 == 0 {
                    near(n)
                } else {
                    Some(keyed[n.rem_euclid(keyed.len() as i64) as usize])
                }
            };
            let cards = [
                PropGroup::Transform,
                PropGroup::Text(TextCard::Type),
                PropGroup::Text(TextCard::Fill),
                PropGroup::Text(TextCard::Outline),
                PropGroup::Text(TextCard::Shadow),
                PropGroup::Text(TextCard::Bubble),
            ];
            let card = cards[value.rem_euclid(cards.len() as i32) as usize];
            let edit = match which {
                0 => KeyEdit::Add {
                    at: near(*at)?,
                    value: Some(key_value(prop, *value)),
                },
                1 => KeyEdit::Add {
                    at: near(*at)?,
                    value: None,
                },
                2 => KeyEdit::Remove { at: existing(*at)? },
                3 => KeyEdit::Move {
                    from: existing(*at)?,
                    to: near(*to)?,
                },
                4 => KeyEdit::SetInterp {
                    at: existing(*at)?,
                    interp: Interp::PRESETS[value.rem_euclid(18) as usize],
                },
                5 => KeyEdit::KeyCard {
                    card,
                    at: near(*at)?,
                },
                6 => KeyEdit::ClearCard {
                    card,
                    at: Some(existing(*at)?),
                },
                _ => KeyEdit::ClearCard { card, at: None },
            };
            EditRequest::EditKeys {
                seq,
                clips,
                prop,
                edit,
            }
        }
        Op::Anim {
            pick: p,
            count,
            slot,
            preset,
            len,
            value,
        } => {
            let slot = [Slot::Entrance, Slot::Exit, Slot::Continuous][usize::from(*slot)];
            let ids = [
                "fc.in.fade",
                "fc.in.typewriter",
                "fc.loop.wave",
                "fc.out.slide",
                "fc.in.pop",
                "fc.unknown",
            ];
            let spec = ids.get(usize::from(*preset)).map(|id| {
                let v = *value;
                let mut s = AnimationSpec::new(PresetId::new(id), f(*len));
                s.params.intensity = PerMille::new(v as u16 % 1500);
                s.params.overlap = PerMille::new((v >> 8) as u16 % 1500);
                s.params.order = [
                    AnimOrder::Forward,
                    AnimOrder::Backward,
                    AnimOrder::CenterOut,
                    AnimOrder::EdgesIn,
                    AnimOrder::Random,
                ][v.rem_euclid(5) as usize];
                s.params.seed = v as u32;
                s.params.unit = [
                    None,
                    Some(AnimUnit::Letter),
                    Some(AnimUnit::Word),
                    Some(AnimUnit::Line),
                ][(v >> 3).rem_euclid(4) as usize];
                s.params.pace = (v % 3 == 0).then(|| f(1));
                s.params.travel = if v % 2 == 0 {
                    Travel::FromEdge
                } else {
                    Travel::Short
                };
                s.direction = [None, Some(Direction::Right), Some(Direction::Up)]
                    [(v >> 5).rem_euclid(3) as usize];
                s.easing = (v % 5 == 0).then(|| Interp::PRESETS[(v >> 2).rem_euclid(18) as usize]);
                s
            });
            EditRequest::SetClipAnimation {
                seq,
                clips: pick(fx, *p, *count),
                slot,
                spec,
            }
        }
        Op::MasterGain { value } => EditRequest::SetMasterGain {
            seq,
            gain: GainDb::from_hundredths(*value),
        },
        Op::TrackStage { track, pan, value } => {
            let track = *tracks.get(usize::from(*track))?;
            if *pan {
                EditRequest::SetTrackPan {
                    seq,
                    track,
                    pan: freecut_timeline::model::Pan::new(*value % 150),
                }
            } else {
                EditRequest::SetTrackGain {
                    seq,
                    track,
                    gain: GainDb::from_hundredths(if value % 9 == 0 {
                        i32::MIN
                    } else {
                        value % 3_000
                    }),
                }
            }
        }
        Op::Stream {
            pick: p,
            count,
            which,
        } => EditRequest::SetAudioStream {
            seq,
            clips: pick(fx, *p, *count),
            sel: match which {
                0 => None,
                1 => Some(StreamSel::AllMixed),
                n => Some(StreamSel::Index(u32::from(*n) - 2)),
            },
        },
        Op::AddStream { pick: p, stream } => {
            // Mostly video clips of files with several streams, with one of
            // their streams; else any clip and stream number.
            let library = fx.doc().library();
            let multi: Vec<(ClipId, Vec<u32>)> = fx
                .seq()
                .tracks()
                .flat_map(|(_, t)| t.clips().iter())
                .filter(|c| {
                    matches!(
                        c.content,
                        ClipContent::Media {
                            kind: MediaKind::Video,
                            ..
                        }
                    )
                })
                .filter_map(|c| {
                    let info = &library.get(c.content.asset()?)?.info;
                    (info.audio.len() > 1)
                        .then(|| (c.id, info.audio.iter().map(|a| a.index).collect()))
                })
                .collect();
            let (clip, stream) = if multi.is_empty() || p % 5 == 0 {
                (*pick(fx, *p, 1).first()?, u32::from(*stream))
            } else {
                let (clip, streams) = &multi[usize::from(*p) % multi.len()];
                (*clip, streams[usize::from(*stream) % streams.len()])
            };
            EditRequest::AddStreamAsClip { seq, clip, stream }
        }
        Op::Slip { pick: p, delta } => {
            let linked: Vec<ClipId> = fx
                .seq()
                .audio_tracks()
                .iter()
                .flat_map(|t| t.clips().iter())
                .filter(|c| c.link.is_some())
                .map(|c| c.id)
                .collect();
            EditRequest::Move(MoveReq {
                seq,
                clips: vec![*linked.get(usize::from(*p) % linked.len().max(1))?],
                delta: f(*delta),
                track_shift: 0,
            })
        }
        Op::Resync { pick: p, count } => {
            // Mostly out-of-sync clips, else any clips.
            let slipped: Vec<ClipId> = sync_offsets(fx.doc(), seq).into_keys().collect();
            let clips = if slipped.is_empty() || p % 4 == 0 {
                pick(fx, *p, *count)
            } else {
                vec![slipped[usize::from(*p) % slipped.len()]]
            };
            EditRequest::Resync { seq, clips }
        }
        Op::Extract {
            pick,
            default_stream,
        } => {
            let items: Vec<_> = fx.doc().library().items().cloned().collect();
            let item = items.get(usize::from(*pick) % items.len().max(1))?;
            let stream = if *default_stream {
                None
            } else {
                // Sometimes a stream that doesn't exist.
                Some(item.info.audio.first().map_or(9, |a| a.index))
            };
            EditRequest::ExtractAudio {
                media: item.id,
                stream,
            }
        }
        Op::Import {
            frames,
            video,
            sound,
            streams,
            default,
        } => {
            let info = if *sound {
                media_info_streams(*frames, *video, *streams, *default)
            } else {
                media_info_with(*frames, *video, false)
            };
            EditRequest::ImportMedia(vec![NewMedia {
                path: StoredPath {
                    absolute: format!("/import/{frames}"),
                    relative: None,
                },
                info,
                fingerprint: None,
                display_name: None,
            }])
        }
        Op::RemoveMedia { pick, with_clips } => {
            let items: Vec<_> = fx.doc().library().items().map(|m| m.id).collect();
            let media = *items.get(usize::from(*pick) % items.len().max(1))?;
            EditRequest::RemoveMedia {
                media: vec![media],
                with_clips: *with_clips,
            }
        }
        Op::Relink { pick, frames } => {
            let items: Vec<_> = fx.doc().library().items().cloned().collect();
            let item = items.get(usize::from(*pick) % items.len().max(1))?;
            let mut info = item.info.clone();
            info.duration = f(*frames);
            EditRequest::RelinkMedia {
                media: item.id,
                path: StoredPath {
                    absolute: format!("/relinked/{frames}"),
                    relative: None,
                },
                info,
                fingerprint: None,
            }
        }
        Op::RateChange => {
            let current = *fx.doc().settings();
            let other = if current.frame_rate == rate() {
                FrameRate::new(30, 1).unwrap()
            } else {
                rate()
            };
            EditRequest::SetProjectSettings {
                settings: ProjectSettings {
                    frame_rate: other,
                    ..current
                },
                conform: false,
            }
        }
        Op::CutAway {
            pick: p,
            count,
            at,
            head,
            ripple,
        } => EditRequest::TrimToPlayhead {
            seq,
            targets: if *count == 0 {
                TrimTargets::MainUnderPlayhead
            } else {
                TrimTargets::Clips(pick(fx, *p, *count))
            },
            at: f(*at),
            cut: if *head { Cut::Head } else { Cut::Tail },
            ripple_others: *ripple,
        },
        Op::Marker {
            pick: p,
            which,
            at,
            value,
        } => marker_request(fx, *p, *which, *at, *value)?,
        Op::Follow { on, kinds } => match kinds {
            Some(bits) => EditRequest::SetFollowKinds {
                seq,
                kinds: FollowKinds::from_bits(*bits),
            },
            None => EditRequest::SetFollowMain { seq, on: *on },
        },
        Op::Record {
            at,
            handle,
            frames,
            new_track,
            replace,
        } => {
            let takes: Vec<ClipId> = fx
                .seq()
                .audio_tracks()
                .iter()
                .flat_map(|t| t.clips().iter())
                .map(|c| c.id)
                .collect();
            EditRequest::InsertRecordedClip {
                seq,
                media: NewMedia {
                    path: StoredPath {
                        absolute: format!("/rec/{frames}.wav"),
                        relative: None,
                    },
                    info: media_info(*frames, false),
                    fingerprint: None,
                    display_name: None,
                },
                // Off the grid now and then, like a latency-compensated start.
                at: Tick::from_raw(f(*at).raw() + f(1).raw() * (*at % 3) / 7),
                handle_in: f(*handle),
                target: if *new_track {
                    RecordTarget::NewTrack
                } else {
                    RecordTarget::FirstFree
                },
                replace: if *replace {
                    takes.first().copied()
                } else {
                    None
                },
            }
        }
        _ => return None,
    })
}

/// A marker request: add (timeline, clip or auto), edit, move or delete.
fn marker_request(fx: &Fixture, p: u16, which: u8, at: i64, value: i32) -> Option<EditRequest> {
    let seq = fx.seq_id();
    let mut refs: Vec<MarkerRef> = fx
        .seq()
        .markers
        .iter()
        .map(|m| MarkerRef::Timeline(m.id))
        .collect();
    for (_, t) in fx.seq().tracks() {
        for c in t.clips() {
            refs.extend(c.markers.iter().map(|m| MarkerRef::Clip(c.id, m.id)));
        }
    }
    let marker = refs.get(usize::from(p) % refs.len().max(1)).copied();
    let color = PaletteColor::ALL[value.rem_euclid(8) as usize];
    Some(match which {
        0 | 1 => EditRequest::AddMarker {
            seq,
            at: f(at),
            owner: MarkerOwner::Timeline,
            color: (value % 2 == 0).then_some(color),
        },
        2 | 3 => EditRequest::AddMarker {
            seq,
            at: f(at),
            owner: if which == 2 {
                MarkerOwner::Auto(pick(fx, p, 1))
            } else {
                MarkerOwner::Clip(*pick(fx, p, 1).first()?)
            },
            color: None,
        },
        4 | 5 => EditRequest::EditMarker {
            seq,
            marker: marker?,
            patch: MarkerPatch {
                name: (value % 3 == 0).then(|| format!("M {value}")),
                note: (value % 5 == 0).then(|| "note".into()),
                color: (value % 2 == 0).then_some(color),
                duration: (value % 7 != 0).then(|| f(i64::from(value.rem_euclid(30)) - 3)),
            },
        },
        6 | 7 => EditRequest::MoveMarker {
            seq,
            marker: marker?,
            to: f(at),
        },
        _ => EditRequest::DeleteMarkers {
            seq,
            which: match value.rem_euclid(4) {
                0 => MarkerSelector::Refs(marker.into_iter().collect()),
                1 => MarkerSelector::Color(color),
                2 => MarkerSelector::OnClip(*pick(fx, p, 1).first()?),
                _ => MarkerSelector::AllTimeline,
            },
        },
    })
}

/// TL-029's property: after an edit, every attached item on an unlocked
/// track starts at its owner's anchor frame, unless it was already out of
/// sync before (an offset left by a lock is kept, never created).
fn check_follow(before: &Document, after: &Document) -> Result<(), TestCaseError> {
    for seq in after.sequences() {
        for (_, track) in seq.tracks() {
            if track.is_locked() {
                continue;
            }
            for clip in track.clips() {
                let Some(a) = clip.attach else { continue };
                if expected_start(seq, a) == Some(clip.start()) {
                    continue;
                }
                let was_out = before.sequence(seq.id).is_some_and(|bs| {
                    bs.find_clip(clip.id).is_some_and(|(_, bt)| {
                        bt.is_locked()
                            || bt.clip(clip.id).is_some_and(|o| {
                                o.attach
                                    .is_some_and(|oa| expected_start(bs, oa) != Some(o.start()))
                            })
                    })
                });
                prop_assert!(
                    was_out,
                    "clip {} left its anchor {:?}: starts {:?}, expected {:?}",
                    clip.id,
                    a,
                    clip.start(),
                    expected_start(seq, a)
                );
            }
        }
    }
    Ok(())
}

/// The (file, container stream) pairs clip `c` plays when its sound is
/// active: an extracted item plays its stream of the original file (AU-004),
/// anything else the streams its selection picks (AU-038).
fn played(doc: &Document, c: &Clip) -> Vec<(MediaId, u32)> {
    let ClipContent::Media { asset, stream, .. } = c.content else {
        return Vec::new();
    };
    let Some(item) = doc.library().get(asset) else {
        return Vec::new();
    };
    if c.audio.active().is_none() {
        return Vec::new();
    }
    match item.source {
        MediaSource::AudioOf { source, stream } => vec![(source, stream)],
        _ => item
            .info
            .played_streams(stream)
            .iter()
            .map(|s| (asset, s.index))
            .collect(),
    }
}

/// Semantic checks of the link edits, which `validate` can't see because
/// lonely links dissolve quietly (TL-030, AU-038): Detach audio and Add
/// stream as clip never take a clip out of its link, and neither a stream
/// added as a clip nor a clip whose stream choice changed sounds twice with
/// its link: no other clip of the link overlapping it plays the same (file,
/// stream).
fn check_link_edit(op: &Op, before: &Document, after: &Document) -> Result<(), TestCaseError> {
    if !matches!(
        op,
        Op::Detach { .. } | Op::AddStream { .. } | Op::Stream { .. }
    ) {
        return Ok(());
    }
    let clips = |doc: &Document| -> Vec<Arc<Clip>> {
        doc.sequences()
            .flat_map(|s| {
                s.tracks()
                    .flat_map(|(_, t)| t.clips().iter().cloned())
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    for c in clips(before) {
        if let (Some(link), Some(now)) = (c.link, after.clip(c.id)) {
            prop_assert_eq!(now.link, Some(link), "{:?} left its link in {:?}", c.id, op);
        }
    }
    if matches!(op, Op::Detach { .. }) {
        return Ok(());
    }
    let all = clips(after);
    // New clips (Add stream as clip) and clips whose stream choice changed
    // (Sound source).
    let changed = |c: &Arc<Clip>| match before.clip(c.id) {
        None => true,
        Some(old) => old.content != c.content,
    };
    for new in all.iter().filter(|c| changed(c)) {
        let Some(link) = new.link else { continue };
        let mine = played(after, new);
        for other in all
            .iter()
            .filter(|o| o.id != new.id && o.link == Some(link) && o.range.overlaps(new.range))
        {
            for pair in played(after, other) {
                prop_assert!(
                    !mine.contains(&pair),
                    "{:?} doubles {:?} of {:?}",
                    new.id,
                    pair,
                    other.id
                );
            }
        }
    }
    Ok(())
}

/// The `which` values of `Op::SetText` below the style edits.
const TEXT_OPS: u8 = 13;
/// How many round-2 style edits `style_edit` makes.
const STYLE_EDITS: u8 = 64;

/// A style edit of kind `k` (`0..STYLE_EDITS`) from the raw value `v`,
/// often out of range so the clamps are exercised.
fn style_edit(k: u8, v: i32) -> TextStyleEdit {
    use TextStyleEdit as E;
    let pm = PerMille::new(v as u16 % 2500);
    let spm = SignedPerMille::new(v as i16);
    let c = Rgba8::new(v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8);
    let b = v % 2 == 0;
    let n = v as u8;
    let angle = DeciDegrees::new(v);
    let layers = [
        TextLayerKind::Outline,
        TextLayerKind::Shadow,
        TextLayerKind::Glow,
        TextLayerKind::Background,
        TextLayerKind::Bubble,
        TextLayerKind::Curve,
    ];
    let pick = |len: usize| (v as u32 as usize) % len;
    let stop = |at: i32, color| GradientStop {
        at: PerMille::new((at.rem_euclid(1001)) as u16),
        color,
    };
    match k {
        0 => E::Underline(b),
        1 => E::Strike(b),
        2 => E::Case(
            [
                CaseTransform::AsTyped,
                CaseTransform::Upper,
                CaseTransform::Lower,
                CaseTransform::Title,
                CaseTransform::Sentence,
            ][pick(5)],
        ),
        3 => E::LetterSpacing(spm),
        4 => E::Fill(
            [
                Fill::Solid(c),
                Fill::Solid(c).with_kind(FillKind::LinearGradient),
                Fill::Solid(c).with_kind(FillKind::RadialGradient),
            ][pick(3)]
            .clone(),
        ),
        5 => E::FillKind(
            [
                FillKind::Solid,
                FillKind::LinearGradient,
                FillKind::RadialGradient,
            ][pick(3)],
        ),
        6 => E::FillStops(
            GradientStops::new(
                (0..2 + pick(7))
                    .map(|i| stop(v.wrapping_mul(i as i32 + 3), c))
                    .collect(),
            )
            .unwrap(),
        ),
        7 => E::FillStopColor {
            index: n % 9,
            color: c,
        },
        8 => E::FillAngle(angle),
        9 => E::FillCenter(FillCenter {
            x: pm,
            y: PerMille::new((v >> 16) as u16 % 1500),
        }),
        10 => E::FillSpread(pm),
        11 => E::FillExtent(if b {
            FillExtent::Block
        } else {
            FillExtent::Line
        }),
        // Layer switches are weighted up: they drive the exclusivity rule.
        12..=19 => E::SetLayer {
            layer: layers[pick(6)],
            on: b,
        },
        20 => E::OutlineColor(c),
        21 => E::OutlineWidth(pm),
        22 => E::OutlineOpacity(pm),
        23 => E::OutlinePosition(
            [
                StrokePosition::Outside,
                StrokePosition::Center,
                StrokePosition::Inside,
            ][pick(3)],
        ),
        24 => E::GlowColor(c),
        25 => E::GlowIntensity(pm),
        26 => E::GlowRadius(pm),
        27 => E::GlowOffset(pm),
        28 => E::GlowAngle(angle),
        29 => E::GlowDirection(
            [
                GlowDirection::Outward,
                GlowDirection::Inward,
                GlowDirection::Both,
            ][pick(3)],
        ),
        30 => E::ShadowColor(c),
        31 => E::ShadowOpacity(pm),
        32 => E::ShadowBlur(pm),
        33 => E::ShadowDistance(pm),
        34 => E::ShadowAngle(angle),
        35 => E::ShadowIncludeBox(b),
        36 => E::BackgroundColor(c),
        37 => E::BackgroundOpacity(pm),
        38 => E::BackgroundShape([BoxShape::Block, BoxShape::PerLine, BoxShape::PerChar][pick(3)]),
        39 => E::BackgroundPadX(pm),
        40 => E::BackgroundPadY(pm),
        41 => E::BackgroundRoundness(pm),
        42 => E::BackgroundHeight(if b {
            BoxHeight::Full
        } else {
            BoxHeight::LowerHalf
        }),
        43 => E::BackgroundBorder(b),
        44 => E::BackgroundBorderColor(c),
        45 => E::BackgroundBorderWidth(pm),
        46 => E::BubbleKind(
            [
                BubbleKind::Rounded,
                BubbleKind::Oval,
                BubbleKind::Thought,
                BubbleKind::Shout,
                BubbleKind::Square,
                BubbleKind::Tag,
                BubbleKind::ArrowTag { point: Side::Left },
                BubbleKind::ArrowTag { point: Side::Right },
                BubbleKind::Ticket,
            ][pick(9)],
        ),
        47 => E::BubbleTail(b),
        48 => E::BubbleColor(c),
        49 => E::BubbleOpacity(pm),
        50 => E::BubbleTailTip(TailTip {
            x: RefPx::from_hundredths(v),
            y: RefPx::from_hundredths(v.wrapping_mul(-7)),
        }),
        51 => E::BubbleTailBase(pm),
        52 => E::BubbleTailBend(spm),
        53 => E::BubblePadX(pm),
        54 => E::BubblePadY(pm),
        55 => E::BubbleRoundness(pm),
        56 => E::BubbleBorder(b),
        57 => E::BubbleBorderColor(c),
        58 => E::BubbleBorderWidth(pm),
        59 => E::BubbleSpikes(n),
        60 => E::BubbleSpikeDepth(pm),
        61 => [E::BubbleJitter(pm), E::BubbleSeed(v as u32)][pick(2)].clone(),
        62 => [E::BubbleLobe(pm), E::BubbleTrail(n)][pick(2)].clone(),
        _ => E::Curve(spm),
    }
}

const LAYERS: [TextLayerKind; 6] = [
    TextLayerKind::Outline,
    TextLayerKind::Shadow,
    TextLayerKind::Glow,
    TextLayerKind::Background,
    TextLayerKind::Bubble,
    TextLayerKind::Curve,
];

/// Whether `a` and `b` hold the same settings for `layer`.
fn same_layer_value(a: &TextStyle, b: &TextStyle, layer: TextLayerKind) -> bool {
    match layer {
        TextLayerKind::Outline => a.stroke.value == b.stroke.value,
        TextLayerKind::Shadow => a.shadow.value == b.shadow.value,
        TextLayerKind::Glow => a.glow.value == b.glow.value,
        TextLayerKind::Background => a.background.value == b.background.value,
        // Switching the bubble on may darken its fill (TX-055).
        TextLayerKind::Bubble => {
            a.bubble.value.spec == b.bubble.value.spec
                && a.bubble.value.border == b.bubble.value.border
        }
        TextLayerKind::Curve => a.curve.value == b.curve.value,
    }
}

/// TX-054 for one committed style edit: switching a layer off keeps its
/// value; switching Background or Bubble on switches the other off; every
/// other edit leaves the switches alone.
fn check_text_edit(
    req: &EditRequest,
    before: &Document,
    after: &Document,
) -> Result<(), TestCaseError> {
    let EditRequest::SetTextStyle { clips, edit, .. } = req else {
        return Ok(());
    };
    let style = |doc: &Document, id: ClipId| {
        doc.clip(id)
            .and_then(|c| c.content.text())
            .map(|t| t.style.clone())
    };
    for id in clips {
        let (Some(old), Some(new)) = (style(before, *id), style(after, *id)) else {
            continue;
        };
        match edit {
            TextStyleEdit::SetLayer { layer, on } => {
                prop_assert_eq!(new.layer_on(*layer), *on);
                prop_assert!(
                    same_layer_value(&old, &new, *layer),
                    "{:?} changed {:?}",
                    edit,
                    layer
                );
                for other in LAYERS.iter().filter(|l| *l != layer) {
                    let exclusive = *on
                        && matches!(
                            (layer, other),
                            (TextLayerKind::Background, TextLayerKind::Bubble)
                                | (TextLayerKind::Bubble, TextLayerKind::Background)
                        );
                    let want = if exclusive {
                        false
                    } else {
                        old.layer_on(*other)
                    };
                    prop_assert_eq!(new.layer_on(*other), want, "{:?} after {:?}", other, edit);
                }
            }
            _ => {
                for layer in LAYERS {
                    prop_assert_eq!(
                        new.layer_on(layer),
                        old.layer_on(layer),
                        "{:?} switched by {:?}",
                        layer,
                        edit
                    );
                }
            }
        }
    }
    Ok(())
}

/// A keyable property (and sometimes one that isn't).
fn key_prop(n: u8, v: i32) -> PropPath {
    use TextPropPath as T;
    match n {
        0 => PropPath::Transform(TransformProp::Position),
        1 => PropPath::Transform(TransformProp::Scale),
        2 => PropPath::Transform(TransformProp::Rotation),
        3 => PropPath::Transform(TransformProp::Opacity),
        4 => PropPath::Text(T::Size),
        5 => PropPath::Text(T::FillColor),
        6 => PropPath::Text(T::OutlineWidth),
        7 => PropPath::Text(T::ShadowAngle),
        8 => PropPath::Text(T::BubbleTailTip),
        9 => PropPath::Text(T::LetterSpacing),
        10 => PropPath::Text(T::LineSpacing),
        11 => PropPath::FillStopColor((v as u8) % 9),
        12 => PropPath::Reveal,
        13 => PropPath::Text(T::FillAngle),
        14 => PropPath::Text(T::Font),
        16 => PropPath::Color(freecut_timeline::model::ColorParamId::Exposure),
        17 => PropPath::Color(freecut_timeline::model::ColorParamId::Amount),
        18 => PropPath::ColorPuck(freecut_timeline::model::WheelId::ALL[(v as u32 as usize) % 4]),
        19 => PropPath::Color(freecut_timeline::model::ColorParamId::HSL[(v as u32 as usize) % 24]),
        20 => PropPath::Color(freecut_timeline::model::ColorParamId::WheelStrength),
        21 => PropPath::Color(freecut_timeline::model::ColorParamId::LutIntensity),
        22 => PropPath::Color(freecut_timeline::model::ColorParamId::SkinAmount(
            freecut_timeline::model::SkinScope::ALL[(v as u32 as usize) % 2],
        )),
        _ => PropPath::Text(T::GlowColor),
    }
}

/// A value for `prop` from the raw `v`, often out of range (and, now and
/// then, of the wrong kind).
fn key_value(prop: PropPath, v: i32) -> PropValue {
    use freecut_timeline::model::ValueKind as K;
    let c = Rgba8::new(v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8);
    if v % 23 == 0 {
        return PropValue::Ratio(PerMille::new(v as u16));
    }
    match prop.kind().unwrap_or(K::Ratio) {
        K::Position => PropValue::Position(CanvasPos::new(v, v.rotate_left(9))),
        K::Scale => PropValue::Scale(Scale::from_basis_points(v)),
        K::Angle => PropValue::Angle(Angle::from_hundredths(v)),
        K::Ratio => PropValue::Ratio(PerMille::new(v as u16)),
        K::Length => PropValue::Length(RefPx::from_hundredths(v)),
        K::SignedRatio => PropValue::SignedRatio(SignedPerMille::new(v as i16)),
        K::Direction => PropValue::Direction(DeciDegrees::new(v)),
        K::Color => PropValue::Color(c),
        K::TailTip => PropValue::TailTip(TailTip {
            x: RefPx::from_hundredths(v),
            y: RefPx::from_hundredths(v.rotate_left(5)),
        }),
        K::Level => PropValue::Level(v as i16),
        K::Puck => PropValue::Puck {
            x: v as i16,
            y: (v >> 16) as i16,
        },
    }
}

/// The frame plan of a random document never fails, keeps opacities in
/// range and hands per-unit work only to text layers.
fn check_plan(fx: &Fixture, frame: i64) -> Result<(), TestCaseError> {
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let none = BTreeSet::new();
    let plan = frame_plan_with(
        fx.doc(),
        fx.seq_id(),
        f(frame),
        &PlanInputs { offline: &none },
        &ctx,
    );
    let plan = plan.map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
    for PlanNode::Layer(l) in &plan.nodes {
        prop_assert!((0.0..=1.0).contains(&l.opacity), "opacity {}", l.opacity);
        prop_assert!(l.layer_to_canvas.is_finite());
        if let Some(c) = &l.color {
            prop_assert!(c.is_valid() && !c.is_identity(), "colour {c:?}");
        }
        if let Some(m) = &l.motion
            && m.text.is_some()
        {
            let text = matches!(l.source, freecut_timeline::plan::LayerSource::Text { .. });
            prop_assert!(text, "per-unit work on a picture");
        }
    }
    Ok(())
}

/// 512 cases, unless `PROPTEST_CASES` asks for another number.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(512)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// Random edits interleaved with undo, redo, gestures (committed,
    /// ended, cancelled), keyboard-repeat merging, saves and frame-rate
    /// changes. After every step the document validates (and Detach audio,
    /// Add stream as clip and Sound source pass [`check_link_edit`]);
    /// whenever the editor says it is clean, the document equals what was
    /// last saved; a cancelled gesture restores the document from before it
    /// began; and at the end, undoing everything restores the start and
    /// redoing everything restores the end.
    #[test]
    fn random_edits_keep_invariants_and_fully_undo(
        ops in proptest::collection::vec((op(), mode()), 1..50),
    ) {
        // The fixture's media plus a file with three audio streams (AU-038),
        // opened fresh so the import isn't in the history.
        let mut seeded = Fixture::new();
        seeded.import("/media/multi.mov", media_info_streams(120, true, 3, Some(1)));
        let mut fx = Fixture {
            editor: Editor::open(seeded.doc().clone(), IdAlloc::default(), HistoryLimits::default())
                .unwrap(),
        };
        let initial = fx.doc().clone();
        let mut saved_doc = initial.clone();
        let mut now_ms = 0u64;
        let mut gesture: Option<(GestureId, Document)> = None;
        let mut step = 0i64;
        for (op, mode) in &ops {
            match op {
                Op::Undo => { let _ = fx.editor.undo(); }
                Op::Redo => { let _ = fx.editor.redo(); }
                Op::BeginGesture => {
                    let before = fx.doc().clone();
                    gesture = Some((fx.editor.begin_gesture(), before));
                }
                Op::EndGesture => {
                    if let Some((g, _)) = gesture.take() {
                        fx.editor.end_gesture(g);
                    }
                }
                Op::CancelGesture => {
                    if let Some((g, before)) = gesture.take() {
                        fx.editor.cancel_gesture(g).unwrap();
                        prop_assert_eq!(fx.doc(), &before, "cancel restores the pre-gesture document");
                    }
                }
                Op::Save => {
                    saved_doc = fx.doc().clone();
                    let at = fx.editor.generation();
                    fx.editor.mark_saved_at(at);
                }
                _ => {
                    let Some(req) = request(&fx, op) else { continue };
                    let ctx = fx.ctx();
                    let coalesce = match (gesture.as_ref(), mode) {
                        (Some((g, _)), _) => Coalesce::Gesture(*g),
                        (None, Mode::Never) => Coalesce::Never,
                        (None, Mode::Repeat { key, dt_ms }) => {
                            now_ms += dt_ms;
                            let key = CoalesceKey { op: OpKind::MoveClips, target: u64::from(*key) };
                            Coalesce::Repeat { key, now_ms }
                        }
                    };
                    let before = fx.doc().clone();
                    match fx.editor.plan(&ctx, &req) {
                        Ok(plan) => {
                            prop_assert_eq!(fx.doc(), &before);
                            let meta = CommitMeta { selection_before: Vec::new(), coalesce };
                            let c = fx.editor.commit(plan, meta).unwrap();
                            if c == Committed::NoOp {
                                prop_assert_eq!(fx.doc(), &before);
                            }
                            check_link_edit(op, &before, fx.doc())?;
                            check_text_edit(&req, &before, fx.doc())?;
                            check_follow(&before, fx.doc())?;
                            step += 1;
                            check_plan(&fx, (step * 7) % 97)?;
                        }
                        Err(err) => {
                            let internal = matches!(
                                err,
                                freecut_timeline::edit::EditError::Internal(_)
                                    | freecut_timeline::edit::EditError::Invariant(_)
                            );
                            prop_assert!(!internal, "planner bug: {:?} for {:?}\n{}", err, op, fx.render());
                            prop_assert_eq!(fx.doc(), &before);
                        }
                    }
                }
            }
            fx.assert_valid();
            if !fx.editor.is_dirty() {
                prop_assert_eq!(fx.doc(), &saved_doc, "clean means the saved document");
            }
        }
        if let Some((g, _)) = gesture.take() {
            fx.editor.end_gesture(g);
        }
        let last = fx.doc().clone();
        let mut undone = 0;
        while fx.editor.undo().is_ok() {
            undone += 1;
            fx.assert_valid();
        }
        prop_assert_eq!(fx.doc(), &initial, "undoing everything restores the start");
        for _ in 0..undone {
            fx.editor.redo().unwrap();
        }
        prop_assert_eq!(fx.doc(), &last, "redoing everything restores the end");
    }
}
