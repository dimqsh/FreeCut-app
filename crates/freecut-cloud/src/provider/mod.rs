//! The provider-neutral generation interface (architecture §6.1) and the
//! operations FreeCut asks for (§6.4). H1 covers connecting, the catalogue
//! and cost quotes; submission and polling come in H2.

pub mod higgsfield;
#[cfg(any(test, feature = "test-support"))]
pub mod scripted;

use std::collections::BTreeMap;
use std::fmt;

use freecut_media::cancel::CancelToken;
use serde_json::Value;

use crate::catalogue::{Catalogue, ModelId, OutputKind, Role};
use crate::credits::Credits;
use crate::error::CloudError;
use crate::guard::is_uuid;
use crate::secret::ProviderId;

/// A duration in whole seconds: the only place a FreeCut duration stops
/// being ticks (§6.4, *Time at the edge*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WholeSeconds(pub u32);

/// An aspect ratio as the service writes it: `W:H` or `auto`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Aspect(String);

impl Aspect {
    /// Parses `W:H` (positive integers) or `auto`.
    pub fn parse(text: &str) -> Option<Aspect> {
        if text == "auto" {
            return Some(Aspect(text.to_owned()));
        }
        let (w, h) = text.split_once(':')?;
        let positive = |s: &str| {
            !s.is_empty()
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.parse::<u32>().is_ok_and(|v| v > 0)
        };
        (positive(w) && positive(h)).then(|| Aspect(text.to_owned()))
    }

    /// The text form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A media id the service issued (an upload or a finished job). Never a URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteMediaId(String);

impl RemoteMediaId {
    /// Accepts UUIDs only.
    pub fn parse(text: &str) -> Option<RemoteMediaId> {
        is_uuid(text).then(|| RemoteMediaId(text.to_ascii_lowercase()))
    }

    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An input in a role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRef {
    /// What it is used for.
    pub role: Role,
    /// The uploaded media or prior result.
    pub media: RemoteMediaId,
}

/// A model setting's value, built from the catalogue's [`ParamSpec`](crate::catalogue::ParamSpec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamValue {
    /// Text.
    Text(String),
    /// A whole number.
    Integer(i64),
    /// On or off.
    Bool(bool),
    /// A list of text values.
    TextList(Vec<String>),
}

/// Model settings by name.
pub type ParamValues = BTreeMap<String, ParamValue>;

/// How a video is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    /// From the prompt only.
    TextToVideo,
    /// From start and/or end frames or references (CG-007).
    FramesToVideo,
    /// Editing a clip section by prompt (CG-015, H3).
    EditByPrompt,
    /// Extending a clip (CG-016, H3).
    Extend {
        /// Forward (after the clip) or backward.
        forward: bool,
    },
}

/// An image generation (CG-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    /// The model.
    pub model: ModelId,
    /// The prompt.
    pub prompt: String,
    /// Reference inputs.
    pub refs: Vec<RoleRef>,
    /// The aspect ratio, already snapped to one the model offers.
    pub aspect: Option<Aspect>,
    /// Variants (1–4).
    pub variants: u8,
    /// Other model settings.
    pub params: ParamValues,
}

/// A video generation (CG-007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoRequest {
    /// The model.
    pub model: ModelId,
    /// How the video is made.
    pub mode: VideoMode,
    /// The prompt.
    pub prompt: String,
    /// Inputs (frames, references).
    pub refs: Vec<RoleRef>,
    /// The aspect ratio, already snapped to one the model offers.
    pub aspect: Option<Aspect>,
    /// The length, already snapped with
    /// [`snap_duration`](higgsfield::map::snap_duration).
    pub duration: Option<WholeSeconds>,
    /// One of the model's resolutions.
    pub resolution: Option<String>,
    /// Whether to make sound, for models that can.
    pub sound: Option<bool>,
    /// Variants (1–4).
    pub variants: u8,
    /// Other model settings.
    pub params: ParamValues,
}

/// What FreeCut asks a provider to do (H1: images and video).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Generate images.
    Image(ImageRequest),
    /// Generate a video.
    Video(VideoRequest),
}

impl Operation {
    /// The model.
    pub fn model(&self) -> &ModelId {
        match self {
            Operation::Image(r) => &r.model,
            Operation::Video(r) => &r.model,
        }
    }

    /// Requested variants.
    pub fn variants(&self) -> u8 {
        match self {
            Operation::Image(r) => r.variants,
            Operation::Video(r) => r.variants,
        }
    }

    /// What it makes.
    pub fn output(&self) -> OutputKind {
        match self {
            Operation::Image(_) => OutputKind::Image,
            Operation::Video(_) => OutputKind::Video,
        }
    }
}

/// The result of connecting (CG-001's *Test connection*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    /// The server's name.
    pub server_name: String,
    /// The server's version.
    pub server_version: String,
    /// The negotiated MCP revision.
    pub protocol_version: String,
    /// Allow-listed tools the service offers.
    pub offered: Vec<String>,
    /// Tools FreeCut needs that the service does not offer; their actions are
    /// disabled with "Not offered by the service right now".
    pub missing: Vec<String>,
}

/// A change the service made to the request (CG-026), as FreeCut's summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adjustment {
    /// What changed, as plain text.
    pub summary: String,
}

/// A cost estimate (CG-005).
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    /// The service's figure.
    pub credits: Credits,
    /// How many variants the figure covers.
    pub variants_covered: u8,
    /// Changes the service announced.
    pub adjustments: Vec<Adjustment>,
    /// The tool asked.
    pub tool: String,
    /// The exact arguments sent (for *Show last request*; no tokens).
    pub arguments: Value,
}

/// A question the service asks before it would run (CG-026).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceQuestion {
    /// Free-trial allowance or credits? FreeCut never answers on its own.
    UseAllowance {
        /// Variants the allowance is limited to, if it is.
        cap_variants_to: Option<u8>,
    },
}

/// Why no estimate can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The tool's live schema no longer offers a cost check (§4.7 rule 2).
    NoEstimate,
    /// A cost check of this tool started, or may have started, a run
    /// earlier; checks stay off until FreeCut restarts (§4.7 rule 4).
    PreflightStopped,
    /// The estimate does not cover more than `max` variants (CG-005).
    VariantsNotCovered {
        /// The variants an estimate covers.
        max: u8,
    },
    /// The service does not offer the tool.
    ToolMissing {
        /// The tool.
        tool: String,
    },
    /// The service asked for another step first (CG-026).
    FollowUp {
        /// The tool it named (untrusted text).
        tool: String,
    },
}

/// A cost check whose answer carried job ids: the service may have started
/// (and charged for) a run. The caller records it like any run and tells the
/// user (§4.7 rule 4).
#[derive(Debug, Clone, PartialEq)]
pub struct StartedRun {
    /// The tool.
    pub tool: String,
    /// The job ids found in the answer.
    pub job_ids: Vec<String>,
    /// The arguments that were sent.
    pub arguments: Value,
}

/// A cost check whose answer was lost, unreadable or of an unknown shape.
/// The service shares the tool with the paid run, so it may have started
/// one (§4.7, review M1). Checks of this tool stay off until FreeCut
/// restarts; the caller records it like a run whose outcome is unknown and
/// tells the user to look at their Higgsfield account.
#[derive(Debug, Clone, PartialEq)]
pub struct PossiblyStarted {
    /// The tool.
    pub tool: String,
    /// The arguments that were sent.
    pub arguments: Value,
    /// What went wrong.
    pub cause: CloudError,
}

/// What a cost check produced.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub enum QuoteOutcome {
    /// An estimate.
    Quote(Quote),
    /// A question to put to the user first.
    Question(ServiceQuestion),
    /// No estimate is possible.
    NotAvailable(Unavailable),
    /// The check started a run.
    StartedRun(StartedRun),
    /// The check may have started a run (fail closed).
    PossiblyStarted(PossiblyStarted),
}

/// A generation service reached through the user's own account.
pub trait GenerationProvider: Send + Sync + fmt::Debug {
    /// The provider's id.
    fn id(&self) -> ProviderId;

    /// Connects: handshake, tool list, allow-list check (CG-001).
    ///
    /// # Errors
    /// [`CloudError`] as classified (§10).
    fn connect(&self, cancel: &CancelToken) -> Result<ServiceInfo, CloudError>;

    /// Loads the model catalogue (CG-003). Free.
    ///
    /// # Errors
    /// [`CloudError`] as classified (§10).
    fn catalogue(&self, cancel: &CancelToken) -> Result<Catalogue, CloudError>;

    /// Asks the service what `op` would cost, without running it (CG-005).
    ///
    /// # Errors
    /// [`CloudError`] as classified (§10).
    fn quote(&self, op: &Operation, cancel: &CancelToken) -> Result<QuoteOutcome, CloudError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspects() {
        assert!(Aspect::parse("16:9").is_some());
        assert!(Aspect::parse("auto").is_some());
        assert!(Aspect::parse("0:9").is_none());
        assert!(Aspect::parse("16x9").is_none());
        assert!(Aspect::parse("-1:2").is_none());
    }

    #[test]
    fn remote_ids_are_uuids_never_urls() {
        assert!(RemoteMediaId::parse("123E4567-E89B-12D3-A456-426614174000").is_some());
        assert!(RemoteMediaId::parse("https://example.test/a.png").is_none());
    }
}
