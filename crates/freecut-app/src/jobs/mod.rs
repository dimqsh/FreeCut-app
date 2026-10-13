//! Background work.
//!
//! - [`Jobs`]: a small pool of worker threads for blocking work: probing,
//!   opening, saving, thumbnails. The UI thread submits closures and polls
//!   finished results once per frame; it never blocks on a job.
//! - [`export`]: the export job (EX-022), which runs on threads of its own.
//! - [`progress`]: its lock-free progress cell, ETA window and the text the
//!   progress and result views show.
//! - [`platform`]: blocking OS calls for the export dialog (destination
//!   checks, "Open" and "Show in folder"), run on the pool.

pub mod export;
pub mod platform;
pub mod progress;

use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};

type Task<T> = Box<dyn FnOnce() -> T + Send + 'static>;

/// Runs closures on worker threads and hands their results back.
pub struct Jobs<T: Send + 'static> {
    tasks: Option<Sender<Task<T>>>,
    results: Receiver<T>,
    workers: Vec<JoinHandle<()>>,
    in_flight: usize,
}

impl<T: Send + 'static> std::fmt::Debug for Jobs<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs")
            .field("workers", &self.workers.len())
            .field("in_flight", &self.in_flight)
            .finish()
    }
}

impl<T: Send + 'static> Jobs<T> {
    /// Starts `workers` threads. `wake` is called after each finished job
    /// (the app passes a repaint request).
    pub fn new(workers: usize, wake: impl Fn() + Send + Sync + Clone + 'static) -> Jobs<T> {
        let (task_tx, task_rx) = unbounded::<Task<T>>();
        let (result_tx, result_rx) = unbounded::<T>();
        let workers = (0..workers.max(1))
            .filter_map(|i| {
                let tasks = task_rx.clone();
                let results = result_tx.clone();
                let wake = wake.clone();
                std::thread::Builder::new()
                    .name(format!("freecut-job-{i}"))
                    .spawn(move || {
                        // Ends when every task sender is dropped.
                        for task in tasks {
                            if results.send(task()).is_err() {
                                break;
                            }
                            wake();
                        }
                    })
                    .ok()
            })
            .collect();
        Jobs {
            tasks: Some(task_tx),
            results: result_rx,
            workers,
            in_flight: 0,
        }
    }

    /// Queues `task`. Returns false if the pool has shut down.
    pub fn spawn(&mut self, task: impl FnOnce() -> T + Send + 'static) -> bool {
        let sent = self
            .tasks
            .as_ref()
            .is_some_and(|tx| tx.send(Box::new(task)).is_ok());
        if sent {
            self.in_flight += 1;
        }
        sent
    }

    /// Finished results, without blocking.
    pub fn poll(&mut self) -> Vec<T> {
        let done: Vec<T> = self.results.try_iter().collect();
        self.in_flight = self.in_flight.saturating_sub(done.len());
        done
    }
}

impl<T: Send + 'static> Drop for Jobs<T> {
    fn drop(&mut self) {
        // Closing the task channel lets idle workers exit; running jobs
        // finish first. Never kill a thread mid-write.
        self.tasks = None;
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// A sensible pool size: half the cores, between 2 and 4.
pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(2)
        .clamp(2, 4)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for<T: Send + 'static>(jobs: &mut Jobs<T>, n: usize) -> Vec<T> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(jobs.poll());
            std::thread::sleep(Duration::from_millis(1));
        }
        out
    }

    #[test]
    fn runs_jobs_off_thread_and_returns_results() {
        let mut jobs: Jobs<u32> = Jobs::new(2, || {});
        for i in 0..10 {
            assert!(jobs.spawn(move || i * 2));
        }
        let mut got = wait_for(&mut jobs, 10);
        got.sort_unstable();
        assert_eq!(got, (0..10).map(|i| i * 2).collect::<Vec<_>>());
        assert_eq!(jobs.in_flight, 0);
    }

    #[test]
    fn dropping_the_pool_waits_for_running_jobs() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut jobs: Jobs<()> = Jobs::new(1, || {});
            let f = flag.clone();
            jobs.spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                f.store(true, std::sync::atomic::Ordering::SeqCst);
            });
        }
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }
}
