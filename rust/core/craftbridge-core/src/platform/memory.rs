//! How much memory the app may still allocate, as the phone says it.
//!
//! An exporter that renders frames in parallel needs this number to size its thread pool: EffectCraft's
//! own free-memory reading has no branch for iOS or Android, so left to itself it bounds the frames in
//! flight by the thread count alone. The core reads the figure itself, because it knows nothing of
//! EffectCraft (AGENTS.md §2).
//!
//! Every decision is plain safe code that the host tests run: what a `/proc/meminfo` line says, in what
//! unit, and what a figure that is missing or cannot be read means. Only the one call into iOS is
//! `unsafe`, and it answers a number.

/// The memory the platform says the app may still allocate, in bytes; `None` when the platform gives no
/// figure, which is every desktop (a desktop export reads its own free memory, so nothing here changes
/// it), the iOS Simulator and a phone whose figure cannot be read. A missing figure is an answer, never
/// a failure: a caller that gets it applies no memory bound.
pub fn available_memory() -> Option<u64> {
    platform_figure()
}

/// iOS's own figure for the app: what it may allocate before the system's per-app limit ends it.
#[cfg(target_os = "ios")]
fn platform_figure() -> Option<u64> {
    ios_figure(app_memory_left(), cfg!(target_env = "sim"))
}

/// Android's `MemAvailable`, the kernel's estimate of what can be allocated without swapping. A file
/// that cannot be read is no figure.
#[cfg(target_os = "android")]
fn platform_figure() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo").ok().and_then(|text| parse_meminfo(&text))
}

/// The desktop reports nothing, so the caller leaves its own memory reading in charge.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn platform_figure() -> Option<u64> {
    None
}

/// `MemAvailable` from the text of `/proc/meminfo`, in bytes. The kernel writes it in kibibytes under
/// the unit `kB`; a line in any other unit, with no number or with a number too large for bytes is no
/// figure rather than a guess, and so is a report that has no such line (a kernel older than 3.14).
#[cfg(any(target_os = "android", test))]
fn parse_meminfo(text: &str) -> Option<u64> {
    let (_, value) = text.lines().filter_map(|line| line.split_once(':')).find(|(name, _)| name.trim() == "MemAvailable")?;
    let mut words = value.split_whitespace();
    let number: u64 = words.next()?.parse().ok()?;
    if words.next()? != "kB" || words.next().is_some() {
        return None;
    }
    number.checked_mul(1024)
}

/// The figure for what iOS's call answered, in the Simulator or not. Apple's header answers 0 for a
/// process that is not an app and for one past its memory limit. On a device this runs only inside an
/// app, so 0 there is an app past its limit, and a figure of 0 bytes gives it the fewest frames in
/// flight. The Simulator puts no app under a limit and answers 0 to every one, so there 0 is no figure.
#[cfg(any(target_os = "ios", test))]
fn ios_figure(answer: usize, simulator: bool) -> Option<u64> {
    u64::try_from(answer).ok().filter(|&bytes| bytes > 0 || !simulator)
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    /// `os_proc_available_memory` from `<os/proc.h>`, in the system library every iOS process links.
    #[link_name = "os_proc_available_memory"]
    fn os_proc_available_memory_raw() -> usize;
}

/// What iOS says the app may still allocate, in bytes.
#[cfg(target_os = "ios")]
fn app_memory_left() -> usize {
    // SAFETY: the function takes no arguments, reads no memory of ours and has no precondition; it only
    // returns a number.
    unsafe { os_proc_available_memory_raw() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report as Android's kernel writes it, the fields around `MemAvailable` included.
    const REPORT: &str = "MemTotal:        3860016 kB\nMemFree:          212336 kB\nMemAvailable:    1887920 kB\nBuffers:            9040 kB\n";

    #[test]
    fn mem_available_is_read_in_bytes() {
        assert_eq!(parse_meminfo(REPORT), Some(1_887_920 * 1024));
    }

    #[test]
    fn only_the_mem_available_line_counts() {
        // Neighbouring names that merely begin with it, or hold it, are other figures.
        let report = "MemAvailableExtra: 5 kB\nXMemAvailable: 6 kB\nMemFree: 7 kB\n";
        assert_eq!(parse_meminfo(report), None);
        assert_eq!(parse_meminfo("MemFree: 7 kB\nMemAvailable:  8 kB\n"), Some(8 * 1024));
    }

    #[test]
    fn a_figure_that_cannot_be_read_is_no_figure() {
        for report in [
            "",
            "MemTotal: 10 kB\n",
            "MemAvailable:\n",
            "MemAvailable: kB\n",
            "MemAvailable: lots kB\n",
            "MemAvailable: -5 kB\n",
            "MemAvailable: 12\n",
            "MemAvailable: 12 MB\n",
            "MemAvailable: 12 kB extra\n",
            "MemAvailable 12 kB\n",
            "no colon at all",
        ] {
            assert_eq!(parse_meminfo(report), None, "{report:?}");
        }
    }

    #[test]
    fn a_figure_too_large_for_bytes_is_no_figure() {
        assert_eq!(parse_meminfo("MemAvailable: 18446744073709551615 kB\n"), None);
        let largest = u64::MAX / 1024;
        assert_eq!(parse_meminfo(&format!("MemAvailable: {largest} kB\n")), Some(largest * 1024));
        assert_eq!(parse_meminfo(&format!("MemAvailable: {} kB\n", largest + 1)), None);
    }

    #[test]
    fn ios_answers_zero_means_past_the_limit_on_a_device_and_no_figure_in_the_simulator() {
        assert_eq!(ios_figure(0, false), Some(0), "a device's app past its limit has no memory to spare");
        assert_eq!(ios_figure(0, true), None, "the Simulator gives no app a limit");
        for simulator in [false, true] {
            assert_eq!(ios_figure(1, simulator), Some(1));
            assert_eq!(ios_figure(3 << 30, simulator), Some(3 << 30));
            assert_eq!(ios_figure(usize::MAX, simulator), u64::try_from(usize::MAX).ok());
        }
    }

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    #[test]
    fn the_desktop_reports_no_figure() {
        assert_eq!(available_memory(), None);
    }

    /// On a phone, the figure the platform gives is a real, plausible one: more than a page and below
    /// the address space a phone has. The numbers go to the run's output for the person reading it.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    #[test]
    fn the_phone_reports_a_figure() {
        let figure = available_memory();
        #[cfg(target_os = "ios")]
        eprintln!("os_proc_available_memory() = {}", app_memory_left());
        eprintln!("available_memory() = {figure:?}");
        if let Some(bytes) = figure {
            assert!(bytes > 4096 && bytes < (1 << 40), "{bytes}");
        }
        #[cfg(target_os = "android")]
        assert!(figure.is_some(), "an Android kernel reports MemAvailable");
    }
}
