//! Offline mixing for export (AU-010, EX-002).
//!
//! [`OfflineMix`] renders the sound of one export range at one sample rate:
//! exactly [`ExportClock::sample_count`] interleaved stereo `f32` frames,
//! starting at the sample that covers the range's start
//! (`floor(start · rate / TPS)`), so adjacent ranges tile with no gap or
//! overlap. It drives the same [`Mixer`] as playback, with the same
//! [`Schedule`] and [`SourceOpener`] (a [`DocSchedule`] and the app's
//! `FileOpener`), only without a device or ring: blocks are rendered as
//! fast as the sources decode, readers are opened inline when first needed,
//! and every block continues the previous one, so sources are read
//! sequentially. Rendering in blocks gives the same samples as one call of
//! [`Mixer::render`] over the whole range.
//!
//! **Solo.** An export ignores track solo (AU-012): it renders the
//! schedule's [`Schedule::export_inputs`]. Only when asked to export as
//! heard ([`OfflineMix::set_export_as_heard`]) does it render what preview
//! plays, [`Schedule::inputs`].

use freecut_timeline::export::{ExportClock, ExportTimeError};
use freecut_timeline::time::{SampleRate, TimeRange};

use crate::document::DocSchedule;
use crate::mixer::Mixer;
use crate::samples::sample_at;
use crate::source::{MixInput, Schedule, SourceOpener};

/// Frames per block unless [`OfflineMix::with_block_frames`] says otherwise.
pub const DEFAULT_BLOCK_FRAMES: usize = 4096;

/// Why an offline mix stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OfflineMixError {
    #[error("the export range is too long to address in samples")]
    TooLong,
    #[error("the mix was cancelled")]
    Cancelled,
}

impl From<ExportTimeError> for OfflineMixError {
    fn from(_: ExportTimeError) -> OfflineMixError {
        OfflineMixError::TooLong
    }
}

/// The sound of one export range, rendered block by block.
#[derive(Debug)]
pub struct OfflineMix {
    mixer: Mixer,
    /// Index of the first output sample at the mix rate.
    first: i64,
    total: u64,
    done: u64,
    block: usize,
    buf: Vec<f32>,
    /// Render what preview plays (solo included) instead of the export
    /// view.
    as_heard: bool,
}

/// A schedule as an export renders it: its export view.
struct ExportView<'a>(&'a dyn Schedule);

impl Schedule for ExportView<'_> {
    fn inputs(&self, window: TimeRange) -> Vec<MixInput> {
        self.0.export_inputs(window)
    }
}

impl OfflineMix {
    /// The export range of `clock` at `rate`, at unity master gain with the
    /// limiter on.
    pub fn new(clock: &ExportClock, rate: SampleRate) -> Result<OfflineMix, OfflineMixError> {
        let total = clock.sample_count(rate)?;
        let first = sample_at(clock.range().start(), rate);
        // The last sample index must be representable too.
        i64::try_from(total)
            .ok()
            .and_then(|n| first.checked_add(n))
            .ok_or(OfflineMixError::TooLong)?;
        Ok(OfflineMix {
            mixer: Mixer::new(rate),
            first,
            total,
            done: 0,
            block: DEFAULT_BLOCK_FRAMES,
            buf: Vec::new(),
            as_heard: false,
        })
    }

    /// Like [`OfflineMix::new`], with the master volume of `schedule`'s
    /// sequence (AU-010: the same in preview and export).
    pub fn for_document(
        schedule: &DocSchedule,
        clock: &ExportClock,
        rate: SampleRate,
    ) -> Result<OfflineMix, OfflineMixError> {
        let mut mix = OfflineMix::new(clock, rate)?;
        mix.set_master_gain(schedule.master_amplitude());
        Ok(mix)
    }

    /// Frames per block (at least 1). It doesn't change the samples.
    pub fn with_block_frames(mut self, frames: usize) -> OfflineMix {
        self.block = frames.max(1);
        self
    }

    /// Master volume as linear amplitude (see [`Mixer::set_master_gain`]).
    /// Set it before the first block: the first block starts at it, later
    /// changes ramp.
    pub fn set_master_gain(&mut self, linear: f32) {
        self.mixer.set_master_gain(linear);
    }

    /// "Export as heard" (AU-012): render what preview plays, track solo
    /// included, instead of ignoring solo (the default). Set it before the
    /// first block.
    pub fn set_export_as_heard(&mut self, as_heard: bool) {
        self.as_heard = as_heard;
    }

    /// Whether the mix renders what preview plays, solo included.
    pub fn exports_as_heard(&self) -> bool {
        self.as_heard
    }

    /// Turns the safety limiter on or off (see [`Mixer::set_limiter`]).
    pub fn set_limiter(&mut self, on: bool) {
        self.mixer.set_limiter(on);
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.mixer.sample_rate()
    }

    /// Index (at the mix rate) of the first sample of the range.
    pub fn first_sample(&self) -> i64 {
        self.first
    }

    /// Stereo frames in the range: [`ExportClock::sample_count`].
    pub fn total_frames(&self) -> u64 {
        self.total
    }

    /// Stereo frames rendered so far.
    pub fn rendered_frames(&self) -> u64 {
        self.done
    }

    pub fn is_done(&self) -> bool {
        self.done >= self.total
    }

    /// Renders the next block (interleaved stereo, at most the block size,
    /// never past the end of the range). `None` once the range is done.
    pub fn next_block(
        &mut self,
        schedule: &dyn Schedule,
        opener: &dyn SourceOpener,
    ) -> Option<&[f32]> {
        let left = self.total.checked_sub(self.done).filter(|&n| n > 0)?;
        let frames = usize::try_from(left).map_or(self.block, |n| n.min(self.block));
        // `new` checked that every sample index of the range fits in i64,
        // and `done < total`.
        let start = self.first + self.done as i64;
        self.buf.resize(frames * 2, 0.0);
        if self.as_heard {
            self.mixer.render(schedule, opener, start, &mut self.buf);
        } else {
            let view = ExportView(schedule);
            self.mixer.render(&view, opener, start, &mut self.buf);
        }
        self.done += frames as u64;
        Some(&self.buf)
    }

    /// Renders the rest of the range block by block, handing each block to
    /// `sink`. `cancelled` is checked before every block; on cancel the
    /// blocks already handed over stay valid and the mix can't continue
    /// meaningfully. Returns the frames rendered by this call.
    pub fn render_all(
        &mut self,
        schedule: &dyn Schedule,
        opener: &dyn SourceOpener,
        cancelled: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(&[f32]),
    ) -> Result<u64, OfflineMixError> {
        let before = self.done;
        while !self.is_done() {
            if cancelled() {
                return Err(OfflineMixError::Cancelled);
            }
            if let Some(block) = self.next_block(schedule, opener) {
                sink(block);
            }
        }
        Ok(self.done - before)
    }

    /// The rest of the range in one buffer (interleaved stereo).
    pub fn render_to_vec(
        &mut self,
        schedule: &dyn Schedule,
        opener: &dyn SourceOpener,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<f32>, OfflineMixError> {
        let left = self.total.saturating_sub(self.done);
        let mut out = Vec::with_capacity(usize::try_from(left.saturating_mul(2)).unwrap_or(0));
        self.render_all(schedule, opener, cancelled, &mut |b| {
            out.extend_from_slice(b)
        })?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
