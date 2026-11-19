//! High-resolution timer and time stamps (`upstream/src/util/hrtimer.*` and the
//! Windows time functions the original uses).

use std::time::{SystemTime, UNIX_EPOCH};

/// Which timer is used (`enum class HighResTimer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerKind {
    /// Wall-clock time only (very coarse).
    None,
    /// CPU time-stamp counter (`RDTSC`).
    Rdtsc,
    /// OS performance counter (`QueryPerformanceCounter`); on Linux
    /// `CLOCK_MONOTONIC_RAW` in nanoseconds.
    Qpc,
}

/// Timer selected on this machine (`HighResTimerCheck()`).
pub fn timer_kind() -> TimerKind {
    if cfg!(any(target_arch = "x86_64", target_arch = "x86")) {
        let a = read_timer(TimerKind::Rdtsc);
        let mut v: u32 = 0xdeadbeef;
        for i in 0..10u32 {
            v = (v << 5).wrapping_add(v).wrapping_add(i);
        }
        std::hint::black_box(v);
        if read_timer(TimerKind::Rdtsc) != a {
            return TimerKind::Rdtsc;
        }
    }
    TimerKind::Qpc
}

fn read_timer(kind: TimerKind) -> u64 {
    match kind {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: RDTSC is available on every x86_64 CPU.
        TimerKind::Rdtsc => unsafe { core::arch::x86_64::_rdtsc() },
        #[cfg(target_arch = "x86")]
        TimerKind::Rdtsc => unsafe { core::arch::x86::_rdtsc() },
        TimerKind::None => filetime_now(),
        _ => monotonic_ns(),
    }
}

/// Current value of the high-resolution timer (`HighResTimer()`).
pub fn high_res_timer() -> u64 {
    static KIND: std::sync::OnceLock<TimerKind> = std::sync::OnceLock::new();
    read_timer(*KIND.get_or_init(timer_kind))
}

/// Nanoseconds of `CLOCK_MONOTONIC_RAW`.
pub fn monotonic_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC_RAW)
}

/// Nanoseconds of the given POSIX clock.
pub fn clock_ns(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(clock, &mut ts) };
    (ts.tv_sec as u64).wrapping_mul(1_000_000_000).wrapping_add(ts.tv_nsec as u64)
}

/// Number of 100-ns intervals between 1601-01-01 and 1970-01-01.
pub const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// Current UTC time as Windows `FILETIME` (`GetSystemTimeAsFileTime()`).
pub fn filetime_now() -> u64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    FILETIME_UNIX_EPOCH + d.as_secs() * 10_000_000 + (d.subsec_nanos() / 100) as u64
}

/// Current *local* time as `FILETIME`, the way the original creates database
/// time stamps (`GetLocalTime()` + `SystemTimeToFileTime()`): the local wall
/// clock time is stored as if it were UTC.
pub fn local_filetime_now() -> u64 {
    let utc = filetime_now();
    let secs = ((utc - FILETIME_UNIX_EPOCH) / 10_000_000) as libc::time_t;
    let offset = utc_offset_seconds(secs);
    // GetLocalTime() has millisecond resolution.
    let local = (utc as i64 + offset * 10_000_000) as u64;
    local - local % 10_000
}

/// Offset of local time from UTC in seconds at the given Unix time.
pub fn utc_offset_seconds(unix_secs: libc::time_t) -> i64 {
    // SAFETY: localtime_r with valid pointers.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&unix_secs, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff
    }
}

/// Simple stopwatch (`class Stopwatch`).
pub struct Stopwatch(std::time::Instant);

impl Stopwatch {
    pub fn new() -> Self {
        Self(std::time::Instant::now())
    }
    pub fn reset(&mut self) {
        self.0 = std::time::Instant::now();
    }
    pub fn elapsed_seconds(&self) -> f64 {
        self.0.elapsed().as_secs_f64()
    }
    /// Return elapsed time and restart.
    pub fn tick(&mut self) -> std::time::Duration {
        let now = std::time::Instant::now();
        let d = now - self.0;
        self.0 = now;
        d
    }
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::new()
    }
}
