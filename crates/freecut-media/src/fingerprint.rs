//! The media content key (PM-038).
//!
//! A [`Fingerprint`] is the file size, an xxh3-64 hash of the first and last
//! [`FINGERPRINT_SPAN`] bytes, and the probed duration. Reading only the two
//! ends keeps it fast on slow drives; together with the size and duration it
//! identifies content well enough for cache keys and relink checks.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use freecut_timeline::model::Fingerprint;
use freecut_timeline::time::Tick;

/// Bytes hashed at each end of the file (1 MiB, PM-038).
pub const FINGERPRINT_SPAN: u64 = 1024 * 1024;

/// Fingerprints the file at `path`.
///
/// Hashes `head ‖ tail`: the first and last [`FINGERPRINT_SPAN`] bytes, or the
/// whole file once when it is at most twice that long (so the two ends never
/// overlap). `duration` is the probed media duration, stored as is.
pub fn fingerprint(path: &Path, duration: Tick) -> io::Result<Fingerprint> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut bytes = Vec::new();
    let size = if len <= 2 * FINGERPRINT_SPAN {
        // Small file: hash all of it, and report the length actually read in
        // case the file changed since `metadata`.
        file.read_to_end(&mut bytes)?;
        bytes.len() as u64
    } else {
        // FINGERPRINT_SPAN is 1 MiB, so this cast cannot truncate.
        let span = FINGERPRINT_SPAN as usize;
        bytes.resize(2 * span, 0);
        let (head, tail) = bytes.split_at_mut(span);
        file.read_exact(head)?;
        file.seek(SeekFrom::Start(len - FINGERPRINT_SPAN))?;
        file.read_exact(tail)?;
        len
    };
    Ok(Fingerprint {
        size,
        head_tail: xxhash_rust::xxh3::xxh3_64(&bytes),
        duration,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use std::io::Write;

    fn write(dir: &tempfile::TempDir, name: &str, data: &[u8]) -> std::path::PathBuf {
        let p = dir.path().join(name);
        File::create(&p).unwrap().write_all(data).unwrap();
        p
    }

    #[test]
    fn small_files_hash_their_whole_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "a.bin", b"hello");
        let fp = fingerprint(&p, Tick::from_raw(7)).unwrap();
        assert_eq!(fp.size, 5);
        assert_eq!(fp.head_tail, xxhash_rust::xxh3::xxh3_64(b"hello"));
        assert_eq!(fp.duration, Tick::from_raw(7));
    }

    #[test]
    fn large_files_hash_head_and_tail_only() {
        let dir = tempfile::tempdir().unwrap();
        let span = FINGERPRINT_SPAN as usize;
        let mut data: Vec<u8> = (0..3 * span).map(|i| (i % 251) as u8).collect();
        let p1 = write(&dir, "a.bin", &data);
        // A change in the middle is not seen; a change at either end is.
        data[span + 10] ^= 0xff;
        let p2 = write(&dir, "b.bin", &data);
        data[3 * span - 1] ^= 0xff;
        let p3 = write(&dir, "c.bin", &data);
        let f1 = fingerprint(&p1, Tick::ZERO).unwrap();
        let f2 = fingerprint(&p2, Tick::ZERO).unwrap();
        let f3 = fingerprint(&p3, Tick::ZERO).unwrap();
        assert_eq!(f1.size, 3 * FINGERPRINT_SPAN);
        assert_eq!(f1, f2);
        assert_ne!(f1, f3);
        let mut expected = data[..span].to_vec();
        expected.extend_from_slice(&data[2 * span..]);
        assert_eq!(f3.head_tail, xxhash_rust::xxh3::xxh3_64(&expected));
    }

    #[test]
    fn missing_files_report_not_found() {
        let err = fingerprint(Path::new("/definitely/not/here.mp4"), Tick::ZERO).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
