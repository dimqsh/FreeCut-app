//! Enforces the crate dependency rules (docs/architecture/README.md §1.2,
//! cloud-generation.md §2.3): which workspace crates may depend on which,
//! what the pure timeline crate may use, that only `freecut-media` links
//! FFmpeg, and that `freecut-cloud` stays free of FFmpeg, GPU and UI crates
//! and has no networking code without its `http` feature.
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
    // Nothing but the app may depend on cloud.
    ("freecut-cloud", &["freecut-media", "freecut-timeline"]),
    (
        "freecut-app",
        &[
            "freecut-audio",
            "freecut-cloud",
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
    default_features: bool,
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
                default_features: d["uses_default_features"].as_bool().unwrap_or(true),
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

/// Package names in `cargo tree -p <krate>` (normal dependencies only).
fn tree(krate: &str, features: &[&str]) -> BTreeSet<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut args = vec![
        "tree",
        "-p",
        krate,
        "-e",
        "normal",
        "--prefix",
        "none",
        "--format",
        "{p}",
        "--offline",
    ];
    if !features.is_empty() {
        args.push("--features");
        args.extend(features);
    }
    let out = Command::new(cargo)
        .args(&args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn cloud_reaches_media_without_ffmpeg() {
    let deps = workspace_deps();
    let media = deps["freecut-cloud"]
        .iter()
        .find(|d| d.name == "freecut-media")
        .expect("cloud uses media for the cancel token");
    assert!(
        !media.default_features,
        "freecut-cloud must use freecut-media with default-features = false"
    );
}

#[test]
fn cloud_with_networking_has_no_ffmpeg_gpu_or_ui() {
    let names = tree("freecut-cloud", &["http"]);
    assert!(
        names.contains("ureq"),
        "the http feature brings the transport"
    );
    // webpki-roots and webpki-root-certs bundle Mozilla's roots
    // (CDLA-Permissive-2.0); FreeCut uses the platform verifier instead.
    for banned in [
        "ffmpeg-sys-next",
        "ffmpeg-next",
        "wgpu",
        "egui",
        "eframe",
        "webpki-roots",
        "webpki-root-certs",
    ] {
        assert!(
            !names.contains(banned),
            "freecut-cloud --features http pulls in {banned}"
        );
    }
}

#[test]
fn cloud_without_http_has_no_networking_crates() {
    let names = tree("freecut-cloud", &[]);
    for banned in [
        "ureq",
        "ureq-proto",
        "rustls",
        "ring",
        "rustls-platform-verifier",
        "socket2",
        "ffmpeg-sys-next",
        "wgpu",
        "egui",
    ] {
        assert!(
            !names.contains(banned),
            "freecut-cloud without http pulls in {banned}"
        );
    }
}

/// freecut-cloud opens sockets only in its http transport (review Q9): no
/// `std::net`, `TcpStream`, `UdpSocket` or `TcpListener` anywhere else in
/// its sources, so the sans-I/O core cannot grow a network path.
#[test]
fn cloud_opens_sockets_only_in_its_http_transport() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../freecut-cloud/src");
    let transport = src.join("http.rs");
    let mut dirs = vec![src];
    let mut offenders = Vec::new();
    let mut scanned = 0;
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path == transport {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for banned in ["std::net", "TcpStream", "UdpSocket", "TcpListener"] {
                if text.contains(banned) {
                    offenders.push(format!("{}: {banned}", path.display()));
                }
            }
        }
    }
    assert!(scanned > 10, "only {scanned} files scanned");
    assert!(
        offenders.is_empty(),
        "socket code outside http.rs: {offenders:?}"
    );
}
