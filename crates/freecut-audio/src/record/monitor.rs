//! "Hear yourself" (AU-027): the gained input, played on the output device
//! on its own path. The capture callback pushes into a small ring; the
//! output callback reads it after the master stage
//! (`AudioEngine::set_monitor`), so it is never mixed into an export and
//! never written to the take.

use rtrb::{Consumer, Producer, RingBuffer};

/// Ring capacity in frames: about 85 ms at 48 kHz.
const RING_FRAMES: usize = 4_096;
/// The most frames left queued when the output starts a buffer; older ones
/// are dropped so the delay can't grow (about 21 ms at 48 kHz).
pub(crate) const MAX_BACKLOG_FRAMES: usize = 1_024;

/// The output side of the monitoring path, handed to the output engine.
/// Frames are stereo.
pub struct MonitorFeed {
    ring: Consumer<f32>,
    rate: u32,
}

impl std::fmt::Debug for MonitorFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonitorFeed")
            .field("rate", &self.rate)
            .field("queued", &self.ring.slots())
            .finish()
    }
}

impl MonitorFeed {
    /// The input's sample rate. The output plays the feed only at this
    /// rate (it is silent otherwise; the panel then turns monitoring off).
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Frames queued.
    pub fn queued_frames(&self) -> usize {
        self.ring.slots() / 2
    }

    /// Drops frames beyond `max` queued ones (real-time safe).
    pub(crate) fn trim(&mut self, max: usize) {
        let extra = self.queued_frames().saturating_sub(max) * 2;
        if extra > 0
            && let Ok(chunk) = self.ring.read_chunk(extra)
        {
            chunk.commit_all();
        }
    }

    /// The next stereo frame, if one is queued (real-time safe). A frame is
    /// pushed as two samples, so a half-pushed one waits.
    #[inline]
    pub(crate) fn next_frame(&mut self) -> Option<(f32, f32)> {
        if self.ring.slots() < 2 {
            return None;
        }
        let l = self.ring.pop().ok()?;
        let r = self.ring.pop().ok()?;
        Some((l, r))
    }
}

/// The capture side.
#[derive(Debug)]
pub(crate) struct MonitorTap {
    ring: Producer<f32>,
}

impl MonitorTap {
    /// Pushes one frame of the take's channels as stereo; dropped when the
    /// ring is full (nobody is listening, or the output is behind).
    #[inline]
    pub(crate) fn push(&mut self, frame: &[f32]) {
        if self.ring.slots() < 2 {
            return;
        }
        let (l, r) = match frame {
            [m] => (*m, *m),
            [l, r, ..] => (*l, *r),
            [] => return,
        };
        let _ = self.ring.push(l);
        let _ = self.ring.push(r);
    }
}

pub(crate) fn channel(rate: u32) -> (MonitorTap, MonitorFeed) {
    let (producer, consumer) = RingBuffer::new(RING_FRAMES * 2);
    (
        MonitorTap { ring: producer },
        MonitorFeed {
            ring: consumer,
            rate,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_plays_on_both_sides_and_the_backlog_is_trimmed() {
        let (mut tap, mut feed) = channel(48_000);
        assert_eq!(feed.rate(), 48_000);
        tap.push(&[0.25]);
        tap.push(&[0.5, -0.5]);
        assert_eq!(feed.next_frame(), Some((0.25, 0.25)));
        assert_eq!(feed.next_frame(), Some((0.5, -0.5)));
        assert_eq!(feed.next_frame(), None);
        for i in 0..RING_FRAMES + 10 {
            tap.push(&[i as f32]);
        }
        // Full: the overflow was dropped, not blocked on.
        assert_eq!(feed.queued_frames(), RING_FRAMES);
        feed.trim(MAX_BACKLOG_FRAMES);
        assert_eq!(feed.queued_frames(), MAX_BACKLOG_FRAMES);
        // The newest frames are the ones kept.
        let first = feed.next_frame().unwrap().0;
        assert_eq!(first, (RING_FRAMES - MAX_BACKLOG_FRAMES) as f32);
    }
}
