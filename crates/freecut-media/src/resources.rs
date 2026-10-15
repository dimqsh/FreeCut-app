//! Process-wide hardware budget, shared by media, render and the app.
//! Query RAM once, without collecting processes, disks or network state.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
/// Limits on retained data and concurrently active CPU workers.
pub struct Resources {
    /// Small caches for machines with at most 4 GiB, or a forced compact run.
    pub low_memory: bool,
    /// Shared compositor worker count.
    pub pixel_threads: usize,
    /// FFmpeg frame threads per video decoder.
    pub decoder_threads: usize,
    /// Retained decoders when fewer sources are visible at once.
    pub idle_decoders: usize,
}

impl Resources {
    /// Unknown RAM is treated conservatively. A small machine keeps CPU
    /// workers and FFmpeg's reference-frame queues small as well as caches.
    pub fn for_machine(ram: u64, cores: usize, compact: bool) -> Self {
        let low_memory = compact || ram <= 4 * 1024 * 1024 * 1024;
        Self {
            low_memory,
            pixel_threads: cores.clamp(1, if low_memory { 2 } else { 8 }),
            decoder_threads: cores.clamp(1, if low_memory { 1 } else { 4 }),
            idle_decoders: if low_memory { 1 } else { 4 },
        }
    }

    /// A cache's budget in bytes. The compact budget controls retention,
    /// never the resolution or precision of the frame being rendered.
    pub fn cache_bytes(self, normal_mib: usize, compact_mib: usize) -> usize {
        (if self.low_memory {
            compact_mib
        } else {
            normal_mib
        }) << 20
    }
}

/// Hardware policy detected once per process, before workers are started.
pub fn current() -> &'static Resources {
    static RESOURCES: OnceLock<Resources> = OnceLock::new();
    RESOURCES.get_or_init(|| {
        let mut system = sysinfo::System::new();
        system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
        #[allow(unused_mut)] // Linux containers may have less RAM than the host.
        let mut ram = system.total_memory();
        #[cfg(target_os = "linux")]
        for path in [
            "/sys/fs/cgroup/memory.max",
            "/sys/fs/cgroup/memory/memory.limit_in_bytes",
        ] {
            if let Some(limit) = std::fs::read_to_string(path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .filter(|&n| n > 0)
            {
                ram = ram.min(limit);
            }
        }
        let compact = std::env::var_os("FREECUT_LOW_MEMORY").is_some_and(|v| v == "1")
            || std::env::args_os().any(|arg| arg == "--low-memory" || arg == "--software");
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        Resources::for_machine(ram, cores, compact)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_gib_and_unknown_memory_keep_small_budgets_on_many_core_machines() {
        for ram in [0, 2 << 30, 4 << 30] {
            let limits = Resources::for_machine(ram, 16, false);
            assert!(limits.low_memory);
            assert_eq!(limits.pixel_threads, 2);
            assert_eq!(limits.decoder_threads, 1);
            assert_eq!(limits.idle_decoders, 1);
            assert_eq!(limits.cache_bytes(64, 8), 8 << 20);
        }
        assert_eq!(Resources::for_machine(2 << 30, 1, false).pixel_threads, 1);
        assert!(!Resources::for_machine(8 << 30, 4, false).low_memory);
        assert!(Resources::for_machine(32 << 30, 4, true).low_memory);
    }
}
