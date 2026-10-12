//! Saving into another folder (Save As, Save a Copy) rewrites relative media
//! paths so they keep naming the same files (PM-100, PM-012).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::fs::RealFs;
use freecut_project::paths::{resolve, stored_path};
use freecut_project::{Extras, SaveRequest, decode, encode, open, save};
use serde_json::{Value, json};

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

/// F12: Save As into another folder recomputes relative paths there.
#[cfg(unix)]
#[test]
fn f12_save_as_recomputes_relative_paths_for_the_new_folder() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("A");
    let b = root.path().join("B");
    std::fs::create_dir_all(a.join("Media")).unwrap();
    std::fs::create_dir_all(b.join("Media")).unwrap();
    let ours = a.join("Media").join("a.mov");
    std::fs::write(&ours, b"ours").unwrap();
    std::fs::write(b.join("Media").join("a.mov"), b"someone else's").unwrap();
    let sp = stored_path(&ours, &a).unwrap();
    let mut v = sink_value();
    v["document"]["library"][0]["path"] = json!({"absolute": sp.absolute, "relative": sp.relative});
    let opened = decode(&compact(&v)).unwrap();
    let copy = b.join("Copy.freecut");
    save(
        &copy,
        &SaveRequest {
            document: &opened.document,
            ids_next: opened.ids.next(),
            extras: &opened.extras,
            backup: false,
            relative_base: Some(&a),
            export: None,
            ui: None,
        },
    )
    .unwrap();
    let reopened = open(&copy).unwrap();
    let item = reopened.document.library().items().next().unwrap().clone();
    let r = resolve(&item.path, &b, &RealFs);
    assert_eq!(r.path, ours);
}

/// The kitchen sink with its first media item at `sp`, as if opened.
fn opened_with(sp: &freecut_timeline::model::StoredPath) -> freecut_project::Opened {
    let mut v = sink_value();
    v["document"]["library"][0]["path"] = json!({"absolute": sp.absolute, "relative": sp.relative});
    decode(&compact(&v)).unwrap()
}

fn save_to(
    path: &std::path::Path,
    opened: &freecut_project::Opened,
    base: Option<&std::path::Path>,
) {
    save(
        path,
        &SaveRequest {
            document: &opened.document,
            ids_next: opened.ids.next(),
            extras: &opened.extras,
            backup: false,
            relative_base: base,
            export: None,
            ui: None,
        },
    )
    .unwrap();
}

fn first_relative(path: &std::path::Path) -> Option<String> {
    let reopened = open(path).unwrap();
    let item = reopened.document.library().items().next().unwrap().clone();
    item.path.relative.clone()
}

/// A plain Save into the folder the paths are relative to keeps them as they
/// are, even when the absolute path is stale because the project folder
/// moved with its media: recomputing from the absolute path would lose the
/// one locator that still works.
#[cfg(unix)]
#[test]
fn saving_in_place_keeps_relative_paths_that_outlived_a_move() {
    use freecut_project::paths::ResolvedHow;

    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old").join("Trip");
    std::fs::create_dir_all(old.join("Media")).unwrap();
    let media = old.join("Media").join("a.mov");
    std::fs::write(&media, b"ours").unwrap();
    let sp = stored_path(&media, &old).unwrap();
    let new = root.path().join("new");
    std::fs::rename(root.path().join("old"), &new).unwrap();
    let project_dir = new.join("Trip");

    let opened = opened_with(&sp);
    let path = project_dir.join("Trip.freecut");
    save_to(&path, &opened, Some(&project_dir));
    assert_eq!(first_relative(&path).as_deref(), Some("Media/a.mov"));
    let item = open(&path)
        .unwrap()
        .document
        .library()
        .items()
        .next()
        .unwrap()
        .clone();
    let r = resolve(&item.path, &project_dir, &RealFs);
    assert_eq!(r.how, ResolvedHow::Relative);
    assert_eq!(r.path, project_dir.join("Media").join("a.mov"));
}

/// Save As, then more saves to the new file in the same session: the
/// document still holds paths relative to the original folder, so every
/// save rebases from there and writes the same paths.
#[cfg(unix)]
#[test]
fn every_save_after_save_as_writes_paths_for_the_new_folder() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("A");
    let b = root.path().join("B").join("deeper");
    std::fs::create_dir_all(a.join("Media")).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let ours = a.join("Media").join("a.mov");
    std::fs::write(&ours, b"ours").unwrap();
    let opened = opened_with(&stored_path(&ours, &a).unwrap());

    let copy = b.join("Copy.freecut");
    save_to(&copy, &opened, Some(&a));
    let first = std::fs::read(&copy).unwrap();
    assert_eq!(
        first_relative(&copy).as_deref(),
        Some("../../A/Media/a.mov")
    );
    save_to(&copy, &opened, Some(&a));
    assert!(std::fs::read(&copy).unwrap() == first, "a fixpoint");

    // Saving back into the original folder gives the original paths.
    let back = a.join("Back.freecut");
    save_to(&back, &opened, Some(&a));
    assert_eq!(first_relative(&back).as_deref(), Some("Media/a.mov"));

    // Without a base the paths are written as the document holds them.
    let verbatim = b.join("Verbatim.freecut");
    save_to(&verbatim, &opened, None);
    assert_eq!(first_relative(&verbatim).as_deref(), Some("Media/a.mov"));

    // The other items: a Windows-style absolute path can't get a relative
    // path on this OS, and a `..` path is rebased like any other.
    let reopened = open(&copy).unwrap();
    let mut items = reopened.document.library().items();
    let song = items.nth(1).unwrap();
    assert_eq!(song.path.relative, None);
    let photo = items.next().unwrap();
    assert_eq!(
        photo.path.relative.as_deref(),
        Some("../../Pictures/sunset.png")
    );
}

/// Two spellings of one folder (through a symlink) are the same folder: a
/// plain save writes the paths unchanged.
#[cfg(unix)]
#[test]
fn a_symlinked_spelling_of_the_base_folder_is_still_the_same_folder() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("A");
    std::fs::create_dir_all(a.join("Media")).unwrap();
    let link = root.path().join("Link");
    std::os::unix::fs::symlink(&a, &link).unwrap();
    let ours = a.join("Media").join("a.mov");
    std::fs::write(&ours, b"ours").unwrap();
    let opened = opened_with(&stored_path(&ours, &a).unwrap());
    let path = link.join("Trip.freecut");
    save_to(&path, &opened, Some(&a));
    assert_eq!(first_relative(&path).as_deref(), Some("Media/a.mov"));
}
