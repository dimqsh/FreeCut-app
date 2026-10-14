//! Shared fixtures: a hand-built document that uses every field and enum
//! value, random valid documents built through the editor's planners, and
//! JSON helpers.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // test code

use core::num::NonZeroU64;
use std::sync::Arc;

use freecut_project::UiState;
use freecut_project::export::{
    AudioExportSettings, Container, ExportSettings, FileTarget, FrameRateChoice, OutputSize,
    QualityTier, ShortSide, VideoCodec, VideoSettings,
};
use freecut_timeline::edit::{
    AddTrackReq, AudioEdit, DeleteReq, Edge, EditCtx, EditRequest, GainKeyEdit, InsertReq,
    InsertTarget, KeyEdit, Limits, MarkerOwner, MarkerPatch, MarkerSelector, MoveReq, NewClip,
    NewMedia, RecordTarget, SplitReq, SplitTargets, TextLayoutEdit, TextStyleEdit, TrackPosition,
    TransformEdit, TrimReq,
};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, GroupId, LinkId, MarkerId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{
    Angle, Animatable, AudioStreamInfo, Border, BoxHeight, BoxShape, BubbleKind, BubbleSpec,
    CanvasPos, CaseTransform, Clip, ClipAudio, ClipContent, DeciDegrees, Decoration, Document,
    EmbeddedAudio, FadeCurve, Fill, FillCenter, FillExtent, FillKind, Fingerprint, FontRef, GainDb,
    GeneratedKind, Glow, GlowDirection, GradientStop, GradientStops, HAlign, Interp, Keyframe,
    Layer, Library, LinearGradient, MediaInfo, MediaItem, MediaKind, MediaSource, Pan, PerMille,
    ProjectSettings, RefPx, Rgba8, RichText, Rotation, Scale, Sequence, SequenceFlags, Shadow,
    Side, SignedPerMille, StoredPath, StreamSel, Stroke, StrokePosition, Tail, TailTip, TextBox,
    TextBubble, TextClipData, TextLayerKind, TextLayout, TextOrigin, TextStyle, Track, TrackAudio,
    TrackFlags, TrackHeader, TrackKind, Transform, VAnchor, VideoStreamInfo,
};
use freecut_timeline::model::{
    AnimOrder, AnimParams, AnimUnit, AnimationSpec, ClipAnimations, ClipKeys, Direction, EaseCurve,
    EaseDir, LoopSeams, PresetId, PropGroup, PropPath, PropValue, Slot, SlotPolicy, TextPropPath,
    TransformProp, Travel, ValueKind,
};
use freecut_timeline::model::{
    Attachment, FollowKind, FollowKinds, GeneratedMediaKind, Marker, MarkerOrigin, MarkerRef,
    Markers, MediaOrigin, PaletteColor,
};
use freecut_timeline::model::{
    AutoCorrection, AutoOffsets, AutoSource, AutoStyle, ColorParamId, ColorStack, Curve,
    CurveChannel, CurvePoint, HslChannel, HueBand, LutInterp, LutRef, LutSlot, MatchRef,
    MatchTransform, Section, SectionMask, SkinProtect, SkinScope, WheelId,
};
use freecut_timeline::time::{
    FrameRate, MarkRange, Rational, SampleRate, TICKS_PER_SECOND, Tick, TimeRange,
};
use freecut_timeline::{CommitMeta, Editor};
use proptest::prelude::*;
use serde_json::Value;

pub fn nz(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

// ---------------------------------------------------------------------------
// Kitchen sink
// ---------------------------------------------------------------------------

/// 29.97 fps, the rate of the kitchen-sink document.
pub fn ntsc() -> FrameRate {
    FrameRate::new(30_000, 1001).unwrap()
}

/// `n` frames at 29.97 fps.
pub fn fr(n: i64) -> Tick {
    ntsc().frames(n).unwrap()
}

fn range(start: Tick, duration: Tick) -> TimeRange {
    TimeRange::new(start, duration).unwrap()
}

fn video_stream(index: u32, rotation: Rotation) -> VideoStreamInfo {
    VideoStreamInfo {
        index,
        codec: "h264".into(),
        width: 1920,
        height: 1080,
        sar: (1, 1),
        rotation,
        mirrored: false,
        rate: Some(Rational::new(30_000, 1001).unwrap()),
        variable_rate: false,
        has_alpha: false,
        bit_depth: 8,
        start: Tick::ZERO,
        duration: fr(100),
    }
}

fn clip(id: u64, start: Tick, duration: Tick, content: ClipContent) -> Clip {
    Clip::new(ClipId::from_raw(nz(id)), range(start, duration), content)
}

fn media_clip(id: u64, at: Tick, asset: u64, kind: MediaKind, src: Tick, len: Tick) -> Clip {
    clip(
        id,
        at,
        len,
        ClipContent::Media {
            asset: MediaId::from_raw(nz(asset)),
            kind,
            source: range(src, len),
            stream: None,
        },
    )
}

fn generated(id: u64, at: Tick, len: Tick, kind: GeneratedKind) -> Clip {
    clip(id, at, len, ClipContent::Generated { kind })
}

fn track(id: u64, kind: TrackKind, header: TrackHeader, clips: Vec<Clip>) -> Arc<Track> {
    mixed_track(id, kind, header, TrackAudio::default(), clips)
}

fn mixed_track(
    id: u64,
    kind: TrackKind,
    header: TrackHeader,
    mix: TrackAudio,
    clips: Vec<Clip>,
) -> Arc<Track> {
    let mut track = Track::from_parts(
        TrackId::from_raw(nz(id)),
        kind,
        header,
        clips.into_iter().map(Arc::new).collect(),
    );
    track.mix = mix;
    Arc::new(track)
}

fn key_of(at: Tick, value: PropValue, interp: Interp) -> Keyframe<PropValue> {
    Keyframe { at, value, interp }
}

fn key(at: Tick, hundredths: i32, interp: Interp) -> Keyframe<GainDb> {
    Keyframe {
        at,
        value: if hundredths == i32::MIN {
            GainDb::SILENT
        } else {
            GainDb::from_hundredths(hundredths)
        },
        interp,
    }
}

fn header(name: Option<&str>, muted: bool, hidden: bool, locked: bool) -> TrackHeader {
    TrackHeader {
        name: name.map(String::from),
        flags: TrackFlags {
            muted,
            hidden,
            locked,
        },
    }
}

fn text_clip(id: u64, at: Tick, len: Tick, data: TextClipData) -> Clip {
    clip(id, at, len, ClipContent::Text(Arc::new(data)))
}

/// A document that uses every field, every enum value, groups, links, a
/// second sequence without the magnet, Windows-style and Unicode strings.
/// Its text: a title (43) with every style and layout value changed and a
/// transform (format 5: a linear gradient with a hard edge, underline,
/// strike, Title case, negative letter spacing, outline, shadow and glow on,
/// a background box switched off with its settings changed, an arrow-tag
/// bubble on with its tail and border off, a curve, justified and anchored
/// at the bottom), and a factory-default text clip (51). Its transforms: a
/// moved, scaled, rotated, mirrored, translucent video clip (42), a
/// vertically flipped sticker (44); every other clip has the identity.
/// Its sound: a detached video clip (40) with its linked audio clip (47),
/// audio clips with gain, mute and fades (47, 48; 48's fade-out is longer
/// than the clip), a silent (−∞ dB) video clip (42), a clip of extracted
/// audio (50 plays item 13, stream 5 of item 10), and a detached video
/// whose audio clip is gone (49, link 92 dissolved). Format 4's sound: the
/// beach video's streams carry probe tags (stream 5 is the default), clips
/// choose streams (40: index 5, 42: all mixed, 49: index 4), 47 has volume
/// keyframes (linear, hold, through −∞) and a smooth fade-in, 48 has fast
/// and slow fades, and three tracks have a non-default stage (20: −∞ and
/// full right, 26: −3 dB and 40 % left, 27: +12 dB and full left). Format
/// 6's motion: the title has keys on every kind of value with a non-zero
/// origin ([`kitchen_title_keys`]) and In, Out and Loop animations; the
/// overlay video (42) has an In and the sticker (44) a Loop. Format 7's
/// markers ([`kitchen_markers`]): two timeline markers in each sequence's
/// list and clip markers on 40, 43 and 49, with every colour and origin,
/// a range, a point, and one trimmed out of view. Format 8's following:
/// the root follows every kind but effects, with the title, sticker,
/// adjustment, voice and music clips attached to the intro (40); the
/// nested sequence doesn't follow, with the two other kinds checked.
/// Format 9's origins: a voiceover take (14) and a text-to-speech item
/// (15) beside the imported files. Returns it with its id counter.
/// The intro clip's colour stack (format 10): Light and Color values,
/// Light's Pivot under More, the Detail section switched off and Amount
/// 80 %.
pub fn kitchen_intro_color() -> ColorStack {
    let s = ColorStack::default()
        .with(ColorParamId::Brightness, 12)
        .with(ColorParamId::Contrast, -15)
        .with(ColorParamId::ContrastPivot, 30)
        .with(ColorParamId::ToneLift, 100)
        .with(ColorParamId::Temperature, 40)
        .with(ColorParamId::Saturation, -100)
        .with(ColorParamId::Amount, 80);
    let s = ColorStack {
        sections_enabled: s.sections_enabled.with(Section::Detail, false),
        ..s
    };
    kitchen_k6(kitchen_k5(kitchen_k3(kitchen_k2(s))))
}

/// Format 14's parts of the intro's stack: four colour masks out of id
/// order, one of each shape, with every value and mode changed somewhere
/// (a turned, unlinked, feathered, shrunk ellipse; an inverted, rounded
/// rectangle cut out; a linked band kept where it crosses; a line moved
/// down).
pub fn kitchen_k6(s: ColorStack) -> ColorStack {
    use freecut_timeline::model::{
        ColorMask, Mask, MaskCombine, MaskPoint, MaskShape, MaskShapeKind,
    };
    let ellipse = ColorMask {
        mask: Mask {
            center: MaskPoint::new(1_200, -800),
            size: MaskPoint::new(7_000, 4_500),
            linked: false,
            rotation: Angle::from_hundredths(1_550),
            feather: PerMille::new(350),
            ..Mask::new(MaskShapeKind::Ellipse)
        },
        expand: -20,
        ..ColorMask::new(1, Mask::new(MaskShapeKind::Ellipse))
    };
    let rect = ColorMask {
        mask: Mask {
            shape: MaskShape::Rectangle {
                roundness: PerMille::new(400),
            },
            invert: true,
            ..Mask::new(MaskShapeKind::Rectangle)
        },
        mode: MaskCombine::Subtract,
        ..ColorMask::new(3, Mask::new(MaskShapeKind::Rectangle))
    };
    let band = ColorMask {
        mask: Mask {
            shape: MaskShape::Band {
                width: PerMille::new(800),
            },
            rotation: Angle::from_hundredths(-9_000),
            linked: true,
            ..Mask::new(MaskShapeKind::Band)
        },
        mode: MaskCombine::Intersect,
        ..ColorMask::new(2, Mask::new(MaskShapeKind::Band))
    };
    let line = ColorMask::new(
        4,
        Mask {
            center: MaskPoint::new(0, 2_500),
            ..Mask::new(MaskShapeKind::Linear)
        },
    );
    ColorStack {
        masks: vec![ellipse, rect, band, line],
        ..s
    }
}

/// Format 13's parts of the intro's stack: a Vivid auto correction from
/// the playhead at Strength 70 (one offset held at its limit), and a match
/// to the overlay (42) measured ungraded at Intensity 80, with the Match
/// section switched off.
pub fn kitchen_k5(s: ColorStack) -> ColorStack {
    let s = s
        .with_auto(Some(AutoCorrection {
            strength: 70,
            ..AutoCorrection::new(
                AutoOffsets {
                    exposure: 12,
                    contrast: -5,
                    blacks: -100,
                    temperature: -8,
                    vibrance: 20,
                    saturation: 5,
                    ..AutoOffsets::default()
                },
                AutoStyle::Vivid,
                AutoSource::Playhead,
            )
        }))
        .with_match(Some(
            MatchTransform::new(
                [12_345, 9_000, 20_000],
                [-1_234, 56, -10_000],
                MatchRef::Clip {
                    clip: ClipId::from_raw(nz(42)),
                    at: Tick::from_raw(705_600_000),
                    graded: false,
                },
            )
            .with_intensity(80),
        ));
    ColorStack {
        sections_enabled: s.sections_enabled.with(Section::Match, false),
        ..s
    }
}

/// The still's match (format 13): to a library image, at Intensity 100.
pub fn kitchen_still_match() -> MatchTransform {
    MatchTransform::new(
        [10_000, 10_500, 9_500],
        [300, -20, 40],
        MatchRef::Media {
            media: MediaId::from_raw(nz(12)),
            at: Tick::ZERO,
        },
    )
}

/// The key path of the intro's mask `id`'s value `param`.
pub fn kitchen_mask_key(id: u16, param: &str) -> PropPath {
    PropPath::ColorMask {
        id,
        param: freecut_timeline::model::ColorMaskParam::parse(param).unwrap(),
    }
}

/// The LUT file the kitchen sink names (a made-up hash).
pub fn kitchen_lut() -> LutRef {
    LutRef::new(
        [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
            0x32, 0x10,
        ],
        "Teal \"night\" ✂.cube",
    )
}

/// Format 12's parts of the intro's stack: a LUT at 60 % sampled
/// trilinearly with skin protection at 40 %, and the Color section's skin
/// protection on at its default Amount.
pub fn kitchen_k3(s: ColorStack) -> ColorStack {
    s.with_lut(Some(LutSlot {
        intensity: 60,
        interp: LutInterp::Trilinear,
        skin: SkinProtect {
            on: true,
            amount: 40,
        },
        ..LutSlot::new(kitchen_lut())
    }))
    .with_skin(SkinScope::Color, true)
}

/// Format 11's parts of the intro's stack: HSL on three bands, a master
/// S-curve and a blue curve with moved endpoints, two pucks (one pulled
/// onto the rim), luminance levels and Strength 80 %, with the Curves
/// section switched off.
pub fn kitchen_k2(s: ColorStack) -> ColorStack {
    let hsl = |band, channel| ColorParamId::Hsl { band, channel };
    let s = s
        .with(hsl(HueBand::Red, HslChannel::Saturation), -40)
        .with(hsl(HueBand::Orange, HslChannel::Hue), 25)
        .with(hsl(HueBand::Blue, HslChannel::Lightness), -100)
        .with(hsl(HueBand::Magenta, HslChannel::Hue), 100)
        .with(ColorParamId::WheelLuma(WheelId::Highlights), -20)
        .with(ColorParamId::WheelLuma(WheelId::Shadows), 15)
        .with(ColorParamId::WheelStrength, 80)
        .with_puck(WheelId::Shadows, 120, -340)
        .with_puck(WheelId::Offset, -2_000, 2_000);
    let point = |x, y| CurvePoint::new(x, y);
    let s_curve = Curve::new(vec![
        point(0, 0),
        point(2_500, 2_000),
        point(7_500, 8_200),
        point(10_000, 10_000),
    ])
    .unwrap();
    let levels = Curve::new(vec![point(1_000, 500), point(9_000, 10_000)]).unwrap();
    let s = s
        .with_curve(CurveChannel::Luma, s_curve)
        .with_curve(CurveChannel::Blue, levels);
    ColorStack {
        sections_enabled: s.sections_enabled.with(Section::Curves, false),
        ..s
    }
}

pub fn kitchen_sink() -> (Document, NonZeroU64) {
    kitchen_sink_with_title(kitchen_title())
}

/// The kitchen sink title's text: every style and layout value changed.
pub fn kitchen_title() -> TextClipData {
    TextClipData {
        content: RichText::new("Beach day\n\"Sunset\" ☀️ \\ tab\there"),
        style: kitchen_title_style(),
        layout: TextLayout {
            align: HAlign::Right,
            justify: true,
            box_width: Some(RefPx::from_px(720)),
            line_spacing: PerMille::new(1_250),
            v_anchor: VAnchor::Bottom,
        },
        origin: TextOrigin::Manual,
    }
}

/// [`kitchen_sink`] with `title` as the text of its title clip (43).
pub fn kitchen_sink_with_title(title_text: TextClipData) -> (Document, NonZeroU64) {
    let settings = ProjectSettings {
        width: 1080,
        height: 1920,
        frame_rate: ntsc(),
        sample_rate: SampleRate::new(44_100).unwrap(),
        match_first_clip: true,
    };

    let mut v1 = video_stream(0, Rotation::None);
    v1.sar = (4, 3);
    v1.width = 1440;
    v1.has_alpha = true;
    v1.bit_depth = 10;
    v1.mirrored = true;
    let mut v2 = video_stream(1, Rotation::Cw90);
    v2.rate = None;
    v2.variable_rate = true;
    v2.start = Tick::from_raw(-1_000);
    let mut v3 = video_stream(2, Rotation::Cw180);
    v3.rate = Some(Rational::new(24, 1).unwrap());
    v3.codec = "hevc".into();
    let v4 = video_stream(3, Rotation::Cw270);
    let beach = MediaItem {
        origin: MediaOrigin::Imported,
        id: MediaId::from_raw(nz(10)),
        source: MediaSource::File,
        path: StoredPath {
            absolute: "/Users/me/Movies/Beach Day/IMG_0412.MOV".into(),
            relative: Some("../Movies/Beach Day/IMG_0412.MOV".into()),
        },
        info: MediaInfo {
            container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            default_video: Some(0),
            duration: fr(100),
            video: vec![v1, v2, v3, v4],
            audio: vec![
                AudioStreamInfo {
                    index: 4,
                    codec: "aac".into(),
                    sample_rate: 48_000,
                    channels: 2,
                    start: Tick::from_raw(-14_700),
                    duration: fr(100),
                    duration_exact: false,
                    default: false,
                    language: Some("eng".into()),
                    title: Some("Camera \"mic\" ✂".into()),
                    layout: Some("stereo".into()),
                },
                AudioStreamInfo {
                    index: 5,
                    codec: "pcm_s24le".into(),
                    sample_rate: 96_000,
                    channels: 6,
                    start: Tick::ZERO,
                    duration: fr(100),
                    duration_exact: true,
                    default: true,
                    language: None,
                    title: None,
                    layout: Some("5.1(side)".into()),
                },
            ],
            still: None,
        },
        fingerprint: Some(Fingerprint {
            size: 734_003_200,
            head_tail: 0xfedc_ba98_7654_3210,
            duration: fr(100),
        }),
        display_name: Some("Beach · day 1 \"wide\"\t🎬".into()),
    };
    let song = MediaItem {
        origin: MediaOrigin::Imported,
        id: MediaId::from_raw(nz(11)),
        source: MediaSource::File,
        path: StoredPath {
            absolute: r"C:\Users\me\Music\song.mp3".into(),
            relative: None,
        },
        info: MediaInfo {
            container: "mp3".into(),
            default_video: None,
            duration: Tick::from_raw(3_000_000_000),
            video: Vec::new(),
            audio: vec![AudioStreamInfo {
                index: 0,
                codec: "mp3".into(),
                sample_rate: 44_100,
                channels: 2,
                start: Tick::ZERO,
                duration: Tick::from_raw(3_000_000_000),
                duration_exact: true,
                default: false,
                language: None,
                title: None,
                layout: None,
            }],
            still: None,
        },
        fingerprint: None,
        display_name: None,
    };
    let photo = MediaItem {
        origin: MediaOrigin::Imported,
        id: MediaId::from_raw(nz(12)),
        source: MediaSource::File,
        path: StoredPath {
            absolute: "/Users/me/Pictures/sunset.png".into(),
            relative: Some("../Pictures/sunset.png".into()),
        },
        info: MediaInfo {
            container: "png_pipe".into(),
            default_video: None,
            duration: Tick::ZERO,
            video: Vec::new(),
            audio: Vec::new(),
            still: Some((4000, 3000)),
        },
        fingerprint: Some(Fingerprint {
            size: 12_345,
            head_tail: 7,
            duration: Tick::ZERO,
        }),
        display_name: Some(String::new()),
    };
    // The 6-channel stream of the beach video, extracted (AU-004).
    let extracted = MediaItem {
        origin: MediaOrigin::Imported,
        id: MediaId::from_raw(nz(13)),
        source: MediaSource::AudioOf {
            source: beach.id,
            stream: 5,
        },
        path: beach.path.clone(),
        info: beach.info.audio_only(5).unwrap(),
        fingerprint: beach.fingerprint,
        display_name: Some("Beach (audio)".into()),
    };
    // Format 9: a voiceover take and a text-to-speech file FreeCut made.
    let take = |id: u64, kind: GeneratedMediaKind, path: &str, name: Option<&str>| MediaItem {
        id: MediaId::from_raw(nz(id)),
        source: MediaSource::File,
        origin: MediaOrigin::Generated { kind },
        path: StoredPath {
            absolute: format!("/Users/me/Movies/Trip Media/Recordings/{path}"),
            relative: Some(format!("Trip Media/Recordings/{path}")),
        },
        info: MediaInfo {
            container: "wav".into(),
            default_video: None,
            duration: fr(40),
            video: Vec::new(),
            audio: vec![AudioStreamInfo {
                index: 0,
                codec: "pcm_s24le".into(),
                sample_rate: 48_000,
                channels: 1,
                start: Tick::ZERO,
                duration: fr(40),
                duration_exact: true,
                default: false,
                language: None,
                title: None,
                layout: Some("mono".into()),
            }],
            still: None,
        },
        fingerprint: None,
        display_name: name.map(String::from),
    };
    let voiceover = take(
        14,
        GeneratedMediaKind::Voiceover,
        "Voiceover 2026-10-12 14.32.05.wav",
        None,
    );
    let speech = take(
        15,
        GeneratedMediaKind::Tts,
        "Intro speech.wav",
        Some("Intro speech"),
    );
    let library = Library::from_parts(vec![
        Arc::new(beach),
        Arc::new(song),
        Arc::new(photo),
        Arc::new(extracted),
        Arc::new(voiceover),
        Arc::new(speech),
    ])
    .unwrap();

    let group = Some(GroupId::from_raw(nz(90)));
    let detached = LinkId::from_raw(nz(91));
    let link = Some(detached);

    let mut intro = media_clip(40, Tick::ZERO, 10, MediaKind::Video, fr(10), fr(90));
    intro.name = Some("Intro \"take\" 1 \\ ✂".into());
    intro.group = group;
    intro.link = link;
    intro.audio = EmbeddedAudio::Detached(detached);
    set_stream(&mut intro, Some(StreamSel::Index(5)));
    // Format 10: colour stacks on the intro (both sections, a switched-off
    // section, Amount and a keyframed slider), the still (bypassed) and
    // the adjustment clip (Mix).
    intro.color = Some(Arc::new(kitchen_intro_color()));
    intro.keys = ClipKeys::from_parts(
        Tick::ZERO,
        vec![
            (
                PropPath::Color(ColorParamId::Exposure),
                vec![
                    key_of(fr(10), PropValue::Level(-20), Interp::Linear),
                    key_of(
                        fr(40),
                        PropValue::Level(35),
                        Interp::Ease {
                            curve: EaseCurve::Quad,
                            dir: EaseDir::InOut,
                        },
                    ),
                ],
            ),
            // Format 11: a keyed puck and a keyed HSL slider.
            (
                PropPath::ColorPuck(WheelId::Midtones),
                vec![
                    key_of(fr(12), PropValue::Puck { x: -600, y: 0 }, Interp::Linear),
                    key_of(fr(30), PropValue::Puck { x: 300, y: 700 }, Interp::Hold),
                ],
            ),
            (
                PropPath::Color(ColorParamId::Hsl {
                    band: HueBand::Green,
                    channel: HslChannel::Saturation,
                }),
                vec![key_of(fr(20), PropValue::Level(-55), Interp::Linear)],
            ),
            // Format 12: a keyed LUT Intensity.
            (
                PropPath::Color(ColorParamId::LutIntensity),
                vec![
                    key_of(fr(15), PropValue::Level(20), Interp::Linear),
                    key_of(fr(35), PropValue::Level(90), Interp::Linear),
                ],
            ),
            // Format 13: a keyed Auto Strength and Match Intensity.
            (
                PropPath::Color(ColorParamId::AutoStrength),
                vec![
                    key_of(fr(11), PropValue::Level(0), Interp::Linear),
                    key_of(fr(31), PropValue::Level(100), Interp::Hold),
                ],
            ),
            (
                PropPath::Color(ColorParamId::MatchIntensity),
                vec![key_of(fr(25), PropValue::Level(45), Interp::Linear)],
            ),
            // Format 14: a moving ellipse with a keyed Expand, a keyed band
            // width and a keyed rectangle rotation.
            (
                kitchen_mask_key(1, "center"),
                vec![
                    key_of(
                        fr(14),
                        PropValue::MaskPoint(freecut_timeline::model::MaskPoint::new(-3_000, 500)),
                        Interp::Linear,
                    ),
                    key_of(
                        fr(44),
                        PropValue::MaskPoint(freecut_timeline::model::MaskPoint::new(2_500, -700)),
                        Interp::Ease {
                            curve: EaseCurve::Quad,
                            dir: EaseDir::Out,
                        },
                    ),
                ],
            ),
            (
                kitchen_mask_key(1, "expand"),
                vec![key_of(fr(16), PropValue::Level(35), Interp::Hold)],
            ),
            (
                kitchen_mask_key(2, "band_width"),
                vec![
                    key_of(fr(18), PropValue::Ratio(PerMille::new(300)), Interp::Linear),
                    key_of(
                        fr(28),
                        PropValue::Ratio(PerMille::new(1_900)),
                        Interp::Linear,
                    ),
                ],
            ),
            (
                kitchen_mask_key(3, "rotation"),
                vec![key_of(
                    fr(22),
                    PropValue::Angle(Angle::from_hundredths(-12_345)),
                    Interp::Linear,
                )],
            ),
        ],
    )
    .unwrap();
    let mut still = clip(
        41,
        fr(90),
        fr(60),
        ClipContent::Image {
            asset: MediaId::from_raw(nz(12)),
        },
    );
    still.group = group;
    still.enabled = false;
    still.color = Some(Arc::new(ColorStack {
        bypass: true,
        ..ColorStack::default()
            .with(ColorParamId::Hue, 60)
            .with_match(Some(kitchen_still_match()))
    }));
    let mut overlay = media_clip(42, fr(200), 10, MediaKind::Video, Tick::ZERO, fr(30));
    overlay.audio = EmbeddedAudio::Active(ClipAudio::with_gain(GainDb::SILENT));
    set_stream(&mut overlay, Some(StreamSel::AllMixed));
    overlay.transform = Transform {
        position: CanvasPos::new(250_000, -125_000),
        scale: Scale::from_basis_points(5_000),
        rotation: Angle::from_hundredths(-4_550),
        flip_h: true,
        flip_v: false,
        opacity: PerMille::new(750),
    };
    let mut title = text_clip(43, Tick::ZERO, fr(45), title_text);
    title.name = Some("Title".into());
    title.transform = Transform {
        position: CanvasPos::new(0, -350_000),
        rotation: Angle::from_degrees(370),
        ..Transform::IDENTITY
    };
    // Format 6: keyframes on the title (with a content-time origin, as
    // after a split) and its In, Out and Loop; an In on the overlay video
    // and a Loop on the sticker.
    title.keys = kitchen_title_keys();
    title.animations = kitchen_title_animations();
    overlay.animations = kitchen_overlay_animations();
    let caption = text_clip(51, fr(60), fr(30), TextClipData::default());
    let mut sticker = generated(44, fr(5), fr(20), GeneratedKind::Sticker);
    sticker.transform.flip_v = true;
    sticker.animations = kitchen_sticker_animations();
    let mut effect = generated(45, fr(10), fr(20), GeneratedKind::Effect);
    effect.enabled = false;
    let mut adjust = generated(46, fr(0), fr(150), GeneratedKind::Adjustment);
    // Format 14: a canvas-space mask, with the Masks section switched off.
    adjust.color = Some(Arc::new(ColorStack {
        sections_enabled: SectionMask::ALL_ON
            .with(Section::Color, false)
            .with(Section::Masks, false),
        masks: vec![freecut_timeline::model::ColorMask::new(
            1,
            freecut_timeline::model::Mask::new(freecut_timeline::model::MaskShapeKind::Ellipse),
        )],
        ..ColorStack::default()
            .with(ColorParamId::Amount, 45)
            .with(ColorParamId::Tint, -12)
    }));
    let mut voice = media_clip(47, Tick::ZERO, 10, MediaKind::Audio, fr(10), fr(90));
    voice.link = link;
    voice.audio = EmbeddedAudio::Active(ClipAudio {
        gain: Animatable::from_parts(
            GainDb::from_hundredths(350),
            vec![
                key(fr(10), 350, Interp::Linear),
                key(fr(20), i32::MIN, Interp::Hold),
                key(fr(40), -600, Interp::Linear),
                key(fr(200), 2_000, Interp::Linear),
            ],
        )
        .unwrap(),
        muted: false,
        fade_in: fr(5),
        fade_out: Tick::ZERO,
        fade_in_curve: FadeCurve::Smooth,
        fade_out_curve: FadeCurve::Linear,
    });
    let mut music = media_clip(48, fr(30), 11, MediaKind::Audio, Tick::ZERO, fr(120));
    music.audio = EmbeddedAudio::Active(ClipAudio {
        gain: Animatable::new(GainDb::from_hundredths(-1250)),
        muted: true,
        fade_in: fr(30),
        // Longer than the clip: stored as set (AU-006).
        fade_out: fr(200),
        fade_in_curve: FadeCurve::Fast,
        fade_out_curve: FadeCurve::Slow,
    });
    let mut extracted_clip = media_clip(50, fr(160), 13, MediaKind::Audio, fr(20), fr(40));
    extracted_clip.audio = EmbeddedAudio::Active(ClipAudio::with_gain(GainDb::MIN));
    let mut orphan = media_clip(49, fr(15), 10, MediaKind::Video, fr(5), fr(20));
    orphan.audio = EmbeddedAudio::Detached(LinkId::from_raw(nz(92)));
    set_stream(&mut orphan, Some(StreamSel::Index(4)));

    // Format 7: markers.
    let marks = kitchen_markers();
    intro.markers = marks.intro;
    title.markers = marks.title;
    orphan.markers = marks.orphan;
    // Format 8: items over the intro follow it, anchored to the intro's
    // source frame under their start (its source starts at frame 10).
    let anchor = |at: Tick| {
        Some(Attachment {
            owner: intro.id,
            owner_source_time: fr(10).checked_add(at).unwrap(),
        })
    };
    title.attach = anchor(title.start());
    sticker.attach = anchor(sticker.start());
    music.attach = anchor(music.start());
    voice.attach = anchor(voice.start());
    adjust.attach = anchor(adjust.start());

    let mut root = Sequence::from_parts(
        SeqId::from_raw(nz(1)),
        SequenceFlags {
            main_magnet: true,
            follow_main: true,
            follow_kinds: FollowKinds::ALL.with(FollowKind::Effect, false),
        },
        vec![
            mixed_track(
                20,
                TrackKind::Video,
                header(Some("Main"), false, false, false),
                TrackAudio {
                    gain: GainDb::SILENT,
                    pan: Pan::RIGHT,
                },
                vec![intro, still],
            ),
            track(
                21,
                TrackKind::Video,
                header(None, false, true, false),
                vec![overlay],
            ),
            track(
                22,
                TrackKind::Text,
                header(None, false, false, true),
                vec![title, caption],
            ),
            track(
                23,
                TrackKind::Sticker,
                TrackHeader::default(),
                vec![sticker],
            ),
            track(24, TrackKind::Effect, TrackHeader::default(), vec![effect]),
            track(
                25,
                TrackKind::Adjustment,
                TrackHeader::default(),
                vec![adjust],
            ),
        ],
        vec![
            mixed_track(
                26,
                TrackKind::Audio,
                header(None, true, false, false),
                TrackAudio {
                    gain: GainDb::from_hundredths(-300),
                    pan: Pan::new(-40),
                },
                vec![voice],
            ),
            mixed_track(
                27,
                TrackKind::Audio,
                header(Some("Música 🎵"), false, false, false),
                TrackAudio {
                    gain: TrackAudio::MAX_GAIN,
                    pan: Pan::LEFT,
                },
                vec![music, extracted_clip],
            ),
        ],
    );
    root.markers = marks.root;
    let mut nested = Sequence::from_parts(
        SeqId::from_raw(nz(2)),
        SequenceFlags {
            main_magnet: false,
            follow_main: false,
            follow_kinds: [FollowKind::Effect, FollowKind::SoundEffect]
                .into_iter()
                .collect(),
        },
        vec![track(
            30,
            TrackKind::Video,
            TrackHeader::default(),
            vec![orphan],
        )],
        Vec::new(),
    );
    nested.markers = marks.nested;
    let doc = Document::from_parts(
        SeqId::from_raw(nz(1)),
        settings,
        library,
        vec![Arc::new(root), Arc::new(nested)],
    )
    .unwrap();
    (doc, nz(100))
}

/// The kitchen sink's markers (format 7), by owner.
pub struct KitchenMarkers {
    pub root: Markers,
    pub nested: Markers,
    pub intro: Markers,
    pub title: Markers,
    pub orphan: Markers,
}

fn marker(id: u64, start: Tick, duration: Tick, color: PaletteColor) -> Marker {
    Marker {
        id: MarkerId::from_raw(nz(id)),
        at: MarkRange::new(start, duration).unwrap(),
        name: String::new(),
        note: String::new(),
        color,
        origin: MarkerOrigin::User,
    }
}

/// Every colour and origin: timeline markers (60, 61; 66, 67 in the
/// nested sequence) and clip markers in content time on the intro (62 a
/// detected beat, 63 a beat range, 64 before the intro's source start, so
/// trimmed out of view), the title (65, in its content time from origin 5)
/// and the orphan (68).
pub fn kitchen_markers() -> KitchenMarkers {
    let mut first = marker(60, Tick::ZERO, Tick::ZERO, PaletteColor::Blue);
    first.name = "Marker 1".into();
    let mut chorus = marker(61, fr(30), fr(15), PaletteColor::Teal);
    chorus.name = "Chorus \"B\" \\ ✂".into();
    chorus.note = "line one\nline two\ttabbed 🎵".into();
    chorus.origin = MarkerOrigin::Scene;
    let mut beat = marker(62, fr(30), Tick::ZERO, PaletteColor::Green);
    beat.origin = MarkerOrigin::Beat { auto: true };
    let mut tapped = marker(63, fr(40), fr(5), PaletteColor::Yellow);
    tapped.origin = MarkerOrigin::Beat { auto: false };
    tapped.name = "Marker 2".into();
    let hidden = marker(64, fr(5), Tick::ZERO, PaletteColor::Orange);
    let mut cue = marker(65, fr(10), fr(2), PaletteColor::Red);
    cue.note = "fade the title here".into();
    let nested_a = marker(66, fr(1), fr(3), PaletteColor::Pink);
    let mut nested_b = marker(67, fr(1), Tick::ZERO, PaletteColor::Purple);
    nested_b.name = "Same start, later id".into();
    let mut on_orphan = marker(68, fr(6), Tick::ZERO, PaletteColor::Blue);
    on_orphan.origin = MarkerOrigin::Scene;
    KitchenMarkers {
        root: Markers::new(vec![chorus, first]),
        nested: Markers::new(vec![nested_b, nested_a]),
        intro: Markers::new(vec![tapped, beat, hidden]),
        title: Markers::new(vec![cue]),
        orphan: Markers::new(vec![on_orphan]),
    }
}

/// The kitchen sink title's keyframes (format 6): every kind of value, and
/// every TR-031 interpolation on `text.size`. Its origin is 5 frames, as on
/// the right piece of a split.
pub fn kitchen_title_keys() -> ClipKeys {
    let key = |at: i64, value: PropValue, interp: Interp| Keyframe {
        at: fr(at),
        value,
        interp,
    };
    let color = |r, g, b, a| PropValue::Color(Rgba8::new(r, g, b, a));
    let ease = |curve, dir| Interp::Ease { curve, dir };
    let text = PropPath::Text;
    let sizes = Interp::PRESETS
        .iter()
        .zip(0i32..)
        .map(|(interp, i)| {
            key(
                5 + 2 * i64::from(i),
                PropValue::Length(RefPx::from_px(60 + 5 * i)),
                *interp,
            )
        })
        .collect();
    ClipKeys::from_parts(
        fr(5),
        vec![
            (
                PropPath::Transform(TransformProp::Position),
                vec![
                    key(
                        5,
                        PropValue::Position(CanvasPos::new(0, -350_000)),
                        ease(EaseCurve::Cubic, EaseDir::InOut),
                    ),
                    key(
                        20,
                        PropValue::Position(CanvasPos::new(120_000, -300_000)),
                        Interp::BounceOut,
                    ),
                    key(
                        40,
                        PropValue::Position(CanvasPos::new(-80_000, -350_000)),
                        Interp::Linear,
                    ),
                ],
            ),
            (
                PropPath::Transform(TransformProp::Scale),
                vec![
                    key(
                        5,
                        PropValue::Scale(Scale::from_basis_points(8_000)),
                        Interp::OvershootOut,
                    ),
                    key(
                        15,
                        PropValue::Scale(Scale::from_basis_points(10_000)),
                        Interp::Linear,
                    ),
                ],
            ),
            (
                PropPath::Transform(TransformProp::Rotation),
                vec![key(
                    5,
                    PropValue::Angle(Angle::from_hundredths(-1_500)),
                    Interp::Hold,
                )],
            ),
            (
                PropPath::Transform(TransformProp::Opacity),
                vec![
                    key(
                        5,
                        PropValue::Ratio(PerMille::ZERO),
                        ease(EaseCurve::Sine, EaseDir::Out),
                    ),
                    key(10, PropValue::Ratio(PerMille::ONE), Interp::Linear),
                ],
            ),
            (text(TextPropPath::Size), sizes),
            (
                text(TextPropPath::LetterSpacing),
                vec![key(
                    5,
                    PropValue::SignedRatio(SignedPerMille::new(-50)),
                    Interp::Linear,
                )],
            ),
            (
                text(TextPropPath::OutlineColor),
                vec![
                    key(5, color(0, 0, 0, 255), Interp::Linear),
                    key(30, color(255, 0, 128, 200), Interp::Linear),
                ],
            ),
            (
                text(TextPropPath::ShadowAngle),
                vec![key(
                    5,
                    PropValue::Direction(DeciDegrees::new(1_350)),
                    Interp::Linear,
                )],
            ),
            (
                text(TextPropPath::BubbleTailTip),
                vec![key(
                    10,
                    PropValue::TailTip(TailTip {
                        x: RefPx::from_px(-40),
                        y: RefPx::from_px(90),
                    }),
                    Interp::ElasticOut,
                )],
            ),
            (
                PropPath::FillStopColor(1),
                vec![
                    key(5, color(16, 32, 64, 255), Interp::Linear),
                    key(25, color(250, 250, 10, 255), Interp::OvershootIn),
                ],
            ),
            (
                PropPath::Reveal,
                vec![
                    key(5, PropValue::Ratio(PerMille::ZERO), Interp::Linear),
                    key(25, PropValue::Ratio(PerMille::ONE), Interp::Linear),
                ],
            ),
        ],
    )
    .unwrap()
}

fn spec(preset: &str, frames: i64) -> AnimationSpec {
    AnimationSpec::new(PresetId::new(preset), fr(frames))
}

/// The kitchen sink title's In (a typewriter with Pace), Out and Loop.
pub fn kitchen_title_animations() -> ClipAnimations {
    let mut entrance = spec("fc.in.typewriter", 12);
    entrance.easing = Some(Interp::Ease {
        curve: EaseCurve::Expo,
        dir: EaseDir::In,
    });
    entrance.params = AnimParams {
        unit: Some(AnimUnit::Letter),
        overlap: PerMille::ZERO,
        order: AnimOrder::Random,
        seed: 0xDEAD_BEEF,
        pace: Some(Tick::from_raw(TICKS_PER_SECOND / 20)),
        intensity: PerMille::new(750),
        travel: Travel::Short,
        caret: false,
    };
    let mut exit = spec("fc.out.slide", 9);
    exit.direction = Some(Direction::Right);
    exit.params.unit = Some(AnimUnit::Word);
    exit.params.order = AnimOrder::EdgesIn;
    let mut wave = spec("fc.loop.wave", 36);
    wave.phase = fr(7);
    wave.direction = Some(Direction::CounterClockwise);
    wave.params.unit = Some(AnimUnit::Line);
    wave.params.order = AnimOrder::CenterOut;
    wave.params.overlap = PerMille::new(250);
    wave.params.intensity = PerMille::ONE;
    ClipAnimations::default()
        .with(Slot::Entrance, Some(entrance), SlotPolicy::LoopBetween)
        .with(Slot::Exit, Some(exit), SlotPolicy::LoopBetween)
        .with(Slot::Continuous, Some(wave), SlotPolicy::LoopBetween)
}

/// The overlay video's In, from below.
pub fn kitchen_overlay_animations() -> ClipAnimations {
    let mut entrance = spec("fc.in.slide", 15);
    entrance.direction = Some(Direction::Up);
    entrance.params.unit = Some(AnimUnit::Block);
    ClipAnimations::default().with(Slot::Entrance, Some(entrance), SlotPolicy::Exclusive)
}

/// The sticker's Loop: a preset this build doesn't have, kept as chosen.
pub fn kitchen_sticker_animations() -> ClipAnimations {
    let mut spin = spec("fc.loop.spin", 60);
    spin.direction = Some(Direction::Clockwise);
    spin.params.order = AnimOrder::Backward;
    spin.params.seed = 7;
    ClipAnimations::default().with(Slot::Continuous, Some(spin), SlotPolicy::Exclusive)
}

/// The kitchen sink title's style: every format-5 value changed from the
/// factory default. Its first gradient stop is the solid fill format 4
/// stored.
pub fn kitchen_title_style() -> TextStyle {
    let stop = |at: u16, color: Rgba8| GradientStop {
        at: PerMille::new(at),
        color,
    };
    TextStyle {
        font: FontRef::new("DejaVu Serif"),
        size: RefPx::from_hundredths(9_650),
        fill: Fill::LinearGradient(LinearGradient {
            angle: DeciDegrees::new(1_355),
            stops: GradientStops::new(vec![
                stop(0, Rgba8::new(255, 128, 0, 192)),
                stop(400, Rgba8::new(255, 255, 255, 255)),
                stop(400, Rgba8::new(0, 64, 255, 255)),
                stop(1000, Rgba8::new(10, 20, 30, 0)),
            ])
            .unwrap(),
            extent: FillExtent::Line,
        }),
        bold: false,
        italic: true,
        decoration: Decoration {
            underline: true,
            strike: true,
        },
        case: CaseTransform::Title,
        letter_spacing: SignedPerMille::new(-150),
        stroke: Layer::on(Stroke {
            color: Rgba8::new(0, 0, 0, 255),
            width: PerMille::new(120),
            opacity: PerMille::new(800),
            position: StrokePosition::Center,
        }),
        shadow: Layer::on(Shadow {
            color: Rgba8::new(20, 0, 40, 255),
            opacity: PerMille::new(650),
            blur: PerMille::new(200),
            distance: PerMille::new(150),
            angle: DeciDegrees::new(1_355),
            include_box: true,
        }),
        glow: Layer::on(Glow {
            color: Rgba8::new(0, 255, 200, 255),
            intensity: PerMille::new(900),
            radius: PerMille::new(400),
            offset: PerMille::new(50),
            angle: DeciDegrees::new(2_700),
            direction: GlowDirection::Both,
        }),
        background: Layer::off(TextBox {
            color: Rgba8::new(255, 230, 0, 255),
            opacity: PerMille::new(550),
            shape: BoxShape::PerChar,
            pad_x: PerMille::new(100),
            pad_y: PerMille::new(50),
            roundness: PerMille::new(1000),
            height: BoxHeight::LowerHalf,
            border: Layer::on(Border {
                color: Rgba8::new(255, 0, 255, 255),
                width: PerMille::new(20),
            }),
        }),
        bubble: Layer::on(TextBubble {
            spec: BubbleSpec {
                kind: BubbleKind::ArrowTag { point: Side::Left },
                tail: Layer::off(Tail {
                    tip: TailTip {
                        x: RefPx::from_hundredths(-12_345),
                        y: RefPx::from_px(250),
                    },
                    base: PerMille::new(300),
                    bend: SignedPerMille::new(-450),
                }),
                spikes: 20,
                spike_depth: PerMille::new(250),
                jitter: PerMille::new(700),
                seed: 4_000_000_000,
                lobe: PerMille::new(300),
                trail: 4,
            },
            fill: Rgba8::new(30, 30, 36, 230),
            opacity: PerMille::new(900),
            pad_x: PerMille::new(500),
            pad_y: PerMille::new(350),
            roundness: PerMille::new(600),
            border: Layer::off(Border {
                color: Rgba8::new(255, 0, 0, 255),
                width: PerMille::new(80),
            }),
        }),
        curve: Layer::on(SignedPerMille::new(-350)),
    }
}

/// `content` as format 4 could hold it: a text keeps its font, size, the
/// first colour of its fill, bold and italic, its alignment, width and
/// line spacing; everything format 5 added is at the factory default.
pub fn v4_text(content: &ClipContent) -> ClipContent {
    let ClipContent::Text(data) = content else {
        return content.clone();
    };
    let style = &data.style;
    ClipContent::Text(Arc::new(TextClipData {
        content: data.content.clone(),
        style: TextStyle {
            font: style.font.clone(),
            size: style.size,
            fill: style.fill.with_kind(FillKind::Solid),
            bold: style.bold,
            italic: style.italic,
            ..TextStyle::default()
        },
        layout: TextLayout {
            justify: false,
            v_anchor: VAnchor::Middle,
            ..data.layout
        },
        origin: data.origin,
    }))
}

/// Sets the stream choice of a media clip.
fn set_stream(clip: &mut Clip, sel: Option<StreamSel>) {
    if let ClipContent::Media { stream, .. } = &mut clip.content {
        *stream = sel;
    }
}

/// The kitchen sink's UI state: the main video track and the music track
/// soloed (AU-012).
pub fn kitchen_ui() -> UiState {
    UiState {
        solo: [20, 27]
            .into_iter()
            .map(|id| TrackId::from_raw(nz(id)))
            .collect(),
    }
}

/// `clip`'s content and sound as format 3 could hold them: no stream
/// choice, no volume keys (the static volume stays), linear fades, and the
/// text format 4 could hold ([`v4_text`]).
pub fn v3_sound(clip: &Clip) -> (ClipContent, EmbeddedAudio) {
    let mut content = v4_text(&clip.content);
    if let ClipContent::Media { stream, .. } = &mut content {
        *stream = None;
    }
    let audio = match &clip.audio {
        EmbeddedAudio::Active(a) => EmbeddedAudio::Active(ClipAudio {
            gain: Animatable::new(*a.gain.static_value()),
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            ..a.clone()
        }),
        other => other.clone(),
    };
    (content, audio)
}

/// Export settings with every value changed from the defaults.
pub fn kitchen_export() -> ExportSettings {
    ExportSettings {
        file: FileTarget {
            name: "Trip \"final\" / v2 ✂".into(),
            folder: Some(r"C:\Users\me\Videos\Exports".into()),
        },
        video: VideoSettings {
            size: OutputSize::Preset(ShortSide::P720),
            frame_rate: FrameRateChoice::Fixed(FrameRate::new(25, 1).unwrap()),
            codec: VideoCodec::H264,
            container: Container::Mp4,
            quality: QualityTier::Higher,
        },
        audio: AudioExportSettings {
            include: false,
            kbps: 256,
            sample_rate: SampleRate::new(44_100).unwrap(),
        },
    }
}

// ---------------------------------------------------------------------------
// Random documents through the editor
// ---------------------------------------------------------------------------

/// One random edit; indices pick among existing items modulo their count.
#[derive(Debug, Clone)]
pub enum Op {
    Insert {
        asset: u8,
        src: u8,
        len: u8,
        at: u8,
        track: u8,
    },
    InsertImage {
        asset: u8,
        at: u8,
        len: u8,
    },
    InsertGenerated {
        kind: u8,
        at: u8,
        len: u8,
    },
    AddTrack {
        kind: u8,
    },
    Header {
        track: u8,
        name: Option<String>,
        flags: u8,
    },
    Magnet {
        on: bool,
    },
    Move {
        pick: u16,
        delta: i8,
        shift: i8,
    },
    Split {
        pick: u16,
        at: u8,
    },
    Trim {
        pick: u16,
        out: bool,
        delta: i8,
        ripple: bool,
    },
    Delete {
        pick: u16,
        ripple: bool,
    },
    // Edits added with format 2 (clip sound, extracted audio).
    Detach {
        pick: u16,
    },
    Restore {
        pick: u16,
    },
    SetAudio {
        pick: u16,
        edit: u8,
        value: i16,
    },
    Extract {
        asset: u8,
        stream: u8,
    },
    RemoveMedia {
        asset: u8,
    },
    // Edits added with format 3 (text clips, transforms).
    InsertText {
        at: u8,
        len: u8,
        text: String,
        target: u8,
    },
    SetText {
        pick: u16,
        text: String,
    },
    TextStyle {
        pick: u16,
        edit: u8,
        value: i32,
        family: String,
    },
    TextLayout {
        pick: u16,
        edit: u8,
        value: i32,
    },
    Transform {
        pick: u16,
        edit: u8,
        x: i32,
        y: i32,
    },
    // Edits added with format 4 (keyframes, curves, track stage, streams).
    FadeCurve {
        pick: u16,
        out: bool,
        curve: u8,
    },
    GainKeys {
        pick: u16,
        edit: u8,
        at: u8,
        to: u8,
        value: i16,
    },
    TrackMix {
        track: u8,
        pan: bool,
        value: i16,
    },
    Stream {
        pick: u16,
        sel: u8,
    },
    AddStream {
        pick: u16,
        stream: u8,
    },
    // Edits added with format 5 (text round 2): `edit` picks one of the
    // style or layout edits, built from `a` and `b`.
    TextLook {
        pick: u16,
        edit: u8,
        a: i32,
        b: i32,
    },
    // Edits added with format 6: property keys (`edit` picks the key
    // edit, `prop` the property, `at`/`to` playhead positions in the clip,
    // `a`/`b` the value), animation slots (`len` 0 clears the slot) and
    // eased volume keys.
    Keys {
        pick: u16,
        prop: u8,
        edit: u8,
        at: u8,
        to: u8,
        a: i32,
        b: i32,
    },
    Animate {
        pick: u16,
        slot: u8,
        preset: u8,
        len: u8,
        a: u32,
        b: u32,
    },
    GainEase {
        pick: u16,
        key: u8,
        interp: u8,
    },
    // Edits added with format 7: markers. `edit` picks add or edit (two
    // in seven each), move, delete one or delete by a selector; `pick` a
    // clip or marker, `at` a frame, `a` the owner rule, colour and selector.
    Marker {
        edit: u8,
        pick: u16,
        at: u8,
        a: u16,
        text: String,
    },
    // Edits added with format 8: *Follow main track* on or off, or (with
    // `kinds`) the set of kinds that follow.
    Follow {
        on: bool,
        kinds: Option<u16>,
    },
    // Edits added with format 9: a recorded take of `len` frames after a
    // pre-roll of `preroll`, placed at `at`, maybe replacing a voiceover
    // clip (`replace` picks it).
    Record {
        at: u8,
        len: u8,
        preroll: u8,
        new_track: bool,
        replace: Option<u16>,
    },
    // Edits added with format 10: a colour edit (`edit` picks which, `a`
    // and `b` the values, often out of range) or a colour key.
    Color {
        pick: u16,
        edit: u8,
        a: i16,
        b: i16,
    },
    // Edits added with format 11: HSL and wheel sliders, pucks, curves,
    // band and wheel resets, and their keys.
    ColorK2 {
        pick: u16,
        edit: u8,
        a: i16,
        b: i16,
        c: i16,
    },
    // Edits added with format 12: the LUT slot, its sampling, skin
    // protection and their keys.
    ColorK3 {
        pick: u16,
        edit: u8,
        a: i16,
        b: i16,
    },
    // Edits added with format 13: auto corrections, matches, Apply to
    // sliders and the Strength and Intensity keys.
    ColorK5 {
        pick: u16,
        edit: u8,
        a: i16,
        b: i16,
    },
    // Edits added with format 14: colour masks and their keys.
    ColorK6 {
        pick: u16,
        edit: u8,
        a: i16,
        b: i16,
    },
}

/// Probe-like data for one random media item.
#[derive(Debug, Clone)]
pub struct MediaSpec {
    pub kind: u8,
    pub frames: u16,
    pub absolute: String,
    pub relative: Option<String>,
    pub fingerprint: Option<(u64, u64)>,
    pub display_name: Option<String>,
    pub rotation: u8,
    pub start: i32,
    /// Audio streams carry probe tags (format 4) and the second one of a
    /// video is the default.
    pub tagged: bool,
}

/// Arbitrary but printable-ish text, including quotes, backslashes,
/// control characters and non-ASCII.
pub fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 _./-]{0,12}",
        "[\\PC]{0,8}",
        "[\"\\\\\t\n\u{0}\u{7f}é✂🎬]{0,6}",
    ]
}

fn media_spec() -> impl Strategy<Value = MediaSpec> {
    (
        0u8..4,
        1u16..400,
        text(),
        proptest::option::of(text()),
        proptest::option::of((any::<u64>(), any::<u64>())),
        proptest::option::of(text()),
        0u8..4,
        -5_000i32..5_000,
        any::<bool>(),
    )
        .prop_map(
            |(
                kind,
                frames,
                absolute,
                relative,
                fingerprint,
                display_name,
                rotation,
                start,
                tagged,
            )| {
                MediaSpec {
                    kind,
                    frames,
                    absolute,
                    relative,
                    fingerprint,
                    display_name,
                    rotation,
                    start,
                    tagged,
                }
            },
        )
}

/// Media as format 3 and older recorded it: no probe tags.
fn media_spec_untagged() -> impl Strategy<Value = MediaSpec> {
    media_spec().prop_map(|spec| MediaSpec {
        tagged: false,
        ..spec
    })
}

/// Edits that existed in format 1: they leave every clip with the sound
/// a format-1 file migrates to.
fn op_v1() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (any::<u8>(), any::<u8>(), 1u8..80, any::<u8>(), any::<u8>())
            .prop_map(|(asset, src, len, at, track)| Op::Insert { asset, src, len, at, track }),
        1 => (any::<u8>(), any::<u8>(), 1u8..60)
            .prop_map(|(asset, at, len)| Op::InsertImage { asset, at, len }),
        2 => (0u8..4, any::<u8>(), 1u8..60)
            .prop_map(|(kind, at, len)| Op::InsertGenerated { kind, at, len }),
        1 => (0u8..6).prop_map(|kind| Op::AddTrack { kind }),
        1 => (any::<u8>(), proptest::option::of(text()), any::<u8>())
            .prop_map(|(track, name, flags)| Op::Header { track, name, flags }),
        1 => any::<bool>().prop_map(|on| Op::Magnet { on }),
        2 => (any::<u16>(), any::<i8>(), -1i8..2)
            .prop_map(|(pick, delta, shift)| Op::Move { pick, delta, shift }),
        2 => (any::<u16>(), any::<u8>()).prop_map(|(pick, at)| Op::Split { pick, at }),
        2 => (any::<u16>(), any::<bool>(), any::<i8>(), any::<bool>())
            .prop_map(|(pick, out, delta, ripple)| Op::Trim { pick, out, delta, ripple }),
        1 => (any::<u16>(), any::<bool>()).prop_map(|(pick, ripple)| Op::Delete { pick, ripple }),
    ]
}

/// Edits that existed in format 2: they leave every clip with the text and
/// transform a format-2 file migrates to.
fn op_v2() -> impl Strategy<Value = Op> {
    prop_oneof![
        10 => op_v1(),
        5 => any::<u16>().prop_map(|pick| Op::Detach { pick }),
        1 => any::<u16>().prop_map(|pick| Op::Restore { pick }),
        3 => (any::<u16>(), 0u8..4, any::<i16>())
            .prop_map(|(pick, edit, value)| Op::SetAudio { pick, edit, value }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(asset, stream)| Op::Extract { asset, stream }),
        1 => any::<u8>().prop_map(|asset| Op::RemoveMedia { asset }),
    ]
}

/// Edits that existed in format 3: they leave every clip with the sound a
/// format-3 file migrates to.
fn op_v3() -> impl Strategy<Value = Op> {
    prop_oneof![
        12 => op_v2(),
        2 => (any::<u8>(), 1u8..60, text(), any::<u8>())
            .prop_map(|(at, len, text, target)| Op::InsertText { at, len, text, target }),
        1 => (any::<u16>(), text()).prop_map(|(pick, text)| Op::SetText { pick, text }),
        2 => (any::<u16>(), 0u8..5, any::<i32>(), text())
            .prop_map(|(pick, edit, value, family)| Op::TextStyle { pick, edit, value, family }),
        2 => (any::<u16>(), 0u8..3, any::<i32>())
            .prop_map(|(pick, edit, value)| Op::TextLayout { pick, edit, value }),
        4 => (any::<u16>(), 0u8..7, any::<i32>(), any::<i32>())
            .prop_map(|(pick, edit, x, y)| Op::Transform { pick, edit, x, y }),
    ]
}

/// Edits that existed in format 4: they leave every text with the style a
/// format-4 file migrates to.
fn op_v4() -> impl Strategy<Value = Op> {
    prop_oneof![
        16 => op_v3(),
        2 => (any::<u16>(), any::<bool>(), 0u8..4)
            .prop_map(|(pick, out, curve)| Op::FadeCurve { pick, out, curve }),
        5 => (any::<u16>(), 0u8..6, any::<u8>(), any::<u8>(), any::<i16>())
            .prop_map(|(pick, edit, at, to, value)| Op::GainKeys { pick, edit, at, to, value }),
        2 => (any::<u8>(), any::<bool>(), any::<i16>())
            .prop_map(|(track, pan, value)| Op::TrackMix { track, pan, value }),
        2 => (any::<u16>(), 0u8..4).prop_map(|(pick, sel)| Op::Stream { pick, sel }),
        1 => (any::<u16>(), 0u8..3).prop_map(|(pick, stream)| Op::AddStream { pick, stream }),
    ]
}

/// Edits that existed in format 5: they leave every clip without keys or
/// animations, and volume keys linear or held.
fn op_v5() -> impl Strategy<Value = Op> {
    prop_oneof![
        24 => op_v4(),
        4 => (any::<u8>(), 1u8..60, text(), any::<u8>())
            .prop_map(|(at, len, text, target)| Op::InsertText { at, len, text, target }),
        16 => (any::<u16>(), any::<u8>(), any::<i32>(), any::<i32>())
            .prop_map(|(pick, edit, a, b)| Op::TextLook { pick, edit, a, b }),
    ]
}

/// Edits that existed in format 6: they add no markers, leave following on
/// for every kind and record no takes.
fn op_v6() -> impl Strategy<Value = Op> {
    prop_oneof![
        44 => op_v5(),
        14 => (any::<u16>(), any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>(), any::<i32>(), any::<i32>())
            .prop_map(|(pick, prop, edit, at, to, a, b)| Op::Keys { pick, prop, edit, at, to, a, b }),
        8 => (any::<u16>(), 0u8..3, any::<u8>(), any::<u8>(), any::<u32>(), any::<u32>())
            .prop_map(|(pick, slot, preset, len, a, b)| Op::Animate { pick, slot, preset, len, a, b }),
        6 => (any::<u16>(), any::<u8>(), any::<u8>())
            .prop_map(|(pick, key, interp)| Op::GainEase { pick, key, interp }),
    ]
}

/// Edits that existed in format 7: they leave following on for every
/// kind and record no takes. Inserts on the main track and overlays above
/// it, and volume keys with their easing, are weighted up here: the nested
/// weights above make them rare, clip markers and attachments need clips
/// to sit on, and the new edits would otherwise crowd out eased volume
/// keys.
fn op_v7() -> impl Strategy<Value = Op> {
    prop_oneof![
        72 => op_v6(),
        3 => (any::<u16>(), 0u8..6, any::<u8>(), any::<u8>(), any::<i16>())
            .prop_map(|(pick, edit, at, to, value)| Op::GainKeys { pick, edit, at, to, value }),
        3 => (any::<u16>(), any::<u8>(), any::<u8>())
            .prop_map(|(pick, key, interp)| Op::GainEase { pick, key, interp }),
        10 => (0u8..7, any::<u16>(), any::<u8>(), any::<u16>(), text())
            .prop_map(|(edit, pick, at, a, text)| Op::Marker { edit, pick, at, a, text }),
        4 => (any::<u8>(), any::<u8>(), 1u8..80, 0u8..40)
            .prop_map(|(asset, src, len, at)| Op::Insert { asset, src, len, at, track: 0 }),
        3 => (0u8..4, 0u8..80, 1u8..60)
            .prop_map(|(kind, at, len)| Op::InsertGenerated { kind, at, len }),
    ]
}

/// Edits that existed in format 8: they record no takes.
fn op_v8() -> impl Strategy<Value = Op> {
    prop_oneof![
        86 => op_v7(),
        3 => (any::<bool>(), proptest::option::of(any::<u16>()))
            .prop_map(|(on, kinds)| Op::Follow { on, kinds }),
    ]
}

/// Edits that existed in format 9: no colour.
fn op_v9() -> impl Strategy<Value = Op> {
    prop_oneof![
        92 => op_v8(),
        3 => (any::<u8>(), 1u8..60, 0u8..30, any::<bool>(), proptest::option::of(any::<u16>()))
            .prop_map(|(at, len, preroll, new_track, replace)| Op::Record {
                at, len, preroll, new_track, replace,
            }),
    ]
}

/// Edits that existed in format 10: colour stacks without HSL, Curves and
/// Wheels.
fn op_v10() -> impl Strategy<Value = Op> {
    prop_oneof![
        95 => op_v9(),
        4 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::Color { pick, edit, a, b }),
    ]
}

/// Edits that existed in format 11: colour stacks without the LUT slot and
/// skin protection. Built on [`op_v9`] rather than [`op_v10`] so the
/// strategy is no deeper than before (a deeper one overflows the test
/// thread's stack in debug builds).
fn op_v11() -> impl Strategy<Value = Op> {
    prop_oneof![
        95 => op_v9(),
        2 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::Color { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b, c)| Op::ColorK2 { pick, edit, a, b, c }),
    ]
}

/// Edits that existed in format 12: colour stacks without Auto and Match;
/// as shallow as [`op_v11`].
fn op_v12() -> impl Strategy<Value = Op> {
    prop_oneof![
        95 => op_v9(),
        2 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::Color { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b, c)| Op::ColorK2 { pick, edit, a, b, c }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::ColorK3 { pick, edit, a, b }),
    ]
}

/// Edits that existed in format 13: colour stacks without masks; as
/// shallow as [`op_v11`].
fn op_v13() -> impl Strategy<Value = Op> {
    prop_oneof![
        95 => op_v9(),
        2 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::Color { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b, c)| Op::ColorK2 { pick, edit, a, b, c }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::ColorK3 { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::ColorK5 { pick, edit, a, b }),
    ]
}

/// Every edit, including format 14's colour masks; as shallow as
/// [`op_v11`].
fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        95 => op_v9(),
        2 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::Color { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b, c)| Op::ColorK2 { pick, edit, a, b, c }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::ColorK3 { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::ColorK5 { pick, edit, a, b }),
        3 => (any::<u16>(), any::<u8>(), any::<i16>(), any::<i16>())
            .prop_map(|(pick, edit, a, b)| Op::ColorK6 { pick, edit, a, b }),
    ]
}

/// Project settings among the supported presets.
fn settings() -> impl Strategy<Value = ProjectSettings> {
    (
        0usize..FrameRate::PRESETS.len(),
        prop_oneof![Just(44_100u32), Just(48_000), Just(96_000)],
        1u32..=2048,
        1u32..=2048,
        any::<bool>(),
    )
        .prop_map(|(rate, hz, w, h, match_first_clip)| {
            let (num, den) = FrameRate::PRESETS[rate];
            ProjectSettings {
                width: w * 2,
                height: h * 2,
                frame_rate: FrameRate::new(num, den).unwrap(),
                sample_rate: SampleRate::new(hz).unwrap(),
                match_first_clip,
            }
        })
}

/// The plan for a random document: settings, media and edits.
#[derive(Debug, Clone)]
pub struct DocPlan {
    pub settings: ProjectSettings,
    pub media: Vec<MediaSpec>,
    pub ops: Vec<Op>,
}

pub fn doc_plan() -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec(), 1..5),
        proptest::collection::vec(op(), 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

/// A plan with `ops` and today's media.
fn plan_of(ops: impl Strategy<Value = Op>) -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec(), 1..5),
        proptest::collection::vec(ops, 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

/// Like [`doc_plan`] with format-13 edits only.
pub fn doc_plan_v13() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v13())
}

/// Like [`doc_plan`] with format-12 edits only.
pub fn doc_plan_v12() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v12())
}

/// Like [`doc_plan`] with format-11 edits only.
pub fn doc_plan_v11() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v11())
}

/// Like [`doc_plan`] with format-10 edits only.
pub fn doc_plan_v10() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v10())
}

/// Like [`doc_plan`] with format-9 edits only.
pub fn doc_plan_v9() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v9())
}

/// Like [`doc_plan`] with format-8 edits only.
pub fn doc_plan_v8() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v8())
}

/// Like [`doc_plan`] with format-7 edits only.
pub fn doc_plan_v7() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v7())
}

/// Like [`doc_plan`] with format-6 edits only.
pub fn doc_plan_v6() -> impl Strategy<Value = DocPlan> {
    plan_of(op_v6())
}

/// Like [`doc_plan`] with format-5 edits only.
pub fn doc_plan_v5() -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec(), 1..5),
        proptest::collection::vec(op_v5(), 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

/// Like [`doc_plan`] with format-4 edits only.
pub fn doc_plan_v4() -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec(), 1..5),
        proptest::collection::vec(op_v4(), 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

/// Like [`doc_plan`] with format-3 media and edits only.
pub fn doc_plan_v3() -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec_untagged(), 1..5),
        proptest::collection::vec(op_v3(), 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

/// Like [`doc_plan`] with format-2 media and edits only.
pub fn doc_plan_v2() -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec_untagged(), 1..5),
        proptest::collection::vec(op_v2(), 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

/// Like [`doc_plan`] with format-1 media and edits only.
pub fn doc_plan_v1() -> impl Strategy<Value = DocPlan> {
    (
        settings(),
        proptest::collection::vec(media_spec_untagged(), 1..5),
        proptest::collection::vec(op_v1(), 0..40),
    )
        .prop_map(|(settings, media, ops)| DocPlan {
            settings,
            media,
            ops,
        })
}

fn new_media(spec: &MediaSpec, rate: FrameRate) -> NewMedia {
    let duration = rate.frames(i64::from(spec.frames)).unwrap();
    let rotation = [
        Rotation::None,
        Rotation::Cw90,
        Rotation::Cw180,
        Rotation::Cw270,
    ][usize::from(spec.rotation % 4)];
    let start = Tick::from_raw(i64::from(spec.start));
    let audio = AudioStreamInfo {
        index: 1,
        codec: "aac".into(),
        sample_rate: 48_000,
        channels: 2,
        start,
        duration,
        duration_exact: spec.frames.is_multiple_of(2),
        default: false,
        language: spec.tagged.then(|| "und".into()),
        title: None,
        layout: spec.tagged.then(|| "stereo".into()),
    };
    let commentary = AudioStreamInfo {
        index: 2,
        codec: "opus".into(),
        channels: 1,
        default: spec.tagged,
        language: spec.tagged.then(|| "eng".into()),
        title: spec.tagged.then(|| "Commentary \"dry\"".into()),
        layout: spec.tagged.then(|| "mono".into()),
        ..audio.clone()
    };
    let video = VideoStreamInfo {
        index: 0,
        codec: "h264".into(),
        width: u32::from(spec.frames) * 3,
        height: 720,
        sar: (u32::from(spec.rotation), 3),
        rotation,
        mirrored: spec.frames.is_multiple_of(3),
        rate: (spec.frames % 2 == 1).then(|| Rational::new(i64::from(spec.frames), 1001).unwrap()),
        variable_rate: spec.frames.is_multiple_of(5),
        has_alpha: spec.frames.is_multiple_of(7),
        bit_depth: 8 + (spec.rotation % 3) * 2,
        start,
        duration,
    };
    let info = match spec.kind {
        // Video with two audio streams.
        0 => MediaInfo {
            container: "mp4".into(),
            default_video: Some(0),
            duration,
            video: vec![video],
            audio: vec![audio, commentary],
            still: None,
        },
        // Video without sound.
        3 => MediaInfo {
            container: "mp4".into(),
            default_video: Some(0),
            duration,
            video: vec![video],
            audio: Vec::new(),
            still: None,
        },
        // Audio only.
        1 => MediaInfo {
            container: "wav".into(),
            default_video: None,
            duration,
            video: Vec::new(),
            audio: vec![audio],
            still: None,
        },
        // A still image.
        _ => MediaInfo {
            container: "image2".into(),
            default_video: None,
            duration,
            video: Vec::new(),
            audio: Vec::new(),
            still: Some((u32::from(spec.frames), 99)),
        },
    };
    NewMedia {
        path: StoredPath {
            absolute: spec.absolute.clone(),
            relative: spec.relative.clone(),
        },
        info,
        fingerprint: spec.fingerprint.map(|(size, head_tail)| Fingerprint {
            size,
            head_tail,
            duration,
        }),
        display_name: spec.display_name.clone(),
    }
}

fn clip_ids(doc: &Document, seq: SeqId) -> Vec<ClipId> {
    doc.sequence(seq)
        .unwrap()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter().map(|c| c.id))
        .collect()
}

fn request(editor: &Editor, op: &Op) -> Option<EditRequest> {
    let doc = editor.document();
    let seq = doc.root();
    let rate = doc.settings().frame_rate;
    let frames = |n: i64| rate.frames(n).unwrap();
    let tracks: Vec<TrackId> = doc.sequence(seq)?.tracks().map(|(_, t)| t.id).collect();
    let items: Vec<Arc<MediaItem>> = doc.library().items().cloned().collect();
    let clips = clip_ids(doc, seq);
    let pick_clip = |pick: u16| clips.get(usize::from(pick) % clips.len().max(1)).copied();
    let pick_item = |pick: u8| items.get(usize::from(pick) % items.len().max(1));
    // Picks among the clips an edit applies to, so most edits do something.
    let pick_where = |pick: u16, wanted: &dyn Fn(&Clip) -> bool| {
        let fits: Vec<ClipId> = clips
            .iter()
            .copied()
            .filter(|id| doc.clip(*id).is_some_and(|c| wanted(c)))
            .collect();
        fits.get(usize::from(pick) % fits.len().max(1)).copied()
    };
    Some(match op {
        Op::Insert {
            asset,
            src,
            len,
            at,
            track,
        } => {
            let movies: Vec<_> = items.iter().filter(|m| !m.info.is_still()).collect();
            let item = movies.get(usize::from(*asset) % movies.len().max(1))?;
            let kind = if item.info.has_video() {
                MediaKind::Video
            } else {
                MediaKind::Audio
            };
            let max = item.info.duration.raw() / frames(1).raw();
            let src = i64::from(*src) % max;
            let len = i64::from(*len).min(max - src);
            let pick = usize::from(*track) % (tracks.len() + 2);
            let target = match tracks.get(pick) {
                Some(t) => InsertTarget::Track(*t),
                None if pick == tracks.len() => InsertTarget::NewTrack,
                None => InsertTarget::FirstFree,
            };
            EditRequest::Insert(InsertReq {
                seq,
                target,
                at: frames(i64::from(*at)),
                clips: vec![NewClip::media(
                    item.id,
                    kind,
                    TimeRange::new(frames(src), frames(len)).ok()?,
                )],
            })
        }
        Op::InsertImage { asset, at, len } => {
            let stills: Vec<_> = items.iter().filter(|m| m.info.is_still()).collect();
            let item = stills.get(usize::from(*asset) % stills.len().max(1))?;
            EditRequest::Insert(InsertReq {
                seq,
                target: InsertTarget::Track(*tracks.first()?),
                at: frames(i64::from(*at)),
                clips: vec![NewClip::image(item.id, frames(i64::from(*len)))],
            })
        }
        Op::InsertGenerated { kind, at, len } => {
            let len = frames(i64::from(*len));
            // Format 1 and 2 stored text as a generated kind; it opens as
            // a factory-default text clip.
            let clip = match kind % 4 {
                0 => NewClip::text(TextClipData::default(), len),
                1 => NewClip::generated(GeneratedKind::Sticker, len),
                2 => NewClip::generated(GeneratedKind::Effect, len),
                _ => NewClip::generated(GeneratedKind::Adjustment, len),
            };
            EditRequest::Insert(InsertReq {
                seq,
                target: InsertTarget::NewTrack,
                at: frames(i64::from(*at)),
                clips: vec![clip],
            })
        }
        Op::AddTrack { kind } => EditRequest::AddTrack(AddTrackReq {
            seq,
            kind: [
                TrackKind::Video,
                TrackKind::Text,
                TrackKind::Sticker,
                TrackKind::Effect,
                TrackKind::Adjustment,
                TrackKind::Audio,
            ][usize::from(*kind % 6)],
            position: TrackPosition::Default,
        }),
        Op::Header { track, name, flags } => EditRequest::SetTrackHeader {
            seq,
            track: *tracks.get(usize::from(*track) % tracks.len().max(1))?,
            header: TrackHeader {
                name: name.clone(),
                flags: TrackFlags {
                    muted: flags & 1 != 0,
                    hidden: flags & 2 != 0,
                    // Rarely lock, so later edits still have room to work.
                    locked: flags & 0b1110_0000 == 0b1110_0000,
                },
            },
        },
        Op::Magnet { on } => EditRequest::SetMagnet { seq, on: *on },
        Op::Move { pick, delta, shift } => EditRequest::Move(MoveReq {
            seq,
            clips: vec![pick_clip(*pick)?],
            delta: frames(i64::from(*delta)),
            track_shift: i32::from(*shift),
        }),
        Op::Split { pick, at } => {
            let id = pick_clip(*pick)?;
            let clip = doc.clip(id)?;
            let offset = i64::from(*at) % (clip.duration().raw() / frames(1).raw()).max(1);
            EditRequest::Split(SplitReq {
                seq,
                at: clip.start().checked_add(frames(offset)).ok()?,
                targets: SplitTargets::Clips(vec![id]),
            })
        }
        Op::Trim {
            pick,
            out,
            delta,
            ripple,
        } => EditRequest::Trim(TrimReq {
            seq,
            clip: pick_clip(*pick)?,
            edge: if *out { Edge::Out } else { Edge::In },
            delta: frames(i64::from(*delta)),
            ripple: *ripple,
        }),
        Op::Delete { pick, ripple } => EditRequest::Delete(DeleteReq {
            seq,
            clips: vec![pick_clip(*pick)?],
            ripple: *ripple,
        }),
        Op::Detach { pick } => EditRequest::DetachAudio {
            seq,
            clips: vec![pick_where(*pick, &|c| {
                c.audio.active().is_some()
                    && matches!(
                        c.content,
                        ClipContent::Media {
                            kind: MediaKind::Video,
                            ..
                        }
                    )
            })?],
        },
        Op::Restore { pick } => EditRequest::RestoreAudio {
            seq,
            clips: vec![pick_where(*pick, &|c| c.audio.detached().is_some())?],
        },
        Op::SetAudio { pick, edit, value } => {
            let fade = || frames(i64::from(value.unsigned_abs() % 300));
            let edit = match edit {
                0 if value % 4 == 0 => AudioEdit::Gain(GainDb::SILENT),
                0 => AudioEdit::Gain(GainDb::from_hundredths(i32::from(*value))),
                1 => AudioEdit::Muted(value % 2 != 0),
                2 => AudioEdit::FadeIn(fade()),
                _ => AudioEdit::FadeOut(fade()),
            };
            EditRequest::SetClipAudio {
                seq,
                clips: vec![pick_where(*pick, &|c| c.audio.active().is_some())?],
                edit,
            }
        }
        Op::Extract { asset, stream } => EditRequest::ExtractAudio {
            media: pick_item(*asset)?.id,
            stream: match stream % 3 {
                0 => None,
                n => Some(u32::from(n)),
            },
        },
        Op::RemoveMedia { asset } => EditRequest::RemoveMedia {
            media: vec![pick_item(*asset)?.id],
            with_clips: true,
        },
        Op::InsertText {
            at,
            len,
            text,
            target,
        } => {
            let pick = usize::from(*target) % (tracks.len() + 2);
            let target = match tracks.get(pick) {
                Some(t) => InsertTarget::Track(*t),
                None if pick == tracks.len() => InsertTarget::NewTrack,
                None => InsertTarget::FirstFree,
            };
            EditRequest::Insert(InsertReq {
                seq,
                target,
                at: frames(i64::from(*at)),
                clips: vec![NewClip::text(
                    TextClipData::with_content(text.as_str()),
                    frames(i64::from(*len)),
                )],
            })
        }
        Op::SetText { pick, text } => EditRequest::SetTextContent {
            seq,
            clip: pick_where(*pick, &|c| c.content.text().is_some())?,
            content: RichText::new(text.as_str()),
        },
        Op::TextStyle {
            pick,
            edit,
            value,
            family,
        } => EditRequest::SetTextStyle {
            seq,
            clips: vec![pick_where(*pick, &|c| c.content.text().is_some())?],
            edit: match edit {
                0 => TextStyleEdit::Font(FontRef::new(family.as_str())),
                1 => TextStyleEdit::Size(RefPx::from_hundredths(value % 200_000)),
                2 => {
                    let [r, g, b, a] = value.to_le_bytes();
                    TextStyleEdit::Fill(Rgba8::new(r, g, b, a).into())
                }
                3 => TextStyleEdit::Bold(value % 2 == 0),
                _ => TextStyleEdit::Italic(value % 2 == 0),
            },
        },
        Op::TextLayout { pick, edit, value } => EditRequest::SetTextLayout {
            seq,
            clips: vec![pick_where(*pick, &|c| c.content.text().is_some())?],
            edit: match edit {
                0 => TextLayoutEdit::Align(
                    [HAlign::Left, HAlign::Center, HAlign::Right]
                        [value.unsigned_abs() as usize % 3],
                ),
                1 if value % 3 == 0 => TextLayoutEdit::BoxWidth(None),
                1 => TextLayoutEdit::BoxWidth(Some(RefPx::from_hundredths(value % 2_000_000))),
                _ => {
                    TextLayoutEdit::LineSpacing(PerMille::new((value.unsigned_abs() % 5000) as u16))
                }
            },
        },
        Op::Transform { pick, edit, x, y } => EditRequest::SetTransform {
            seq,
            clips: vec![pick_where(*pick, &|c| c.content.is_visual())?],
            edit: match edit {
                0 => TransformEdit::Position(CanvasPos::new(*x, *y)),
                1 => TransformEdit::Scale(Scale::from_basis_points(x % 200_000)),
                2 => TransformEdit::Rotation(Angle::from_hundredths(x % 800_000)),
                3 => TransformEdit::FlipH(x % 2 == 0),
                4 => TransformEdit::FlipV(y % 2 == 0),
                5 => TransformEdit::Opacity(PerMille::new((x.unsigned_abs() % 1200) as u16)),
                _ => TransformEdit::Reset,
            },
        },
        Op::FadeCurve { pick, out, curve } => {
            let curve = FadeCurve::ALL[usize::from(*curve) % FadeCurve::ALL.len()];
            EditRequest::SetClipAudio {
                seq,
                clips: vec![pick_where(*pick, &|c| c.audio.active().is_some())?],
                edit: if *out {
                    AudioEdit::FadeOutCurve(curve)
                } else {
                    AudioEdit::FadeInCurve(curve)
                },
            }
        }
        Op::GainKeys {
            pick,
            edit,
            at,
            to,
            value,
        } => {
            let id = pick_where(*pick, &|c| c.audio.active().is_some())?;
            let clip = doc.clip(id)?;
            let keys = clip.audio.active()?.gain.keys();
            // Mostly at a key that exists, so edits of keys do something.
            let existing = |n: u8| keys.get(usize::from(n) % keys.len().max(1)).map(|k| k.at);
            let source = clip.content.source()?.start();
            let fresh = source.checked_add(frames(i64::from(*at) % 120)).ok()?;
            let gain = match value % 5 {
                0 => None,
                1 => Some(GainDb::SILENT),
                _ => Some(GainDb::from_hundredths(i32::from(*value))),
            };
            EditRequest::EditGainKeys {
                seq,
                clips: vec![id],
                edit: match edit {
                    0 | 1 => GainKeyEdit::Set { at: fresh, gain },
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
                        to: source.checked_add(frames(i64::from(*to) % 120)).ok()?,
                    },
                    4 => GainKeyEdit::Remove(vec![existing(*at)?]),
                    _ => GainKeyEdit::Clear,
                },
            }
        }
        Op::TrackMix { track, pan, value } => {
            let track = *tracks.get(usize::from(*track) % tracks.len().max(1))?;
            if *pan {
                EditRequest::SetTrackPan {
                    seq,
                    track,
                    pan: Pan::new(i32::from(*value) % 150),
                }
            } else {
                EditRequest::SetTrackGain {
                    seq,
                    track,
                    gain: if value % 7 == 0 {
                        GainDb::SILENT
                    } else {
                        GainDb::from_hundredths(i32::from(*value))
                    },
                }
            }
        }
        Op::Stream { pick, sel } => EditRequest::SetAudioStream {
            seq,
            clips: vec![pick_where(*pick, &|c| {
                matches!(c.content, ClipContent::Media { .. }) && c.audio.active().is_some()
            })?],
            sel: match sel {
                0 => None,
                1 => Some(StreamSel::AllMixed),
                n => Some(StreamSel::Index(u32::from(*n) - 1)),
            },
        },
        Op::AddStream { pick, stream } => EditRequest::AddStreamAsClip {
            seq,
            clip: pick_where(*pick, &|c| {
                matches!(
                    c.content,
                    ClipContent::Media {
                        kind: MediaKind::Video,
                        ..
                    }
                )
            })?,
            stream: u32::from(*stream),
        },
        Op::TextLook { pick, edit, a, b } => {
            let clip = pick_where(*pick, &|c| c.content.text().is_some())?;
            let edit = text_look(*edit, *a, *b);
            match edit {
                Ok(edit) => EditRequest::SetTextStyle {
                    seq,
                    clips: vec![clip],
                    edit,
                },
                Err(edit) => EditRequest::SetTextLayout {
                    seq,
                    clips: vec![clip],
                    edit,
                },
            }
        }
        Op::Keys {
            pick,
            prop,
            edit,
            at,
            to,
            a,
            b,
        } => {
            let id = pick_where(*pick, &|c| c.content.is_visual())?;
            let clip = doc.clip(id)?;
            let mut props: Vec<PropPath> = TransformProp::ALL.map(PropPath::Transform).to_vec();
            if clip.content.text().is_some() {
                props.extend(PropPath::all_fixed().into_iter().filter(|p| {
                    !matches!(
                        p,
                        PropPath::Transform(_) | PropPath::Color(_) | PropPath::ColorPuck(_)
                    )
                }));
                props.extend((0..4).map(PropPath::FillStopColor));
            }
            // Edits of existing keys mostly pick a property that has some.
            let keyed: Vec<PropPath> = clip.keys.tracks().iter().map(|t| t.prop()).collect();
            let prop = if matches!(edit % 8, 3..=5) && !keyed.is_empty() {
                keyed[usize::from(*prop) % keyed.len()]
            } else {
                props[usize::from(*prop) % props.len()]
            };
            // A playhead inside the clip.
            let playhead = |n: u8| {
                Tick::from_raw(clip.start().raw() + clip.duration().raw() * i64::from(n) / 256)
            };
            // Mostly at a key that exists, so edits of keys do something.
            let existing = |n: u8| {
                let keys = clip.keys.keys(prop);
                let key = keys.get(usize::from(n) % keys.len().max(1))?;
                clip.sequence_time_of(key.at)
            };
            let value = key_value(prop, *a, *b);
            let card = prop.group().unwrap_or(PropGroup::Transform);
            EditRequest::EditKeys {
                seq,
                clips: vec![id],
                prop,
                edit: match edit % 8 {
                    0..=2 => KeyEdit::Add {
                        at: playhead(*at),
                        value,
                    },
                    3 => KeyEdit::Remove { at: existing(*at)? },
                    4 => KeyEdit::Move {
                        from: existing(*at)?,
                        to: playhead(*to),
                    },
                    5 => KeyEdit::SetInterp {
                        at: existing(*at)?,
                        interp: Interp::PRESETS[a.unsigned_abs() as usize % Interp::PRESETS.len()],
                    },
                    6 => KeyEdit::KeyCard {
                        card,
                        at: playhead(*at),
                    },
                    _ => KeyEdit::ClearCard {
                        card,
                        at: (a % 2 != 0).then(|| playhead(*at)),
                    },
                },
            }
        }
        Op::Animate {
            pick,
            slot,
            preset,
            len,
            a,
            b,
        } => {
            let id = pick_where(*pick, &|c| c.content.is_visual())?;
            EditRequest::SetClipAnimation {
                seq,
                clips: vec![id],
                slot: Slot::ALL[usize::from(*slot) % 3],
                spec: (*len != 0).then(|| animation(*preset, frames(i64::from(*len)), *a, *b)),
            }
        }
        Op::GainEase { pick, key, interp } => {
            let id = pick_where(*pick, &|c| {
                c.audio.active().is_some_and(|a| !a.gain.keys().is_empty())
            })?;
            let keys = doc.clip(id)?.audio.active()?.gain.keys();
            EditRequest::EditGainKeys {
                seq,
                clips: vec![id],
                edit: GainKeyEdit::Interp {
                    at: keys.get(usize::from(*key) % keys.len().max(1))?.at,
                    interp: Interp::PRESETS[usize::from(*interp) % Interp::PRESETS.len()],
                },
            }
        }
        Op::Marker {
            edit,
            pick,
            at,
            a,
            text,
        } => {
            let sequence = doc.sequence(seq)?;
            let mut refs: Vec<MarkerRef> = sequence
                .markers
                .iter()
                .map(|m| MarkerRef::Timeline(m.id))
                .collect();
            for (_, track) in sequence.tracks() {
                for clip in track.clips() {
                    refs.extend(clip.markers.iter().map(|m| MarkerRef::Clip(clip.id, m.id)));
                }
            }
            let picked = || refs.get(usize::from(*pick) % refs.len().max(1)).copied();
            // `a % 2`, `a % 3` and `a % 5` pick other things below.
            let color = PaletteColor::ALL[usize::from(*a / 30) % PaletteColor::ALL.len()];
            let at = frames(i64::from(*at));
            match edit % 7 {
                0 | 1 => {
                    // A clip marker goes inside its clip (the edit refuses
                    // one outside), so most of them land.
                    let owner = match a % 3 {
                        0 => MarkerOwner::Timeline,
                        1 => MarkerOwner::Clip(pick_clip(*pick)?),
                        _ => MarkerOwner::Auto(pick_clip(*pick).into_iter().collect()),
                    };
                    let inside = match &owner {
                        MarkerOwner::Clip(id) => Some(*id),
                        MarkerOwner::Auto(ids) => ids.first().copied(),
                        MarkerOwner::Timeline => None,
                    };
                    let at = match inside {
                        Some(id) if a % 5 != 0 => {
                            let clip = doc.clip(id)?;
                            let span = (clip.duration().raw() / frames(1).raw()).max(1);
                            clip.start()
                                .checked_add(frames(at.raw() / frames(1).raw() % span))
                                .ok()?
                        }
                        _ => at,
                    };
                    EditRequest::AddMarker {
                        seq,
                        at,
                        owner,
                        color: (a % 2 == 0).then_some(color),
                    }
                }
                2 | 3 => EditRequest::EditMarker {
                    seq,
                    marker: picked()?,
                    patch: MarkerPatch {
                        name: Some(text.clone()),
                        note: (a % 2 == 0).then(|| text.repeat(3)),
                        color: Some(color),
                        duration: Some(frames(i64::from(a % 20))),
                    },
                },
                4 => EditRequest::MoveMarker {
                    seq,
                    marker: picked()?,
                    to: at,
                },
                5 => EditRequest::DeleteMarkers {
                    seq,
                    which: MarkerSelector::Refs(vec![picked()?]),
                },
                _ => EditRequest::DeleteMarkers {
                    seq,
                    which: match a % 3 {
                        0 => MarkerSelector::Color(color),
                        1 => MarkerSelector::OnClip(pick_clip(*pick)?),
                        _ => MarkerSelector::AllTimeline,
                    },
                },
            }
        }
        Op::Follow { on, kinds } => match kinds {
            None => EditRequest::SetFollowMain { seq, on: *on },
            Some(bits) => EditRequest::SetFollowKinds {
                seq,
                kinds: FollowKinds::from_bits(*bits),
            },
        },
        Op::Record {
            at,
            len,
            preroll,
            new_track,
            replace,
        } => {
            let handle = frames(i64::from(*preroll));
            let duration = frames(i64::from(*len) + i64::from(*preroll));
            let name = format!("Voiceover {at}-{len}-{preroll}.wav");
            let replace = match replace {
                Some(pick) => Some(pick_where(*pick, &|c| {
                    c.content
                        .asset()
                        .and_then(|asset| doc.library().get(asset))
                        .is_some_and(|m| m.origin != MediaOrigin::Imported)
                })?),
                None => None,
            };
            EditRequest::InsertRecordedClip {
                seq,
                media: NewMedia {
                    path: StoredPath {
                        absolute: format!("/p/Trip Media/Recordings/{name}"),
                        relative: Some(format!("Trip Media/Recordings/{name}")),
                    },
                    info: MediaInfo {
                        container: "wav".into(),
                        default_video: None,
                        duration,
                        video: Vec::new(),
                        audio: vec![AudioStreamInfo {
                            index: 0,
                            codec: "pcm_s24le".into(),
                            sample_rate: 48_000,
                            channels: 1 + u16::from(*new_track),
                            start: Tick::ZERO,
                            duration,
                            duration_exact: true,
                            default: false,
                            language: None,
                            title: None,
                            layout: None,
                        }],
                        still: None,
                    },
                    fingerprint: None,
                    display_name: None,
                },
                at: frames(i64::from(*at)),
                handle_in: handle,
                target: if *new_track {
                    RecordTarget::NewTrack
                } else {
                    RecordTarget::FirstFree
                },
                replace,
            }
        }
        Op::Color { pick, edit, a, b } => {
            use freecut_timeline::edit::ColorEdit as E;
            let id = pick_where(*pick, &|c| c.content.takes_color())?;
            let clip = doc.clip(id)?;
            // Format 10's sliders: Light, Color and Amount.
            let k1 = &ColorParamId::ALL[..15];
            let param = k1[usize::from(a.unsigned_abs()) % k1.len()];
            let section = Section::ALL[usize::from(b.unsigned_abs()) % Section::ALL.len()];
            let wild = |v: i16| v % 160;
            let request = |edit| EditRequest::EditColor {
                seq,
                clips: vec![id],
                edit,
            };
            match edit % 8 {
                0 | 1 => request(E::SetParam(param, wild(*b))),
                2 => request(E::SetParams(vec![
                    (param, wild(*b)),
                    (ColorParamId::Temperature, wild(*a)),
                ])),
                3 => request(E::SetSectionEnabled(section, a % 2 == 0)),
                4 => request(E::SetBypass(b % 3 == 0)),
                5 => request(E::ResetSection(section)),
                _ => {
                    // A key at a playhead inside the clip.
                    let at = Tick::from_raw(
                        clip.start().raw()
                            + clip.duration().raw() * i64::from(a.unsigned_abs() % 256) / 256,
                    );
                    EditRequest::EditKeys {
                        seq,
                        clips: vec![id],
                        prop: PropPath::Color(param),
                        edit: KeyEdit::Add {
                            at,
                            value: Some(PropValue::Level(wild(*b))),
                        },
                    }
                }
            }
        }
        Op::ColorK2 {
            pick,
            edit,
            a,
            b,
            c,
        } => {
            use freecut_timeline::edit::ColorEdit as E;
            let id = pick_where(*pick, &|c| c.content.takes_color())?;
            let clip = doc.clip(id)?;
            // Format 11's sliders: HSL and Wheels.
            let k2 = &ColorParamId::ALL[15..44];
            let param = k2[usize::from(a.unsigned_abs()) % k2.len()];
            let band = HueBand::ALL[usize::from(b.unsigned_abs()) % 8];
            let wheel = WheelId::ALL[usize::from(c.unsigned_abs()) % 4];
            let channel = CurveChannel::ALL[usize::from(a.unsigned_abs()) % 4];
            let request = |edit| EditRequest::EditColor {
                seq,
                clips: vec![id],
                edit,
            };
            let at = Tick::from_raw(
                clip.start().raw()
                    + clip.duration().raw() * i64::from(b.unsigned_abs() % 256) / 256,
            );
            match edit % 8 {
                0 => request(E::SetParam(param, b % 160)),
                1 => request(E::SetPuck {
                    wheel,
                    x: *a,
                    y: *b,
                }),
                2 => {
                    // A curve through up to three random inner points; a
                    // list that breaks a rule leaves the curve as it is.
                    let base = ColorStack::of(clip.color.as_ref())
                        .curves
                        .get(channel)
                        .clone();
                    let x = 300 + (a.unsigned_abs() % 9_400);
                    let y = b.unsigned_abs() % 10_001;
                    let curve = base.with_point(CurvePoint::new(x, y)).unwrap_or_else(|_| {
                        base.moved(0, CurvePoint::new(c.unsigned_abs() % 3_000, y))
                    });
                    request(E::SetCurve { channel, curve })
                }
                3 => {
                    let base = ColorStack::of(clip.color.as_ref())
                        .curves
                        .get(channel)
                        .clone();
                    let n = base.points().len();
                    let curve = base
                        .without_point(usize::from(c.unsigned_abs()) % n)
                        .unwrap_or(base);
                    request(E::SetCurve { channel, curve })
                }
                4 => request(E::ResetBand(band)),
                5 => request(E::ResetWheel(wheel)),
                6 => EditRequest::EditKeys {
                    seq,
                    clips: vec![id],
                    prop: PropPath::ColorPuck(wheel),
                    edit: KeyEdit::Add {
                        at,
                        value: (c % 4 != 0).then_some(PropValue::Puck { x: *a, y: *c }),
                    },
                },
                _ => EditRequest::EditKeys {
                    seq,
                    clips: vec![id],
                    prop: PropPath::Color(param),
                    edit: KeyEdit::Add {
                        at,
                        value: Some(PropValue::Level(c % 160)),
                    },
                },
            }
        }
        Op::ColorK3 { pick, edit, a, b } => {
            use freecut_timeline::edit::ColorEdit as E;
            let id = pick_where(*pick, &|c| c.content.takes_color())?;
            let clip = doc.clip(id)?;
            // Format 12's sliders: the LUT's Intensity and the skin Amounts
            // (the LUT's only while the clip has a LUT).
            let has_lut = clip.color.as_ref().is_some_and(|c| c.lut.is_some());
            let k3 = if has_lut {
                &ColorParamId::ALL[44..47]
            } else {
                &ColorParamId::ALL[45..46]
            };
            let param = k3[usize::from(a.unsigned_abs()) % k3.len()];
            let scope = SkinScope::ALL[usize::from(b.unsigned_abs()) % 2];
            let request = |edit| EditRequest::EditColor {
                seq,
                clips: vec![id],
                edit,
            };
            let lut = |n: i16| {
                let n = n.unsigned_abs() % 3;
                let mut hash = [0x5au8; 16];
                hash[0] = n as u8;
                LutRef::new(hash, format!("look {n}.cube"))
            };
            match edit % 7 {
                0 | 1 => request(E::SetLut(Some(LutSlot {
                    intensity: (b.unsigned_abs() % 101) as u8,
                    skin: SkinProtect {
                        on: b % 2 == 0,
                        amount: (a.unsigned_abs() % 101) as u8,
                    },
                    ..LutSlot::new(lut(*a))
                }))),
                2 => request(E::SetLut(None)),
                3 => request(E::SetLutInterp(
                    LutInterp::ALL[usize::from(a.unsigned_abs()) % 2],
                )),
                4 => request(E::SetSkin {
                    scope,
                    on: a % 3 != 0,
                }),
                5 => request(E::SetParam(param, b % 160)),
                _ => EditRequest::EditKeys {
                    seq,
                    clips: vec![id],
                    prop: PropPath::Color(param),
                    edit: KeyEdit::Add {
                        at: Tick::from_raw(
                            clip.start().raw()
                                + clip.duration().raw() * i64::from(b.unsigned_abs() % 256) / 256,
                        ),
                        value: Some(PropValue::Level(b % 160)),
                    },
                },
            }
        }
        Op::ColorK5 { pick, edit, a, b } => {
            use freecut_timeline::edit::ColorEdit as E;
            // Slider edits and keys go to a clip with a result when there
            // is one.
            let with_result = pick_where(*pick, &|c| {
                c.color
                    .as_ref()
                    .is_some_and(|s| s.auto.is_some() || s.match_xf.is_some())
            });
            let id = match with_result {
                Some(id) if edit % 8 >= 6 => id,
                _ => pick_where(*pick, &|c| c.content.takes_color())?,
            };
            let clip = doc.clip(id)?;
            let request = |edit| EditRequest::EditColor {
                seq,
                clips: vec![id],
                edit,
            };
            // Format 13's sliders, each only while the clip has its result.
            let stack = ColorStack::of(clip.color.as_ref());
            // The slider of a result the clip has, when it has one.
            let param = if stack.auto.is_some() && (a % 2 == 0 || stack.match_xf.is_none()) {
                ColorParamId::AutoStrength
            } else {
                ColorParamId::MatchIntensity
            };
            let level = |v: i16| v % 101;
            match edit % 8 {
                0 | 1 => request(E::SetAuto(Some(AutoCorrection {
                    strength: (b.unsigned_abs() % 101) as u8,
                    ..AutoCorrection::new(
                        AutoOffsets {
                            exposure: level(*a),
                            temperature: level(*b),
                            blacks: level(a.wrapping_mul(3)),
                            vibrance: level(b.wrapping_mul(7)),
                            ..AutoOffsets::default()
                        },
                        AutoStyle::ALL[usize::from(a.unsigned_abs()) % 2],
                        AutoSource::ALL[usize::from(b.unsigned_abs()) % 2],
                    )
                }))),
                2 => request(if a % 2 == 0 {
                    E::ApplyAutoToSliders
                } else {
                    E::SetAuto(None)
                }),
                3 | 4 => {
                    let at = Tick::from_raw(i64::from(*b) * 1_000);
                    let reference = if a % 3 == 0 {
                        MatchRef::Media {
                            media: MediaId::from_raw(nz(u64::from(a.unsigned_abs()) + 1)),
                            at,
                        }
                    } else {
                        MatchRef::Clip {
                            clip: id,
                            at,
                            graded: b % 2 == 0,
                        }
                    };
                    request(E::SetMatch(Some(
                        MatchTransform::new(
                            [i32::from(*a) + 10_000, i32::from(*b) / 2 + 10_000, 10_000],
                            [i32::from(*b) / 4, i32::from(*a) / 8, -i32::from(*b) / 16],
                            reference,
                        )
                        .with_intensity((a.unsigned_abs() % 101) as u8),
                    )))
                }
                5 => request(E::SetMatch(None)),
                6 if a % 3 == 0 => request(E::SetParam(param, b % 160)),
                _ if stack.has_param(param) => EditRequest::EditKeys {
                    seq,
                    clips: vec![id],
                    prop: PropPath::Color(param),
                    edit: KeyEdit::Add {
                        at: Tick::from_raw(
                            clip.start().raw()
                                + clip.duration().raw() * i64::from(b.unsigned_abs() % 256) / 256,
                        ),
                        value: Some(PropValue::Level(b % 160)),
                    },
                },
                _ => request(E::SetSectionEnabled(
                    [Section::Auto, Section::Match][usize::from(b.unsigned_abs()) % 2],
                    a % 2 == 0,
                )),
            }
        }
        Op::ColorK6 { pick, edit, a, b } => {
            use freecut_timeline::edit::ColorEdit as E;
            use freecut_timeline::model::{
                ColorMask, ColorMaskParam, Mask, MaskCombine, MaskPoint, MaskShapeKind,
            };
            // Edits of a mask go to a clip with masks when there is one.
            let masked = pick_where(*pick, &|c| {
                c.color.as_ref().is_some_and(|s| !s.masks.is_empty())
            });
            let id = match masked {
                Some(id) if edit % 8 > 1 => id,
                _ => pick_where(*pick, &|c| c.content.takes_color())?,
            };
            let clip = doc.clip(id)?;
            let request = |edit| EditRequest::EditColor {
                seq,
                clips: vec![id],
                edit,
            };
            let stack = ColorStack::of(clip.color.as_ref());
            let kind = MaskShapeKind::ALL[usize::from(a.unsigned_abs()) % 4];
            // One of the clip's masks, when it has any.
            let existing = (!stack.masks.is_empty())
                .then(|| stack.masks[usize::from(b.unsigned_abs()) % stack.masks.len()]);
            match (edit % 8, existing) {
                (0 | 1, _) | (_, None) => request(E::AddMask(Mask {
                    center: MaskPoint::new(i32::from(*a) * 2, i32::from(*b)),
                    feather: PerMille::new(b.unsigned_abs() % 1_200),
                    invert: a % 5 == 0,
                    ..Mask::new(kind)
                })),
                (2, Some(m)) => request(E::RemoveMask(m.id)),
                (3, Some(m)) => request(E::ReorderMask {
                    id: m.id,
                    to: usize::from(a.unsigned_abs()) % 9,
                }),
                (4, Some(m)) => request(E::SetMaskMode {
                    id: m.id,
                    mode: MaskCombine::ALL[usize::from(a.unsigned_abs()) % 3],
                }),
                (5, Some(m)) => request(E::SetMask(ColorMask {
                    mask: Mask {
                        size: MaskPoint::new(i32::from(*a) * 3, i32::from(*b) * 3),
                        rotation: Angle::from_hundredths(i32::from(*b) * 7),
                        linked: b % 2 == 0,
                        ..m.mask.with_kind(kind)
                    },
                    expand: a % 150,
                    ..m
                })),
                (6, Some(_)) => request(E::SetSectionEnabled(Section::Masks, a % 2 == 0)),
                (_, Some(m)) => {
                    let param = ColorMaskParam::ALL[usize::from(a.unsigned_abs()) % 7];
                    let prop = PropPath::ColorMask { id: m.id, param };
                    if prop.get(clip).is_none() {
                        return Some(request(E::SetMaskMode {
                            id: m.id,
                            mode: MaskCombine::ALL[usize::from(b.unsigned_abs()) % 3],
                        }));
                    }
                    EditRequest::EditKeys {
                        seq,
                        clips: vec![id],
                        prop,
                        edit: KeyEdit::Add {
                            at: Tick::from_raw(
                                clip.start().raw()
                                    + clip.duration().raw() * i64::from(b.unsigned_abs() % 256)
                                        / 256,
                            ),
                            value: key_value(prop, i32::from(*a) * 37, i32::from(*b) * 53),
                        },
                    }
                }
            }
        }
    })
}

/// A key value for `prop` built from `a` and `b`, past the property's range
/// at times so the edit's clamping is part of what gets saved; `None` (take
/// the current value) for a quarter of the values.
fn key_value(prop: PropPath, a: i32, b: i32) -> Option<PropValue> {
    if a % 4 == 0 {
        return None;
    }
    let [a0, a1, ..] = a.to_le_bytes();
    let [b0, b1, ..] = b.to_le_bytes();
    Some(match prop.kind()? {
        ValueKind::Position => PropValue::Position(CanvasPos::new(a % 3_000_000, b % 3_000_000)),
        ValueKind::Scale => PropValue::Scale(Scale::from_basis_points(a % 2_000_000)),
        ValueKind::Angle => PropValue::Angle(Angle::from_hundredths(a % 1_000_000)),
        ValueKind::Ratio => PropValue::Ratio(PerMille::new(u16::from_le_bytes([a0, a1]) % 4_000)),
        ValueKind::Length => PropValue::Length(RefPx::from_hundredths(a % 300_000)),
        ValueKind::SignedRatio => {
            PropValue::SignedRatio(SignedPerMille::new(i16::from_le_bytes([a0, a1])))
        }
        ValueKind::Direction => PropValue::Direction(DeciDegrees::new(a)),
        ValueKind::Color => PropValue::Color(Rgba8::new(a0, a1, b0, b1)),
        ValueKind::TailTip => PropValue::TailTip(TailTip {
            x: RefPx::from_hundredths(a % 200_000),
            y: RefPx::from_hundredths(b % 200_000),
        }),
        ValueKind::Level => PropValue::Level((a % 150) as i16),
        ValueKind::Puck => PropValue::Puck {
            x: (a % 1_500) as i16,
            y: (b % 1_500) as i16,
        },
        ValueKind::MaskPoint => PropValue::MaskPoint(freecut_timeline::model::MaskPoint::new(
            a % 50_000,
            b % 50_000,
        )),
    })
}

/// An animation of one of a few presets (some this build has, one it
/// doesn't), lasting `len`, with settings built from `a` and `b`.
fn animation(preset: u8, len: Tick, a: u32, b: u32) -> AnimationSpec {
    const PRESETS: [&str; 8] = [
        "fc.in.fade",
        "fc.in.rise",
        "fc.in.slide",
        "fc.in.pop",
        "fc.in.typewriter",
        "fc.out.fade",
        "fc.loop.wave",
        "fc.loop.nowhere",
    ];
    let pick = |n: u32, of: usize| n as usize % of;
    let directions = [
        None,
        Some(Direction::Left),
        Some(Direction::Right),
        Some(Direction::Up),
        Some(Direction::Down),
        Some(Direction::Clockwise),
        Some(Direction::CounterClockwise),
    ];
    let units = [
        None,
        Some(AnimUnit::Block),
        Some(AnimUnit::Line),
        Some(AnimUnit::Word),
        Some(AnimUnit::Letter),
    ];
    let orders = [
        AnimOrder::Forward,
        AnimOrder::Backward,
        AnimOrder::CenterOut,
        AnimOrder::EdgesIn,
        AnimOrder::Random,
    ];
    AnimationSpec {
        preset: PresetId::new(PRESETS[usize::from(preset) % PRESETS.len()]),
        duration: len,
        easing: (!a.is_multiple_of(3))
            .then(|| Interp::PRESETS[pick(a >> 2, Interp::PRESETS.len())]),
        direction: directions[pick(b, directions.len())],
        params: AnimParams {
            unit: units[pick(a >> 8, units.len())],
            overlap: PerMille::new(((a >> 12) % 1_001) as u16),
            order: orders[pick(b >> 4, orders.len())],
            seed: b,
            pace: (b >> 8)
                .is_multiple_of(3)
                .then(|| Tick::from_raw(TICKS_PER_SECOND / 50 * i64::from(1 + (b >> 12) % 50))),
            intensity: PerMille::new(((b >> 16) % 1_001) as u16),
            travel: if a & 1 == 0 {
                Travel::FromEdge
            } else {
                Travel::Short
            },
            caret: a & 2 == 0,
        },
        phase: Tick::ZERO,
        // Kept only on a Loop that has none yet (as a split leaves them).
        seams: (a & 4 == 0).then(|| LoopSeams {
            start: (b & 1 == 0).then(|| Tick::from_raw(i64::from(b % 997) * 1_000)),
            end: (b & 2 == 0).then(|| Tick::from_raw(i64::from(a % 991) * 1_000_000)),
            ramp: Tick::from_raw(i64::from(a % 1_000)),
        }),
    }
}

/// One of the format-5 text edits, chosen by `edit` and built from `a` and
/// `b`: a style edit, or (`Err`) a layout edit. Values go past their ranges
/// so the edits' clamping is part of what gets saved.
fn text_look(edit: u8, a: i32, b: i32) -> Result<TextStyleEdit, TextLayoutEdit> {
    use TextStyleEdit as E;
    let u = a.unsigned_abs();
    let pm = PerMille::new((u % 2_500) as u16);
    let signed = SignedPerMille::new((a % 1_500) as i16);
    let angle = DeciDegrees::new(a);
    let [r, g, bl, al] = b.to_le_bytes();
    let color = Rgba8::new(r, g, bl, al);
    let layers = [
        TextLayerKind::Outline,
        TextLayerKind::Shadow,
        TextLayerKind::Glow,
        TextLayerKind::Background,
        TextLayerKind::Bubble,
        TextLayerKind::Curve,
    ];
    let kinds = [
        BubbleKind::Rounded,
        BubbleKind::Oval,
        BubbleKind::Thought,
        BubbleKind::Shout,
        BubbleKind::Square,
        BubbleKind::Tag,
        BubbleKind::ArrowTag { point: Side::Left },
        BubbleKind::ArrowTag { point: Side::Right },
        BubbleKind::Ticket,
    ];
    let pick = |n: usize| u as usize % n;
    Ok(match edit % 72 {
        // Layer switches are the most common edit, so layers get turned on.
        0..=7 => E::SetLayer {
            layer: layers[pick(6)],
            on: b % 4 != 0,
        },
        8 | 9 => E::FillKind(
            [
                FillKind::Solid,
                FillKind::LinearGradient,
                FillKind::RadialGradient,
            ][pick(3)],
        ),
        70 => E::FillStops(
            GradientStops::new(
                (0..2 + pick(7))
                    .map(|i| GradientStop {
                        at: PerMille::new(((u >> i) % 1_001) as u16),
                        color: Rgba8::new(r ^ i as u8, g, bl, al),
                    })
                    .collect(),
            )
            .unwrap(),
        ),
        10 => E::FillStopColor {
            index: (u % 9) as u8,
            color,
        },
        11 => E::FillAngle(angle),
        12 => E::FillCenter(FillCenter {
            x: pm,
            y: PerMille::new((b.unsigned_abs() % 1_500) as u16),
        }),
        13 => E::FillSpread(pm),
        14 => E::FillExtent([FillExtent::Block, FillExtent::Line][pick(2)]),
        15 => E::Fill(color.into()),
        16 => E::Underline(b % 2 == 0),
        17 => E::Strike(b % 2 == 0),
        18 => E::Case(
            [
                CaseTransform::AsTyped,
                CaseTransform::Upper,
                CaseTransform::Lower,
                CaseTransform::Title,
                CaseTransform::Sentence,
            ][pick(5)],
        ),
        19 => E::LetterSpacing(signed),
        20 => E::OutlineColor(color),
        21 => E::OutlineWidth(pm),
        22 => E::OutlineOpacity(pm),
        23 => E::OutlinePosition(
            [
                StrokePosition::Outside,
                StrokePosition::Center,
                StrokePosition::Inside,
            ][pick(3)],
        ),
        24 => E::GlowColor(color),
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
        30 => E::ShadowColor(color),
        31 => E::ShadowOpacity(pm),
        32 => E::ShadowBlur(pm),
        33 => E::ShadowDistance(pm),
        34 => E::ShadowAngle(angle),
        35 => E::ShadowIncludeBox(b % 2 == 0),
        36 => E::BackgroundColor(color),
        37 => E::BackgroundOpacity(pm),
        38 => E::BackgroundShape([BoxShape::Block, BoxShape::PerLine, BoxShape::PerChar][pick(3)]),
        39 => E::BackgroundPadX(pm),
        40 => E::BackgroundPadY(pm),
        41 => E::BackgroundRoundness(pm),
        42 => E::BackgroundHeight([BoxHeight::Full, BoxHeight::LowerHalf][pick(2)]),
        43 => E::BackgroundBorder(b % 2 == 0),
        44 => E::BackgroundBorderColor(color),
        45 => E::BackgroundBorderWidth(pm),
        46 => E::BubbleKind(kinds[pick(kinds.len())]),
        47 => E::BubbleTail(b % 2 == 0),
        48 => E::BubbleColor(color),
        49 => E::BubbleOpacity(pm),
        50 => E::BubbleTailTip(TailTip {
            x: RefPx::from_hundredths(a),
            y: RefPx::from_hundredths(b),
        }),
        51 => E::BubbleTailBase(pm),
        52 => E::BubbleTailBend(signed),
        53 => E::BubblePadX(pm),
        54 => E::BubblePadY(pm),
        55 => E::BubbleRoundness(pm),
        56 => E::BubbleBorder(b % 2 == 0),
        57 => E::BubbleBorderColor(color),
        58 => E::BubbleBorderWidth(pm),
        59 => E::BubbleSpikes(a as u8),
        60 => E::BubbleSpikeDepth(pm),
        61 => E::BubbleJitter(pm),
        62 => E::BubbleSeed(b as u32),
        63 => E::BubbleLobe(pm),
        64 => E::BubbleTrail(a as u8),
        65 | 66 => E::Curve(signed),
        67 => return Err(TextLayoutEdit::Justify(b % 2 == 0)),
        68 => {
            return Err(TextLayoutEdit::VAnchor(
                [VAnchor::Top, VAnchor::Middle, VAnchor::Bottom][pick(3)],
            ));
        }
        69 => {
            return Err(TextLayoutEdit::Align(
                [HAlign::Left, HAlign::Center, HAlign::Right][pick(3)],
            ));
        }
        _ => E::Size(RefPx::from_hundredths(a % 200_000)),
    })
}

/// Builds a valid document by importing media and applying edits through
/// the editor; refused edits are skipped. Returns the document and the
/// editor's id counter.
pub fn build(plan: &DocPlan) -> (Document, NonZeroU64) {
    let mut editor = Editor::with_settings(plan.settings, HistoryLimits::default()).unwrap();
    let ctx = EditCtx {
        limits: Limits::default(),
    };
    let media = plan
        .media
        .iter()
        .map(|m| new_media(m, plan.settings.frame_rate))
        .collect();
    let import = editor.plan(&ctx, &EditRequest::ImportMedia(media)).unwrap();
    editor.commit(import, CommitMeta::default()).unwrap();
    for op in &plan.ops {
        let Some(req) = request(&editor, op) else {
            continue;
        };
        if let Ok(p) = editor.plan(&ctx, &req) {
            editor.commit(p, CommitMeta::default()).unwrap();
        }
    }
    (editor.document().clone(), editor.ids().next())
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

/// Every clip object of every sequence in `v`.
fn clips_mut(v: &mut Value) -> impl Iterator<Item = &mut serde_json::Map<String, Value>> {
    v["document"]["sequences"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .flat_map(|seq| {
            let seq = seq.as_object_mut().unwrap();
            let mut lanes = Vec::new();
            for (key, lane) in seq.iter_mut() {
                if key == "visual" || key == "audio" {
                    lanes.push(lane);
                }
            }
            lanes
        })
        .flat_map(|lane| lane.as_array_mut().unwrap().iter_mut())
        .flat_map(|track| track["clips"].as_array_mut().unwrap().iter_mut())
        .map(|clip| clip.as_object_mut().unwrap())
}

/// Whether a keyframe path is one format 13 added (Auto's Strength and
/// Match's Intensity).
pub fn is_k5_key(prop: &str) -> bool {
    prop.starts_with("color.auto.") || prop.starts_with("color.match.")
}

/// `v`, a current file, as format 12 wrote it: format 13 added `auto` and
/// `match` inside `color`, and keyframes of Auto's Strength and Match's
/// Intensity. Only faithful for documents without them.
/// Whether a keyframe path is one format 14 added (a colour mask's).
pub fn is_k6_key(prop: &str) -> bool {
    prop.starts_with("color.masks.")
}

/// `v`, a current file, as format 13 wrote it: format 14 added `masks`
/// inside `color` and keyframes of mask values. Only faithful for
/// documents without them.
pub fn to_v13(v: &Value) -> Value {
    let mut v = v.clone();
    v["format_version"] = Value::from(13);
    for clip in clips_mut(&mut v) {
        if let Some(color) = clip.get("color") {
            assert!(color.get("masks").is_none(), "format 13 had no masks");
        }
        if let Some(tracks) = clip.get("keys").and_then(|k| k["tracks"].as_array()) {
            for t in tracks {
                assert!(
                    !is_k6_key(t["prop"].as_str().unwrap()),
                    "format 13 had no K6 keys"
                );
            }
        }
    }
    v
}

/// The kitchen sink's file without what format 14 added: no `masks`, no
/// Masks switch and no mask keyframes.
pub fn strip_k6(v: &mut Value) {
    for clip in clips_mut(v) {
        if let Some(Value::Object(color)) = clip.get_mut("color") {
            color.remove("masks");
            if let Some(Value::Array(off)) = color.get_mut("off") {
                off.retain(|s| s != "masks");
                if off.is_empty() {
                    color.remove("off");
                }
            }
        }
        let keys = clip.get_mut("keys").and_then(|k| k.get_mut("tracks"));
        if let Some(Value::Array(tracks)) = keys {
            tracks.retain(|t| !is_k6_key(t["prop"].as_str().unwrap_or("")));
        }
    }
}

pub fn to_v12(v: &Value) -> Value {
    let mut v = to_v13(v);
    v["format_version"] = Value::from(12);
    for clip in clips_mut(&mut v) {
        if let Some(color) = clip.get("color") {
            assert!(color.get("auto").is_none(), "format 12 had no Auto");
            assert!(color.get("match").is_none(), "format 12 had no Match");
        }
        if let Some(tracks) = clip.get("keys").and_then(|k| k["tracks"].as_array()) {
            for t in tracks {
                assert!(
                    !is_k5_key(t["prop"].as_str().unwrap()),
                    "format 12 had no K5 keys"
                );
            }
        }
    }
    v
}

/// The kitchen sink's file without what format 13 added: no `auto` or
/// `match`, no Match switch and no Strength or Intensity keyframes.
pub fn strip_k5(v: &mut Value) {
    for clip in clips_mut(v) {
        if let Some(Value::Object(color)) = clip.get_mut("color") {
            color.remove("auto");
            color.remove("match");
            if let Some(Value::Array(off)) = color.get_mut("off") {
                off.retain(|s| s != "auto" && s != "match");
                if off.is_empty() {
                    color.remove("off");
                }
            }
        }
        let keys = clip.get_mut("keys").and_then(|k| k.get_mut("tracks"));
        if let Some(Value::Array(tracks)) = keys {
            tracks.retain(|t| !is_k5_key(t["prop"].as_str().unwrap_or("")));
        }
    }
}

/// Whether a keyframe path is one format 12 added (the LUT's Intensity
/// and the skin Amounts).
pub fn is_k3_key(prop: &str) -> bool {
    prop.starts_with("color.lut.") || prop == "color.skin_amount"
}

/// `v`, a current file, as format 11 wrote it: format 12 added `lut` inside
/// `color`, `skin` inside its Color section, and keyframes of the LUT's
/// Intensity and the skin Amounts. Only faithful for documents without
/// them.
pub fn to_v11(v: &Value) -> Value {
    let mut v = to_v12(v);
    v["format_version"] = Value::from(11);
    for clip in clips_mut(&mut v) {
        if let Some(color) = clip.get("color") {
            assert!(color.get("lut").is_none(), "format 11 had no LUT");
            assert!(
                color.get("color").and_then(|c| c.get("skin")).is_none(),
                "format 11 had no skin protection"
            );
        }
        if let Some(tracks) = clip.get("keys").and_then(|k| k["tracks"].as_array()) {
            for t in tracks {
                assert!(
                    !is_k3_key(t["prop"].as_str().unwrap()),
                    "format 11 had no K3 keys"
                );
            }
        }
    }
    v
}

/// The kitchen sink's file without what format 12 added: no `lut`, no
/// `skin` and no LUT or skin keyframes.
pub fn strip_k3(v: &mut Value) {
    for clip in clips_mut(v) {
        if let Some(Value::Object(color)) = clip.get_mut("color") {
            color.remove("lut");
            if let Some(Value::Object(balance)) = color.get_mut("color") {
                balance.remove("skin");
                if balance.is_empty() {
                    color.remove("color");
                }
            }
        }
        let keys = clip.get_mut("keys").and_then(|k| k.get_mut("tracks"));
        if let Some(Value::Array(tracks)) = keys {
            tracks.retain(|t| !is_k3_key(t["prop"].as_str().unwrap_or("")));
        }
    }
}

/// Whether a keyframe path is one format 11 added (pucks, HSL and wheel
/// sliders).
pub fn is_k2_key(prop: &str) -> bool {
    prop.starts_with("color.hsl.") || prop.starts_with("color.wheels.")
}

/// `v`, a current file, as format 10 wrote it: format 11 added `hsl`,
/// `curves` and `wheels` inside `color`, and keyframes of pucks, HSL and
/// wheel sliders. Only faithful for documents without them.
pub fn to_v10(v: &Value) -> Value {
    let mut v = to_v11(v);
    v["format_version"] = Value::from(10);
    for clip in clips_mut(&mut v) {
        if let Some(color) = clip.get("color") {
            for part in ["hsl", "curves", "wheels"] {
                assert!(color.get(part).is_none(), "format 10 had no {part}");
            }
        }
        if let Some(tracks) = clip.get("keys").and_then(|k| k["tracks"].as_array()) {
            for t in tracks {
                assert!(
                    !is_k2_key(t["prop"].as_str().unwrap()),
                    "format 10 had no K2 keys"
                );
            }
        }
    }
    v
}

/// The kitchen sink's file without what format 11 added: no `hsl`,
/// `curves` or `wheels` in `color`, no puck, HSL or wheel keyframes, and
/// no Curves switch.
pub fn strip_k2(v: &mut Value) {
    for clip in clips_mut(v) {
        if let Some(Value::Object(color)) = clip.get_mut("color") {
            for part in ["hsl", "curves", "wheels"] {
                color.remove(part);
            }
            if let Some(Value::Array(off)) = color.get_mut("off") {
                off.retain(|s| s != "curves");
                if off.is_empty() {
                    color.remove("off");
                }
            }
        }
        let keys = clip.get_mut("keys").and_then(|k| k.get_mut("tracks"));
        if let Some(Value::Array(tracks)) = keys {
            tracks.retain(|t| !is_k2_key(t["prop"].as_str().unwrap_or("")));
        }
    }
}

/// `v`, a current file, as format 9 wrote it: format 10 added clip `color`
/// and `color.*` keyframes. Only faithful for documents without colour.
pub fn to_v9(v: &Value) -> Value {
    let mut v = to_v10(v);
    v["format_version"] = Value::from(9);
    for clip in clips_mut(&mut v) {
        assert!(clip.get("color").is_none(), "format 9 had no colour");
        if let Some(tracks) = clip.get("keys").and_then(|k| k["tracks"].as_array()) {
            for t in tracks {
                let prop = t["prop"].as_str().unwrap();
                assert!(!prop.starts_with("color."), "format 9 had no colour keys");
            }
        }
    }
    v
}

/// The kitchen sink's file without its colour (format 10): no `color`
/// objects and no `color.*` keyframes.
pub fn strip_color(v: &mut Value) {
    for clip in clips_mut(v) {
        clip.remove("color");
        let keys = clip.get_mut("keys").and_then(|k| k.get_mut("tracks"));
        if let Some(Value::Array(tracks)) = keys {
            tracks.retain(|t| !t["prop"].as_str().unwrap_or("").starts_with("color."));
        }
        let empty = clip
            .get("keys")
            .is_some_and(|k| k["tracks"].as_array().is_some_and(Vec::is_empty) && k["origin"] == 0);
        if empty {
            clip.remove("keys");
        }
    }
}

/// `v`, a current file, as format 8 wrote it: without the library item
/// `origin` format 9 added. Only faithful for documents that format 8
/// could hold: imported items only.
pub fn to_v8(v: &Value) -> Value {
    let mut v = to_v9(v);
    v["format_version"] = Value::from(8);
    for item in v["document"]["library"].as_array_mut().unwrap() {
        assert!(
            item.get("origin").is_none(),
            "format 8 had only imported items"
        );
    }
    v
}

/// `v`, a current file, as format 7 wrote it: [`to_v8`], then without the
/// follow flags and clip attachments format 8 added. Only faithful for
/// documents whose sequences follow with every kind, which is what format
/// 7 files open with; their attachments are dropped, because format 7 had
/// none, so compare what it opens to with [`older_view`].
pub fn to_v7(v: &Value) -> Value {
    let mut v = to_v8(v);
    v["format_version"] = Value::from(7);
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        let flags = seq["flags"].as_object_mut().unwrap();
        assert_eq!(flags.remove("follow_main"), Some(Value::Bool(true)));
        let kinds = flags.remove("follow_kinds").unwrap();
        assert_eq!(kinds.as_array().unwrap().len(), FollowKind::ALL.len());
    }
    for clip in clips_mut(&mut v) {
        clip.remove("attach");
    }
    v
}

/// `v`, a current file, as format 6 wrote it: [`to_v7`], then without the
/// markers format 7 added. Only faithful for documents without markers.
pub fn to_v6(v: &Value) -> Value {
    let mut v = to_v7(v);
    v["format_version"] = Value::from(6);
    for seq in v["document"]["sequences"].as_array().unwrap() {
        assert!(seq.get("markers").is_none(), "format 6 had no markers");
    }
    for clip in clips_mut(&mut v) {
        assert!(clip.get("markers").is_none(), "format 6 had no markers");
    }
    v
}

/// `v`, a current file, as format 5 wrote it: [`to_v6`], then without the
/// clip `keys` and `animations` format 6 added. Only faithful for
/// documents that format 5 could hold: no keys or animations, volume keys
/// linear or held.
pub fn to_v5(v: &Value) -> Value {
    let mut v = to_v6(v);
    v["format_version"] = Value::from(5);
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                for clip in track["clips"].as_array_mut().unwrap() {
                    let clip = clip.as_object_mut().unwrap();
                    clip.remove("keys");
                    clip.remove("animations");
                    if let Some(keys) = clip["audio"].get("gain_keys") {
                        for key in keys.as_array().unwrap() {
                            assert!(
                                key["interp"] == "linear" || key["interp"] == "hold",
                                "format 5 had only linear and held volume keys"
                            );
                        }
                    }
                }
            }
        }
    }
    v
}

/// `v`, a current file, as format 4 wrote it: text fills as colour
/// strings, without the keys format 5 added (decoration, case, letter
/// spacing, layers, justify, vertical anchor). Only faithful for documents
/// that format 4 could hold: solid fills and factory values for the rest.
pub fn to_v4(v: &Value) -> Value {
    let mut v = to_v5(v);
    v["format_version"] = Value::from(4);
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                for clip in track["clips"].as_array_mut().unwrap() {
                    if clip["content"]["type"] != "text" {
                        continue;
                    }
                    let content = &mut clip["content"];
                    let color = content["style"]["fill"]["color"].clone();
                    assert!(color.is_string(), "format 4 had only solid fills");
                    let style = content["style"].as_object_mut().unwrap();
                    style.insert("fill".into(), color);
                    for key in [
                        "underline",
                        "strike",
                        "case",
                        "letter_spacing",
                        "outline",
                        "shadow",
                        "glow",
                        "background",
                        "bubble",
                        "curve",
                    ] {
                        style.remove(key);
                    }
                    let layout = content["layout"].as_object_mut().unwrap();
                    layout.remove("justify");
                    layout.remove("v_anchor");
                }
            }
        }
    }
    v
}

/// `v`, a current file, as format 3 wrote it: [`to_v4`], then without the
/// keys format 4 added (`ui`, track `mix`, media clip `stream`, volume `gain_keys`, fade
/// curves, audio stream tags). Only faithful for documents that format 3
/// could hold: no UI state, default track stages and stream choices, no
/// keys, linear fades, untagged streams.
pub fn to_v3(v: &Value) -> Value {
    let mut v = to_v4(v);
    v["format_version"] = Value::from(3);
    v.as_object_mut().unwrap().remove("ui");
    for item in v["document"]["library"].as_array_mut().unwrap() {
        for stream in item["info"]["audio"].as_array_mut().unwrap() {
            let stream = stream.as_object_mut().unwrap();
            for key in ["default", "language", "title", "layout"] {
                stream.remove(key);
            }
        }
    }
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                track.as_object_mut().unwrap().remove("mix");
                for clip in track["clips"].as_array_mut().unwrap() {
                    if clip["content"]["type"] == "media" {
                        clip["content"].as_object_mut().unwrap().remove("stream");
                    }
                    if clip["audio"]["type"] == "active" {
                        let audio = clip["audio"].as_object_mut().unwrap();
                        for key in ["gain_keys", "fade_in_curve", "fade_out_curve"] {
                            audio.remove(key);
                        }
                    }
                }
            }
        }
    }
    v
}

/// `v`, a current file, as format 2 wrote it: [`to_v3`], then text clips as
/// generated text, no transforms, no export settings. Only faithful for
/// documents that format 2 could hold: factory-default text and identity
/// transforms.
pub fn to_v2(v: &Value) -> Value {
    let mut v = to_v3(v);
    v["format_version"] = Value::from(2);
    v.as_object_mut().unwrap().remove("export");
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                for clip in track["clips"].as_array_mut().unwrap() {
                    let clip = clip.as_object_mut().unwrap();
                    clip.remove("transform");
                    if clip["content"]["type"] == "text" {
                        clip.insert(
                            "content".into(),
                            serde_json::json!({"type": "generated", "kind": "text"}),
                        );
                    }
                }
            }
        }
    }
    v
}

/// `v`, a current file, as format 1 wrote it: [`to_v2`], then without the
/// keys format 2 added (`source` on library items, `audio` on clips). Only
/// faithful for documents that format 1 could hold: file items, and the
/// sound a new clip of each content gets.
pub fn to_v1(v: &Value) -> Value {
    let mut v = to_v2(v);
    v["format_version"] = Value::from(1);
    let document = &mut v["document"];
    for item in document["library"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("source");
    }
    for seq in document["sequences"].as_array_mut().unwrap() {
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                for clip in track["clips"].as_array_mut().unwrap() {
                    clip.as_object_mut().unwrap().remove("audio");
                }
            }
        }
    }
    v
}

pub fn to_value(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

/// Serializes `value` the way a foreign tool might (compact), so the reader
/// can't depend on our formatting.
pub fn compact(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

/// Every number in `value` that is not an integer.
pub fn floats(value: &Value, path: &str, out: &mut Vec<String>) {
    match value {
        Value::Number(n) if n.is_f64() => out.push(path.to_string()),
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                floats(v, &format!("{path}[{i}]"), out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                floats(v, &format!("{path}.{k}"), out);
            }
        }
        _ => {}
    }
}

/// What a saved file holds that format 7 and older could hold too: the file
/// [`without_origins`], and without clip attachments (format 8), which an
/// older file opens without.
pub fn older_view(bytes: &[u8]) -> Value {
    let mut v = without_origins(bytes);
    for clip in clips_mut(&mut v) {
        clip.remove("attach");
    }
    v
}

/// A saved file without the `keys` objects of clips that have no keys,
/// only a content-time origin (format 6). An origin without keys changes
/// nothing a user sees: keys added later are placed relative to it, so a
/// clip opened from an older format, with origin 0, behaves the same.
pub fn without_origins(bytes: &[u8]) -> Value {
    let mut v = to_value(bytes);
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                for clip in track["clips"].as_array_mut().unwrap() {
                    let clip = clip.as_object_mut().unwrap();
                    if clip
                        .get("keys")
                        .is_some_and(|k| k["tracks"].as_array().is_some_and(Vec::is_empty))
                    {
                        clip.remove("keys");
                    }
                }
            }
        }
    }
    v
}
