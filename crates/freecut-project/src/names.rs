//! Names for the files this crate keeps next to a project file: the save's
//! temp file, the one-time `.bak` copy and the `.lock` sidecar.
//!
//! Each adds a few bytes to the project's own file name, which may already
//! be close to the file-system limit (about 80 CJK characters are enough).
//! [`sibling_name`] shortens the name when needed instead of letting every
//! save or lock fail with "file name too long".

use std::ffi::{OsStr, OsString};

/// The longest file name common file systems accept: 255 bytes on ext4 and
/// APFS, 255 UTF-16 units on NTFS and HFS+. The length is measured in UTF-8
/// bytes, which is never less than the number of UTF-16 units.
pub(crate) const MAX_NAME_BYTES: usize = 255;

/// `prefix`, `name`, `suffix`: the name of a file kept next to the file
/// `name`. When that is longer than [`MAX_NAME_BYTES`], the middle is
/// shortened to the start of `name`, then `~` and a hash of all of `name`.
/// The result is the same on every call, so a lock or backup is found
/// again, and two long names that start alike still get different names.
pub(crate) fn sibling_name(prefix: &str, name: &OsStr, suffix: &str) -> OsString {
    let mut out = OsString::from(prefix);
    if prefix.len() + name.len() + suffix.len() <= MAX_NAME_BYTES {
        out.push(name);
    } else {
        let tag = format!("~{:016x}", fnv1a(name.as_encoded_bytes()));
        let budget = MAX_NAME_BYTES.saturating_sub(prefix.len() + tag.len() + suffix.len());
        // Lossy only for names that aren't Unicode; the hash still tells
        // them apart.
        let text = name.to_string_lossy();
        let mut cut = budget.min(text.len());
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        out.push(&text[..cut]);
        out.push(tag);
    }
    out.push(suffix);
    out
}

/// 64-bit FNV-1a: stable across runs and platforms, for names only.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_are_kept_whole() {
        assert_eq!(
            sibling_name(".", OsStr::new("Trip.freecut"), ".7-0.tmp"),
            OsString::from(".Trip.freecut.7-0.tmp")
        );
        assert_eq!(
            sibling_name("", OsStr::new("Trip.freecut"), ".bak"),
            OsString::from("Trip.freecut.bak")
        );
    }

    #[test]
    fn long_names_are_shortened_to_fit_stably_and_distinctly() {
        // 82 three-byte characters and the extension: 254 bytes, a legal
        // name that leaves no room for any suffix.
        let long = format!("{}.freecut", "旅".repeat(82));
        assert!(long.len() <= MAX_NAME_BYTES);
        for suffix in [".bak", ".lock", ".4294967295-18446744073709551615.tmp"] {
            let name = sibling_name(".", OsStr::new(&long), suffix);
            let text = name.to_str().unwrap();
            assert!(text.len() <= MAX_NAME_BYTES, "{} bytes", text.len());
            assert!(
                text.starts_with(".旅旅") && text.ends_with(suffix),
                "{text}"
            );
            assert_eq!(name, sibling_name(".", OsStr::new(&long), suffix));
        }
        let other = format!("{}.freecut", "旅".repeat(81) + "x");
        assert_ne!(
            sibling_name("", OsStr::new(&long), ".bak"),
            sibling_name("", OsStr::new(&other), ".bak")
        );
    }

    #[test]
    fn fnv1a_matches_the_reference_values() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
