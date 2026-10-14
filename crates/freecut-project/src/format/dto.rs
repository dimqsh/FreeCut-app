//! The on-disk schema of the current format version, as private serde types.
//!
//! These DTOs are the only definition of the file format: the timeline model
//! stays serde-free (README C4), and a model refactor can't silently change
//! the format because every field is mapped by hand in [`super::map`].
//!
//! Rules every DTO follows:
//! - Field order is the key order in the file (the canonical writer keeps
//!   it), so related keys stay together and diffs stay small.
//! - Every object ends with a flattened `extra` map that catches keys this
//!   build doesn't know; they are written back after the known keys.
//! - Times are `i64` ticks, ids are non-zero `u64`s, enums are kebab-case
//!   strings. There are no float fields anywhere.
//! - Optional values are written as `null`, never omitted, so each key has
//!   exactly one meaning. The exception is a section whose absence means
//!   its defaults in every format version (a clip `transform`, a track
//!   `mix`, the text style layers): it is left out while it holds them.
//! - A value with variants is an object that names its variant in a `"type"`
//!   key, followed by the variant's fields ([`Tagged`]).

use core::fmt;
use core::num::NonZeroU64;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::extras::Fields;

/// The top-level object.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EnvelopeDto {
    /// Always [`super::FORMAT_MAGIC`]; checked before typed decoding.
    pub format: String,
    /// Always the current version after migration.
    pub format_version: u32,
    /// The writer, `"freecut <version>"`.
    pub app: String,
    /// The editor's id allocator position: every id in the file is below it.
    pub ids_next: NonZeroU64,
    pub document: DocumentDto,
    /// The settings of the last export started from this project (EX-021),
    /// `null` before the first one.
    pub export: Option<ExportDto>,
    /// Per-project UI state that is saved with the project but is not part
    /// of the document or its undo history (AU-012's solo set). Left out
    /// while empty, which is what a missing key means in every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The project's UI state.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UiDto {
    /// Soloed tracks (AU-012), by id, ascending.
    pub solo: Vec<NonZeroU64>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DocumentDto {
    /// The root sequence.
    pub root: NonZeroU64,
    pub settings: SettingsDto,
    /// Media items, by id.
    pub library: Vec<MediaItemDto>,
    /// Sequences, by id.
    pub sequences: Vec<SequenceDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SettingsDto {
    pub canvas: SizeDto,
    pub frame_rate: RateDto,
    /// Hz.
    pub sample_rate: u32,
    pub match_first_clip: bool,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A pixel size.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SizeDto {
    pub width: u32,
    pub height: u32,
    #[serde(flatten)]
    pub extra: Fields,
}

/// An unsigned ratio: frame rates and sample aspect ratios.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RateDto {
    pub num: u32,
    pub den: u32,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A signed exact ratio (`Rational`).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RationalDto {
    pub num: i64,
    pub den: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A half-open tick range.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RangeDto {
    pub start: i64,
    pub duration: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MediaItemDto {
    pub id: NonZeroU64,
    /// What the item reads: its own file, or one audio stream of another
    /// item's file.
    pub source: Tagged<SourceDto>,
    /// Format 9: where the item came from (PM-041), `"type": "imported"`
    /// or `"generated"`. Left out while it is imported, which is what a
    /// missing key means in every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Tagged<MediaOriginDto>>,
    pub path: StoredPathDto,
    pub info: MediaInfoDto,
    pub fingerprint: Option<FingerprintDto>,
    pub display_name: Option<String>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StoredPathDto {
    pub absolute: String,
    /// `/`-separated, from the project file's folder.
    pub relative: Option<String>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FingerprintDto {
    pub size: u64,
    /// 16 lowercase hex digits: a 64-bit hash doesn't fit a JSON number
    /// exactly.
    pub head_tail: String,
    pub duration: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MediaInfoDto {
    pub container: String,
    /// Container index of the stream that defines media time 0.
    pub default_video: Option<u32>,
    pub duration: i64,
    pub video: Vec<VideoStreamDto>,
    pub audio: Vec<AudioStreamDto>,
    /// Pixel size of a still image.
    pub still: Option<SizeDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VideoStreamDto {
    pub index: u32,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub sar: RateDto,
    pub rotation: RotationDto,
    pub mirrored: bool,
    pub rate: Option<RationalDto>,
    pub variable_rate: bool,
    pub has_alpha: bool,
    pub bit_depth: u8,
    pub start: i64,
    pub duration: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AudioStreamDto {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub start: i64,
    pub duration: i64,
    pub duration_exact: bool,
    /// The file flags this stream as the one to play by default (AU-038).
    pub default: bool,
    /// Language tag as the file stores it (`"eng"`).
    pub language: Option<String>,
    pub title: Option<String>,
    /// Channel layout name as the probe reports it (`"stereo"`).
    pub layout: Option<String>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RotationDto {
    None,
    Cw90,
    Cw180,
    Cw270,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SequenceDto {
    pub id: NonZeroU64,
    pub flags: SequenceFlagsDto,
    /// The master stage (AU-010). Left out while it holds the defaults, so
    /// files that never changed it are written exactly as before, and a
    /// missing key reads as the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master: Option<MasterAudioDto>,
    /// Format 7: the timeline markers (TL-037), by start then id. Left out
    /// while there are none, which is what a missing key means in every
    /// version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MarkerDto>,
    /// Bottom to top; the first one is the main track (README C31: position
    /// decides, there is no `main` key).
    pub visual: Vec<TrackDto>,
    /// Top to bottom.
    pub audio: Vec<TrackDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A sequence's master stage (AU-010).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MasterAudioDto {
    /// Master volume, spelled like a clip gain; at most +12 dB.
    pub gain: GainDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SequenceFlagsDto {
    pub main_magnet: bool,
    /// Format 8: *Follow main track* (TL-029). Always written; a missing
    /// key (every older file) reads as on, the model's default.
    #[serde(default = "yes")]
    pub follow_main: bool,
    /// Format 8: the item kinds that follow, each listed once in the
    /// model's kind order. Always written; a missing key (every older file)
    /// reads as every kind.
    #[serde(default = "every_follow_kind")]
    pub follow_kinds: Vec<FollowKindDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

fn yes() -> bool {
    true
}

fn every_follow_kind() -> Vec<FollowKindDto> {
    FollowKindDto::ALL.to_vec()
}

/// An item kind of *Follow main track* (TL-029's *Item kinds…*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FollowKindDto {
    VideoOverlay,
    Text,
    Sticker,
    Effect,
    Adjustment,
    Music,
    SoundEffect,
    Speech,
    OtherAudio,
}

impl FollowKindDto {
    pub const ALL: [FollowKindDto; 9] = [
        FollowKindDto::VideoOverlay,
        FollowKindDto::Text,
        FollowKindDto::Sticker,
        FollowKindDto::Effect,
        FollowKindDto::Adjustment,
        FollowKindDto::Music,
        FollowKindDto::SoundEffect,
        FollowKindDto::Speech,
        FollowKindDto::OtherAudio,
    ];
}

/// A timeline or clip marker (format 7, TL-037).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MarkerDto {
    pub id: NonZeroU64,
    /// Sequence time for a timeline marker, the clip's content time for a
    /// clip marker (source time for media). A zero duration is a point.
    pub at: RangeDto,
    /// Possibly empty.
    pub name: String,
    /// Possibly empty; may hold line breaks.
    pub note: String,
    pub color: PaletteColorDto,
    pub origin: MarkerOriginDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A slot of FreeCut's 8-colour palette (TL-049), shared by markers,
/// track colours and clip labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PaletteColorDto {
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
}

/// Where a marker came from. Beats are clip markers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MarkerOriginDto {
    /// Placed by the user.
    User,
    /// A beat tapped or placed by hand (AU-025).
    Beat,
    /// A beat found by beat detection (AU-025).
    DetectedBeat,
    /// A detected scene change.
    Scene,
}

/// The main-track clip an item follows (format 8, TL-029).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AttachmentDto {
    /// A clip on the sequence's main track.
    pub owner: NonZeroU64,
    /// The owner's content frame the item starts on, in ticks of its
    /// content time (source time for media).
    pub owner_source_time: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TrackDto {
    pub id: NonZeroU64,
    pub kind: TrackKindDto,
    pub header: TrackHeaderDto,
    /// The track stage (AU-013). Left out while it holds the defaults
    /// (0 dB, centre), which is what a missing key means in every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix: Option<TrackMixDto>,
    /// By start, then id.
    pub clips: Vec<ClipDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A track's volume and pan (AU-013).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TrackMixDto {
    /// Spelled like a clip gain; at most +12 dB.
    pub gain: GainDto,
    /// Balance in percent, −100 (left) to +100 (right).
    pub pan: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TrackKindDto {
    Video,
    Text,
    Sticker,
    Effect,
    Adjustment,
    Audio,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TrackHeaderDto {
    pub name: Option<String>,
    pub muted: bool,
    pub hidden: bool,
    pub locked: bool,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClipDto {
    pub id: NonZeroU64,
    /// Placement in sequence time.
    pub range: RangeDto,
    pub content: Tagged<ContentDto>,
    pub enabled: bool,
    pub name: Option<String>,
    pub group: Option<NonZeroU64>,
    pub link: Option<NonZeroU64>,
    /// The clip's sound (AU-002, TL-030).
    pub audio: Tagged<AudioDto>,
    /// Placement on the canvas (TR-001, TR-003, TR-015). Left out while it
    /// is the identity, which is what a missing key means in every format
    /// version (audio clips and untouched clips never write it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformDto>,
    /// Format 6: keyframes on transform and text properties (TR-027,
    /// TX-021). Left out while the clip has none and its content-time
    /// origin is 0, which is what a missing key means in every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<ClipKeysDto>,
    /// Format 6: the In, Out and Loop animations (FX-018, FX-019, TX-022).
    /// Left out while every slot is empty, which is what a missing key
    /// means in every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animations: Option<ClipAnimationsDto>,
    /// Format 7: the clip's markers (TL-037), by start then id. Left out
    /// while there are none, which is what a missing key means in every
    /// version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MarkerDto>,
    /// Format 8: the main-track clip this item follows (TL-029). Left out
    /// while it follows none, which is what a missing key means in every
    /// version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach: Option<AttachmentDto>,
    /// Format 10: the colour stack's static values (CL-001) on video,
    /// image and adjustment clips. Left out while it is the default stack,
    /// which is what a missing key means in every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A clip's colour stack (format 10, CL-001, CL-007, CL-008, CL-023,
/// CL-035; format 11 adds HSL, Curves and Wheels, CL-013 to CL-015; format
/// 12 the LUT slot and skin protection, CL-017, CL-030). Every part at its
/// default is left out: Amount 100, no bypass, every section on, sliders at
/// 0, curves on the diagonal, pucks centred, no LUT, skin protection off at
/// Amount 70.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ColorDto {
    /// The master bypass (CL-023); absent is off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
    /// The overall Amount, 0–100 %; absent is 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    /// The sections switched off (CL-023), by name (`light`, `color`, …),
    /// in display order; absent is none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off: Vec<String>,
    /// The Light sliders (CL-007) that aren't 0, −100 to +100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<LightDto>,
    /// The Color sliders (CL-008) that aren't 0, −100 to +100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorBalanceDto>,
    /// Format 11: the HSL bands (CL-013) that aren't at their defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hsl: Option<HslDto>,
    /// Format 11: the curves (CL-014) that aren't the plain diagonal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curves: Option<CurvesDto>,
    /// Format 11: the colour wheels (CL-015) that aren't at their defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheels: Option<WheelsDto>,
    /// Format 12: the user LUT (CL-017); absent is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<LutDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The LUT slot (format 12, CL-017): the file by the XXH3-128 hash of its
/// bytes (32 hex digits) and its original name, then its settings, each
/// left out at its default (Intensity 100, tetrahedral, no skin
/// protection).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LutDto {
    pub hash: String,
    pub file: String,
    /// 0–100 %; absent is 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<i64>,
    /// `tetrahedral` or `trilinear`; absent is tetrahedral.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interp: Option<String>,
    /// Skin protection of the LUT (CL-030); absent is off at Amount 70.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<SkinDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// Skin protection of one scope (format 12, CL-030).
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct SkinDto {
    /// The switch; absent is off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on: bool,
    /// 0–100 %; absent is 70.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The HSL section (format 11, CL-013): each band left out at its
/// defaults.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct HslDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orange: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yellow: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub green: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyan: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blue: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purple: Option<HslBandDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magenta: Option<HslBandDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// One HSL band's levels, −100 to +100; each left out at 0.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct HslBandDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lightness: Option<i64>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The curves (format 11, CL-014): each a list of `[x, y]` points in
/// `0..=10000`, left to right, endpoints included; left out while it is
/// the diagonal from `[0, 0]` to `[10000, 10000]`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct CurvesDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luma: Option<Vec<[i64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<Vec<[i64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub green: Option<Vec<[i64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blue: Option<Vec<[i64; 2]>>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The colour wheels (format 11, CL-015): Strength (absent is 100) and
/// each wheel left out at its defaults.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct WheelsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<WheelDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midtones: Option<WheelDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlights: Option<WheelDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<WheelDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// One wheel: the puck (`x`, `y` on a disk of radius 1000) and the
/// luminance level (−100 to +100); each left out at 0.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct WheelDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luma: Option<i64>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The Light section's sliders; each left out at 0.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct LightDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast_pivot: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlights: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whites: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blacks: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone_lift: Option<i64>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The Color section's sliders; each left out at 0.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct ColorBalanceDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrance: Option<i64>,
    /// Format 12: skin protection of the Color section (CL-030); absent is
    /// off at Amount 70.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<SkinDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A clip's keyframes (format 6, TR-027, TX-021).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClipKeysDto {
    /// Content time of the clip's start, in ticks, for clips without a
    /// source range (images, text, generated items): keys stick to the
    /// content through splits and head trims.
    pub origin: i64,
    /// The keyed properties, each listed once, in the model's property
    /// order.
    pub tracks: Vec<KeyTrackDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The keys of one property. Unknown keys are filed by the property's path.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct KeyTrackDto {
    /// The property's stable path: `transform.position`, `text.size`,
    /// `text.fill.stops.1.color`, `text.reveal`, …
    pub prop: String,
    /// At least one, strictly increasing in time.
    pub keys: Vec<PropKeyDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// One keyframe of a clip property. Unknown keys are filed by the key's
/// time, like volume keyframes.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PropKeyDto {
    /// Content time of the clip in ticks: source time for media, clip
    /// time plus the origin otherwise (TR-027).
    pub at: i64,
    /// Shaped by the property's kind: an integer in the property's
    /// fixed-point unit (as the static value is stored), a `"#RRGGBBAA"`
    /// colour, or an `{"x", "y"}` point for positions and tail tips.
    pub value: KeyValueDto,
    /// How the segment that starts here reaches the next key (TR-031).
    pub interp: InterpDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A keyframe value. The reader keeps it as read ([`KeyValueDto::Unparsed`])
/// and [`super::map`] decodes it by its property's kind, so errors keep
/// their full path; the writer always writes a decoded value.
#[derive(Debug)]
pub(crate) enum KeyValueDto {
    /// Fixed-point numbers: scale, angles, ratios, lengths.
    Int(i64),
    /// `"#RRGGBBAA"`.
    Color(String),
    /// Positions (millionths of the canvas) and tail tips (hundredths of a
    /// reference pixel).
    Point(PointDto),
    Unparsed(Value),
}

impl Serialize for KeyValueDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            KeyValueDto::Int(v) => serializer.serialize_i64(*v),
            KeyValueDto::Color(c) => serializer.serialize_str(c),
            KeyValueDto::Point(p) => p.serialize(serializer),
            KeyValueDto::Unparsed(raw) => raw.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for KeyValueDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(KeyValueDto::Unparsed)
    }
}

/// A clip's animation slots (format 6, FX-018, FX-019, TX-022): `null`
/// for an empty slot.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClipAnimationsDto {
    pub entrance: Option<AnimationSpecDto>,
    pub exit: Option<AnimationSpecDto>,
    pub continuous: Option<AnimationSpecDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// One slot's animation, with its settings (FX-018, FX-030, TX-023).
/// Numbers are read wide, as in [`TransformDto`].
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AnimationSpecDto {
    /// The preset id, `fc.in.fade`; a preset this build doesn't have plays
    /// as no animation and is kept (FX-018's **Unavailable animation**).
    pub preset: String,
    /// Ticks: the slot length (In, Out) or the cycle (Loop), as set.
    pub duration: i64,
    /// The easing override; `null` keeps the preset's curve.
    pub easing: Option<InterpDto>,
    /// `null` takes the preset's default direction.
    pub direction: Option<DirectionDto>,
    /// The pieces that move (TX-023); `null` takes the preset's default.
    pub unit: Option<AnimUnitDto>,
    /// Per-mille, 0 (one after another) to 1000 (all together).
    pub overlap: i64,
    pub order: AnimOrderDto,
    /// Seed of random orders and seeded motions (0–4294967295; wider
    /// numbers are saturated).
    pub seed: i64,
    /// Ticks per unit (TX-022's Pace); `null` when the length is set
    /// directly.
    pub pace: Option<i64>,
    /// Per-mille of FX-030's 0–100 Intensity: 500 is the default.
    pub intensity: i64,
    pub travel: TravelDto,
    /// Ticks: a Loop's phase offset (FX-019), so a split continues it.
    pub phase: i64,
    /// Typewriter's **Caret** (TX-056); written only when on, and absent
    /// reads as off, so files without it need no migration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub caret: bool,
    /// A Loop's seams from before a split (TX-022); written only after a
    /// split, and absent reads as none, so files without it need no
    /// migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seams: Option<LoopSeamsDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// Where a split Loop eases in and out (TX-022), in ticks of loop time
/// (clip-local time plus the phase).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LoopSeamsDto {
    /// In's end; `null` without an In.
    pub start: Option<i64>,
    /// Out's start; `null` without an Out.
    pub end: Option<i64>,
    /// Ramp length, 0–250 ms.
    pub ramp: i64,
}

/// The side an animation comes from or goes to, or a spin's sense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DirectionDto {
    Left,
    Right,
    Up,
    Down,
    Clockwise,
    CounterClockwise,
}

/// The pieces an animation moves (TX-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnimUnitDto {
    Block,
    Line,
    Word,
    Letter,
}

/// The order pieces start in (TX-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnimOrderDto {
    Forward,
    Backward,
    CenterOut,
    EdgesIn,
    Random,
}

/// Where a slide starts (FX-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TravelDto {
    FromEdge,
    Short,
}

/// A clip transform (TR-001, TR-003, TR-015) in fixed point. Numbers are
/// read wider than the model stores them, so a hand-edited value beyond the
/// model's integer width is clamped like any other out-of-range value
/// instead of failing the file.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TransformDto {
    /// Clip centre relative to the canvas centre.
    pub position: PositionDto,
    /// Basis points of the fitted size: 10000 is 100 %.
    pub scale: i64,
    /// Hundredths of a degree, clockwise, not wrapped.
    pub rotation: i64,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Per-mille.
    pub opacity: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A canvas position in millionths of the canvas width (`x`, +right) and
/// height (`y`, +down) from the centre.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PositionDto {
    pub x: i64,
    pub y: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// An object tagged by `"type"`, with the variant's fields after the tag.
///
/// Serde reads an internally tagged enum through a buffer that hides the
/// JSON path of any error inside it. So the reader keeps the object as it is
/// ([`Tagged::Unparsed`]) and [`super::map`] decodes it by its tag into one
/// of the variant structs, where errors keep their full path
/// (`…clips[0].content.source.start`). The writer always writes a decoded
/// variant, `T`, which serializes itself with the tag first.
#[derive(Debug)]
pub(crate) enum Tagged<T> {
    Decoded(T),
    /// The object as read, before decoding by its tag.
    Unparsed(Value),
}

impl<T: Serialize> Serialize for Tagged<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Tagged::Decoded(variant) => variant.serialize(serializer),
            Tagged::Unparsed(raw) => raw.serialize(serializer),
        }
    }
}

impl<'de, T> Deserialize<'de> for Tagged<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(Tagged::Unparsed)
    }
}

/// What a clip shows: `"type": "media"`, `"image"`, `"text"` or
/// `"generated"`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum ContentDto {
    Media(MediaContentDto),
    Image(ImageContentDto),
    /// Boxed: a styled text is far larger than the other variants.
    Text(Box<TextContentDto>),
    Generated(GeneratedContentDto),
}

/// `"type": "media"`: a span of a media item's video or audio.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MediaContentDto {
    pub asset: NonZeroU64,
    pub kind: MediaKindDto,
    /// In source time.
    pub source: RangeDto,
    /// The audio stream the clip plays (AU-038): `null` for the media's
    /// default stream, else `"type": "index"` or `"all-mixed"`.
    pub stream: Option<Tagged<StreamSelDto>>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A chosen audio stream: `"type": "index"` or `"all-mixed"`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum StreamSelDto {
    Index(StreamIndexDto),
    AllMixed(PlainDto),
}

/// `"type": "index"`: the audio stream with this container index. It may
/// be missing from the media after a relink; the default stream plays then.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StreamIndexDto {
    pub index: u32,
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "image"`: a still image.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ImageContentDto {
    pub asset: NonZeroU64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "text"`: a text clip (TX-001). Lengths are hundredths of a
/// reference pixel (a pixel of a canvas whose short edge is 1080 px), ratios
/// per-mille (TX conventions).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TextContentDto {
    /// Exactly as typed (TX-002).
    pub text: String,
    pub style: TextStyleDto,
    pub layout: TextLayoutDto,
    pub origin: TextOriginDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// Character style of a whole text clip (TX-003, TX-004, TX-008 to
/// TX-016, TX-054, TX-055). Numbers are read wide, as in [`TransformDto`].
///
/// The keys after `italic` came with format 5. Each reads as the factory
/// default when it is missing, which is what every older file means: they
/// had no decoration, case, spacing or layers. The scalars are always
/// written; the optional layers (`outline` to `curve`) are left out while
/// they hold the factory default (off, with default settings), so a plain
/// title stays short. A layer that is off but was changed is written with
/// `"on": false`, keeping its settings (TX-054).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TextStyleDto {
    pub font: FontDto,
    /// Hundredths of a reference pixel.
    pub size: i64,
    /// `"type": "solid"`, `"linear-gradient"` or `"radial-gradient"`.
    pub fill: Tagged<FillDto>,
    pub bold: bool,
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub strike: bool,
    /// The display case (TX-008); the stored text stays as typed.
    #[serde(default)]
    pub case: CaseDto,
    /// Per-mille of the size, added after each grapheme cluster (TX-009).
    #[serde(default)]
    pub letter_spacing: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<OutlineDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<GlowDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bubble: Option<BubbleDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CurveDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A font by family name (TX-003); face details come later.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FontDto {
    pub family: String,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CaseDto {
    #[default]
    AsTyped,
    Upper,
    Lower,
    Title,
    Sentence,
}

/// How glyphs are filled (TX-004, TX-014).
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum FillDto {
    Solid(SolidFillDto),
    LinearGradient(LinearFillDto),
    RadialGradient(RadialFillDto),
}

/// `"type": "solid"`: one colour.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SolidFillDto {
    /// `"#RRGGBBAA"`, straight alpha.
    pub color: String,
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "linear-gradient"`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LinearFillDto {
    /// Tenths of a degree, `0..3600`: 0 runs left to right, 900 top to
    /// bottom.
    pub angle: i64,
    /// 2–8 stops by position.
    pub stops: Vec<GradientStopDto>,
    pub extent: FillExtentDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "radial-gradient"`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RadialFillDto {
    /// Per-mille of the extent's bounds.
    pub center: PointDto,
    /// The radius at the last stop: per-mille of half the extent's diagonal.
    pub spread: i64,
    /// 2–8 stops by position.
    pub stops: Vec<GradientStopDto>,
    pub extent: FillExtentDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// One gradient stop. Its unknown keys are filed by its index once the
/// stops are sorted by position, the order they are written in.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GradientStopDto {
    /// Per-mille along the gradient, 0–1000.
    pub at: i64,
    pub color: String,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FillExtentDto {
    Block,
    Line,
}

/// Two integers; their unit is given where the point is used.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PointDto {
    pub x: i64,
    pub y: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The outline layer (TX-010). Lengths are per-mille of the font size.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OutlineDto {
    pub on: bool,
    pub color: String,
    pub width: i64,
    /// Per-mille.
    pub opacity: i64,
    pub position: StrokePositionDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StrokePositionDto {
    Outside,
    Center,
    Inside,
}

/// The drop shadow layer (TX-011). Lengths are per-mille of the font size.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ShadowDto {
    pub on: bool,
    pub color: String,
    /// Per-mille.
    pub opacity: i64,
    pub blur: i64,
    pub distance: i64,
    /// Tenths of a degree clockwise from the right, `0..3600`.
    pub angle: i64,
    pub include_box: bool,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The glow layer (TX-012). Lengths are per-mille of the font size.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GlowDto {
    pub on: bool,
    pub color: String,
    /// Per-mille.
    pub intensity: i64,
    pub radius: i64,
    pub offset: i64,
    /// Tenths of a degree clockwise from the right, `0..3600`.
    pub angle: i64,
    pub direction: GlowDirectionDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GlowDirectionDto {
    Outward,
    Inward,
    Both,
}

/// The background box layer (TX-013). Padding is per-mille of the font
/// size; opacity and roundness are per-mille.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BackgroundDto {
    pub on: bool,
    pub color: String,
    pub opacity: i64,
    pub shape: BoxShapeDto,
    pub pad_x: i64,
    pub pad_y: i64,
    pub roundness: i64,
    pub height: BoxHeightDto,
    pub border: BorderDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BoxShapeDto {
    Block,
    PerLine,
    PerChar,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BoxHeightDto {
    Full,
    LowerHalf,
}

/// The border of a box or bubble, a layer of its own (TX-013, TX-055).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BorderDto {
    pub on: bool,
    pub color: String,
    /// Per-mille of the font size.
    pub width: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The bubble layer (TX-055). Padding is per-mille of the font size;
/// opacity, roundness and the shape values are per-mille.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BubbleDto {
    pub on: bool,
    /// `"type"`: `"rounded"`, `"oval"`, `"thought"`, `"shout"`, `"square"`,
    /// `"tag"`, `"arrow-tag"` (with `point`) or `"ticket"`.
    pub kind: Tagged<BubbleKindDto>,
    /// The tail, or Thought's trail of circles.
    pub tail: TailDto,
    /// The bubble's fill colour.
    pub color: String,
    pub opacity: i64,
    pub pad_x: i64,
    pub pad_y: i64,
    pub roundness: i64,
    pub border: BorderDto,
    /// Shout: the number of spikes.
    pub spikes: i64,
    /// Shout: per-mille of the body's shorter side.
    pub spike_depth: i64,
    /// Shout: irregularity.
    pub jitter: i64,
    /// Shout: the seed of the irregularity, `0..2^32`.
    pub seed: i64,
    /// Thought: per-mille of the body's shorter side.
    pub lobe: i64,
    /// Thought: the number of trail circles.
    pub trail: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum BubbleKindDto {
    Rounded(PlainDto),
    Oval(PlainDto),
    Thought(PlainDto),
    Shout(PlainDto),
    Square(PlainDto),
    Tag(PlainDto),
    ArrowTag(ArrowTagDto),
    Ticket(PlainDto),
}

/// `"type": "arrow-tag"`: which end points.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ArrowTagDto {
    pub point: SideDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SideDto {
    Left,
    Right,
}

/// A bubble's tail, a layer of its own (TX-055).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TailDto {
    pub on: bool,
    /// Hundredths of a reference pixel from the text box centre, +x right,
    /// +y down.
    pub tip: PointDto,
    /// Width at the base, per-mille of the body's shorter side.
    pub base: i64,
    /// −1000 to 1000.
    pub bend: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// The curve layer (TX-016).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CurveDto {
    pub on: bool,
    /// −1000 to 1000, the UI's −100 to +100.
    pub amount: i64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// Paragraph layout of a whole text clip (TX-005, TX-009). Numbers are read
/// wide, as in [`TransformDto`].
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TextLayoutDto {
    pub align: HAlignDto,
    /// Format 5 (TX-005): justified lines, which win over `align` (kept for
    /// when justification is switched off). Missing reads as `false`.
    #[serde(default)]
    pub justify: bool,
    /// Hundredths of a reference pixel; `null` is auto width.
    pub box_width: Option<i64>,
    /// Per-mille of the font's natural line height.
    pub line_spacing: i64,
    /// Format 5 (TX-005): the edge of the box that stays put. Missing reads
    /// as `"middle"`.
    #[serde(default)]
    pub v_anchor: VAnchorDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VAnchorDto {
    Top,
    #[default]
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HAlignDto {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TextOriginDto {
    Manual,
}

/// `"type": "generated"`: a sticker, an effect or an adjustment.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GeneratedContentDto {
    pub kind: GeneratedKindDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MediaKindDto {
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GeneratedKindDto {
    Sticker,
    Effect,
    Adjustment,
}

/// A clip's sound: `"type": "none"`, `"active"` or `"detached"`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum AudioDto {
    None(NoAudioDto),
    Active(ActiveAudioDto),
    Detached(DetachedAudioDto),
}

/// `"type": "none"`: the clip has no sound (images, generated items, video
/// without an audio stream).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct NoAudioDto {
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "active"`: the clip plays its own sound with these settings.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ActiveAudioDto {
    /// The static volume; with keys, kept for when the last key goes.
    pub gain: GainDto,
    /// Volume keyframes (AU-008) in source time, strictly increasing.
    pub gain_keys: Vec<GainKeyDto>,
    pub muted: bool,
    /// Fade lengths in ticks, stored as set: they may be longer than the
    /// clip (AU-006).
    pub fade_in: i64,
    pub fade_in_curve: FadeCurveDto,
    pub fade_out: i64,
    pub fade_out_curve: FadeCurveDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// One volume keyframe (AU-008, TR-027).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GainKeyDto {
    /// Source time of the clip, in ticks.
    pub at: i64,
    pub gain: GainDto,
    /// How the segment that starts here reaches the next key.
    pub interp: InterpDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// How a keyframe segment reaches the next key (TR-031). Format 4 had only
/// `linear` and `hold`; format 6 added the easing presets, spelled
/// `ease-<in|out|in-out>-<sine|quad|cubic|expo>`, and the four shaped
/// curves. Preset manifests use the same names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum InterpDto {
    Linear,
    Hold,
    EaseInSine,
    EaseOutSine,
    EaseInOutSine,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInExpo,
    EaseOutExpo,
    EaseInOutExpo,
    OvershootOut,
    OvershootIn,
    ElasticOut,
    BounceOut,
}

/// A fade shape (AU-006).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FadeCurveDto {
    Linear,
    Smooth,
    Fast,
    Slow,
}

/// `"type": "detached"`: the sound was moved to the audio clips carrying
/// `link` (TL-030). The video clip's own `link` may be `null` by now (the
/// audio clips were deleted); restoring gives the sound back.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DetachedAudioDto {
    pub link: NonZeroU64,
    #[serde(flatten)]
    pub extra: Fields,
}

/// A clip gain: an integer in hundredths of a decibel (`-350` is −3.5 dB),
/// or the string [`GAIN_SILENT`] for −∞ dB, which no JSON number can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GainDto {
    Hundredths(i64),
    Silent,
}

/// How the file spells a gain of −∞ dB (silence).
pub(crate) const GAIN_SILENT: &str = "-inf";

impl Serialize for GainDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            GainDto::Hundredths(h) => serializer.serialize_i64(*h),
            GainDto::Silent => serializer.serialize_str(GAIN_SILENT),
        }
    }
}

impl<'de> Deserialize<'de> for GainDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct GainVisitor;

        impl Visitor<'_> for GainVisitor {
            type Value = GainDto;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "a gain in hundredths of a decibel (an integer) or \"{GAIN_SILENT}\""
                )
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<GainDto, E> {
                Ok(GainDto::Hundredths(v))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<GainDto, E> {
                i64::try_from(v)
                    .map(GainDto::Hundredths)
                    .map_err(|_| E::invalid_value(de::Unexpected::Unsigned(v), &self))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<GainDto, E> {
                if v == GAIN_SILENT {
                    Ok(GainDto::Silent)
                } else {
                    Err(E::invalid_value(de::Unexpected::Str(v), &self))
                }
            }
        }

        deserializer.deserialize_any(GainVisitor)
    }
}

/// What a media item reads: `"type": "file"` or `"audio-of"`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum SourceDto {
    File(FileSourceDto),
    AudioOf(AudioOfSourceDto),
}

/// `"type": "file"`: an imported file, read whole.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FileSourceDto {
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "audio-of"`: one audio stream of the file of item `media`
/// (AU-004). `media` may no longer be in the library: the item has its own
/// copy of the path and keeps working.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AudioOfSourceDto {
    pub media: NonZeroU64,
    /// Container index of the audio stream.
    pub stream: u32,
    #[serde(flatten)]
    pub extra: Fields,
}

/// Where a media item came from (format 9, PM-041): `"type": "imported"`
/// or `"generated"`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum MediaOriginDto {
    Imported(PlainDto),
    Generated(GeneratedOriginDto),
}

/// `"type": "generated"`: a file FreeCut made, kept in the project's
/// generated-media store.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GeneratedOriginDto {
    pub kind: GeneratedMediaKindDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// What made a generated media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GeneratedMediaKindDto {
    /// A voiceover take recorded in the app (AU-027).
    Voiceover,
    /// Text-to-speech audio (AI-025).
    Tts,
}

// ---------------------------------------------------------------------------
// Export settings (EX-021)
// ---------------------------------------------------------------------------

/// The last export's settings.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ExportDto {
    pub file: ExportFileDto,
    pub video: ExportVideoDto,
    pub audio: ExportAudioDto,
    #[serde(flatten)]
    pub extra: Fields,
}

/// Output name (as typed, before sanitizing) and folder (EX-023).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ExportFileDto {
    pub name: String,
    /// An absolute folder path; `null` when none was chosen yet (or the
    /// folder's path isn't Unicode).
    pub folder: Option<String>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ExportVideoDto {
    /// Output size (EX-024): `"type": "canvas"` or `"preset"`.
    pub size: Tagged<OutputSizeDto>,
    /// Output rate (EX-026): `"type": "project"` or `"fixed"`.
    pub frame_rate: Tagged<FrameRateChoiceDto>,
    pub codec: VideoCodecDto,
    pub container: ContainerDto,
    /// Quality (EX-027): `"type": "tier"`.
    pub quality: Tagged<QualityDto>,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum OutputSizeDto {
    Canvas(PlainDto),
    Preset(PresetSizeDto),
}

/// A variant without fields of its own.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PlainDto {
    #[serde(flatten)]
    pub extra: Fields,
}

/// `"type": "preset"`: the output's short side in pixels.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PresetSizeDto {
    pub short_side: u32,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum FrameRateChoiceDto {
    Project(PlainDto),
    Fixed(FixedRateDto),
}

/// `"type": "fixed"`: a rate from the export list.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FixedRateDto {
    pub rate: RateDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum QualityDto {
    Tier(TierDto),
}

/// `"type": "tier"`: one of the bitrate tiers.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TierDto {
    pub tier: QualityTierDto,
    #[serde(flatten)]
    pub extra: Fields,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum QualityTierDto {
    Smaller,
    Recommended,
    Higher,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VideoCodecDto {
    H264,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ContainerDto {
    Mp4,
}

/// Audio of a video export (EX-031): AAC stereo.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ExportAudioDto {
    pub include: bool,
    pub kbps: u32,
    /// Hz.
    pub sample_rate: u32,
    #[serde(flatten)]
    pub extra: Fields,
}

/// serde's `skip_serializing_if` for flags written only when set.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes a reference
fn is_false(b: &bool) -> bool {
    !*b
}
