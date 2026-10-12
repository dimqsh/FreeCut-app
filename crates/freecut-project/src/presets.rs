//! Animation preset manifests (FX-030, TX-056): the JSON file format, its
//! parser, and the presets compiled into FreeCut.
//!
//! A preset is one JSON file. Clip presets live in `resources/animations/`,
//! text presets in `resources/text-animations/`, each named `<id>.json`.
//! The parser turns a file into `freecut_timeline::motion::PresetManifest`,
//! the typed form the pure evaluator plays; nothing here evaluates motion.
//!
//! ```json
//! {
//!   "schema": 1,
//!   "id": "fc.in.rise",
//!   "kind": "entrance",
//!   "units": null,
//!   "directions": null,
//!   "default_duration_ms": 500,
//!   "easing": "ease-out-cubic",
//!   "pivot": "anchor",
//!   "defaults": { "overlap": 500, "order": "forward", "pace_ms": null },
//!   "channels": [
//!     { "property": "translate_y", "curve": { "type": "eased" },
//!       "amplitude": 1.0, "unit": "em", "scales_with_intensity": true },
//!     { "property": "opacity",
//!       "curve": { "type": "sampled", "points": [[0.0, 1.0], [0.4, 0.0], [1.0, 0.0]], "eased": false },
//!       "amplitude": -1.0, "unit": "ratio", "scales_with_intensity": false }
//!   ]
//! }
//! ```
//!
//! - `schema`: the file format version, [`MANIFEST_SCHEMA`]. A file with a
//!   newer schema is ignored with a log line ([`PresetFileError::NewerSchema`]),
//!   so a preset made for a newer FreeCut is simply missing (FX-018's
//!   **Unavailable animation** for clips that chose it).
//! - `kind`: `entrance`, `exit`, `loop`, `drift` or `combined`.
//! - `units` (TX-023's pieces, the first being the default: `block`,
//!   `line`, `word`, `letter`) and `directions` (`left`, `right`, `up`,
//!   `down`, `clockwise`, `counter-clockwise`, the first being the
//!   default): optional, `null` or missing for none.
//! - `default_duration_ms`: the slot length, or the cycle of a loop, in
//!   whole milliseconds (exact in ticks).
//! - `easing`: the built-in time curve, a TR-031 name as in project files
//!   (`linear`, `ease-out-cubic`, `overshoot-out`, …).
//! - `pivot`: `anchor` (default), `top-center`, `bottom-center` or `edge`.
//! - `defaults` (optional): TX-056's starting `overlap` (per-mille, default
//!   500), `order` (`forward` default, `backward`, `center-out`,
//!   `edges-in`, `random`) and `pace_ms` (20–1000, or `null`).
//! - `channels`: at least one. `property` is a channel name of
//!   `freecut_timeline::motion::ChannelProp` (`translate_x`, `scale`,
//!   `style.hue`, the seeded `random.offset` and `jitter.rotate`, …); a
//!   name this build doesn't know makes the preset unavailable
//!   ([`PresetFileError::Unavailable`]). `curve` is
//!   `{"type": "eased"}` (the slot easing), `{"type": "ease", "easing": …}`
//!   (a fixed TR-031 curve), `{"type": "sampled", "points": [[t, v], …],
//!   "eased": bool}` (monotone cubic through 1–16 points, `t` strictly
//!   increasing in 0–1), `{"type": "sine"}` or `{"type": "step"}`.
//!   `amplitude` is a number in `unit`s (`se`, `em`, `deg`, `ratio`,
//!   `travel`); `scales_with_intensity` defaults to `false`.
//!
//! Unknown keys are refused, so a typo can't silently change a preset; a
//! format change bumps `schema` instead. Amplitudes and curve points are
//! the only floats: they are non-time scalars (FX-030), and time is whole
//! milliseconds.

use std::collections::BTreeSet;
use std::sync::Arc;

use freecut_timeline::model::{Interp, PerMille, PresetId};
use freecut_timeline::motion::{
    Channel, ChannelProp, ChannelUnit, Curve, CurvePoint, ManifestError, ParamDefaults, Pivot,
    PresetKind, PresetManifest, PresetTable,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};
use serde::Deserialize;

use crate::format::dto::{AnimOrderDto, AnimUnitDto, DirectionDto, InterpDto};
use crate::format::map::motion::{anim_order, anim_unit, direction, interp};

/// The manifest file format this build reads.
pub const MANIFEST_SCHEMA: u32 = 1;

/// Why a preset file was not loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresetFileError {
    /// Not a manifest: bad JSON, a missing or unknown key, a value of the
    /// wrong type. `path` is the JSON path (`channels[1].curve.type`).
    #[error("{path}: {message}")]
    Json { path: String, message: String },
    /// A value of the right type that can't be used: a sampled curve
    /// whose times don't increase, a Pace outside 0.02–1 s.
    #[error("{path}: {message}")]
    BadValue { path: String, message: String },
    /// Written for a newer FreeCut; ignored.
    #[error("manifest schema {found} is newer than this build's {supported}")]
    NewerSchema { found: u32, supported: u32 },
    /// Readable, but this build can't play it: an unknown channel, or a
    /// manifest that fails `PresetManifest::check` (FX-030: shown as
    /// **Unavailable animation**).
    #[error("unavailable: {0}")]
    Unavailable(ManifestError),
    /// Another loaded file already has this id; the first one wins.
    #[error("the preset id {id} is already taken")]
    Duplicate { id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDto {
    /// Checked by [`SchemaPeek`] before the typed decode.
    #[serde(rename = "schema")]
    _schema: u32,
    id: String,
    kind: KindDto,
    #[serde(default)]
    units: Option<Vec<AnimUnitDto>>,
    #[serde(default)]
    directions: Option<Vec<DirectionDto>>,
    default_duration_ms: u32,
    easing: InterpDto,
    #[serde(default)]
    pivot: PivotDto,
    #[serde(default)]
    defaults: Option<DefaultsDto>,
    channels: Vec<ChannelDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum KindDto {
    Entrance,
    Exit,
    Loop,
    Drift,
    Combined,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
enum PivotDto {
    #[default]
    Anchor,
    TopCenter,
    BottomCenter,
    Edge,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsDto {
    #[serde(default = "default_overlap")]
    overlap: u16,
    #[serde(default = "default_order")]
    order: AnimOrderDto,
    #[serde(default)]
    pace_ms: Option<u32>,
}

fn default_overlap() -> u16 {
    ParamDefaults::default().overlap.get()
}

fn default_order() -> AnimOrderDto {
    AnimOrderDto::Forward
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelDto {
    property: String,
    curve: CurveDto,
    amplitude: f64,
    unit: UnitDto,
    #[serde(default)]
    scales_with_intensity: bool,
}

/// A curve: `type` and the keys that type uses. A plain struct rather than
/// a tagged enum, so serde reads it without buffering (which would lose
/// the JSON paths and, with `arbitrary_precision`, the floats).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurveDto {
    #[serde(rename = "type")]
    kind: CurveKindDto,
    #[serde(default)]
    easing: Option<InterpDto>,
    #[serde(default)]
    points: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    eased: Option<bool>,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum CurveKindDto {
    Eased,
    Ease,
    Sampled,
    Sine,
    Step,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum UnitDto {
    Se,
    Em,
    Deg,
    Ratio,
    Travel,
}

/// Just the schema version, read before the typed decode so a newer file
/// with keys this build doesn't know is ignored rather than refused.
#[derive(Deserialize)]
struct SchemaPeek {
    schema: Option<u32>,
}

/// Just the id, read from a file that could not be loaded so the table can
/// still name it as unavailable.
#[derive(Deserialize)]
struct IdPeek {
    id: Option<String>,
}

/// The id a file declares, if it is JSON with a non-empty string `id`.
fn peek_id(text: &str) -> Option<PresetId> {
    let peek: IdPeek = serde_json::from_str(text).ok()?;
    peek.id.filter(|id| !id.is_empty()).map(PresetId::new)
}

fn json_error(err: serde_path_to_error::Error<serde_json::Error>) -> PresetFileError {
    let path = err.path().to_string();
    PresetFileError::Json {
        path,
        message: err.into_inner().to_string(),
    }
}

fn bad(path: impl Into<String>, message: impl Into<String>) -> PresetFileError {
    PresetFileError::BadValue {
        path: path.into(),
        message: message.into(),
    }
}

/// `ms` whole milliseconds in ticks (exact: a millisecond is 705 600
/// ticks).
fn millis(ms: u32) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * i64::from(ms))
}

/// Parses one manifest file.
pub fn parse_manifest(text: &str) -> Result<PresetManifest, PresetFileError> {
    let peek: SchemaPeek =
        serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_str(text))
            .map_err(json_error)?;
    match peek.schema {
        None => {
            return Err(PresetFileError::Json {
                path: ".".into(),
                message: "missing field `schema`".into(),
            });
        }
        Some(0) => return Err(bad("schema", "schema versions start at 1")),
        Some(found) if found > MANIFEST_SCHEMA => {
            return Err(PresetFileError::NewerSchema {
                found,
                supported: MANIFEST_SCHEMA,
            });
        }
        Some(_) => {}
    }
    let mut de = serde_json::Deserializer::from_str(text);
    let dto: ManifestDto = serde_path_to_error::deserialize(&mut de).map_err(json_error)?;
    de.end().map_err(|e| PresetFileError::Json {
        path: ".".into(),
        message: e.to_string(),
    })?;
    manifest(dto)
}

fn manifest(dto: ManifestDto) -> Result<PresetManifest, PresetFileError> {
    if dto.id.is_empty() {
        return Err(bad("id", "a preset id can't be empty"));
    }
    let defaults = match dto.defaults {
        None => ParamDefaults::default(),
        Some(d) => {
            if d.overlap > 1000 {
                return Err(bad("defaults.overlap", "overlap is at most 1000 per-mille"));
            }
            let pace = match d.pace_ms {
                Some(ms) if !(20..=1000).contains(&ms) => {
                    return Err(bad("defaults.pace_ms", "Pace is 20 to 1000 ms per unit"));
                }
                other => other.map(millis),
            };
            ParamDefaults {
                overlap: PerMille::new(d.overlap),
                order: anim_order(d.order),
                pace,
            }
        }
    };
    let mut channels = Vec::with_capacity(dto.channels.len());
    for (i, c) in dto.channels.into_iter().enumerate() {
        let at = format!("channels[{i}]");
        let property = ChannelProp::parse(&c.property).ok_or_else(|| {
            PresetFileError::Unavailable(ManifestError::UnknownChannel(c.property.clone()))
        })?;
        channels.push(Channel {
            property,
            curve: curve(c.curve, &format!("{at}.curve"))?,
            amplitude: c.amplitude,
            unit: match c.unit {
                UnitDto::Se => ChannelUnit::Se,
                UnitDto::Em => ChannelUnit::Em,
                UnitDto::Deg => ChannelUnit::Deg,
                UnitDto::Ratio => ChannelUnit::Ratio,
                UnitDto::Travel => ChannelUnit::Travel,
            },
            scales_with_intensity: c.scales_with_intensity,
        });
    }
    let out = PresetManifest {
        id: PresetId::new(&dto.id),
        kind: match dto.kind {
            KindDto::Entrance => PresetKind::Entrance,
            KindDto::Exit => PresetKind::Exit,
            KindDto::Loop => PresetKind::Loop,
            KindDto::Drift => PresetKind::Drift,
            KindDto::Combined => PresetKind::Combined,
        },
        units: dto
            .units
            .map(|units| units.into_iter().map(anim_unit).collect()),
        directions: dto
            .directions
            .map(|dirs| dirs.into_iter().map(direction).collect()),
        default_duration: millis(dto.default_duration_ms),
        easing: interp(dto.easing),
        pivot: match dto.pivot {
            PivotDto::Anchor => Pivot::Anchor,
            PivotDto::TopCenter => Pivot::TopCenter,
            PivotDto::BottomCenter => Pivot::BottomCenter,
            PivotDto::Edge => Pivot::Edge,
        },
        channels,
        defaults,
    };
    out.check().map_err(PresetFileError::Unavailable)?;
    Ok(out)
}

fn curve(dto: CurveDto, at: &str) -> Result<Curve, PresetFileError> {
    let wants = |key: &str, present: bool, used: bool| {
        if present && !used {
            Err(bad(
                format!("{at}.{key}"),
                format!("a curve of this type has no `{key}`"),
            ))
        } else if !present && used {
            Err(PresetFileError::Json {
                path: at.to_string(),
                message: format!("missing field `{key}`"),
            })
        } else {
            Ok(())
        }
    };
    let sampled = dto.kind == CurveKindDto::Sampled;
    wants(
        "easing",
        dto.easing.is_some(),
        dto.kind == CurveKindDto::Ease,
    )?;
    wants("points", dto.points.is_some(), sampled)?;
    wants("eased", dto.eased.is_some(), sampled)?;
    Ok(match dto.kind {
        CurveKindDto::Eased => Curve::Eased,
        CurveKindDto::Sine => Curve::Sine,
        CurveKindDto::Step => Curve::Step,
        CurveKindDto::Ease => Curve::Ease(dto.easing.map_or(Interp::Linear, interp)),
        CurveKindDto::Sampled => {
            let points = dto
                .points
                .unwrap_or_default()
                .into_iter()
                .map(|[t, v]| CurvePoint { t, v })
                .collect();
            Curve::sampled(points, dto.eased.unwrap_or(false)).ok_or_else(|| {
                bad(
                    format!("{at}.points"),
                    "a sampled curve has 1 to 16 points with times strictly increasing in 0–1",
                )
            })?
        }
    })
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// A preset file that was not loaded, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The file, as given to [`load`] (bundled files: their path under
    /// `resources/`).
    pub file: String,
    pub error: PresetFileError,
}

/// The loaded presets and the files that were left out.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// The loaded presets. It also knows, by id, the presets this build
    /// can't play (a channel it lacks, a newer schema), so grids can show
    /// them as FX-018's **Unavailable animation** instead of hiding them
    /// (`PresetTable::availability`).
    pub table: PresetTable,
    pub skipped: Vec<Skipped>,
}

/// Parses `(file name, text)` pairs into a catalogue. Files that can't be
/// used are left out and logged: a newer schema or a channel this build
/// lacks as information, anything else as a warning. Of two files with one
/// id, the first wins. A left-out file that is readable but can't be played
/// ([`PresetFileError::NewerSchema`], [`PresetFileError::Unavailable`]) is
/// still registered by its id as unavailable in the table, with the reason;
/// a malformed or duplicate file is not.
pub fn load<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Catalog {
    let mut ids = BTreeSet::new();
    let mut manifests = Vec::new();
    let mut skipped = Vec::new();
    let mut unavailable = Vec::new();
    for (file, text) in files {
        let result = parse_manifest(text).and_then(|m| {
            if ids.insert(m.id.clone()) {
                Ok(m)
            } else {
                Err(PresetFileError::Duplicate {
                    id: m.id.to_string(),
                })
            }
        });
        match result {
            Ok(m) => manifests.push(m),
            Err(error) => {
                match &error {
                    // Made for a newer FreeCut (a newer schema, or a channel
                    // this build lacks): not a fault.
                    PresetFileError::NewerSchema { .. }
                    | PresetFileError::Unavailable(ManifestError::UnknownChannel(_)) => {
                        tracing::info!(file, %error, "animation preset ignored");
                    }
                    _ => tracing::warn!(file, %error, "animation preset ignored"),
                }
                // FX-030: a preset this build can't play is still listed,
                // as **Unavailable animation** (FX-018).
                if matches!(
                    error,
                    PresetFileError::NewerSchema { .. } | PresetFileError::Unavailable(_)
                ) && let Some(id) = peek_id(text)
                {
                    unavailable.push((id, Arc::<str>::from(error.to_string())));
                }
                skipped.push(Skipped {
                    file: file.to_string(),
                    error,
                });
            }
        }
    }
    Catalog {
        table: PresetTable::new(manifests).with_unavailable(unavailable),
        skipped,
    }
}

/// One preset file compiled into FreeCut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledFile {
    /// The path under `resources/`, `text-animations/fc.in.fade.json`.
    pub path: &'static str,
    pub text: &'static str,
}

macro_rules! bundled {
    ($($path:literal),* $(,)?) => {
        &[$(BundledFile {
            path: $path,
            text: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/", $path)),
        }),*]
    };
}

/// Every preset file in `resources/animations/` and
/// `resources/text-animations/`, in path order. A test checks that this
/// list matches the directories, so adding a file means adding it here.
/// A few files name a channel this build lacks and load as unavailable
/// (`resources/README.md` lists them); the tests pin which.
pub const BUNDLED: &[BundledFile] = bundled![
    "animations/fc.clip.combined.pop-fade.json",
    "animations/fc.clip.combined.slide-through.json",
    "animations/fc.clip.combined.spin-through.json",
    "animations/fc.clip.combined.zoom-through.json",
    "animations/fc.clip.drift.pan.json",
    "animations/fc.clip.drift.zoom-in.json",
    "animations/fc.clip.drift.zoom-out.json",
    "animations/fc.clip.in.blur.json",
    "animations/fc.clip.in.drop.json",
    "animations/fc.clip.in.fade.json",
    "animations/fc.clip.in.flash.json",
    "animations/fc.clip.in.flip.json",
    "animations/fc.clip.in.grow.json",
    "animations/fc.clip.in.pop.json",
    "animations/fc.clip.in.rise.json",
    "animations/fc.clip.in.shake.json",
    "animations/fc.clip.in.shrink.json",
    "animations/fc.clip.in.slide.json",
    "animations/fc.clip.in.spin.json",
    "animations/fc.clip.in.squash.json",
    "animations/fc.clip.in.stretch.json",
    "animations/fc.clip.in.swing.json",
    "animations/fc.clip.in.wipe.json",
    "animations/fc.clip.loop.bounce.json",
    "animations/fc.clip.loop.float.json",
    "animations/fc.clip.loop.heartbeat.json",
    "animations/fc.clip.loop.jitter.json",
    "animations/fc.clip.loop.pendulum.json",
    "animations/fc.clip.loop.rotate-zoom.json",
    "animations/fc.clip.loop.spin.json",
    "text-animations/fc.in.blur.json",
    "text-animations/fc.in.bounce.json",
    "text-animations/fc.in.decode.json",
    "text-animations/fc.in.drop.json",
    "text-animations/fc.in.fade.json",
    "text-animations/fc.in.flip-open.json",
    "text-animations/fc.in.focus-pull.json",
    "text-animations/fc.in.letter-blur.json",
    "text-animations/fc.in.letter-drop.json",
    "text-animations/fc.in.letter-fade.json",
    "text-animations/fc.in.letter-pop.json",
    "text-animations/fc.in.letter-rise.json",
    "text-animations/fc.in.letter-spin.json",
    "text-animations/fc.in.marker-sweep.json",
    "text-animations/fc.in.pop.json",
    "text-animations/fc.in.rise.json",
    "text-animations/fc.in.scatter.json",
    "text-animations/fc.in.slide.json",
    "text-animations/fc.in.spin.json",
    "text-animations/fc.in.spread.json",
    "text-animations/fc.in.typewriter.json",
    "text-animations/fc.in.wipe.json",
    "text-animations/fc.in.word-rise.json",
    "text-animations/fc.in.word-slide.json",
    "text-animations/fc.in.zoom-in.json",
    "text-animations/fc.in.zoom-out.json",
    "text-animations/fc.loop.breathing-glow.json",
    "text-animations/fc.loop.flicker.json",
    "text-animations/fc.loop.float.json",
    "text-animations/fc.loop.heartbeat.json",
    "text-animations/fc.loop.hop.json",
    "text-animations/fc.loop.hue-cycle.json",
    "text-animations/fc.loop.jelly.json",
    "text-animations/fc.loop.pulse.json",
    "text-animations/fc.loop.shake.json",
    "text-animations/fc.loop.shimmer.json",
    "text-animations/fc.loop.sparkle.json",
    "text-animations/fc.loop.swing.json",
    "text-animations/fc.loop.wave.json",
    "text-animations/fc.loop.wiggle.json",
    "text-animations/fc.loop.wobble.json",
    "text-animations/fc.out.erase.json",
    "text-animations/fc.out.fall-away.json",
];

/// The presets compiled into FreeCut.
pub fn bundled() -> Catalog {
    load(BUNDLED.iter().map(|f| (f.path, f.text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecut_timeline::motion::{Availability, PresetCatalog, samples};
    use std::path::PathBuf;

    fn resources() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources")
    }

    /// The `.json` files of `dir` under `resources/`, as `dir/name`.
    fn listed(dir: &str) -> Vec<String> {
        let entries = match std::fs::read_dir(resources().join(dir)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => panic!("{dir}: {e}"),
        };
        entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".json"))
            .map(|name| format!("{dir}/{name}"))
            .collect()
    }

    #[test]
    fn the_bundled_table_matches_the_resource_directories() {
        let mut on_disk = listed("animations");
        on_disk.extend(listed("text-animations"));
        on_disk.sort();
        let table: Vec<String> = BUNDLED.iter().map(|f| f.path.to_string()).collect();
        let mut sorted = table.clone();
        sorted.sort();
        assert_eq!(table, sorted, "keep BUNDLED in path order");
        assert_eq!(
            table, on_disk,
            "BUNDLED must list exactly the .json files in resources/animations and \
             resources/text-animations"
        );
        for file in BUNDLED {
            let on_disk = std::fs::read_to_string(resources().join(file.path)).unwrap();
            assert_eq!(file.text, on_disk, "{}", file.path);
        }
    }

    #[test]
    fn every_bundled_file_loads_and_is_named_after_its_id() {
        let catalog = bundled();
        // Every channel the catalogue names exists (TX-056 and FX-030's
        // seeded and line-relative ones included), so nothing waits.
        assert_eq!(catalog.skipped, Vec::<Skipped>::new());
        assert_eq!(catalog.table.manifests().count(), BUNDLED.len());
        assert_eq!(catalog.table.unavailable().count(), 0);
        for file in BUNDLED {
            let v: serde_json::Value = serde_json::from_str(file.text).unwrap();
            let id = v["id"].as_str().unwrap();
            let name = file.path.rsplit('/').next().unwrap();
            assert_eq!(name, format!("{id}.json"), "{}", file.path);
            let dir_is_text = file.path.starts_with("text-animations/");
            assert!(dir_is_text || file.path.starts_with("animations/"));
            // Clip presets have ids of their own, so the two grids never
            // share one.
            assert_eq!(!dir_is_text, id.starts_with("fc.clip."), "{id}");
        }
    }

    #[test]
    fn the_sample_files_equal_the_timelines_built_in_samples() {
        let catalog = bundled();
        for sample in samples() {
            let loaded = catalog
                .table
                .get(&sample.id)
                .unwrap_or_else(|| panic!("{} is not bundled", sample.id));
            assert_eq!(*loaded, sample, "{}", sample.id);
        }
    }

    const FADE: &str = r#"{
        "schema": 1, "id": "fc.in.fade", "kind": "entrance",
        "default_duration_ms": 500, "easing": "ease-out-quad",
        "channels": [{"property": "opacity", "curve": {"type": "eased"},
                      "amplitude": -1.0, "unit": "ratio"}]
    }"#;

    fn with(change: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut v: serde_json::Value = serde_json::from_str(FADE).unwrap();
        change(&mut v);
        v.to_string()
    }

    #[test]
    fn optional_keys_take_their_defaults() {
        let m = parse_manifest(FADE).unwrap();
        assert_eq!(m.units, None);
        assert_eq!(m.directions, None);
        assert_eq!(m.pivot, Pivot::Anchor);
        assert_eq!(m.defaults, ParamDefaults::default());
        assert!(!m.channels[0].scales_with_intensity);
        assert_eq!(m.default_duration, Tick::from_raw(TICKS_PER_SECOND / 2));
        let partial = with(|v| v["defaults"] = serde_json::json!({"pace_ms": 40}));
        let m = parse_manifest(&partial).unwrap();
        assert_eq!(m.defaults.overlap, PerMille::new(500));
        assert_eq!(m.defaults.pace, Some(Tick::from_raw(TICKS_PER_SECOND / 25)));
    }

    #[test]
    fn an_unknown_channel_makes_the_preset_unavailable() {
        let text = with(|v| v["channels"][0]["property"] = "glitch".into());
        assert_eq!(
            parse_manifest(&text),
            Err(PresetFileError::Unavailable(ManifestError::UnknownChannel(
                "glitch".into()
            )))
        );
        let text = with(|v| {
            v["id"] = "fc.in.glitch".into();
            v["channels"][0]["property"] = "glitch".into();
        });
        let catalog = load([("glitch.json", text.as_str()), ("fade.json", FADE)]);
        assert_eq!(catalog.table.manifests().count(), 1);
        assert_eq!(catalog.skipped.len(), 1);
        assert_eq!(catalog.skipped[0].file, "glitch.json");
        // Grids still list it, as **Unavailable animation** with why.
        let glitch = PresetId::new("fc.in.glitch");
        assert_eq!(
            catalog.table.availability(&glitch),
            Availability::Unavailable("unavailable: unknown channel \"glitch\"".into())
        );
        assert_eq!(
            catalog.table.availability(&PresetId::new("fc.in.fade")),
            Availability::Ready
        );
        // A clip that chose it plays no animation (FX-018).
        assert!(
            catalog
                .table
                .resolve(freecut_timeline::model::Slot::Entrance, &glitch)
                .is_none()
        );
    }

    #[test]
    fn a_newer_schema_is_ignored_even_with_keys_this_build_lacks() {
        let text = with(|v| {
            v["schema"] = 2.into();
            v["sparkles"] = true.into();
        });
        assert_eq!(
            parse_manifest(&text),
            Err(PresetFileError::NewerSchema {
                found: 2,
                supported: MANIFEST_SCHEMA
            })
        );
        let catalog = load([("future.json", text.as_str())]);
        assert_eq!(catalog.table.manifests().count(), 0);
        assert!(matches!(
            catalog.skipped[0].error,
            PresetFileError::NewerSchema { .. }
        ));
        assert!(matches!(
            catalog.table.availability(&PresetId::new("fc.in.fade")),
            Availability::Unavailable(_)
        ));
    }

    #[test]
    fn malformed_files_are_refused_with_their_path() {
        let json_path = |text: &str| match parse_manifest(text) {
            Err(PresetFileError::Json { path, .. }) => path,
            other => panic!("{other:?}"),
        };
        let bad_path = |text: &str| match parse_manifest(text) {
            Err(PresetFileError::BadValue { path, .. }) => path,
            other => panic!("{other:?}"),
        };
        assert_eq!(json_path("{"), "?");
        assert_eq!(
            json_path(&with(|v| v.as_object_mut().unwrap().clear())),
            "."
        );
        assert_eq!(json_path(&with(|v| v["colour"] = 1.into())), "colour");
        assert_eq!(
            json_path(&with(
                |v| v["channels"][0]["curve"]["type"] = "wobble".into()
            )),
            "channels[0].curve.type"
        );
        assert_eq!(
            json_path(&with(|v| v["channels"][0]["unit"] = "px".into())),
            "channels[0].unit"
        );
        assert_eq!(json_path(&with(|v| v["easing"] = "fast".into())), "easing");
        assert_eq!(
            json_path(&with(|v| v["default_duration_ms"] = 0.5.into())),
            "default_duration_ms"
        );
        assert_eq!(
            json_path(&with(
                |v| v["channels"][0]["curve"] = serde_json::json!({"type": "ease"})
            )),
            "channels[0].curve"
        );
        assert_eq!(
            bad_path(&with(|v| v["channels"][0]["curve"] = serde_json::json!(
                {"type": "sampled", "points": [[0.5, 0.0], [0.2, 1.0]], "eased": false}
            ))),
            "channels[0].curve.points"
        );
        assert_eq!(
            bad_path(&with(
                |v| v["channels"][0]["curve"]["easing"] = "linear".into()
            )),
            "channels[0].curve.easing"
        );
        assert_eq!(
            bad_path(&with(
                |v| v["defaults"] = serde_json::json!({"overlap": 1001})
            )),
            "defaults.overlap"
        );
        assert_eq!(
            bad_path(&with(|v| v["defaults"] = serde_json::json!({"pace_ms": 5}))),
            "defaults.pace_ms"
        );
        assert_eq!(bad_path(&with(|v| v["id"] = "".into())), "id");
        assert_eq!(bad_path(&with(|v| v["schema"] = 0.into())), "schema");
        // What `check` refuses: no channels, a zero length, empty lists.
        let unavailable =
            |text: &str| matches!(parse_manifest(text), Err(PresetFileError::Unavailable(_)));
        assert!(unavailable(
            &with(|v| v["channels"] = serde_json::json!([]))
        ));
        assert!(unavailable(&with(|v| v["default_duration_ms"] = 0.into())));
        assert!(unavailable(&with(|v| v["units"] = serde_json::json!([]))));
    }

    #[test]
    fn the_first_of_two_files_with_one_id_wins() {
        let longer = with(|v| v["default_duration_ms"] = 900.into());
        let catalog = load([("a.json", FADE), ("b.json", longer.as_str())]);
        let fade = catalog.table.get(&PresetId::new("fc.in.fade")).unwrap();
        assert_eq!(fade.default_duration, Tick::from_raw(TICKS_PER_SECOND / 2));
        assert_eq!(
            catalog.skipped,
            vec![Skipped {
                file: "b.json".into(),
                error: PresetFileError::Duplicate {
                    id: "fc.in.fade".into()
                }
            }]
        );
        assert_eq!(catalog.table.unavailable().count(), 0);
    }

    #[test]
    fn only_presets_this_build_cant_play_are_listed_as_unavailable() {
        // Malformed files are faults, not presets: nothing to list.
        let typo = with(|v| v["colour"] = 1.into());
        let no_id = with(|v| {
            v.as_object_mut().unwrap().remove("id");
        });
        let bad_pace = with(|v| {
            v["id"] = "fc.in.slow".into();
            v["defaults"] = serde_json::json!({"pace_ms": 5});
        });
        // Readable but unplayable: listed, even without a channel to name.
        let empty = with(|v| {
            v["id"] = "fc.in.empty".into();
            v["channels"] = serde_json::json!([]);
        });
        let catalog = load([
            ("typo.json", typo.as_str()),
            ("no-id.json", no_id.as_str()),
            ("bad-pace.json", bad_pace.as_str()),
            ("empty.json", empty.as_str()),
            ("not-json.json", "{"),
        ]);
        assert_eq!(catalog.skipped.len(), 5);
        let listed: Vec<&str> = catalog
            .table
            .unavailable()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(listed, ["fc.in.empty"]);
        assert_eq!(
            catalog.table.availability(&PresetId::new("fc.in.slow")),
            Availability::Unknown
        );
    }
}
