//! The project lock with real OS file locks: busy while held, released on
//! drop, stale after a crash.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use freecut_project::lock::{LockError, LockInfo, ProjectLock, StaleLock};
use pretty_assertions::assert_eq;

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Trip.freecut");
    (dir, path)
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[test]
fn a_held_lock_is_busy_and_names_its_holder() {
    let (_dir, path) = project();
    let lock = ProjectLock::acquire(&path).unwrap();
    assert!(lock.stale().is_none());
    assert_eq!(lock.info().pid, std::process::id());
    assert!(lock.path().exists());
    #[cfg(not(windows))]
    {
        let on_disk: LockInfo =
            serde_json::from_slice(&std::fs::read(lock.path()).unwrap()).unwrap();
        assert_eq!(&on_disk, lock.info());
    }
    #[cfg(windows)]
    assert!(
        std::fs::read(lock.path()).is_err(),
        "the OS lock denies other readers"
    );

    match ProjectLock::acquire(&path) {
        Err(LockError::Busy { holder }) => {
            // Windows' OS lock makes the file unreadable to others.
            if cfg!(unix) {
                assert_eq!(holder.as_ref(), Some(lock.info()));
            }
            if cfg!(windows) {
                assert!(holder.is_none());
            }
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    // "Open anyway" can't steal a lock held by a live process here.
    assert!(matches!(
        ProjectLock::take_over(&path),
        Err(LockError::Busy { .. })
    ));
    let message = ProjectLock::acquire(&path).unwrap_err().to_string();
    assert!(
        message.starts_with("the project is open in another FreeCut"),
        "{message}"
    );
}

#[test]
fn dropping_the_lock_releases_and_removes_it() {
    let (_dir, path) = project();
    let lock_path = ProjectLock::lock_path(&path);
    {
        let _lock = ProjectLock::acquire(&path).unwrap();
        assert!(lock_path.exists());
    }
    assert!(!lock_path.exists());
    let again = ProjectLock::acquire(&path).unwrap();
    assert!(
        again.stale().is_none(),
        "a clean release leaves nothing stale"
    );
    again.release().unwrap();
    assert!(!lock_path.exists());
}

#[test]
fn metadata_left_by_a_crash_is_stale_and_taken_over() {
    let (_dir, path) = project();
    let lock_path = ProjectLock::lock_path(&path);
    // Simulate a crash: the metadata stays, the OS lock is gone.
    let lock = ProjectLock::acquire(&path).unwrap();
    let crashed = lock.info().clone();
    let left_behind = serde_json::to_vec(&crashed).unwrap();
    drop(lock);
    std::fs::write(&lock_path, &left_behind).unwrap();

    let lock = ProjectLock::acquire(&path).unwrap();
    assert_eq!(
        lock.stale(),
        Some(&StaleLock {
            holder: Some(crashed)
        })
    );
}

#[test]
fn unreadable_metadata_is_stale_without_a_holder() {
    let (_dir, path) = project();
    std::fs::write(ProjectLock::lock_path(&path), b"\0garbage").unwrap();
    let lock = ProjectLock::acquire(&path).unwrap();
    assert_eq!(lock.stale(), Some(&StaleLock { holder: None }));
}

#[test]
fn another_hosts_lock_is_busy_while_fresh() {
    let (_dir, path) = project();
    let remote = |refreshed_ms| LockInfo {
        pid: 7,
        host: "some-other-machine.invalid".into(),
        user: Some("them".into()),
        app: "freecut 0.1.0".into(),
        started_ms: 0,
        refreshed_ms,
    };
    let write = |info: &LockInfo| {
        std::fs::write(
            ProjectLock::lock_path(&path),
            serde_json::to_vec(info).unwrap(),
        )
        .unwrap();
    };

    let fresh = remote(now_ms());
    write(&fresh);
    match ProjectLock::acquire(&path) {
        Err(LockError::Busy { holder }) => assert_eq!(holder, Some(fresh.clone())),
        other => panic!("expected Busy, got {other:?}"),
    }
    let lock = ProjectLock::take_over(&path).unwrap();
    assert_eq!(lock.stale().unwrap().holder, Some(fresh));
    drop(lock);

    let old = remote(0);
    write(&old);
    let lock = ProjectLock::acquire(&path).unwrap();
    assert_eq!(lock.stale().unwrap().holder, Some(old));
}

// ---- review regressions ----

/// F4: a project reached through a symlink shares the real file's lock.
#[cfg(unix)]
#[test]
fn f4_a_symlinked_project_file_is_still_single_writer() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("Trip.freecut");
    std::fs::write(&real, b"{}").unwrap();
    let alias = dir.path().join("Alias.freecut");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let _a = ProjectLock::acquire(&real).unwrap();
    let b = ProjectLock::acquire(&alias);
    assert!(matches!(b, Err(LockError::Busy { .. })), "{b:?}");
}

/// F13: a lock file removed under a live holder is noticed by its refresh.
#[cfg(unix)]
#[test]
fn f13_refresh_notices_a_vanished_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("Trip.freecut");
    let mut a = ProjectLock::acquire(&project).unwrap();
    std::fs::remove_file(a.path()).unwrap();
    let refreshed = a.refresh();
    let b = ProjectLock::acquire(&project);
    assert!(refreshed.is_err() || matches!(b, Err(LockError::Busy { .. })));
}

/// F13: releasing a lock whose file was replaced never deletes the new
/// holder's file.
#[cfg(unix)]
#[test]
fn f13b_dropping_a_superseded_lock_deletes_the_new_holders_file() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("Trip.freecut");
    let a = ProjectLock::acquire(&project).unwrap();
    std::fs::remove_file(a.path()).unwrap();
    let b = ProjectLock::acquire(&project).unwrap();
    drop(a);
    assert!(b.path().exists());
}

/// F4: the lock path is the same for every path to one file, including a
/// symlinked folder.
#[cfg(unix)]
#[test]
fn every_path_to_one_project_names_one_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let real_dir = dir.path().join("Projects");
    std::fs::create_dir_all(&real_dir).unwrap();
    let real = real_dir.join("Trip.freecut");
    std::fs::write(&real, b"{}").unwrap();
    let file_link = dir.path().join("Alias.freecut");
    std::os::unix::fs::symlink(&real, &file_link).unwrap();
    let dir_link = dir.path().join("Shortcut");
    std::os::unix::fs::symlink(&real_dir, &dir_link).unwrap();

    let expected = ProjectLock::lock_path(&real);
    assert_eq!(ProjectLock::lock_path(&file_link), expected);
    assert_eq!(
        ProjectLock::lock_path(&dir_link.join("Trip.freecut")),
        expected
    );
    assert_eq!(
        expected.file_name().unwrap(),
        "Trip.freecut.lock",
        "the sidecar sits next to the real file"
    );
    // A project that doesn't exist yet still gets its folder resolved.
    assert_eq!(
        ProjectLock::lock_path(&dir_link.join("New.freecut")),
        expected.with_file_name("New.freecut.lock")
    );

    let _held = ProjectLock::acquire(&dir_link.join("Trip.freecut")).unwrap();
    assert!(matches!(
        ProjectLock::take_over(&file_link),
        Err(LockError::Busy { .. })
    ));
}

/// F13: after its file was removed, a refresh recreates and relocks it.
#[cfg(unix)]
#[test]
fn refresh_recreates_a_removed_lock_file() {
    let (_dir, path) = project();
    let mut a = ProjectLock::acquire(&path).unwrap();
    std::fs::remove_file(a.path()).unwrap();
    a.refresh().unwrap();
    let on_disk: LockInfo = serde_json::from_slice(&std::fs::read(a.path()).unwrap()).unwrap();
    assert_eq!(&on_disk, a.info());
    assert!(matches!(
        ProjectLock::acquire(&path),
        Err(LockError::Busy { .. })
    ));
    drop(a);
    assert!(!ProjectLock::lock_path(&path).exists());
}

/// F13: when another session locked the project after the file vanished,
/// refresh reports the loss, and releasing leaves the new holder alone.
#[cfg(unix)]
#[test]
fn refresh_reports_a_lock_taken_by_another_session() {
    let (_dir, path) = project();
    let mut a = ProjectLock::acquire(&path).unwrap();
    std::fs::remove_file(a.path()).unwrap();
    let b = ProjectLock::acquire(&path).unwrap();
    match a.refresh() {
        Err(LockError::Lost { holder }) => assert_eq!(holder.as_ref(), Some(b.info())),
        other => panic!("expected Lost, got {other:?}"),
    }
    a.release().unwrap();
    let on_disk: LockInfo = serde_json::from_slice(&std::fs::read(b.path()).unwrap()).unwrap();
    assert_eq!(&on_disk, b.info(), "B's lock file is untouched");
    assert!(matches!(
        ProjectLock::acquire(&path),
        Err(LockError::Busy { .. })
    ));
}

/// F13: a lock file replaced (by rename) with a fresh lock from another host
/// is that host's lock now.
#[cfg(unix)]
#[test]
fn refresh_respects_a_replacement_from_another_host() {
    let (dir, path) = project();
    let mut a = ProjectLock::acquire(&path).unwrap();
    let theirs = LockInfo {
        pid: 9,
        host: "some-other-machine.invalid".into(),
        user: None,
        app: "freecut 0.1.0".into(),
        started_ms: now_ms(),
        refreshed_ms: now_ms(),
    };
    let incoming = dir.path().join("incoming");
    std::fs::write(&incoming, serde_json::to_vec(&theirs).unwrap()).unwrap();
    std::fs::rename(&incoming, a.path()).unwrap();
    match a.refresh() {
        Err(LockError::Lost { holder }) => assert_eq!(holder, Some(theirs.clone())),
        other => panic!("expected Lost, got {other:?}"),
    }
    drop(a);
    let on_disk: LockInfo =
        serde_json::from_slice(&std::fs::read(ProjectLock::lock_path(&path)).unwrap()).unwrap();
    assert_eq!(on_disk, theirs);
}
