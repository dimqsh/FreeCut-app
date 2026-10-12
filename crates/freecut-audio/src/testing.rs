//! Synthetic sources and schedules for unit tests (no FFmpeg).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use freecut_timeline::time::{SampleRate, Tick, TimeRange};

use crate::samples::sample_at;
use crate::source::{MixInput, ReadError, Schedule, SourceKey, SourceOpener, SourceReader};

/// What a synthetic source plays.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Wave {
    /// Left = media sample index, right = its negation.
    Counter,
    /// The same value on both channels.
    Dc(f32),
    /// A sine of `amp` at `hz`, in phase on both channels.
    Sine { amp: f32, hz: f64 },
    /// Opens, but every read fails.
    Broken,
}

/// A clip's worth of inputs; answers windows by overlap.
#[derive(Debug, Clone, Default)]
pub(crate) struct List(pub Vec<MixInput>);

impl Schedule for List {
    fn inputs(&self, window: TimeRange) -> Vec<MixInput> {
        self.0
            .iter()
            .filter(|i| i.range.overlaps(window))
            .cloned()
            .collect()
    }
}

/// Every read a test reader served: `(asset, at, frames)`.
pub(crate) type ReadLog = Arc<Mutex<Vec<(u64, Tick, usize)>>>;

/// Opens synthetic readers by asset id; unknown assets are offline.
#[derive(Debug, Clone, Default)]
pub(crate) struct Opener {
    pub waves: HashMap<u64, Wave>,
    /// Per-stream waves, which win over `waves` for that stream.
    pub streams: HashMap<(u64, u32), Wave>,
    pub opens: Arc<AtomicUsize>,
    pub drops: Arc<AtomicUsize>,
    pub log: ReadLog,
}

impl Opener {
    pub fn with(waves: &[(u64, Wave)]) -> Opener {
        Opener {
            waves: waves.iter().copied().collect(),
            ..Opener::default()
        }
    }

    pub fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }

    pub fn drops(&self) -> usize {
        self.drops.load(Ordering::SeqCst)
    }

    pub fn reads(&self) -> Vec<(u64, Tick, usize)> {
        self.log.lock().unwrap().clone()
    }
}

impl SourceOpener for Opener {
    fn open(&self, source: &SourceKey, rate: SampleRate) -> Option<Box<dyn SourceReader>> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let wave = *self
            .streams
            .get(&(source.asset, source.stream))
            .or_else(|| self.waves.get(&source.asset))?;
        Some(Box::new(Reader {
            asset: source.asset,
            wave,
            rate,
            log: Arc::clone(&self.log),
            drops: Arc::clone(&self.drops),
        }))
    }
}

struct Reader {
    asset: u64,
    wave: Wave,
    rate: SampleRate,
    log: ReadLog,
    drops: Arc<AtomicUsize>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl SourceReader for Reader {
    fn read(&mut self, at: Tick, frames: usize, out: &mut [f32]) -> Result<(), ReadError> {
        assert_eq!(out.len(), frames * 2);
        self.log.lock().unwrap().push((self.asset, at, frames));
        let first = sample_at(at, self.rate);
        for (i, o) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let k = first + i as i64;
            let (l, r) = match self.wave {
                Wave::Counter => (k as f32, -(k as f32)),
                Wave::Dc(v) => (v, v),
                Wave::Sine { amp, hz } => {
                    let v = amp
                        * (std::f64::consts::TAU * hz * k as f64 / f64::from(self.rate.hz())).sin()
                            as f32;
                    (v, v)
                }
                Wave::Broken => return Err(ReadError::new("broken test source")),
            };
            o[0] = l;
            o[1] = r;
        }
        Ok(())
    }
}

pub(crate) fn rate(hz: u32) -> SampleRate {
    SampleRate::new(hz).unwrap()
}

pub(crate) fn secs(s: f64) -> Tick {
    Tick::from_raw((s * 705_600_000.0).round() as i64)
}

/// An input with unity gain and no fades.
pub(crate) fn input(key: u64, asset: u64, start: Tick, dur: Tick, source_start: Tick) -> MixInput {
    MixInput::new(
        key,
        SourceKey { asset, stream: 0 },
        TimeRange::new(start, dur).unwrap(),
        source_start,
    )
}
