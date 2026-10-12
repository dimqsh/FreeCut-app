//! The file-system operations this crate uses, behind a small trait so tests
//! can run the save pipeline against injected failures and the path
//! resolver against a virtual file system.

use std::io::{self, Write};
use std::path::Path;

/// File-system access for saving and path resolution.
///
/// Implementations must be usable from worker threads.
pub trait FileSystem: Send + Sync {
    /// Whether `path` names an existing regular file (following links).
    fn is_file(&self, path: &Path) -> bool;

    /// The whole content of `path`.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Creates `path`, which must not exist yet (`AlreadyExists` otherwise),
    /// writes `bytes` and flushes them to stable storage. On error, a
    /// partially written file may remain; the caller removes it.
    fn write_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Renames `from` to `to`, atomically replacing `to` if it exists. Both
    /// are in the same directory.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Makes a completed rename in `dir` durable (fsync of the directory on
    /// Unix; a no-op where the platform has no such operation).
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
}

/// The real file system, through `std::fs`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl FileSystem for RealFs {
    fn is_file(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.is_file())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)?;
        // std uses F_FULLFSYNC on Apple platforms, so this reaches the disk.
        file.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    #[cfg(unix)]
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        std::fs::File::open(dir)?.sync_all()
    }

    #[cfg(not(unix))]
    fn sync_dir(&self, _dir: &Path) -> io::Result<()> {
        // Windows has no directory fsync; the rename is already
        // metadata-journaled by NTFS.
        Ok(())
    }
}
