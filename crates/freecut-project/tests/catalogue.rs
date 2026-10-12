//! The bundled preset catalogue (TX-056, FX-030) against our spec: every
//! preset's start pose and key moments as the spec's tables give them
//! (with bounds for the seeded and line-relative ones), golden samples at
//! 0/25/50/75/100 % of its slot, the rest rules (In ends at rest, Out
//! starts at rest, a Loop is periodic), Intensity scaling, and the
//! photosensitivity rule over every manifest, every seed and unit tried.
//!
//! Golden samples live in `tests/golden/presets.txt`. After a deliberate
//! change to a preset, re-bless them and review the diff:
//!
//! FREECUT_BLESS_GOLDEN=1 cargo test -p freecut-project --test catalogue
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use freecut_project::presets::{BUNDLED, bundled};
use freecut_timeline::glam::DVec2;
use freecut_timeline::model::{Direction, PerMille, PresetId, Slot};
use freecut_timeline::motion::{
    Availability, BlockOffsets, Channel, ChannelProp, ChannelUnit, Curve, CurvePoint, MotionScale,
    PresetCatalog, PresetKind, PresetManifest, UnitContext, is_photosafe, max_flashes_per_second,
    photosensitivity, pose_with,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};
use pretty_assertions::assert_eq;
use serde_json::Value;

/// Channel units measured in themselves: values read in se, em, degrees
/// and travel distances.
const UNITS: MotionScale = MotionScale {
    se_px: 1.0,
    em_px: 1.0,
    travel_px: 1.0,
};

const SAMPLES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn available() -> Vec<PresetManifest> {
    let mut all: Vec<PresetManifest> = bundled().table.manifests().map(|m| (**m).clone()).collect();
    all.sort_by(|a, b| a.id.cmp(&b.id));
    all
}

/// Every bundled file as JSON, with its path.
fn files() -> Vec<(&'static str, Value)> {
    BUNDLED
        .iter()
        .map(|f| (f.path, serde_json::from_str(f.text).unwrap()))
        .collect()
}

/// The slots a preset plays in, with a short label: an entrance also plays
/// as its own exit, backwards (FX-030).
fn plays(m: &PresetManifest) -> Vec<(Slot, &'static str)> {
    match m.kind {
        PresetKind::Entrance => vec![(Slot::Entrance, "in"), (Slot::Exit, "out")],
        PresetKind::Exit => vec![(Slot::Exit, "out")],
        PresetKind::Loop => vec![(Slot::Continuous, "loop")],
        PresetKind::Drift => vec![(Slot::Continuous, "drift")],
        PresetKind::Combined => vec![(Slot::Continuous, "combined")],
    }
}

/// The slot the spec's tables describe: In for entrances, Out for
/// exit-only presets, the Continuous slot otherwise.
fn main_slot(m: &PresetManifest) -> Slot {
    plays(m)[0].0
}

fn pose(m: &PresetManifest, slot: Slot, p: f64, intensity: u16) -> BlockOffsets {
    pose_of(m, slot, p, intensity, 0, &UnitContext::BLOCK)
}

/// One unit's pose with the stored `seed` (TX-023's dice).
fn pose_of(
    m: &PresetManifest,
    slot: Slot,
    p: f64,
    intensity: u16,
    seed: u32,
    unit: &UnitContext,
) -> BlockOffsets {
    let mut spec = m.default_spec();
    spec.params.intensity = PerMille::new(intensity);
    spec.params.seed = seed;
    pose_with(m, &spec, slot, p, &UNITS, unit)
}

/// Whether a preset has a channel that differs per unit or per seed
/// (TX-056's seeded and line-relative channels).
fn varies_per_unit(m: &PresetManifest) -> bool {
    m.channels
        .iter()
        .any(|c| c.property.is_seeded() || c.property.needs_line_geometry())
}

/// Unit `index` of a made-up line: a line 1.5 em high whose centre lies
/// `(1.5 − index, 0.25)` em from the unit's centre.
fn sample_unit(index: u32) -> UnitContext {
    UnitContext {
        index,
        to_line_center: DVec2::new(1.5 - f64::from(index), 0.25),
        line_height: Some(1.5),
        ..UnitContext::BLOCK
    }
}

/// The seeds and units the photosensitivity check and the bounds below
/// try on every seeded preset.
const SEEDS: [u32; 5] = [0, 1, 42, 0xDEAD_BEEF, u32::MAX];
const UNIT_INDEXES: [u32; 4] = [0, 1, 2, 17];

const FIELDS: [&str; 16] = [
    "tx",
    "ty",
    "sx",
    "sy",
    "rot",
    "turn_x",
    "turn_y",
    "opacity",
    "blur",
    "reveal",
    "tint",
    "decode",
    "hue",
    "glow",
    "lightness",
    "line_clip",
];

fn fields(o: &BlockOffsets) -> [f64; 16] {
    [
        o.translate.x,
        o.translate.y,
        o.scale.x,
        o.scale.y,
        o.rotation,
        o.turn.x,
        o.turn.y,
        o.opacity,
        o.blur,
        o.reveal,
        o.tint,
        o.decode,
        o.hue,
        o.glow,
        o.lightness,
        if o.line_clip { 1.0 } else { 0.0 },
    ]
}

fn field(o: &BlockOffsets, name: &str) -> f64 {
    let i = FIELDS
        .iter()
        .position(|f| *f == name)
        .unwrap_or_else(|| panic!("no field {name}"));
    fields(o)[i]
}

/// Angles (rotation, turns, hue) compare modulo a full turn.
fn is_angle(name: &str) -> bool {
    matches!(name, "rot" | "turn_x" | "turn_y" | "hue")
}

fn same(name: &str, a: f64, b: f64, eps: f64) -> bool {
    if is_angle(name) {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d) <= eps
    } else {
        (a - b).abs() <= eps
    }
}

fn assert_same_pose(a: &BlockOffsets, b: &BlockOffsets, eps: f64, what: &str) {
    for ((name, x), y) in FIELDS.iter().zip(fields(a)).zip(fields(b)) {
        assert!(same(name, x, y, eps), "{what}: {name} {x} vs {y}");
    }
}

// ---------------------------------------------------------------------------
// The spec's numbers
// ---------------------------------------------------------------------------

/// One preset as TX-056 / FX-030 define it: whether it has Intensity, and
/// values of its pose (at Intensity 50, `k` = 1) at slot progress `p` in
/// the slot the table describes. Lengths are in the preset's own units
/// (em on text, short edges on clips, the Travel distance for slides).
struct Spec {
    id: &'static str,
    intensity: bool,
    at: &'static [(f64, &'static str, f64)],
}

const fn spec(id: &'static str, intensity: bool, at: &'static [(f64, &'static str, f64)]) -> Spec {
    Spec { id, intensity, at }
}

const SPEC: &[Spec] = &[
    // TX-056, whole text, In.
    spec(
        "fc.in.fade",
        false,
        &[(0.0, "opacity", 0.0), (0.5, "opacity", 0.75)],
    ),
    spec(
        "fc.in.rise",
        true,
        &[
            (0.0, "ty", 1.0),
            (0.0, "opacity", 0.0),
            (0.4, "opacity", 1.0),
        ],
    ),
    // 1.5 em above; lands, then one bounce of 10 % of the distance.
    spec(
        "fc.in.drop",
        true,
        &[(0.0, "ty", -1.5), (0.6, "ty", 0.0), (0.8, "ty", -0.15)],
    ),
    spec("fc.in.slide", false, &[(0.0, "tx", -1.0)]),
    spec(
        "fc.in.zoom-in",
        true,
        &[
            (0.0, "sx", 0.5),
            (0.0, "sy", 0.5),
            (0.0, "opacity", 0.0),
            (0.4, "opacity", 1.0),
        ],
    ),
    spec(
        "fc.in.zoom-out",
        true,
        &[
            (0.0, "sx", 1.5),
            (0.0, "opacity", 0.0),
            (0.4, "opacity", 1.0),
        ],
    ),
    spec("fc.in.pop", true, &[(0.0, "sx", 0.0), (0.65, "sx", 1.15)]),
    spec("fc.in.bounce", true, &[(0.0, "ty", -2.0)]),
    spec(
        "fc.in.blur",
        true,
        &[
            (0.0, "blur", 0.4),
            (0.0, "opacity", 0.0),
            (0.5, "opacity", 1.0),
        ],
    ),
    spec(
        "fc.in.focus-pull",
        true,
        &[(0.0, "blur", 0.3), (0.0, "sx", 1.1), (0.0, "opacity", 1.0)],
    ),
    spec("fc.in.spin", true, &[(0.0, "rot", -90.0), (0.0, "sx", 0.6)]),
    spec("fc.in.flip-open", false, &[(0.0, "turn_x", 90.0)]),
    spec(
        "fc.in.wipe",
        false,
        &[(0.0, "reveal", 0.0), (0.5, "reveal", 0.5)],
    ),
    // TX-056, per unit (one unit's pose over its own window).
    spec(
        "fc.in.typewriter",
        false,
        &[(0.0, "opacity", 0.0), (0.001, "opacity", 1.0)],
    ),
    spec("fc.in.letter-fade", false, &[(0.0, "opacity", 0.0)]),
    spec(
        "fc.in.letter-rise",
        true,
        &[(0.0, "ty", 0.6), (0.0, "opacity", 0.0)],
    ),
    spec(
        "fc.in.letter-drop",
        true,
        &[(0.0, "ty", -0.8), (0.6, "ty", 0.0), (0.8, "ty", -0.08)],
    ),
    spec(
        "fc.in.letter-pop",
        true,
        &[(0.0, "sx", 0.0), (0.65, "sx", 1.2)],
    ),
    spec(
        "fc.in.letter-blur",
        true,
        &[(0.0, "blur", 0.3), (0.0, "opacity", 0.0)],
    ),
    spec(
        "fc.in.letter-spin",
        true,
        &[(0.0, "rot", 180.0), (0.0, "opacity", 0.0)],
    ),
    // Stand-ins until the end of the window; opacity stays at 1.
    spec(
        "fc.in.decode",
        false,
        &[
            (0.0, "decode", 1.0),
            (0.9, "decode", 1.0),
            (0.5, "opacity", 1.0),
            (0.0, "opacity", 1.0),
        ],
    ),
    spec(
        "fc.in.word-slide",
        true,
        &[(0.0, "tx", -1.0), (0.0, "opacity", 0.0)],
    ),
    // Seeded offset and turn: bounds in `seeded_presets_stay_within_the_spec`.
    spec(
        "fc.in.scatter",
        true,
        &[(0.0, "opacity", 0.0), (1.0, "opacity", 1.0)],
    ),
    // A whole block has no line centre to gather at: it only fades.
    spec(
        "fc.in.spread",
        true,
        &[(0.0, "opacity", 0.0), (0.0, "tx", 0.0)],
    ),
    // One line height (1.2 em without a layout), clipped while it moves.
    spec(
        "fc.in.word-rise",
        false,
        &[
            (0.0, "ty", 1.2),
            (0.0, "line_clip", 1.0),
            (0.0, "opacity", 1.0),
        ],
    ),
    // The box over the first half, then the text fades in.
    spec(
        "fc.in.marker-sweep",
        false,
        &[
            (0.0, "reveal", 0.0),
            (0.0, "opacity", 0.0),
            (0.5, "reveal", 1.0),
            (0.5, "opacity", 0.0),
        ],
    ),
    // TX-056, Out only.
    spec(
        "fc.out.erase",
        false,
        &[(0.0, "opacity", 1.0), (0.001, "opacity", 0.0)],
    ),
    // Falls 2·k em, fading over the last 40 %; the turn is seeded.
    spec(
        "fc.out.fall-away",
        true,
        &[
            (0.0, "ty", 0.0),
            (1.0, "ty", 2.0),
            (0.6, "opacity", 1.0),
            (1.0, "opacity", 0.0),
        ],
    ),
    // TX-056, Loop, whole text.
    spec(
        "fc.loop.pulse",
        true,
        &[(0.0, "sx", 1.0), (0.5, "sx", 1.06), (0.5, "sy", 1.06)],
    ),
    spec(
        "fc.loop.heartbeat",
        true,
        &[
            (0.1, "sx", 1.08),
            (0.3, "sx", 1.05),
            (0.4, "sx", 1.0),
            (0.7, "sx", 1.0),
        ],
    ),
    spec(
        "fc.loop.float",
        true,
        &[(0.25, "ty", -0.15), (0.75, "ty", 0.15)],
    ),
    spec(
        "fc.loop.wobble",
        true,
        &[(0.25, "rot", 4.0), (0.75, "rot", -4.0)],
    ),
    spec(
        "fc.loop.swing",
        true,
        &[(0.25, "rot", 8.0), (0.75, "rot", -8.0)],
    ),
    spec(
        "fc.loop.hue-cycle",
        false,
        &[(0.25, "hue", 90.0), (0.5, "hue", 180.0)],
    ),
    // 30 % to 30 % + 70 %·k of the set glow: at k = 1, 1.0 down to 0.3.
    spec(
        "fc.loop.breathing-glow",
        true,
        &[(0.0, "glow", 1.0), (0.5, "glow", 0.3)],
    ),
    // Seeded jitter and dips: `seeded_presets_stay_within_the_spec`.
    spec("fc.loop.shake", true, &[(0.5, "opacity", 1.0)]),
    spec("fc.loop.flicker", true, &[(0.5, "tx", 0.0)]),
    // TX-056, Loop, per unit.
    spec(
        "fc.loop.wave",
        true,
        &[(0.25, "ty", -0.25), (0.75, "ty", 0.25)],
    ),
    spec(
        "fc.loop.hop",
        true,
        &[(0.125, "ty", -0.4), (0.5, "ty", 0.0)],
    ),
    spec(
        "fc.loop.jelly",
        true,
        &[(0.25, "sx", 1.1), (0.25, "sy", 0.9)],
    ),
    spec(
        "fc.loop.shimmer",
        true,
        &[(0.1, "lightness", 0.4), (0.5, "lightness", 0.0)],
    ),
    spec("fc.loop.wiggle", true, &[(0.5, "opacity", 1.0)]),
    spec("fc.loop.sparkle", true, &[(0.5, "opacity", 1.0)]),
    // FX-030, entrances.
    spec("fc.clip.in.fade", false, &[(0.0, "opacity", 0.0)]),
    spec(
        "fc.clip.in.grow",
        true,
        &[
            (0.0, "sx", 0.6),
            (0.0, "opacity", 0.0),
            (0.4, "opacity", 1.0),
        ],
    ),
    spec(
        "fc.clip.in.shrink",
        true,
        &[(0.0, "sx", 1.4), (0.4, "opacity", 1.0)],
    ),
    spec("fc.clip.in.slide", false, &[(0.0, "tx", -1.0)]),
    // 0.12 se below, passes rest by 8 % of that distance.
    spec(
        "fc.clip.in.rise",
        true,
        &[
            (0.0, "ty", 0.12),
            (0.0, "opacity", 0.0),
            (0.3, "opacity", 1.0),
            (0.55, "ty", -0.0096),
        ],
    ),
    spec("fc.clip.in.drop", true, &[(0.0, "ty", -0.2)]),
    spec(
        "fc.clip.in.spin",
        true,
        &[(0.0, "rot", -180.0), (0.0, "sx", 0.5)],
    ),
    spec(
        "fc.clip.in.swing",
        true,
        &[(0.0, "rot", -60.0), (0.6, "rot", 7.2)],
    ),
    spec(
        "fc.clip.in.blur",
        true,
        &[
            (0.0, "blur", 0.02),
            (0.0, "opacity", 0.0),
            (0.5, "opacity", 1.0),
        ],
    ),
    spec("fc.clip.in.wipe", false, &[(0.0, "reveal", 0.0)]),
    // From the left by default: about the vertical axis.
    spec(
        "fc.clip.in.flip",
        false,
        &[
            (0.0, "turn_y", 90.0),
            (0.0, "turn_x", 0.0),
            (1.0, "turn_y", 0.0),
        ],
    ),
    // ±0.03 se at 12 Hz over the default 0.5 s, dying away.
    spec(
        "fc.clip.in.shake",
        true,
        &[
            (0.0, "tx", 0.0),
            (0.125, "tx", -0.03 * 0.875),
            (0.375, "tx", 0.03 * 0.625),
            (0.0, "opacity", 0.0),
            (0.2, "opacity", 1.0),
        ],
    ),
    spec(
        "fc.clip.in.pop",
        true,
        &[(0.0, "sx", 0.0), (0.6, "sx", 1.15)],
    ),
    spec(
        "fc.clip.in.stretch",
        true,
        &[(0.0, "sx", 1.6), (0.0, "sy", 0.6)],
    ),
    spec(
        "fc.clip.in.flash",
        true,
        &[
            (0.0, "tint", 1.0),
            (0.0, "opacity", 0.0),
            (0.1, "opacity", 1.0),
            (0.4, "tint", 0.0),
        ],
    ),
    spec(
        "fc.clip.in.squash",
        true,
        &[
            (0.0, "sy", 0.0),
            (0.4, "sy", 1.25),
            (0.4, "sx", 0.8),
            (0.7, "sy", 0.85),
            (0.7, "sx", 1.1),
        ],
    ),
    // FX-030, continuous.
    spec(
        "fc.clip.loop.pendulum",
        true,
        &[(0.25, "rot", 8.0), (0.75, "rot", -8.0)],
    ),
    spec(
        "fc.clip.loop.spin",
        false,
        &[(0.25, "rot", 90.0), (0.5, "rot", 180.0)],
    ),
    spec(
        "fc.clip.loop.bounce",
        true,
        &[(0.0, "ty", 0.0), (0.5, "ty", -0.05)],
    ),
    spec(
        "fc.clip.loop.heartbeat",
        true,
        &[(0.1, "sx", 1.08), (0.3, "sx", 1.05), (0.4, "sx", 1.0)],
    ),
    spec(
        "fc.clip.loop.float",
        true,
        &[(0.25, "ty", -0.02), (0.75, "ty", 0.02)],
    ),
    spec("fc.clip.loop.jitter", true, &[(0.5, "opacity", 1.0)]),
    // A quarter cycle apart: the turn leads the zoom.
    spec(
        "fc.clip.loop.rotate-zoom",
        true,
        &[
            (0.25, "rot", 6.0),
            (0.0, "sx", 0.95),
            (0.5, "sx", 1.05),
            (0.25, "sx", 1.0),
        ],
    ),
    spec(
        "fc.clip.drift.zoom-in",
        true,
        &[(0.0, "sx", 1.0), (0.5, "sx", 1.05), (1.0, "sx", 1.1)],
    ),
    spec(
        "fc.clip.drift.zoom-out",
        true,
        &[(0.0, "sx", 1.1), (1.0, "sx", 1.0)],
    ),
    spec(
        "fc.clip.drift.pan",
        true,
        &[(0.0, "tx", 0.0), (0.5, "tx", 0.025), (1.0, "tx", 0.05)],
    ),
    // Combined: entrance over the first 30 %, exit over the last 30 %.
    spec(
        "fc.clip.combined.zoom-through",
        true,
        &[
            (0.0, "sx", 0.6),
            (0.3, "sx", 1.0),
            (0.7, "sx", 1.1),
            (1.0, "sx", 1.5),
            (0.0, "opacity", 0.0),
            (0.12, "opacity", 1.0),
            (0.88, "opacity", 1.0),
            (1.0, "opacity", 0.0),
        ],
    ),
    spec(
        "fc.clip.combined.spin-through",
        true,
        &[
            (0.0, "rot", -180.0),
            (0.3, "rot", 0.0),
            (0.7, "rot", 360.0),
            (1.0, "rot", 540.0),
            (0.0, "sx", 0.5),
            (0.5, "sx", 1.0),
            (1.0, "sx", 0.5),
        ],
    ),
    spec(
        "fc.clip.combined.slide-through",
        false,
        &[(0.0, "tx", -1.0), (0.3, "tx", 0.0), (1.0, "tx", 1.1)],
    ),
    spec(
        "fc.clip.combined.pop-fade",
        true,
        &[
            (0.0, "sx", 0.0),
            (0.18, "sx", 1.15),
            (0.5, "sx", 1.0),
            (0.5, "opacity", 1.0),
            (1.0, "opacity", 0.0),
        ],
    ),
];

#[test]
fn every_preset_has_its_spec_entry() {
    let spec: BTreeSet<&str> = SPEC.iter().map(|s| s.id).collect();
    assert_eq!(spec.len(), SPEC.len(), "a preset is listed twice");
    let loaded: BTreeSet<String> = available().iter().map(|m| m.id.to_string()).collect();
    let spec_ids: BTreeSet<String> = spec.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(loaded, spec_ids);
    // Every bundled file loads: no preset waits for an engine channel.
    let in_files: BTreeSet<String> = files()
        .into_iter()
        .map(|(_, v)| v["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(in_files, loaded);
    assert_eq!(loaded.len(), BUNDLED.len());
}

#[test]
fn every_preset_moves_as_the_spec_says() {
    let all = available();
    for s in SPEC {
        let m = all.iter().find(|m| m.id.as_str() == s.id).unwrap();
        for &(p, name, want) in s.at {
            let got = field(&pose(m, main_slot(m), p, 500), name);
            assert!(
                same(name, got, want, 1e-6),
                "{} at {p}: {name} is {got}, the spec says {want}",
                s.id
            );
        }
    }
}

#[test]
fn intensity_is_offered_exactly_where_the_spec_gives_it() {
    for s in SPEC {
        let m = available()
            .into_iter()
            .find(|m| m.id.as_str() == s.id)
            .unwrap();
        let has = m.channels.iter().any(|c| c.scales_with_intensity);
        assert_eq!(has, s.intensity, "{}", s.id);
    }
}

// ---------------------------------------------------------------------------
// Golden samples
// ---------------------------------------------------------------------------

/// One golden line: the fields of `o` that are not at rest.
fn golden_line(head: String, o: &BlockOffsets) -> String {
    let rest = fields(&BlockOffsets::REST);
    let mut line = head;
    let mut any = false;
    for ((name, v), r) in FIELDS.iter().zip(fields(o)).zip(rest) {
        let v = format!("{v:.6}");
        let v = if v == "-0.000000" {
            "0.000000".into()
        } else {
            v
        };
        if v != format!("{r:.6}") {
            write!(line, " {name}={v}").unwrap();
            any = true;
        }
    }
    if !any {
        line.push_str(" rest");
    }
    line.push('\n');
    line
}

fn golden_text(manifests: &[PresetManifest]) -> String {
    let mut out = String::from(
        "# Preset poses at 0/25/50/75/100 % of the slot, Intensity 50, seed 0, lengths in\n\
         # each channel's unit (em, se, degrees, travel distances). Fields at rest are left\n\
         # out. Presets with seeded or line-relative channels add units 1 and 2 of a line\n\
         # 1.5 em high whose centre is (1.5 - unit, 0.25) em away, and seeded loops that\n\
         # move only at moments (Flicker, Sparkle) add when in the cycle each moment falls.\n",
    );
    for m in manifests {
        for (slot, label) in plays(m) {
            for p in SAMPLES {
                let head = format!("{} {label} {:>3}%:", m.id, (p * 100.0).round());
                out.push_str(&golden_line(head, &pose(m, slot, p, 500)));
            }
            if !varies_per_unit(m) {
                continue;
            }
            for unit in [1, 2] {
                for p in SAMPLES {
                    let head = format!("{} {label} unit {unit} {:>3}%:", m.id, (p * 100.0).round());
                    let o = pose_of(m, slot, p, 500, 0, &sample_unit(unit));
                    out.push_str(&golden_line(head, &o));
                }
            }
            if m.kind == PresetKind::Loop {
                for unit in [0, 1, 2] {
                    out.push_str(&golden_moments(m, unit));
                }
            }
        }
    }
    out
}

/// A seeded loop that moves only at moments (Flicker's dips, Sparkle's
/// swells), which the 25 % samples miss: where in the cycle each moment
/// falls, to the millisecond, how long it lasts, and the pose at its
/// middle. Nothing for a loop that always moves.
fn golden_moments(m: &PresetManifest, unit: u32) -> String {
    let ms = (m.default_duration.as_secs_f64() * 1000.0).round() as usize;
    let at = |i: usize| {
        let p = i as f64 / ms as f64;
        pose_of(m, Slot::Continuous, p, 500, 0, &sample_unit(unit))
    };
    let moving: Vec<bool> = (0..ms).map(|i| !at(i).is_rest(1e-9)).collect();
    if moving.iter().all(|&b| b) || !moving.iter().any(|&b| b) {
        return String::new();
    }
    let mut out = String::new();
    for start in (0..ms).filter(|&i| moving[i] && !moving[(i + ms - 1) % ms]) {
        let len = (0..ms).take_while(|&j| moving[(start + j) % ms]).count();
        let head = format!(
            "{} loop unit {unit} moves at {start} ms for {len} ms, middle:",
            m.id
        );
        out.push_str(&golden_line(head, &at((start + len / 2) % ms)));
    }
    out
}

#[test]
fn every_preset_matches_its_golden_samples() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/presets.txt");
    let got = golden_text(&available());
    if std::env::var_os("FREECUT_BLESS_GOLDEN").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).expect(
        "missing golden file; run with FREECUT_BLESS_GOLDEN=1 to create it, then commit it",
    );
    assert_eq!(got, want);
}

// ---------------------------------------------------------------------------
// Rest rules
// ---------------------------------------------------------------------------

/// The rest rules of FX-030 and TX-056 for one manifest, at Intensity 0,
/// 50 and 100.
fn check_rest_rules(m: &PresetManifest) {
    for intensity in [0, 500, 1000] {
        let at = |slot, p| pose(m, slot, p, intensity);
        let what = |s: &str| format!("{} at intensity {intensity}: {s}", m.id);
        // Finite everywhere.
        for (slot, _) in plays(m) {
            for i in 0..=200 {
                let o = at(slot, f64::from(i) / 200.0);
                assert!(fields(&o).iter().all(|v| v.is_finite()), "{}", what("NaN"));
            }
        }
        match m.kind {
            PresetKind::Entrance => {
                assert!(
                    at(Slot::Entrance, 1.0).is_rest(1e-9),
                    "{}",
                    what("In ends at rest")
                );
                assert!(
                    at(Slot::Exit, 0.0).is_rest(1e-9),
                    "{}",
                    what("Out starts at rest")
                );
                // Out is In backwards (from the opposite side by default:
                // slides, rotations and 3D turns mirror).
                let back = at(Slot::Exit, 1.0);
                let start = at(Slot::Entrance, 0.0);
                for ((name, b), s) in FIELDS.iter().zip(fields(&back)).zip(fields(&start)) {
                    let mirrored = match *name {
                        "tx" | "rot" | "turn_x" | "turn_y" if m.directions.is_some() => -s,
                        _ => s,
                    };
                    assert!(same(name, b, mirrored, 1e-9), "{}", what(name));
                }
            }
            PresetKind::Exit => {
                assert!(
                    at(Slot::Exit, 0.0).is_rest(1e-9),
                    "{}",
                    what("Out starts at rest")
                );
            }
            PresetKind::Loop => {
                assert_same_pose(
                    &at(Slot::Continuous, 0.0),
                    &at(Slot::Continuous, 1.0),
                    1e-9,
                    &what("periodic"),
                );
                // No jump where one cycle meets the next.
                assert_same_pose(
                    &at(Slot::Continuous, 0.0),
                    &at(Slot::Continuous, 1.0 - 1e-7),
                    1e-3,
                    &what("seamless"),
                );
            }
            PresetKind::Drift | PresetKind::Combined => {}
        }
    }
    // At the default amplitude every preset visibly moves.
    let (slot, _) = plays(m)[0];
    assert!(
        (0..=20).any(|i| !pose(m, slot, f64::from(i) / 20.0, 500).is_rest(1e-6)),
        "{} never moves",
        m.id
    );
}

#[test]
fn in_ends_at_rest_out_starts_at_rest_and_loops_repeat() {
    for m in available() {
        check_rest_rules(&m);
    }
}

#[test]
fn every_seed_and_unit_follows_the_rest_rules() {
    for m in available().iter().filter(|m| varies_per_unit(m)) {
        for seed in SEEDS {
            for index in UNIT_INDEXES {
                let unit = sample_unit(index);
                let at = |slot, p| pose_of(m, slot, p, 1000, seed, &unit);
                let what = format!("{} seed {seed} unit {index}", m.id);
                match m.kind {
                    PresetKind::Entrance => {
                        assert!(at(Slot::Entrance, 1.0).is_rest(1e-9), "{what}: In");
                        assert!(at(Slot::Exit, 0.0).is_rest(1e-9), "{what}: Out");
                    }
                    PresetKind::Exit => {
                        assert!(at(Slot::Exit, 0.0).is_rest(1e-9), "{what}: Out");
                    }
                    PresetKind::Loop => {
                        let (a, b) = (at(Slot::Continuous, 0.0), at(Slot::Continuous, 1.0));
                        assert_same_pose(&a, &b, 1e-9, &what);
                        let c = at(Slot::Continuous, 1.0 - 1e-7);
                        assert_same_pose(&a, &c, 1e-3, &what);
                    }
                    PresetKind::Drift | PresetKind::Combined => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Seeded and line-relative presets (TX-056, FX-030)
// ---------------------------------------------------------------------------

fn manifest(id: &str) -> PresetManifest {
    available()
        .into_iter()
        .find(|m| m.id.as_str() == id)
        .unwrap_or_else(|| panic!("{id} is not bundled"))
}

/// `f` of every pose of one cycle of a loop, sampled each millisecond,
/// for every seed and unit tried.
fn over_cycles(m: &PresetManifest, mut f: impl FnMut(u32, u32, &[(f64, BlockOffsets)])) {
    let secs = m.default_duration.as_secs_f64();
    let n = (secs * 1000.0).round() as u32;
    for seed in SEEDS {
        for index in UNIT_INDEXES {
            let poses: Vec<(f64, BlockOffsets)> = (0..n)
                .map(|i| {
                    let p = f64::from(i) / f64::from(n);
                    let o = pose_of(m, Slot::Continuous, p, 500, seed, &sample_unit(index));
                    (p * secs, o)
                })
                .collect();
            f(seed, index, &poses);
        }
    }
}

/// How often `values` turns from rising to falling or back (cyclic).
fn turns(values: &[f64]) -> usize {
    let n = values.len();
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (values[(i + n - 1) % n], values[i], values[(i + 1) % n]);
            (b - a) * (c - b) < 0.0
        })
        .count()
}

#[test]
fn scatter_and_fall_away_draw_a_seeded_offset_and_turn_per_unit() {
    let scatter = manifest("fc.in.scatter");
    let fall = manifest("fc.out.fall-away");
    let mut offsets = BTreeSet::new();
    for seed in SEEDS {
        for index in 0..24 {
            let unit = sample_unit(index);
            // Scatter: 0.5–1 × 1.5·k em away (the line doesn't matter),
            // turned within ±40·k°, transparent.
            let o = pose_of(&scatter, Slot::Entrance, 0.0, 500, seed, &unit);
            let r = o.translate.length();
            assert!(
                (0.75 - 1e-9..=1.5 + 1e-9).contains(&r),
                "scatter offset {r}"
            );
            assert!(o.rotation.abs() <= 40.0, "scatter turn {}", o.rotation);
            assert_eq!(o.opacity, 0.0);
            offsets.insert(format!("{:.4} {:.4}", o.translate.x, o.translate.y));
            // Same unit, same seed: the same motion, whatever the line.
            let block = UnitContext {
                index,
                ..UnitContext::BLOCK
            };
            let again = pose_of(&scatter, Slot::Entrance, 0.0, 500, seed, &block);
            assert_same_pose(&o, &again, 1e-12, "scatter ignores the line");
            // Fall away: straight down 2·k em, turned within ±30·k°.
            let o = pose_of(&fall, Slot::Exit, 1.0, 500, seed, &unit);
            assert!((o.translate.x).abs() < 1e-12);
            assert!((o.translate.y - 2.0).abs() < 1e-12);
            assert!(o.rotation.abs() <= 30.0, "fall-away turn {}", o.rotation);
            // Intensity 100 doubles both.
            let o2 = pose_of(&fall, Slot::Exit, 1.0, 1000, seed, &unit);
            assert!((o2.rotation - 2.0 * o.rotation).abs() < 1e-9);
        }
    }
    // Every unit draws its own.
    assert!(offsets.len() > 100, "{} distinct offsets", offsets.len());
}

#[test]
fn spread_and_word_rise_follow_the_line() {
    let spread = manifest("fc.in.spread");
    let unit = UnitContext {
        to_line_center: DVec2::new(3.0, -0.5),
        ..UnitContext::BLOCK
    };
    // At (1 − 0.6·k) of its distance from the line's centre: moved 0.6·k
    // of the way toward it.
    for (intensity, share) in [(500, 0.6), (1000, 1.2), (0, 0.0)] {
        let o = pose_of(&spread, Slot::Entrance, 0.0, intensity, 0, &unit);
        assert!(
            o.translate
                .abs_diff_eq(DVec2::new(3.0, -0.5) * share, 1e-12)
        );
    }
    let half = pose_of(&spread, Slot::Entrance, 0.5, 500, 0, &unit);
    assert!(half.translate.x > 0.0 && half.translate.x < 1.8);
    let rise = manifest("fc.in.word-rise");
    let unit = UnitContext {
        line_height: Some(1.5),
        ..UnitContext::BLOCK
    };
    let start = pose_of(&rise, Slot::Entrance, 0.0, 500, 0, &unit);
    assert!((start.translate.y - 1.5).abs() < 1e-12);
    assert!(start.line_clip);
    // Rises through its slot: below the line all the way, clipped.
    for i in 1..20 {
        let o = pose_of(&rise, Slot::Entrance, f64::from(i) / 20.0, 500, 0, &unit);
        assert!(o.translate.y > 0.0 && o.translate.y < 1.5 && o.line_clip);
    }
    assert!(!pose_of(&rise, Slot::Entrance, 1.0, 500, 0, &unit).line_clip);
}

#[test]
fn jitter_presets_stay_within_their_amplitude_and_turn_12_times_a_second() {
    // (preset, x/y amplitude, turn amplitude)
    for (id, shift, turn) in [
        ("fc.loop.shake", 0.06, 0.0),
        ("fc.loop.wiggle", 0.0, 6.0),
        ("fc.clip.loop.jitter", 0.01, 1.5),
    ] {
        let m = manifest(id);
        let per_second = 12.0 * m.default_duration.as_secs_f64();
        over_cycles(&m, |seed, index, poses| {
            let what = format!("{id} seed {seed} unit {index}");
            let xs: Vec<f64> = poses.iter().map(|(_, o)| o.translate.x).collect();
            let ys: Vec<f64> = poses.iter().map(|(_, o)| o.translate.y).collect();
            let rs: Vec<f64> = poses.iter().map(|(_, o)| o.rotation).collect();
            for (values, amp) in [(&xs, shift), (&ys, shift), (&rs, turn)] {
                let most = values.iter().fold(0.0f64, |a, v| a.max(v.abs()));
                assert!(most <= amp + 1e-12, "{what}: {most} past {amp}");
                if amp == 0.0 {
                    continue;
                }
                // Knots of 0.4–1 × the amplitude, so it visibly moves.
                assert!(most >= 0.4 * amp - 1e-9, "{what}: only {most}");
                let t = turns(values) as f64;
                assert!(
                    (t - per_second).abs() <= 1.0,
                    "{what}: {t} turns, want {per_second}"
                );
            }
            if shift > 0.0 {
                // Wanders in 2D, not along a diagonal.
                assert!(xs.iter().zip(&ys).any(|(x, y)| (x - y).abs() > 0.1 * shift));
                assert!(xs.iter().zip(&ys).any(|(x, y)| (x + y).abs() > 0.1 * shift));
            }
            // Nothing else moves.
            for (_, o) in poses {
                assert_eq!(o.opacity, 1.0, "{what}");
                assert!(o.scale.abs_diff_eq(DVec2::ONE, 1e-12), "{what}");
            }
        });
    }
}

#[test]
fn flicker_dips_once_a_second_for_two_frames() {
    let m = manifest("fc.loop.flicker");
    over_cycles(&m, |seed, index, poses| {
        let what = format!("seed {seed} unit {index}");
        let low: Vec<bool> = poses.iter().map(|(_, o)| o.opacity < 1.0).collect();
        // Dips to 1 − 0.6·k = 0.4, nothing in between.
        for (_, o) in poses {
            assert!(
                o.opacity == 1.0 || (o.opacity - 0.4).abs() < 1e-12,
                "{what}: {}",
                o.opacity
            );
        }
        // One dip per 1 s cycle, counted cyclically, 2 frames at 30 fps.
        let n = low.len();
        let starts = (0..n).filter(|&i| low[i] && !low[(i + n - 1) % n]).count();
        assert_eq!(starts, 1, "{what}");
        let ms = low.iter().filter(|&&l| l).count();
        assert!((66..=68).contains(&ms), "{what}: {ms} ms");
    });
    // With the sequence's frame rate: 2 frames at 25 fps.
    let unit = UnitContext {
        frame: Some(freecut_timeline::time::Tick::from_raw(
            TICKS_PER_SECOND / 25,
        )),
        ..UnitContext::BLOCK
    };
    let ms = (0..1000)
        .filter(|&i| {
            let p = f64::from(i) / 1000.0;
            pose_of(&m, Slot::Continuous, p, 500, 0, &unit).opacity < 1.0
        })
        .count();
    assert!((79..=81).contains(&ms), "{ms} ms at 25 fps");
}

#[test]
fn sparkle_swells_once_a_cycle_for_a_fifth_of_a_second() {
    let m = manifest("fc.loop.sparkle");
    let mut moments = BTreeSet::new();
    over_cycles(&m, |seed, index, poses| {
        let what = format!("seed {seed} unit {index}");
        let s: Vec<f64> = poses.iter().map(|(_, o)| o.scale.x).collect();
        let most = s.iter().fold(1.0f64, |a, &v| a.max(v));
        // Up to 1 + 0.2·k, sampled within a millisecond of the peak.
        assert!(most <= 1.2 + 1e-12 && most > 1.2 - 1e-3, "{what}: {most}");
        let up = s.iter().filter(|&&v| v > 1.0 + 1e-12).count();
        assert!((198..=200).contains(&up), "{what}: {up} ms");
        let n = s.len();
        let starts = (0..n)
            .filter(|&i| s[i] > 1.0 && s[(i + n - 1) % n] <= 1.0)
            .count();
        assert_eq!(starts, 1, "{what}");
        let peak = s.iter().position(|&v| v == most).unwrap_or(0);
        moments.insert(peak);
        for (_, o) in poses {
            assert_eq!(o.scale.x, o.scale.y, "{what}");
        }
    });
    // Seeded moments: units don't swell together.
    assert!(moments.len() > 10, "{} moments", moments.len());
}

#[test]
fn the_3d_flip_turns_about_the_axis_its_side_gives() {
    let m = manifest("fc.clip.in.flip");
    // The side the clip comes from: turned 90° about the vertical axis
    // from the left or right, the horizontal one from the top or bottom.
    for (dir, want) in [
        (Direction::Left, (0.0, 90.0)),
        (Direction::Right, (0.0, -90.0)),
        (Direction::Up, (90.0, 0.0)),
        (Direction::Down, (-90.0, 0.0)),
    ] {
        let mut spec = m.default_spec();
        spec.direction = Some(dir);
        let o = pose_with(&m, &spec, Slot::Entrance, 0.0, &UNITS, &UnitContext::BLOCK);
        assert_eq!((o.turn.x, o.turn.y), want, "{dir:?}");
        assert!(o.has_turn());
        let end = pose_with(&m, &spec, Slot::Entrance, 1.0, &UNITS, &UnitContext::BLOCK);
        assert!(end.is_rest(1e-12));
        // Its Out leaves toward the side it names.
        let out = pose_with(&m, &spec, Slot::Exit, 1.0, &UNITS, &UnitContext::BLOCK);
        assert_eq!((out.turn.x, out.turn.y), want, "out {dir:?}");
    }
}

// ---------------------------------------------------------------------------
// Intensity
// ---------------------------------------------------------------------------

/// A field's offset from rest: added for positions, angles, blur, tint and
/// style; `factor − 1` for scales, opacity, reveal and glow.
fn offset(name: &str, v: f64) -> f64 {
    match name {
        "sx" | "sy" | "opacity" | "reveal" | "glow" => v - 1.0,
        _ => v,
    }
}

/// Whether the engine clamped a field (opacity, reveal and tint to 0–1,
/// scales, blur and glow at 0), so it says nothing about scaling.
fn clamped(name: &str, v: f64) -> bool {
    match name {
        "opacity" | "reveal" | "tint" => v <= 1e-12 || v >= 1.0 - 1e-12,
        "sx" | "sy" | "blur" | "glow" => v <= 1e-12,
        _ => false,
    }
}

#[test]
fn intensity_scales_flagged_channels_and_nothing_else() {
    for m in available() {
        for (i, ch) in m.channels.iter().enumerate() {
            let alone = PresetManifest {
                channels: vec![ch.clone()],
                ..m.clone()
            };
            for (slot, _) in plays(&alone) {
                for step in 0..=20 {
                    let p = f64::from(step) / 20.0;
                    let at = |k| fields(&pose(&alone, slot, p, k));
                    let (k0, k1, k2) = (at(0), at(500), at(1000));
                    for (j, name) in FIELDS.iter().enumerate() {
                        if [k0[j], k1[j], k2[j]].iter().any(|&v| clamped(name, v)) {
                            continue;
                        }
                        let (a0, a1, a2) = (
                            offset(name, k0[j]),
                            offset(name, k1[j]),
                            offset(name, k2[j]),
                        );
                        let what = format!("{} channel {i} {name} at {p}", m.id);
                        if ch.scales_with_intensity {
                            assert!(a0.abs() < 1e-9, "{what}: moves at intensity 0");
                            assert!((a2 - 2.0 * a1).abs() < 1e-9, "{what}: {a1} → {a2}");
                        } else {
                            assert!((a0 - a1).abs() < 1e-9 && (a2 - a1).abs() < 1e-9, "{what}");
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Photosensitivity (FX-030, TX-056)
// ---------------------------------------------------------------------------

/// Brightness signals of a pose, each 0–1: the layer's opacity (light
/// content on a dark background), a white tint over dark content, a fill
/// lightened toward white, and the glow's strength.
fn brightness(o: &BlockOffsets) -> [f64; 4] {
    let a = o.opacity;
    [
        a,
        a * o.tint,
        a * o.lightness.clamp(0.0, 1.0),
        a * (o.glow / 2.0).clamp(0.0, 1.0),
    ]
}

/// A swing of at least 10 % of the full range counts as a sharp change
/// (the three-flash guideline's threshold).
const SWING: f64 = 0.1;

/// The times of sharp changes in `samples`, each with its sign.
fn changes(samples: &[(f64, f64)]) -> Vec<(f64, i8)> {
    let mut out = Vec::new();
    let Some(&(_, first)) = samples.first() else {
        return out;
    };
    let (mut dir, mut ext) = (0i8, first);
    for &(t, v) in samples {
        let turned = match dir {
            0 => (v - ext).abs() >= SWING,
            1 => ext - v >= SWING,
            _ => v - ext >= SWING,
        };
        if turned {
            dir = if v > ext { 1 } else { -1 };
            out.push((t, dir));
            ext = v;
        } else if (dir == 1 && v > ext) || (dir == -1 && v < ext) {
            ext = v;
        }
    }
    out
}

/// The most entries of `times` inside any one second.
fn most_in_a_second(times: &[f64]) -> usize {
    (0..times.len())
        .map(|i| {
            times[i..]
                .iter()
                .take_while(|&&t| t < times[i] + 1.0)
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// Every way a preset plays, as (slot, seconds of slot progress 0→1, how
/// long to watch): In and Out at their default length and at FX-018's
/// 0.1 s minimum (everything inside one second); loops for three seconds
/// of cycles; drifts and combined presets over their default span.
fn timings(m: &PresetManifest) -> Vec<(Slot, f64, f64)> {
    let d = m.default_duration.as_secs_f64();
    match m.kind {
        PresetKind::Entrance | PresetKind::Exit => plays(m)
            .into_iter()
            .flat_map(|(slot, _)| [(slot, d, d), (slot, 0.1, 0.1)])
            .collect(),
        PresetKind::Loop => vec![(Slot::Continuous, d, 3.0)],
        PresetKind::Drift | PresetKind::Combined => vec![(Slot::Continuous, d, d)],
    }
}

/// Fails when a preset changes brightness or opacity sharply more than 3
/// times in any second, or, for loops, dips more than twice in a second
/// (FX-030's photosensitivity rule, TX-056's design rules), at Intensity
/// 0, 50 and 100.
fn assert_photosafe(m: &PresetManifest) {
    // Seeded presets: several seeds and units, as the dice may give them.
    let tries: Vec<(u32, u32)> = if varies_per_unit(m) {
        SEEDS
            .iter()
            .flat_map(|&s| UNIT_INDEXES.iter().map(move |&u| (s, u)))
            .collect()
    } else {
        vec![(0, 0)]
    };
    for (seed, index) in tries {
        assert_photosafe_unit(m, seed, &sample_unit(index));
    }
    let flashes = max_flashes_per_second(m);
    let limit = if m.kind == PresetKind::Loop { 2 } else { 3 };
    assert!(flashes <= limit, "{}: {flashes} flashes per second", m.id);
    // The engine's own survey (what the grids ask) agrees.
    let survey = photosensitivity(m);
    assert!(survey.is_safe(), "{}: {survey:?}", m.id);
    assert!(is_photosafe(m), "{}", m.id);
}

fn assert_photosafe_unit(m: &PresetManifest, seed: u32, unit: &UnitContext) {
    const RATE: f64 = 240.0;
    for intensity in [0, 500, 1000] {
        for (slot, length, watch) in timings(m) {
            let n = (watch * RATE).ceil() as u32;
            let poses: Vec<(f64, [f64; 4])> = (0..=n)
                .map(|i| {
                    let t = f64::from(i) / RATE;
                    let p = if m.kind == PresetKind::Loop {
                        (t / length).fract()
                    } else {
                        (t / length).min(1.0)
                    };
                    (t, brightness(&pose_of(m, slot, p, intensity, seed, unit)))
                })
                .collect();
            for s in 0..4 {
                let signal: Vec<(f64, f64)> = poses.iter().map(|(t, b)| (*t, b[s])).collect();
                let found = changes(&signal);
                let times: Vec<f64> = found.iter().map(|c| c.0).collect();
                let sharp = most_in_a_second(&times);
                assert!(
                    sharp <= 3,
                    "{} ({slot:?}, {length} s, intensity {intensity}, seed {seed}, unit {}): \
                     {sharp} sharp changes of brightness signal {s} in one second",
                    m.id,
                    unit.index
                );
                if m.kind == PresetKind::Loop {
                    // A dip: a fall followed by a rise.
                    let dips: Vec<f64> = found
                        .windows(2)
                        .filter(|w| w[0].1 < 0 && w[1].1 > 0)
                        .map(|w| w[0].0)
                        .collect();
                    let worst = most_in_a_second(&dips);
                    assert!(worst <= 2, "{}: {worst} dips in one second", m.id);
                }
            }
        }
    }
}

#[test]
fn no_preset_flashes() {
    let all = available();
    assert_eq!(all.len(), BUNDLED.len());
    for m in &all {
        assert_photosafe(m);
    }
    // So every bundled preset is offered.
    let table = bundled().table;
    for m in &all {
        assert_eq!(table.availability(&m.id), Availability::Ready, "{}", m.id);
    }
    assert_eq!(table.safe_manifests().count(), BUNDLED.len());
}

#[test]
fn no_loop_flashes_at_any_cycle_the_user_can_set() {
    use freecut_timeline::model::AnimationSpec;
    // FX-019's Cycle goes down to 0.1 s; a loop plays no faster than its
    // shortest safe cycle (FX-030), whatever is stored.
    let table = bundled().table;
    let ms = |v: i64| Tick::from_raw(TICKS_PER_SECOND / 1000 * v);
    for m in available().iter().filter(|m| m.kind == PresetKind::Loop) {
        let floor = table.min_cycle(&m.id);
        assert!(
            floor >= AnimationSpec::MIN_DURATION && floor <= m.default_duration,
            "{}: {floor:?}",
            m.id
        );
        for asked in [AnimationSpec::MIN_DURATION, ms(250), ms(500), ms(1000)] {
            let mut fast = m.clone();
            fast.default_duration = asked.max(floor);
            let report = photosensitivity(&fast);
            assert!(report.is_safe(), "{} at {asked:?}: {report:?}", m.id);
        }
    }
    // The brightness loops need their floor: at 0.1 s they would strobe.
    for id in ["fc.loop.breathing-glow", "fc.loop.shimmer"] {
        let mut m = manifest(id);
        m.default_duration = AnimationSpec::MIN_DURATION;
        assert!(!photosensitivity(&m).is_safe(), "{id}");
        assert!(table.min_cycle(&m.id) > AnimationSpec::MIN_DURATION, "{id}");
    }
}

#[test]
fn the_photosensitivity_check_catches_a_strobe() {
    // A made-up loop that blinks four times a second: refused.
    let blink = Curve::sampled(
        [(0.0, 0.0), (0.4, 0.0), (0.5, 1.0), (0.9, 1.0), (1.0, 0.0)]
            .into_iter()
            .map(|(t, v)| CurvePoint { t, v })
            .collect(),
        false,
    )
    .unwrap();
    let mut strobe = available()
        .into_iter()
        .find(|m| m.id.as_str() == "fc.loop.pulse")
        .unwrap();
    strobe.channels = vec![Channel {
        property: ChannelProp::Opacity,
        curve: blink,
        amplitude: -1.0,
        unit: ChannelUnit::Ratio,
        scales_with_intensity: false,
    }];
    strobe.default_duration = Tick::from_raw(TICKS_PER_SECOND / 4);
    let fast = strobe.clone();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_photosafe_unit(&fast, 0, &UnitContext::BLOCK);
    }));
    assert!(caught.is_err());
    assert!(!is_photosafe(&fast));
    // The same blink once every two seconds passes.
    strobe.default_duration = Tick::from_raw(TICKS_PER_SECOND * 2);
    assert_photosafe(&strobe);
}

#[test]
fn every_entrance_has_its_exit_by_name() {
    let table = bundled().table;
    let mut entrances = 0;
    for m in available() {
        match m.kind {
            PresetKind::Entrance => {
                // `fc.in.x` plays as `fc.out.x`, `fc.clip.in.x` as
                // `fc.clip.out.x`, and is no file of its own.
                let out = PresetId::new(m.id.as_str().replacen(".in.", ".out.", 1));
                assert_ne!(out, m.id);
                assert!(table.get(&out).is_none(), "{out} shadows {}", m.id);
                let played = table.resolve(Slot::Exit, &out).unwrap();
                assert_eq!(played.id, m.id);
                assert!(table.resolve(Slot::Entrance, &out).is_none());
                entrances += 1;
            }
            // Out-only presets (Erase, Fall away) play by their own id.
            PresetKind::Exit => {
                let played = table.resolve(Slot::Exit, &m.id).unwrap();
                assert_eq!(played.id, m.id);
            }
            _ => {}
        }
    }
    // Including Scatter, Spread, Word rise and the 3D flip.
    assert_eq!(entrances, 42);
}
