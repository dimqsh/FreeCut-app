//! The models FreeCut shows (architecture §6.2; owner decision, review round
//! 2): v1 offers only plain image and video generation models that someone
//! reviewed. Every other model the service lists, including any it adds
//! later, is hidden: with its specific reason where the catalogue knows one
//! (identity training, service styles, pipelines, voice tools), otherwise as
//! [`HiddenReason::NotReviewed`].

use crate::catalogue::{Catalogue, HiddenReason, ModelInfo, OutputKind};

/// What a reviewed model makes. Only plain generation is reviewed for v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewedKind {
    /// Prompt (and optional reference images) to image.
    Image,
    /// Prompt, start/end frames or references to video.
    Video,
}

impl ReviewedKind {
    /// Whether a model with `output` makes this kind.
    pub fn matches(self, output: &OutputKind) -> bool {
        matches!(
            (self, output),
            (ReviewedKind::Image, OutputKind::Image) | (ReviewedKind::Video, OutputKind::Video)
        )
    }
}

/// One reviewed model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewedModel {
    /// The service's model id.
    pub id: &'static str,
    /// What it makes.
    pub kind: ReviewedKind,
}

const fn image(id: &'static str) -> ReviewedModel {
    ReviewedModel {
        id,
        kind: ReviewedKind::Image,
    }
}

const fn video(id: &'static str) -> ReviewedModel {
    ReviewedModel {
        id,
        kind: ReviewedKind::Video,
    }
}

/// The reviewed models.
///
/// Review: proposed 2026-10-14 by the branch agent from the recorded listing
/// (`tests/fixtures/higgsfield/models_explore-list-{image,video}.json`, which
/// are excerpts: 11 of 45 image and 17 of 57 video models). Taken: models
/// with a named maker and a description that generate from a prompt, frames
/// or references. Left out until someone reviews them: models without a
/// description (`nano_banana_2_shots`, `cinematic_studio_video_3_5`), tools
/// that are not generation (background removal, upscaling, reframe), and the
/// service's own pipelines. **Owner confirmation pending**; update this note
/// with who confirmed it and when.
pub const REVIEWED_MODELS: &[ReviewedModel] = &[
    image("gpt_image_2"),
    image("gpt_image_2_5"),
    image("nano_banana_2"),
    image("nano_banana_pro"),
    image("seedream_v5_pro"),
    video("seedance_2_5"),
    video("kling3_0"),
    video("veo3_1"),
    video("minimax_hailuo"),
    video("wan3_0"),
    video("cinematic_studio_3_0"),
    video("cinematic_studio_video"),
];

/// The reviewed entry for `id`, if any.
pub fn reviewed(id: &str) -> Option<&'static ReviewedModel> {
    REVIEWED_MODELS.iter().find(|m| m.id == id)
}

/// Why `model` is hidden by the review, if it is: it is not on the list, or
/// it no longer makes what was reviewed.
pub fn unreviewed(model: &ModelInfo) -> Option<HiddenReason> {
    match reviewed(model.id.as_str()) {
        Some(entry) if entry.kind.matches(&model.output) => None,
        _ => Some(HiddenReason::NotReviewed),
    }
}

/// Hides every model of `catalogue` that is not reviewed; models already
/// hidden keep their own reason.
pub fn restrict_to_reviewed(catalogue: &mut Catalogue) {
    catalogue.hide(unreviewed);
}
