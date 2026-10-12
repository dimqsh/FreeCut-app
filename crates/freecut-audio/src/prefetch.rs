//! Opening readers ahead of playback, off the feeder thread (AU-010).
//!
//! Opening a source (probing a file, building its decoder) and positioning
//! it can take longer than the feeder's whole read-ahead; done on the feeder
//! it would starve the device. The feeder instead asks for the readers its
//! next couple of seconds need ([`Request`]), a worker thread opens and
//! positions them ([`SourceReader::prepare`]), and the feeder hands each one
//! to the mixer as it arrives ([`Ready`]).
//!
//! Requests carry an epoch. When the feeder moves on (a seek, a new opener)
//! it bumps the epoch, and the worker skips requests made before.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender};
use freecut_timeline::time::{SampleRate, Tick};

use crate::source::{SourceKey, SourceOpener, SourceReader};

/// A reader to open and position.
pub(crate) struct Request {
    pub epoch: u64,
    /// The mixer input it is for.
    pub key: u64,
    pub source: SourceKey,
    /// Media time of the first read it should be ready for.
    pub at: Tick,
    pub rate: SampleRate,
    pub opener: Arc<dyn SourceOpener>,
}

/// An opened reader (or `None`: the source can't be opened), for the
/// request with the same fields.
pub(crate) struct Ready {
    pub key: u64,
    pub source: SourceKey,
    pub at: Tick,
    pub rate: SampleRate,
    pub opener: Arc<dyn SourceOpener>,
    pub reader: Option<Box<dyn SourceReader>>,
}

/// Opens and positions the reader `req` asks for.
fn serve(req: Request) -> Ready {
    let mut reader = req.opener.open(&req.source, req.rate);
    if let Some(r) = reader.as_mut()
        && let Err(e) = r.prepare(req.at)
    {
        tracing::warn!(key = req.key, error = %e, "audio source can't be positioned; playing silence");
        reader = None;
    }
    Ready {
        key: req.key,
        source: req.source,
        at: req.at,
        rate: req.rate,
        opener: req.opener,
        reader,
    }
}

/// The feeder's side of the prefetch worker.
pub(crate) struct Prefetcher {
    requests: Sender<Request>,
    ready: Receiver<Ready>,
    epoch: Arc<AtomicU64>,
    /// Test mode: requests wait in this inbox until served by hand.
    #[cfg(test)]
    manual: Option<Manual>,
}

#[cfg(test)]
struct Manual {
    inbox: Receiver<Request>,
    outbox: Sender<Ready>,
    held: bool,
}

impl Prefetcher {
    /// Starts the worker thread. It exits once the prefetcher is dropped
    /// and the open in progress, if any, is done.
    pub(crate) fn spawn() -> std::io::Result<Prefetcher> {
        let (requests, inbox) = crossbeam_channel::unbounded::<Request>();
        let (outbox, ready) = crossbeam_channel::unbounded();
        let epoch = Arc::new(AtomicU64::new(0));
        let current = Arc::clone(&epoch);
        std::thread::Builder::new()
            .name("freecut-audio-prefetch".to_owned())
            .spawn(move || {
                for req in inbox {
                    if req.epoch < current.load(Ordering::Acquire) {
                        continue;
                    }
                    if outbox.send(serve(req)).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Prefetcher {
            requests,
            ready,
            epoch,
            #[cfg(test)]
            manual: None,
        })
    }

    /// A prefetcher without a thread: requests are served by
    /// [`Prefetcher::serve_queued`].
    #[cfg(test)]
    pub(crate) fn manual() -> Prefetcher {
        let (requests, inbox) = crossbeam_channel::unbounded();
        let (outbox, ready) = crossbeam_channel::unbounded();
        Prefetcher {
            requests,
            ready,
            epoch: Arc::new(AtomicU64::new(0)),
            manual: Some(Manual {
                inbox,
                outbox,
                held: false,
            }),
        }
    }

    /// The current epoch, to tag requests with.
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Starts a new epoch: requests made before are skipped.
    pub(crate) fn next_epoch(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn request(&self, req: Request) {
        // Fails only once the worker is gone, when nothing plays anyway.
        let _ = self.requests.send(req);
    }

    /// The readers opened since the last call.
    pub(crate) fn ready(&self) -> impl Iterator<Item = Ready> + '_ {
        self.ready.try_iter()
    }

    /// Test mode: serves every queued request now, unless held.
    #[cfg(test)]
    pub(crate) fn serve_queued(&self) {
        let Some(m) = &self.manual else {
            return;
        };
        if m.held {
            return;
        }
        for req in m.inbox.try_iter() {
            if req.epoch >= self.epoch() {
                let _ = m.outbox.send(serve(req));
            }
        }
    }

    /// Test mode: while held, requests wait (a slow disk).
    #[cfg(test)]
    pub(crate) fn hold(&mut self, held: bool) {
        if let Some(m) = &mut self.manual {
            m.held = held;
        }
    }
}
