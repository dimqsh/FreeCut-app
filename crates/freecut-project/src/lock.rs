//! The single-writer project lock (PM-013).
//!
//! A sidecar file `<name>.freecut.lock` next to the project holds the
//! holder's metadata (pid, host, user, app, start and refresh times) and is
//! locked with the OS file lock (`File::try_lock`) for as long as the
//! project is open. The OS releases that lock when the process dies, so:
//!
//! - **Busy**: the OS lock is held by another process (or another
//!   [`ProjectLock`] in this one), or the metadata names another host and was
//!   refreshed within [`STALE_AFTER`] (OS locks don't reliably cross network
//!   file systems, so a live remote holder is recognized by its refreshes).
//! - **Stale**: metadata is left over but nobody holds the lock: the previous
//!   session crashed. [`ProjectLock::acquire`] takes the lock over and
//!   reports the old holder through [`ProjectLock::stale`], so the app can
//!   offer recovery (PM-015).
//!
//! Dropping a [`ProjectLock`] removes the file and releases the OS lock.
//!
//! The lock belongs to the project *file*, not to the path used to reach
//! it: [`ProjectLock::lock_path`] resolves symbolic links first, so opening
//! a project through a symlink (or a symlinked folder) finds the same lock.
//! (Hard links get separate locks, but the first save through either path
//! replaces that path with a file of its own, so they don't share a file
//! for long.)
//!
//! The lock file can disappear or be replaced while it is held (a user
//! cleaning up, a sync client restoring its copy). [`ProjectLock::refresh`]
//! notices: it locks the file now at that path again, or reports
//! [`LockError::Lost`] when another FreeCut got there first. Releasing never
//! deletes a lock file that isn't the one this lock holds.
//!
//! Limitations on Windows: the OS lock makes the file unreadable to other
//! processes, so a `Busy` error there has no holder details; and only a
//! removed lock file is noticed, not one replaced by another file (std has
//! no stable file identity there).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::format::APP_ID;
use crate::names::sibling_name;

/// A lock from another host counts as live while it was refreshed this
/// recently. The holder should call [`ProjectLock::refresh`] about once a
/// minute.
pub const STALE_AFTER: Duration = Duration::from_secs(120);

/// Who holds (or held) a project lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    pub pid: u32,
    pub host: String,
    pub user: Option<String>,
    /// The holder's [`APP_ID`].
    pub app: String,
    /// Wall-clock milliseconds since the Unix epoch.
    pub started_ms: u64,
    pub refreshed_ms: u64,
}

/// The lock a crashed session left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleLock {
    /// The old holder, when its metadata was readable.
    pub holder: Option<LockInfo>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LockError {
    /// Another FreeCut has the project open.
    #[error("the project is open in another FreeCut{}", describe(.holder.as_ref()))]
    Busy { holder: Option<LockInfo> },
    /// The lock file was removed or replaced while this lock was held, and
    /// another FreeCut has locked the project since. This session no longer
    /// has it: saving would overwrite the other session's work.
    #[error("another FreeCut took over the project{}", describe(.holder.as_ref()))]
    Lost { holder: Option<LockInfo> },
    #[error("cannot lock {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn describe(holder: Option<&LockInfo>) -> String {
    holder.map_or_else(String::new, |h| {
        format!(" (host {}, process {})", h.host, h.pid)
    })
}

/// A held project lock. Released on drop.
#[derive(Debug)]
pub struct ProjectLock {
    /// `None` once released.
    file: Option<File>,
    path: PathBuf,
    info: LockInfo,
    stale: Option<StaleLock>,
}

/// Who is asking for the lock, and when.
#[derive(Debug, Clone)]
struct Me {
    pid: u32,
    host: String,
    user: Option<String>,
    now_ms: u64,
}

impl Me {
    fn current() -> Me {
        Me {
            pid: std::process::id(),
            host: gethostname::gethostname().to_string_lossy().into_owned(),
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            now_ms: now_ms(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Whether a lock from another host should be treated as live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Remote {
    Respect,
    TakeOver,
}

/// How often to retry when the lock file is replaced under us.
const ATTEMPTS: usize = 3;

impl ProjectLock {
    /// The lock file for `project`: `<file name>.lock` next to the real
    /// project file. Symbolic links to the file or to a folder above it are
    /// resolved first, so every path that reaches one file names one lock
    /// file. A name too long for the suffix is shortened.
    pub fn lock_path(project: &Path) -> PathBuf {
        let real = real_path(project);
        let name = real.file_name().unwrap_or_default();
        let lock_name = sibling_name("", name, ".lock");
        real.with_file_name(lock_name)
    }

    /// Locks `project` for this process. Fails with [`LockError::Busy`] when
    /// another live session holds it; takes over a stale lock.
    pub fn acquire(project: &Path) -> Result<ProjectLock, LockError> {
        acquire_as(project, &Me::current(), Remote::Respect)
    }

    /// Like [`ProjectLock::acquire`], but also takes over a recently
    /// refreshed lock from another host ("Open anyway"). A lock held by a
    /// live process on this machine still fails with `Busy`.
    pub fn take_over(project: &Path) -> Result<ProjectLock, LockError> {
        acquire_as(project, &Me::current(), Remote::TakeOver)
    }

    /// The lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The metadata this lock wrote.
    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    /// The lock left by a crashed session, if one was taken over.
    pub fn stale(&self) -> Option<&StaleLock> {
        self.stale.as_ref()
    }

    /// Rewrites the refresh time, so other hosts keep seeing the lock as
    /// live. Call it about once a minute.
    ///
    /// If the lock file was removed or replaced since the last call, the
    /// file now at the lock path is locked and written instead, so other
    /// sessions see the project as open again. If another session has
    /// locked it meanwhile (here, or recently from another host), this
    /// fails with [`LockError::Lost`].
    pub fn refresh(&mut self) -> Result<(), LockError> {
        self.refresh_at(now_ms())
    }

    fn refresh_at(&mut self, now_ms: u64) -> Result<(), LockError> {
        let io_err = |path: &Path, source| LockError::Io {
            path: path.to_path_buf(),
            source,
        };
        let Some(held) = &self.file else {
            return Ok(());
        };
        if !same_file(held, &self.path).map_err(|e| io_err(&self.path, e))? {
            let (file, previous) = match lock_file(&self.path) {
                Ok(locked) => locked,
                Err(LockError::Busy { holder }) => return Err(LockError::Lost { holder }),
                Err(e) => return Err(e),
            };
            let holder: Option<LockInfo> = serde_json::from_slice(&previous).ok();
            if let Some(remote) = live_remote(holder.as_ref(), &self.info.host, now_ms) {
                // Dropping `file` releases the OS lock; the file is unchanged.
                return Err(LockError::Lost {
                    holder: Some(remote.clone()),
                });
            }
            // The old handle only locked a file nobody can reach any more.
            self.file = Some(file);
        }
        self.info.refreshed_ms = now_ms;
        let Some(file) = &self.file else {
            return Ok(());
        };
        write_info(file, &self.info).map_err(|e| io_err(&self.path, e))
    }

    /// Removes the lock file and releases the lock, reporting errors that
    /// dropping would ignore.
    pub fn release(mut self) -> Result<(), LockError> {
        self.release_inner()
    }

    fn release_inner(&mut self) -> Result<(), LockError> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        // Remove the file only while it is the one we hold: after a removal
        // or replacement, the file at the path may be another session's live
        // lock. Remove while still locked: anyone who opened the old file
        // before the removal notices it is gone once they get the lock.
        let removed = match same_file(&file, &self.path) {
            Ok(true) => match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
            Ok(false) => Ok(()),
            Err(e) => Err(e),
        };
        let unlocked = file.unlock();
        removed.and(unlocked).map_err(|source| LockError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        if let Err(err) = self.release_inner() {
            tracing::warn!(%err, "releasing the project lock failed");
        }
    }
}

/// `project` with symbolic links resolved: the canonical path of the file,
/// or of its folder when the file doesn't exist yet, or `project` itself
/// when neither resolves.
fn real_path(project: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(project) {
        return real;
    }
    match (project.parent(), project.file_name()) {
        (Some(dir), Some(name)) => {
            let dir = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            std::fs::canonicalize(dir).map_or_else(|_| project.to_path_buf(), |d| d.join(name))
        }
        _ => project.to_path_buf(),
    }
}

/// Opens (creating it if needed) and OS-locks the lock file at `path`, and
/// returns it with what it held before. Starts over when the file is
/// removed or replaced between opening and locking it.
fn lock_file(path: &Path) -> Result<(File, Vec<u8>), LockError> {
    let io_err = |source| LockError::Io {
        path: path.to_path_buf(),
        source,
    };
    for _ in 0..ATTEMPTS {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io_err)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(LockError::Busy {
                    holder: read_holder(path),
                });
            }
            Err(TryLockError::Error(e)) => return Err(io_err(e)),
        }
        // The previous holder may have removed the file between our open
        // and our lock; then we locked an orphan and must start over.
        if !same_file(&file, path).map_err(io_err)? {
            continue;
        }
        let previous = read_all(&file).map_err(io_err)?;
        return Ok((file, previous));
    }
    Err(io_err(io::Error::other(
        "the lock file kept being replaced while locking it",
    )))
}

/// `holder`, when it is a session on another host that refreshed its lock
/// recently: OS locks don't reliably cross network file systems, so such a
/// lock counts as live.
fn live_remote<'a>(
    holder: Option<&'a LockInfo>,
    my_host: &str,
    now_ms: u64,
) -> Option<&'a LockInfo> {
    holder.filter(|h| {
        h.host != my_host && h.refreshed_ms.saturating_add(duration_ms(STALE_AFTER)) > now_ms
    })
}

fn acquire_as(project: &Path, me: &Me, remote: Remote) -> Result<ProjectLock, LockError> {
    let path = ProjectLock::lock_path(project);
    let (file, previous) = lock_file(&path)?;
    let stale = if previous.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        Some(StaleLock {
            holder: serde_json::from_slice(&previous).ok(),
        })
    };
    if remote == Remote::Respect
        && let Some(holder) = live_remote(
            stale.as_ref().and_then(|s| s.holder.as_ref()),
            &me.host,
            me.now_ms,
        )
    {
        // Dropping `file` releases our OS lock; the file is unchanged.
        return Err(LockError::Busy {
            holder: Some(holder.clone()),
        });
    }
    let info = LockInfo {
        pid: me.pid,
        host: me.host.clone(),
        user: me.user.clone(),
        app: APP_ID.to_string(),
        started_ms: me.now_ms,
        refreshed_ms: me.now_ms,
    };
    write_info(&file, &info).map_err(|source| LockError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(ProjectLock {
        file: Some(file),
        path,
        info,
        stale,
    })
}

fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn read_holder(path: &Path) -> Option<LockInfo> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn read_all(mut file: &File) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut out = Vec::new();
    file.read_to_end(&mut out)?;
    Ok(out)
}

fn write_info(mut file: &File, info: &LockInfo) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(info).map_err(io::Error::other)?;
    bytes.push(b'\n');
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&bytes)?;
    file.sync_data()
}

#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let held = file.metadata()?;
    match std::fs::metadata(path) {
        Ok(named) => Ok(held.dev() == named.dev() && held.ino() == named.ino()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
fn same_file(_file: &File, path: &Path) -> io::Result<bool> {
    // Windows refuses to open a file that is pending deletion, so the race
    // this guards against can't happen there; just check the name exists.
    Ok(path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(host: &str, now_ms: u64) -> Me {
        Me {
            pid: 4242,
            host: host.into(),
            user: Some("tester".into()),
            now_ms,
        }
    }

    fn project(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("Trip.freecut")
    }

    fn plant(project: &Path, host: &str, refreshed_ms: u64) -> LockInfo {
        let info = LockInfo {
            pid: 1,
            host: host.into(),
            user: None,
            app: "freecut 0.0.1".into(),
            started_ms: 0,
            refreshed_ms,
        };
        std::fs::write(
            ProjectLock::lock_path(project),
            serde_json::to_vec(&info).unwrap(),
        )
        .unwrap();
        info
    }

    #[test]
    fn lock_path_appends_lock_to_the_file_name() {
        assert_eq!(
            ProjectLock::lock_path(Path::new("/a/My Trip.freecut")),
            Path::new("/a/My Trip.freecut.lock")
        );
    }

    #[test]
    fn a_crashed_session_on_this_host_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(&dir);
        let old = plant(&p, "here", 1_000_000);
        let lock = acquire_as(&p, &me("here", 1_000_500), Remote::Respect).unwrap();
        assert_eq!(lock.stale(), Some(&StaleLock { holder: Some(old) }));
        assert_eq!(lock.info().pid, 4242);
    }

    #[test]
    fn a_fresh_lock_from_another_host_is_busy_unless_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(&dir);
        let now = 10_000_000;
        let fresh = plant(&p, "laptop", now - 30_000);
        match acquire_as(&p, &me("desktop", now), Remote::Respect) {
            Err(LockError::Busy { holder }) => assert_eq!(holder, Some(fresh.clone())),
            other => panic!("expected Busy, got {other:?}"),
        }
        // A future refresh time (clock skew) also counts as fresh.
        plant(&p, "laptop", now + 60_000);
        assert!(matches!(
            acquire_as(&p, &me("desktop", now), Remote::Respect),
            Err(LockError::Busy { .. })
        ));
        plant(&p, "laptop", now - 30_000);
        let lock = acquire_as(&p, &me("desktop", now), Remote::TakeOver).unwrap();
        assert_eq!(
            lock.stale(),
            Some(&StaleLock {
                holder: Some(fresh)
            })
        );
    }

    #[test]
    fn an_old_lock_from_another_host_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(&dir);
        let now = 10_000_000;
        let old = plant(&p, "laptop", now - duration_ms(STALE_AFTER) - 1);
        let lock = acquire_as(&p, &me("desktop", now), Remote::Respect).unwrap();
        assert_eq!(lock.stale(), Some(&StaleLock { holder: Some(old) }));
    }

    #[test]
    fn refresh_rewrites_the_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let p = project(&dir);
        let mut lock = acquire_as(&p, &me("here", 5), Remote::Respect).unwrap();
        lock.refresh_at(99).unwrap();
        let on_disk: LockInfo =
            serde_json::from_slice(&std::fs::read(lock.path()).unwrap()).unwrap();
        assert_eq!((on_disk.started_ms, on_disk.refreshed_ms), (5, 99));
    }
}
