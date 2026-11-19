//! How much memory the app may still allocate, as the platform says it.
//!
//! An exporter that renders frames needs this number to size its thread pool, and to refuse a frame the
//! memory cannot hold at all before it renders: EffectCraft's own free-memory reading has no branch for
//! iOS or Android, so left to itself it bounds the frames in flight by the thread count alone. The core
//! reads the figure itself, because it knows nothing of EffectCraft (AGENTS.md §2).
//!
//! Every platform answers its own figure or the reason it has none, never a figure made up in its place:
//! a caller holding work to the memory cannot tell a guess from a reading, and a guess too high lets the
//! system end the app mid-export. Every decision is plain safe code that the host tests run: what a
//! `/proc/meminfo` line says and in what unit, how the kernel's answers read, and the words of each
//! failure. Only the calls into the system, which answer numbers, are `unsafe`.

/// The memory the platform says the app may still allocate, in bytes:
///
/// - on an iPhone, `os_proc_available_memory()`: what the app may allocate before the system's per-app
///   limit ends it, 0 for an app already past it;
/// - on Android and on Linux, the kernel's `MemAvailable`: its estimate of what can be allocated without
///   swapping;
/// - on a Mac, and in the iOS Simulator, which holds no app to a limit of its own and runs on the Mac's
///   memory, the host's own figure: the share of its memory the kernel counts as available
///   (`kern.memorystatus_level`, in whole percents of `hw.memsize`).
///
/// # Errors
///
/// Why the platform gives no figure, in words: its report cannot be read or holds no figure, or the
/// platform is none of the above.
pub fn available_memory() -> Result<u64, String> {
    platform_figure()
}

/// An iPhone's own figure for the app.
#[cfg(all(target_os = "ios", not(target_env = "sim")))]
fn platform_figure() -> Result<u64, String> {
    let answer = app_memory_left();
    u64::try_from(answer).map_err(|_| format!("os_proc_available_memory answered {answer}, more bytes than a figure counts"))
}

/// The kernel's `MemAvailable`, from the report every Android and Linux kernel since 3.14 writes.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn platform_figure() -> Result<u64, String> {
    meminfo_figure(std::fs::read_to_string(MEMINFO))
}

/// The host's own figure, on a Mac and in the iOS Simulator.
#[cfg(any(target_os = "macos", all(target_os = "ios", target_env = "sim")))]
fn platform_figure() -> Result<u64, String> {
    host_figure(sysctl_number(c"kern.memorystatus_level")?, sysctl_number(c"hw.memsize")?)
}

/// A platform none of the above: it has no figure this module knows how to ask for.
#[cfg(not(any(target_os = "ios", target_os = "android", target_os = "linux", target_os = "macos")))]
fn platform_figure() -> Result<u64, String> {
    Err("this platform gives no figure for the memory an app may still allocate".to_string())
}

/// Where Android's and Linux's kernels report their memory.
#[cfg(any(target_os = "android", target_os = "linux", test))]
const MEMINFO: &str = "/proc/meminfo";

/// The figure a read of [`MEMINFO`] gives, or why it gives none: a report the system would not let the
/// app read, or one with no `MemAvailable` figure in kibibytes.
#[cfg(any(target_os = "android", target_os = "linux", test))]
fn meminfo_figure(read: std::io::Result<String>) -> Result<u64, String> {
    let text = read.map_err(|error| format!("{MEMINFO} could not be read: {error}"))?;
    parse_meminfo(&text).ok_or_else(|| format!("{MEMINFO} holds no MemAvailable figure in kB"))
}

/// `MemAvailable` from the text of `/proc/meminfo`, in bytes. The kernel writes it in kibibytes under
/// the unit `kB`; a line in any other unit, with no number or with a number too large for bytes is no
/// figure rather than a guess, and so is a report that has no such line (a kernel older than 3.14).
#[cfg(any(target_os = "android", target_os = "linux", test))]
fn parse_meminfo(text: &str) -> Option<u64> {
    let (_, value) = text.lines().filter_map(|line| line.split_once(':')).find(|(name, _)| name.trim() == "MemAvailable")?;
    let mut words = value.split_whitespace();
    let number: u64 = words.next()?.parse().ok()?;
    if words.next()? != "kB" || words.next().is_some() {
        return None;
    }
    number.checked_mul(1024)
}

/// The host's available memory from the kernel's two answers: `level` percent of its `total` bytes. A
/// share above the whole is no figure the kernel could mean.
#[cfg(any(target_os = "macos", all(target_os = "ios", target_env = "sim"), test))]
fn host_figure(level: u64, total: u64) -> Result<u64, String> {
    if level > 100 {
        return Err(format!("kern.memorystatus_level reads {level} percent, more than the whole memory"));
    }
    // A share of at most the whole of a 64-bit number is itself one.
    u64::try_from(u128::from(total) * u128::from(level) / 100).map_err(|_| format!("{level} percent of {total} bytes is more bytes than a figure counts"))
}

/// The number the kernel answers for the sysctl `name`, which it writes as an `int` or a 64-bit quad.
#[cfg(any(target_os = "macos", all(target_os = "ios", target_env = "sim")))]
fn sysctl_number(name: &std::ffi::CStr) -> Result<u64, String> {
    let mut answer = [0_u8; 8];
    let mut size = answer.len();
    // SAFETY: `name` is a NUL-terminated string the call only reads; the call writes at most `size`
    // bytes into `answer`, which holds that many, and sets `size` to the bytes it wrote; a null new
    // value of length 0 sets nothing.
    let status = unsafe { libc::sysctlbyname(name.as_ptr(), answer.as_mut_ptr().cast(), &raw mut size, std::ptr::null_mut(), 0) };
    if status != 0 {
        return Err(format!("the kernel did not answer {}: {}", name.to_string_lossy(), std::io::Error::last_os_error()));
    }
    sysctl_value(answer, size).ok_or_else(|| format!("the kernel answered {} in {size} bytes, which is no number", name.to_string_lossy()))
}

/// The number in a sysctl's answer, in the machine's own byte order: its first four bytes for an `int`,
/// all eight for a quad, and no number for an answer of any other size.
#[cfg(any(target_os = "macos", all(target_os = "ios", target_env = "sim"), test))]
fn sysctl_value(answer: [u8; 8], size: usize) -> Option<u64> {
    match size {
        4 => answer.first_chunk::<4>().map(|int| u64::from(u32::from_ne_bytes(*int))),
        8 => Some(u64::from_ne_bytes(answer)),
        _ => None,
    }
}

#[cfg(all(target_os = "ios", not(target_env = "sim")))]
unsafe extern "C" {
    /// `os_proc_available_memory` from `<os/proc.h>`, in the system library every iOS process links.
    #[link_name = "os_proc_available_memory"]
    fn os_proc_available_memory_raw() -> usize;
}

/// What iOS says the app may still allocate, in bytes.
#[cfg(all(target_os = "ios", not(target_env = "sim")))]
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
    fn a_report_that_cannot_be_read_or_holds_no_figure_answers_why() {
        assert_eq!(meminfo_figure(Ok(REPORT.to_string())), Ok(1_887_920 * 1024));
        let refused = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let words = format!("/proc/meminfo could not be read: {refused}");
        assert_eq!(meminfo_figure(Err(refused)), Err(words));
        assert_eq!(meminfo_figure(Ok("MemTotal: 10 kB\n".to_string())), Err("/proc/meminfo holds no MemAvailable figure in kB".to_string()));
    }

    #[test]
    fn the_hosts_figure_is_its_share_of_the_whole_memory() {
        assert_eq!(host_figure(79, 64 << 30), Ok(79 * (64 << 30) / 100));
        assert_eq!(host_figure(0, 64 << 30), Ok(0), "a host with nothing to spare has no memory to give");
        assert_eq!(host_figure(100, u64::MAX), Ok(u64::MAX), "the whole of the largest memory");
        assert_eq!(host_figure(101, 64 << 30), Err("kern.memorystatus_level reads 101 percent, more than the whole memory".to_string()));
    }

    #[test]
    fn a_sysctl_answers_an_int_or_a_quad_and_nothing_else() {
        let int = [79, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(sysctl_value(int, 4), Some(u64::from(u32::from_ne_bytes([79, 0, 0, 0]))), "only the int's own bytes count");
        let quad = (64_u64 << 30).to_ne_bytes();
        assert_eq!(sysctl_value(quad, 8), Some(64 << 30));
        for size in [0, 1, 2, 3, 5, 7, 9] {
            assert_eq!(sysctl_value(quad, size), None, "{size} bytes");
        }
    }

    /// A Mac's figure is the kernel's own, as its `sysctl` command reads the same two values, not a
    /// number of the core's: the level moves while the test runs, so the figure is held to the levels
    /// read just before and just after it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_reports_the_kernels_share_of_its_memory() {
        let read = || {
            let output = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "kern.memorystatus_level", "hw.memsize"]).output().expect("sysctl runs");
            let text = String::from_utf8(output.stdout).expect("sysctl writes text");
            let numbers: Vec<u64> = text.split_whitespace().map(|word| word.parse().expect("a number")).collect();
            assert_eq!(numbers.len(), 2, "{text:?}");
            (numbers[0], numbers[1])
        };
        let (before, total) = read();
        let figure = available_memory().expect("a Mac reports its figure");
        let (after, _) = read();
        eprintln!("available_memory() = {figure} of {total} bytes, at levels {before} and {after}");
        let shares = [before.min(after), before.max(after)].map(|level| u128::from(total) * u128::from(level) / 100);
        assert!((shares[0]..=shares[1]).contains(&u128::from(figure)), "{figure} bytes, against {shares:?}");
    }

    /// A Linux machine's figure is its kernel's `MemAvailable`, as the report reads it just before and
    /// just after.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reports_its_kernels_mem_available() {
        let read = || parse_meminfo(&std::fs::read_to_string(MEMINFO).expect("the report reads")).expect("a MemAvailable figure");
        let before = read();
        let figure = available_memory().expect("Linux reports its figure");
        let after = read();
        eprintln!("available_memory() = {figure}, MemAvailable {before} then {after}");
        assert!((before.min(after)..=before.max(after)).contains(&figure), "{figure} bytes, against {before} and {after}");
    }

    /// On a phone, the figure the platform gives is a real, plausible one: more than a page and below
    /// the address space a phone has. The numbers go to the run's output for the person reading it.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    #[test]
    fn the_phone_reports_a_figure() {
        let figure = available_memory();
        #[cfg(all(target_os = "ios", not(target_env = "sim")))]
        eprintln!("os_proc_available_memory() = {}", app_memory_left());
        eprintln!("available_memory() = {figure:?}");
        let bytes = figure.expect("a phone reports its figure");
        assert!(bytes > 4096 && bytes < (1 << 40), "{bytes}");
    }
}
