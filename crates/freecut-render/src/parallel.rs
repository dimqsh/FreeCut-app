//! A shared, bounded pool for pixel work. Small jobs run on their caller;
//! large jobs reuse workers instead of creating OS threads for every frame.

use std::sync::OnceLock;

use rayon::prelude::*;

/// About 128 KiB of RGBA per band: keep tiles and small slates serial.
pub(crate) const MIN_BAND_PIXELS: usize = 32 * 1024;

fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(freecut_media::resources::current().pixel_threads)
            .thread_name(|i| format!("freecut-pixels-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Disjoint bands, with their index. If workers can't be started the same
/// work runs serially, preserving every pixel on resource-limited systems.
pub(crate) fn bands<T: Send>(
    data: &mut [T],
    band_len: usize,
    run: impl Fn(usize, &mut [T]) + Send + Sync,
) {
    if data.len() <= band_len {
        run(0, data);
    } else if let Some(pool) = pool() {
        pool.install(|| {
            data.par_chunks_mut(band_len)
                .enumerate()
                .for_each(|(i, band)| run(i, band));
        });
    } else {
        for (i, band) in data.chunks_mut(band_len).enumerate() {
            run(i, band);
        }
    }
}
