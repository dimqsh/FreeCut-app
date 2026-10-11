//! Re-chunking interleaved audio into encoder frames (export-pipeline §7.4).

use std::collections::VecDeque;

/// An interleaved `f32` queue that hands out exactly `frame_size` sample
/// frames at a time, plus one short frame at the end. Every sample pushed
/// comes out exactly once, in order.
#[derive(Debug, Clone)]
pub(crate) struct SampleFifo {
    channels: usize,
    frame_size: usize,
    buf: VecDeque<f32>,
}

impl SampleFifo {
    /// A queue for `channels` interleaved channels and encoder frames of
    /// `frame_size` sample frames (both at least 1).
    pub(crate) fn new(channels: usize, frame_size: usize) -> SampleFifo {
        SampleFifo {
            channels: channels.max(1),
            frame_size: frame_size.max(1),
            buf: VecDeque::new(),
        }
    }

    /// Appends interleaved samples. The caller checks that `samples` holds
    /// whole sample frames.
    pub(crate) fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples.iter().copied());
    }

    /// Sample frames waiting.
    pub(crate) fn frames(&self) -> usize {
        self.buf.len() / self.channels
    }

    /// The next full encoder frame (`frame_size × channels` interleaved
    /// samples), if one is complete.
    pub(crate) fn pop_frame(&mut self) -> Option<Vec<f32>> {
        (self.frames() >= self.frame_size).then(|| self.take(self.frame_size))
    }

    /// Whatever is left (a short last frame), if anything.
    pub(crate) fn pop_rest(&mut self) -> Option<Vec<f32>> {
        let n = self.frames();
        (n > 0).then(|| self.take(n))
    }

    fn take(&mut self, frames: usize) -> Vec<f32> {
        self.buf.drain(..frames * self.channels).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any sequence of block sizes in → full frames, then one short
        /// frame, with every sample conserved in order.
        #[test]
        fn samples_are_conserved(blocks in proptest::collection::vec(0usize..3000, 0..20)) {
            let mut fifo = SampleFifo::new(2, 1024);
            let mut next = 0u32;
            let mut out = Vec::new();
            let mut sizes = Vec::new();
            for frames in blocks {
                let block: Vec<f32> = (0..frames * 2)
                    .map(|_| {
                        next += 1;
                        next as f32
                    })
                    .collect();
                fifo.push(&block);
                while let Some(f) = fifo.pop_frame() {
                    sizes.push(f.len() / 2);
                    out.extend(f);
                }
            }
            if let Some(rest) = fifo.pop_rest() {
                prop_assert!(rest.len() / 2 < 1024);
                sizes.push(rest.len() / 2);
                out.extend(rest);
            }
            prop_assert_eq!(fifo.frames(), 0);
            prop_assert_eq!(out.len(), next as usize);
            prop_assert!(out.iter().enumerate().all(|(i, &v)| v == (i + 1) as f32));
            // Only the last frame may be short.
            if let Some((last, full)) = sizes.split_last() {
                prop_assert!(full.iter().all(|&s| s == 1024));
                prop_assert!(*last >= 1 && *last <= 1024);
            }
        }
    }

    #[test]
    fn empty_fifo_has_nothing() {
        let mut fifo = SampleFifo::new(2, 1024);
        assert_eq!(fifo.pop_frame(), None);
        assert_eq!(fifo.pop_rest(), None);
        fifo.push(&[1.0, 2.0]);
        assert_eq!(fifo.pop_frame(), None);
        assert_eq!(fifo.pop_rest(), Some(vec![1.0, 2.0]));
    }
}
