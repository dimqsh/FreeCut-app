//! The recorded catalogue parses as expected, and every cost check's
//! arguments match their golden JSON and the recorded tool schemas.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::{fake_higgsfield, fixture_json, golden, image_op, provider, video_op};
use freecut_cloud::CancelToken;
use freecut_cloud::catalogue::{
    AudioSupport, Catalogue, DurationSpec, Durations, HiddenReason, ModelId, OutputKind, Role,
};
use freecut_cloud::guard::{QuoteRequest, schema_offers_get_cost};
use freecut_cloud::mcp::StaticToken;
use freecut_cloud::mcp::schema::validate;
use freecut_cloud::provider::GenerationProvider;
use freecut_cloud::provider::higgsfield::map::tool_call;
use freecut_cloud::provider::higgsfield::reviewed::{
    REVIEWED_MODELS, ReviewedKind, restrict_to_reviewed,
};
use freecut_cloud::provider::higgsfield::{DEFAULT_IMAGE_MODEL, DEFAULT_VIDEO_MODEL};
use freecut_cloud::provider::{
    Aspect, ImageRequest, Operation, ParamValue, ParamValues, RemoteMediaId, RoleRef, VideoMode,
    VideoRequest, WholeSeconds,
};
use freecut_cloud::secret::Secret;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

fn catalogue() -> Catalogue {
    Catalogue::from_listings(&[
        fixture_json("models_explore-list-image.json"),
        fixture_json("models_explore-list-video.json"),
        fixture_json("models_explore-list-audio.json"),
    ])
    .unwrap()
}

/// The recorded catalogue as the Higgsfield provider offers it.
fn reviewed_catalogue() -> Catalogue {
    let mut c = catalogue();
    restrict_to_reviewed(&mut c);
    c
}

/// A listing of copies of a recorded `kind` entry under other ids.
fn listing_with_ids(kind: &str, ids: &[&str]) -> Value {
    let recorded = fixture_json(&format!("models_explore-list-{kind}.json"));
    let template = recorded["items"][0].clone();
    let items: Vec<Value> = ids
        .iter()
        .map(|id| {
            let mut item = template.clone();
            item["id"] = Value::String((*id).to_owned());
            item["name"] = Value::String(format!("Model {id}"));
            item["description"] = Value::String("Text to media.".into());
            item
        })
        .collect();
    serde_json::json!({"items": items, "has_more": false})
}

fn reviewed_ids() -> BTreeSet<&'static str> {
    REVIEWED_MODELS.iter().map(|m| m.id).collect()
}

fn visible_ids(c: &Catalogue) -> BTreeSet<String> {
    c.visible().map(|m| m.id.as_str().to_owned()).collect()
}

fn schema(tool: &str) -> Value {
    fixture_json("tools-list-schemas.json")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == tool)
        .unwrap()["inputSchema"]
        .clone()
}

#[test]
fn the_recorded_listing_parses_without_skipping() {
    let c = catalogue();
    assert_eq!(c.skipped(), 0);
    assert_eq!(c.models().len(), 11 + 17 + 8);
    let counts = c.counts();
    // Hidden: soul_2, ms_image, image_auto (image); marketing_studio_video,
    // sync_so, dubbing, voice_change, higgsfield_preset, clipify (video); the
    // three game-pipeline models (audio).
    assert_eq!((counts.image, counts.video, counts.audio), (8, 11, 5));
    assert_eq!(
        c.get("image_auto").unwrap().hidden,
        Some(HiddenReason::NotReviewed)
    );
}

#[test]
fn the_defaults_are_listed_and_offered() {
    let c = catalogue();
    let image = c.get(DEFAULT_IMAGE_MODEL).unwrap();
    assert!(image.is_visible());
    assert_eq!(image.output, OutputKind::Image);
    assert_eq!(image.company.as_deref(), Some("OpenAI"));
    assert!(image.accepts(&Role::ImageReferences));
    assert_eq!(image.resolutions, vec!["1k", "2k", "4k"]);

    let video = c.get(DEFAULT_VIDEO_MODEL).unwrap();
    assert!(video.is_visible());
    assert_eq!(
        video.durations,
        Durations {
            spec: DurationSpec::Range {
                min: 4,
                max: Some(30)
            },
            default: Some(5)
        }
    );
    assert_eq!(video.resolutions, vec!["480p", "720p", "1080p"]);
    assert_eq!(
        video.audio,
        Some(AudioSupport {
            param: "generate_audio".into(),
            default: Some(true)
        })
    );
    for role in [
        Role::StartImage,
        Role::EndImage,
        Role::ImageReferences,
        Role::VideoReferences,
        Role::AudioReferences,
    ] {
        assert!(video.accepts(&role), "{role:?}");
    }
}

#[test]
fn recorded_duration_shapes() {
    let c = catalogue();
    let spec = |id: &str| c.get(id).unwrap().durations.spec.clone();
    assert_eq!(
        spec("cinematic_studio_video"),
        DurationSpec::Set(vec![5, 10])
    );
    assert_eq!(
        spec("cinematic_studio_3_0"),
        DurationSpec::Range {
            min: 4,
            max: Some(15)
        }
    );
    assert_eq!(spec("minimax_hailuo"), DurationSpec::Set(vec![6, 10]));
    assert_eq!(spec("veo3_1"), DurationSpec::Set(vec![4, 6, 8]));
    assert_eq!(
        spec("wan3_0"),
        DurationSpec::Range {
            min: 1,
            max: Some(30)
        }
    );
    assert_eq!(
        spec("cinematic_studio_video_3_5"),
        DurationSpec::Range { min: 4, max: None }
    );
    assert_eq!(spec("video_background_remover"), DurationSpec::Unspecified);
}

#[test]
fn hidden_models_have_their_reasons() {
    let c = catalogue();
    let reason = |id: &str| c.get(id).unwrap().hidden;
    assert_eq!(reason("soul_2"), Some(HiddenReason::NeedsIdentityTraining));
    assert_eq!(reason("ms_image"), Some(HiddenReason::ServiceStyles));
    assert_eq!(
        reason("marketing_studio_video"),
        Some(HiddenReason::ServiceStyles)
    );
    assert_eq!(
        reason("sonilo_music"),
        Some(HiddenReason::ServicePipelineOnly)
    );
    assert_eq!(
        reason("higgsfield_preset"),
        Some(HiddenReason::LegacyPresets)
    );
    assert_eq!(reason("clipify"), Some(HiddenReason::WebAddressInput));
    assert_eq!(reason("dubbing"), Some(HiddenReason::VoiceToolNotBuilt));
    assert_eq!(reason("sync_so"), Some(HiddenReason::VoiceToolNotBuilt));
    // A media group without roles (marketing_studio_video's avatars) parses.
    assert!(
        c.get("marketing_studio_video")
            .unwrap()
            .media
            .iter()
            .any(|m| m.roles.is_empty())
    );
}

#[test]
fn input_filters_and_duplicate_names() {
    let c = catalogue();
    let with_end = c.filter(&OutputKind::Video, &[Role::EndImage]);
    assert!(
        with_end
            .models
            .iter()
            .any(|m| m.id == ModelId::new("seedance_2_5"))
    );
    assert!(with_end.hidden_by_inputs > 0);
    assert!(with_end.models.iter().all(|m| m.accepts(&Role::EndImage)));
    let masks = c.filter(&OutputKind::Image, &[Role::Mask]);
    assert_eq!(
        masks
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["nano_banana_2"]
    );
    // Two offered models are named "Nano Banana Pro".
    assert_eq!(
        c.label(c.get("nano_banana_pro").unwrap()),
        "Nano Banana Pro (nano_banana_pro)"
    );
}

fn checked_quote_args(op: &Operation) -> Value {
    let c = catalogue();
    let model = c.get(op.model().as_str()).unwrap();
    let call = tool_call(op, model).unwrap();
    let quote = QuoteRequest::new(call.tool.name(), call.arguments).unwrap();
    let tool_schema = schema(call.tool.name());
    assert!(schema_offers_get_cost(&tool_schema));
    validate(&tool_schema, quote.arguments()).unwrap();
    quote.arguments().clone()
}

fn id(n: &str) -> RemoteMediaId {
    RemoteMediaId::parse(n).unwrap()
}

#[test]
fn golden_cost_check_arguments() {
    assert_eq!(
        checked_quote_args(&image_op()),
        golden("get_cost-image-gpt_image_2_5.json")
    );
    assert_eq!(
        checked_quote_args(&video_op()),
        golden("get_cost-video-seedance_2_5-t2v.json")
    );

    let frames = Operation::Video(VideoRequest {
        model: ModelId::new("seedance_2_5"),
        mode: VideoMode::FramesToVideo,
        prompt: "the camera rises over the city".into(),
        refs: vec![
            RoleRef {
                role: Role::StartImage,
                media: id("00000000-0000-4000-8000-000000000001"),
            },
            RoleRef {
                role: Role::EndImage,
                media: id("00000000-0000-4000-8000-000000000002"),
            },
        ],
        aspect: Aspect::parse("9:16"),
        duration: Some(WholeSeconds(8)),
        resolution: Some("480p".into()),
        sound: Some(false),
        variants: 1,
        params: ParamValues::new(),
    });
    assert_eq!(
        checked_quote_args(&frames),
        golden("get_cost-video-seedance_2_5-frames.json")
    );

    let mut params = ParamValues::new();
    params.insert("resolution".into(), ParamValue::Text("2k".into()));
    params.insert("is_inpaint".into(), ParamValue::Bool(true));
    let edit = Operation::Image(ImageRequest {
        model: ModelId::new("nano_banana_2"),
        prompt: "make it night".into(),
        refs: vec![
            RoleRef {
                role: Role::ImageReferences,
                media: id("00000000-0000-4000-8000-00000000000a"),
            },
            RoleRef {
                role: Role::Mask,
                media: id("00000000-0000-4000-8000-00000000000b"),
            },
        ],
        aspect: Aspect::parse("4:5"),
        variants: 1,
        params,
    });
    assert_eq!(
        checked_quote_args(&edit),
        golden("get_cost-image-nano_banana_2-edit.json")
    );
}

#[test]
fn image_resolution_goes_through_settings_and_is_reserved_for_video() {
    // `resolution` is an ordinary setting for image models but a field of
    // its own for video, so a video setting of that name is refused.
    let c = catalogue();
    let Operation::Video(mut v) = video_op() else {
        unreachable!()
    };
    v.params
        .insert("resolution".into(), ParamValue::Text("480p".into()));
    let model = c.get("seedance_2_5").unwrap();
    assert!(tool_call(&Operation::Video(v), model).is_err());
}

#[test]
fn the_recorded_get_answer_matches_the_listing_entry() {
    let page = freecut_cloud::catalogue::parse_listing(&serde_json::json!({
        "items": [fixture_json("models_explore-get-gpt_image_2_5.json")]
    }))
    .unwrap();
    assert_eq!(
        page.models[0],
        catalogue().get("gpt_image_2_5").unwrap().clone()
    );
}

// ---- Reviewed models (owner decision, review round 2) ----

#[test]
fn every_visible_model_is_on_the_reviewed_list() {
    let c = reviewed_catalogue();
    let visible = visible_ids(&c);
    let reviewed: BTreeSet<String> = reviewed_ids().into_iter().map(str::to_owned).collect();
    // The list was built from this listing: every entry is in it and shown.
    assert_eq!(visible, reviewed);
    assert!(
        c.visible()
            .all(|m| matches!(m.output, OutputKind::Image | OutputKind::Video))
    );
    assert_eq!(c.counts().audio, 0);

    // The same through the provider, against the fake service.
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, Arc::new(StaticToken(Secret::new("t1"))));
    let offered = p.catalogue(&CancelToken::new()).unwrap();
    assert_eq!(visible_ids(&offered), visible);
}

#[test]
fn pipeline_models_and_image_auto_are_hidden() {
    let c = reviewed_catalogue();
    assert_eq!(
        c.get("image_auto").unwrap().hidden,
        Some(HiddenReason::NotReviewed)
    );
    // Specific reasons survive the review.
    assert_eq!(
        c.get("soul_2").unwrap().hidden,
        Some(HiddenReason::NeedsIdentityTraining)
    );
    let mut pipelines = Catalogue::from_listings(&[listing_with_ids(
        "video",
        &[
            "ad_multiplier_v2",
            "hf_mult_motion_control",
            "3d_jutsu_scene",
            "cinematic_studio_soul_cinema",
        ],
    )])
    .unwrap();
    restrict_to_reviewed(&mut pipelines);
    assert_eq!(pipelines.visible().count(), 0);
    assert!(
        pipelines
            .models()
            .iter()
            .all(|m| m.hidden == Some(HiddenReason::NotReviewed))
    );
}

#[test]
fn a_model_nobody_reviewed_is_hidden_as_not_reviewed() {
    for (kind, id) in [
        ("image", "brand_new_image_3"),
        ("video", "brand_new_video_3"),
    ] {
        let mut c = Catalogue::from_listings(&[listing_with_ids(kind, &[id])]).unwrap();
        // Without the review it would be shown.
        assert!(c.get(id).unwrap().is_visible(), "{id}");
        restrict_to_reviewed(&mut c);
        assert_eq!(c.get(id).unwrap().hidden, Some(HiddenReason::NotReviewed));
    }
    // A reviewed id that now makes something else is hidden too.
    let mut moved =
        Catalogue::from_listings(&[listing_with_ids("video", &["gpt_image_2_5"])]).unwrap();
    restrict_to_reviewed(&mut moved);
    assert_eq!(
        moved.get("gpt_image_2_5").unwrap().hidden,
        Some(HiddenReason::NotReviewed)
    );
}

#[test]
fn every_reviewed_model_is_a_plain_image_or_video_generator() {
    let recorded = catalogue();
    for entry in REVIEWED_MODELS {
        let model = recorded
            .get(entry.id)
            .unwrap_or_else(|| panic!("{} is not in the recorded listing", entry.id));
        let expected = match entry.kind {
            ReviewedKind::Image => OutputKind::Image,
            ReviewedKind::Video => OutputKind::Video,
        };
        assert_eq!(model.output, expected, "{}", entry.id);
        // No pipeline, styles, identity or voice reason applies to it.
        assert_eq!(model.hidden, None, "{}", entry.id);
        assert!(model.company.is_some(), "{} has no maker", entry.id);
        assert!(
            !model.description.is_empty(),
            "{} has no description",
            entry.id
        );
    }
    let ids = reviewed_ids();
    assert_eq!(ids.len(), REVIEWED_MODELS.len(), "no duplicates");
    assert!(ids.contains(DEFAULT_IMAGE_MODEL) && ids.contains(DEFAULT_VIDEO_MODEL));
}
