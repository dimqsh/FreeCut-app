//! Media path storage and resolution (PM-100) over a virtual file system.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use freecut_project::fs::FileSystem;
use freecut_project::paths::{Resolved, ResolvedHow, resolve, stored_path};
use freecut_timeline::model::StoredPath;
use pretty_assertions::assert_eq;

/// A file system that only knows which files exist.
#[derive(Default)]
struct MemFs {
    files: BTreeSet<PathBuf>,
}

impl MemFs {
    fn with(files: &[PathBuf]) -> MemFs {
        MemFs {
            files: files.iter().cloned().collect(),
        }
    }
}

impl FileSystem for MemFs {
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains(path)
    }
    fn read(&self, _: &Path) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn write_new(&self, _: &Path, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_file(&self, _: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn sync_dir(&self, _: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// An absolute native path from `/`-separated parts.
fn abs(parts: &str) -> PathBuf {
    let mut p = PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" });
    for part in parts.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

fn stored(absolute: &Path, project_dir: &Path) -> StoredPath {
    stored_path(absolute, project_dir).unwrap()
}

struct Case {
    name: &'static str,
    stored: StoredPath,
    project_dir: PathBuf,
    files: Vec<PathBuf>,
    expected: Resolved,
}

fn found(path: PathBuf, how: ResolvedHow) -> Resolved {
    Resolved { path, how }
}

#[test]
fn resolution_table() {
    let old_dir = abs("old/Trip");
    let in_project = stored(&abs("old/Trip/Media/a.mov"), &old_dir);
    let beside = stored(&abs("old/Footage/b.mov"), &old_dir);
    assert_eq!(in_project.relative.as_deref(), Some("Media/a.mov"));
    assert_eq!(beside.relative.as_deref(), Some("../Footage/b.mov"));

    let cases = vec![
        Case {
            name: "nothing moved: the relative path finds it first",
            stored: in_project.clone(),
            project_dir: old_dir.clone(),
            files: vec![abs("old/Trip/Media/a.mov")],
            expected: found(abs("old/Trip/Media/a.mov"), ResolvedHow::Relative),
        },
        Case {
            name: "project folder moved with its media",
            stored: in_project.clone(),
            project_dir: abs("new/place/Trip"),
            files: vec![abs("new/place/Trip/Media/a.mov")],
            expected: found(abs("new/place/Trip/Media/a.mov"), ResolvedHow::Relative),
        },
        Case {
            name: "project moved, media stayed: the absolute path finds it",
            stored: in_project.clone(),
            project_dir: abs("new/place/Trip"),
            files: vec![abs("old/Trip/Media/a.mov")],
            expected: found(abs("old/Trip/Media/a.mov"), ResolvedHow::Absolute),
        },
        Case {
            name: "'..' paths: the parent layout moved together",
            stored: beside.clone(),
            project_dir: abs("disk2/Trip"),
            files: vec![abs("disk2/Footage/b.mov")],
            expected: found(abs("disk2/Footage/b.mov"), ResolvedHow::Relative),
        },
        Case {
            name: "both forms exist: the relative one wins",
            stored: beside.clone(),
            project_dir: abs("disk2/Trip"),
            files: vec![abs("disk2/Footage/b.mov"), abs("old/Footage/b.mov")],
            expected: found(abs("disk2/Footage/b.mov"), ResolvedHow::Relative),
        },
        Case {
            name: "neither exists: offline, reported at the stored absolute path",
            stored: beside.clone(),
            project_dir: abs("disk2/Trip"),
            files: vec![],
            expected: found(abs("old/Footage/b.mov"), ResolvedHow::NotFound),
        },
        Case {
            name: "Windows separators in a stored relative path",
            stored: StoredPath {
                absolute: abs("gone/a.mov").to_str().unwrap().into(),
                relative: Some(r"..\Footage\b.mov".into()),
            },
            project_dir: abs("disk2/Trip"),
            files: vec![abs("disk2/Footage/b.mov")],
            expected: found(abs("disk2/Footage/b.mov"), ResolvedHow::Relative),
        },
        Case {
            name: "a relative path that climbs above the root is ignored",
            stored: StoredPath {
                absolute: abs("x/a.mov").to_str().unwrap().into(),
                relative: Some("../../../../a.mov".into()),
            },
            project_dir: abs("p"),
            files: vec![abs("x/a.mov")],
            expected: found(abs("x/a.mov"), ResolvedHow::Absolute),
        },
        Case {
            name: "an absolute string in the relative slot is ignored",
            stored: StoredPath {
                absolute: abs("x/a.mov").to_str().unwrap().into(),
                relative: Some("/etc/a.mov".into()),
            },
            project_dir: abs("p"),
            files: vec![abs("etc/a.mov"), abs("p/etc/a.mov")],
            expected: found(abs("x/a.mov"), ResolvedHow::NotFound),
        },
    ];
    for case in cases {
        let fs = MemFs::with(&case.files);
        assert_eq!(
            resolve(&case.stored, &case.project_dir, &fs),
            case.expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn a_file_on_another_volume_has_no_relative_path() {
    // Windows drives are compared as volumes on every host.
    let sp = StoredPath {
        absolute: r"D:\clips\a.mov".into(),
        relative: freecut_project::paths::relative_path(r"C:\Users\me\Trip", r"D:\clips\a.mov"),
    };
    assert_eq!(sp.relative, None);
    let fs = MemFs::default();
    let r = resolve(&sp, &abs("Users/me/Trip"), &fs);
    assert_eq!(r.how, ResolvedHow::NotFound);
}

#[cfg(unix)]
#[test]
fn a_windows_project_opened_on_unix_resolves_through_its_relative_paths() {
    let sp = StoredPath {
        absolute: r"C:\Users\me\Trip\Media\a.mov".into(),
        relative: Some("Media/a.mov".into()),
    };
    let fs = MemFs::with(&[abs("Volumes/Shared/Trip/Media/a.mov")]);
    assert_eq!(
        resolve(&sp, &abs("Volumes/Shared/Trip"), &fs),
        found(
            abs("Volumes/Shared/Trip/Media/a.mov"),
            ResolvedHow::Relative
        )
    );
    // The foreign absolute path is never joined onto a local folder or the
    // current directory.
    let only_relative_name = MemFs::with(&[PathBuf::from(r"C:\Users\me\Trip\Media\a.mov")]);
    let r = resolve(
        &StoredPath {
            relative: None,
            ..sp
        },
        &abs("Volumes/Shared/Trip"),
        &only_relative_name,
    );
    assert_eq!(r.how, ResolvedHow::NotFound);
}

#[test]
fn stored_paths_from_real_absolute_paths() {
    let dir = abs("Users/me/Movies/Trip");
    let cases = [
        (
            "Users/me/Movies/Trip/Media/clip.mov",
            Some("Media/clip.mov"),
        ),
        ("Users/me/Movies/Trip/clip.mov", Some("clip.mov")),
        ("Users/me/Desktop/clip.mov", Some("../../Desktop/clip.mov")),
        ("clip.mov", Some("../../../../clip.mov")),
    ];
    for (file, relative) in cases {
        let sp = stored(&abs(file), &dir);
        assert_eq!(sp.absolute, abs(file).to_str().unwrap(), "{file}");
        assert_eq!(sp.relative.as_deref(), relative, "{file}");
    }
}

// ---- review regressions ----

/// F2: a POSIX file name with a colon keeps a usable relative path.
#[cfg(unix)]
#[test]
fn f2_relative_path_with_colon_survives_a_project_move() {
    use freecut_project::fs::RealFs;

    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old").join("Trip");
    std::fs::create_dir_all(&old).unwrap();
    let media = old.join("1:30 take.mov");
    std::fs::write(&media, b"x").unwrap();
    let sp = stored_path(&media, &old).unwrap();
    assert_eq!(sp.relative.as_deref(), Some("1:30 take.mov"));
    let new = root.path().join("new");
    std::fs::rename(root.path().join("old"), &new).unwrap();
    let r = resolve(&sp, &new.join("Trip"), &RealFs);
    assert_eq!(r.how, ResolvedHow::Relative);
}
