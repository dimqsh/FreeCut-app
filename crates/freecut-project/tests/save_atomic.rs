//! Saving is atomic: injected failures at every step leave either the old
//! file or the complete new one, never a mix, and never a stray temp file.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use common::*;
use freecut_project::fs::{FileSystem, RealFs};
use freecut_project::{Extras, SaveError, SaveRequest, SaveStep, encode, open, save, save_with};
use freecut_timeline::model::Document;
use pretty_assertions::assert_eq;

/// Where to inject a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    /// Writing the temp file: half the bytes land, then "disk full".
    TempWrite,
    /// Syncing the temp file fails after all bytes were written.
    TempSync,
    Rename,
    SyncDir,
    Backup,
    /// Reading back returns different bytes (a broken network share).
    ReadBackCorrupt,
}

/// The real file system with one injected failure.
struct Faulty {
    fail: Fail,
    log: Mutex<Vec<String>>,
}

impl Faulty {
    fn new(fail: Fail) -> Faulty {
        Faulty {
            fail,
            log: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, what: String) {
        self.log.lock().unwrap().push(what);
    }
}

fn is_temp(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "tmp")
}

/// The backup or its temp file (`.Trip.freecut.bak.<pid>-<n>.tmp`).
fn is_backup(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().contains(".freecut.bak"))
}

fn full() -> io::Error {
    io::Error::new(io::ErrorKind::StorageFull, "injected: no space left")
}

impl FileSystem for Faulty {
    fn is_file(&self, path: &Path) -> bool {
        RealFs.is_file(path)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = RealFs.read(path)?;
        if self.fail == Fail::ReadBackCorrupt && !bytes.is_empty() {
            let last = bytes.len() - 2;
            bytes[last] ^= 0x20;
        }
        Ok(bytes)
    }

    fn write_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.note(format!("write {}", path.display()));
        match self.fail {
            Fail::TempWrite if is_temp(path) => {
                RealFs.write_new(path, &bytes[..bytes.len() / 2])?;
                Err(full())
            }
            Fail::TempSync if is_temp(path) => {
                RealFs.write_new(path, bytes)?;
                Err(io::Error::other("injected: fsync failed"))
            }
            Fail::Backup if is_backup(path) => {
                RealFs.write_new(path, &bytes[..1])?;
                Err(full())
            }
            _ => RealFs.write_new(path, bytes),
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.note(format!("rename {} -> {}", from.display(), to.display()));
        if self.fail == Fail::Rename {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected: rename refused",
            ));
        }
        RealFs.rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.note(format!("remove {}", path.display()));
        RealFs.remove_file(path)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        if self.fail == Fail::SyncDir {
            return Err(io::Error::other("injected: directory fsync failed"));
        }
        RealFs.sync_dir(dir)
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    dir: PathBuf,
    path: PathBuf,
    old_bytes: Vec<u8>,
    new_doc: Document,
    new_ids: core::num::NonZeroU64,
}

/// A folder with a saved project, and a different document to save over it.
fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Trip.freecut");
    let (old, old_ids) = kitchen_sink();
    let extras = Extras::new();
    save(
        &path,
        &SaveRequest {
            document: &old,
            ids_next: old_ids,
            extras: &extras,
            backup: false,
            relative_base: None,
            export: None,
            ui: None,
        },
    )
    .unwrap();
    let old_bytes = std::fs::read(&path).unwrap();
    let plan = proptest::strategy::ValueTree::current(
        &proptest::strategy::Strategy::new_tree(
            &doc_plan(),
            &mut proptest::test_runner::TestRunner::deterministic(),
        )
        .unwrap(),
    );
    let (new_doc, new_ids) = build(&plan);
    Setup {
        dir: dir.path().to_path_buf(),
        _dir: dir,
        path,
        old_bytes,
        new_doc,
        new_ids,
    }
}

fn request<'a>(
    doc: &'a Document,
    ids: core::num::NonZeroU64,
    extras: &'a Extras,
    backup: bool,
) -> SaveRequest<'a> {
    SaveRequest {
        document: doc,
        ids_next: ids,
        extras,
        backup,
        relative_base: None,
        export: None,
        ui: None,
    }
}

/// File names in the folder, sorted.
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn failed_step(result: Result<(), SaveError>) -> SaveStep {
    match result {
        Err(SaveError::Io { step, .. }) => step,
        other => panic!("expected an I/O failure, got {other:?}"),
    }
}

#[test]
fn failures_before_the_rename_keep_the_old_file_and_leave_no_temp() {
    for (fail, step) in [
        (Fail::TempWrite, SaveStep::WriteTemp),
        (Fail::TempSync, SaveStep::WriteTemp),
        (Fail::Rename, SaveStep::Replace),
    ] {
        let s = setup();
        let fs = Faulty::new(fail);
        let extras = Extras::new();
        let result = save_with(
            &fs,
            &s.path,
            &request(&s.new_doc, s.new_ids, &extras, false),
        );
        assert_eq!(failed_step(result), step, "{fail:?}");
        assert_eq!(
            std::fs::read(&s.path).unwrap(),
            s.old_bytes,
            "{fail:?}: the previous file is intact"
        );
        assert_eq!(
            listing(&s.dir),
            vec!["Trip.freecut"],
            "{fail:?}: no temp left"
        );
        // The old project still opens.
        assert_eq!(open(&s.path).unwrap().document, kitchen_sink().0);
    }
}

#[test]
fn a_failed_directory_sync_reports_but_leaves_the_complete_new_file() {
    let s = setup();
    let fs = Faulty::new(Fail::SyncDir);
    let extras = Extras::new();
    let result = save_with(
        &fs,
        &s.path,
        &request(&s.new_doc, s.new_ids, &extras, false),
    );
    assert_eq!(failed_step(result), SaveStep::SyncDir);
    assert_eq!(
        std::fs::read(&s.path).unwrap(),
        encode(&s.new_doc, s.new_ids, &extras).unwrap()
    );
    assert_eq!(listing(&s.dir), vec!["Trip.freecut"]);
}

#[test]
fn verify_after_write_catches_a_corrupting_file_system() {
    let s = setup();
    let fs = Faulty::new(Fail::ReadBackCorrupt);
    let extras = Extras::new();
    let result = save_with(
        &fs,
        &s.path,
        &request(&s.new_doc, s.new_ids, &extras, false),
    );
    assert!(
        matches!(result, Err(SaveError::Verify { ref path }) if *path == s.path),
        "{result:?}"
    );
}

#[test]
fn the_temp_file_is_written_next_to_the_target_and_renamed_over_it() {
    let s = setup();
    let fs = Faulty::new(Fail::Backup); // never triggers without backup
    let extras = Extras::new();
    save_with(
        &fs,
        &s.path,
        &request(&s.new_doc, s.new_ids, &extras, false),
    )
    .unwrap();
    let log = fs.log.lock().unwrap().clone();
    assert_eq!(log.len(), 2, "{log:?}");
    let tmp = log[0].strip_prefix("write ").unwrap();
    assert!(
        tmp.starts_with(&s.dir.join(".Trip.freecut.").display().to_string()),
        "{tmp}"
    );
    assert!(tmp.ends_with(".tmp"));
    assert_eq!(log[1], format!("rename {tmp} -> {}", s.path.display()));
    assert_eq!(open(&s.path).unwrap().document, s.new_doc);
}

#[test]
fn backup_keeps_a_one_time_copy_of_the_replaced_file() {
    let s = setup();
    let extras = Extras::new();
    let bak = s.dir.join("Trip.freecut.bak");

    // First save with backup: the old file is kept.
    save(&s.path, &request(&s.new_doc, s.new_ids, &extras, true)).unwrap();
    assert_eq!(std::fs::read(&bak).unwrap(), s.old_bytes);
    assert_eq!(open(&s.path).unwrap().document, s.new_doc);

    // A later backup request never overwrites the first copy.
    let (sink, sink_ids) = kitchen_sink();
    save(&s.path, &request(&sink, sink_ids, &extras, true)).unwrap();
    assert_eq!(std::fs::read(&bak).unwrap(), s.old_bytes);
    assert_eq!(listing(&s.dir), vec!["Trip.freecut", "Trip.freecut.bak"]);

    // No old file, no backup.
    let fresh = s.dir.join("New.freecut");
    save(&fresh, &request(&sink, sink_ids, &extras, true)).unwrap();
    assert!(!s.dir.join("New.freecut.bak").exists());
}

#[test]
fn a_failed_backup_stops_the_save_before_anything_is_replaced() {
    let s = setup();
    let fs = Faulty::new(Fail::Backup);
    let extras = Extras::new();
    let result = save_with(&fs, &s.path, &request(&s.new_doc, s.new_ids, &extras, true));
    assert_eq!(failed_step(result), SaveStep::WriteBackup);
    assert_eq!(std::fs::read(&s.path).unwrap(), s.old_bytes);
    assert_eq!(
        listing(&s.dir),
        vec!["Trip.freecut"],
        "no partial backup left"
    );
}

#[test]
fn an_invalid_document_is_never_written() {
    let s = setup();
    let extras = Extras::new();
    // A counter below the ids in use would let ids be reused.
    let result = save(&s.path, &request(&s.new_doc, nz(1), &extras, false));
    assert!(
        matches!(result, Err(SaveError::InvalidDocument(_))),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&s.path).unwrap(), s.old_bytes);
}

#[test]
fn saving_into_a_missing_folder_fails_cleanly() {
    let s = setup();
    let extras = Extras::new();
    let path = s.dir.join("missing").join("Trip.freecut");
    let result = save(&path, &request(&s.new_doc, s.new_ids, &extras, false));
    assert_eq!(failed_step(result), SaveStep::WriteTemp);
    assert!(matches!(
        save(
            Path::new("/"),
            &request(&s.new_doc, s.new_ids, &extras, false)
        ),
        Err(SaveError::InvalidPath { .. })
    ));
}

// ---- review regressions ----

/// F8: a long but legal project name can be saved: the temp name never
/// exceeds the file-name limit.
#[cfg(unix)]
#[test]
fn f8_long_project_names_can_be_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{}.freecut", "a".repeat(240)));
    std::fs::write(&path, b"probe").unwrap();
    let (doc, ids) = kitchen_sink();
    let extras = Extras::new();
    let r = save(
        &path,
        &SaveRequest {
            document: &doc,
            ids_next: ids,
            extras: &extras,
            backup: false,
            relative_base: None,
            export: None,
            ui: None,
        },
    );
    assert!(r.is_ok(), "{r:?}");
}

/// F9: a truncated `.bak` left by a crash doesn't stand in for the original.
#[test]
fn f9_a_truncated_backup_does_not_stand_in_for_the_original() {
    use freecut_project::decode;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Trip.freecut");
    let (doc, ids) = kitchen_sink();
    let extras = Extras::new();
    let original = encode(&doc, ids, &extras).unwrap();
    std::fs::write(&path, &original).unwrap();
    std::fs::write(dir.path().join("Trip.freecut.bak"), &original[..10]).unwrap();
    let mut v = to_value(&original);
    v["document"]["settings"]["match_first_clip"] = json!(false);
    let changed = decode(&compact(&v)).unwrap();
    save(
        &path,
        &SaveRequest {
            document: &changed.document,
            ids_next: changed.ids.next(),
            extras: &changed.extras,
            backup: true,
            relative_base: None,
            export: None,
            ui: None,
        },
    )
    .unwrap();
    let kept = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| std::fs::read(e.unwrap().path()).unwrap())
        .any(|b| b == original);
    assert!(kept);
    // The fragment was replaced by a complete copy, written via a temp file.
    assert_eq!(
        std::fs::read(dir.path().join("Trip.freecut.bak")).unwrap(),
        original
    );
    assert_eq!(
        listing(dir.path()),
        vec!["Trip.freecut", "Trip.freecut.bak"]
    );
}

/// F3, F10: what can't be read back exactly is refused before writing.
#[test]
fn values_the_file_cant_hold_are_never_written() {
    use std::sync::Arc;

    use freecut_timeline::ids::MAX_ID;
    use freecut_timeline::model::Library;
    use freecut_timeline::time::Tick;

    let s = setup();
    let extras = Extras::new();
    let (doc, ids) = kitchen_sink();
    let result = save(&s.path, &request(&doc, nz(MAX_ID + 2), &extras, false));
    assert!(
        matches!(&result, Err(SaveError::OutOfRange { path, .. }) if path == "ids_next"),
        "{result:?}"
    );

    // A media item longer than 2^53 ticks (about 148 days).
    let mut items: Vec<_> = doc.library().items().cloned().collect();
    let mut song = (*items[1]).clone();
    song.info.duration = Tick::from_raw((1 << 53) + 1);
    items[1] = Arc::new(song);
    let long = Document::from_parts(
        doc.root(),
        *doc.settings(),
        Library::from_parts(items).unwrap(),
        doc.sequences().map(|q| Arc::new(q.clone())).collect(),
    )
    .unwrap();
    let result = save(&s.path, &request(&long, ids, &extras, false));
    match result {
        Err(SaveError::OutOfRange { path, message }) => {
            assert_eq!(path, "document.library[1].info.duration");
            assert!(message.contains("9007199254740993"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&s.path).unwrap(), s.old_bytes, "untouched");
    assert_eq!(listing(&s.dir), vec!["Trip.freecut"]);
}

/// F8: with a long name, the backup gets a shortened name of its own, is
/// kept once, and nothing else is left behind.
#[cfg(unix)]
#[test]
fn long_project_names_get_a_backup_too() {
    let dir = tempfile::tempdir().unwrap();
    let name = format!("{}.freecut", "旅".repeat(82));
    let path = dir.path().join(&name);
    let (doc, ids) = kitchen_sink();
    let extras = Extras::new();
    let original = encode(&doc, ids, &extras).unwrap();
    std::fs::write(&path, &original).unwrap();
    let s = setup();
    for _ in 0..2 {
        save(&path, &request(&s.new_doc, s.new_ids, &extras, true)).unwrap();
    }
    let names = listing(dir.path());
    assert_eq!(names.len(), 2, "{names:?}");
    let bak = names.iter().find(|n| **n != name).unwrap();
    assert!(bak.ends_with(".bak") && bak.len() <= 255, "{bak}");
    assert_eq!(std::fs::read(dir.path().join(bak)).unwrap(), original);
    assert_eq!(open(&path).unwrap().document, s.new_doc);
}
