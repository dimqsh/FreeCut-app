//! The tool allow list (architecture §6.3).
//!
//! The service offers 100+ tools, many of which spend money, publish or
//! change the account. FreeCut calls only the tools below; everything else
//! is denied by construction. The `class` column is the transport's input
//! (§4.5): a tool is free only if this table says so. During development,
//! upload tools also count as paid (handoff brief §7): none is called by
//! tests or tooling.

use std::time::Duration;

use crate::mcp::CallClass;
use crate::transport::CallLimits;

/// One allowed tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowedTool {
    /// The tool name.
    pub name: &'static str,
    /// Retry and resend rules.
    pub class: CallClass,
    /// Whether it offers a `get_cost` preflight.
    pub cost_preflight: bool,
    /// Timeouts.
    pub limits: CallLimits,
    /// The feature that uses it.
    pub used_for: &'static str,
}

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

/// Every tool FreeCut may call. Voice tools (`dubbing`, `voice_change`,
/// `list_voices`) are absent: the owner decided not to build CG-021..CG-023.
pub const ALLOW_LIST: &[AllowedTool] = &[
    AllowedTool {
        name: "models_explore",
        class: CallClass::Free,
        cost_preflight: false,
        limits: CallLimits::READ,
        used_for: "catalogue (CG-003)",
    },
    AllowedTool {
        name: "media_upload",
        class: CallClass::Free,
        cost_preflight: false,
        limits: UPLOAD_LIMITS,
        used_for: "upload slots (H2)",
    },
    AllowedTool {
        name: "media_confirm",
        // Sent once, then the run fails (§4.5).
        class: CallClass::Paid,
        cost_preflight: false,
        limits: UPLOAD_LIMITS,
        used_for: "upload confirmation (H2)",
    },
    AllowedTool {
        name: "generate_image",
        class: CallClass::Paid,
        cost_preflight: true,
        limits: CallLimits::PAID,
        used_for: "CG-006, CG-015 (images)",
    },
    AllowedTool {
        name: "generate_video",
        class: CallClass::Paid,
        cost_preflight: true,
        limits: CallLimits::PAID,
        used_for: "CG-007, CG-015, CG-016",
    },
    AllowedTool {
        name: "jobs_wait",
        class: CallClass::Free,
        cost_preflight: false,
        limits: POLL_LIMITS,
        used_for: "polling (H2)",
    },
    AllowedTool {
        name: "upscale_image",
        class: CallClass::Paid,
        cost_preflight: true,
        limits: CallLimits::PAID,
        used_for: "CG-018 images (later)",
    },
    AllowedTool {
        name: "reframe",
        class: CallClass::Paid,
        cost_preflight: true,
        limits: CallLimits::PAID,
        used_for: "CG-019 (later)",
    },
    AllowedTool {
        name: "balance",
        class: CallClass::Free,
        cost_preflight: false,
        limits: CallLimits::READ,
        used_for: "Show balance (CG-001), on click only",
    },
];

/// Tools whose absence disables H1's actions (connect checks them).
pub const NEEDED_NOW: &[&str] = &["models_explore", "generate_image", "generate_video"];

/// The allow-list entry for `name`, if FreeCut may call it.
pub fn lookup(name: &str) -> Option<&'static AllowedTool> {
    ALLOW_LIST.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            if t.name.starts_with("generate_") {
                assert_eq!(t.class, CallClass::Paid, "{}", t.name);
                assert!(t.cost_preflight, "{}", t.name);
            }
        }
        let free: Vec<_> = ALLOW_LIST
            .iter()
            .filter(|t| t.class == CallClass::Free)
            .map(|t| t.name)
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
