//! The recorded catalogue parses as expected, and every cost check's
//! arguments match their golden JSON and the recorded tool schemas.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::{fixture_json, golden, image_op, video_op};
use freecut_cloud::catalogue::{
    AudioSupport, Catalogue, DurationSpec, Durations, HiddenReason, ModelId, OutputKind, Role,
};
use freecut_cloud::guard::{QuoteRequest, schema_offers_get_cost};
use freecut_cloud::mcp::schema::validate;
use freecut_cloud::provider::higgsfield::map::tool_call;
use freecut_cloud::provider::higgsfield::{DEFAULT_IMAGE_MODEL, DEFAULT_VIDEO_MODEL};
use freecut_cloud::provider::{
    Aspect, ImageRequest, Operation, ParamValue, ParamValues, RemoteMediaId, RoleRef, VideoMode,
    VideoRequest, WholeSeconds,
};
use pretty_assertions::assert_eq;
use serde_json::Value;

fn catalogue() -> Catalogue {
    Catalogue::from_listings(&[
        fixture_json("models_explore-list-image.json"),
        fixture_json("models_explore-list-video.json"),
        fixture_json("models_explore-list-audio.json"),
    ])
    .unwrap()
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
    let quote = QuoteRequest::new(call.tool, call.arguments).unwrap();
    let tool_schema = schema(call.tool);
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
