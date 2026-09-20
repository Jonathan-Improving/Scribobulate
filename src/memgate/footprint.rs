//! Process footprint sampler — one function, three `cfg` bodies.
//!
//! The returned number is **not RSS**. Linux reads `VmRSS`, macOS reads
//! `ri_phys_footprint`, Windows reads `WorkingSetSize`. A shared name invites
//! a shared threshold; tolerances live next to this module as per-platform
//! constants.

/// Per-platform tolerance for [`super::slope::assert_flat`], in bytes.
///
/// Chosen as a *shape* bound after a measured clean baseline, never as one
/// shared number. Freed pages stay with the allocator, so a few hundred KiB of
/// second-half noise is not a leak; a climb of megabytes per render is.
pub(crate) const TOLERANCE_BYTES: u64 = {
    #[cfg(target_os = "linux")]
    {
        2 * 1024 * 1024
    }
    #[cfg(target_os = "macos")]
    {
        4 * 1024 * 1024
    }
    #[cfg(windows)]
    {
        4 * 1024 * 1024
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        4 * 1024 * 1024
    }
};

/// Warm-up renders discarded before the slope is taken. Windows measured its
/// entire 1.09 MB of warm-up arriving at iteration 2; three covers that and
/// the GTK icon-cache / font first-paint on the other seats.
pub(crate) const WARMUP: usize = 3;

/// Samples collected *including* warm-up. After discarding [`WARMUP`] this
/// leaves ten readings, five per half.
pub(crate) const SAMPLE_COUNT: usize = WARMUP + 10;

/// Serialises every footprint measurement in the process.
///
/// **The instrument is process-wide and the failure direction is the reassuring one.**
/// `current()` reads the whole process's footprint, so a foreign allocation made while
/// a series is being collected lands in that series. If it arrives in the first half it
/// raises the baseline and FLATTENS the slope — the gate then passes on a real leak,
/// which is the reading nobody investigates.
///
/// That is reachable because these bodies register with both harnesses: the
/// `harness = false` suite runs them one at a time on the main thread, and the ordinary
/// libtest binary runs them in parallel threads. The runner script pins the first; it
/// cannot pin the second, and a claim about a shared instrument that depends on which
/// harness invoked it is not a claim.
///
/// Same shape, and the same cause, as the counting allocator in richimg's
/// oversized-allocation targets: contention on a shared instrument presents as a null
/// reading, and here the null reading is "no growth".
#[cfg(all(test, feature = "memory-gates"))]
static MEASUREMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Claim the instrument for this test's whole body, and release it on drop.
///
/// A guard rather than a wrapper around the sampling loop, deliberately: building the
/// window, decoding the fixture and pumping the loop all allocate, and a series whose
/// BASELINE was taken while another test was allocating is as wrong as one whose
/// samples were. The measured region is the test.
///
/// Poisoning is ignored: a failing assertion inside one series must not turn every
/// later one into a second, misleading failure.
#[cfg(all(test, feature = "memory-gates"))]
#[must_use = "the instrument is only claimed while the guard is alive"]
pub(crate) fn measuring() -> MeasurementGuard {
    MeasurementGuard {
        _lock: MEASUREMENT.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

#[cfg(all(test, feature = "memory-gates"))]
pub(crate) struct MeasurementGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

/// Current process footprint in bytes, or `None` if this platform's sampler
/// could not read it. A `None` is a broken instrument, not a zero — the
/// caller must refuse rather than treat it as a flat series.
pub(crate) fn current() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux_vmrss_bytes()
    }
    #[cfg(target_os = "macos")]
    {
        macos_phys_footprint()
    }
    #[cfg(windows)]
    {
        crate::platform::win32::process::current_working_set_bytes()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn linux_vmrss_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_vmrss_kb(&text).map(|kb| kb.saturating_mul(1024))
}

/// Parse `VmRSS:` from a `/proc/<pid>/status` blob. Split from the file read
/// so the grammar is unit-tested with no `/proc`.
#[cfg(target_os = "linux")]
fn parse_vmrss_kb(status: &str) -> Option<u64> {
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        return rest.split_whitespace().next()?.parse::<u64>().ok();
    }
    None
}

/// macOS physical footprint via `proc_pid_rusage(RUSAGE_INFO_V2)`.
///
/// `RUSAGE_INFO_V2` is the earliest flavour that carries `ri_phys_footprint`.
/// Later flavours add fields this gate does not use.
///
/// ⚠ This number is a high-water of pages the zone still holds, not "bytes
/// currently referenced". macOS malloc keeps freed pages: a 256 MB allocation
/// dropped moved the reading by nothing. Never write a single-shot
/// "allocate, free, assert this came back" against it.
#[cfg(target_os = "macos")]
fn macos_phys_footprint() -> Option<u64> {
    // SAFETY: `rusage_info_v2` is the buffer `RUSAGE_INFO_V2` writes; a zeroed
    // struct is a valid empty starting point, and `getpid` is this process.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            std::ptr::addr_of_mut!(info) as *mut libc::rusage_info_t,
        );
        if rc == 0 {
            Some(info.ri_phys_footprint)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::current;

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_vmrss_reads_the_kb_field() {
        let blob = "Name:\tfoo\nVmPeak:\t999 kB\nVmRSS:\t  1234 kB\nVmData:\t1 kB\n";
        assert_eq!(super::parse_vmrss_kb(blob), Some(1234));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_vmrss_none_when_the_field_is_absent() {
        assert_eq!(super::parse_vmrss_kb("Name:\tfoo\nVmPeak:\t9 kB\n"), None);
    }

    #[test]
    fn current_returns_a_nonzero_reading_on_this_host() {
        let n = current().expect("footprint sampler must work on a supported host");
        assert!(
            n > 0,
            "a zero footprint is a dark instrument, not a reading"
        );
    }

    #[test]
    fn sample_shape_and_tolerance_are_the_stated_constants() {
        assert_eq!(super::SAMPLE_COUNT, super::WARMUP + 10);
        #[cfg(target_os = "linux")]
        assert_eq!(super::TOLERANCE_BYTES, 2 * 1024 * 1024);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(super::TOLERANCE_BYTES, 4 * 1024 * 1024);
    }
}
