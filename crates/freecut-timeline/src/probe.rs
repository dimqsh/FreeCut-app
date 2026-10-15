//! Work counters for the scaling regression tests (`regressions.rs`): in
//! test builds the clip lookups count the clips they visit, per thread, so
//! a test can tell linear work from quadratic without a clock. Other builds
//! compile the counter away.

pub(crate) use imp::*;

#[cfg(not(test))]
mod imp {
    /// Counts nothing outside tests.
    #[inline(always)]
    pub(crate) fn visited(_: usize) {}
}

#[cfg(test)]
mod imp {
    use std::cell::Cell;

    thread_local! {
        static VISITED: Cell<u64> = const { Cell::new(0) };
    }

    /// Counts `n` clips visited by a lookup.
    pub(crate) fn visited(n: usize) {
        let n = u64::try_from(n).unwrap_or(u64::MAX);
        VISITED.with(|c| c.set(c.get().saturating_add(n)));
    }

    /// The count since the last call, and a fresh start.
    pub(crate) fn take() -> u64 {
        VISITED.with(|c| c.replace(0))
    }
}
