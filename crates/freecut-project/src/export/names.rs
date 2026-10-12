//! Output file names (EX-023): sanitizing, the target path and the
//! Keep-both conflict rule. Pure: the file system is reached only through a
//! callback the app supplies.

use std::path::{Path, PathBuf};

use super::settings::Container;

/// The longest output name in characters (EX-023).
pub const MAX_STEM_CHARS: usize = 200;

/// The longest file name most file systems accept, in UTF-8 bytes (ext4 and
/// APFS; NTFS counts UTF-16 units, never more than UTF-8 bytes).
pub const MAX_FILE_NAME_BYTES: usize = 255;

/// The name used when nothing valid is left of the typed one (EX-023).
pub const UNTITLED: &str = "Untitled";

/// The highest Keep-both number tried by [`resolve_conflict`].
pub const MAX_CONFLICT_SUFFIX: u32 = 9_999;

/// Characters that no output name may contain on any OS (EX-023).
const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Windows device names that can't be file names, with or without an
/// extension (EX-023).
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file stem that is valid on every OS, from the name the user typed
/// (EX-023), applied the same way on every OS:
///
/// - `<>:"/\|?*` and control characters become `_`;
/// - leading and trailing spaces and trailing dots are trimmed;
/// - a Windows device name (`CON`, `com1`, `Nul.backup`, …) gets `_` after
///   the device part (`CON_`, `Nul_.backup`).
///
/// An empty or whitespace-only name gives `""`: the dialog then asks for a
/// name instead of filling one in (preflight reports it). A name with
/// nothing valid in it (only forbidden characters, or only dots) gives
/// [`UNTITLED`]. Length is not changed here; preflight refuses names over
/// [`MAX_STEM_CHARS`].
pub fn sanitize_file_stem(name: &str) -> String {
    if name.trim().is_empty() {
        return String::new();
    }
    let mut kept_any = false;
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || FORBIDDEN.contains(&c) {
                '_'
            } else {
                if c != '.' && !c.is_whitespace() {
                    kept_any = true;
                }
                c
            }
        })
        .collect();
    if !kept_any {
        return UNTITLED.to_string();
    }
    // Trailing dots and spaces in any mix ("a . ."), and leading spaces.
    let trimmed = replaced
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_start_matches(|c: char| c.is_whitespace());
    let mut stem = trimmed.to_string();
    // The device part is everything before the first dot, without the
    // spaces Windows ignores there.
    let device_end = stem.find('.').unwrap_or(stem.len());
    let device = stem[..device_end].trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(device)) {
        stem.insert(device.len(), '_');
    }
    stem
}

/// `folder/stem.ext` for `container`.
pub fn output_path(folder: &Path, stem: &str, container: Container) -> PathBuf {
    folder.join(file_name(stem, None, container))
}

/// `stem.ext`, or `stem (n).ext` for a Keep-both number.
fn file_name(stem: &str, number: Option<u32>, container: Container) -> String {
    match number {
        None => format!("{stem}.{}", container.extension()),
        Some(n) => format!("{stem} ({n}).{}", container.extension()),
    }
}

/// What to do when the output file exists (EX-023).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ConflictChoice {
    /// Write over it (the old file stays until the finished export is
    /// renamed into place).
    Replace,
    /// Add ` (2)`, ` (3)`, … to the new file's name. The default button.
    #[default]
    KeepBoth,
}

/// The Keep-both target (EX-023): `folder/stem.ext` if `exists` says it is
/// free, else the first free `folder/stem (n).ext` for n = 2, 3, …
/// `exists` is the file system's answer (case-insensitive where the file
/// system is), possibly widened by the app to the targets of queued jobs.
/// `None` when every number up to [`MAX_CONFLICT_SUFFIX`] is taken.
pub fn resolve_conflict(
    folder: &Path,
    stem: &str,
    container: Container,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    core::iter::once(None)
        .chain((2..=MAX_CONFLICT_SUFFIX).map(Some))
        .map(|n| folder.join(file_name(stem, n, container)))
        .find(|path| !exists(path))
}

/// The path to export to for `choice`: the plain path for Replace, the
/// first free one for Keep both.
pub fn target_path(
    folder: &Path,
    stem: &str,
    container: Container,
    choice: ConflictChoice,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    match choice {
        ConflictChoice::Replace => Some(output_path(folder, stem, container)),
        ConflictChoice::KeepBoth => resolve_conflict(folder, stem, container, exists),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn forbidden_and_control_characters_become_underscores() {
        assert_eq!(sanitize_file_stem("My Trip"), "My Trip");
        assert_eq!(
            sanitize_file_stem("a<b>c:d\"e/f\\g|h?i*j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_file_stem("tab\there\nnew\u{7f}"), "tab_here_new_");
        assert_eq!(sanitize_file_stem("Café ✂ 🎬"), "Café ✂ 🎬");
        assert_eq!(sanitize_file_stem("v1.2 final"), "v1.2 final");
    }

    #[test]
    fn spaces_and_trailing_dots_are_trimmed() {
        assert_eq!(sanitize_file_stem("  Trip  "), "Trip");
        assert_eq!(sanitize_file_stem("Trip..."), "Trip");
        assert_eq!(sanitize_file_stem("Trip. . ."), "Trip");
        assert_eq!(sanitize_file_stem(".hidden"), ".hidden");
        assert_eq!(sanitize_file_stem("a .b"), "a .b");
    }

    #[test]
    fn windows_device_names_get_a_suffix() {
        assert_eq!(sanitize_file_stem("CON"), "CON_");
        assert_eq!(sanitize_file_stem("con"), "con_");
        assert_eq!(sanitize_file_stem("Com1"), "Com1_");
        assert_eq!(sanitize_file_stem("LPT9"), "LPT9_");
        assert_eq!(sanitize_file_stem("nul.backup"), "nul_.backup");
        assert_eq!(sanitize_file_stem("AUX ."), "AUX_");
        assert_eq!(sanitize_file_stem("CONSOLE"), "CONSOLE");
        assert_eq!(sanitize_file_stem("COM10"), "COM10");
        assert_eq!(sanitize_file_stem("COM0"), "COM0");
    }

    #[test]
    fn empty_names_stay_empty_and_invalid_ones_become_untitled() {
        assert_eq!(sanitize_file_stem(""), "");
        assert_eq!(sanitize_file_stem("   \t "), "");
        assert_eq!(sanitize_file_stem("???"), UNTITLED);
        assert_eq!(sanitize_file_stem("..."), UNTITLED);
        assert_eq!(sanitize_file_stem(" /\\ "), UNTITLED);
        assert_eq!(sanitize_file_stem("\u{1}"), UNTITLED);
        // One valid character is enough.
        assert_eq!(sanitize_file_stem("?a?"), "_a_");
    }

    #[test]
    fn sanitizing_is_idempotent() {
        for name in [
            "CON",
            "a<b",
            " x. ",
            "nul.backup",
            "Trip...",
            "?a?",
            "Untitled",
        ] {
            let once = sanitize_file_stem(name);
            assert_eq!(sanitize_file_stem(&once), once, "{name:?}");
        }
    }

    #[test]
    fn keep_both_counts_from_two_and_asks_the_file_system() {
        let folder = Path::new("/out");
        let taken: BTreeSet<String> = ["trip.mp4", "trip (2).mp4", "trip (4).mp4"]
            .into_iter()
            .map(String::from)
            .collect();
        // A case-insensitive file system, as macOS and Windows have.
        let exists = |p: &Path| {
            let name = p.file_name().unwrap().to_string_lossy().to_lowercase();
            taken.contains(&name)
        };
        assert_eq!(
            resolve_conflict(folder, "Trip", Container::Mp4, &exists),
            Some(PathBuf::from("/out/Trip (3).mp4"))
        );
        assert_eq!(
            resolve_conflict(folder, "Other", Container::Mp4, &exists),
            Some(PathBuf::from("/out/Other.mp4"))
        );
        assert_eq!(
            target_path(
                folder,
                "Trip",
                Container::Mp4,
                ConflictChoice::Replace,
                &exists
            ),
            Some(PathBuf::from("/out/Trip.mp4"))
        );
        assert_eq!(
            resolve_conflict(folder, "x", Container::Mp4, &|_| true),
            None
        );
        assert_eq!(ConflictChoice::default(), ConflictChoice::KeepBoth);
    }
}
