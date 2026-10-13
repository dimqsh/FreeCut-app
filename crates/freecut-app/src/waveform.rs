//! Waveform peaks for the timeline (TL-042, AU-001, AU-002).
//!
//! Peaks are computed once per (library item, audio stream) by
//! `freecut_media::audio_peaks` on a worker of their own, so a long file
//! never holds up imports. They are kept as a small pyramid: min/max per
//! 1/100 s, then per 4×, 16×… that, so drawing a zoomed-out timeline reads
//! a few values per pixel column instead of thousands.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use freecut_media::cancel::CancelToken;
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{ClipContent, Document, MediaSource, StreamSel};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick, TimeRange};

use crate::jobs::Jobs;
use crate::session::Project;

/// Media time covered by one finest peak bucket: 1/100 s.
pub const BUCKET: Tick = Tick::from_raw(TICKS_PER_SECOND / 100);
/// How many buckets of one level make one bucket of the next.
const FAN: usize = 4;

/// Which peaks a clip shows: a library item and a container stream index.
pub type PeakKey = (MediaId, u32);

/// Min/max peaks of one audio stream, at several resolutions.
#[derive(Debug, Clone, PartialEq)]
pub struct Peaks {
    /// `levels[n][k]` covers media time `[k, k + 1) × BUCKET × FAN^n`.
    levels: Vec<Vec<(f32, f32)>>,
}

impl Peaks {
    /// The pyramid over `finest` (one entry per [`BUCKET`]).
    pub fn new(finest: Vec<(f32, f32)>) -> Peaks {
        let mut levels = vec![finest];
        while let Some(last) = levels.last()
            && last.len() > 1
        {
            let next = last.chunks(FAN).map(merge_all).collect();
            levels.push(next);
        }
        Peaks { levels }
    }

    /// The min and max sample over media time `[from, to)`, exact to the
    /// finest bucket: whole aligned blocks are read from coarser levels and
    /// only the edges from finer ones, so a few values are read whatever
    /// the span, and no peak is skipped or smeared into a neighbour.
    /// `None` outside the stream.
    pub fn range(&self, from: Tick, to: Tick) -> Option<(f32, f32)> {
        let from = from.max(Tick::ZERO);
        if to <= from {
            return None;
        }
        let finest = self.levels.first()?;
        let mut lo = usize::try_from(from.raw() / BUCKET.raw()).ok()?;
        // One past the bucket holding the span's last tick.
        let mut hi = usize::try_from((to.raw() - 1) / BUCKET.raw())
            .ok()?
            .saturating_add(1)
            .min(finest.len());
        if lo >= hi {
            return None;
        }
        let mut acc = (f32::INFINITY, f32::NEG_INFINITY);
        let mut take = |v: &[(f32, f32)]| {
            let (a, b) = merge_all(v);
            acc = (acc.0.min(a), acc.1.max(b));
        };
        for (n, values) in self.levels.iter().enumerate() {
            if lo >= hi {
                break;
            }
            if n + 1 == self.levels.len() {
                take(&values[lo..hi]);
                break;
            }
            let head = lo.next_multiple_of(FAN).min(hi);
            take(&values[lo..head]);
            let tail = (hi / FAN * FAN).max(head);
            take(&values[tail..hi]);
            (lo, hi) = (head / FAN, tail / FAN);
        }
        Some(acc)
    }
}

fn merge_all(values: &[(f32, f32)]) -> (f32, f32) {
    values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &(a, b)| {
            (lo.min(a), hi.max(b))
        })
}

/// The media time shown by timeline time `[a, b)` of a clip placed at
/// `range` that plays from `source_start`, clipped to the clip. `None`
/// when the span misses the clip.
pub fn column_source_range(
    range: TimeRange,
    source_start: Tick,
    a: Tick,
    b: Tick,
) -> Option<(Tick, Tick)> {
    let a = a.max(range.start());
    let b = b.min(range.end());
    if b <= a {
        return None;
    }
    let from = source_start
        .checked_add(a.checked_sub(range.start()).ok()?)
        .ok()?;
    let to = source_start
        .checked_add(b.checked_sub(range.start()).ok()?)
        .ok()?;
    Some((from, to))
}

/// The peaks a clip of `asset` with stream selection `sel` shows: the
/// stream it plays (AU-038), so choosing another stream in the Sound
/// source picker redraws the waveform from that stream. "All streams
/// mixed" shows the first stream the mix plays (FreeCut default: no mixed
/// peaks are computed). `sel` `None` (the Audio tab, or a clip following
/// the default) shows the file's default stream (AU-001). An extracted item
/// that still reads its parent's file shares the parent's peaks, so the
/// file is scanned once.
pub fn peak_key(doc: &Document, asset: MediaId, sel: Option<StreamSel>) -> Option<PeakKey> {
    let library = doc.library();
    let item = library.get(asset)?;
    let stream = item.info.played_streams(sel).first()?.index;
    match item.source {
        MediaSource::AudioOf { source, stream: s }
            if library.get(source).is_some_and(|p| p.path == item.path) =>
        {
            Some((source, s))
        }
        _ => Some((asset, stream)),
    }
}

/// Every stream whose peaks the project may draw: each library item's
/// default stream (the Audio tab) and the stream each media clip plays.
fn wanted_keys(doc: &Document) -> BTreeSet<PeakKey> {
    let defaults = doc
        .library()
        .items()
        .filter_map(|item| peak_key(doc, item.id, None));
    let clips = doc
        .sequences()
        .flat_map(|s| s.tracks().flat_map(|(_, t)| t.clips().iter()))
        .filter_map(|c| match &c.content {
            ClipContent::Media { asset, stream, .. } => peak_key(doc, *asset, *stream),
            _ => None,
        });
    defaults.chain(clips).collect()
}

/// Peaks of one stream, as far as they are known.
#[derive(Debug, Clone)]
pub enum PeakState {
    Pending,
    Ready(Arc<Peaks>),
    /// The stream couldn't be read; the reason is kept for the tooltip.
    Failed(String),
}

struct Done {
    generation: u64,
    key: PeakKey,
    result: Result<Peaks, String>,
}

/// The peak cache and its worker.
pub struct Waveforms {
    states: HashMap<PeakKey, PeakState>,
    jobs: Jobs<Done>,
    /// Cancels every running job when the project changes.
    cancel: CancelToken,
    /// Results from before the last [`Waveforms::clear`] are dropped.
    generation: u64,
}

impl std::fmt::Debug for Waveforms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waveforms")
            .field("streams", &self.states.len())
            .finish_non_exhaustive()
    }
}

impl Waveforms {
    /// `wake` is called after each finished job.
    pub fn new(wake: impl Fn() + Send + Sync + Clone + 'static) -> Waveforms {
        Waveforms {
            states: HashMap::new(),
            jobs: Jobs::new(1, wake),
            cancel: CancelToken::new(),
            generation: 0,
        }
    }

    pub fn get(&self, key: PeakKey) -> Option<&PeakState> {
        self.states.get(&key)
    }

    /// Forgets everything and cancels running scans (another project).
    pub fn clear(&mut self) {
        self.cancel.cancel();
        self.cancel = CancelToken::new();
        self.generation += 1;
        self.states.clear();
    }

    /// Whether a scan is still running (headless snapshots wait for it).
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.states
            .values()
            .any(|s| matches!(s, PeakState::Pending))
    }

    /// Takes in finished scans.
    pub fn poll(&mut self) {
        for done in self.jobs.poll() {
            if done.generation != self.generation {
                continue;
            }
            let state = match done.result {
                Ok(peaks) => PeakState::Ready(Arc::new(peaks)),
                Err(e) => PeakState::Failed(e),
            };
            self.states.insert(done.key, state);
        }
    }

    /// Starts a scan for every stream the project's clips may show that
    /// hasn't been asked for yet. Offline media gets none.
    pub fn request_missing(&mut self, project: &Project) {
        let doc = project.doc();
        let wanted: Vec<(PeakKey, PathBuf)> = wanted_keys(doc)
            .into_iter()
            .filter(|key| !self.states.contains_key(key))
            .filter(|(id, _)| !project.offline.contains(id))
            .filter_map(|key| project.media_paths.get(&key.0).map(|p| (key, p.clone())))
            .collect();
        for (key, path) in wanted {
            self.states.insert(key, PeakState::Pending);
            let generation = self.generation;
            let cancel = self.cancel.clone();
            self.jobs.spawn(move || {
                let result = usize::try_from(key.1)
                    .map_err(|e| e.to_string())
                    .and_then(|stream| {
                        freecut_media::audio_peaks(&path, stream, BUCKET, &cancel)
                            .map_err(|e| e.to_string())
                    })
                    .map(Peaks::new);
                Done {
                    generation,
                    key,
                    result,
                }
            });
        }
    }
}

impl Drop for Waveforms {
    fn drop(&mut self) {
        // The worker pool joins its threads when dropped; stop long scans
        // first so closing the window doesn't wait for them.
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)] // test code

    use super::*;

    fn t(buckets: i64) -> Tick {
        Tick::from_raw(BUCKET.raw() * buckets)
    }

    #[test]
    fn pyramid_levels_merge_four_buckets() {
        let finest: Vec<(f32, f32)> = (0..10).map(|i| (-(i as f32), i as f32)).collect();
        let peaks = Peaks::new(finest);
        assert_eq!(peaks.levels.len(), 3);
        assert_eq!(peaks.levels[1], vec![(-3.0, 3.0), (-7.0, 7.0), (-9.0, 9.0)]);
        assert_eq!(peaks.levels[2], vec![(-9.0, 9.0)]);
        assert_eq!(Peaks::new(Vec::new()).range(t(0), t(1)), None);
    }

    #[test]
    fn range_never_skips_a_peak() {
        let mut finest = vec![(0.0f32, 0.0f32); 1_000];
        finest[517] = (-0.9, 0.8);
        let peaks = Peaks::new(finest);
        // Fine column over the spike.
        assert_eq!(peaks.range(t(517), t(518)), Some((-0.9, 0.8)));
        // Columns far wider than a bucket use coarser levels; still found.
        assert_eq!(peaks.range(t(500), t(600)), Some((-0.9, 0.8)));
        assert_eq!(peaks.range(t(0), t(1_000)), Some((-0.9, 0.8)));
        assert_eq!(peaks.range(t(518), t(600)), Some((0.0, 0.0)));
        // A sub-bucket span inside one bucket reads that bucket.
        let half = Tick::from_raw(BUCKET.raw() / 2);
        assert_eq!(
            peaks.range(t(517), t(517).checked_add(half).unwrap()),
            Some((-0.9, 0.8))
        );
        // Past the end of the stream, or empty.
        assert_eq!(peaks.range(t(1_000), t(1_010)), None);
        assert_eq!(peaks.range(t(5), t(5)), None);
        // Partly past the end: the part inside counts.
        assert_eq!(peaks.range(t(990), t(2_000)), Some((0.0, 0.0)));
        // Negative media time is clamped to the start.
        assert_eq!(peaks.range(t(-5), t(1)), Some((0.0, 0.0)));
    }

    #[test]
    fn peaks_follow_the_stream_a_clip_plays() {
        use freecut_timeline::model::AudioStreamInfo;

        use crate::clip_streams::{set_stream, tests::project_with_streams};

        let stream = |index: u32, default: bool| AudioStreamInfo {
            index,
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: Tick::from_secs(4).unwrap(),
            duration_exact: true,
            default,
            language: None,
            title: None,
            layout: None,
        };
        let (mut project, clip) = project_with_streams(vec![stream(1, true), stream(2, false)]);
        let asset = project.doc().clip(clip).unwrap().content.asset().unwrap();
        let doc = project.doc();
        assert_eq!(peak_key(doc, asset, None), Some((asset, 1)));
        assert_eq!(
            peak_key(doc, asset, Some(StreamSel::Index(2))),
            Some((asset, 2))
        );
        // "All streams mixed" shows the first stream it plays.
        assert_eq!(
            peak_key(doc, asset, Some(StreamSel::AllMixed)),
            Some((asset, 1))
        );
        assert_eq!(wanted_keys(doc), BTreeSet::from([(asset, 1)]));

        // Picking stream 2 asks for its peaks; the Audio tab keeps the
        // default stream's.
        set_stream(&mut project, &[clip], Some(StreamSel::Index(2))).unwrap();
        assert_eq!(
            wanted_keys(project.doc()),
            BTreeSet::from([(asset, 1), (asset, 2)])
        );
    }

    #[test]
    fn columns_map_through_the_source_range() {
        let s = |n: i64| Tick::from_secs(n).unwrap();
        // A clip at 10 s..14 s playing source from 2 s.
        let range = TimeRange::new(s(10), s(4)).unwrap();
        assert_eq!(
            column_source_range(range, s(2), s(11), s(12)),
            Some((s(3), s(4)))
        );
        // Clipped to the clip's own span.
        assert_eq!(
            column_source_range(range, s(2), s(9), s(11)),
            Some((s(2), s(3)))
        );
        assert_eq!(
            column_source_range(range, s(2), s(13), s(20)),
            Some((s(5), s(6)))
        );
        assert_eq!(column_source_range(range, s(2), s(14), s(15)), None);
        assert_eq!(column_source_range(range, s(2), s(5), s(10)), None);
    }
}
