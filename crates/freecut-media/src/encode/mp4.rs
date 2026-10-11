//! Reading the top-level structure of an ISO BMFF (MP4/MOV) file, for
//! `verify_output`: box order (fast start puts `moov` before `mdat`) and the
//! `colr`/`nclx` colour description.

use std::io::{self, Read, Seek, SeekFrom};

/// The largest `moov` read into memory to look for `colr`. A `moov` this
/// big only occurs for files many hours long; the colour check is skipped.
const MAX_MOOV_READ: u64 = 64 * 1024 * 1024;

/// The `colr` box's `nclx` colour description (ISO/IEC 23001-8 code
/// points: 1 = BT.709).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Nclx {
    /// Colour primaries code point.
    pub primaries: u16,
    /// Transfer characteristics code point.
    pub transfer: u16,
    /// Matrix coefficients code point.
    pub matrix: u16,
    /// Full range (`true`) or limited range (`false`).
    pub full_range: bool,
}

/// One top-level box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TopBox {
    pub(crate) kind: [u8; 4],
    pub(crate) offset: u64,
    /// Whole box size including the header.
    pub(crate) size: u64,
    pub(crate) header: u64,
}

/// The top-level boxes of an ISO BMFF stream, in file order. Stops at the
/// first malformed header (a non-MP4 file yields garbage or nothing; callers
/// check for `ftyp` first).
pub(crate) fn top_level_boxes<R: Read + Seek>(r: &mut R) -> io::Result<Vec<TopBox>> {
    let len = r.seek(SeekFrom::End(0))?;
    let mut boxes = Vec::new();
    let mut offset = 0u64;
    while offset.saturating_add(8) <= len {
        r.seek(SeekFrom::Start(offset))?;
        let mut head = [0u8; 8];
        r.read_exact(&mut head)?;
        let size32 = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        let kind = [head[4], head[5], head[6], head[7]];
        let (size, header) = match size32 {
            0 => (len - offset, 8),
            1 => {
                let mut large = [0u8; 8];
                r.read_exact(&mut large)?;
                (u64::from_be_bytes(large), 16)
            }
            s => (u64::from(s), 8),
        };
        if size < header || offset.saturating_add(size) > len {
            break;
        }
        boxes.push(TopBox {
            kind,
            offset,
            size,
            header,
        });
        offset += size;
    }
    Ok(boxes)
}

/// The first `colr` box of type `nclx` inside `moov`, if any.
pub(crate) fn find_nclx<R: Read + Seek>(r: &mut R, moov: &TopBox) -> io::Result<Option<Nclx>> {
    let body = moov.size - moov.header;
    if body > MAX_MOOV_READ {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(moov.offset + moov.header))?;
    let mut buf = vec![0u8; usize::try_from(body).unwrap_or(0)];
    r.read_exact(&mut buf)?;
    Ok(nclx_in(&buf))
}

/// Scans box payload bytes for `colr` + `nclx` + three code points + the
/// range flag.
fn nclx_in(buf: &[u8]) -> Option<Nclx> {
    buf.windows(15).find_map(|w| {
        (&w[0..4] == b"colr" && &w[4..8] == b"nclx").then(|| Nclx {
            primaries: u16::from_be_bytes([w[8], w[9]]),
            transfer: u16::from_be_bytes([w[10], w[11]]),
            matrix: u16::from_be_bytes([w[12], w[13]]),
            full_range: w[14] & 0x80 != 0,
        })
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use std::io::Cursor;

    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn reads_box_order_and_colour() {
        let mut colr = b"nclx".to_vec();
        colr.extend_from_slice(&[0, 1, 0, 1, 0, 1, 0x00]);
        let moov = boxed(b"moov", &boxed(b"trak", &boxed(b"colr", &colr)));
        let mut file = boxed(b"ftyp", b"isom");
        file.extend(&moov);
        file.extend(boxed(b"mdat", &[0; 32]));
        let mut c = Cursor::new(file);
        let boxes = top_level_boxes(&mut c).unwrap();
        let kinds: Vec<_> = boxes.iter().map(|b| &b.kind).collect();
        assert_eq!(kinds, [b"ftyp", b"moov", b"mdat"]);
        let nclx = find_nclx(&mut c, &boxes[1]).unwrap().unwrap();
        assert_eq!(
            nclx,
            Nclx {
                primaries: 1,
                transfer: 1,
                matrix: 1,
                full_range: false
            }
        );
    }

    #[test]
    fn large_size_and_to_end_boxes() {
        let mut file = boxed(b"ftyp", b"isom");
        // A 64-bit size box: size field 1, then the real size.
        file.extend(1u32.to_be_bytes());
        file.extend(b"mdat");
        file.extend(24u64.to_be_bytes());
        file.extend([0u8; 8]);
        // A size-0 box runs to the end of the file.
        file.extend(0u32.to_be_bytes());
        file.extend(b"free");
        file.extend([0u8; 5]);
        let boxes = top_level_boxes(&mut Cursor::new(file)).unwrap();
        let kinds: Vec<_> = boxes.iter().map(|b| &b.kind).collect();
        assert_eq!(kinds, [b"ftyp", b"mdat", b"free"]);
        assert_eq!(boxes[1].size, 24);
        assert_eq!(boxes[2].size, 13);
    }

    #[test]
    fn truncated_files_stop_cleanly() {
        let mut file = boxed(b"ftyp", b"isom");
        file.extend(100u32.to_be_bytes());
        file.extend(b"mdat");
        let boxes = top_level_boxes(&mut Cursor::new(file)).unwrap();
        assert_eq!(boxes.len(), 1);
        assert!(
            top_level_boxes(&mut Cursor::new(Vec::new()))
                .unwrap()
                .is_empty()
        );
        assert_eq!(nclx_in(b"colrnclx"), None);
    }
}
