//! Importing LUT files into the library (CL-016, README C54), on a worker
//! thread: each file is read, validated with `freecut_media::lut`, and
//! added through `freecut_project::luts::LutLibrary`, which copies it under
//! its hash and updates the index. Folders are searched (with their
//! subfolders) for `.cube` and `.3dl` files; other files in them are
//! skipped and counted. A broken file never stops the others.

use std::path::{Path, PathBuf};

use freecut_media::lut::{LutNotice, MAX_FILE_BYTES};
use freecut_project::luts::{AddOutcome, LutEntry, LutFileKind, LutLibrary, display_name};

use crate::luts::parse;

/// How deep folders are searched.
const MAX_DEPTH: usize = 8;

/// What an import did, for its summary (CL-016).
#[derive(Debug)]
pub struct ImportReport {
    /// New entries: display name and hash.
    pub added: Vec<(String, [u8; 16])>,
    /// Files whose bytes were in the library already ("Already in
    /// library").
    pub already: Vec<String>,
    /// Files that couldn't be imported: the file and why (with the line
    /// number for a malformed or unsupported LUT).
    pub failed: Vec<(PathBuf, String)>,
    /// Non-LUT files skipped inside dropped folders.
    pub skipped: usize,
    /// What the parsers changed or ignored, one line per file.
    pub notices: Vec<String>,
    /// The library after the import (`None`: it couldn't be opened, and
    /// `failed` says why).
    pub library: Option<LutLibrary>,
}

impl ImportReport {
    /// The status line: what was added, then the files that failed with
    /// their reasons.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let n = self.added.len();
        if n > 0 {
            parts.push(format!("Imported {n} LUT{}", if n == 1 { "" } else { "s" }));
        }
        if !self.already.is_empty() {
            let k = self.already.len();
            parts.push(if k == 1 {
                format!("{} is already in the library", self.already[0])
            } else {
                format!("{k} were already in the library")
            });
        }
        if !self.failed.is_empty() {
            let k = self.failed.len();
            let list: Vec<String> = self
                .failed
                .iter()
                .map(|(path, why)| format!("{} ({why})", file_label(path)))
                .collect();
            parts.push(format!("{k} couldn't be imported: {}", list.join("; ")));
        }
        if self.skipped > 0 {
            parts.push(format!(
                "{} other file{} skipped",
                self.skipped,
                if self.skipped == 1 { "" } else { "s" }
            ));
        }
        if parts.is_empty() {
            return "No LUT files found. FreeCut can import .cube and .3dl LUT files".to_owned();
        }
        parts.join(". ")
    }
}

fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The LUT files in `paths` (folders searched), the files with other
/// extensions named directly, and how many other files folders held.
fn collect(paths: &[PathBuf]) -> (Vec<PathBuf>, Vec<PathBuf>, usize) {
    let (mut luts, mut wrong, mut skipped) = (Vec::new(), Vec::new(), 0);
    fn walk(dir: &Path, depth: usize, luts: &mut Vec<PathBuf>, skipped: &mut usize) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, luts, skipped);
                }
            } else if LutFileKind::of_path(&path).is_some() {
                luts.push(path);
            } else {
                *skipped += 1;
            }
        }
    }
    for path in paths {
        if path.is_dir() {
            walk(path, 0, &mut luts, &mut skipped);
        } else if LutFileKind::of_path(path).is_some() {
            luts.push(path.clone());
        } else {
            wrong.push(path.clone());
        }
    }
    (luts, wrong, skipped)
}

/// Imports `paths` (files and folders) into the library in `library_dir`,
/// stamping new entries with `now` (seconds since the Unix epoch).
pub fn import(paths: &[PathBuf], library_dir: &Path, now: u64) -> ImportReport {
    let (files, wrong, skipped) = collect(paths);
    let mut report = ImportReport {
        added: Vec::new(),
        already: Vec::new(),
        failed: wrong
            .into_iter()
            .map(|p| (p, "FreeCut can import .cube and .3dl LUT files".to_owned()))
            .collect(),
        skipped,
        notices: Vec::new(),
        library: None,
    };
    let mut library = match LutLibrary::open(library_dir) {
        Ok(l) => l,
        Err(e) => {
            report
                .failed
                .extend(files.into_iter().map(|p| (p, e.to_string())));
            return report;
        }
    };
    for path in files {
        match import_one(&path, &mut library, now) {
            Ok((outcome, name, notes)) => {
                match outcome {
                    AddOutcome::Added(hash) => report.added.push((name, hash)),
                    AddOutcome::AlreadyPresent(_) => report.already.push(file_label(&path)),
                }
                if !notes.is_empty() {
                    report
                        .notices
                        .push(format!("{}: {}", file_label(&path), notes.join(", ")));
                }
            }
            Err(why) => report.failed.push((path, why)),
        }
    }
    report.library = Some(library);
    report
}

fn import_one(
    path: &Path,
    library: &mut LutLibrary,
    now: u64,
) -> Result<(AddOutcome, String, Vec<String>), String> {
    let kind = LutFileKind::of_path(path)
        .ok_or_else(|| "FreeCut can import .cube and .3dl LUT files".to_owned())?;
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_BYTES as u64 {
        return Err("the file is larger than 600 MB".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let lut = parse(kind, &bytes).map_err(|e| e.to_string())?;
    let file_name = file_label(path);
    let name = display_name(lut.title.as_deref().unwrap_or(""), &file_name);
    let mut notes: Vec<String> = lut
        .notices
        .iter()
        .map(|n| match n {
            LutNotice::Resampled { from, to } => {
                format!("resampled from {from}³ to {to}³")
            }
            LutNotice::MeshResampled { size } => {
                format!("uneven input mesh resampled onto an even {size}³ lattice")
            }
        })
        .collect();
    if lut.skipped_keywords > 0 {
        notes.push(format!(
            "{} unknown keyword{} ignored",
            lut.skipped_keywords,
            if lut.skipped_keywords == 1 { "" } else { "s" }
        ));
    }
    let identity = lut.is_identity();
    if identity {
        notes.push("no visible change".to_owned());
    }
    let entry = LutEntry {
        hash: [0; 16],
        name: name.clone(),
        file_name,
        kind,
        size: lut.cube.as_ref().map_or(0, |c| c.size()),
        identity,
        imported: now,
        hidden: false,
    };
    let outcome = library.add_file(&bytes, entry).map_err(|e| e.to_string())?;
    Ok((outcome, name, notes))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    /// A valid 2³ cube whose corners are scaled by `k` (so each `k` is a
    /// different file).
    fn cube(k: u32) -> String {
        let v = |b: u32| f64::from(b) * f64::from(k) / 100.0;
        let mut s = format!("TITLE \"Look {k}\"\nLUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    s.push_str(&format!("{} {} {}\n", v(r), v(g), v(b)));
                }
            }
        }
        s
    }

    #[test]
    fn fifty_files_with_three_broken_add_47_and_name_the_lines() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("Looks");
        std::fs::create_dir_all(src.join("more")).unwrap();
        for k in 0..47u32 {
            let folder = if k % 2 == 0 {
                src.clone()
            } else {
                src.join("more")
            };
            std::fs::write(folder.join(format!("look{k:02}.cube")), cube(50 + k)).unwrap();
        }
        // Three broken files: a missing value, a short table, a bad mesh.
        std::fs::write(src.join("broken-a.cube"), "LUT_3D_SIZE 2\n0 0\n").unwrap();
        std::fs::write(
            src.join("broken-b.cube"),
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n",
        )
        .unwrap();
        std::fs::write(src.join("broken-c.3dl"), "0 512 x\n").unwrap();
        std::fs::write(src.join("notes.txt"), "not a LUT").unwrap();
        let app_data = dir.path().join("app");
        let lib_dir = app_data.join(freecut_project::luts::LIBRARY_FOLDER);

        let report = import(std::slice::from_ref(&src), &lib_dir, 1_760_000_000);
        assert_eq!(report.added.len(), 47, "{}", report.summary());
        assert_eq!(report.failed.len(), 3);
        assert_eq!(report.skipped, 1);
        for (path, why) in &report.failed {
            assert!(why.starts_with("line "), "{}: {why}", path.display());
        }
        let summary = report.summary();
        assert!(
            summary.starts_with("Imported 47 LUTs. 3 couldn't be imported: broken-a.cube (line 2"),
            "{summary}"
        );
        assert_eq!(report.library.as_ref().unwrap().entries().len(), 47);
        assert_eq!(report.added[0].0, "Look 50");

        // Importing again keeps one entry per file.
        let again = import(&[src], &lib_dir, 1_760_000_100);
        assert!(again.added.is_empty());
        assert_eq!(again.already.len(), 47);
        assert_eq!(LutLibrary::open(&lib_dir).unwrap().entries().len(), 47);

        // A file of another kind named directly is refused with the hint.
        let other = dir.path().join("grade.dat");
        std::fs::write(&other, "x").unwrap();
        let r = import(&[other], &lib_dir, 0);
        assert_eq!(
            r.summary(),
            "1 couldn't be imported: grade.dat (FreeCut can import .cube and .3dl LUT files)"
        );
    }
}
