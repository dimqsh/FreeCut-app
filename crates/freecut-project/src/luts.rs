//! The user's LUT library (CL-016): copies of imported `.cube` and `.3dl`
//! files, named by the XXH3-128 hash of their bytes, and an index of them,
//! in FreeCut's app data. The library is app state, not project state, so
//! nothing here is on the undo stack.
//!
//! # Layout
//!
//! ```text
//! <app data>/luts/
//!   index.json                                  the entries, in import order
//!   0123456789abcdeffedcba9876543210.cube       one copy per file, by hash
//!   …
//! ```
//!
//! This crate never parses a LUT (`freecut-project` may not depend on
//! `freecut-media`): the app's import job reads a file, validates it with
//! `freecut_media::lut`, then hands the bytes and what it learned to
//! [`LutLibrary::add_file`] (README C54). Everything here is blocking file
//! I/O for a worker thread.
//!
//! "Remove from library" only hides an entry ([`LutLibrary::set_hidden`]):
//! projects keep rendering with the copy (CL-016's edge case). Deleting the
//! copy is a separate, later action.

use std::path::{Path, PathBuf};

use freecut_timeline::model::{hash_hex, parse_hash_hex};
use serde::{Deserialize, Serialize};

use crate::fs::RealFs;
use crate::save::{SaveError, write_atomic};

/// The library's folder inside FreeCut's app data.
pub const LIBRARY_FOLDER: &str = "luts";

/// The index file inside the library folder.
pub const INDEX_FILE: &str = "index.json";

/// The index format this build reads and writes.
pub const INDEX_VERSION: u32 = 1;

/// The longest display name, in characters (CL-016).
pub const MAX_NAME_CHARS: usize = 64;

/// The XXH3-128 hash a LUT file is known by: of its bytes, big-endian, so
/// its hex form reads like the 128-bit number (CL-016's model notes).
pub fn hash_lut_bytes(bytes: &[u8]) -> [u8; 16] {
    xxhash_rust::xxh3::xxh3_128(bytes).to_be_bytes()
}

/// A display name cut to [`MAX_NAME_CHARS`] and trimmed; `fallback` when
/// nothing is left.
pub fn display_name(name: &str, fallback: &str) -> String {
    let cut: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
    let cut = cut.trim();
    if cut.is_empty() {
        fallback.chars().take(MAX_NAME_CHARS).collect()
    } else {
        cut.to_string()
    }
}

/// Which format a library file is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LutFileKind {
    Cube,
    ThreeDl,
}

impl LutFileKind {
    /// The kind of a file by its extension (either case); `None` for
    /// anything but `.cube` and `.3dl` (CL-016).
    pub fn of_path(path: &Path) -> Option<LutFileKind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "cube" => Some(LutFileKind::Cube),
            "3dl" => Some(LutFileKind::ThreeDl),
            _ => None,
        }
    }

    /// The extension of its copies.
    pub const fn extension(self) -> &'static str {
        match self {
            LutFileKind::Cube => "cube",
            LutFileKind::ThreeDl => "3dl",
        }
    }
}

/// One file of the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutEntry {
    /// XXH3-128 of the file's bytes ([`hash_lut_bytes`]).
    pub hash: [u8; 16],
    /// The display name, 1–[`MAX_NAME_CHARS`] characters: the cube's
    /// `TITLE`, else the file name.
    pub name: String,
    /// The file's name when it was imported.
    pub file_name: String,
    pub kind: LutFileKind,
    /// The 3D table's lattice size, 0 for a 1D-only LUT.
    pub size: u32,
    /// Whether the table changes nothing ("No visible change", CL-016).
    pub identity: bool,
    /// When it was imported, in seconds since the Unix epoch.
    pub imported: u64,
    /// Removed from the library (CL-016's Remove only hides the entry).
    pub hidden: bool,
}

/// What [`LutLibrary::add_file`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    /// A new entry with this hash.
    Added([u8; 16]),
    /// The same bytes were in the library already ("Already in library");
    /// a hidden entry is shown again.
    AlreadyPresent([u8; 16]),
}

impl AddOutcome {
    pub fn hash(self) -> [u8; 16] {
        match self {
            AddOutcome::Added(h) | AddOutcome::AlreadyPresent(h) => h,
        }
    }
}

/// Why the library couldn't be read or changed.
#[derive(Debug, thiserror::Error)]
pub enum LutLibraryError {
    #[error("couldn't read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the LUT index {path} is damaged: {message}")]
    Index { path: PathBuf, message: String },
    #[error("couldn't write {path}: {source}")]
    Save {
        path: PathBuf,
        #[source]
        source: SaveError,
    },
    #[error("no LUT in the library has the hash {0}")]
    Unknown(String),
}

#[derive(Serialize, Deserialize)]
struct IndexDto {
    version: u32,
    luts: Vec<EntryDto>,
}

#[derive(Serialize, Deserialize)]
struct EntryDto {
    hash: String,
    name: String,
    file: String,
    kind: String,
    size: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    identity: bool,
    imported: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

impl EntryDto {
    fn of(e: &LutEntry) -> EntryDto {
        EntryDto {
            hash: hash_hex(&e.hash),
            name: e.name.clone(),
            file: e.file_name.clone(),
            kind: e.kind.extension().to_string(),
            size: e.size,
            identity: e.identity,
            imported: e.imported,
            hidden: e.hidden,
        }
    }

    fn entry(self) -> Result<LutEntry, String> {
        let hash = parse_hash_hex(&self.hash).ok_or_else(|| format!("bad hash `{}`", self.hash))?;
        let kind = match self.kind.as_str() {
            "cube" => LutFileKind::Cube,
            "3dl" => LutFileKind::ThreeDl,
            other => return Err(format!("unknown kind `{other}`")),
        };
        Ok(LutEntry {
            hash,
            name: display_name(&self.name, &self.file),
            file_name: self.file,
            kind,
            size: self.size,
            identity: self.identity,
            imported: self.imported,
            hidden: self.hidden,
        })
    }
}

/// The LUT library in one folder: its entries and their copies.
#[derive(Debug, Clone)]
pub struct LutLibrary {
    dir: PathBuf,
    entries: Vec<LutEntry>,
}

impl LutLibrary {
    /// The library in `<app_data>/luts`.
    pub fn in_app_data(app_data: &Path) -> Result<LutLibrary, LutLibraryError> {
        LutLibrary::open(&app_data.join(LIBRARY_FOLDER))
    }

    /// Reads the library in `dir`; a folder or index that doesn't exist yet
    /// is an empty library (nothing is created until the first import).
    pub fn open(dir: &Path) -> Result<LutLibrary, LutLibraryError> {
        let index = dir.join(INDEX_FILE);
        let bytes = match std::fs::read(&index) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LutLibrary {
                    dir: dir.to_path_buf(),
                    entries: Vec::new(),
                });
            }
            Err(source) => {
                return Err(LutLibraryError::Io {
                    path: index,
                    source,
                });
            }
        };
        let bad = |message: String| LutLibraryError::Index {
            path: index.clone(),
            message,
        };
        let dto: IndexDto = serde_json::from_slice(&bytes).map_err(|e| bad(e.to_string()))?;
        if dto.version > INDEX_VERSION {
            return Err(bad(format!(
                "version {} is newer than this build reads ({INDEX_VERSION})",
                dto.version
            )));
        }
        let mut entries: Vec<LutEntry> = Vec::with_capacity(dto.luts.len());
        for e in dto.luts {
            let e = e.entry().map_err(bad)?;
            if !entries.iter().any(|x| x.hash == e.hash) {
                entries.push(e);
            }
        }
        Ok(LutLibrary {
            dir: dir.to_path_buf(),
            entries,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every entry, hidden ones included, in import order.
    pub fn entries(&self) -> &[LutEntry] {
        &self.entries
    }

    /// The entries the library shows (not removed).
    pub fn visible(&self) -> impl Iterator<Item = &LutEntry> {
        self.entries.iter().filter(|e| !e.hidden)
    }

    pub fn entry(&self, hash: &[u8; 16]) -> Option<&LutEntry> {
        self.entries.iter().find(|e| e.hash == *hash)
    }

    /// Where the copy of `entry` lives.
    pub fn file_path(&self, entry: &LutEntry) -> PathBuf {
        self.dir.join(format!(
            "{}.{}",
            hash_hex(&entry.hash),
            entry.kind.extension()
        ))
    }

    /// The copy of the file with `hash`, when the library has it.
    pub fn path_of(&self, hash: &[u8; 16]) -> Option<PathBuf> {
        self.entry(hash).map(|e| self.file_path(e))
    }

    /// Adds a validated file: its copy (written once, atomically) and its
    /// entry, then the index. `entry.hash` is replaced by the hash of
    /// `bytes`. A file already in the library keeps its entry (shown again
    /// if it was removed) and adds nothing.
    pub fn add_file(
        &mut self,
        bytes: &[u8],
        mut entry: LutEntry,
    ) -> Result<AddOutcome, LutLibraryError> {
        let hash = hash_lut_bytes(bytes);
        entry.hash = hash;
        entry.name = display_name(&entry.name, &entry.file_name);
        if let Some(i) = self.entries.iter().position(|e| e.hash == hash) {
            let copy = self.file_path(&self.entries[i]);
            if !copy.is_file() {
                self.write_copy(&copy, bytes)?;
            }
            if self.entries[i].hidden {
                self.entries[i].hidden = false;
                self.save_index()?;
            }
            return Ok(AddOutcome::AlreadyPresent(hash));
        }
        std::fs::create_dir_all(&self.dir).map_err(|source| LutLibraryError::Io {
            path: self.dir.clone(),
            source,
        })?;
        let copy = self.file_path(&entry);
        self.write_copy(&copy, bytes)?;
        self.entries.push(entry);
        self.save_index()?;
        Ok(AddOutcome::Added(hash))
    }

    /// Gives the entry with `hash` a new display name (1–64 characters;
    /// a blank name falls back to the file name).
    pub fn rename(&mut self, hash: &[u8; 16], name: &str) -> Result<(), LutLibraryError> {
        let e = self.entry_mut(hash)?;
        e.name = display_name(name, &e.file_name);
        self.save_index()
    }

    /// Removes the entry with `hash` from the library's lists (or shows it
    /// again); its copy stays, so projects that use it keep rendering.
    pub fn set_hidden(&mut self, hash: &[u8; 16], hidden: bool) -> Result<(), LutLibraryError> {
        self.entry_mut(hash)?.hidden = hidden;
        self.save_index()
    }

    fn entry_mut(&mut self, hash: &[u8; 16]) -> Result<&mut LutEntry, LutLibraryError> {
        self.entries
            .iter_mut()
            .find(|e| e.hash == *hash)
            .ok_or_else(|| LutLibraryError::Unknown(hash_hex(hash)))
    }

    fn write_copy(&self, path: &Path, bytes: &[u8]) -> Result<(), LutLibraryError> {
        write_atomic(&RealFs, path, bytes, false).map_err(|source| LutLibraryError::Save {
            path: path.to_path_buf(),
            source,
        })
    }

    fn save_index(&self) -> Result<(), LutLibraryError> {
        let dto = IndexDto {
            version: INDEX_VERSION,
            luts: self.entries.iter().map(EntryDto::of).collect(),
        };
        let path = self.dir.join(INDEX_FILE);
        let mut bytes = serde_json::to_vec_pretty(&dto).map_err(|e| LutLibraryError::Index {
            path: path.clone(),
            message: e.to_string(),
        })?;
        bytes.push(b'\n');
        std::fs::create_dir_all(&self.dir).map_err(|source| LutLibraryError::Io {
            path: self.dir.clone(),
            source,
        })?;
        self.write_copy(&path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, file: &str) -> LutEntry {
        LutEntry {
            hash: [0; 16],
            name: name.into(),
            file_name: file.into(),
            kind: LutFileKind::Cube,
            size: 2,
            identity: false,
            imported: 1_760_000_000,
            hidden: false,
        }
    }

    #[test]
    fn importing_the_same_bytes_twice_keeps_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = LutLibrary::in_app_data(dir.path()).unwrap();
        assert!(lib.entries().is_empty());
        let bytes = b"LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let first = lib.add_file(bytes, entry("Teal", "teal.cube")).unwrap();
        assert_eq!(first, AddOutcome::Added(hash_lut_bytes(bytes)));
        let again = lib.add_file(bytes, entry("Other", "copy.cube")).unwrap();
        assert_eq!(again, AddOutcome::AlreadyPresent(first.hash()));
        assert_eq!(lib.entries().len(), 1);
        let copy = lib.path_of(&first.hash()).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), bytes);
        assert_eq!(
            copy.file_name().unwrap().to_str().unwrap(),
            format!("{}.cube", hash_hex(&first.hash()))
        );

        // Rename and remove persist; re-importing a removed file shows it.
        lib.rename(&first.hash(), "  Teal night  ").unwrap();
        lib.set_hidden(&first.hash(), true).unwrap();
        let reread = LutLibrary::in_app_data(dir.path()).unwrap();
        assert_eq!(reread.entries()[0].name, "Teal night");
        assert!(reread.entries()[0].hidden && reread.visible().count() == 0);
        let mut lib = reread;
        lib.add_file(bytes, entry("x", "x.cube")).unwrap();
        assert_eq!(lib.visible().count(), 1);
        assert!(copy.is_file(), "removing never deletes the copy");
        assert!(matches!(
            lib.rename(&[9; 16], "x"),
            Err(LutLibraryError::Unknown(_))
        ));
    }

    #[test]
    fn names_are_limited_and_kinds_come_from_the_extension() {
        assert_eq!(display_name(&"é".repeat(80), "f.cube").chars().count(), 64);
        assert_eq!(display_name("   ", "file.3dl"), "file.3dl");
        assert_eq!(
            LutFileKind::of_path(Path::new("a/B.CUBE")),
            Some(LutFileKind::Cube)
        );
        assert_eq!(
            LutFileKind::of_path(Path::new("x.3dl")),
            Some(LutFileKind::ThreeDl)
        );
        assert_eq!(LutFileKind::of_path(Path::new("x.dat")), None);
        assert_eq!(LutFileKind::of_path(Path::new("cube")), None);
    }

    #[test]
    fn a_damaged_or_newer_index_is_an_error_not_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib_dir = dir.path().join(LIBRARY_FOLDER);
        std::fs::create_dir_all(&lib_dir).unwrap();
        std::fs::write(lib_dir.join(INDEX_FILE), b"{not json").unwrap();
        assert!(matches!(
            LutLibrary::open(&lib_dir),
            Err(LutLibraryError::Index { .. })
        ));
        std::fs::write(lib_dir.join(INDEX_FILE), br#"{"version": 9, "luts": []}"#).unwrap();
        assert!(matches!(
            LutLibrary::open(&lib_dir),
            Err(LutLibraryError::Index { .. })
        ));
    }
}
