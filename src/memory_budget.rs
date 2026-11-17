//! In-process memory pressure for indexing.
//!
//! `ATLAS_MEMORY_LIMIT_MB` is also a Docker cgroup limit. This module never aborts
//! the index: under pressure it shrinks parallelism and batch size, trims the
//! allocator, and asks the coordinator to spill earlier so a full pass can finish.

use std::sync::atomic::{AtomicU64, Ordering};

/// Fraction of the limit → start cutting workers / batch size.
const ELEVATED_FRACTION: f64 = 0.55;
/// Fraction → single worker, small batches, prefer spill.
const HIGH_FRACTION: f64 = 0.70;
/// Fraction → minimal batch, trim every step (still no abort).
const CRITICAL_FRACTION: f64 = 0.85;

const FLOW_BATCH_NORMAL: usize = 512;
const FLOW_BATCH_ELEVATED: usize = 128;
const FLOW_BATCH_HIGH: usize = 32;
const FLOW_BATCH_CRITICAL: usize = 8;

const SPILL_NORMAL: usize = 50_000;
const SPILL_HIGH: usize = 10_000;
const SPILL_CRITICAL: usize = 2_000;

/// Injected usage for tests (`0` = read the real process/cgroup).
static TEST_USED_MB: AtomicU64 = AtomicU64::new(0);

/// Test-only: force reported usage in MiB. `0` restores live probes.
pub fn set_test_used_mb(mb: u64) {
    TEST_USED_MB.store(mb, Ordering::SeqCst);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    Low,
    Elevated,
    High,
    Critical,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryBudget {
    /// Cap from `ATLAS_MEMORY_LIMIT_MB`. `0` → always Low (unit tests / unset).
    pub limit_mb: u64,
}

impl MemoryBudget {
    pub fn from_limit_mb(limit_mb: u64) -> Self {
        Self { limit_mb }
    }

    /// Current process/cgroup usage in MiB when observable.
    pub fn used_mb(&self) -> Option<u64> {
        let forced = TEST_USED_MB.load(Ordering::SeqCst);
        if forced > 0 {
            return Some(forced);
        }
        cgroup_usage_mb().or_else(process_rss_mb)
    }

    pub fn pressure(&self) -> Pressure {
        let limit = self.limit_mb;
        if limit == 0 {
            return Pressure::Low;
        }
        let Some(used) = self.used_mb() else {
            return Pressure::Low;
        };
        let ratio = used as f64 / limit as f64;
        if ratio >= CRITICAL_FRACTION {
            Pressure::Critical
        } else if ratio >= HIGH_FRACTION {
            Pressure::High
        } else if ratio >= ELEVATED_FRACTION {
            Pressure::Elevated
        } else {
            Pressure::Low
        }
    }

    /// Parallel flow workers under current pressure (never zero).
    pub fn throttle_workers(&self, requested: usize) -> usize {
        let requested = requested.max(1);
        match self.pressure() {
            Pressure::Low => requested,
            Pressure::Elevated => requested.div_ceil(2).max(1),
            Pressure::High | Pressure::Critical => 1,
        }
    }

    /// Functions per flow batch — shrinks so peaks fit a tight cgroup.
    pub fn flow_batch(&self) -> usize {
        match self.pressure() {
            Pressure::Low => FLOW_BATCH_NORMAL,
            Pressure::Elevated => FLOW_BATCH_ELEVATED,
            Pressure::High => FLOW_BATCH_HIGH,
            Pressure::Critical => FLOW_BATCH_CRITICAL,
        }
    }

    /// Spill graph to disk once node count reaches this (lower under pressure).
    pub fn spill_node_threshold(&self) -> usize {
        match self.pressure() {
            Pressure::Low | Pressure::Elevated => SPILL_NORMAL,
            Pressure::High => SPILL_HIGH,
            Pressure::Critical => SPILL_CRITICAL,
        }
    }

    /// Flush occurrence chunks mid-pass once this many sit in RAM.
    pub fn occurrence_flush_at(&self) -> usize {
        match self.pressure() {
            Pressure::Low => SPILL_NORMAL,
            Pressure::Elevated => 5_000,
            Pressure::High => 1_000,
            Pressure::Critical => 1,
        }
    }

    /// Trim after every batch when pressure is elevated or worse.
    pub fn should_trim(&self) -> bool {
        self.pressure() >= Pressure::Elevated
    }
}

/// Ask glibc to return free arenas to the OS (no-op off Linux).
pub fn trim_allocator() {
    #[cfg(target_os = "linux")]
    unsafe {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        let _ = malloc_trim(0);
    }
}

fn process_rss_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kb / 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn cgroup_usage_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(bytes) = std::fs::read_to_string("/sys/fs/cgroup/memory.current") {
            if let Ok(n) = bytes.trim().parse::<u64>() {
                return Some(n / (1024 * 1024));
            }
        }
        if let Ok(bytes) = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes") {
            if let Ok(n) = bytes.trim().parse::<u64>() {
                return Some(n / (1024 * 1024));
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_throttles_but_never_aborts() {
        let budget = MemoryBudget::from_limit_mb(1000);
        set_test_used_mb(400);
        assert_eq!(budget.pressure(), Pressure::Low);
        assert_eq!(budget.throttle_workers(8), 8);
        assert_eq!(budget.flow_batch(), FLOW_BATCH_NORMAL);

        set_test_used_mb(600);
        assert_eq!(budget.pressure(), Pressure::Elevated);
        assert_eq!(budget.throttle_workers(8), 4);
        assert_eq!(budget.flow_batch(), FLOW_BATCH_ELEVATED);

        set_test_used_mb(750);
        assert_eq!(budget.pressure(), Pressure::High);
        assert_eq!(budget.throttle_workers(8), 1);
        assert_eq!(budget.flow_batch(), FLOW_BATCH_HIGH);
        assert_eq!(budget.spill_node_threshold(), SPILL_HIGH);

        set_test_used_mb(950);
        assert_eq!(budget.pressure(), Pressure::Critical);
        assert_eq!(budget.throttle_workers(8), 1);
        assert_eq!(budget.flow_batch(), FLOW_BATCH_CRITICAL);
        assert_eq!(budget.spill_node_threshold(), SPILL_CRITICAL);
        assert_eq!(budget.occurrence_flush_at(), 1);
        set_test_used_mb(0);
    }

    #[test]
    fn zero_limit_is_always_low() {
        set_test_used_mb(9_000);
        let budget = MemoryBudget::default();
        assert_eq!(budget.pressure(), Pressure::Low);
        assert_eq!(budget.throttle_workers(4), 4);
        set_test_used_mb(0);
    }
}
