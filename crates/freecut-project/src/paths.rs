//! Portable media paths (PM-100).
//!
//! Each media item stores the absolute path it was imported from and, when
//! the file sits on the same volume and within [`MAX_PARENT_STEPS`] `..`
//! steps of the project folder, a relative path with `/` separators. Moving
//! the project folder together with its media then keeps the media online.
//!
//! The relative form is computed by pure string math that understands both
//! POSIX and Windows paths (drive letters, UNC shares, `\\?\` prefixes), so
//! a project saved on one OS behaves predictably on another and the rules
//! can be table-tested anywhere. Resolution goes through
//! [`FileSystem`], so it can be tested without disks.
//!
//! Resolution tries the relative form first, then the absolute one. An
//! absolute path in a foreign style (a Windows path on macOS) is never
//! joined onto a local folder; only its relative form can find the file.
//!
//! A relative path only means something next to the project file it was
//! computed for. Saving into another folder therefore rewrites each one with
//! [`rebase_relative`] (the save pipeline does this; see
//! `SaveRequest::relative_base`).

use std::path::{Component, Path, PathBuf};

use freecut_timeline::model::StoredPath;

use crate::fs::FileSystem;

/// The most `..` steps a stored relative path may take (PM-100).
pub const MAX_PARENT_STEPS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("{} is not an absolute path", .0.display())]
    NotAbsolute(PathBuf),
    /// The model stores paths as UTF-8 strings.
    #[error("{} is not valid UTF-8", .0.display())]
    NotUtf8(PathBuf),
}

/// Builds the stored form of the media file at `absolute` for a project
/// whose file lives in `project_dir`. The relative form is omitted when the
/// file is on another volume, too far up, or `project_dir` isn't absolute
/// UTF-8.
pub fn stored_path(absolute: &Path, project_dir: &Path) -> Result<StoredPath, PathError> {
    if !absolute.is_absolute() {
        return Err(PathError::NotAbsolute(absolute.to_path_buf()));
    }
    let abs = absolute
        .to_str()
        .ok_or_else(|| PathError::NotUtf8(absolute.to_path_buf()))?;
    let relative = project_dir.to_str().and_then(|dir| relative_path(dir, abs));
    Ok(StoredPath {
        absolute: abs.to_string(),
        relative,
    })
}

/// The path of `target` relative to the folder `base_dir`, `/`-separated,
/// or `None` when there is none worth storing: different path styles or
/// volumes, more than [`MAX_PARENT_STEPS`] `..` steps, a target that is not
/// below a common folder, or a POSIX name containing `\` (which a Windows
/// reader would split). Both arguments must be absolute.
///
/// Windows paths compare case-insensitively; POSIX paths byte for byte.
pub fn relative_path(base_dir: &str, target: &str) -> Option<String> {
    let base = Absolute::parse(base_dir)?;
    let target = Absolute::parse(target)?;
    if base.style != target.style || base.volume != target.volume {
        return None;
    }
    let same = |a: &str, b: &str| match base.style {
        Style::Posix => a == b,
        Style::Windows => a == b || a.to_lowercase() == b.to_lowercase(),
    };
    let common = base
        .parts
        .iter()
        .zip(&target.parts)
        .take_while(|(a, b)| same(a, b))
        .count();
    let ups = base.parts.len() - common;
    let rest = &target.parts[common..];
    if ups > MAX_PARENT_STEPS || rest.is_empty() {
        return None;
    }
    if base.style == Style::Posix && rest.iter().any(|p| p.contains('\\')) {
        return None;
    }
    let mut parts: Vec<&str> = vec![".."; ups];
    parts.extend(rest.iter().copied());
    let relative = parts.join("/");
    // A POSIX folder named like a drive (`C:`) would make the path read as
    // absolute, and `join_relative` refuses those.
    (!looks_absolute(&relative)).then_some(relative)
}

/// The relative path to store for `stored` in a project file in `to_dir`,
/// when its stored relative path is relative to `from_dir`: the path from
/// `to_dir` to the file the stored relative path names from `from_dir`, or,
/// without a usable stored relative path, to the stored absolute path.
///
/// This keeps a project resolving to the same files when it is saved into
/// another folder (Save As, Save a Copy; PM-012, PM-100). Pure string math
/// like [`relative_path`]: `None` where no relative path is possible.
pub fn rebase_relative(stored: &StoredPath, from_dir: &Path, to_dir: &Path) -> Option<String> {
    let to_dir = to_dir.to_str()?;
    match stored
        .relative
        .as_deref()
        .and_then(|rel| join_relative(from_dir, rel))
    {
        Some(target) => relative_path(to_dir, target.to_str()?),
        None => relative_path(to_dir, &stored.absolute),
    }
}

/// Whether a relative path string would read as absolute somewhere: it
/// starts with a separator or a Windows drive (`C:` alone or followed by a
/// separator). Other colons are ordinary POSIX name characters
/// (`1:30 take.mov`).
fn looks_absolute(relative: &str) -> bool {
    let bytes = relative.as_bytes();
    let drive = bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes.get(2).is_none_or(|b| matches!(b, b'/' | b'\\'));
    relative.starts_with(['/', '\\']) || drive
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Posix,
    Windows,
}

/// An absolute path split into a volume and normalized components.
#[derive(Debug)]
struct Absolute<'a> {
    style: Style,
    /// `""` for POSIX, `"c:"` for a drive, `"\\server\share"` for UNC;
    /// lowercased on Windows.
    volume: String,
    parts: Vec<&'a str>,
}

impl<'a> Absolute<'a> {
    fn parse(path: &'a str) -> Option<Absolute<'a>> {
        if let Some(rest) = path
            .strip_prefix(r"\\?\")
            .or_else(|| path.strip_prefix(r"\\.\"))
        {
            // Verbatim and device prefixes: `\\?\C:\…` or `\\?\UNC\server\share\…`.
            return match rest.get(..4) {
                Some(unc) if unc.eq_ignore_ascii_case(r"UNC\") => Absolute::unc(&rest[4..]),
                _ => Absolute::drive(rest),
            };
        }
        if let Some(rest) = path.strip_prefix(r"\\") {
            return Absolute::unc(rest);
        }
        if path.starts_with('/') {
            return Some(Absolute {
                style: Style::Posix,
                volume: String::new(),
                parts: normalize(path.split('/')),
            });
        }
        Absolute::drive(path)
    }

    /// `C:\rest` or `C:/rest`.
    fn drive(path: &'a str) -> Option<Absolute<'a>> {
        let bytes = path.as_bytes();
        let is_drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/');
        if !is_drive {
            return None;
        }
        Some(Absolute {
            style: Style::Windows,
            volume: path[..2].to_ascii_lowercase(),
            parts: normalize(windows_split(&path[3..])),
        })
    }

    /// `server\share\rest` (the leading `\\` already removed).
    fn unc(rest: &'a str) -> Option<Absolute<'a>> {
        let mut parts = windows_split(rest);
        let server = parts.next().filter(|s| !s.is_empty())?;
        let share = parts.next().filter(|s| !s.is_empty())?;
        Some(Absolute {
            style: Style::Windows,
            volume: format!(r"\\{server}\{share}").to_lowercase(),
            parts: normalize(parts),
        })
    }
}

fn windows_split(path: &str) -> impl Iterator<Item = &str> {
    path.split(['\\', '/'])
}

/// Drops empty and `.` components and applies `..` lexically (`..` at the
/// root stays at the root, as the OS does).
fn normalize<'a>(parts: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out = Vec::new();
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            name => out.push(name),
        }
    }
    out
}

/// How [`resolve`] found a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedHow {
    /// Through the relative path from the project folder.
    Relative,
    /// Through the stored absolute path.
    Absolute,
    /// Neither exists: the item is offline (relink applies).
    NotFound,
}

/// Where a stored path points now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The file found, or the stored absolute path when nothing was found.
    pub path: PathBuf,
    pub how: ResolvedHow,
}

/// Finds the file a stored path refers to, for a project whose file now
/// lives in `project_dir`: the relative path first, then the absolute one.
pub fn resolve(stored: &StoredPath, project_dir: &Path, fs: &dyn FileSystem) -> Resolved {
    if let Some(candidate) = stored
        .relative
        .as_deref()
        .and_then(|rel| join_relative(project_dir, rel))
        && fs.is_file(&candidate)
    {
        return Resolved {
            path: candidate,
            how: ResolvedHow::Relative,
        };
    }
    let absolute = Path::new(&stored.absolute);
    // A path in another OS's style is not absolute here; never join it.
    if absolute.is_absolute() && fs.is_file(absolute) {
        return Resolved {
            path: absolute.to_path_buf(),
            how: ResolvedHow::Absolute,
        };
    }
    Resolved {
        path: absolute.to_path_buf(),
        how: ResolvedHow::NotFound,
    }
}

/// `dir` joined with a stored relative path, accepting `\` as a separator
/// too and applying `..` lexically (the same way the relative path was
/// computed). `None` for anything that isn't a plain relative path: one that
/// starts with a separator or a drive (`C:/…`), climbs above the root, or
/// has a component this OS reads as more than a plain name.
pub fn join_relative(dir: &Path, relative: &str) -> Option<PathBuf> {
    if looks_absolute(relative) {
        return None;
    }
    let mut out = dir.to_path_buf();
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if !out.pop() {
                    return None;
                }
            }
            name => {
                // Refuse anything the OS would read as more than one plain
                // component (a drive like `C:` on Windows would replace `out`).
                let mut components = Path::new(name).components();
                match (components.next(), components.next()) {
                    (Some(Component::Normal(_)), None) => out.push(name),
                    _ => return None,
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_table() {
        let cases: &[(&str, &str, Option<&str>)] = &[
            // POSIX: below, beside and above the project folder.
            ("/u/me/proj", "/u/me/proj/media/a.mov", Some("media/a.mov")),
            ("/u/me/proj", "/u/me/clips/a.mov", Some("../clips/a.mov")),
            ("/u/me/proj/", "/u/me/proj/a.mov", Some("a.mov")),
            ("/u/me/proj", "/a.mov", Some("../../../a.mov")),
            ("/u/me/./proj/sub/..", "/u/me/proj/a.mov", Some("a.mov")),
            // Byte-exact on POSIX.
            (
                "/u/Me/proj",
                "/u/me/proj/a.mov",
                Some("../../me/proj/a.mov"),
            ),
            // At most 8 `..` steps.
            (
                "/1/2/3/4/5/6/7/8",
                "/a.mov",
                Some("../../../../../../../../a.mov"),
            ),
            ("/1/2/3/4/5/6/7/8/9", "/a.mov", None),
            // The folder itself or an ancestor is not a file.
            ("/u/me/proj", "/u/me/proj", None),
            ("/u/me/proj", "/u/me", None),
            // A POSIX name with a backslash would split on Windows.
            ("/u/me/proj", "/u/me/proj/a\\b.mov", None),
            // Colons are plain POSIX name characters, except where the
            // path would start like a Windows drive.
            (
                "/u/me/proj",
                "/u/me/proj/1:30 take.mov",
                Some("1:30 take.mov"),
            ),
            ("/u/me/proj", "/u/me/proj/C:x.mov", Some("C:x.mov")),
            ("/u/me/proj", "/u/me/proj/C:/a.mov", None),
            ("/u/me/proj", "/u/me/proj/C:", None),
            ("/u/me/proj/sub", "/u/me/proj/C:/a.mov", Some("../C:/a.mov")),
            // Windows drives, case-insensitive, both separators.
            (
                r"C:\Users\Me\proj",
                r"c:\users\me\media\a.mov",
                Some("../media/a.mov"),
            ),
            (
                r"C:\Users\me\proj",
                "C:/Users/me/proj/media/a.mov",
                Some("media/a.mov"),
            ),
            // Different volumes never get a relative path.
            (r"C:\Users\me\proj", r"D:\clips\a.mov", None),
            (r"C:\proj", r"\\server\share\a.mov", None),
            (r"\\server\share\proj", r"\\server\other\a.mov", None),
            // UNC shares and verbatim prefixes.
            (
                r"\\Server\Share\proj",
                r"\\server\share\media\a.mov",
                Some("../media/a.mov"),
            ),
            (r"\\?\C:\proj", r"C:\proj\a.mov", Some("a.mov")),
            (
                r"\\?\UNC\server\share\proj",
                r"\\server\share\proj\a.mov",
                Some("a.mov"),
            ),
            // Mixed styles or relative inputs.
            ("/u/me/proj", r"C:\a.mov", None),
            ("proj", "/u/a.mov", None),
            ("/u/me/proj", "a.mov", None),
            (r"C:proj", r"C:\a.mov", None),
        ];
        for (base, target, expected) in cases {
            assert_eq!(
                relative_path(base, target).as_deref(),
                *expected,
                "relative_path({base:?}, {target:?})"
            );
        }
    }

    #[test]
    fn join_relative_accepts_both_separators_and_refuses_escapes() {
        let dir = Path::new("/new/proj");
        assert_eq!(
            join_relative(dir, "media/a.mov"),
            Some(PathBuf::from("/new/proj/media/a.mov"))
        );
        assert_eq!(
            join_relative(dir, r"..\clips\a.mov"),
            Some(PathBuf::from("/new/clips/a.mov"))
        );
        assert_eq!(
            join_relative(dir, "./media//a.mov"),
            Some(PathBuf::from("/new/proj/media/a.mov"))
        );
        assert_eq!(join_relative(dir, "/etc/passwd"), None);
        assert_eq!(join_relative(dir, r"\etc"), None);
        assert_eq!(join_relative(dir, "C:/a.mov"), None);
        assert_eq!(join_relative(dir, r"c:\a.mov"), None);
        assert_eq!(join_relative(dir, "C:"), None);
        assert_eq!(join_relative(dir, "../../../../a.mov"), None);
    }

    /// Everything `relative_path` writes, `join_relative` reads back to the
    /// same file (review F2: `1:30 take.mov` used to be refused).
    #[cfg(unix)]
    #[test]
    fn join_relative_reads_every_posix_relative_path_back() {
        let dir = "/u/me/proj";
        for target in [
            "/u/me/proj/1:30 take.mov",
            "/u/me/proj/a:b/c.mov",
            "/u/me/proj/C:x.mov",
            "/u/me/clips/1:30.mov",
            "/u/me/proj/sub/C:/a.mov",
        ] {
            let rel = relative_path(dir, target).unwrap();
            assert_eq!(
                join_relative(Path::new(dir), &rel),
                Some(PathBuf::from(target)),
                "{rel}"
            );
        }
    }

    #[test]
    fn rebasing_keeps_naming_the_same_file() {
        let stored = |absolute: &str, relative: Option<&str>| StoredPath {
            absolute: absolute.into(),
            relative: relative.map(String::from),
        };
        let cases: &[(StoredPath, &str, &str, Option<&str>)] = &[
            // Save As into a sibling folder.
            (
                stored("/u/A/Media/a.mov", Some("Media/a.mov")),
                "/u/A",
                "/u/B",
                Some("../A/Media/a.mov"),
            ),
            // The stored relative path wins over a stale absolute one (the
            // project moved with its media before this Save As).
            (
                stored("/old/A/Media/a.mov", Some("Media/a.mov")),
                "/new/A",
                "/new/B",
                Some("../A/Media/a.mov"),
            ),
            // Into a subfolder, and with Windows separators in the stored path.
            (
                stored("/u/A/clips/a.mov", Some(r"..\A\clips\a.mov")),
                "/u/A",
                "/u/A/copies",
                Some("../clips/a.mov"),
            ),
            // No usable relative path: computed from the absolute one.
            (
                stored("/u/A/a.mov", None),
                "/u/A",
                "/u/B",
                Some("../A/a.mov"),
            ),
            (
                stored("/u/A/a.mov", Some("/etc/a.mov")),
                "/u/A",
                "/u/B",
                Some("../A/a.mov"),
            ),
            // Too far from the new folder, or a foreign absolute path.
            (
                stored("/a.mov", Some("../a.mov")),
                "/u",
                "/1/2/3/4/5/6/7/8/9",
                None,
            ),
            (stored(r"C:\m\a.mov", None), "/u/A", "/u/B", None),
            // Windows folders.
            (
                stored(r"C:\p\A\a.mov", Some("a.mov")),
                r"C:\p\A",
                r"C:\p\B",
                Some("../A/a.mov"),
            ),
        ];
        for (sp, from, to, expected) in cases {
            if cfg!(windows) && from.starts_with('/') {
                continue;
            }
            if !cfg!(windows) && from.contains('\\') {
                continue;
            }
            assert_eq!(
                rebase_relative(sp, Path::new(from), Path::new(to)).as_deref(),
                *expected,
                "{sp:?} from {from} to {to}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn stored_path_needs_an_absolute_utf8_path() {
        let sp = stored_path(Path::new("/u/me/clips/a.mov"), Path::new("/u/me/proj")).unwrap();
        assert_eq!(sp.absolute, "/u/me/clips/a.mov");
        assert_eq!(sp.relative.as_deref(), Some("../clips/a.mov"));
        assert_eq!(
            stored_path(Path::new("a.mov"), Path::new("/u")),
            Err(PathError::NotAbsolute(PathBuf::from("a.mov")))
        );
        let far = stored_path(Path::new("/a.mov"), Path::new("/1/2/3/4/5/6/7/8/9")).unwrap();
        assert_eq!(far.relative, None);

        use std::os::unix::ffi::OsStrExt;
        let bad = Path::new(std::ffi::OsStr::from_bytes(b"/u/\xff.mov"));
        assert!(matches!(
            stored_path(bad, Path::new("/u")),
            Err(PathError::NotUtf8(_))
        ));
    }
}
