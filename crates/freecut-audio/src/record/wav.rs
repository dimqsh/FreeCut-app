//! FreeCut's own 24-bit PCM WAV writer (AU-027, architecture C53).
//!
//! The file is written so that a crash at any moment leaves something a
//! player can open:
//!
//! - the header always has room for both forms: a 28-byte `JUNK` chunk
//!   right after `WAVE` becomes the `ds64` chunk when the file switches to
//!   RF64 (EBU Tech 3306), so the switch never moves the audio;
//! - [`Writer::refresh_header`] (the recorder calls it every second) first
//!   flushes and syncs the audio, then writes the sizes of what is on disk,
//!   so the header never claims more than the file holds;
//! - a `<file>.recording` marker sits beside the file from creation until
//!   [`Writer::finish`] (PM-043), so a startup scan can find interrupted
//!   takes and [`repair`] them.
//!
//! The layout (80-byte header, then the samples):
//!
//! ```text
//!  0 "RIFF" | "RF64"   4 riff size (0xFFFF_FFFF in RF64)   8 "WAVE"
//! 12 "JUNK" | "ds64"  16 28   20 riff size u64, data size u64,
//!                              sample count u64, table length u32 (0)
//! 48 "fmt "  52 16   56 PCM (1), channels, rate, byte rate, block align, 24
//! 72 "data"  76 data size (0xFFFF_FFFF in RF64)   80 samples (3 bytes LE)
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Bytes before the first sample.
pub const HEADER_LEN: u64 = 80;
/// Bits per sample of every file this writer makes.
pub const BITS: u16 = 24;
/// The largest RIFF size a plain WAV can state; past it the file becomes
/// RF64 (the default [`Writer::with_rf64_threshold`]).
pub const RIFF_LIMIT: u64 = u32::MAX as u64;
/// What the marker's name adds to the file's name.
pub const MARKER_SUFFIX: &str = ".recording";

/// Size of the `JUNK`/`ds64` payload.
const DS64_LEN: u32 = 28;
const FMT_PCM: u16 = 1;
/// Size fields of an RF64 file that live in `ds64` instead.
const RF64_SIZE: u32 = u32::MAX;

/// Sample rate and channel count of a file (the sample size is 24 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavSpec {
    pub rate: u32,
    pub channels: u16,
}

impl WavSpec {
    /// Bytes per frame.
    pub fn block_align(self) -> u16 {
        self.channels.saturating_mul(3)
    }
}

/// Which header form a file has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavForm {
    Riff,
    Rf64,
}

/// What a WAV file's header says (see [`read_info`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub form: WavForm,
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    /// Where the samples start.
    pub data_offset: u64,
    /// The data size the header states.
    pub data_bytes: u64,
}

impl WavInfo {
    /// Whole frames the header states.
    pub fn frames(&self) -> u64 {
        let align = u64::from(self.channels) * u64::from(self.bits / 8);
        self.data_bytes.checked_div(align).unwrap_or(0)
    }
}

/// What [`Writer::finish`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavSummary {
    pub frames: u64,
    pub form: WavForm,
    /// The file's length.
    pub bytes: u64,
}

/// `<path>.recording`: the marker beside a file being written.
pub fn marker_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(MARKER_SUFFIX);
    PathBuf::from(name)
}

/// Streams 24-bit samples into a new WAV file. See the module docs for how
/// it stays readable after a crash.
#[derive(Debug)]
pub struct Writer {
    out: BufWriter<File>,
    path: PathBuf,
    spec: WavSpec,
    data_bytes: u64,
    rf64_threshold: u64,
    form: WavForm,
    /// Frames the header on disk states.
    stated_frames: u64,
    bytes: Vec<u8>,
}

impl Writer {
    /// Creates `path` (never replacing an existing file) and its marker,
    /// with a header stating no samples.
    pub fn create(path: &Path, spec: WavSpec) -> io::Result<Writer> {
        if spec.channels == 0 || spec.rate == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a WAV file needs a rate and at least one channel",
            ));
        }
        let marker = marker_path(path);
        File::create(&marker)?;
        let file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(e) => {
                let _ = fs::remove_file(&marker);
                return Err(e);
            }
        };
        let mut writer = Writer {
            out: BufWriter::with_capacity(1 << 16, file),
            path: path.to_owned(),
            spec,
            data_bytes: 0,
            rf64_threshold: RIFF_LIMIT,
            form: WavForm::Riff,
            stated_frames: 0,
            bytes: Vec::new(),
        };
        let (header, _) = header(spec, 0, false, RIFF_LIMIT);
        writer.out.write_all(&header)?;
        writer.out.flush()?;
        Ok(writer)
    }

    /// Switches to RF64 once the RIFF size would pass `bytes` instead of
    /// 4 GiB. Tests use a small value; real takes keep the default.
    pub fn with_rf64_threshold(mut self, bytes: u64) -> Writer {
        self.rf64_threshold = bytes.min(RIFF_LIMIT);
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn spec(&self) -> WavSpec {
        self.spec
    }

    /// Frames written so far.
    pub fn frames(&self) -> u64 {
        self.data_bytes / u64::from(self.spec.block_align())
    }

    /// Frames the header on disk states (as of the last refresh).
    pub fn stated_frames(&self) -> u64 {
        self.stated_frames
    }

    /// The header form on disk.
    pub fn form(&self) -> WavForm {
        self.form
    }

    /// Appends interleaved samples, each a signed 24-bit value (clamped).
    /// `samples.len()` must be a whole number of frames.
    pub fn write_samples(&mut self, samples: &[i32]) -> io::Result<()> {
        let channels = usize::from(self.spec.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "samples must be whole frames",
            ));
        }
        self.bytes.clear();
        for &s in samples {
            let s = s.clamp(-(1 << 23), (1 << 23) - 1);
            self.bytes.extend_from_slice(&s.to_le_bytes()[..3]);
        }
        self.out.write_all(&self.bytes)?;
        self.data_bytes += self.bytes.len() as u64;
        Ok(())
    }

    /// Makes everything written so far durable and readable: flushes and
    /// syncs the samples, then rewrites the header with their size,
    /// switching to RF64 when the size needs it.
    pub fn refresh_header(&mut self) -> io::Result<()> {
        self.write_header(false)?;
        Ok(())
    }

    fn write_header(&mut self, last: bool) -> io::Result<()> {
        self.out.flush()?;
        let file = self.out.get_mut();
        file.sync_data()?;
        let threshold = if self.form == WavForm::Rf64 {
            0
        } else {
            self.rf64_threshold
        };
        let (header, form) = header(self.spec, self.data_bytes, last, threshold);
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.seek(SeekFrom::End(0))?;
        if last {
            file.sync_all()?;
        }
        self.form = form;
        self.stated_frames = self.frames();
        Ok(())
    }

    /// Completes the file (pad byte, final header, sync) and removes the
    /// marker.
    pub fn finish(mut self) -> io::Result<WavSummary> {
        if self.data_bytes % 2 == 1 {
            self.out.write_all(&[0])?;
        }
        self.write_header(true)?;
        let bytes = self.out.get_ref().metadata()?.len();
        if let Err(e) = fs::remove_file(marker_path(&self.path))
            && e.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), "can't remove the recording marker: {e}");
        }
        Ok(WavSummary {
            frames: self.frames(),
            form: self.form,
            bytes,
        })
    }

    /// Closes and deletes the file and its marker (a take cancelled before
    /// its start position).
    pub fn discard(self) -> io::Result<()> {
        let path = self.path.clone();
        drop(self);
        fs::remove_file(&path)?;
        match fs::remove_file(marker_path(&path)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// The header for `data_bytes` of samples. `last` adds the pad byte of an
/// odd-sized data chunk to the RIFF size. The form is RF64 when the RIFF
/// size passes `threshold`.
fn header(spec: WavSpec, data_bytes: u64, last: bool, threshold: u64) -> ([u8; 80], WavForm) {
    let pad = u64::from(last && data_bytes % 2 == 1);
    let riff = HEADER_LEN - 8 + data_bytes + pad;
    let form = if riff > threshold {
        WavForm::Rf64
    } else {
        WavForm::Riff
    };
    let align = spec.block_align();
    let mut h = [0u8; 80];
    let mut at = 0;
    let mut put = |bytes: &[u8]| {
        h[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    };
    let small = |v: u64| u32::try_from(v).unwrap_or(RF64_SIZE);
    match form {
        WavForm::Riff => {
            put(b"RIFF");
            put(&small(riff).to_le_bytes());
            put(b"WAVE");
            put(b"JUNK");
            put(&DS64_LEN.to_le_bytes());
            put(&[0; DS64_LEN as usize]);
        }
        WavForm::Rf64 => {
            put(b"RF64");
            put(&RF64_SIZE.to_le_bytes());
            put(b"WAVE");
            put(b"ds64");
            put(&DS64_LEN.to_le_bytes());
            put(&riff.to_le_bytes());
            put(&data_bytes.to_le_bytes());
            let frames = data_bytes / u64::from(align.max(1));
            put(&frames.to_le_bytes());
            put(&0u32.to_le_bytes());
        }
    }
    put(b"fmt ");
    put(&16u32.to_le_bytes());
    put(&FMT_PCM.to_le_bytes());
    put(&spec.channels.to_le_bytes());
    put(&spec.rate.to_le_bytes());
    put(&spec.rate.saturating_mul(u32::from(align)).to_le_bytes());
    put(&align.to_le_bytes());
    put(&BITS.to_le_bytes());
    put(b"data");
    let data = match form {
        WavForm::Riff => small(data_bytes),
        WavForm::Rf64 => RF64_SIZE,
    };
    put(&data.to_le_bytes());
    (h, form)
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn u16_at(b: &[u8], at: usize) -> io::Result<u16> {
    b.get(at..at + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| invalid("truncated WAV header"))
}

fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| invalid("truncated WAV header"))
}

fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
    b.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| invalid("truncated WAV header"))
}

/// Parses the header of a RIFF or RF64 WAVE file: walks the chunks up to
/// `data`, reading `fmt ` and, for RF64, the sizes in `ds64`. Only the
/// first 4 KiB are read.
pub fn read_info(path: &Path) -> io::Result<WavInfo> {
    let mut head = Vec::with_capacity(4096);
    File::open(path)?.take(4096).read_to_end(&mut head)?;
    parse_header(&head)
}

/// [`read_info`] on bytes already read.
pub fn parse_header(b: &[u8]) -> io::Result<WavInfo> {
    let form = match b.get(..4) {
        Some(b"RIFF") => WavForm::Riff,
        Some(b"RF64") => WavForm::Rf64,
        _ => return Err(invalid("not a RIFF or RF64 file")),
    };
    if b.get(8..12) != Some(b"WAVE") {
        return Err(invalid("not a WAVE file"));
    }
    let mut at = 12usize;
    let mut ds64_data = None;
    let mut fmt = None;
    loop {
        let id = b
            .get(at..at + 4)
            .ok_or_else(|| invalid("no data chunk in the header"))?;
        let size = u32_at(b, at + 4)?;
        let body = at + 8;
        match id {
            b"ds64" => ds64_data = Some(u64_at(b, body + 8)?),
            b"fmt " => {
                fmt = Some((
                    u16_at(b, body)?,
                    u16_at(b, body + 2)?,
                    u32_at(b, body + 4)?,
                    u16_at(b, body + 14)?,
                ));
            }
            b"data" => {
                let (tag, channels, rate, bits) = fmt.ok_or_else(|| invalid("no fmt chunk"))?;
                // PCM, or WAVE_FORMAT_EXTENSIBLE (its sub-format is not checked).
                if tag != FMT_PCM && tag != 0xFFFE {
                    return Err(invalid("not PCM"));
                }
                let data_bytes = match (form, ds64_data) {
                    (WavForm::Rf64, Some(d)) if size == RF64_SIZE => d,
                    (WavForm::Rf64, None) => return Err(invalid("RF64 without ds64")),
                    _ => u64::from(size),
                };
                return Ok(WavInfo {
                    form,
                    rate,
                    channels,
                    bits,
                    data_offset: body as u64,
                    data_bytes,
                });
            }
            _ => {}
        }
        let size = usize::try_from(size).map_err(|_| invalid("chunk too large"))?;
        at = body
            .checked_add(size + size % 2)
            .ok_or_else(|| invalid("chunk too large"))?;
    }
}

/// Fixes the header of a file whose writer never finished (a crash): its
/// sizes are set to every whole frame on disk (a partial last frame is cut
/// off), in RF64 form when needed, and the marker is removed. Files this
/// writer didn't make are refused unless their data chunk is the last one.
pub fn repair(path: &Path) -> io::Result<WavInfo> {
    let info = read_info(path)?;
    if info.bits != BITS || info.data_offset != HEADER_LEN {
        return Err(invalid("not a FreeCut recording"));
    }
    let spec = WavSpec {
        rate: info.rate,
        channels: info.channels,
    };
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    let align = u64::from(spec.block_align());
    let data_bytes = len.saturating_sub(HEADER_LEN) / align * align;
    file.set_len(HEADER_LEN + data_bytes)?;
    let mut file = file;
    if data_bytes % 2 == 1 {
        file.seek(SeekFrom::End(0))?;
        file.write_all(&[0])?;
    }
    let (header, _) = header(spec, data_bytes, true, RIFF_LIMIT);
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    file.sync_all()?;
    match fs::remove_file(marker_path(path)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    read_info(path)
}

/// The samples of a file this module wrote, as 24-bit values (tests and
/// debug dumps; reads the whole data chunk the header states).
pub fn read_samples(path: &Path) -> io::Result<(WavInfo, Vec<i32>)> {
    let info = read_info(path)?;
    if info.bits != BITS {
        return Err(invalid("not 24-bit"));
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(info.data_offset))?;
    let len = usize::try_from(info.frames() * u64::from(info.channels) * 3)
        .map_err(|_| invalid("too large"))?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)?;
    let samples = bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[a, b, c]| i32::from_le_bytes([0, a, b, c]) >> 8)
        .collect();
    Ok((info, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: WavSpec = WavSpec {
        rate: 8_000,
        channels: 1,
    };
    const STEREO: WavSpec = WavSpec {
        rate: 48_000,
        channels: 2,
    };

    #[test]
    fn a_finished_file_reads_back_sample_exact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.wav");
        let mut w = Writer::create(&path, STEREO).unwrap();
        assert!(marker_path(&path).exists());
        let samples: Vec<i32> = (0..2_000)
            .map(|i| (i - 1_000) * 4_001)
            .chain([(1 << 23) - 1, -(1 << 23)])
            .collect();
        w.write_samples(&samples).unwrap();
        let summary = w.finish().unwrap();
        assert_eq!(summary.frames, 1_001);
        assert_eq!(summary.form, WavForm::Riff);
        assert_eq!(summary.bytes, HEADER_LEN + 1_001 * 6);
        assert!(!marker_path(&path).exists());
        let (info, back) = read_samples(&path).unwrap();
        assert_eq!(back, samples);
        assert_eq!(
            (info.rate, info.channels, info.bits, info.form),
            (48_000, 2, 24, WavForm::Riff)
        );
        // RIFF size = file length − 8.
        let bytes = fs::read(&path).unwrap();
        assert_eq!(
            u64::from(u32_at(&bytes, 4).unwrap()),
            bytes.len() as u64 - 8
        );
        // Byte rate and block align.
        assert_eq!(u32_at(&bytes, 64).unwrap(), 48_000 * 6);
        assert_eq!(u16_at(&bytes, 68).unwrap(), 6);
    }

    #[test]
    fn out_of_range_values_are_clamped_and_odd_data_is_padded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("odd.wav");
        let mut w = Writer::create(&path, MONO).unwrap();
        w.write_samples(&[1 << 24, -(1 << 24), 5]).unwrap();
        let summary = w.finish().unwrap();
        // 9 data bytes + 1 pad byte.
        assert_eq!(summary.bytes, HEADER_LEN + 10);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(u32_at(&bytes, 4).unwrap(), 72 + 10);
        assert_eq!(u32_at(&bytes, 76).unwrap(), 9);
        let (_, back) = read_samples(&path).unwrap();
        assert_eq!(back, [(1 << 23) - 1, -(1 << 23), 5]);
    }

    #[test]
    fn partial_frames_are_refused_and_existing_files_are_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.wav");
        let mut w = Writer::create(&path, STEREO).unwrap();
        assert!(w.write_samples(&[1, 2, 3]).is_err());
        w.finish().unwrap();
        let err = Writer::create(&path, STEREO).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // The failed create left no marker behind.
        assert!(!marker_path(&path).exists());
    }

    #[test]
    fn the_file_switches_to_rf64_past_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.wav");
        let mut w = Writer::create(&path, MONO)
            .unwrap()
            .with_rf64_threshold(1_000);
        let block: Vec<i32> = (0..100).collect();
        // 300 data bytes: RIFF size 372, still RIFF.
        w.write_samples(&block).unwrap();
        w.refresh_header().unwrap();
        assert_eq!(w.form(), WavForm::Riff);
        assert_eq!(read_info(&path).unwrap().form, WavForm::Riff);
        // 1 200 data bytes: RIFF size 1 272 > 1 000.
        for _ in 0..3 {
            w.write_samples(&block).unwrap();
        }
        w.refresh_header().unwrap();
        assert_eq!(w.form(), WavForm::Rf64);
        let info = read_info(&path).unwrap();
        assert_eq!((info.form, info.frames()), (WavForm::Rf64, 400));
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RF64");
        assert_eq!(u32_at(&bytes, 4).unwrap(), u32::MAX);
        assert_eq!(&bytes[12..16], b"ds64");
        assert_eq!(u64_at(&bytes, 20).unwrap(), 72 + 1_200);
        assert_eq!(u64_at(&bytes, 28).unwrap(), 1_200);
        assert_eq!(u64_at(&bytes, 36).unwrap(), 400);
        assert_eq!(u32_at(&bytes, 76).unwrap(), u32::MAX);
        // The samples didn't move.
        w.write_samples(&block[..1]).unwrap();
        let summary = w.finish().unwrap();
        assert_eq!((summary.form, summary.frames), (WavForm::Rf64, 401));
        let (_, back) = read_samples(&path).unwrap();
        let want: Vec<i32> = block.iter().cycle().take(400).copied().chain([0]).collect();
        assert_eq!(back, want);
        // Odd data: the pad byte counts in the RIFF size.
        let bytes = fs::read(&path).unwrap();
        assert_eq!(u64_at(&bytes, 20).unwrap(), bytes.len() as u64 - 8);
    }

    #[test]
    fn a_file_cut_off_mid_write_reads_up_to_the_last_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.wav");
        let mut w = Writer::create(&path, STEREO).unwrap();
        let block: Vec<i32> = (0..960).collect();
        w.write_samples(&block).unwrap();
        w.refresh_header().unwrap();
        assert_eq!(w.stated_frames(), 480);
        w.write_samples(&block).unwrap();
        // The process dies here: the buffered samples reach the file (the
        // OS keeps what was written), the header is not updated.
        drop(w);
        assert!(marker_path(&path).exists());
        let (info, back) = read_samples(&path).unwrap();
        assert_eq!(info.frames(), 480);
        assert_eq!(back, block);
        // Half a frame more on disk, as if the crash hit mid-write.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[1, 2, 3]).unwrap();
        drop(f);
        // Repair finds every whole frame and removes the marker.
        let info = repair(&path).unwrap();
        assert_eq!((info.frames(), info.form), (960, WavForm::Riff));
        assert!(!marker_path(&path).exists());
        let (_, back) = read_samples(&path).unwrap();
        assert_eq!(back.len(), 1_920);
        assert_eq!(&back[960..], &block[..]);
        assert_eq!(fs::metadata(&path).unwrap().len(), HEADER_LEN + 960 * 6);
    }

    #[test]
    fn a_crash_before_any_refresh_leaves_an_empty_but_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("early.wav");
        let mut w = Writer::create(&path, MONO).unwrap();
        w.write_samples(&[7; 30]).unwrap();
        drop(w);
        let info = read_info(&path).unwrap();
        assert_eq!((info.frames(), info.rate, info.channels), (0, 8_000, 1));
        assert_eq!(repair(&path).unwrap().frames(), 30);
    }

    #[test]
    fn discard_removes_the_file_and_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.wav");
        let mut w = Writer::create(&path, MONO).unwrap();
        w.write_samples(&[1, 2]).unwrap();
        w.discard().unwrap();
        assert!(!path.exists());
        assert!(!marker_path(&path).exists());
    }

    #[test]
    fn the_parser_skips_unknown_chunks_and_rejects_non_wav() {
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        b.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        b.extend_from_slice(b"fmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\0\0\0\0\x04\0\x10\0");
        b.extend_from_slice(b"data\x08\0\0\0");
        let info = parse_header(&b).unwrap();
        assert_eq!(
            (info.rate, info.channels, info.bits, info.data_bytes),
            (44_100, 2, 16, 8)
        );
        assert_eq!(info.data_offset, b.len() as u64);
        assert_eq!(info.frames(), 2);
        assert!(parse_header(b"RIFX\0\0\0\0WAVE").is_err());
        assert!(parse_header(b"RIFF\0\0\0\0AVI ").is_err());
        assert!(parse_header(b"RIFF\0\0\0\0WAVEfmt ").is_err());
        assert_eq!(
            marker_path(Path::new("/a/b.wav")),
            Path::new("/a/b.wav.recording")
        );
    }
}
