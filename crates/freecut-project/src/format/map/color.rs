//! The colour stack of a clip (format 10, CL-001; format 11 adds HSL,
//! Curves and Wheels; format 12 the LUT slot and skin protection): read
//! and written with every default left out. A LUT hash that isn't 32 hex
//! digits and an unknown interpolation are refused at their path.
//! Values outside a slider's range are clamped into it, curve coordinates
//! into 0–10000 and pucks onto their disk (and counted like other clamped
//! values); a list of points that is no curve (CL-014's rules) and a stack
//! on a clip that can't carry one (text, sound, stickers, effects) are
//! refused at their path; a stack that reads back to the defaults is
//! stored as none.

use freecut_timeline::model::{
    ClipContent, ColorParamId, ColorStack, ColorWheels, Curve, CurveChannel, CurvePoint, Curves,
    HslBand, HslChannel, HueBand, LutInterp, LutRef, LutSlot, Section, SkinProtect, SkinScope,
    Wheel, WheelId, hash_hex, parse_hash_hex,
};

use super::{At, Give, fit};
use crate::extras::{Extras, Owner};
use crate::format::dto::{
    ColorBalanceDto, ColorDto, CurvesDto, HslBandDto, HslDto, LightDto, LutDto, SkinDto, WheelDto,
    WheelsDto,
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
    let balance_empty = stack.section_is_default(Section::Color) && balance.extra.is_empty();
    let dto = ColorDto {
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
        && dto.extra.is_empty();
    (!empty).then_some(dto)
}
