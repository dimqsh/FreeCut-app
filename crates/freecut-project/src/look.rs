//! Saved looks (CL-026): `.fclook` files and the library of them in
//! FreeCut's app data. A look is the values of some colour sections taken
//! at the playhead: Light, Color, LUT, Filter, HSL, Curves and Wheels
//! ([`Section::LOOK`]). It never holds Auto, Match, masks, footage repair
//! (cleanup), the overall Amount or keyframes. Applying one copies its
//! sections onto clips (`ColorEdit::ApplySections`), so deleting a look
//! never changes a project.
//!
//! # File (look format 1)
//!
//! ```json
//! {
//!   "format": "freecut-look",
//!   "look_version": 1,
//!   "name": "Warm evening",
//!   "tags": [],
//!   "created": 1760000000,
//!   "sections": ["light", "color", "lut"],
//!   "color": { "light": { "exposure": 20 }, "lut": { "hash": "…", "file": "teal.cube" } },
//!   "luts": [{ "hash": "…", "file": "teal.cube" }]
//! }
//! ```
//!
//! `color` uses the project format's colour object (project-format.md),
//! holding only the listed sections. `luts` repeats the LUT files the look
//! names, for readers that don't parse the colour object; it is written
//! and not read back. A file from a newer FreeCut is refused with its
//! version ([`LookError::Newer`]) and listed as unavailable, never deleted.
//!
//! # Library layout
//!
//! ```text
//! <app data>/looks/
//!   3f2a…e1.fclook      one file per look, named by a random id
//! ```
//!
//! The file name is never the display name, so a name can hold any
//! Unicode. Everything here is blocking file I/O for a worker thread.

use std::path::{Path, PathBuf};

use freecut_timeline::model::{
    ColorStack, LutRef, Section, SectionMask, SectionSnapshot, hash_hex,
};
use serde_json::{Map, Value};

use crate::fs::RealFs;
use crate::save::{SaveError, write_atomic};

/// The look format this build reads and writes.
pub const LOOK_VERSION: u32 = 1;

/// The value of a look file's `format` key.
pub const LOOK_FORMAT: &str = "freecut-look";

/// The extension of look files.
pub const LOOK_EXTENSION: &str = "fclook";

/// The library's folder inside FreeCut's app data.
pub const LOOKS_FOLDER: &str = "looks";

/// The longest look name, in characters (CL-026).
pub const MAX_LOOK_NAME_CHARS: usize = 64;

/// Look files larger than this are not read (a look is a few kB).
pub const MAX_LOOK_BYTES: u64 = 1 << 20;

/// A saved look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookFile {
    /// 1–[`MAX_LOOK_NAME_CHARS`] characters, any Unicode.
    pub name: String,
    pub tags: Vec<String>,
    /// When it was saved, in seconds since the Unix epoch.
    pub created: u64,
    /// The sections it holds and their values; only [`Section::LOOK`]
    /// sections.
    pub snapshot: SectionSnapshot,
}

impl LookFile {
    /// A look named `name` holding `sections` of `stack` (the values the
    /// clip shows at the playhead). Sections a look can't hold are left
    /// out, and the name is trimmed and cut to [`MAX_LOOK_NAME_CHARS`].
    pub fn new(name: &str, stack: &ColorStack, sections: SectionMask, created: u64) -> LookFile {
        LookFile {
            name: look_name(name),
            tags: Vec::new(),
            created,
            snapshot: SectionSnapshot::new(stack, look_sections(sections)),
        }
    }

    /// The LUT files the look names (CL-026's LUT references).
    pub fn luts(&self) -> Vec<LutRef> {
        let values = self.snapshot.values();
        match (&values.lut, self.snapshot.sections().is_on(Section::Lut)) {
            (Some(slot), true) => vec![slot.lut.clone()],
            _ => Vec::new(),
        }
    }
}

/// Only the sections a look can hold.
fn look_sections(sections: SectionMask) -> SectionMask {
    SectionMask::of(
        &sections
            .sections()
            .filter(|s| s.in_look())
            .collect::<Vec<_>>(),
    )
}

/// `name` trimmed and cut to [`MAX_LOOK_NAME_CHARS`]; "Look" when nothing
/// is left.
pub fn look_name(name: &str) -> String {
    let cut: String = name.trim().chars().take(MAX_LOOK_NAME_CHARS).collect();
    let cut = cut.trim();
    if cut.is_empty() {
        "Look".to_owned()
    } else {
        cut.to_owned()
    }
}

/// Why a look file can't be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LookError {
    #[error("not JSON: {0}")]
    Json(String),
    #[error("not a FreeCut look")]
    NotALook,
    /// Made by a newer FreeCut (CL-026: "Made with a newer FreeCut (look
    /// format N)").
    #[error("made with a newer FreeCut (look format {0})")]
    Newer(u64),
    #[error("{path}: {message}")]
    BadValue { path: String, message: String },
    #[error("larger than a look can be")]
    TooLarge,
    #[error("can't read the file: {0}")]
    Io(String),
}

fn bad(path: &str, message: impl Into<String>) -> LookError {
    LookError::BadValue {
        path: path.to_owned(),
        message: message.into(),
    }
}

/// Reads a look file. Values outside their ranges are clamped, as a
/// project's are; a section this build doesn't know, a list that is no
/// curve or a bad LUT hash refuse the file.
pub fn read_look(bytes: &[u8]) -> Result<LookFile, LookError> {
    if bytes.len() as u64 > MAX_LOOK_BYTES {
        return Err(LookError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| LookError::Json(e.to_string()))?;
    let Value::Object(mut root) = value else {
        return Err(LookError::NotALook);
    };
    if root.get("format").and_then(Value::as_str) != Some(LOOK_FORMAT) {
        return Err(LookError::NotALook);
    }
    let version = root
        .get("look_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad("look_version", "a look format number is needed"))?;
    if version > u64::from(LOOK_VERSION) {
        return Err(LookError::Newer(version));
    }
    if version == 0 {
        return Err(bad("look_version", "look formats start at 1"));
    }
    let name = match root.get("name") {
        Some(Value::String(s)) => look_name(s),
        _ => return Err(bad("name", "a look needs a name")),
    };
    let tags = match root.get("tags") {
        None => Vec::new(),
        Some(Value::Array(tags)) => tags
            .iter()
            .enumerate()
            .map(|(i, t)| {
                t.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad(&format!("tags[{i}]"), "a tag is text"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(bad("tags", "a list of text")),
    };
    let created = match root.get("created") {
        None => 0,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| bad("created", "seconds since 1970"))?,
    };
    let mut sections = SectionMask::NONE;
    match root.get("sections") {
        Some(Value::Array(list)) => {
            for (i, s) in list.iter().enumerate() {
                let section = s
                    .as_str()
                    .and_then(Section::parse)
                    .ok_or_else(|| bad(&format!("sections[{i}]"), "not a colour section"))?;
                sections = sections.with(section, true);
            }
        }
        _ => return Err(bad("sections", "a list of colour sections")),
    }
    let color = root
        .remove("color")
        .unwrap_or_else(|| Value::Object(Map::new()));
    let (stack, _clamped) = crate::format::map::read_stack(color, "color").map_err(|e| {
        let text = e.to_string();
        bad("color", text)
    })?;
    Ok(LookFile {
        name,
        tags,
        created,
        snapshot: SectionSnapshot::new(&stack, look_sections(sections)),
    })
}

/// The bytes of `look` as a look file (format [`LOOK_VERSION`]), pretty
/// printed with a final newline.
pub fn write_look(look: &LookFile) -> Vec<u8> {
    let mut root = Map::new();
    root.insert("format".into(), Value::from(LOOK_FORMAT));
    root.insert("look_version".into(), Value::from(LOOK_VERSION));
    root.insert("name".into(), Value::from(look.name.clone()));
    root.insert(
        "tags".into(),
        Value::Array(look.tags.iter().cloned().map(Value::from).collect()),
    );
    root.insert("created".into(), Value::from(look.created));
    root.insert(
        "sections".into(),
        Value::Array(
            look.snapshot
                .sections()
                .sections()
                .map(|s| Value::from(s.as_str()))
                .collect(),
        ),
    );
    root.insert(
        "color".into(),
        crate::format::map::write_stack(look.snapshot.values()),
    );
    root.insert(
        "luts".into(),
        Value::Array(
            look.luts()
                .iter()
                .map(|l| {
                    let mut m = Map::new();
                    m.insert("hash".into(), Value::from(hash_hex(&l.hash)));
                    m.insert("file".into(), Value::from(l.file_name.clone()));
                    Value::Object(m)
                })
                .collect(),
        ),
    );
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(root)).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// One file of the library: its look, or why it can't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookEntry {
    /// The file's stem, the look's id in the library.
    pub id: String,
    pub path: PathBuf,
    pub look: Result<LookFile, LookError>,
}

impl LookEntry {
    /// The name to list it under: the look's, or the file's.
    pub fn display_name(&self) -> &str {
        match &self.look {
            Ok(look) => &look.name,
            Err(_) => &self.id,
        }
    }
}

/// Why the library couldn't be read or changed.
#[derive(Debug, thiserror::Error)]
pub enum LookLibraryError {
    #[error("can't read the looks folder {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("can't save the look: {0}")]
    Write(#[from] SaveError),
    #[error("can't remove the look: {0}")]
    Remove(std::io::Error),
    #[error("no look `{0}` in the library")]
    Unknown(String),
}

/// The user's saved looks (CL-026): every `.fclook` file of one folder,
/// oldest first. Unreadable files are listed with their reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookLibrary {
    dir: PathBuf,
    entries: Vec<LookEntry>,
}

impl LookLibrary {
    /// The library in `<app_data>/looks`.
    pub fn in_app_data(app_data: &Path) -> Result<LookLibrary, LookLibraryError> {
        LookLibrary::open(&app_data.join(LOOKS_FOLDER))
    }

    /// Reads every look file in `dir`; a folder that doesn't exist yet is
    /// an empty library.
    pub fn open(dir: &Path) -> Result<LookLibrary, LookLibraryError> {
        let mut entries = Vec::new();
        let read = match std::fs::read_dir(dir) {
            Ok(r) => Some(r),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(LookLibraryError::Read {
                    path: dir.to_path_buf(),
                    source,
                });
            }
        };
        for item in read.into_iter().flatten().flatten() {
            let path = item.path();
            let is_look = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(LOOK_EXTENSION));
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_look || !path.is_file() {
                continue;
            }
            entries.push(LookEntry {
                id: id.to_owned(),
                look: read_file(&path),
                path,
            });
        }
        let mut library = LookLibrary {
            dir: dir.to_path_buf(),
            entries,
        };
        library.sort();
        Ok(library)
    }

    fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            let created = |e: &LookEntry| e.look.as_ref().map_or(u64::MAX, |l| l.created);
            created(a)
                .cmp(&created(b))
                .then_with(|| a.display_name().cmp(b.display_name()))
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every file, oldest look first, unusable ones last.
    pub fn entries(&self) -> &[LookEntry] {
        &self.entries
    }

    /// The usable looks, oldest first.
    pub fn looks(&self) -> impl Iterator<Item = (&str, &LookFile)> {
        self.entries
            .iter()
            .filter_map(|e| e.look.as_ref().ok().map(|l| (e.id.as_str(), l)))
    }

    /// The entry `id`.
    pub fn entry(&self, id: &str) -> Option<&LookEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// `wanted` (trimmed and cut), or with " (2)", " (3)", … when a look
    /// already has that name (CL-026: a duplicate gets a suffix).
    pub fn unique_name(&self, wanted: &str) -> String {
        let base = look_name(wanted);
        let taken = |n: &str| self.looks().any(|(_, l)| l.name == n);
        if !taken(&base) {
            return base;
        }
        (2u32..)
            .map(|k| {
                let suffix = format!(" ({k})");
                let room = MAX_LOOK_NAME_CHARS.saturating_sub(suffix.chars().count());
                let head: String = base.chars().take(room).collect();
                format!("{}{suffix}", head.trim_end())
            })
            .find(|n| !taken(n))
            .unwrap_or(base)
    }

    /// Saves `look` as a new file (its name made unique first) and returns
    /// its id. `seed` varies the random id (the caller's clock); a taken id
    /// is never overwritten.
    pub fn save(&mut self, mut look: LookFile, seed: u64) -> Result<String, LookLibraryError> {
        look.name = self.unique_name(&look.name);
        let bytes = write_look(&look);
        std::fs::create_dir_all(&self.dir).map_err(|source| LookLibraryError::Read {
            path: self.dir.clone(),
            source,
        })?;
        let mut salt = seed;
        let (id, path) = loop {
            let mut input = bytes.clone();
            input.extend_from_slice(&salt.to_le_bytes());
            let id = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&input));
            let path = self.dir.join(format!("{id}.{LOOK_EXTENSION}"));
            if !path.exists() && self.entry(&id).is_none() {
                break (id, path);
            }
            salt = salt.wrapping_add(1);
        };
        write_atomic(&RealFs, &path, &bytes, false)?;
        self.entries.push(LookEntry {
            id: id.clone(),
            path,
            look: Ok(look),
        });
        self.sort();
        Ok(id)
    }

    /// Deletes look `id`'s file. Projects that used it keep their values
    /// (they were copied, CL-026). FreeCut has no trash of its own, so this
    /// is permanent and the UI says so before calling it.
    pub fn delete(&mut self, id: &str) -> Result<(), LookLibraryError> {
        let i = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| LookLibraryError::Unknown(id.to_owned()))?;
        match std::fs::remove_file(&self.entries[i].path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(LookLibraryError::Remove(e)),
        }
        self.entries.remove(i);
        Ok(())
    }
}

/// One look file, read and parsed.
fn read_file(path: &Path) -> Result<LookFile, LookError> {
    let size = std::fs::metadata(path)
        .map_err(|e| LookError::Io(e.to_string()))?
        .len();
    if size > MAX_LOOK_BYTES {
        return Err(LookError::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|e| LookError::Io(e.to_string()))?;
    read_look(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecut_timeline::model::{
        ColorParamId, Curve, CurveChannel, CurvePoint, HslChannel, HueBand, LutSlot, WheelId,
    };

    fn graded() -> ColorStack {
        let curve = Curve::new(vec![
            CurvePoint::new(0, 500),
            CurvePoint::new(5_000, 6_000),
            CurvePoint::new(10_000, 9_500),
        ])
        .unwrap();
        ColorStack::default()
            .with(ColorParamId::Exposure, 20)
            .with(ColorParamId::Temperature, -35)
            .with(
                ColorParamId::Hsl {
                    band: HueBand::Blue,
                    channel: HslChannel::Saturation,
                },
                40,
            )
            .with_curve(CurveChannel::Luma, curve)
            .with_puck(WheelId::Highlights, 200, 150)
            .with_lut(Some(LutSlot::new(LutRef::new([0xab; 16], "teal.cube"))))
            .with(ColorParamId::LutIntensity, 60)
            .with(ColorParamId::Amount, 30)
    }

    #[test]
    fn a_look_round_trips_and_leaves_out_what_belongs_to_one_shot() {
        let all = SectionMask::ALL_ON;
        let look = LookFile::new("  Warm evening ", &graded(), all, 1_760_000_000);
        assert_eq!(look.name, "Warm evening");
        assert!(!look.snapshot.sections().is_on(Section::Masks));
        assert!(!look.snapshot.sections().is_on(Section::Auto));
        assert!(!look.snapshot.sections().is_on(Section::Match));
        assert!(!look.snapshot.sections().is_on(Section::Detail));
        assert_eq!(look.snapshot.values().amount, 100, "Amount is not saved");
        let bytes = write_look(&look);
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("\"look_version\": 1"));
        assert!(!text.contains("amount\""), "{text}");
        assert!(text.contains("teal.cube"));
        let back = read_look(&bytes).unwrap();
        assert_eq!(back, look);
        assert_eq!(back.luts(), vec![LutRef::new([0xab; 16], "teal.cube")]);
        assert_eq!(write_look(&back), bytes, "canonical bytes");
        // Only some sections: the others read back at their defaults.
        let light = LookFile::new("Light", &graded(), SectionMask::of(&[Section::Light]), 0);
        let back = read_look(&write_look(&light)).unwrap();
        assert_eq!(back.snapshot.values().basic.exposure, 20);
        assert_eq!(back.snapshot.values().basic.temperature, 0);
        assert!(back.luts().is_empty());
    }

    #[test]
    fn newer_and_broken_files_are_refused_with_a_reason() {
        let look = LookFile::new("A", &graded(), SectionMask::of(&Section::LOOK), 0);
        let mut v: Value = serde_json::from_slice(&write_look(&look)).unwrap();
        v["look_version"] = Value::from(2);
        let newer = serde_json::to_vec(&v).unwrap();
        assert_eq!(read_look(&newer), Err(LookError::Newer(2)));
        assert_eq!(
            LookError::Newer(2).to_string(),
            "made with a newer FreeCut (look format 2)"
        );
        assert!(matches!(read_look(b"{"), Err(LookError::Json(_))));
        assert_eq!(read_look(b"{\"name\":\"x\"}"), Err(LookError::NotALook));
        v["look_version"] = Value::from(1);
        v["sections"] = serde_json::json!(["light", "sparkle"]);
        assert!(matches!(
            read_look(&serde_json::to_vec(&v).unwrap()),
            Err(LookError::BadValue { path, .. }) if path == "sections[1]"
        ));
        v["sections"] = serde_json::json!(["curves"]);
        v["color"] = serde_json::json!({ "curves": { "luma": [[0, 0]] } });
        assert!(matches!(
            read_look(&serde_json::to_vec(&v).unwrap()),
            Err(LookError::BadValue { path, .. }) if path == "color"
        ));
        // Out of range values are clamped, as in projects.
        v["sections"] = serde_json::json!(["light", "masks"]);
        v["color"] = serde_json::json!({ "light": { "exposure": 900 } });
        let clamped = read_look(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(clamped.snapshot.values().basic.exposure, 100);
        assert!(
            !clamped.snapshot.sections().is_on(Section::Masks),
            "a look never holds masks"
        );
    }

    #[test]
    fn the_library_saves_lists_dedupes_names_and_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(LOOKS_FOLDER);
        let mut lib = LookLibrary::open(&dir).unwrap();
        assert!(lib.entries().is_empty(), "no folder yet: empty");
        let a = lib
            .save(LookFile::new("Teal", &graded(), SectionMask::ALL_ON, 10), 1)
            .unwrap();
        let b = lib
            .save(LookFile::new("Teal", &graded(), SectionMask::ALL_ON, 20), 1)
            .unwrap();
        assert_ne!(a, b, "the same look twice gets two files");
        let names: Vec<&str> = lib.looks().map(|(_, l)| l.name.as_str()).collect();
        assert_eq!(names, ["Teal", "Teal (2)"]);
        // A newer file and a broken one are listed, not deleted.
        std::fs::write(
            dir.join("newer.fclook"),
            br#"{"format":"freecut-look","look_version":9,"name":"Future","sections":[]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a look").unwrap();
        let reopened = LookLibrary::open(&dir).unwrap();
        assert_eq!(reopened.entries().len(), 3);
        assert_eq!(reopened.looks().count(), 2);
        let newer = reopened.entry("newer").unwrap();
        assert_eq!(newer.look, Err(LookError::Newer(9)));
        assert_eq!(reopened.entries().last().unwrap().id, "newer");
        assert_eq!(
            reopened.entry(&a).unwrap().look,
            lib.entry(&a).unwrap().look
        );
        lib.delete(&a).unwrap();
        assert!(lib.entry(&a).is_none());
        assert!(!dir.join(format!("{a}.fclook")).exists());
        assert!(matches!(lib.delete(&a), Err(LookLibraryError::Unknown(_))));
        let long = "x".repeat(80);
        assert_eq!(lib.unique_name(&long).chars().count(), MAX_LOOK_NAME_CHARS);
        assert_eq!(lib.unique_name("   "), "Look");
    }
}
