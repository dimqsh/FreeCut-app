//! The save pipeline (PM-012): validate → canonical bytes → temp file in
//! the same folder → fsync → atomic rename over the target → fsync the
//! folder → read back and verify.
//!
//! The target is never half-written: until the rename, the previous file is
//! untouched and the temp file is removed on every error path. After the
//! rename the target holds the complete new bytes; a later failure (folder
//! sync, verification) is still reported so the app keeps the project dirty
//! and retries. The one-time `.bak` copy is written the same way, so it too
//! is either complete or absent.
//!
//! Saving into another folder rewrites the media items' relative paths for
//! that folder ([`SaveRequest::relative_base`], PM-100).

use core::fmt;
use core::num::NonZeroU64;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use freecut_timeline::edit::Limits;
use freecut_timeline::model::{Document, InvariantViolation, ValidateCtx};

use crate::export::ExportSettings;
use crate::extras::Extras;
use crate::format;
use crate::format::map::{RelativePaths, check_writable};
use crate::fs::{FileSystem, RealFs};
use crate::migrate::CURRENT_FORMAT_VERSION;
use crate::names::sibling_name;
use crate::ui::UiState;

/// What to save.
#[derive(Debug, Clone, Copy)]
pub struct SaveRequest<'a> {
    pub document: &'a Document,
    /// The editor's id allocator position (`Editor::ids().next()`).
    pub ids_next: NonZeroU64,
    /// Unknown keys from the opened file, written back for entities that
    /// still exist.
    pub extras: &'a Extras,
    /// Keep a one-time copy of the file being replaced (after a repaired or
    /// migrated open, PM-013, PM-021; `Opened::backup_on_save` says when).
    /// A file in an older format is kept as `<name>.freecut.v<N>.bak`, `N`
    /// being its format version, so a pre-migration copy never shares its
    /// slot with a repair copy; any other as `<name>.freecut.bak`. An
    /// existing copy that parses is never overwritten: the oldest original
    /// wins. One that doesn't (a fragment left by a crash) is replaced.
    pub backup: bool,
    /// The folder the document's relative media paths are relative to: the
    /// folder of the project file it was opened from or created in. The
    /// document keeps those paths for the whole editing session, so this
    /// stays the same after a Save As, and media imported later should get
    /// their `StoredPath` against this folder too.
    ///
    /// When the target is in another folder (Save As, Save a Copy), each
    /// relative path is rewritten for the target's folder so that it still
    /// names the same file (PM-012, PM-100; [`crate::paths::rebase_relative`]).
    /// `None` writes the relative paths as the document holds them.
    pub relative_base: Option<&'a Path>,
    /// The settings of the last export started from this project (EX-021),
    /// saved with it; `None` before the first export. Pass what
    /// [`crate::Opened::export`] gave until an export starts with new ones.
    pub export: Option<&'a ExportSettings>,
    /// The project's UI state (AU-012's solo set), saved with it but not
    /// part of the document; pass what [`crate::Opened::ui`] gave, as the
    /// app has changed it since. `None` saves none. Ids that are no
    /// sound-carrying track of the document are left out.
    pub ui: Option<&'a UiState>,
}

/// The step of the save pipeline that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStep {
    /// Reading the file about to be replaced, for the backup copy.
    ReadOld,
    WriteBackup,
    WriteTemp,
    /// The atomic rename over the target.
    Replace,
    SyncDir,
    ReadBack,
}

impl fmt::Display for SaveStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SaveStep::ReadOld => "read the current file",
            SaveStep::WriteBackup => "write the backup",
            SaveStep::WriteTemp => "write the temporary file",
            SaveStep::Replace => "replace",
            SaveStep::SyncDir => "flush the folder of",
            SaveStep::ReadBack => "read back",
        })
    }
}

/// Why a save failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SaveError {
    /// The document breaks model invariants. This is a bug upstream; the
    /// file is not touched.
    #[error("refusing to save an invalid document ({} problem(s))", .0.len())]
    InvalidDocument(Vec<InvariantViolation>),
    #[error("cannot serialize the project: {message}")]
    Encode { message: String },
    /// A number is outside what the file holds exactly (an id counter above
    /// `MAX_ID + 1`, ticks beyond ±2^53), so the file would not open again.
    /// `path` is its JSON path. The file is not touched.
    #[error("cannot save the value at {path}: {message}")]
    OutOfRange { path: String, message: String },
    #[error("{} is not a file path", path.display())]
    InvalidPath { path: PathBuf },
    /// An I/O step failed. For steps before [`SaveStep::Replace`] (and
    /// `Replace` itself) the previous file is unchanged.
    #[error("cannot {step} {}: {source}", path.display())]
    Io {
        step: SaveStep,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The file read back after writing differs from what was written
    /// (a broken network share or sync filter).
    #[error("{} doesn't hold what was written", path.display())]
    Verify { path: PathBuf },
}

impl SaveError {
    fn io(step: SaveStep, path: &Path, source: io::Error) -> SaveError {
        SaveError::Io {
            step,
            path: path.to_path_buf(),
            source,
        }
    }
}

/// The canonical file bytes for a document, with its relative media paths
/// as it holds them. Refuses an invalid document, numbers the file can't
/// hold exactly, and bytes that would not open again.
///
/// The file holds no export settings (`"export": null`); see
/// [`encode_with_export`].
pub fn encode(
    document: &Document,
    ids_next: NonZeroU64,
    extras: &Extras,
) -> Result<Vec<u8>, SaveError> {
    encode_with_ui(document, ids_next, None, &UiState::default(), extras)
}

/// [`encode`] with the project's last export settings (EX-021).
pub fn encode_with_export(
    document: &Document,
    ids_next: NonZeroU64,
    export: Option<&ExportSettings>,
    extras: &Extras,
) -> Result<Vec<u8>, SaveError> {
    encode_with_ui(document, ids_next, export, &UiState::default(), extras)
}

/// [`encode_with_export`] with the project's UI state (AU-012).
pub fn encode_with_ui(
    document: &Document,
    ids_next: NonZeroU64,
    export: Option<&ExportSettings>,
    ui: &UiState,
    extras: &Extras,
) -> Result<Vec<u8>, SaveError> {
    encode_with(
        document,
        ids_next,
        export,
        ui,
        extras,
        RelativePaths::AsStored,
    )
}

fn encode_with(
    document: &Document,
    ids_next: NonZeroU64,
    export: Option<&ExportSettings>,
    ui: &UiState,
    extras: &Extras,
    relative: RelativePaths<'_>,
) -> Result<Vec<u8>, SaveError> {
    let ctx = ValidateCtx {
        limits: Limits::default(),
        id_high_water: Some(ids_next),
    };
    document
        .validate(&ctx)
        .map_err(SaveError::InvalidDocument)?;
    check_writable(document, ids_next).map_err(|e| SaveError::OutOfRange {
        path: e.path,
        message: e.message,
    })?;
    let bytes =
        format::to_bytes(document, ids_next, export, ui, extras, relative).map_err(|e| {
            SaveError::Encode {
                message: e.to_string(),
            }
        })?;
    // What is written must open again, unknown keys and all: a file that
    // parses but no longer decodes is never reported as saved.
    crate::open::decode(&bytes).map_err(|e| SaveError::Encode {
        message: format!("the project would not open again: {e}"),
    })?;
    Ok(bytes)
}

/// Saves a project to `path`. Blocking; call it from a worker thread.
pub fn save(path: &Path, request: &SaveRequest<'_>) -> Result<(), SaveError> {
    save_with(&RealFs, path, request)
}

/// [`save`] through an explicit file system.
pub fn save_with(
    fs: &dyn FileSystem,
    path: &Path,
    request: &SaveRequest<'_>,
) -> Result<(), SaveError> {
    let target_dir = real_dir(&folder_of(path));
    let base = request.relative_base.map(real_dir);
    let relative = match &base {
        Some(base) if *base != target_dir => RelativePaths::Rebase {
            from: base,
            to: &target_dir,
        },
        _ => RelativePaths::AsStored,
    };
    let no_ui = UiState::default();
    let bytes = encode_with(
        request.document,
        request.ids_next,
        request.export,
        request.ui.unwrap_or(&no_ui),
        request.extras,
        relative,
    )?;
    write_atomic(fs, path, &bytes, request.backup)?;
    // Verify: what is on disk must be what we meant to write, and parse.
    let back = fs
        .read(path)
        .map_err(|e| SaveError::io(SaveStep::ReadBack, path, e))?;
    if back != bytes || format::parse(&back).is_err() {
        return Err(SaveError::Verify {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// The folder a file path is in (`.` for a bare file name).
fn folder_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The folder `path` with symbolic links resolved, so two spellings of one
/// folder compare equal and relative media paths are computed between real
/// folders; made absolute lexically when it can't be resolved (a base
/// folder that is gone). Read-only: the file system isn't changed.
fn real_dir(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Process-wide counter that makes temp names unique between concurrent
/// saves of one process.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// How many temp names to try before giving up (names left by a crash).
const TEMP_ATTEMPTS: u32 = 16;

/// Replaces `target` with `bytes` atomically: temp file in the same folder,
/// fsync, rename over the target, fsync of the folder. With `backup`, a
/// one-time copy of the old file is kept first (`<name>.v<N>.bak` for an
/// older format, else `<name>.bak`). Does not
/// verify; [`save_with`] does.
pub fn write_atomic(
    fs: &dyn FileSystem,
    target: &Path,
    bytes: &[u8],
    backup: bool,
) -> Result<(), SaveError> {
    let name = target.file_name().ok_or_else(|| SaveError::InvalidPath {
        path: target.to_path_buf(),
    })?;
    let dir = folder_of(target);

    if backup {
        keep_backup(fs, target, &dir, name)?;
    }

    let tmp = write_temp(fs, &dir, name, bytes, SaveStep::WriteTemp)?;
    if let Err(e) = fs.rename(&tmp, target) {
        let _ = fs.remove_file(&tmp);
        return Err(SaveError::io(SaveStep::Replace, target, e));
    }
    fs.sync_dir(&dir)
        .map_err(|e| SaveError::io(SaveStep::SyncDir, target, e))
}

/// The one-time copy of the project file `name` holding `old`: `<name>.v<N>.bak`
/// when `old` is in format version `N`, older than this build's (the
/// pre-migration copy, PM-021), else `<name>.bak`. Shortened when the name
/// is too long for that (see [`sibling_name`]).
fn backup_name(name: &OsStr, old: &[u8]) -> std::ffi::OsString {
    let version = format::parse(old)
        .ok()
        .and_then(|v| v.get("format_version").and_then(serde_json::Value::as_u64));
    match version {
        Some(v) if v < u64::from(CURRENT_FORMAT_VERSION) => {
            sibling_name("", name, &format!(".v{v}.bak"))
        }
        _ => sibling_name("", name, ".bak"),
    }
}

/// Copies the current target to its backup name (`<name>.v<N>.bak` or
/// `<name>.bak`, see [`backup_name`]) unless an intact copy is already
/// there. The copy is written under a temp name and renamed into
/// place, so a crash never leaves a partial `.bak` that would later pass for
/// the original.
fn keep_backup(
    fs: &dyn FileSystem,
    target: &Path,
    dir: &Path,
    name: &OsStr,
) -> Result<(), SaveError> {
    let old = match fs.read(target) {
        Ok(old) => old,
        // Nothing to back up: this is the first save at this path.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(SaveError::io(SaveStep::ReadOld, target, e)),
    };
    let bak_name = backup_name(name, &old);
    let bak = dir.join(&bak_name);
    match fs.read(&bak) {
        // One-time: an earlier original is already kept.
        Ok(kept) if format::parse(&kept).is_ok() => return Ok(()),
        // A fragment (an older build crashed while writing it in place):
        // it is no copy of anything, so replace it.
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(SaveError::io(SaveStep::WriteBackup, &bak, e)),
    }
    let tmp = write_temp(fs, dir, &bak_name, &old, SaveStep::WriteBackup)?;
    if let Err(e) = fs.rename(&tmp, &bak) {
        let _ = fs.remove_file(&tmp);
        return Err(SaveError::io(SaveStep::WriteBackup, &bak, e));
    }
    // The folder sync after the main rename makes this rename durable too.
    Ok(())
}

/// Writes `bytes` to a fresh, hidden temp file `.<name>.<pid>-<n>.tmp` next
/// to `name` (shortened to fit the file-name limit). Failures report `step`.
fn write_temp(
    fs: &dyn FileSystem,
    dir: &Path,
    name: &OsStr,
    bytes: &[u8],
    step: SaveStep,
) -> Result<PathBuf, SaveError> {
    let pid = std::process::id();
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(sibling_name(".", name, &format!(".{pid}-{seq}.tmp")));
        match fs.write_new(&tmp, bytes) {
            Ok(()) => return Ok(tmp),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some((tmp, e)),
            Err(e) => {
                let _ = fs.remove_file(&tmp);
                return Err(SaveError::io(step, &tmp, e));
            }
        }
    }
    let (tmp, e) = last.unwrap_or_else(|| {
        (
            dir.to_path_buf(),
            io::Error::from(io::ErrorKind::AlreadyExists),
        )
    });
    Err(SaveError::io(step, &tmp, e))
}
