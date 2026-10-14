//! Enforces the crate dependency rules (docs/architecture/README.md §1.2):
//! which workspace crates may depend on which, what the pure timeline crate
//! may use, and that only `freecut-media` links FFmpeg.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;

const ALLOWED_EDGES: &[(&str, &[&str])] = &[
    ("freecut-timeline", &[]),
    ("freecut-media", &["freecut-timeline"]),
    ("freecut-project", &["freecut-timeline"]),
    ("freecut-render", &["freecut-media", "freecut-timeline"]),
    ("freecut-audio", &["freecut-media", "freecut-timeline"]),
    (
        "freecut-app",
        &[
            "freecut-audio",
            "freecut-media",
            "freecut-project",
            "freecut-render",
            "freecut-timeline",
        ],
    ),
];

/// Pure, I/O-free crates the timeline may use.
const TIMELINE_EXTERNAL_ALLOWLIST: &[&str] = &[
    "thiserror",
    "serde",
    "smallvec",
    "glam",
    "xxhash-rust",
    "num-rational",
    "num-integer",
    "ethnum",
    "libm",
    "unicode-segmentation",
    "unicode-normalization",
    "icu_segmenter",
    "icu_casemap",
];

const FFMPEG_CRATES: &[&str] = &["ffmpeg-next", "ffmpeg-sys-next"];

struct Dep {
    name: String,
    optional: bool,
}

/// Normal (non-dev, non-build) dependencies of every workspace member.
fn workspace_deps() -> BTreeMap<String, Vec<Dep>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut map = BTreeMap::new();
    for pkg in meta["packages"].as_array().unwrap() {
        let deps = pkg["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["kind"].is_null())
            .map(|d| Dep {
                name: d["name"].as_str().unwrap().to_string(),
                optional: d["optional"].as_bool().unwrap_or(false),
            })
            .collect();
        map.insert(pkg["name"].as_str().unwrap().to_string(), deps);
    }
    map
}

#[test]
fn workspace_edges_follow_the_dependency_direction() {
    let deps = workspace_deps();
    let allowed: BTreeMap<&str, BTreeSet<&str>> = ALLOWED_EDGES
        .iter()
        .map(|(k, v)| (*k, v.iter().copied().collect()))
        .collect();
    for (krate, list) in &deps {
        let ok = allowed
            .get(krate.as_str())
            .unwrap_or_else(|| panic!("{krate} is not in the dependency table"));
        for dep in list.iter().filter(|d| d.name.starts_with("freecut-")) {
            assert!(
                ok.contains(dep.name.as_str()),
                "{krate} must not depend on {}",
                dep.name
            );
        }
    }
    let audio_media = deps["freecut-audio"]
        .iter()
        .find(|d| d.name == "freecut-media")
        .expect("audio uses media behind a feature");
    assert!(
        audio_media.optional,
        "freecut-audio must reach freecut-media only through the optional `media` feature"
    );
}

#[test]
fn the_timeline_crate_uses_only_allowlisted_pure_crates() {
    let deps = workspace_deps();
    for dep in &deps["freecut-timeline"] {
        assert!(
            TIMELINE_EXTERNAL_ALLOWLIST.contains(&dep.name.as_str()),
            "freecut-timeline must stay pure; {} is not on the allowlist",
            dep.name
        );
    }
}

#[test]
fn only_the_media_crate_links_ffmpeg() {
    for (krate, list) in workspace_deps() {
        if krate == "freecut-media" {
            continue;
        }
        for dep in list {
            assert!(
                !FFMPEG_CRATES.contains(&dep.name.as_str()),
                "{krate} must not depend on {} directly",
                dep.name
            );
        }
    }
}

#[test]
fn render_stays_independent_of_ui_and_project() {
    let deps = workspace_deps();
    for dep in &deps["freecut-render"] {
        assert!(
            !["egui", "eframe", "freecut-project", "freecut-audio"].contains(&dep.name.as_str()),
            "freecut-render must not depend on {}",
            dep.name
        );
    }
}

/// The LUT library lives in `freecut-project` but never parses a LUT: the
/// app validates files with `freecut-media` first (README C54, CL-016).
#[test]
fn the_project_crate_never_reaches_media() {
    let deps = workspace_deps();
    for dep in &deps["freecut-project"] {
        assert!(
            !["freecut-media", "freecut-render", "freecut-audio"].contains(&dep.name.as_str()),
            "freecut-project must not depend on {}",
            dep.name
        );
    }
}
