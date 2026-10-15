//! The colour stack of a clip (format 10, CL-001; format 11 adds HSL,
//! Curves and Wheels; format 12 the LUT slot and skin protection; format
//! 13 Auto and Match; format 14 colour masks): read and written with every
//! default left out. A mask with an unknown shape or mode, an id that is 0,
//! out of range or repeated, more than 8 masks, and a roundness or band
//! width on a shape without one are refused at their path; mask values
//! outside their ranges are clamped. A LUT
//! hash that isn't 32 hex digits, an unknown interpolation, auto style or
//! source, and a match reference that names neither or both of a clip and
//! a library item are refused at their path; match gains and offsets
//! outside their limits are clamped.
//! Values outside a slider's range are clamped into it, curve coordinates
//! into 0–10000 and pucks onto their disk (and counted like other clamped
//! values); a list of points that is no curve (CL-014's rules) and a stack
//! on a clip that can't carry one (text, sound, stickers, effects) are
//! refused at their path; a stack that reads back to the defaults is
//! stored as none.

use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::model::{
    Angle, AutoCorrection, AutoOffsets, AutoSource, AutoStyle, ClipContent, ColorMask,
    ColorParamId, ColorStack, ColorWheels, Curve, CurveChannel, CurvePoint, Curves, HslBand,
    HslChannel, HueBand, LutInterp, LutRef, LutSlot, MATCH_GAIN_MAX, MATCH_GAIN_MIN,
    MATCH_OFFSET_LIMIT, MAX_COLOR_MASKS, Mask, MaskCombine, MaskPoint, MaskShape, MaskShapeKind,
    MatchRef, MatchTransform, PerMille, Section, SkinProtect, SkinScope, Wheel, WheelId, hash_hex,
    parse_hash_hex,
};

use super::{At, Give, fit, id, tick};
use crate::extras::{Extras, Owner};
use crate::format::dto::{
    AutoDto, AutoOffsetsDto, ColorBalanceDto, ColorDto, ColorMaskDto, CurvesDto, HslBandDto,
    HslDto, LightDto, LutDto, MatchDto, MatchRefDto, SkinDto, WheelDto, WheelsDto,
};
use crate::open::OpenError;

/// The Light sliders with their DTO fields, in file order.
fn light_fields(dto: &mut LightDto) -> [(ColorParamId, &mut Option<i64>); 9] {
    [
        (ColorParamId::Exposure, &mut dto.exposure),
        (ColorParamId::Brightness, &mut dto.brightness),
        (ColorParamId::Contrast, &mut dto.contrast),
        (ColorParamId::ContrastPivot, &mut dto.contrast_pivot),
        (ColorParamId::Highlights, &mut dto.highlights),
        (ColorParamId::Shadows, &mut dto.shadows),
        (ColorParamId::Whites, &mut dto.whites),
        (ColorParamId::Blacks, &mut dto.blacks),
        (ColorParamId::ToneLift, &mut dto.tone_lift),
    ]
}

fn balance_fields(dto: &mut ColorBalanceDto) -> [(ColorParamId, &mut Option<i64>); 5] {
    [
        (ColorParamId::Temperature, &mut dto.temperature),
        (ColorParamId::Tint, &mut dto.tint),
        (ColorParamId::Hue, &mut dto.hue),
        (ColorParamId::Saturation, &mut dto.saturation),
        (ColorParamId::Vibrance, &mut dto.vibrance),
    ]
}

fn hsl_bands(dto: &mut HslDto) -> [(HueBand, &mut Option<HslBandDto>); 8] {
    [
        (HueBand::Red, &mut dto.red),
        (HueBand::Orange, &mut dto.orange),
        (HueBand::Yellow, &mut dto.yellow),
        (HueBand::Green, &mut dto.green),
        (HueBand::Cyan, &mut dto.cyan),
        (HueBand::Blue, &mut dto.blue),
        (HueBand::Purple, &mut dto.purple),
        (HueBand::Magenta, &mut dto.magenta),
    ]
}

fn band_fields(dto: &mut HslBandDto) -> [(HslChannel, &mut Option<i64>); 3] {
    [
        (HslChannel::Hue, &mut dto.hue),
        (HslChannel::Saturation, &mut dto.saturation),
        (HslChannel::Lightness, &mut dto.lightness),
    ]
}

fn curve_fields(dto: &mut CurvesDto) -> [(CurveChannel, &mut Option<Vec<[i64; 2]>>); 4] {
    [
        (CurveChannel::Luma, &mut dto.luma),
        (CurveChannel::Red, &mut dto.red),
        (CurveChannel::Green, &mut dto.green),
        (CurveChannel::Blue, &mut dto.blue),
    ]
}

fn wheel_fields(dto: &mut WheelsDto) -> [(WheelId, &mut Option<WheelDto>); 4] {
    [
        (WheelId::Shadows, &mut dto.shadows),
        (WheelId::Midtones, &mut dto.midtones),
        (WheelId::Highlights, &mut dto.highlights),
        (WheelId::Offset, &mut dto.offset),
    ]
}

/// The extras path of an HSL band, a wheel or a part of the stack.
fn part(name: &str) -> String {
    format!("color.{name}")
}

/// HSL (CL-013) into `stack`.
fn read_hsl(
    mut dto: HslDto,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) {
    for (band, field) in hsl_bands(&mut dto) {
        let Some(mut b) = field.take() else {
            continue;
        };
        for (channel, v) in band_fields(&mut b) {
            if let Some(v) = v.take() {
                let id = ColorParamId::Hsl { band, channel };
                *stack = stack.with(id, level(id, v, clamped));
            }
        }
        extras.put(owner, part(&format!("hsl.{}", band.as_str())), b.extra);
    }
    extras.put(owner, part("hsl"), dto.extra);
}

/// A curve's points, coordinates clamped into 0–10000.
fn read_curve(points: &[[i64; 2]], at: &At, clamped: &mut bool) -> Result<Curve, OpenError> {
    let points = points
        .iter()
        .map(|[x, y]| {
            let fix = |v: i64, clamped: &mut bool| {
                let c = v.clamp(0, i64::from(Curve::ONE));
                *clamped |= c != v;
                // In 0..=10000 after the clamp.
                u16::try_from(c).unwrap_or(Curve::ONE)
            };
            CurvePoint::new(fix(*x, clamped), fix(*y, clamped))
        })
        .collect();
    Curve::new(points).map_err(|e| at.bad(format!("not a curve: {e}")))
}

/// Curves (CL-014) into `stack`.
fn read_curves(
    mut dto: CurvesDto,
    at: &At,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(), OpenError> {
    for (channel, field) in curve_fields(&mut dto) {
        if let Some(points) = field.take() {
            let curve = read_curve(&points, &at.key(channel.as_str()), clamped)?;
            *stack = stack.with_curve(channel, curve);
        }
    }
    extras.put(owner, part("curves"), dto.extra);
    Ok(())
}

/// Wheels (CL-015) into `stack`: pucks outside the disk land on its rim.
fn read_wheels(
    mut dto: WheelsDto,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) {
    if let Some(v) = dto.strength.take() {
        let id = ColorParamId::WheelStrength;
        *stack = stack.with(id, level(id, v, clamped));
    }
    for (wheel, field) in wheel_fields(&mut dto) {
        let Some(w) = field.take() else {
            continue;
        };
        let x = fit(w.x.unwrap_or(0), i16::MIN, i16::MAX, clamped);
        let y = fit(w.y.unwrap_or(0), i16::MIN, i16::MAX, clamped);
        *stack = stack.with_puck(wheel, x, y);
        *clamped |= stack.puck(wheel) != (x, y);
        if let Some(v) = w.luma {
            let id = ColorParamId::WheelLuma(wheel);
            *stack = stack.with(id, level(id, v, clamped));
        }
        extras.put(owner, part(&format!("wheels.{}", wheel.as_str())), w.extra);
    }
    extras.put(owner, part("wheels"), dto.extra);
}

/// Skin protection (CL-030) of `scope` into `stack`.
fn read_skin(
    dto: SkinDto,
    scope: SkinScope,
    stack: &mut ColorStack,
    owner: Owner,
    path: &str,
    extras: &mut Extras,
    clamped: &mut bool,
) {
    *stack = stack.with_skin(scope, dto.on);
    if let Some(v) = dto.amount {
        let id = ColorParamId::SkinAmount(scope);
        *stack = stack.with(id, level(id, v, clamped));
    }
    extras.put(owner, part(path), dto.extra);
}

/// The LUT slot (CL-017) into `stack`.
fn read_lut(
    dto: LutDto,
    at: &At,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(), OpenError> {
    let Some(hash) = parse_hash_hex(&dto.hash) else {
        return Err(at
            .key("hash")
            .bad("a LUT hash is 32 hex digits (the XXH3-128 of the file)"));
    };
    let interp = match dto.interp.as_deref() {
        None => LutInterp::default(),
        Some(name) => LutInterp::parse(name).ok_or_else(|| {
            at.key("interp")
                .bad(format!("`{name}` is not `tetrahedral` or `trilinear`"))
        })?,
    };
    *stack = stack.with_lut(Some(LutSlot {
        interp,
        ..LutSlot::new(LutRef::new(hash, dto.file))
    }));
    if let Some(v) = dto.intensity {
        let id = ColorParamId::LutIntensity;
        *stack = stack.with(id, level(id, v, clamped));
    }
    if let Some(skin) = dto.skin {
        read_skin(
            skin,
            SkinScope::Lut,
            stack,
            owner,
            "lut.skin",
            extras,
            clamped,
        );
    }
    extras.put(owner, part("lut"), dto.extra);
    Ok(())
}

fn auto_fields(dto: &mut AutoOffsetsDto) -> [(ColorParamId, &mut Option<i64>); 8] {
    [
        (ColorParamId::Exposure, &mut dto.exposure),
        (ColorParamId::Contrast, &mut dto.contrast),
        (ColorParamId::Whites, &mut dto.whites),
        (ColorParamId::Blacks, &mut dto.blacks),
        (ColorParamId::Temperature, &mut dto.temperature),
        (ColorParamId::Tint, &mut dto.tint),
        (ColorParamId::Vibrance, &mut dto.vibrance),
        (ColorParamId::Saturation, &mut dto.saturation),
    ]
}

/// The auto correction (CL-019) into `stack`.
fn read_auto(
    mut dto: AutoDto,
    at: &At,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(), OpenError> {
    let style = match dto.style.as_deref() {
        None => AutoStyle::default(),
        Some(name) => AutoStyle::parse(name).ok_or_else(|| {
            at.key("style")
                .bad(format!("`{name}` is not `natural` or `vivid`"))
        })?,
    };
    let source = match dto.source.as_deref() {
        None => AutoSource::default(),
        Some(name) => AutoSource::parse(name).ok_or_else(|| {
            at.key("source")
                .bad(format!("`{name}` is not `clip` or `playhead`"))
        })?,
    };
    let mut offsets = AutoOffsets::default();
    for (id, v) in auto_fields(&mut dto.offsets) {
        if let Some(v) = v.take() {
            let wide = fit(v, i16::MIN, i16::MAX, clamped);
            offsets = offsets.with(id, wide);
            *clamped |= offsets.get(id) != wide;
        }
    }
    *stack = stack.with_auto(Some(AutoCorrection::new(offsets, style, source)));
    if let Some(v) = dto.strength {
        let id = ColorParamId::AutoStrength;
        *stack = stack.with(id, level(id, v, clamped));
    }
    extras.put(owner, part("auto.offsets"), dto.offsets.extra);
    extras.put(owner, part("auto"), dto.extra);
    Ok(())
}

fn write_auto(auto: &AutoCorrection, give: &Give<'_>) -> AutoDto {
    let mut offsets = AutoOffsetsDto {
        extra: give.at(&part("auto.offsets")),
        ..AutoOffsetsDto::default()
    };
    for (id, field) in auto_fields(&mut offsets) {
        let v = auto.offsets.get(id);
        *field = (v != 0).then_some(i64::from(v));
    }
    AutoDto {
        offsets,
        style: (auto.style != AutoStyle::default()).then(|| auto.style.as_str().to_string()),
        source: (auto.source != AutoSource::default()).then(|| auto.source.as_str().to_string()),
        strength: (auto.strength != 100).then_some(i64::from(auto.strength)),
        extra: give.at(&part("auto")),
    }
}

/// The colour match (CL-020) into `stack`.
fn read_match(
    dto: MatchDto,
    at: &At,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(), OpenError> {
    let r = &dto.reference;
    let r_at = at.key("reference");
    let when = tick(r.at, &r_at.key("at"))?;
    let reference = match (r.clip, r.media) {
        (Some(clip), None) => MatchRef::Clip {
            clip: ClipId::from_raw(id(clip, &r_at.key("clip"))?),
            at: when,
            graded: !r.ungraded,
        },
        (None, Some(media)) => MatchRef::Media {
            media: MediaId::from_raw(id(media, &r_at.key("media"))?),
            at: when,
        },
        _ => return Err(r_at.bad("a reference names exactly one of `clip` and `media`")),
    };
    let limit = |v: i64, lo: i32, hi: i32, clamped: &mut bool| {
        let wide = fit(v, i32::MIN, i32::MAX, clamped);
        let fixed = wide.clamp(lo, hi);
        *clamped |= fixed != wide;
        fixed
    };
    let gain = dto
        .gain
        .map(|g| limit(g, MATCH_GAIN_MIN, MATCH_GAIN_MAX, clamped));
    let offset = dto
        .offset
        .map(|o| limit(o, -MATCH_OFFSET_LIMIT, MATCH_OFFSET_LIMIT, clamped));
    *stack = stack.with_match(Some(MatchTransform::new(gain, offset, reference)));
    if let Some(v) = dto.intensity {
        let id = ColorParamId::MatchIntensity;
        *stack = stack.with(id, level(id, v, clamped));
    }
    extras.put(owner, part("match.reference"), dto.reference.extra);
    extras.put(owner, part("match"), dto.extra);
    Ok(())
}

fn write_match(m: &MatchTransform, give: &Give<'_>) -> MatchDto {
    let reference = match &m.reference {
        MatchRef::Clip { clip, at, graded } => MatchRefDto {
            clip: Some(clip.nonzero()),
            media: None,
            at: at.raw(),
            ungraded: !graded,
            extra: give.at(&part("match.reference")),
        },
        MatchRef::Media { media, at } => MatchRefDto {
            clip: None,
            media: Some(media.nonzero()),
            at: at.raw(),
            ungraded: false,
            extra: give.at(&part("match.reference")),
        },
    };
    MatchDto {
        gain: m.gain().map(i64::from),
        offset: m.offset().map(i64::from),
        intensity: (m.intensity != 100).then_some(i64::from(m.intensity)),
        reference,
        extra: give.at(&part("match")),
    }
}

/// The colour masks (CL-031) into `stack`.
fn read_masks(
    dtos: Vec<ColorMaskDto>,
    at: &At,
    stack: &mut ColorStack,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<(), OpenError> {
    if dtos.len() > MAX_COLOR_MASKS {
        return Err(at.index(MAX_COLOR_MASKS).bad(format!(
            "a colour stack holds at most {MAX_COLOR_MASKS} masks"
        )));
    }
    let mut masks: Vec<ColorMask> = Vec::with_capacity(dtos.len());
    for (i, dto) in dtos.into_iter().enumerate() {
        let m_at = at.index(i);
        let id = u16::try_from(dto.id)
            .ok()
            .filter(|id| *id != 0)
            .ok_or_else(|| m_at.key("id").bad("a mask id is 1 to 65535"))?;
        if masks.iter().any(|m| m.id == id) {
            return Err(m_at.key("id").bad(format!("mask id {id} is used twice")));
        }
        let kind = MaskShapeKind::parse(&dto.shape).ok_or_else(|| {
            m_at.key("shape").bad(format!(
                "`{}` is not `linear`, `band`, `ellipse` or `rectangle`",
                dto.shape
            ))
        })?;
        let mode = match dto.mode.as_deref() {
            None => MaskCombine::default(),
            Some(name) => MaskCombine::parse(name).ok_or_else(|| {
                m_at.key("mode")
                    .bad(format!("`{name}` is not `add`, `subtract` or `intersect`"))
            })?,
        };
        let wide = |v: i64, clamped: &mut bool| fit(v, i32::MIN, i32::MAX, clamped);
        let point = |p: [i64; 2], clamped: &mut bool| {
            MaskPoint::new(wide(p[0], clamped), wide(p[1], clamped))
        };
        let ratio = |v: i64, clamped: &mut bool| PerMille::new(fit(v, 0, u16::MAX, clamped));
        let mut mask = Mask::new(kind);
        let shape_only = |name: &str| {
            m_at.key(name)
                .bad(format!("a `{}` mask has no `{name}`", kind.as_str()))
        };
        match (kind, dto.roundness, dto.width) {
            (MaskShapeKind::Rectangle, Some(r), None) => {
                mask.shape = MaskShape::Rectangle {
                    roundness: ratio(r, clamped),
                };
            }
            (MaskShapeKind::Band, None, Some(w)) => {
                mask.shape = MaskShape::Band {
                    width: ratio(w, clamped),
                };
            }
            (MaskShapeKind::Rectangle, None, None) | (MaskShapeKind::Band, None, None) => {}
            // A rectangle takes a roundness, so a width is what's wrong.
            (MaskShapeKind::Rectangle, _, Some(_)) => return Err(shape_only("width")),
            (_, Some(_), _) => return Err(shape_only("roundness")),
            (_, _, Some(_)) => return Err(shape_only("width")),
            _ => {}
        }
        if let Some(c) = dto.center {
            mask.center = point(c, clamped);
        }
        if let Some(sz) = dto.size {
            mask.size = point(sz, clamped);
        }
        if let Some(linked) = dto.linked {
            mask.linked = linked;
        }
        if let Some(r) = dto.rotation {
            mask.rotation = Angle::from_hundredths(wide(r, clamped));
        }
        if let Some(f) = dto.feather {
            mask.feather = ratio(f, clamped);
        }
        mask.invert = dto.invert;
        let raw = ColorMask {
            id,
            mask,
            expand: fit(dto.expand.unwrap_or(0), i16::MIN, i16::MAX, clamped),
            mode,
        };
        let fixed = raw.clamped();
        *clamped |= fixed != raw;
        masks.push(fixed);
        // Filed by id, which reordering and removing masks keep.
        extras.put(owner, mask_path(id), dto.extra);
    }
    stack.masks = masks;
    Ok(())
}

/// Where a colour mask's unknown keys are filed: by its id, so a mask
/// keeps them when another is removed or the list is reordered. A mask
/// added later under the id of a removed one gets that one's keys back.
fn mask_path(id: u16) -> String {
    part(&format!("masks@{id}"))
}

fn write_masks(masks: &[ColorMask], give: &Give<'_>) -> Vec<ColorMaskDto> {
    masks
        .iter()
        .map(|m| {
            let kind = m.mask.kind();
            let plain = Mask::new(kind);
            let pair = |p: MaskPoint| [i64::from(p.x), i64::from(p.y)];
            let (roundness, width) = match m.mask.shape {
                MaskShape::Rectangle { roundness } => (
                    (roundness != PerMille::ZERO).then_some(i64::from(roundness.get())),
                    None,
                ),
                MaskShape::Band { width } => {
                    let default = MaskShape::of(MaskShapeKind::Band);
                    let changed = MaskShape::Band { width } != default;
                    (None, changed.then_some(i64::from(width.get())))
                }
                _ => (None, None),
            };
            ColorMaskDto {
                id: i64::from(m.id),
                shape: kind.as_str().to_string(),
                center: (m.mask.center != plain.center).then(|| pair(m.mask.center)),
                size: (m.mask.size != plain.size).then(|| pair(m.mask.size)),
                linked: (m.mask.linked != plain.linked).then_some(m.mask.linked),
                rotation: (m.mask.rotation != Angle::ZERO)
                    .then_some(i64::from(m.mask.rotation.hundredths())),
                feather: (m.mask.feather != PerMille::ZERO)
                    .then_some(i64::from(m.mask.feather.get())),
                invert: m.mask.invert,
                roundness,
                width,
                expand: (m.expand != 0).then_some(i64::from(m.expand)),
                mode: (m.mode != MaskCombine::default()).then(|| m.mode.as_str().to_string()),
                extra: give.at(&mask_path(m.id)),
            }
        })
        .collect()
}

/// The DTO of a scope's skin protection, unless it is at its defaults.
fn write_skin(skin: SkinProtect, give: &Give<'_>, path: &str) -> Option<SkinDto> {
    let dto = SkinDto {
        on: skin.on,
        amount: (skin.amount != SkinProtect::default().amount).then_some(i64::from(skin.amount)),
        extra: give.at(&part(path)),
    };
    let empty = !dto.on && dto.amount.is_none() && dto.extra.is_empty();
    (!empty).then_some(dto)
}

fn write_lut(slot: &LutSlot, give: &Give<'_>) -> LutDto {
    LutDto {
        hash: hash_hex(&slot.lut.hash),
        file: slot.lut.file_name.clone(),
        intensity: (slot.intensity != 100).then_some(i64::from(slot.intensity)),
        interp: (slot.interp != LutInterp::default()).then(|| slot.interp.as_str().to_string()),
        skin: write_skin(slot.skin, give, "lut.skin"),
        extra: give.at(&part("lut")),
    }
}

fn write_hsl(stack: &ColorStack, give: &Give<'_>) -> Option<HslDto> {
    let mut dto = HslDto {
        extra: give.at(&part("hsl")),
        ..HslDto::default()
    };
    for (band, field) in hsl_bands(&mut dto) {
        let mut b = HslBandDto {
            extra: give.at(&part(&format!("hsl.{}", band.as_str()))),
            ..HslBandDto::default()
        };
        let levels: &HslBand = stack.hsl.band(band);
        for (channel, v) in band_fields(&mut b) {
            let level = levels.get(channel);
            *v = (level != 0).then_some(i64::from(level));
        }
        let empty = b.hue.is_none()
            && b.saturation.is_none()
            && b.lightness.is_none()
            && b.extra.is_empty();
        *field = (!empty).then_some(b);
    }
    let empty = hsl_bands(&mut dto).iter().all(|(_, b)| b.is_none()) && dto.extra.is_empty();
    (!empty).then_some(dto)
}

fn write_curves(curves: &Curves, give: &Give<'_>) -> Option<CurvesDto> {
    let mut dto = CurvesDto {
        extra: give.at(&part("curves")),
        ..CurvesDto::default()
    };
    for (channel, field) in curve_fields(&mut dto) {
        let curve = curves.get(channel);
        *field = (!curve.is_default()).then(|| {
            curve
                .points()
                .iter()
                .map(|p| [i64::from(p.x), i64::from(p.y)])
                .collect()
        });
    }
    let empty = curve_fields(&mut dto).iter().all(|(_, c)| c.is_none()) && dto.extra.is_empty();
    (!empty).then_some(dto)
}

fn write_wheels(wheels: &ColorWheels, give: &Give<'_>) -> Option<WheelsDto> {
    let mut dto = WheelsDto {
        strength: (wheels.strength != 100).then_some(i64::from(wheels.strength)),
        extra: give.at(&part("wheels")),
        ..WheelsDto::default()
    };
    for (id, field) in wheel_fields(&mut dto) {
        let w: &Wheel = wheels.wheel(id);
        let nonzero = |v: i16| (v != 0).then_some(i64::from(v));
        let out = WheelDto {
            x: nonzero(w.x()),
            y: nonzero(w.y()),
            luma: nonzero(w.luma()),
            extra: give.at(&part(&format!("wheels.{}", id.as_str()))),
        };
        let empty =
            out.x.is_none() && out.y.is_none() && out.luma.is_none() && out.extra.is_empty();
        *field = (!empty).then_some(out);
    }
    let empty = dto.strength.is_none()
        && wheel_fields(&mut dto).iter().all(|(_, w)| w.is_none())
        && dto.extra.is_empty();
    (!empty).then_some(dto)
}

/// `v` as `id`'s level, clamped into its range.
fn level(id: ColorParamId, v: i64, clamped: &mut bool) -> i16 {
    let spec = id.spec();
    let wide = fit(v, i16::MIN, i16::MAX, clamped);
    let fixed = spec.clamp(wide);
    *clamped |= fixed != wide;
    fixed
}

/// The stack stored for a clip of `content`; `None` for the defaults.
pub(super) fn read_color(
    mut dto: ColorDto,
    at: &At,
    content: &ClipContent,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<Option<ColorStack>, OpenError> {
    if !content.takes_color() {
        return Err(at.bad(
            "only video, image and adjustment clips take colour adjustments; leave `color` out",
        ));
    }
    let mut stack = ColorStack {
        bypass: dto.bypass,
        ..ColorStack::default()
    };
    if let Some(amount) = dto.amount {
        stack = stack.with(
            ColorParamId::Amount,
            level(ColorParamId::Amount, amount, clamped),
        );
    }
    let off_at = at.key("off");
    for (i, name) in dto.off.iter().enumerate() {
        let Some(section) = Section::parse(name) else {
            return Err(off_at
                .index(i)
                .bad(format!("`{name}` is not a colour section")));
        };
        stack.sections_enabled = stack.sections_enabled.with(section, false);
    }
    if let Some(mut light) = dto.light.take() {
        for (id, v) in light_fields(&mut light) {
            if let Some(v) = v.take() {
                stack = stack.with(id, level(id, v, clamped));
            }
        }
        extras.put(owner, "color.light", light.extra);
    }
    if let Some(mut balance) = dto.color.take() {
        for (id, v) in balance_fields(&mut balance) {
            if let Some(v) = v.take() {
                stack = stack.with(id, level(id, v, clamped));
            }
        }
        if let Some(skin) = balance.skin.take() {
            read_skin(
                skin,
                SkinScope::Color,
                &mut stack,
                owner,
                "color.skin",
                extras,
                clamped,
            );
        }
        extras.put(owner, "color.color", balance.extra);
    }
    if let Some(hsl) = dto.hsl.take() {
        read_hsl(hsl, &mut stack, owner, extras, clamped);
    }
    if let Some(curves) = dto.curves.take() {
        read_curves(
            curves,
            &at.key("curves"),
            &mut stack,
            owner,
            extras,
            clamped,
        )?;
    }
    if let Some(wheels) = dto.wheels.take() {
        read_wheels(wheels, &mut stack, owner, extras, clamped);
    }
    if let Some(lut) = dto.lut.take() {
        read_lut(lut, &at.key("lut"), &mut stack, owner, extras, clamped)?;
    }
    if let Some(auto) = dto.auto.take() {
        read_auto(auto, &at.key("auto"), &mut stack, owner, extras, clamped)?;
    }
    if let Some(m) = dto.matched.take() {
        read_match(m, &at.key("match"), &mut stack, owner, extras, clamped)?;
    }
    let masks = std::mem::take(&mut dto.masks);
    if !masks.is_empty() {
        read_masks(masks, &at.key("masks"), &mut stack, owner, extras, clamped)?;
    }
    extras.put(owner, "color", dto.extra);
    Ok((stack != ColorStack::default()).then_some(stack))
}

/// The DTO of `stack` (`None`: the defaults), unless everything, unknown
/// keys included, is at its default.
pub(super) fn write_color(stack: Option<&ColorStack>, give: &Give<'_>) -> Option<ColorDto> {
    let stack = stack.cloned().unwrap_or_default();
    let mut light = LightDto {
        extra: give.at("color.light"),
        ..LightDto::default()
    };
    for (id, field) in light_fields(&mut light) {
        let v = stack.get(id);
        *field = (v != id.spec().default).then_some(i64::from(v));
    }
    let mut balance = ColorBalanceDto {
        skin: write_skin(stack.skin(SkinScope::Color), give, "color.skin"),
        extra: give.at("color.color"),
        ..ColorBalanceDto::default()
    };
    for (id, field) in balance_fields(&mut balance) {
        let v = stack.get(id);
        *field = (v != id.spec().default).then_some(i64::from(v));
    }
    let light_empty = stack.section_is_default(Section::Light) && light.extra.is_empty();
    // Skin protection's unknown keys keep the object even at its defaults.
    let balance_empty = stack.section_is_default(Section::Color)
        && balance.extra.is_empty()
        && balance.skin.is_none();
    let dto = ColorDto {
        auto: stack.auto.as_ref().map(|a| write_auto(a, give)),
        matched: stack.match_xf.as_ref().map(|m| write_match(m, give)),
        bypass: stack.bypass,
        amount: (stack.amount != 100).then_some(i64::from(stack.amount)),
        off: Section::ALL
            .into_iter()
            .filter(|s| !stack.sections_enabled.is_on(*s))
            .map(|s| s.as_str().to_string())
            .collect(),
        light: (!light_empty).then_some(light),
        color: (!balance_empty).then_some(balance),
        hsl: write_hsl(&stack, give),
        curves: write_curves(&stack.curves, give),
        wheels: write_wheels(&stack.wheels, give),
        lut: stack.lut.as_ref().map(|l| write_lut(l, give)),
        masks: write_masks(&stack.masks, give),
        extra: give.at("color"),
    };
    let empty = !dto.bypass
        && dto.amount.is_none()
        && dto.off.is_empty()
        && dto.light.is_none()
        && dto.color.is_none()
        && dto.hsl.is_none()
        && dto.curves.is_none()
        && dto.wheels.is_none()
        && dto.lut.is_none()
        && dto.auto.is_none()
        && dto.matched.is_none()
        && dto.masks.is_empty()
        && dto.extra.is_empty();
    (!empty).then_some(dto)
}
