//! Blocking OS calls for the export dialog, run on the worker pool, never
//! on the UI thread:
//!
//! - [`probe_destination`]: is the folder there and writable, which drive
//!   is it on, how much space is free, does the file exist already
//!   (EX-023, EX-036);
//! - [`encoder_available`]: does the linked FFmpeg have the MVP encoders
//!   (EX-029);
//! - [`open_path`]: "Open" and "Show in folder" with the platform's own
//!   opener (EX-039).
//!
//! The decisions inside are pure helpers ([`disconnected_drive`],
//! [`volume_name`], [`opener_command`]) with tests.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use freecut_project::export::{
    Container, FolderStatus, output_path, resolve_conflict, sanitize_file_stem,
};

/// What the dialog knows about the destination.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Destination {
    pub folder: FolderStatus,
    /// Free bytes for this user on the folder's volume; `None` when unknown.
    pub free_bytes: Option<u64>,
    /// The volume's name, when it has one worth showing ("Card A", "E:").
    pub volume: Option<String>,
    /// `folder/<name>.mp4` exists: the dialog asks Replace or Keep both.
    pub target_exists: bool,
    /// Where Keep both would save (`name (2).mp4`, …).
    pub keep_both: Option<PathBuf>,
}

/// Checks `folder` and the file `name` would be saved as (EX-023: checked
/// when the dialog opens, when the folder or name changes, and again by the
/// job itself).
pub fn probe_destination(folder: &Path, name: &str, container: Container) -> Destination {
    let exists = |p: &Path| p.exists();
    let volume = volume_name(folder);
    if let Some(drive) = disconnected_drive(folder, &exists) {
        return Destination {
            folder: FolderStatus::Disconnected { drive },
            volume,
            ..Destination::default()
        };
    }
    if !folder.is_dir() {
        return Destination {
            folder: FolderStatus::Missing,
            volume,
            ..Destination::default()
        };
    }
    let status = if can_write_in(folder) {
        FolderStatus::Writable
    } else {
        FolderStatus::NotWritable
    };
    let stem = sanitize_file_stem(name);
    let (target_exists, keep_both) = if stem.is_empty() {
        (false, None)
    } else {
        let target = output_path(folder, &stem, container);
        let taken = target.exists();
        let keep = if taken {
            resolve_conflict(folder, &stem, container, &exists)
        } else {
            None
        };
        (taken, keep)
    };
    Destination {
        folder: status,
        free_bytes: fs4::available_space(folder).ok(),
        volume,
        target_exists,
        keep_both,
    }
}

/// Whether a file can be created in `folder`: creates and deletes a small
/// uniquely named probe file. Permissions, read-only volumes and OS folder
/// privacy all show up here, which permission bits alone would miss.
fn can_write_in(folder: &Path) -> bool {
    static PROBES: AtomicU64 = AtomicU64::new(0);
    let n = PROBES.fetch_add(1, Ordering::Relaxed);
    let probe = folder.join(format!(".freecut-write-check-{}-{n}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        // A leftover probe with the same name means the folder is writable.
        Err(e) => e.kind() == std::io::ErrorKind::AlreadyExists,
    }
}

/// The name of the external drive `path` is on, when that drive is not
/// connected: `/Volumes/<name>` (macOS), `/media/<user>/<name>` and
/// `/run/media/<user>/<name>` (Linux), or a missing Windows drive (`E:`).
/// `exists` is the file system's answer.
pub fn disconnected_drive(path: &Path, exists: &dyn Fn(&Path) -> bool) -> Option<String> {
    let (mount, name) = mount_point(path)?;
    (!exists(&mount)).then_some(name)
}

/// The volume name worth showing for `path` (the mount point's name for
/// external drives, the drive letter on Windows); `None` for the system
/// volume.
pub fn volume_name(path: &Path) -> Option<String> {
    mount_point(path).map(|(_, name)| name)
}

/// The external mount point `path` is under, and its display name.
fn mount_point(path: &Path) -> Option<(PathBuf, String)> {
    let mut components = path.components();
    let first = components.next()?;
    if let Component::Prefix(prefix) = first {
        let drive = prefix.as_os_str().to_string_lossy().into_owned();
        let mut root = PathBuf::from(prefix.as_os_str());
        root.push(std::path::MAIN_SEPARATOR_STR);
        return Some((root, drive));
    }
    if first != Component::RootDir {
        return None;
    }
    let names: Vec<&str> = components
        .take(4)
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let take = match names.as_slice() {
        ["Volumes", _, ..] => 2,
        ["media", _, _, ..] => 3,
        ["run", "media", _, _, ..] => 4,
        _ => return None,
    };
    let mut mount = PathBuf::from("/");
    for n in &names[..take] {
        mount.push(n);
    }
    Some((mount, names[take - 1].to_owned()))
}

/// Whether this FFmpeg build can write the MVP format: libx264 video and
/// native AAC audio (EX-029).
pub fn encoder_available() -> bool {
    freecut_media::init().is_ok_and(|caps| caps.has_encoder("libx264") && caps.has_encoder("aac"))
}

/// What the result view's buttons do (EX-039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAction {
    /// Opens the file with the OS default player.
    Open,
    /// Shows the file selected in Finder, Explorer or the file manager.
    Reveal,
}

/// The OS of the opener commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    /// Linux and the BSDs: `xdg-open`.
    Other,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// The program and arguments that perform `action` on `path` on `os`. The
/// path is always passed as one argument, never through a shell.
pub fn opener_command(os: Os, action: OpenAction, path: &Path) -> (&'static str, Vec<OsString>) {
    match (os, action) {
        (Os::Mac, OpenAction::Open) => ("open", vec![path.into()]),
        (Os::Mac, OpenAction::Reveal) => ("open", vec!["-R".into(), path.into()]),
        (Os::Windows, OpenAction::Open) => ("explorer", vec![path.into()]),
        (Os::Windows, OpenAction::Reveal) => {
            let mut arg = OsString::from("/select,");
            arg.push(path);
            ("explorer", vec![arg])
        }
        (Os::Other, OpenAction::Open) => ("xdg-open", vec![path.into()]),
        // xdg-open has no "select"; open the folder that holds the file.
        (Os::Other, OpenAction::Reveal) => (
            "xdg-open",
            vec![path.parent().unwrap_or(Path::new("/")).into()],
        ),
    }
}

/// Blocking: runs the platform opener for `path`. Errors are sentences for
/// the status line ("File no longer exists").
pub fn open_path(path: &Path, action: OpenAction) -> Result<(), String> {
    if !path.exists() {
        return Err("File no longer exists".to_owned());
    }
    let os = Os::current();
    let (program, args) = opener_command(os, action, path);
    let mut command = Command::new(program);
    command.args(args);
    if os == Os::Windows {
        // Explorer's exit code doesn't say whether it worked.
        return command
            .spawn()
            .map(drop)
            .map_err(|e| format!("Couldn't start Explorer: {e}"));
    }
    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(match action {
            OpenAction::Open => "No app is set up to play this file".to_owned(),
            OpenAction::Reveal => "Couldn't show the file in its folder".to_owned(),
        }),
        Err(e) => Err(format!("Couldn't run {program}: {e}")),
    }
}

/// The command that shows the system's microphone privacy settings
/// (AU-027: when FreeCut isn't allowed to use the microphone), if the
/// system has such a page.
pub fn microphone_settings_command(os: Os) -> Option<(&'static str, &'static [&'static str])> {
    match os {
        Os::Mac => Some((
            "open",
            &["x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"],
        )),
        // `start` needs an empty window title before the URI.
        Os::Windows => Some((
            "cmd",
            &["/C", "start", "", "ms-settings:privacy-microphone"],
        )),
        Os::Other => None,
    }
}

/// Blocking: opens the system's microphone privacy settings.
pub fn open_microphone_settings() -> Result<(), String> {
    let Some((program, args)) = microphone_settings_command(Os::current()) else {
        return Err("Allow FreeCut to use the microphone in your system's settings".to_owned());
    };
    Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("Couldn't run {program}: {e}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err("Couldn't open the privacy settings".to_owned())
            }
        })
}

/// The folder a first export suggests when the project has none
/// (EX-021): the OS Movies/Videos folder in the user's home.
pub fn default_export_folder(os: Os, home: Option<PathBuf>) -> Option<PathBuf> {
    let home = home?;
    Some(match os {
        Os::Mac => home.join("Movies"),
        Os::Windows | Os::Other => home.join("Videos"),
    })
}

/// The user's home folder, from the environment (no file I/O).
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::collections::BTreeSet;

    use super::*;

    fn exists_in(set: &[&str]) -> impl Fn(&Path) -> bool {
        let set: BTreeSet<PathBuf> = set.iter().map(PathBuf::from).collect();
        move |p: &Path| set.contains(p)
    }

    #[test]
    fn a_missing_external_drive_is_named() {
        let none = exists_in(&[]);
        assert_eq!(
            disconnected_drive(Path::new("/Volumes/Card A/Exports"), &none),
            Some("Card A".to_owned())
        );
        assert_eq!(
            disconnected_drive(Path::new("/media/dana/USB/clips"), &none),
            Some("USB".to_owned())
        );
        assert_eq!(
            disconnected_drive(Path::new("/run/media/dana/Backup"), &none),
            Some("Backup".to_owned())
        );
        let mounted = exists_in(&["/Volumes/Card A", "/media/dana/USB"]);
        assert_eq!(
            disconnected_drive(Path::new("/Volumes/Card A/Exports"), &mounted),
            None
        );
        assert_eq!(
            disconnected_drive(Path::new("/media/dana/USB/clips"), &mounted),
            None
        );
        // Folders on the system volume are never "disconnected".
        for p in ["/Users/dana/Movies", "/home/dana/Videos", "/Volumes", "rel"] {
            assert_eq!(disconnected_drive(Path::new(p), &none), None, "{p}");
        }
    }

    #[test]
    fn volume_names_come_from_the_mount_point() {
        assert_eq!(
            volume_name(Path::new("/Volumes/Data/Export")),
            Some("Data".to_owned())
        );
        assert_eq!(volume_name(Path::new("/home/dana/Videos")), None);
    }

    #[test]
    fn the_microphone_settings_open_where_the_system_keeps_them() {
        let (program, args) = microphone_settings_command(Os::Mac).unwrap();
        assert_eq!(program, "open");
        assert!(args[0].contains("Privacy_Microphone"));
        let (program, args) = microphone_settings_command(Os::Windows).unwrap();
        assert_eq!(program, "cmd");
        assert_eq!(args.last(), Some(&"ms-settings:privacy-microphone"));
        assert_eq!(microphone_settings_command(Os::Other), None);
    }

    #[test]
    fn opener_commands_pass_the_path_as_one_argument() {
        let p = Path::new("/tmp/My clip; rm -rf.mp4");
        assert_eq!(
            opener_command(Os::Mac, OpenAction::Open, p),
            ("open", vec![OsString::from(p)])
        );
        assert_eq!(
            opener_command(Os::Mac, OpenAction::Reveal, p),
            ("open", vec![OsString::from("-R"), OsString::from(p)])
        );
        assert_eq!(
            opener_command(Os::Windows, OpenAction::Reveal, Path::new("C:\\a b.mp4")),
            ("explorer", vec![OsString::from("/select,C:\\a b.mp4")])
        );
        assert_eq!(
            opener_command(Os::Windows, OpenAction::Open, Path::new("C:\\a.mp4")),
            ("explorer", vec![OsString::from("C:\\a.mp4")])
        );
        assert_eq!(
            opener_command(Os::Other, OpenAction::Open, p),
            ("xdg-open", vec![OsString::from(p)])
        );
        assert_eq!(
            opener_command(Os::Other, OpenAction::Reveal, p),
            ("xdg-open", vec![OsString::from("/tmp")])
        );
    }

    #[test]
    fn first_exports_suggest_the_os_video_folder() {
        let home = Some(PathBuf::from("/Users/dana"));
        assert_eq!(
            default_export_folder(Os::Mac, home.clone()),
            Some(PathBuf::from("/Users/dana/Movies"))
        );
        assert_eq!(
            default_export_folder(Os::Other, home),
            Some(PathBuf::from("/Users/dana/Videos"))
        );
        assert_eq!(default_export_folder(Os::Windows, None), None);
    }

    #[test]
    fn the_destination_probe_reads_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let d = probe_destination(dir.path(), "clip", Container::Mp4);
        assert_eq!(d.folder, FolderStatus::Writable);
        assert!(d.free_bytes.is_some_and(|b| b > 0));
        assert!(!d.target_exists);
        assert_eq!(d.keep_both, None);
        // The probe leaves nothing behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

        std::fs::write(dir.path().join("clip.mp4"), b"old").unwrap();
        let d = probe_destination(dir.path(), "clip", Container::Mp4);
        assert!(d.target_exists);
        assert_eq!(d.keep_both, Some(dir.path().join("clip (2).mp4")));

        let gone = probe_destination(&dir.path().join("nope"), "clip", Container::Mp4);
        assert_eq!(gone.folder, FolderStatus::Missing);
        assert_eq!(gone.free_bytes, None);
    }

    #[test]
    fn opening_a_deleted_file_says_so() {
        assert_eq!(
            open_path(Path::new("/definitely/not/here.mp4"), OpenAction::Open),
            Err("File no longer exists".to_owned())
        );
    }
}
