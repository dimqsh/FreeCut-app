//! The service's model catalogue (CG-003, architecture §6.2).
//!
//! Parsed from `models_explore` listings, which are free and carry each
//! model's parameters, aspect ratios, durations and input roles (but no
//! prices). Every value is untrusted data: unknown fields are ignored, and
//! an entry that cannot be read is skipped rather than failing the whole
//! catalogue. Option controls in the UI are built from [`ModelInfo`], so new
//! models need no FreeCut release.
//!
//! Durations come in three shapes in the recorded listing (2026-10-14): a
//! top-level `durations` set, a top-level `duration_range`, or a `duration`
//! parameter with `options` or `min`/`max`. [`DurationSpec`] covers all three.

use std::fmt;

use serde::Deserialize;
use serde_json::{Number, Value};

/// A model id as the service names it (`seedance_2_5`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelId(String);

impl ModelId {
    /// Wraps an id.
    pub fn new(id: impl Into<String>) -> ModelId {
        ModelId(id.into())
    }

    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a model makes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputKind {
    /// Still images.
    Image,
    /// Video.
    Video,
    /// Audio.
    Audio,
    /// 3D assets.
    ThreeD,
    /// Anything else the service adds.
    Other(String),
}

impl OutputKind {
    fn parse(text: &str) -> OutputKind {
        match text {
            "image" => OutputKind::Image,
            "video" => OutputKind::Video,
            "audio" => OutputKind::Audio,
            "3d" => OutputKind::ThreeD,
            other => OutputKind::Other(other.to_owned()),
        }
    }

    /// The service's name for it (`image`, `video`, …).
    pub fn as_str(&self) -> &str {
        match self {
            OutputKind::Image => "image",
            OutputKind::Video => "video",
            OutputKind::Audio => "audio",
            OutputKind::ThreeD => "3d",
            OutputKind::Other(s) => s,
        }
    }
}

/// What an input is used for. The roles, not the listing's per-media `type`
/// field (which says "image" even for video inputs), say what a model takes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// The first frame of a video.
    StartImage,
    /// The last frame of a video.
    EndImage,
    /// Reference images.
    ImageReferences,
    /// Reference videos.
    VideoReferences,
    /// Reference audio.
    AudioReferences,
    /// An inpainting mask.
    Mask,
    /// A video to transform.
    InputVideo,
    /// Audio to transform.
    InputAudio,
    /// A single image input.
    Image,
    /// A single video input.
    Video,
    /// A role FreeCut does not know.
    Other(String),
}

impl Role {
    /// Reads the service's role name.
    pub fn parse(text: &str) -> Role {
        match text {
            "start_image" => Role::StartImage,
            "end_image" => Role::EndImage,
            "image_references" => Role::ImageReferences,
            "video_references" => Role::VideoReferences,
            "audio_references" => Role::AudioReferences,
            "mask" => Role::Mask,
            "input_video" => Role::InputVideo,
            "input_audio" => Role::InputAudio,
            "image" => Role::Image,
            "video" => Role::Video,
            other => Role::Other(other.to_owned()),
        }
    }

    /// The service's role name.
    pub fn as_str(&self) -> &str {
        match self {
            Role::StartImage => "start_image",
            Role::EndImage => "end_image",
            Role::ImageReferences => "image_references",
            Role::VideoReferences => "video_references",
            Role::AudioReferences => "audio_references",
            Role::Mask => "mask",
            Role::InputVideo => "input_video",
            Role::InputAudio => "input_audio",
            Role::Image => "image",
            Role::Video => "video",
            Role::Other(s) => s,
        }
    }
}

/// A group of media inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSlot {
    /// The argument name (`medias`).
    pub name: String,
    /// Roles accepted in it.
    pub roles: Vec<Role>,
    /// How many inputs it takes at most, if limited.
    pub max: Option<u32>,
    /// Whether the model needs at least one.
    pub required: bool,
}

/// A model setting's type and allowed values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamKind {
    /// Text, from `options` when given.
    Text {
        /// Allowed values (empty: free text).
        options: Vec<String>,
    },
    /// A number, from `options` or within `min`/`max`.
    Number {
        /// Allowed values (empty: any within bounds).
        options: Vec<Number>,
        /// Lower bound.
        min: Option<Number>,
        /// Upper bound.
        max: Option<Number>,
    },
    /// On or off.
    Bool,
    /// A list of text values.
    TextList,
    /// A type FreeCut does not know; such settings are not offered.
    Other(String),
}

/// One model setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// The argument name.
    pub name: String,
    /// Whether the model needs it.
    pub required: bool,
    /// Its type and allowed values.
    pub kind: ParamKind,
    /// The model's default.
    pub default: Option<Value>,
    /// Whether `null` is allowed.
    pub nullable: bool,
    /// The service's description (untrusted text).
    pub description: String,
}

/// Allowed durations in whole seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurationSpec {
    /// The model takes no duration.
    Unspecified,
    /// Exactly these values, ascending.
    Set(Vec<u32>),
    /// Any whole number in the range (no upper bound when `max` is `None`).
    Range {
        /// Shortest.
        min: u32,
        /// Longest.
        max: Option<u32>,
    },
}

/// A model's durations and default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Durations {
    /// What is allowed.
    pub spec: DurationSpec,
    /// The model's default, in seconds.
    pub default: Option<u32>,
}

/// A model that can make sound, and the setting that switches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSupport {
    /// The boolean setting (`generate_audio`, `sound`, …).
    pub param: String,
    /// Its default.
    pub default: Option<bool>,
}

/// Why FreeCut does not offer a listed model (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenReason {
    /// The service restricts it to its own pipelines.
    ServicePipelineOnly,
    /// It needs identity training (the `soul_*` family).
    NeedsIdentityTraining,
    /// Styles chosen in the service's own UI (marketing studio).
    ServiceStyles,
    /// Legacy preset recipes.
    LegacyPresets,
    /// It takes web addresses instead of media.
    WebAddressInput,
    /// A game-asset tool outside FreeCut's scope.
    OutOfScope,
    /// Dubbing, lip sync and voice change are not built (owner decision).
    VoiceToolNotBuilt,
}

/// One model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// The id.
    pub id: ModelId,
    /// The display name.
    pub display_name: String,
    /// The company behind it as the service reports it, if any.
    pub company: Option<String>,
    /// The service's description (untrusted text).
    pub description: String,
    /// What it makes.
    pub output: OutputKind,
    /// Its media inputs.
    pub media: Vec<MediaSlot>,
    /// Its durations.
    pub durations: Durations,
    /// Aspect ratios (`16:9`, `auto`, …).
    pub aspects: Vec<String>,
    /// Resolutions from its `resolution` setting.
    pub resolutions: Vec<String>,
    /// Whether and how it makes sound.
    pub audio: Option<AudioSupport>,
    /// All settings.
    pub params: Vec<ParamSpec>,
    /// Whether the account's free-trial allowance may cover it.
    pub supports_unlim: bool,
    /// The service's tags.
    pub tags: Vec<String>,
    /// Why it is hidden, if it is.
    pub hidden: Option<HiddenReason>,
}

impl ModelInfo {
    /// Every role any of its slots accepts.
    pub fn roles(&self) -> Vec<&Role> {
        let mut roles: Vec<&Role> = self.media.iter().flat_map(|m| &m.roles).collect();
        roles.sort();
        roles.dedup();
        roles
    }

    /// Whether it takes inputs in `role`.
    pub fn accepts(&self, role: &Role) -> bool {
        self.media.iter().any(|m| m.roles.contains(role))
    }

    /// A setting by name.
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Whether FreeCut offers it.
    pub fn is_visible(&self) -> bool {
        self.hidden.is_none()
    }
}

/// Why a listing could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogueError {
    /// Not a listing (no `items` array).
    #[error("not a model listing: {0}")]
    NotAListing(String),
}

/// Model counts by output type, for *Test connection* (CG-001).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelCounts {
    /// Image models offered.
    pub image: usize,
    /// Video models offered.
    pub video: usize,
    /// Audio models offered.
    pub audio: usize,
}

/// The models a picker shows, and how many inputs ruled out (CG-003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtered<'a> {
    /// Offered models of the output type that take every given input.
    pub models: Vec<&'a ModelInfo>,
    /// Models of the output type hidden because they cannot take an input.
    pub hidden_by_inputs: usize,
}

/// The parsed catalogue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalogue {
    models: Vec<ModelInfo>,
    skipped: usize,
}

/// One page of a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct ListingPage {
    /// The page's models.
    pub models: Vec<ModelInfo>,
    /// Entries that could not be read.
    pub skipped: usize,
    /// The cursor for the next page, when the service says there is one.
    pub next_page: Option<String>,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    provider_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    output_type: String,
    #[serde(default)]
    parameters: Vec<Value>,
    #[serde(default)]
    medias: Vec<RawMedia>,
    #[serde(default)]
    aspect_ratios: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    supports_unlim: bool,
    #[serde(default)]
    durations: Option<Vec<Value>>,
    #[serde(default)]
    duration_range: Option<RawRange>,
}

#[derive(Deserialize)]
struct RawMedia {
    #[serde(default)]
    name: String,
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Deserialize)]
struct RawRange {
    min: Option<i64>,
    max: Option<i64>,
}

#[derive(Deserialize)]
struct RawParam {
    name: String,
    #[serde(default)]
    required: Value,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    default: Option<Value>,
    #[serde(default)]
    options: Option<Vec<Value>>,
    #[serde(default)]
    min: Option<Number>,
    #[serde(default)]
    max: Option<Number>,
    #[serde(default)]
    nullable: bool,
}

/// Models and prefixes FreeCut hides (§6.2), plus voice tools the owner
/// decided not to build.
const HIDDEN_IDS: &[(&str, HiddenReason)] = &[
    ("sonilo_music", HiddenReason::ServicePipelineOnly),
    ("mirelo_text_to_audio", HiddenReason::ServicePipelineOnly),
    ("inworld_text_to_speech", HiddenReason::ServicePipelineOnly),
    ("higgsfield_preset", HiddenReason::LegacyPresets),
    ("autosprite", HiddenReason::OutOfScope),
    ("clipify", HiddenReason::WebAddressInput),
    ("dubbing", HiddenReason::VoiceToolNotBuilt),
    ("voice_change", HiddenReason::VoiceToolNotBuilt),
    ("sync_so", HiddenReason::VoiceToolNotBuilt),
];

const HIDDEN_PREFIXES: &[(&str, HiddenReason)] = &[
    ("soul_", HiddenReason::NeedsIdentityTraining),
    ("ms_", HiddenReason::ServiceStyles),
    ("marketing_studio", HiddenReason::ServiceStyles),
];

fn hidden_reason(id: &str, description: &str) -> Option<HiddenReason> {
    if let Some((_, r)) = HIDDEN_IDS.iter().find(|(i, _)| *i == id) {
        return Some(*r);
    }
    if let Some((_, r)) = HIDDEN_PREFIXES.iter().find(|(p, _)| id.starts_with(p)) {
        return Some(*r);
    }
    description
        .to_lowercase()
        .contains("pipeline only")
        .then_some(HiddenReason::ServicePipelineOnly)
}

fn positive_seconds(v: &Value) -> Option<u32> {
    v.as_u64()
        .filter(|&s| s > 0)
        .and_then(|s| u32::try_from(s).ok())
}

fn clamp_seconds(v: i64) -> u32 {
    u32::try_from(v.max(1)).unwrap_or(u32::MAX)
}

fn parse_param(value: &Value) -> Option<ParamSpec> {
    let raw: RawParam = serde_json::from_value(value.clone()).ok()?;
    let required = match &raw.required {
        Value::String(s) => s == "required",
        Value::Bool(b) => *b,
        _ => false,
    };
    let options = raw.options.unwrap_or_default();
    let kind = match raw.kind.as_str() {
        "string" => ParamKind::Text {
            options: options
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        },
        "number" | "integer" => ParamKind::Number {
            options: options
                .iter()
                .filter_map(|o| o.as_number().cloned())
                .collect(),
            min: raw.min,
            max: raw.max,
        },
        "bool" | "boolean" => ParamKind::Bool,
        "string_array" => ParamKind::TextList,
        other => ParamKind::Other(other.to_owned()),
    };
    Some(ParamSpec {
        name: raw.name,
        required,
        kind,
        default: raw.default,
        nullable: raw.nullable,
        description: raw.description,
    })
}

fn durations_of(raw: &RawModel, params: &[ParamSpec]) -> Durations {
    if let Some(set) = &raw.durations {
        let mut values: Vec<u32> = set.iter().filter_map(positive_seconds).collect();
        values.sort_unstable();
        values.dedup();
        if !values.is_empty() {
            return Durations {
                spec: DurationSpec::Set(values),
                default: None,
            };
        }
    }
    if let Some(range) = &raw.duration_range {
        return Durations {
            spec: DurationSpec::Range {
                min: clamp_seconds(range.min.unwrap_or(1)),
                max: range.max.map(clamp_seconds),
            },
            default: None,
        };
    }
    let Some(param) = params.iter().find(|p| p.name == "duration") else {
        return Durations {
            spec: DurationSpec::Unspecified,
            default: None,
        };
    };
    let default = param.default.as_ref().and_then(positive_seconds);
    let spec = match &param.kind {
        ParamKind::Number { options, min, max } => {
            let mut values: Vec<u32> = options
                .iter()
                .filter_map(|n| positive_seconds(&Value::Number(n.clone())))
                .collect();
            values.sort_unstable();
            values.dedup();
            if !values.is_empty() {
                DurationSpec::Set(values)
            } else if min.is_some() || max.is_some() {
                // A minimum below one second (one model lists -1) means
                // "shortest possible": FreeCut starts at 1 s.
                DurationSpec::Range {
                    min: min
                        .as_ref()
                        .and_then(Number::as_i64)
                        .map_or(1, clamp_seconds),
                    max: max.as_ref().and_then(Number::as_i64).map(clamp_seconds),
                }
            } else {
                DurationSpec::Range { min: 1, max: None }
            }
        }
        _ => DurationSpec::Unspecified,
    };
    Durations { spec, default }
}

const AUDIO_PARAMS: &[&str] = &["generate_audio", "sound", "with_sound", "audio"];

fn audio_of(params: &[ParamSpec]) -> Option<AudioSupport> {
    params
        .iter()
        .find(|p| p.kind == ParamKind::Bool && AUDIO_PARAMS.contains(&p.name.as_str()))
        .map(|p| AudioSupport {
            param: p.name.clone(),
            default: p.default.as_ref().and_then(Value::as_bool),
        })
}

fn parse_model(value: &Value) -> Option<ModelInfo> {
    let raw: RawModel = serde_json::from_value(value.clone()).ok()?;
    if raw.id.is_empty() {
        return None;
    }
    let params: Vec<ParamSpec> = raw.parameters.iter().filter_map(parse_param).collect();
    let durations = durations_of(&raw, &params);
    let resolutions = params
        .iter()
        .find(|p| p.name == "resolution")
        .and_then(|p| match &p.kind {
            ParamKind::Text { options } => Some(options.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let audio = audio_of(&params);
    let hidden = hidden_reason(&raw.id, &raw.description);
    Some(ModelInfo {
        display_name: if raw.name.is_empty() {
            raw.id.clone()
        } else {
            raw.name
        },
        company: Some(raw.provider_name).filter(|c| !c.is_empty()),
        description: raw.description,
        output: OutputKind::parse(&raw.output_type),
        media: raw
            .medias
            .into_iter()
            .map(|m| MediaSlot {
                name: m.name,
                roles: m.roles.iter().map(|r| Role::parse(r)).collect(),
                max: m.max,
                required: m.required,
            })
            .collect(),
        durations,
        aspects: raw.aspect_ratios,
        resolutions,
        audio,
        params,
        supports_unlim: raw.supports_unlim,
        tags: raw.tags,
        hidden,
        id: ModelId(raw.id),
    })
}

/// Reads one `models_explore` listing page.
///
/// # Errors
/// [`CatalogueError::NotAListing`] when there is no `items` array.
pub fn parse_listing(listing: &Value) -> Result<ListingPage, CatalogueError> {
    let items = listing
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| CatalogueError::NotAListing("no items array".into()))?;
    let mut models = Vec::with_capacity(items.len());
    let mut skipped = 0;
    for item in items {
        match parse_model(item) {
            Some(m) => models.push(m),
            None => skipped += 1,
        }
    }
    let has_more = listing
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next_page = listing
        .get("next_page_token")
        .and_then(Value::as_str)
        .filter(|t| has_more && !t.is_empty())
        .map(str::to_owned);
    Ok(ListingPage {
        models,
        skipped,
        next_page,
    })
}

impl Catalogue {
    /// Builds a catalogue from listing pages; a model listed twice keeps its
    /// first entry.
    pub fn from_pages(pages: Vec<ListingPage>) -> Catalogue {
        let mut catalogue = Catalogue::default();
        for page in pages {
            catalogue.skipped += page.skipped;
            for model in page.models {
                if catalogue.get(model.id.as_str()).is_none() {
                    catalogue.models.push(model);
                }
            }
        }
        catalogue
    }

    /// Builds a catalogue from raw listing replies.
    ///
    /// # Errors
    /// [`CatalogueError`] when a reply is not a listing.
    pub fn from_listings(listings: &[Value]) -> Result<Catalogue, CatalogueError> {
        let pages = listings
            .iter()
            .map(parse_listing)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Catalogue::from_pages(pages))
    }

    /// Every listed model, hidden ones included.
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    /// Entries that could not be read.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The models FreeCut offers.
    pub fn visible(&self) -> impl Iterator<Item = &ModelInfo> {
        self.models.iter().filter(|m| m.is_visible())
    }

    /// A model by id, hidden or not.
    pub fn get(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id.as_str() == id)
    }

    /// Offered models by output type.
    pub fn counts(&self) -> ModelCounts {
        let mut counts = ModelCounts::default();
        for m in self.visible() {
            match m.output {
                OutputKind::Image => counts.image += 1,
                OutputKind::Video => counts.video += 1,
                OutputKind::Audio => counts.audio += 1,
                _ => {}
            }
        }
        counts
    }

    /// Offered models of `output` that take every role in `inputs`.
    pub fn filter<'a>(&'a self, output: &OutputKind, inputs: &[Role]) -> Filtered<'a> {
        let mut models = Vec::new();
        let mut hidden_by_inputs = 0;
        for m in self.visible().filter(|m| &m.output == output) {
            if inputs.iter().all(|r| m.accepts(r)) {
                models.push(m);
            } else {
                hidden_by_inputs += 1;
            }
        }
        Filtered {
            models,
            hidden_by_inputs,
        }
    }

    /// The picker label: the display name, with the id in brackets when
    /// another offered model has the same name (CG-003).
    pub fn label(&self, model: &ModelInfo) -> String {
        let clash = self
            .visible()
            .any(|other| other.id != model.id && other.display_name == model.display_name);
        if clash {
            format!("{} ({})", model.display_name, model.id)
        } else {
            model.display_name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn model(extra: Value) -> ModelInfo {
        let mut base =
            json!({"id": "m", "name": "M", "output_type": "video", "parameters": [], "medias": []});
        if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                b.insert(k.clone(), v.clone());
            }
        }
        parse_model(&base).unwrap()
    }

    #[test]
    fn duration_shapes() {
        assert_eq!(
            model(json!({"durations": [10, 5]})).durations.spec,
            DurationSpec::Set(vec![5, 10])
        );
        assert_eq!(
            model(json!({"duration_range": {"min": 4, "max": 15}}))
                .durations
                .spec,
            DurationSpec::Range {
                min: 4,
                max: Some(15)
            }
        );
        let p = model(
            json!({"parameters": [{"name": "duration", "type": "number", "options": [6, 10], "default": 6}]}),
        );
        assert_eq!(
            p.durations,
            Durations {
                spec: DurationSpec::Set(vec![6, 10]),
                default: Some(6)
            }
        );
        let r = model(
            json!({"parameters": [{"name": "duration", "type": "number", "min": -1, "max": 30, "default": 5}]}),
        );
        assert_eq!(
            r.durations.spec,
            DurationSpec::Range {
                min: 1,
                max: Some(30)
            }
        );
        let open = model(
            json!({"parameters": [{"name": "duration", "type": "number", "min": 4, "default": 15}]}),
        );
        assert_eq!(
            open.durations.spec,
            DurationSpec::Range { min: 4, max: None }
        );
        let any = model(
            json!({"parameters": [{"name": "duration", "type": "number", "nullable": true}]}),
        );
        assert_eq!(
            any.durations.spec,
            DurationSpec::Range { min: 1, max: None }
        );
        assert_eq!(model(json!({})).durations.spec, DurationSpec::Unspecified);
    }

    #[test]
    fn audio_resolution_and_required_settings() {
        let m = model(json!({"parameters": [
            {"name": "resolution", "required": "optional", "type": "string", "options": ["480p", "720p"]},
            {"name": "generate_audio", "required": "optional", "type": "bool", "default": true},
            {"name": "output_width", "required": "required", "type": "number", "min": 1}
        ]}));
        assert_eq!(m.resolutions, vec!["480p", "720p"]);
        assert_eq!(
            m.audio,
            Some(AudioSupport {
                param: "generate_audio".into(),
                default: Some(true)
            })
        );
        assert!(m.param("output_width").unwrap().required);
    }

    #[test]
    fn hidden_models() {
        assert_eq!(
            hidden_reason("soul_2", ""),
            Some(HiddenReason::NeedsIdentityTraining)
        );
        assert_eq!(
            hidden_reason("ms_image", ""),
            Some(HiddenReason::ServiceStyles)
        );
        assert_eq!(
            hidden_reason("marketing_studio_v2_reference2video", ""),
            Some(HiddenReason::ServiceStyles)
        );
        assert_eq!(
            hidden_reason("dubbing", ""),
            Some(HiddenReason::VoiceToolNotBuilt)
        );
        assert_eq!(
            hidden_reason("x", "Text-to-music. Game pipeline only."),
            Some(HiddenReason::ServicePipelineOnly)
        );
        assert_eq!(hidden_reason("seedance_2_5", "video"), None);
        assert_eq!(hidden_reason("cinematic_studio_soul_cast", ""), None);
    }

    #[test]
    fn media_without_roles_and_unreadable_entries() {
        let listing = json!({"items": [
            {"id": "a", "name": "A", "output_type": "video", "medias": [{"name": "avatars", "type": "image"}]},
            {"name": "no id"},
            "not an object"
        ], "has_more": false});
        let page = parse_listing(&listing).unwrap();
        assert_eq!(page.models.len(), 1);
        assert_eq!(page.skipped, 2);
        assert!(page.models[0].media[0].roles.is_empty());
        assert!(parse_listing(&json!({"models": []})).is_err());
    }

    #[test]
    fn pagination_cursor_only_with_has_more() {
        let more = parse_listing(&json!({"items": [], "has_more": true, "next_page_token": "p2"}))
            .unwrap();
        assert_eq!(more.next_page.as_deref(), Some("p2"));
        let done = parse_listing(&json!({"items": [], "has_more": false, "next_page_token": "p2"}))
            .unwrap();
        assert_eq!(done.next_page, None);
    }

    #[test]
    fn filters_and_labels() {
        let listing = json!({"items": [
            {"id": "t2v", "name": "Same", "output_type": "video", "medias": []},
            {"id": "i2v", "name": "Same", "output_type": "video", "medias": [{"roles": ["start_image", "end_image"]}]},
            {"id": "img", "name": "Img", "output_type": "image", "medias": [{"roles": ["image_references"]}]},
            {"id": "soul_x", "name": "Hidden", "output_type": "image"}
        ]});
        let c = Catalogue::from_listings(&[listing]).unwrap();
        assert_eq!(
            c.counts(),
            ModelCounts {
                image: 1,
                video: 2,
                audio: 0
            }
        );
        let f = c.filter(&OutputKind::Video, &[Role::StartImage]);
        assert_eq!(
            f.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["i2v"]
        );
        assert_eq!(f.hidden_by_inputs, 1);
        assert_eq!(c.label(c.get("t2v").unwrap()), "Same (t2v)");
        assert_eq!(c.label(c.get("img").unwrap()), "Img");
        assert_eq!(c.filter(&OutputKind::Image, &[]).models.len(), 1);
    }
}
