//! The tool allow list (architecture §6.3).
//!
//! The service offers 100+ tools, many of which spend money, publish or
//! change the account. FreeCut calls only the tools below; everything else
//! is denied by construction: [`McpClient::call_tool`](crate::mcp::McpClient::call_tool)
//! takes an [`AllowedTool`], which only this list can create, and reads the
//! call's class and timeouts from it (§4.5). A tool is free only if this
//! list says so.
//!
//! A separate development rule (handoff brief §7) also forbids calling the
//! upload tools during development; that is a rule for people and tooling,
//! not a property of their class here.

use std::time::Duration;

use crate::mcp::AllowedTool;
use crate::transport::CallLimits;

const UPLOAD_LIMITS: CallLimits = CallLimits {
    connect: Duration::from_secs(10),
    total: Duration::from_secs(60),
    fresh_connection: false,
};

const POLL_LIMITS: CallLimits = CallLimits {
    connect: Duration::from_secs(10),
    total: Duration::from_secs(45),
    fresh_connection: false,
};

/// The model catalogue (CG-003).
pub static MODELS_EXPLORE: AllowedTool =
    AllowedTool::free("models_explore", CallLimits::READ, "catalogue (CG-003)");
/// Upload slots (H2); it only creates slots, so it is free (§4.5).
pub static MEDIA_UPLOAD: AllowedTool =
    AllowedTool::free("media_upload", UPLOAD_LIMITS, "upload slots (H2)");
/// Upload confirmation (H2): sent once, then the run fails (§4.5).
pub static MEDIA_CONFIRM: AllowedTool = AllowedTool::paid(
    "media_confirm",
    UPLOAD_LIMITS,
    false,
    "upload confirmation (H2)",
);
/// Image generation.
pub static GENERATE_IMAGE: AllowedTool = AllowedTool::paid(
    "generate_image",
    CallLimits::PAID,
    true,
    "CG-006, CG-015 (images)",
);
/// Video generation.
pub static GENERATE_VIDEO: AllowedTool = AllowedTool::paid(
    "generate_video",
    CallLimits::PAID,
    true,
    "CG-007, CG-015, CG-016",
);
/// Polling (H2).
pub static JOBS_WAIT: AllowedTool = AllowedTool::free("jobs_wait", POLL_LIMITS, "polling (H2)");
/// Image upscaling (later).
pub static UPSCALE_IMAGE: AllowedTool = AllowedTool::paid(
    "upscale_image",
    CallLimits::PAID,
    true,
    "CG-018 images (later)",
);
/// Generative reframe (later).
pub static REFRAME: AllowedTool =
    AllowedTool::paid("reframe", CallLimits::PAID, true, "CG-019 (later)");
/// *Show balance* (CG-001), on click only.
pub static BALANCE: AllowedTool = AllowedTool::free(
    "balance",
    CallLimits::READ,
    "Show balance (CG-001), on click only",
);

/// Every tool FreeCut may call. Voice tools (`dubbing`, `voice_change`,
/// `list_voices`) are absent: the owner decided not to build CG-021..CG-023.
pub static ALLOW_LIST: [&AllowedTool; 9] = [
    &MODELS_EXPLORE,
    &MEDIA_UPLOAD,
    &MEDIA_CONFIRM,
    &GENERATE_IMAGE,
    &GENERATE_VIDEO,
    &JOBS_WAIT,
    &UPSCALE_IMAGE,
    &REFRAME,
    &BALANCE,
];

/// Tools whose absence disables H1's actions (connect checks them).
pub const NEEDED_NOW: &[&str] = &["models_explore", "generate_image", "generate_video"];

/// The allow-list entry for a tool name the service listed, if FreeCut may
/// call it.
pub fn lookup(name: &str) -> Option<&'static AllowedTool> {
    ALLOW_LIST.iter().copied().find(|t| t.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::CallClass;

    #[test]
    fn spending_and_publishing_tools_are_not_listed() {
        for denied in [
            "execute_preset",
            "generate_image_batch",
            "generate_video_batch",
            "media_import_url",
            "dubbing",
            "voice_change",
            "list_voices",
            "create_voice",
            "publish_website",
            "tiktok_prepare_publish",
            "update_preferences",
            "upscale_video",
            "remove_background",
        ] {
            assert!(lookup(denied).is_none(), "{denied}");
        }
    }

    #[test]
    fn only_listed_free_tools_are_free_and_every_generator_has_a_preflight() {
        for t in ALLOW_LIST {
            assert_ne!(t.class(), CallClass::Preflight, "{}", t.name());
            if t.name().starts_with("generate_") {
                assert_eq!(t.class(), CallClass::Paid, "{}", t.name());
                assert!(t.has_cost_preflight(), "{}", t.name());
            }
            if t.class() == CallClass::Paid {
                assert!(
                    t.limits().fresh_connection || t.name() == "media_confirm",
                    "{}",
                    t.name()
                );
            }
        }
        let free: Vec<_> = ALLOW_LIST
            .iter()
            .filter(|t| t.class() == CallClass::Free)
            .map(|t| t.name())
            .collect();
        assert_eq!(
            free,
            vec!["models_explore", "media_upload", "jobs_wait", "balance"]
        );
    }

    #[test]
    fn needed_tools_are_allowed() {
        assert!(NEEDED_NOW.iter().all(|n| lookup(n).is_some()));
    }
}
