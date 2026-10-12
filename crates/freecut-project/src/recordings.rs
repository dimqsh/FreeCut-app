//! Where voiceover takes are written, and how a take interrupted by a crash
//! is found again (AU-027, PM-041, PM-043).
//!
//! # Layout
//!
//! Recordings are part of the project's generated-media store (PM-041):
//!
//! ```text
//! My Trip.freecut
//! My Trip Media/
//!   Recordings/
//!     Voiceover 2026-10-12 14.32.05.wav
//!     Voiceover 2026-10-12 14.32.05.wav.recording   only while the take is written
//! ```
//!
//! A project folder that can't be written, and a project that was never
//! saved, use [`fallback_recordings_folder`] in app data instead (PM-041's
//! portability warning is the app's).
//!
//! # Interrupted takes
//!
//! The recorder (`freecut_audio::record`) creates `<take>.recording` before
//! the take and removes it once the take is finished, so a marker left
//! behind means the app or the computer stopped mid-take. The take's header
//! then names fewer samples than the file holds; the recorder refreshes it
//! every second, so at most the last second is unaccounted for.
//!
//! [`recover_takes`] is the startup scan: for each marker it has the take's
//! header repaired by the caller's `repair` and removes the marker, and
//! returns the takes to offer for import. This crate never reads audio
//! (it doesn't depend on `freecut-audio`), so the app passes
//! `freecut_audio::record::wav::repair`. Everything here is blocking file
//! I/O for a worker thread.

use std::io;
use std::path::{Path, PathBuf};

use crate::names::{fnv1a, sibling_name};

/// Added to the project's name for its media folder: `My Trip Media`.
pub const MEDIA_FOLDER_SUFFIX: &str = " Media";

/// The recordings folder inside the media folder (and inside app data for
/// the fallback).
pub const RECORDINGS_FOLDER: &str = "Recordings";

/// Added to a take's file name for its in-progress marker. Must equal
/// `freecut_audio::record::wav::MARKER_SUFFIX`, which writes the markers;
/// this crate can't depend on `freecut-audio`, so the app checks the two
/// agree.
pub const MARKER_SUFFIX: &str = ".recording";

/// The fallback folder of a project that was never saved.
pub const UNSAVED_FOLDER: &str = "Unsaved";

/// The most takes [`new_take_path`] numbers within one second.
pub const MAX_TAKES_PER_SECOND: u32 = 999;

/// The project's media folder, `<Name> Media` next to `<Name>.freecut`;
/// `None` when `project_file` has no file name. A name too long for the
/// file system is shortened the way the lock and backup names are.
pub fn media_folder(project_file: &Path) -> Option<PathBuf> {
    let stem = project_file.file_stem()?;
    let dir = project_file.parent().unwrap_or(Path::new(""));
    Some(dir.join(sibling_name("", stem, MEDIA_FOLDER_SUFFIX)))
}

/// Where the project's recordings go: `<Name> Media/Recordings`.
pub fn recordings_folder(project_file: &Path) -> Option<PathBuf> {
    media_folder(project_file).map(|media| media.join(RECORDINGS_FOLDER))
}

/// Where recordings go when the project folder can't be written, or the
/// project was never saved: `<app data>/Recordings/<Name> <hash>`, where the
/// hash of the project file's path tells apart projects of one name, or
/// `<app data>/Recordings/Unsaved`.
pub fn fallback_recordings_folder(app_data: &Path, project_file: Option<&Path>) -> PathBuf {
    let base = app_data.join(RECORDINGS_FOLDER);
    let Some((path, stem)) = project_file.and_then(|p| Some((p, p.file_stem()?))) else {
        return base.join(UNSAVED_FOLDER);
    };
    let tag = format!(" {:016x}", fnv1a(path.as_os_str().as_encoded_bytes()));
    base.join(sibling_name("", stem, &tag))
}

/// The local date and time a take started, as the app reads it from the
/// clock (this crate never does).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TakeStamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// A take's file name: `Voiceover 2026-10-12 14.32.05.wav` for the first
/// take of that second, then `… 14.32.05 (2).wav` and so on. Dots, not
/// colons, separate the time, so the name is valid on every file system.
pub fn take_file_name(stamp: TakeStamp, n: u32) -> String {
    let TakeStamp {
        year,
        month,
        day,
        hour,
        minute,
        second,
    } = stamp;
    let number = if n > 1 {
        format!(" ({n})")
    } else {
        String::new()
    };
    format!("Voiceover {year:04}-{month:02}-{day:02} {hour:02}.{minute:02}.{second:02}{number}.wav")
}

/// The path for a new take in `folder`: the first [`take_file_name`] whose
/// file and marker are both free (`exists` tells), so a take never replaces
/// another, finished or interrupted. `None` after
/// [`MAX_TAKES_PER_SECOND`] names in one second.
pub fn new_take_path(
    folder: &Path,
    stamp: TakeStamp,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    (1..=MAX_TAKES_PER_SECOND)
        .map(|n| folder.join(take_file_name(stamp, n)))
        .find(|take| !exists(take) && !exists(&marker_path(take)))
}

/// The marker that sits beside `take` while it is written:
/// `<take>.recording`.
pub fn marker_path(take: &Path) -> PathBuf {
    let mut name = take.as_os_str().to_owned();
    name.push(MARKER_SUFFIX);
    PathBuf::from(name)
}

/// A marker found in a recordings folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedTake {
    /// The take the marker belongs to.
    pub take: PathBuf,
    pub marker: PathBuf,
    /// Whether the take's file is there. The recorder creates the marker
    /// first, so a crash in between leaves a marker without a take.
    pub take_exists: bool,
}

/// The markers in `folder`, by path. A folder that doesn't exist has none.
pub fn find_interrupted(folder: &Path) -> io::Result<Vec<InterruptedTake>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let marker = entry.path();
        let Some(take) = take_of_marker(&marker) else {
            continue;
        };
        let take_exists = std::fs::metadata(&take).is_ok_and(|m| m.is_file());
        found.push(InterruptedTake {
            take,
            marker,
            take_exists,
        });
    }
    found.sort_by(|a, b| a.marker.cmp(&b.marker));
    Ok(found)
}

/// The take `marker` belongs to, when its name is `<take name>.recording`.
/// Take names are Unicode ([`take_file_name`]); a marker whose name isn't is
/// not one of ours.
fn take_of_marker(marker: &Path) -> Option<PathBuf> {
    let name = marker.file_name()?.to_str()?;
    let take = name.strip_suffix(MARKER_SUFFIX).filter(|t| !t.is_empty())?;
    Some(marker.with_file_name(take))
}

/// A take [`recover_takes`] couldn't finish.
#[derive(Debug)]
pub struct FailedTake {
    pub take: PathBuf,
    pub error: io::Error,
}

/// What [`recover_takes`] did.
#[derive(Debug, Default)]
pub struct Recovery {
    /// Takes whose header was repaired and whose marker was removed, by
    /// path: offer them for import (AU-027's crash edge case). A take
    /// already in the library (by path) needs no offer.
    pub recovered: Vec<PathBuf>,
    /// Markers without a take, removed.
    pub cleared_markers: Vec<PathBuf>,
    /// Takes that couldn't be repaired, or whose marker couldn't be
    /// removed. The take and its marker are left as they are, so the next
    /// scan tries again; nothing is ever deleted but a marker.
    pub failed: Vec<FailedTake>,
}

/// The startup scan of a recordings folder (AU-027, PM-043): repairs each
/// interrupted take with `repair` (the app passes
/// `freecut_audio::record::wav::repair`), removes its marker, and reports
/// the takes to offer for import.
///
/// Run it on a worker when the project opens, while no recorder writes into
/// `folder` (the project lock is held and the recorder isn't open yet): a
/// take being recorded has a marker too. The error is the folder's own (it
/// can't be listed); each take's errors go into [`Recovery::failed`].
pub fn recover_takes(
    folder: &Path,
    mut repair: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<Recovery> {
    let mut out = Recovery::default();
    for found in find_interrupted(folder)? {
        if !found.take_exists {
            match std::fs::remove_file(&found.marker) {
                Ok(()) => out.cleared_markers.push(found.marker),
                Err(error) => out.failed.push(FailedTake {
                    take: found.take,
                    error,
                }),
            }
            continue;
        }
        let done = repair(&found.take).and_then(|()| std::fs::remove_file(&found.marker));
        match done {
            Ok(()) => out.recovered.push(found.take),
            Err(error) => out.failed.push(FailedTake {
                take: found.take,
                error,
            }),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> TakeStamp {
        TakeStamp {
            year: 2026,
            month: 10,
            day: 2,
            hour: 9,
            minute: 5,
            second: 7,
        }
    }

    #[test]
    fn folders_sit_next_to_the_project_file() {
        let project = Path::new("/u/me/Movies/My Trip.freecut");
        assert_eq!(
            media_folder(project),
            Some(PathBuf::from("/u/me/Movies/My Trip Media"))
        );
        assert_eq!(
            recordings_folder(project),
            Some(PathBuf::from("/u/me/Movies/My Trip Media/Recordings"))
        );
        assert_eq!(
            recordings_folder(Path::new("Trip.freecut")),
            Some(PathBuf::from("Trip Media/Recordings"))
        );
        assert_eq!(media_folder(Path::new("/")), None);
        // A name at the file-system limit is shortened, not refused.
        let long = format!("/p/{}.freecut", "語".repeat(84));
        let media = media_folder(Path::new(&long)).unwrap();
        let name = media.file_name().unwrap();
        assert!(name.len() <= 255, "{}", name.len());
        assert!(name.to_string_lossy().ends_with(MEDIA_FOLDER_SUFFIX));
    }

    #[test]
    fn the_fallback_folder_tells_projects_apart() {
        let data = Path::new("/data/FreeCut");
        assert_eq!(
            fallback_recordings_folder(data, None),
            PathBuf::from("/data/FreeCut/Recordings/Unsaved")
        );
        let a = fallback_recordings_folder(data, Some(Path::new("/a/Trip.freecut")));
        let b = fallback_recordings_folder(data, Some(Path::new("/b/Trip.freecut")));
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(Path::new("/data/FreeCut/Recordings")));
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("Trip ") && name.len() == "Trip ".len() + 16,
            "{name}"
        );
        // The same project always gets the same folder.
        assert_eq!(
            a,
            fallback_recordings_folder(data, Some(Path::new("/a/Trip.freecut")))
        );
    }

    #[test]
    fn take_names_carry_the_start_time_and_a_number_after_the_first() {
        assert_eq!(
            take_file_name(stamp(), 1),
            "Voiceover 2026-10-02 09.05.07.wav"
        );
        assert_eq!(
            take_file_name(stamp(), 2),
            "Voiceover 2026-10-02 09.05.07 (2).wav"
        );
        let folder = Path::new("/r");
        let taken = |p: &Path| {
            [
                "/r/Voiceover 2026-10-02 09.05.07.wav",
                // An interrupted take's marker keeps its name taken.
                "/r/Voiceover 2026-10-02 09.05.07 (2).wav.recording",
            ]
            .iter()
            .any(|t| Path::new(t) == p)
        };
        assert_eq!(
            new_take_path(folder, stamp(), taken),
            Some(PathBuf::from("/r/Voiceover 2026-10-02 09.05.07 (3).wav"))
        );
        assert_eq!(new_take_path(folder, stamp(), |_| true), None);
    }

    #[test]
    fn markers_name_their_take() {
        let take = Path::new("/r/Voiceover 2026-10-02 09.05.07.wav");
        let marker = marker_path(take);
        assert_eq!(
            marker,
            PathBuf::from("/r/Voiceover 2026-10-02 09.05.07.wav.recording")
        );
        assert_eq!(take_of_marker(&marker).as_deref(), Some(take));
        assert_eq!(take_of_marker(Path::new("/r/.recording")), None);
        assert_eq!(take_of_marker(Path::new("/r/take.wav")), None);
    }

    #[test]
    fn a_missing_folder_has_no_interrupted_takes() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("Recordings");
        assert!(find_interrupted(&gone).unwrap().is_empty());
        let recovery = recover_takes(&gone, |_| panic!("nothing to repair")).unwrap();
        assert!(recovery.recovered.is_empty() && recovery.failed.is_empty());
    }

    #[test]
    fn interrupted_takes_are_repaired_and_their_markers_removed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let write = |name: &str, bytes: &[u8]| std::fs::write(folder.join(name), bytes).unwrap();
        // A finished take, an interrupted one, one whose repair fails, a
        // marker whose take was never created, and a folder that only
        // looks like a marker.
        write("Voiceover 1.wav", b"finished");
        write("Voiceover 2.wav", b"short header");
        write("Voiceover 2.wav.recording", b"");
        write("Voiceover 3.wav", b"not a recording");
        write("Voiceover 3.wav.recording", b"");
        write("Voiceover 4.wav.recording", b"");
        std::fs::create_dir(folder.join("odd.wav.recording")).unwrap();

        let found = find_interrupted(folder).unwrap();
        let names: Vec<(String, bool)> = found
            .iter()
            .map(|f| {
                let name = f.take.file_name().unwrap().to_string_lossy().into_owned();
                (name, f.take_exists)
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("Voiceover 2.wav".into(), true),
                ("Voiceover 3.wav".into(), true),
                ("Voiceover 4.wav".into(), false),
            ]
        );

        let mut repaired = Vec::new();
        let recovery = recover_takes(folder, |take| {
            repaired.push(take.to_path_buf());
            if std::fs::read(take)? == b"not a recording" {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "not ours"));
            }
            std::fs::write(take, b"repaired")
        })
        .unwrap();
        assert_eq!(
            repaired,
            vec![
                folder.join("Voiceover 2.wav"),
                folder.join("Voiceover 3.wav")
            ]
        );
        assert_eq!(recovery.recovered, vec![folder.join("Voiceover 2.wav")]);
        assert_eq!(
            recovery.cleared_markers,
            vec![folder.join("Voiceover 4.wav.recording")]
        );
        assert_eq!(recovery.failed.len(), 1);
        assert_eq!(recovery.failed[0].take, folder.join("Voiceover 3.wav"));
        assert_eq!(recovery.failed[0].error.kind(), io::ErrorKind::InvalidData);

        // The repaired take keeps its audio; the failed one keeps its file
        // and marker for the next scan; the finished take is untouched.
        assert_eq!(
            std::fs::read(folder.join("Voiceover 2.wav")).unwrap(),
            b"repaired"
        );
        assert!(!folder.join("Voiceover 2.wav.recording").exists());
        assert!(folder.join("Voiceover 3.wav.recording").exists());
        assert_eq!(
            std::fs::read(folder.join("Voiceover 1.wav")).unwrap(),
            b"finished"
        );
        let again = find_interrupted(folder).unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].take, folder.join("Voiceover 3.wav"));
    }
}
