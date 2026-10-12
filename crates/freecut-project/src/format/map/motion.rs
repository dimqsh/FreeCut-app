//! Keyframes and animations (format 6; TR-027, TR-031, TX-021, FX-018,
//! FX-019, TX-022): DTOs ⇄ `ClipKeys` and `ClipAnimations`.
//!
//! Reading refuses what can't be a value: a property that takes no keys or
//! that the clip has no place for (text keys on a video clip), a property
//! listed twice, two keys at one time, a value of the wrong shape, an
//! animation on an audio clip, a Loop beside In or Out on a clip whose kind
//! plays them exclusively (FX-019), a duration that isn't positive.
//! Numbers are only brought into range, as an edit would bring them:
//! beyond the model's integer width they are saturated, key values are
//! clamped into their property's range, angles wrapped, overlap and
//! intensity capped at 1000, Pace held within 0.02–1 s, a `hold` easing
//! override dropped, an In or Out raised to 0.1 s and a Loop's cycle held
//! within 0.1–10 s (FX-018, FX-019), a seed saturated into 32 bits and a
//! split Loop's ramp held within 0–250 ms. Each of these sets `clamped`
//! (reported as a repair);
//! keys out of time order are sorted and reported like volume keyframes.
//!
//! Unknown keys: the `keys` and `animations` objects and each animation
//! slot are filed by path; a key track by its property
//! (`keys[text.size]`), a key by its property and time
//! (`keys[text.size]@<ticks>`) and a point value under that
//! (`…@<ticks>.value`), so they stay with their key when an earlier one is
//! added or removed, and go when it is moved or deleted. A slot's unknown
//! keys stay with the slot when another preset is chosen in it.

use std::collections::BTreeSet;
use std::sync::Arc;

use freecut_timeline::model::{
    Angle, AnimOrder, AnimParams, AnimUnit, AnimationSpec, CanvasPos, Clip, ClipAnimations,
    ClipContent, ClipKeys, DeciDegrees, Direction, EaseCurve, EaseDir, Interp, Keyframe, LoopSeams,
    PerMille, PresetId, PropPath, PropValue, RefPx, Scale, SignedPerMille, Slot, SlotPolicy,
    TailTip, Travel, ValueKind, slot_policy,
};
use freecut_timeline::time::Tick;

use super::{At, Give, color_text, decode_at, fit, json_kind, parse_color, tick};
use crate::extras::{Extras, Fields, Owner};
use crate::format::dto::{
    AnimOrderDto, AnimUnitDto, AnimationSpecDto, ClipAnimationsDto, ClipKeysDto, DirectionDto,
    InterpDto, KeyTrackDto, KeyValueDto, LoopSeamsDto, PointDto, PropKeyDto, TravelDto,
};
use crate::open::OpenError;

// ---------------------------------------------------------------------------
// Shared enums (also used by the preset manifests)
// ---------------------------------------------------------------------------

pub(crate) fn interp(dto: InterpDto) -> Interp {
    use EaseCurve::{Cubic, Expo, Quad, Sine};
    use EaseDir::{In, InOut, Out};
    let ease = |curve, dir| Interp::Ease { curve, dir };
    match dto {
        InterpDto::Linear => Interp::Linear,
        InterpDto::Hold => Interp::Hold,
        InterpDto::EaseInSine => ease(Sine, In),
        InterpDto::EaseOutSine => ease(Sine, Out),
        InterpDto::EaseInOutSine => ease(Sine, InOut),
        InterpDto::EaseInQuad => ease(Quad, In),
        InterpDto::EaseOutQuad => ease(Quad, Out),
        InterpDto::EaseInOutQuad => ease(Quad, InOut),
        InterpDto::EaseInCubic => ease(Cubic, In),
        InterpDto::EaseOutCubic => ease(Cubic, Out),
        InterpDto::EaseInOutCubic => ease(Cubic, InOut),
        InterpDto::EaseInExpo => ease(Expo, In),
        InterpDto::EaseOutExpo => ease(Expo, Out),
        InterpDto::EaseInOutExpo => ease(Expo, InOut),
        InterpDto::OvershootOut => Interp::OvershootOut,
        InterpDto::OvershootIn => Interp::OvershootIn,
        InterpDto::ElasticOut => Interp::ElasticOut,
        InterpDto::BounceOut => Interp::BounceOut,
    }
}

pub(crate) fn interp_dto(interp: Interp) -> InterpDto {
    use EaseCurve::{Cubic, Expo, Quad, Sine};
    use EaseDir::{In, InOut, Out};
    match interp {
        Interp::Linear => InterpDto::Linear,
        Interp::Hold => InterpDto::Hold,
        Interp::Ease { curve, dir } => match (curve, dir) {
            (Sine, In) => InterpDto::EaseInSine,
            (Sine, Out) => InterpDto::EaseOutSine,
            (Sine, InOut) => InterpDto::EaseInOutSine,
            (Quad, In) => InterpDto::EaseInQuad,
            (Quad, Out) => InterpDto::EaseOutQuad,
            (Quad, InOut) => InterpDto::EaseInOutQuad,
            (Cubic, In) => InterpDto::EaseInCubic,
            (Cubic, Out) => InterpDto::EaseOutCubic,
            (Cubic, InOut) => InterpDto::EaseInOutCubic,
            (Expo, In) => InterpDto::EaseInExpo,
            (Expo, Out) => InterpDto::EaseOutExpo,
            (Expo, InOut) => InterpDto::EaseInOutExpo,
        },
        Interp::OvershootOut => InterpDto::OvershootOut,
        Interp::OvershootIn => InterpDto::OvershootIn,
        Interp::ElasticOut => InterpDto::ElasticOut,
        Interp::BounceOut => InterpDto::BounceOut,
    }
}

pub(crate) fn direction(dto: DirectionDto) -> Direction {
    match dto {
        DirectionDto::Left => Direction::Left,
        DirectionDto::Right => Direction::Right,
        DirectionDto::Up => Direction::Up,
        DirectionDto::Down => Direction::Down,
        DirectionDto::Clockwise => Direction::Clockwise,
        DirectionDto::CounterClockwise => Direction::CounterClockwise,
    }
}

pub(crate) fn direction_dto(direction: Direction) -> DirectionDto {
    match direction {
        Direction::Left => DirectionDto::Left,
        Direction::Right => DirectionDto::Right,
        Direction::Up => DirectionDto::Up,
        Direction::Down => DirectionDto::Down,
        Direction::Clockwise => DirectionDto::Clockwise,
        Direction::CounterClockwise => DirectionDto::CounterClockwise,
    }
}

pub(crate) fn anim_unit(dto: AnimUnitDto) -> AnimUnit {
    match dto {
        AnimUnitDto::Block => AnimUnit::Block,
        AnimUnitDto::Line => AnimUnit::Line,
        AnimUnitDto::Word => AnimUnit::Word,
        AnimUnitDto::Letter => AnimUnit::Letter,
    }
}

pub(crate) fn anim_unit_dto(unit: AnimUnit) -> AnimUnitDto {
    match unit {
        AnimUnit::Block => AnimUnitDto::Block,
        AnimUnit::Line => AnimUnitDto::Line,
        AnimUnit::Word => AnimUnitDto::Word,
        AnimUnit::Letter => AnimUnitDto::Letter,
    }
}

pub(crate) fn anim_order(dto: AnimOrderDto) -> AnimOrder {
    match dto {
        AnimOrderDto::Forward => AnimOrder::Forward,
        AnimOrderDto::Backward => AnimOrder::Backward,
        AnimOrderDto::CenterOut => AnimOrder::CenterOut,
        AnimOrderDto::EdgesIn => AnimOrder::EdgesIn,
        AnimOrderDto::Random => AnimOrder::Random,
    }
}

pub(crate) fn anim_order_dto(order: AnimOrder) -> AnimOrderDto {
    match order {
        AnimOrder::Forward => AnimOrderDto::Forward,
        AnimOrder::Backward => AnimOrderDto::Backward,
        AnimOrder::CenterOut => AnimOrderDto::CenterOut,
        AnimOrder::EdgesIn => AnimOrderDto::EdgesIn,
        AnimOrder::Random => AnimOrderDto::Random,
    }
}

fn travel(dto: TravelDto) -> Travel {
    match dto {
        TravelDto::FromEdge => Travel::FromEdge,
        TravelDto::Short => Travel::Short,
    }
}

fn travel_dto(travel: Travel) -> TravelDto {
    match travel {
        Travel::FromEdge => TravelDto::FromEdge,
        Travel::Short => TravelDto::Short,
    }
}

// ---------------------------------------------------------------------------
// Keyframes
// ---------------------------------------------------------------------------

/// Where the unknown keys of the key track of `prop` are filed.
fn track_path(prop: PropPath) -> String {
    format!("keys[{prop}]")
}

/// Where the unknown keys of `prop`'s key at `at` are filed: by its time,
/// which is unique within the property.
fn key_path(prop: PropPath, at: Tick) -> String {
    format!("keys[{prop}]@{}", at.raw())
}

/// `clip`'s keys as stored, checked against the clip (already read without
/// keys), and whether any key track had to be sorted by time. Values
/// brought into range set `clamped`.
pub(super) fn read_keys(
    dto: ClipKeysDto,
    at: &At,
    clip: &Clip,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(ClipKeys, bool), OpenError> {
    let origin = tick(dto.origin, &at.key("origin"))?;
    extras.put(owner, "keys", dto.extra);
    let mut seen = BTreeSet::new();
    let mut tracks = Vec::with_capacity(dto.tracks.len());
    let mut sorted = false;
    for (i, track) in dto.tracks.into_iter().enumerate() {
        let track_at = at.key("tracks").index(i);
        let (prop, keys, track_sorted) =
            read_track(track, &track_at, clip, owner, extras, clamped)?;
        if !seen.insert(prop) {
            return Err(track_at.key("prop").bad(format!(
                "`{prop}` is listed twice; each property has one key list"
            )));
        }
        sorted |= track_sorted;
        tracks.push((prop, keys));
    }
    // Every rule `from_parts` checks was checked above with a path.
    let keys = ClipKeys::from_parts(origin, tracks).map_err(|e| at.bad(e))?;
    Ok((keys, sorted))
}

fn read_track(
    dto: KeyTrackDto,
    at: &At,
    clip: &Clip,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(PropPath, Vec<Keyframe<PropValue>>, bool), OpenError> {
    let prop_at = at.key("prop");
    let (prop, kind) = match PropPath::parse(&dto.prop).and_then(|p| Some((p, p.kind()?))) {
        Some(found) => found,
        None => {
            return Err(prop_at.bad(format!(
                "`{}` is not a property that takes keyframes",
                dto.prop
            )));
        }
    };
    let fits = match prop {
        PropPath::Transform(_) => clip.content.is_visual(),
        PropPath::Text(_) | PropPath::FillStopColor(_) | PropPath::Reveal => {
            clip.content.text().is_some()
        }
        PropPath::Color(_) | PropPath::ColorPuck(_) => clip.content.takes_color(),
    };
    if !fits {
        let only = match prop {
            PropPath::Transform(_) => "visual clips",
            PropPath::Color(_) | PropPath::ColorPuck(_) => "video, image and adjustment clips",
            _ => "text clips",
        };
        return Err(prop_at.bad(format!("`{prop}` takes keyframes only on {only}")));
    }
    let keys_at = at.key("keys");
    if dto.keys.is_empty() {
        return Err(keys_at.bad(format!(
            "`{prop}` has no keyframes; leave the property out instead"
        )));
    }
    let mut keys = Vec::with_capacity(dto.keys.len());
    for (i, k) in dto.keys.into_iter().enumerate() {
        let key_at = keys_at.index(i);
        let (raw, value_extra) = read_value(kind, k.value, &key_at.key("value"), clamped)?;
        // As an edit would hold it; a property the clip's style lacks right
        // now (a stop beyond the gradient's) keeps its value as stored.
        let value = match prop.clamp_value(clip, raw) {
            Some(fixed) => {
                *clamped |= fixed != raw;
                fixed
            }
            None => raw,
        };
        let key = Keyframe {
            at: tick(k.at, &key_at.key("at"))?,
            value,
            interp: interp(k.interp),
        };
        keys.push((i, key, k.extra, value_extra));
    }
    let sorted = keys.windows(2).any(|w| w[0].1.at > w[1].1.at);
    // Stable, so of two keys at one time the later one is named below.
    keys.sort_by_key(|(_, key, _, _)| key.at);
    if let Some(w) = keys.windows(2).find(|w| w[0].1.at == w[1].1.at) {
        return Err(keys_at.index(w[1].0).key("at").bad(format!(
            "two `{prop}` keyframes at {} ticks (`{}` and `{}`); keyframe times must differ",
            w[1].1.at.raw(),
            keys_at.index(w[0].0).spelled(),
            keys_at.index(w[1].0).spelled(),
        )));
    }
    extras.put(owner, track_path(prop), dto.extra);
    let keys = keys
        .into_iter()
        .map(|(_, key, extra, value_extra)| {
            let path = key_path(prop, key.at);
            extras.put(owner, format!("{path}.value"), value_extra);
            extras.put(owner, path, extra);
            key
        })
        .collect();
    Ok((prop, keys, sorted))
}

/// An integer of at most 64 bits; fractions, strings and wider numbers are
/// refused like any other damaged value.
fn integer(raw: &serde_json::Value, at: &At) -> Result<i64, OpenError> {
    match raw {
        serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| {
            at.schema(format!(
                "invalid value: {n}, expected an integer of at most 64 bits"
            ))
        }),
        other => Err(at.schema(format!(
            "invalid type: {}, expected an integer",
            json_kind(other)
        ))),
    }
}

/// A key value of `kind`, and the unknown keys of a point value.
fn read_value(
    kind: ValueKind,
    dto: KeyValueDto,
    at: &At,
    clamped: &mut bool,
) -> Result<(PropValue, Fields), OpenError> {
    let raw = match dto {
        KeyValueDto::Unparsed(raw) => raw,
        // The writer's forms; the reader only ever sees `Unparsed`.
        KeyValueDto::Int(v) => serde_json::Value::from(v),
        KeyValueDto::Color(c) => serde_json::Value::String(c),
        KeyValueDto::Point(p) => {
            let mut map = serde_json::Map::new();
            map.insert("x".into(), p.x.into());
            map.insert("y".into(), p.y.into());
            serde_json::Value::Object(map)
        }
    };
    let value = match kind {
        ValueKind::Position => {
            let (x, y, extra) = read_point(raw, at, clamped)?;
            return Ok((PropValue::Position(CanvasPos::new(x, y)), extra));
        }
        ValueKind::TailTip => {
            let (x, y, extra) = read_point(raw, at, clamped)?;
            let tip = TailTip {
                x: RefPx::from_hundredths(x),
                y: RefPx::from_hundredths(y),
            };
            return Ok((PropValue::TailTip(tip), extra));
        }
        ValueKind::Puck => {
            let (x, y, extra) = read_point(raw, at, clamped)?;
            let x = fit(i64::from(x), i16::MIN, i16::MAX, clamped);
            let y = fit(i64::from(y), i16::MIN, i16::MAX, clamped);
            return Ok((PropValue::Puck { x, y }, extra));
        }
        ValueKind::Color => match &raw {
            serde_json::Value::String(text) => match parse_color(text) {
                Some(c) => PropValue::Color(c),
                None => {
                    return Err(at.bad(format!("`{text}` is not a #RRGGBBAA colour")));
                }
            },
            other => {
                return Err(at.schema(format!(
                    "invalid type: {}, expected a \"#RRGGBBAA\" colour string",
                    json_kind(other)
                )));
            }
        },
        ValueKind::Scale => PropValue::Scale(Scale::from_basis_points(fit(
            integer(&raw, at)?,
            i32::MIN,
            i32::MAX,
            clamped,
        ))),
        ValueKind::Angle => PropValue::Angle(Angle::from_hundredths(fit(
            integer(&raw, at)?,
            i32::MIN,
            i32::MAX,
            clamped,
        ))),
        ValueKind::Ratio => {
            PropValue::Ratio(PerMille::new(fit(integer(&raw, at)?, 0, u16::MAX, clamped)))
        }
        ValueKind::Length => PropValue::Length(RefPx::from_hundredths(fit(
            integer(&raw, at)?,
            i32::MIN,
            i32::MAX,
            clamped,
        ))),
        ValueKind::SignedRatio => PropValue::SignedRatio(SignedPerMille::new(fit(
            integer(&raw, at)?,
            i16::MIN,
            i16::MAX,
            clamped,
        ))),
        ValueKind::Level => PropValue::Level(fit(integer(&raw, at)?, i16::MIN, i16::MAX, clamped)),
        ValueKind::Direction => {
            let tenths = fit(integer(&raw, at)?, i32::MIN, i32::MAX, clamped);
            // Wrapped into one turn, as the model does.
            *clamped |= !(0..3600).contains(&tenths);
            PropValue::Direction(DeciDegrees::new(tenths))
        }
    };
    Ok((value, Fields::new()))
}

/// An `{"x", "y"}` point, saturated to the model's width, and its unknown
/// keys.
fn read_point(
    mut raw: serde_json::Value,
    at: &At,
    clamped: &mut bool,
) -> Result<(i32, i32, Fields), OpenError> {
    // Reading the value as a `Value` turned wrapped wide integers back into
    // numbers; wrap them again for the point's unknown-key buffer.
    crate::format::carry_wide_integers(&mut raw);
    let p: PointDto = decode_at(raw, at)?;
    let x = fit(p.x, i32::MIN, i32::MAX, clamped);
    let y = fit(p.y, i32::MIN, i32::MAX, clamped);
    Ok((x, y, p.extra))
}

/// The `keys` object, left out while the clip has no keys, its origin is
/// 0 and it carries no unknown keys.
pub(super) fn write_keys(keys: &ClipKeys, give: &Give<'_>) -> Option<ClipKeysDto> {
    let extra = give.at("keys");
    if keys.is_empty() && keys.origin() == Tick::ZERO && extra.is_empty() {
        return None;
    }
    let tracks = keys
        .tracks()
        .iter()
        .map(|track| {
            let prop = track.prop();
            KeyTrackDto {
                prop: prop.to_path_string(),
                keys: track
                    .keys()
                    .iter()
                    .map(|key| PropKeyDto {
                        at: key.at.raw(),
                        value: write_value(key.value, || {
                            give.at_with(|| format!("{}.value", key_path(prop, key.at)))
                        }),
                        interp: interp_dto(key.interp),
                        extra: give.at_with(|| key_path(prop, key.at)),
                    })
                    .collect(),
                extra: give.at_with(|| track_path(prop)),
            }
        })
        .collect();
    Some(ClipKeysDto {
        origin: keys.origin().raw(),
        tracks,
        extra,
    })
}

/// How the file holds a key value: the same fixed-point integers as the
/// static values, a colour string, or a point.
fn write_value(value: PropValue, point_extra: impl FnOnce() -> Fields) -> KeyValueDto {
    let point = |x: i32, y: i32, extra: Fields| {
        KeyValueDto::Point(PointDto {
            x: i64::from(x),
            y: i64::from(y),
            extra,
        })
    };
    match value {
        PropValue::Position(p) => point(p.x, p.y, point_extra()),
        PropValue::TailTip(t) => point(t.x.hundredths(), t.y.hundredths(), point_extra()),
        PropValue::Scale(s) => KeyValueDto::Int(i64::from(s.basis_points())),
        PropValue::Angle(a) => KeyValueDto::Int(i64::from(a.hundredths())),
        PropValue::Ratio(r) => KeyValueDto::Int(i64::from(r.get())),
        PropValue::Length(l) => KeyValueDto::Int(i64::from(l.hundredths())),
        PropValue::SignedRatio(r) => KeyValueDto::Int(i64::from(r.get())),
        PropValue::Direction(d) => KeyValueDto::Int(i64::from(d.tenths())),
        PropValue::Color(c) => KeyValueDto::Color(color_text(c)),
        PropValue::Level(v) => KeyValueDto::Int(i64::from(v)),
        PropValue::Puck { x, y } => point(i32::from(x), i32::from(y), point_extra()),
    }
}

// ---------------------------------------------------------------------------
// Animations
// ---------------------------------------------------------------------------

const SLOTS: [(Slot, &str); 3] = [
    (Slot::Entrance, "entrance"),
    (Slot::Exit, "exit"),
    (Slot::Continuous, "continuous"),
];

/// The extras path of a slot.
fn slot_path(name: &str) -> String {
    format!("animations.{name}")
}

/// A clip's animations as stored, checked against its content's slot
/// policy (FX-019). Settings brought into range set `clamped`.
pub(super) fn read_animations(
    dto: ClipAnimationsDto,
    at: &At,
    content: &ClipContent,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<ClipAnimations, OpenError> {
    extras.put(owner, "animations", dto.extra);
    let slots = [dto.entrance, dto.exit, dto.continuous];
    let policy = slot_policy(content);
    let mut out = ClipAnimations::default();
    for ((slot, name), spec) in SLOTS.into_iter().zip(slots) {
        let Some(spec) = spec else { continue };
        let slot_at = at.key(name);
        if policy.is_none() {
            return Err(slot_at.bad("audio clips have no animation slots"));
        }
        let spec = read_spec(spec, slot, &slot_at, clamped)?;
        extras.put(owner, slot_path(name), spec.1);
        let spec = Some(Arc::new(spec.0));
        match slot {
            Slot::Entrance => out.entrance = spec,
            Slot::Exit => out.exit = spec,
            Slot::Continuous => out.continuous = spec,
        }
    }
    if policy == Some(SlotPolicy::Exclusive)
        && out.continuous.is_some()
        && (out.entrance.is_some() || out.exit.is_some())
    {
        return Err(at.key("continuous").bad(
            "this clip plays either a continuous animation or entrance and exit \
             animations, not both (FX-019)",
        ));
    }
    Ok(out)
}

/// One slot's spec, clamped into range, and its unknown keys.
fn read_spec(
    dto: AnimationSpecDto,
    slot: Slot,
    at: &At,
    clamped: &mut bool,
) -> Result<(AnimationSpec, Fields), OpenError> {
    if dto.preset.is_empty() {
        return Err(at.key("preset").bad("a preset id can't be empty"));
    }
    let stored = tick(dto.duration, &at.key("duration"))?;
    if !stored.is_positive() {
        return Err(at.key("duration").bad(format!(
            "{} ticks; an animation's length must be positive",
            dto.duration
        )));
    }
    // As SetClipAnimation would store it: In and Out at least 0.1 s
    // (FX-018), a Loop's cycle within 0.1–10 s (FX-019). The edit also
    // snaps In and Out to frames and fits them to the clip; playback does
    // the latter (FX-018's proportional scaling).
    let duration = match slot {
        Slot::Continuous => stored.clamp(AnimationSpec::MIN_DURATION, AnimationSpec::MAX_CYCLE),
        Slot::Entrance | Slot::Exit => stored.max(AnimationSpec::MIN_DURATION),
    };
    *clamped |= duration != stored;
    let pace = match dto.pace {
        Some(p) => Some(tick(p, &at.key("pace"))?),
        None => None,
    };
    let spec = AnimationSpec {
        preset: PresetId::new(&dto.preset),
        duration,
        easing: dto.easing.map(interp),
        direction: dto.direction.map(direction),
        params: AnimParams {
            unit: dto.unit.map(anim_unit),
            overlap: PerMille::new(fit(dto.overlap, 0, u16::MAX, clamped)),
            order: anim_order(dto.order),
            seed: fit(dto.seed, 0, u32::MAX, clamped),
            pace,
            intensity: PerMille::new(fit(dto.intensity, 0, u16::MAX, clamped)),
            travel: travel(dto.travel),
            caret: dto.caret,
        },
        phase: tick(dto.phase, &at.key("phase"))?,
        seams: match dto.seams {
            Some(seams) => Some(read_seams(seams, &at.key("seams"), clamped)?),
            None => None,
        },
    };
    let fixed = spec.clamped();
    *clamped |= fixed != spec;
    Ok((fixed, dto.extra))
}

/// A split Loop's seams (TX-022), the ramp held within 0–250 ms.
fn read_seams(dto: LoopSeamsDto, at: &At, clamped: &mut bool) -> Result<LoopSeams, OpenError> {
    let edge = |v: Option<i64>, key: &str| v.map(|v| tick(v, &at.key(key))).transpose();
    let stored = tick(dto.ramp, &at.key("ramp"))?;
    let ramp = stored.clamp(Tick::ZERO, LoopSeams::MAX_RAMP);
    *clamped |= ramp != stored;
    Ok(LoopSeams {
        start: edge(dto.start, "start")?,
        end: edge(dto.end, "end")?,
        ramp,
    })
}

/// The `animations` object, left out while every slot is empty and it
/// carries no unknown keys.
pub(super) fn write_animations(
    anims: &ClipAnimations,
    give: &Give<'_>,
) -> Option<ClipAnimationsDto> {
    let extra = give.at("animations");
    if anims.is_empty() && extra.is_empty() {
        return None;
    }
    let spec = |slot: Slot, name: &str| {
        anims
            .get(slot)
            .map(|s| write_spec(s, give.at(&slot_path(name))))
    };
    Some(ClipAnimationsDto {
        entrance: spec(Slot::Entrance, "entrance"),
        exit: spec(Slot::Exit, "exit"),
        continuous: spec(Slot::Continuous, "continuous"),
        extra,
    })
}

fn write_spec(spec: &AnimationSpec, extra: Fields) -> AnimationSpecDto {
    let p = &spec.params;
    AnimationSpecDto {
        preset: spec.preset.as_str().to_string(),
        duration: spec.duration.raw(),
        easing: spec.easing.map(interp_dto),
        direction: spec.direction.map(direction_dto),
        unit: p.unit.map(anim_unit_dto),
        overlap: i64::from(p.overlap.get()),
        order: anim_order_dto(p.order),
        seed: i64::from(p.seed),
        pace: p.pace.map(Tick::raw),
        intensity: i64::from(p.intensity.get()),
        travel: travel_dto(p.travel),
        phase: spec.phase.raw(),
        caret: p.caret,
        seams: spec.seams.map(|s| LoopSeamsDto {
            start: s.start.map(Tick::raw),
            end: s.end.map(Tick::raw),
            ramp: s.ramp.raw(),
        }),
        extra,
    }
}

/// Every tick a clip's keys and animations hold, with its JSON path below
/// the clip, for [`super::check_writable`].
pub(super) fn ticks(clip: &Clip) -> Vec<(Tick, String)> {
    let mut out = Vec::new();
    if !clip.keys.is_empty() || clip.keys.origin() != Tick::ZERO {
        out.push((clip.keys.origin(), "keys.origin".to_string()));
    }
    for (i, track) in clip.keys.tracks().iter().enumerate() {
        for (j, key) in track.keys().iter().enumerate() {
            out.push((key.at, format!("keys.tracks[{i}].keys[{j}].at")));
        }
    }
    for (slot, name) in SLOTS {
        if let Some(spec) = clip.animations.get(slot) {
            out.push((spec.duration, format!("animations.{name}.duration")));
            out.push((spec.phase, format!("animations.{name}.phase")));
            if let Some(seams) = spec.seams {
                let at = |key: &str| format!("animations.{name}.seams.{key}");
                out.extend(seams.start.map(|t| (t, at("start"))));
                out.extend(seams.end.map(|t| (t, at("end"))));
                out.push((seams.ramp, at("ramp")));
            }
            if let Some(pace) = spec.params.pace {
                out.push((pace, format!("animations.{name}.pace")));
            }
        }
    }
    out
}
