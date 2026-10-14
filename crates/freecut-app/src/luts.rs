//! LUTs in the app (CL-016, CL-017, CL-045): FreeCut's bundled set,
//! compiled in from `assets/luts/`, the user's library in app data
//! (`freecut_project::luts`), and [`AppLuts`], the `LutSource` every
//! compositor (preview, posters, export) reads tables through.
//!
//! [`AppLuts`] looks in the bundled set first, so a project that names a
//! bundled LUT never depends on the user's library, then in the library.
//! It parses a file with `freecut_media::lut` the first time a render
//! thread asks for its hash and keeps the table; the UI thread only swaps
//! in a new library index after an import.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use freecut_media::lut::{Lut, parse_3dl, parse_cube};
use freecut_project::luts::{LutEntry, LutFileKind, LutLibrary, hash_lut_bytes};
use freecut_render::color::LutSource;
use freecut_timeline::model::{LutRef, parse_hash_hex};
use serde_json::Value;

/// The bundled files, by their name in `assets/luts/`.
const BUNDLED_FILES: [(&str, &[u8]); 8] = [
    (
        "contrast-neutral-01.cube",
        include_bytes!("../../../assets/luts/contrast-neutral-01.cube"),
    ),
    (
        "cool-gentle-01.cube",
        include_bytes!("../../../assets/luts/cool-gentle-01.cube"),
    ),
    (
        "faded-print-01.cube",
        include_bytes!("../../../assets/luts/faded-print-01.cube"),
    ),
    (
        "mono-neutral-01.cube",
        include_bytes!("../../../assets/luts/mono-neutral-01.cube"),
    ),
    (
        "mono-warm-01.cube",
        include_bytes!("../../../assets/luts/mono-warm-01.cube"),
    ),
    (
        "night-blue-01.cube",
        include_bytes!("../../../assets/luts/night-blue-01.cube"),
    ),
    (
        "split-teal-warm-01.cube",
        include_bytes!("../../../assets/luts/split-teal-warm-01.cube"),
    ),
    (
        "warm-gentle-01.cube",
        include_bytes!("../../../assets/luts/warm-gentle-01.cube"),
    ),
];

const MANIFEST: &str = include_str!("../../../assets/luts/manifest.json");

/// One of FreeCut's bundled LUTs (CL-045).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledLut {
    pub id: String,
    /// "Category · Descriptor 01".
    pub name: String,
    /// The tooltip's one line.
    pub summary: String,
    pub file: String,
    pub hash: [u8; 16],
    pub bytes: &'static [u8],
}

impl BundledLut {
    pub fn lut_ref(&self) -> LutRef {
        LutRef::new(self.hash, self.file.clone())
    }
}

/// The bundled set, in manifest order. Entries the manifest and the
/// compiled-in files don't agree on are left out (a test keeps them in
/// step).
pub fn bundled() -> Vec<BundledLut> {
    let Ok(manifest) = serde_json::from_str::<Value>(MANIFEST) else {
        return Vec::new();
    };
    let Some(luts) = manifest["luts"].as_array() else {
        return Vec::new();
    };
    luts.iter()
        .filter_map(|e| {
            let text = |k: &str| e[k].as_str().map(str::to_owned);
            let file = text("file")?;
            let bytes = BUNDLED_FILES
                .iter()
                .find(|(name, _)| *name == file)
                .map(|(_, b)| *b)?;
            Some(BundledLut {
                id: text("id")?,
                name: text("name")?,
                summary: text("summary").unwrap_or_default(),
                hash: parse_hash_hex(e["hash"].as_str()?)?,
                file,
                bytes,
            })
        })
        .collect()
}

/// Parses LUT file bytes by kind.
pub fn parse(kind: LutFileKind, bytes: &[u8]) -> Result<Lut, freecut_media::lut::LutError> {
    match kind {
        LutFileKind::Cube => parse_cube(bytes),
        LutFileKind::ThreeDl => parse_3dl(bytes),
    }
}

/// What the LUT select offers (CL-016, CL-043): the bundled set, then the
/// library's imported files, and which named files are missing.
#[derive(Debug, Clone)]
pub struct LutChoices {
    pub bundled: Vec<LutChoice>,
    pub imported: Vec<LutChoice>,
    /// Every file the bundled set and the library hold, removed entries
    /// included.
    pub known: BTreeSet<[u8; 16]>,
    /// Files the renderer was asked for and couldn't read (CL-017).
    pub missing: BTreeSet<[u8; 16]>,
    /// The library is still loading at startup: nothing counts as missing
    /// for not being known yet.
    pub loading: bool,
}

impl Default for LutChoices {
    fn default() -> Self {
        LutChoices {
            bundled: Vec::new(),
            imported: Vec::new(),
            known: BTreeSet::new(),
            missing: BTreeSet::new(),
            loading: true,
        }
    }
}

/// One entry of the select.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutChoice {
    pub lut: LutRef,
    pub name: String,
    pub hint: String,
}

impl LutChoices {
    /// The display name of `lut`: its bundled or library name, else its
    /// file name.
    pub fn name_of(&self, lut: &LutRef) -> String {
        self.bundled
            .iter()
            .chain(&self.imported)
            .find(|c| c.lut.hash == lut.hash)
            .map_or_else(|| lut.file_name.clone(), |c| c.name.clone())
    }

    /// Whether `lut`'s file can't be used: neither bundled nor in the
    /// library (once it loaded), or unreadable (the missing badge, CL-017).
    pub fn is_missing(&self, lut: &LutRef) -> bool {
        self.missing.contains(&lut.hash) || (!self.loading && !self.known.contains(&lut.hash))
    }
}

/// The app's LUT source: the bundled set and the library.
#[derive(Debug)]
pub struct AppLuts {
    bundled: Vec<BundledLut>,
    library: RwLock<Option<Arc<LutLibrary>>>,
    tables: Mutex<HashMap<[u8; 16], Arc<Lut>>>,
}

impl Default for AppLuts {
    fn default() -> Self {
        AppLuts::new()
    }
}

impl AppLuts {
    /// The bundled set and no library yet.
    pub fn new() -> AppLuts {
        AppLuts {
            bundled: bundled(),
            library: RwLock::new(None),
            tables: Mutex::new(HashMap::new()),
        }
    }

    /// Uses `library`'s index from now on (after it loaded or after an
    /// import).
    pub fn set_library(&self, library: LutLibrary) {
        if let Ok(mut l) = self.library.write() {
            *l = Some(Arc::new(library));
        }
    }

    /// The library's index, once loaded.
    pub fn library(&self) -> Option<Arc<LutLibrary>> {
        self.library.read().ok().and_then(|l| l.clone())
    }

    /// The select's entries.
    pub fn choices(&self, missing: &BTreeSet<[u8; 16]>) -> LutChoices {
        let library = self.library();
        LutChoices {
            bundled: self
                .bundled
                .iter()
                .map(|b| LutChoice {
                    lut: b.lut_ref(),
                    name: b.name.clone(),
                    hint: b.summary.clone(),
                })
                .collect(),
            imported: library
                .as_ref()
                .map(|lib| {
                    lib.visible()
                        .filter(|e| !self.bundled.iter().any(|b| b.hash == e.hash))
                        .map(|e| LutChoice {
                            lut: LutRef::new(e.hash, e.file_name.clone()),
                            name: e.name.clone(),
                            hint: entry_hint(e),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            known: self
                .bundled
                .iter()
                .map(|b| b.hash)
                .chain(
                    library
                        .iter()
                        .flat_map(|l| l.entries().iter().map(|e| e.hash)),
                )
                .collect(),
            missing: missing.clone(),
            loading: library.is_none(),
        }
    }

    fn load(&self, hash: &[u8; 16]) -> Option<Lut> {
        if let Some(b) = self.bundled.iter().find(|b| b.hash == *hash) {
            return parse_cube(b.bytes).ok();
        }
        let library = self.library()?;
        let entry = library.entry(hash)?;
        let bytes = std::fs::read(library.file_path(entry)).ok()?;
        // A copy that no longer matches its name is not that LUT.
        if hash_lut_bytes(&bytes) != *hash {
            return None;
        }
        parse(entry.kind, &bytes).ok()
    }
}

/// An imported entry's tooltip: its file and table.
fn entry_hint(e: &LutEntry) -> String {
    let table = if e.size > 0 {
        format!("{n}×{n}×{n}", n = e.size)
    } else {
        "1D".to_owned()
    };
    let same = if e.identity {
        " · No visible change"
    } else {
        ""
    };
    format!("Imported from {} · {table}{same}", e.file_name)
}

impl LutSource for AppLuts {
    fn lut(&self, hash: &[u8; 16]) -> Option<Arc<Lut>> {
        if let Some(t) = self.tables.lock().ok()?.get(hash) {
            return Some(Arc::clone(t));
        }
        // Parsed outside the lock; two threads may both parse a file once.
        let table = Arc::new(self.load(hash)?);
        if let Ok(mut tables) = self.tables.lock() {
            tables.insert(*hash, Arc::clone(&table));
        }
        Some(table)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    #[test]
    fn the_bundled_set_matches_its_manifest_and_parses() {
        let set = bundled();
        assert_eq!(set.len(), BUNDLED_FILES.len(), "every file is listed");
        let luts = AppLuts::new();
        for b in &set {
            assert_eq!(hash_lut_bytes(b.bytes), b.hash, "{}", b.file);
            let table = luts.lut(&b.hash).unwrap();
            assert_eq!(table.title.as_deref(), Some(b.name.as_str()));
            assert!(Arc::ptr_eq(&table, &luts.lut(&b.hash).unwrap()), "kept");
            assert!(b.name.contains(" · ") && !b.summary.is_empty());
        }
        assert!(luts.lut(&[0; 16]).is_none());
        let choices = luts.choices(&BTreeSet::new());
        assert_eq!(choices.bundled.len(), 8);
        assert!(choices.imported.is_empty() && choices.loading);
        // Until the library loaded, an unknown file isn't called missing.
        assert!(!choices.is_missing(&LutRef::new([0; 16], "x.cube")));
        assert!(!choices.is_missing(&set[0].lut_ref()));
    }

    #[test]
    fn library_files_are_found_by_hash_after_an_import() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = LutLibrary::in_app_data(dir.path()).unwrap();
        let bytes = b"TITLE \"Invert\"\nLUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";
        let hash = lib
            .add_file(
                bytes,
                LutEntry {
                    hash: [0; 16],
                    name: "Invert".into(),
                    file_name: "invert.cube".into(),
                    kind: LutFileKind::Cube,
                    size: 2,
                    identity: false,
                    imported: 0,
                    hidden: false,
                },
            )
            .unwrap()
            .hash();
        let luts = AppLuts::new();
        assert!(luts.lut(&hash).is_none(), "no library yet");
        luts.set_library(lib);
        let table = luts.lut(&hash).unwrap();
        assert_eq!(table.apply_tetrahedral([0.0; 3]), [1.0; 3]);
        let choices = luts.choices(&BTreeSet::new());
        assert!(!choices.is_missing(&LutRef::new(hash, "x")));
        assert!(choices.is_missing(&LutRef::new([3; 16], "gone.cube")));
        let choices = luts.choices(&BTreeSet::from([hash]));
        assert_eq!(choices.imported.len(), 1);
        assert_eq!(choices.name_of(&LutRef::new(hash, "x")), "Invert");
        assert!(choices.is_missing(&LutRef::new(hash, "x")));
        assert!(choices.imported[0].hint.contains("2×2×2"));
    }
}
