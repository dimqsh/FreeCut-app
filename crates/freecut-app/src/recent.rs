//! Recent projects (PM-017) and their covers: `recent.json` next to
//! `settings.json`, which feeds Home's project cards (PM-001, UI-016) and
//! File › Open Recent.
//!
//! - Each entry holds the path, the name, when it was last opened or
//!   saved, the canvas, the frame rate and the timeline's length. It is
//!   updated on open and save; the newest goes to the top; at most
//!   [`MAX_ENTRIES`] are kept.
//! - The file is read once at startup on the worker pool ([`load`]), which
//!   also checks whether each project file still exists ("Not found"), and
//!   written a second after a change, atomically, on a writer thread (the
//!   workspace store's).
//! - A **cover** is the frame the Stage showed when the project was last
//!   saved, made small and written to `covers/<hash>.bmp` in the same
//!   folder, on a worker; Home reads it back on a worker too. The UI
//!   thread never touches the disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::session::Project;
use crate::workspace::WorkspaceStore;

/// The registry's file name.
pub const FILE_NAME: &str = "recent.json";
/// The most entries kept (PM-017).
pub const MAX_ENTRIES: usize = 200;
/// The entries File › Open Recent lists.
pub const MENU_ENTRIES: usize = 10;
/// A cover's widest side.
pub const COVER_WIDTH: u32 = 480;
const KEY: &str = "projects";

/// One project of the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub name: String,
    /// When it was last opened or saved, in Unix seconds.
    pub opened: u64,
    pub canvas: (u32, u32),
    /// The frame rate as num / den.
    pub rate: (u32, u32),
    /// The timeline's length in milliseconds.
    pub duration_ms: u64,
    /// The file was there when last looked at (not saved in the file).
    pub found: bool,
}

/// Now in Unix seconds.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl RecentEntry {
    /// The entry for `project`, saved at `project.file`; `None` before its
    /// first save.
    pub fn of(project: &Project, length_ms: u64, now: u64) -> Option<RecentEntry> {
        let path = project.file.clone()?;
        let s = project.doc().settings();
        Some(RecentEntry {
            path,
            name: project.name(),
            opened: now,
            canvas: (s.width, s.height),
            rate: (s.frame_rate.num(), s.frame_rate.den()),
            duration_ms: length_ms,
            found: true,
        })
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "path": self.path.to_string_lossy(),
            "name": self.name,
            "opened": self.opened,
            "canvas": [self.canvas.0, self.canvas.1],
            "rate": [self.rate.0, self.rate.1],
            "duration_ms": self.duration_ms,
        })
    }

    fn from_json(v: &Value) -> Option<RecentEntry> {
        let pair = |key: &str| -> Option<(u32, u32)> {
            let a = v.get(key)?.as_array()?;
            let n = |i: usize| u32::try_from(a.get(i)?.as_u64()?).ok();
            Some((n(0)?, n(1)?))
        };
        let path = PathBuf::from(v.get("path")?.as_str()?);
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(RecentEntry {
            name: v
                .get("name")
                .and_then(Value::as_str)
                .map_or_else(|| stem(&path), str::to_owned),
            path,
            opened: v.get("opened").and_then(Value::as_u64).unwrap_or(0),
            canvas: pair("canvas").unwrap_or((1920, 1080)),
            rate: pair("rate").unwrap_or((30, 1)),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
            found: true,
        })
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().map_or_else(
        || "Untitled".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    )
}

/// The registry file of this machine's user (no file I/O).
pub fn registry_path() -> Option<PathBuf> {
    crate::settings::settings_path().map(|p| p.with_file_name(FILE_NAME))
}

/// The covers folder next to the registry (no file I/O).
pub fn covers_dir() -> Option<PathBuf> {
    crate::settings::settings_path().and_then(|p| p.parent().map(|d| d.join("covers")))
}

/// FNV-1a, 64 bit: a hash that stays the same across builds.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The cover file of the project at `project` in `dir`.
pub fn cover_path(dir: &Path, project: &Path) -> PathBuf {
    let key = fnv1a(project.to_string_lossy().as_bytes());
    dir.join(format!("{key:016x}.bmp"))
}

/// The entries of a registry file as JSON: newest first, unreadable ones
/// left out.
pub fn entries_from(raw: &Map<String, Value>) -> Vec<RecentEntry> {
    raw.get(KEY)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(RecentEntry::from_json).collect())
        .unwrap_or_default()
}

/// Blocking: the registry at `path` and its entries, each marked found or
/// not. A missing, damaged or newer file gives no entries (and is replaced
/// at the next change).
pub fn load(path: &Path) -> (Option<Map<String, Value>>, Vec<RecentEntry>) {
    let raw = crate::workspace::load(path);
    let mut entries = raw.as_ref().map(entries_from).unwrap_or_default();
    for e in &mut entries {
        e.found = e.path.is_file();
    }
    (raw, entries)
}

/// The registry in memory and when to write it.
#[derive(Debug, Default)]
pub struct RecentProjects {
    store: WorkspaceStore,
    entries: Vec<RecentEntry>,
    /// The file was read (or there is none to read).
    loaded: bool,
}

impl RecentProjects {
    /// The newest first.
    pub fn entries(&self) -> &[RecentEntry] {
        &self.entries
    }

    /// The file was read: its entries join the ones opened or saved since
    /// start (those stay on top), and changes are written to `path` from
    /// now on.
    pub fn loaded(
        &mut self,
        raw: Option<Map<String, Value>>,
        from_file: Vec<RecentEntry>,
        path: PathBuf,
        now: Instant,
    ) {
        let mine = std::mem::take(&mut self.entries);
        self.store = WorkspaceStore::new(raw, path);
        self.entries = from_file;
        self.loaded = true;
        if !mine.is_empty() {
            for e in mine.into_iter().rev() {
                self.put(e);
            }
            self.sync(now);
        }
    }

    /// Marks the registry as having nothing to read (no settings folder).
    pub fn without_file(&mut self) {
        self.loaded = true;
    }

    fn put(&mut self, entry: RecentEntry) {
        self.entries.retain(|e| e.path != entry.path);
        self.entries.insert(0, entry);
        self.entries.truncate(MAX_ENTRIES);
    }

    /// `entry` was opened or saved: it goes to the top.
    pub fn touch(&mut self, entry: RecentEntry, now: Instant) {
        self.put(entry);
        self.sync(now);
    }

    /// Home's × and Remove from list: forgets `path` (the file stays).
    pub fn remove(&mut self, path: &Path, now: Instant) {
        let before = self.entries.len();
        self.entries.retain(|e| e.path != path);
        if self.entries.len() != before {
            self.sync(now);
        }
    }

    /// File › Open Recent › Clear list.
    pub fn clear(&mut self, now: Instant) {
        self.entries.clear();
        self.sync(now);
    }

    fn sync(&mut self, now: Instant) {
        let list = self.entries.iter().map(RecentEntry::to_json).collect();
        self.store.set(KEY, Value::Array(list), now);
    }

    /// See [`WorkspaceStore::tick`].
    pub fn tick(&mut self, now: Instant, wake: impl Fn() + Send + 'static) -> Option<Instant> {
        self.store.tick(now, wake)
    }

    /// See [`WorkspaceStore::flush_and_wait`].
    pub fn flush_and_wait(&mut self, timeout: Duration) {
        self.store.flush_and_wait(timeout);
    }
}

/// "Just now", "12 min ago", "3 h ago", "Yesterday", "4 days ago", "2
/// weeks ago", "5 months ago", "2 years ago": when `then` was, seen at
/// `now` (Unix seconds). A time in the future reads "Just now".
pub fn edited_ago(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    let plural = |n: u64, one: &str| {
        if n == 1 {
            format!("1 {one} ago")
        } else {
            format!("{n} {one}s ago")
        }
    };
    match s {
        0..60 => "Just now".to_owned(),
        60..3_600 => format!("{} min ago", s / 60),
        3_600..86_400 => format!("{} h ago", s / 3_600),
        86_400..172_800 => "Yesterday".to_owned(),
        172_800..1_209_600 => plural(s / 86_400, "day"),
        1_209_600..5_184_000 => plural(s / 604_800, "week"),
        5_184_000..31_536_000 => plural(s / 2_592_000, "month"),
        _ => plural(s / 31_536_000, "year"),
    }
}

/// "16:9 · 1080p", "9:16 · 1080p", "64:27 · 1080p": the canvas's ratio in
/// lowest terms (the common ones by name) and its short side.
pub fn canvas_badge(canvas: (u32, u32)) -> String {
    let (w, h) = (canvas.0.max(1), canvas.1.max(1));
    let named = [
        ((16, 9), "16:9"),
        ((9, 16), "9:16"),
        ((1, 1), "1:1"),
        ((4, 5), "4:5"),
        ((4, 3), "4:3"),
        ((64, 27), "21:9"),
    ];
    let g = gcd(w, h);
    let (rw, rh) = (w / g, h / g);
    let ratio = named
        .iter()
        .find(|((a, b), _)| *a == rw && *b == rh)
        .map_or_else(|| format!("{rw}:{rh}"), |(_, n)| (*n).to_owned());
    format!("{ratio} · {}p", w.min(h))
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// A frame to keep as a cover: the Stage's picture, `width` × `height`
/// RGBA.
#[derive(Debug, Clone)]
pub struct CoverFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// `rgba` (`w` × `h`) averaged down to at most `max_w` wide, keeping the
/// shape: `(width, height, rgb)`. A box filter: each output pixel is the
/// mean of the input pixels it covers.
pub fn downsample(w: u32, h: u32, rgba: &[u8], max_w: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w.max(1), h.max(1));
    let ow = w.min(max_w.max(1));
    let oh = ((u64::from(h) * u64::from(ow)) / u64::from(w)).max(1) as u32;
    let mut out = Vec::with_capacity((ow * oh * 3) as usize);
    for oy in 0..oh {
        let y0 = oy * h / oh;
        let y1 = ((oy + 1) * h / oh).max(y0 + 1).min(h);
        for ox in 0..ow {
            let x0 = ox * w / ow;
            let x1 = ((ox + 1) * w / ow).max(x0 + 1).min(w);
            let mut sum = [0u64; 3];
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    if let Some(px) = rgba.get(i..i + 3) {
                        sum[0] += u64::from(px[0]);
                        sum[1] += u64::from(px[1]);
                        sum[2] += u64::from(px[2]);
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            out.extend(sum.map(|c| (c / n) as u8));
        }
    }
    (ow, oh, out)
}

/// A 24-bit uncompressed BMP of `rgb` (`w` × `h`, rows top to bottom).
pub fn encode_bmp(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let data = row * h;
    let mut out = Vec::with_capacity(54 + data as usize);
    out.extend(b"BM");
    out.extend((54 + data).to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(54u32.to_le_bytes());
    out.extend(40u32.to_le_bytes());
    out.extend(w.to_le_bytes());
    out.extend(h.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend([0u8; 4]); // no compression
    out.extend(data.to_le_bytes());
    out.extend(2835u32.to_le_bytes());
    out.extend(2835u32.to_le_bytes());
    out.extend([0u8; 8]);
    // Bottom row first.
    for y in (0..h).rev() {
        let start = (y * w * 3) as usize;
        for x in 0..w as usize {
            let px = rgb
                .get(start + x * 3..start + x * 3 + 3)
                .unwrap_or(&[0, 0, 0]);
            out.extend([px[2], px[1], px[0]]);
        }
        out.extend(std::iter::repeat_n(0u8, (row - w * 3) as usize));
    }
    out
}

/// Reads back what [`encode_bmp`] wrote: `(w, h, rgba)`; `None` for
/// anything else.
pub fn decode_bmp(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |i: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(i..i + 4)?.try_into().ok()?))
    };
    if bytes.get(0..2)? != b"BM" || bytes.get(28..30)? != [24, 0] || u32_at(30)? != 0 {
        return None;
    }
    let offset = u32_at(10)? as usize;
    let (w, h) = (u32_at(18)?, u32_at(22)?);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let row = ((w * 3).div_ceil(4) * 4) as usize;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let src = offset + (h as usize - 1 - y) * row;
        let line = bytes.get(src..src + w as usize * 3)?;
        for px in line.as_chunks::<3>().0 {
            rgba.extend([px[2], px[1], px[0], 255]);
        }
    }
    Some((w, h, rgba))
}

/// Blocking: writes `frame` as the cover of `project` in `dir`.
pub fn write_cover(dir: &Path, project: &Path, frame: &CoverFrame) -> std::io::Result<()> {
    let (w, h, rgb) = downsample(frame.width, frame.height, &frame.rgba, COVER_WIDTH);
    std::fs::create_dir_all(dir)?;
    crate::settings::write_atomic(&cover_path(dir, project), &encode_bmp(w, h, &rgb))
}

/// Blocking: the cover of `project` in `dir`, as `(w, h, rgba)`.
pub fn read_cover(dir: &Path, project: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(cover_path(dir, project)).ok()?;
    decode_bmp(&bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;

    fn entry(path: &str, opened: u64) -> RecentEntry {
        RecentEntry {
            path: PathBuf::from(path),
            name: stem(Path::new(path)),
            opened,
            canvas: (1080, 1920),
            rate: (30_000, 1001),
            duration_ms: 42_000,
            found: true,
        }
    }

    /// PM-017: newest first, one entry per path, at most 200.
    #[test]
    fn touching_moves_to_the_top_and_keeps_two_hundred() {
        let now = Instant::now();
        let mut recent = RecentProjects::default();
        recent.touch(entry("/a.freecut", 1), now);
        recent.touch(entry("/b.freecut", 2), now);
        recent.touch(entry("/a.freecut", 3), now);
        let names: Vec<_> = recent.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(recent.entries()[0].opened, 3);
        for i in 0..250 {
            recent.touch(entry(&format!("/p{i}.freecut"), i), now);
        }
        assert_eq!(recent.entries().len(), MAX_ENTRIES);
        assert_eq!(recent.entries()[0].name, "p249");
        recent.remove(Path::new("/p249.freecut"), now);
        assert_eq!(recent.entries()[0].name, "p248");
        recent.clear(now);
        assert!(recent.entries().is_empty());
    }

    /// The file is written a second after a change, read back with every
    /// field, and what was opened before it was read stays on top.
    #[test]
    fn the_registry_round_trips_through_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let project = dir.path().join("Trip.freecut");
        std::fs::write(&project, b"{}").unwrap();
        let now = Instant::now();
        let mut recent = RecentProjects::default();
        recent.loaded(None, Vec::new(), path.clone(), now);
        let mut trip = entry(project.to_str().unwrap(), 10);
        trip.name = "Trip".to_owned();
        recent.touch(trip.clone(), now);
        recent.touch(entry("/gone/Old.freecut", 5), now);
        recent.flush_and_wait(Duration::from_secs(10));
        let (raw, entries) = load(&path);
        assert!(raw.is_some());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1], trip);
        assert!(!entries[0].found, "a missing file is marked");

        let mut again = RecentProjects::default();
        again.touch(entry("/new.freecut", 99), now);
        again.loaded(raw, entries, path, now);
        let names: Vec<_> = again.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["new", "Old", "Trip"]);
    }

    #[test]
    fn damaged_entries_are_skipped() {
        let raw: Map<String, Value> = serde_json::from_str(
            r#"{"version":1,"projects":[{"path":""},{"name":"x"},{"path":"/ok.freecut","canvas":"bad"}]}"#,
        )
        .unwrap();
        let entries = entries_from(&raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "ok");
        assert_eq!(entries[0].canvas, (1920, 1080));
    }

    #[test]
    fn edited_times_read_like_people_talk() {
        let now = 1_000_000_000;
        let ago = |s: u64| edited_ago(now - s, now);
        assert_eq!(ago(5), "Just now");
        assert_eq!(edited_ago(now + 50, now), "Just now");
        assert_eq!(ago(12 * 60), "12 min ago");
        assert_eq!(ago(3 * 3600 + 5), "3 h ago");
        assert_eq!(ago(30 * 3600), "Yesterday");
        assert_eq!(ago(2 * 86_400), "2 days ago");
        assert_eq!(ago(15 * 86_400), "2 weeks ago");
        assert_eq!(ago(70 * 86_400), "2 months ago");
        assert_eq!(ago(400 * 86_400), "1 year ago");
    }

    #[test]
    fn canvas_badges_name_the_shape_and_short_side() {
        assert_eq!(canvas_badge((1920, 1080)), "16:9 · 1080p");
        assert_eq!(canvas_badge((1080, 1920)), "9:16 · 1080p");
        assert_eq!(canvas_badge((1080, 1350)), "4:5 · 1080p");
        assert_eq!(canvas_badge((2560, 1080)), "21:9 · 1080p");
        assert_eq!(canvas_badge((1000, 700)), "10:7 · 700p");
        assert_eq!(canvas_badge((0, 0)), "1:1 · 1p");
    }

    #[test]
    fn covers_are_small_bmps_that_read_back() {
        // A 4 × 2 picture: left half red, right half blue.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                rgba.extend(if x < 2 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                });
            }
        }
        let (w, h, rgb) = downsample(4, 2, &rgba, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgb, [255, 0, 0, 0, 0, 255]);
        let (w, h, rgb) = downsample(4, 2, &rgba, 480);
        assert_eq!((w, h, rgb.len()), (4, 2, 24));

        let bmp = encode_bmp(w, h, &rgb);
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(bmp.len(), 54 + 2 * 12);
        let (bw, bh, back) = decode_bmp(&bmp).unwrap();
        assert_eq!((bw, bh), (4, 2));
        assert_eq!(back, rgba);
        assert!(decode_bmp(b"BMnope").is_none());

        let dir = tempfile::tempdir().unwrap();
        let frame = CoverFrame {
            width: 4,
            height: 2,
            rgba: Arc::new(rgba.clone()),
        };
        let project = Path::new("/projects/Trip.freecut");
        write_cover(dir.path(), project, &frame).unwrap();
        assert_eq!(read_cover(dir.path(), project).unwrap().2, rgba);
        assert!(read_cover(dir.path(), Path::new("/other.freecut")).is_none());
        assert_ne!(
            cover_path(dir.path(), project),
            cover_path(dir.path(), Path::new("/projects/Trip 2.freecut"))
        );
    }
}
