//! Cooperative cancellation for blocking media work.
//!
//! Every long-running call in this crate (probing, decoding a span, building
//! waveform peaks) checks a [`CancelToken`] between packets. The same token is
//! installed as FFmpeg's I/O interrupt callback, so a read that stalls inside
//! FFmpeg (a network share, a cloud placeholder) aborts too.
//!
//! A token fires when [`CancelToken::cancel`] is called on it or on any of its
//! clones, or when its optional deadline passes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Why a token reports itself as cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// [`CancelToken::cancel`] was called.
    Cancelled,
    /// The deadline set with [`CancelToken::with_deadline`] passed.
    DeadlinePassed,
}

/// A cheap, clone-able cancellation flag with an optional deadline.
///
/// Clones share the flag: cancelling one cancels all. A deadline is per
/// clone, so a job can derive a time-limited token from a shared one.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl CancelToken {
    /// A token that fires only when cancelled.
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// Requests cancellation. Idempotent; visible to every clone.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// A clone sharing this token's flag that also fires at `deadline`.
    #[must_use]
    pub fn with_deadline(&self, deadline: Instant) -> CancelToken {
        CancelToken {
            flag: Arc::clone(&self.flag),
            deadline: Some(deadline),
        }
    }

    /// A clone sharing this token's flag that also fires `timeout` from now.
    /// A timeout too large to represent means "no deadline".
    #[must_use]
    pub fn with_timeout(&self, timeout: Duration) -> CancelToken {
        match Instant::now().checked_add(timeout) {
            Some(deadline) => self.with_deadline(deadline),
            None => self.clone(),
        }
    }

    /// True once the token was cancelled or its deadline passed.
    pub fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }

    /// Why the token fired, or `None` while it hasn't.
    pub fn reason(&self) -> Option<CancelReason> {
        if self.flag.load(Ordering::Acquire) {
            return Some(CancelReason::Cancelled);
        }
        match self.deadline {
            Some(d) if Instant::now() >= d => Some(CancelReason::DeadlinePassed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert_eq!(b.reason(), Some(CancelReason::Cancelled));
    }

    #[test]
    fn a_passed_deadline_fires_only_on_that_clone() {
        let shared = CancelToken::new();
        let timed = shared.with_deadline(Instant::now());
        assert_eq!(timed.reason(), Some(CancelReason::DeadlinePassed));
        assert!(!shared.is_cancelled());
        shared.cancel();
        assert_eq!(timed.reason(), Some(CancelReason::Cancelled));
    }

    #[test]
    fn a_huge_timeout_means_no_deadline() {
        let t = CancelToken::new().with_timeout(Duration::MAX);
        assert!(!t.is_cancelled());
    }
}
