//! FreeCut's operations as Higgsfield tool calls (architecture §6.4).
//!
//! Arguments are built only from the [`Operation`] and the model's catalogue
//! entry: `params.model`, `params.prompt`, `params.medias` (ids, never URLs),
//! `aspect_ratio`, `count`, `duration` in whole seconds, and model settings
//! at the top level of `params`. Fields the service offers for its own
//! analytics, folders, presets or allowances are never set here; `get_cost`
//! is set only by [`QuoteRequest`](crate::guard::QuoteRequest).

use freecut_timeline::time::{TICKS_PER_SECOND, Tick};
use serde_json::{Map, Value, json};

use super::allow::{GENERATE_IMAGE, GENERATE_VIDEO};
use crate::catalogue::{DurationSpec, ModelInfo, OutputKind, ParamKind, ParamSpec};
use crate::mcp::AllowedTool;
use crate::provider::{ImageRequest, Operation, ParamValue, VideoMode, VideoRequest, WholeSeconds};

/// The longest prompt FreeCut sends, in characters (CG-006).
pub const MAX_PROMPT_CHARS: usize = 4_000;
/// The most variants per run (CG-006).
pub const MAX_VARIANTS: u8 = 4;

/// Arguments FreeCut sets itself or never sends; model settings with these
/// names are refused.
const RESERVED: &[&str] = &[
    "model",
    "prompt",
    "medias",
    "aspect_ratio",
    "count",
    "get_cost",
    "use_unlim",
    "use_free_gens",
    "context",
    "folder_id",
    "preset_id",
    "preset_session_id",
    "declined_preset_id",
    "draft_job_id",
];

/// Video fields of their own; for image models `resolution` is an ordinary
/// setting.
const VIDEO_FIELDS: &[&str] = &["duration", "resolution", "mode", "extension_mode"];

/// An allow-listed tool and its arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The tool.
    pub tool: &'static AllowedTool,
    /// Its arguments (`{"params": {...}}`).
    pub arguments: Value,
}

/// An operation the model cannot take as asked. The UI's controls come from
/// the same catalogue entry, so this means stale UI state or a bug.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    /// The model makes something else.
    #[error("{model} does not make {expected}")]
    WrongOutput {
        /// The model.
        model: String,
        /// What the operation wants.
        expected: &'static str,
    },
    /// No prompt and no inputs.
    #[error("a prompt or an input is needed")]
    EmptyPrompt,
    /// Longer than [`MAX_PROMPT_CHARS`].
    #[error("the prompt is longer than {MAX_PROMPT_CHARS} characters")]
    PromptTooLong,
    /// Outside 1–4.
    #[error("{0} variants requested; 1 to {MAX_VARIANTS} are possible")]
    Variants(u8),
    /// The model takes no input in this role.
    #[error("the model takes no {0} input")]
    RoleNotAccepted(String),
    /// Not one of the model's aspect ratios.
    #[error("the model does not offer the aspect ratio {0}")]
    AspectNotOffered(String),
    /// Not one of the model's durations.
    #[error("the model does not offer {0} s")]
    DurationNotOffered(u32),
    /// Not one of the model's resolutions.
    #[error("the model does not offer {0}")]
    ResolutionNotOffered(String),
    /// Sound requested from a model that cannot make it.
    #[error("the model cannot make sound")]
    NoSound,
    /// The model has no such mode.
    #[error("the model has no {0} mode")]
    ModeNotOffered(&'static str),
    /// A setting the model does not have.
    #[error("the model has no setting {0}")]
    UnknownParam(String),
    /// A setting FreeCut sets itself or never sends.
    #[error("the setting {0} cannot be set directly")]
    ReservedParam(String),
    /// A value the setting does not allow.
    #[error("{name}: {problem}")]
    BadParamValue {
        /// The setting.
        name: String,
        /// What is wrong.
        problem: String,
    },
    /// A required setting is missing.
    #[error("the model needs the setting {0}")]
    MissingParam(String),
}

/// Maps an operation to its tool call for `model`.
///
/// # Errors
/// [`MapError`] when the model cannot take the operation as asked.
pub fn tool_call(op: &Operation, model: &ModelInfo) -> Result<ToolCall, MapError> {
    match op {
        Operation::Image(r) => image_call(r, model),
        Operation::Video(r) => video_call(r, model),
    }
}

fn common(
    model: &ModelInfo,
    prompt: &str,
    refs: &[crate::provider::RoleRef],
    variants: u8,
) -> Result<Map<String, Value>, MapError> {
    if !(1..=MAX_VARIANTS).contains(&variants) {
        return Err(MapError::Variants(variants));
    }
    if prompt.trim().is_empty() && refs.is_empty() {
        return Err(MapError::EmptyPrompt);
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(MapError::PromptTooLong);
    }
    let mut params = Map::new();
    params.insert("model".into(), model.id.as_str().into());
    if !prompt.trim().is_empty() {
        params.insert("prompt".into(), prompt.into());
    }
    if !refs.is_empty() {
        let mut medias = Vec::with_capacity(refs.len());
        for r in refs {
            if !model.accepts(&r.role) {
                return Err(MapError::RoleNotAccepted(r.role.as_str().to_owned()));
            }
            medias.push(json!({ "value": r.media.as_str(), "role": r.role.as_str() }));
        }
        params.insert("medias".into(), Value::Array(medias));
    }
    params.insert("count".into(), variants.into());
    Ok(params)
}

fn aspect(
    params: &mut Map<String, Value>,
    model: &ModelInfo,
    aspect: Option<&crate::provider::Aspect>,
) -> Result<(), MapError> {
    if let Some(a) = aspect {
        if !model.aspects.is_empty() && !model.aspects.iter().any(|m| m == a.as_str()) {
            return Err(MapError::AspectNotOffered(a.as_str().to_owned()));
        }
        params.insert("aspect_ratio".into(), a.as_str().into());
    }
    Ok(())
}

fn image_call(r: &ImageRequest, model: &ModelInfo) -> Result<ToolCall, MapError> {
    if model.output != OutputKind::Image {
        return Err(MapError::WrongOutput {
            model: model.id.to_string(),
            expected: "images",
        });
    }
    let mut params = common(model, &r.prompt, &r.refs, r.variants)?;
    aspect(&mut params, model, r.aspect.as_ref())?;
    settings(&mut params, model, &r.params, &[])?;
    Ok(ToolCall {
        tool: &GENERATE_IMAGE,
        arguments: json!({ "params": params }),
    })
}

/// The service's mode values (from `seedance_2_5`'s listing, 2026-10-14).
fn mode_values(mode: VideoMode) -> (&'static str, Option<&'static str>) {
    match mode {
        VideoMode::TextToVideo => ("t2v", None),
        VideoMode::FramesToVideo => ("omni_reference", None),
        VideoMode::EditByPrompt => ("video_edit", None),
        VideoMode::Extend { forward: true } => ("video_extension", Some("forward")),
        VideoMode::Extend { forward: false } => ("video_extension", Some("backward")),
    }
}

fn text_options(spec: Option<&ParamSpec>) -> Option<&[String]> {
    match spec.map(|p| &p.kind) {
        Some(ParamKind::Text { options }) => Some(options),
        _ => None,
    }
}

fn video_call(r: &VideoRequest, model: &ModelInfo) -> Result<ToolCall, MapError> {
    if model.output != OutputKind::Video {
        return Err(MapError::WrongOutput {
            model: model.id.to_string(),
            expected: "video",
        });
    }
    let mut params = common(model, &r.prompt, &r.refs, r.variants)?;
    let (mode, extension) = mode_values(r.mode);
    let offered_modes = text_options(model.param("mode"));
    let mode_offered = offered_modes.is_some_and(|o| o.iter().any(|m| m == mode));
    let needs_mode = matches!(r.mode, VideoMode::EditByPrompt | VideoMode::Extend { .. });
    if mode_offered {
        params.insert("mode".into(), mode.into());
    } else if needs_mode {
        return Err(MapError::ModeNotOffered(mode));
    }
    if let Some(direction) = extension {
        if !text_options(model.param("extension_mode"))
            .is_some_and(|o| o.iter().any(|d| d == direction))
        {
            return Err(MapError::ModeNotOffered(mode));
        }
        params.insert("extension_mode".into(), direction.into());
    }
    // Edits keep the source's length and shape; extensions its shape.
    let keeps_aspect = needs_mode;
    let keeps_length = r.mode == VideoMode::EditByPrompt;
    if !keeps_aspect {
        aspect(&mut params, model, r.aspect.as_ref())?;
    }
    if let (Some(WholeSeconds(secs)), false) = (r.duration, keeps_length) {
        let allowed = match &model.durations.spec {
            DurationSpec::Unspecified => false,
            DurationSpec::Set(values) => values.contains(&secs),
            DurationSpec::Range { min, max } => secs >= *min && max.is_none_or(|m| secs <= m),
        };
        if !allowed {
            return Err(MapError::DurationNotOffered(secs));
        }
        params.insert("duration".into(), secs.into());
    }
    if let Some(res) = &r.resolution {
        if !model.resolutions.iter().any(|m| m == res) {
            return Err(MapError::ResolutionNotOffered(res.clone()));
        }
        params.insert("resolution".into(), res.as_str().into());
    }
    let audio_param = model.audio.as_ref().map(|a| a.param.as_str());
    match (r.sound, audio_param) {
        (Some(on), Some(name)) => {
            params.insert(name.into(), on.into());
        }
        (Some(true), None) => return Err(MapError::NoSound),
        _ => {}
    }
    let mut video_fields = VIDEO_FIELDS.to_vec();
    video_fields.extend(audio_param);
    settings(&mut params, model, &r.params, &video_fields)?;
    Ok(ToolCall {
        tool: &GENERATE_VIDEO,
        arguments: json!({ "params": params }),
    })
}

/// Adds model settings after checking them against the catalogue.
fn settings(
    params: &mut Map<String, Value>,
    model: &ModelInfo,
    values: &crate::provider::ParamValues,
    also_reserved: &[&str],
) -> Result<(), MapError> {
    for (name, value) in values {
        if RESERVED.contains(&name.as_str()) || also_reserved.contains(&name.as_str()) {
            return Err(MapError::ReservedParam(name.clone()));
        }
        let spec = model
            .param(name)
            .ok_or_else(|| MapError::UnknownParam(name.clone()))?;
        params.insert(name.clone(), checked_value(spec, value)?);
    }
    for spec in model.params.iter().filter(|p| p.required) {
        if !RESERVED.contains(&spec.name.as_str()) && !params.contains_key(&spec.name) {
            return Err(MapError::MissingParam(spec.name.clone()));
        }
    }
    Ok(())
}

fn checked_value(spec: &ParamSpec, value: &ParamValue) -> Result<Value, MapError> {
    let bad = |problem: &str| MapError::BadParamValue {
        name: spec.name.clone(),
        problem: problem.to_owned(),
    };
    match (&spec.kind, value) {
        (ParamKind::Text { options }, ParamValue::Text(t)) => {
            if !options.is_empty() && !options.contains(t) {
                return Err(bad("not one of the model's values"));
            }
            Ok(Value::String(t.clone()))
        }
        (ParamKind::Number { options, min, max }, ParamValue::Integer(n)) => {
            if !options.is_empty() && !options.iter().any(|o| o.as_i64() == Some(*n)) {
                return Err(bad("not one of the model's values"));
            }
            if min
                .as_ref()
                .and_then(serde_json::Number::as_i64)
                .is_some_and(|m| *n < m)
            {
                return Err(bad("below the model's minimum"));
            }
            if max
                .as_ref()
                .and_then(serde_json::Number::as_i64)
                .is_some_and(|m| *n > m)
            {
                return Err(bad("above the model's maximum"));
            }
            Ok(Value::from(*n))
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => Ok(Value::Bool(*b)),
        (ParamKind::TextList, ParamValue::TextList(items)) => Ok(json!(items)),
        _ => Err(bad("wrong type for this setting")),
    }
}

/// The allowed duration nearest to `length`, in whole seconds: exact integer
/// math on ticks, ties go to the longer value (§6.4). Lengths at or below
/// zero give the shortest allowed duration.
pub fn snap_duration(length: Tick, spec: &DurationSpec) -> WholeSeconds {
    let ticks = i128::from(length.raw().max(0));
    let per_second = i128::from(TICKS_PER_SECOND);
    let rounded = u32::try_from((ticks + per_second / 2) / per_second).unwrap_or(u32::MAX);
    match spec {
        DurationSpec::Set(values) if !values.is_empty() => {
            let mut best = values[0];
            let mut best_distance = i128::MAX;
            for &v in values {
                let distance = (ticks - i128::from(v) * per_second).abs();
                if distance < best_distance || (distance == best_distance && v > best) {
                    best = v;
                    best_distance = distance;
                }
            }
            WholeSeconds(best)
        }
        DurationSpec::Range { min, max } => {
            WholeSeconds(rounded.max(*min).min(max.unwrap_or(u32::MAX)).max(1))
        }
        _ => WholeSeconds(rounded.max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{Catalogue, Role};
    use crate::provider::{Aspect, ParamValues, RemoteMediaId, RoleRef};
    use pretty_assertions::assert_eq;

    fn secs_ticks(num: i64, den: i64) -> Tick {
        Tick::from_raw(TICKS_PER_SECOND * num / den)
    }

    #[test]
    fn snapping_to_a_set_picks_the_nearest_and_ties_go_up() {
        let set = DurationSpec::Set(vec![4, 6, 8]);
        assert_eq!(snap_duration(secs_ticks(5, 1), &set), WholeSeconds(6));
        assert_eq!(snap_duration(secs_ticks(49, 10), &set), WholeSeconds(4));
        assert_eq!(snap_duration(secs_ticks(51, 10), &set), WholeSeconds(6));
        assert_eq!(snap_duration(secs_ticks(100, 1), &set), WholeSeconds(8));
        assert_eq!(snap_duration(Tick::ZERO, &set), WholeSeconds(4));
        assert_eq!(snap_duration(Tick::from_raw(-5), &set), WholeSeconds(4));
    }

    #[test]
    fn snapping_to_a_range_rounds_half_up_and_clamps() {
        let range = DurationSpec::Range {
            min: 4,
            max: Some(15),
        };
        assert_eq!(snap_duration(secs_ticks(55, 10), &range), WholeSeconds(6));
        assert_eq!(snap_duration(secs_ticks(54, 10), &range), WholeSeconds(5));
        assert_eq!(snap_duration(secs_ticks(1, 1), &range), WholeSeconds(4));
        assert_eq!(snap_duration(secs_ticks(40, 1), &range), WholeSeconds(15));
        // One tick short of 5.5 s rounds down.
        assert_eq!(
            snap_duration(Tick::from_raw(TICKS_PER_SECOND * 11 / 2 - 1), &range),
            WholeSeconds(5)
        );
        let open = DurationSpec::Range { min: 4, max: None };
        assert_eq!(snap_duration(secs_ticks(60, 1), &open), WholeSeconds(60));
        assert_eq!(
            snap_duration(Tick::ZERO, &DurationSpec::Unspecified),
            WholeSeconds(1)
        );
    }

    fn catalogue() -> Catalogue {
        Catalogue::from_listings(&[json!({"items": [
            {"id": "img", "name": "Img", "output_type": "image",
             "parameters": [{"name": "quality", "required": "optional", "type": "string", "options": ["low", "high"]},
                            {"name": "seed", "required": "optional", "type": "number", "min": 0, "max": 10}],
             "medias": [{"name": "medias", "type": "image", "roles": ["image_references"]}],
             "aspect_ratios": ["1:1", "16:9"]},
            {"id": "vid", "name": "Vid", "output_type": "video",
             "parameters": [
                {"name": "mode", "required": "optional", "type": "string", "options": ["t2v", "omni_reference", "video_edit", "video_extension"]},
                {"name": "extension_mode", "required": "optional", "type": "string", "options": ["backward", "forward"], "nullable": true},
                {"name": "duration", "required": "optional", "type": "number", "min": 4, "max": 30, "default": 5},
                {"name": "resolution", "required": "optional", "type": "string", "options": ["480p", "720p"]},
                {"name": "generate_audio", "required": "optional", "type": "bool", "default": true}],
             "medias": [{"name": "medias", "type": "image", "roles": ["start_image", "video_references"]}],
             "aspect_ratios": ["16:9", "9:16"]},
            {"id": "plain", "name": "Plain", "output_type": "video", "parameters": [], "medias": [], "aspect_ratios": []},
            {"id": "needy", "name": "Needy", "output_type": "image",
             "parameters": [{"name": "output_width", "required": "required", "type": "number", "min": 1}]}
        ]})])
        .unwrap()
    }

    fn image(model: &str) -> ImageRequest {
        ImageRequest {
            model: crate::catalogue::ModelId::new(model),
            prompt: "a red apple".into(),
            refs: vec![],
            aspect: Aspect::parse("1:1"),
            variants: 1,
            params: ParamValues::new(),
        }
    }

    fn video(mode: VideoMode) -> VideoRequest {
        VideoRequest {
            model: crate::catalogue::ModelId::new("vid"),
            mode,
            prompt: "waves".into(),
            refs: vec![],
            aspect: Aspect::parse("16:9"),
            duration: Some(WholeSeconds(5)),
            resolution: Some("720p".into()),
            sound: Some(false),
            variants: 1,
            params: ParamValues::new(),
        }
    }

    fn call(op: Operation) -> Result<ToolCall, MapError> {
        let c = catalogue();
        let model = c.get(op.model().as_str()).unwrap().clone();
        tool_call(&op, &model)
    }

    #[test]
    fn an_image_call() {
        let mut r = image("img");
        r.params
            .insert("quality".into(), ParamValue::Text("high".into()));
        r.refs.push(RoleRef {
            role: Role::ImageReferences,
            media: RemoteMediaId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap(),
        });
        assert_eq!(
            call(Operation::Image(r)).unwrap(),
            ToolCall {
                tool: &GENERATE_IMAGE,
                arguments: json!({"params": {
                    "model": "img", "prompt": "a red apple", "count": 1, "aspect_ratio": "1:1", "quality": "high",
                    "medias": [{"value": "123e4567-e89b-12d3-a456-426614174000", "role": "image_references"}]
                }})
            }
        );
    }

    #[test]
    fn video_modes() {
        let t2v = call(Operation::Video(video(VideoMode::TextToVideo))).unwrap();
        assert_eq!(
            t2v.arguments,
            json!({"params": {"model": "vid", "prompt": "waves", "count": 1, "mode": "t2v", "aspect_ratio": "16:9",
                "duration": 5, "resolution": "720p", "generate_audio": false}})
        );
        let ext = call(Operation::Video(video(VideoMode::Extend {
            forward: false,
        })))
        .unwrap();
        assert_eq!(ext.arguments["params"]["mode"], "video_extension");
        assert_eq!(ext.arguments["params"]["extension_mode"], "backward");
        assert!(ext.arguments["params"].get("aspect_ratio").is_none());
        let edit = call(Operation::Video(video(VideoMode::EditByPrompt))).unwrap();
        assert!(edit.arguments["params"].get("duration").is_none());
        let mut plain = video(VideoMode::TextToVideo);
        plain.model = crate::catalogue::ModelId::new("plain");
        plain.duration = None;
        plain.resolution = None;
        plain.sound = None;
        plain.aspect = None;
        assert_eq!(
            call(Operation::Video(plain.clone())).unwrap().arguments,
            json!({"params": {"model": "plain", "prompt": "waves", "count": 1}})
        );
        plain.mode = VideoMode::Extend { forward: true };
        assert_eq!(
            call(Operation::Video(plain)),
            Err(MapError::ModeNotOffered("video_extension"))
        );
    }

    #[test]
    fn refusals() {
        let mut r = image("img");
        r.prompt = "  ".into();
        assert_eq!(call(Operation::Image(r)), Err(MapError::EmptyPrompt));
        let mut r = image("img");
        r.variants = 5;
        assert_eq!(call(Operation::Image(r)), Err(MapError::Variants(5)));
        let mut r = image("img");
        r.aspect = Aspect::parse("21:9");
        assert_eq!(
            call(Operation::Image(r)),
            Err(MapError::AspectNotOffered("21:9".into()))
        );
        let mut r = image("img");
        r.params
            .insert("folder_id".into(), ParamValue::Text("x".into()));
        assert_eq!(
            call(Operation::Image(r)),
            Err(MapError::ReservedParam("folder_id".into()))
        );
        let mut r = image("img");
        r.params.insert("seed".into(), ParamValue::Integer(11));
        assert!(matches!(
            call(Operation::Image(r)),
            Err(MapError::BadParamValue { .. })
        ));
        let mut r = image("img");
        r.params.insert("nope".into(), ParamValue::Bool(true));
        assert_eq!(
            call(Operation::Image(r)),
            Err(MapError::UnknownParam("nope".into()))
        );
        assert_eq!(
            call(Operation::Image(image("needy"))),
            Err(MapError::MissingParam("output_width".into()))
        );
        let mut r = image("img");
        r.refs.push(RoleRef {
            role: Role::Mask,
            media: RemoteMediaId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap(),
        });
        assert_eq!(
            call(Operation::Image(r)),
            Err(MapError::RoleNotAccepted("mask".into()))
        );
        let mut v = video(VideoMode::TextToVideo);
        v.duration = Some(WholeSeconds(31));
        assert_eq!(
            call(Operation::Video(v)),
            Err(MapError::DurationNotOffered(31))
        );
        let mut v = video(VideoMode::TextToVideo);
        v.params
            .insert("generate_audio".into(), ParamValue::Bool(true));
        assert_eq!(
            call(Operation::Video(v)),
            Err(MapError::ReservedParam("generate_audio".into()))
        );
        assert!(matches!(
            call(Operation::Image(ImageRequest {
                model: crate::catalogue::ModelId::new("vid"),
                ..image("img")
            })),
            Err(MapError::WrongOutput { .. })
        ));
    }
}
