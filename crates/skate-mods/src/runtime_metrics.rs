use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

/// Measurements for one resource generation. Inclusive durations include nested
/// exports and IPC waits. Queue bytes use host accounting, not wire byte counts.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RuntimeMetrics {
    pub generation: u64,
    pub language: String,
    pub running: bool,
    pub invocations: u64,
    pub errors: u64,
    pub last_error: Option<String>,
    pub last_phase: String,
    pub last_wall_time_us: u64,
    pub total_wall_time_us: u64,
    pub max_wall_time_us: u64,
    /// CPU consumed by the calling native host thread; excludes C# worker CPU.
    pub last_host_cpu_time_us: Option<u64>,
    pub total_host_cpu_time_us: Option<u64>,
    pub max_host_cpu_time_us: Option<u64>,
    pub last_budget_units: usize,
    pub max_budget_units: usize,
    pub lua_heap_bytes: Option<usize>,
    pub javascript_heap_bytes: Option<usize>,
    pub managed_resident_bytes: Option<usize>,
    pub queued_outputs: usize,
    pub queued_output_accounted_bytes: usize,
}

pub(crate) type Counters = Arc<Mutex<RuntimeMetrics>>;

/// Error messages cross VM boundaries and may be much larger than their useful
/// diagnostic prefix. Bound UTF-8 bytes while formatting, before retaining them.
pub(crate) fn bounded_error(value: impl std::fmt::Display) -> String {
    use std::fmt::Write;
    struct Limited(String);
    impl std::fmt::Write for Limited {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let mut end = text.len().min(2048 - self.0.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.0.push_str(&text[..end]);
            if end < text.len() {
                Err(std::fmt::Error)
            } else {
                Ok(())
            }
        }
    }
    let mut limited = Limited(String::new());
    let _ = write!(limited, "{value}");
    limited.0
}

pub(crate) struct Timer {
    wall: Instant,
    cpu: Option<u64>,
}
impl Timer {
    pub(crate) fn start() -> Self {
        Self {
            wall: Instant::now(),
            cpu: thread_cpu_us(),
        }
    }
    pub(crate) fn record(
        self,
        counters: &Counters,
        phase: &str,
        budget_units: usize,
        error: Option<&str>,
    ) {
        let wall = self.wall.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let cpu = self
            .cpu
            .zip(thread_cpu_us())
            .map(|(before, after)| after.saturating_sub(before));
        let mut metrics = counters.lock().unwrap();
        metrics.invocations = metrics.invocations.saturating_add(1);
        metrics.last_phase = phase.chars().take(128).collect();
        metrics.last_wall_time_us = wall;
        metrics.total_wall_time_us = metrics.total_wall_time_us.saturating_add(wall);
        metrics.max_wall_time_us = metrics.max_wall_time_us.max(wall);
        metrics.last_host_cpu_time_us = cpu;
        if let Some(cpu) = cpu {
            metrics.total_host_cpu_time_us = Some(
                metrics
                    .total_host_cpu_time_us
                    .unwrap_or(0)
                    .saturating_add(cpu),
            );
            metrics.max_host_cpu_time_us = Some(metrics.max_host_cpu_time_us.unwrap_or(0).max(cpu));
        }
        metrics.last_budget_units = budget_units;
        metrics.max_budget_units = metrics.max_budget_units.max(budget_units);
        if let Some(error) = error {
            metrics.errors = metrics.errors.saturating_add(1);
            metrics.last_error = Some(bounded_error(error));
        }
    }
}

#[cfg(target_os = "linux")]
fn thread_cpu_us() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes only to this initialized timespec; the clock
    // identifies the calling thread and exposes no mutable process state.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return None;
    }
    Some(
        (time.tv_sec as u64)
            .saturating_mul(1_000_000)
            .saturating_add(time.tv_nsec as u64 / 1000),
    )
}

#[cfg(target_os = "windows")]
fn thread_cpu_us() -> Option<u64> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // SAFETY: the pseudo handle represents the current thread; all four output
    // pointers refer to initialized FILETIME values for this call's duration.
    if unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return None;
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Some(ticks(kernel).saturating_add(ticks(user)) / 10)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn thread_cpu_us() -> Option<u64> {
    None
}
