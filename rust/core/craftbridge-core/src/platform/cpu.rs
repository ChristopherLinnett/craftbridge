//! Whether this phone's processor can run the code the Android arm64 library is built from.
//!
//! The Android arm64 library can only be built for ARMv8.2-A half-precision arithmetic in every crate
//! (`+fp16`): `gemm`'s own assembly needs it, and the rules allow no way to give it to that one crate.
//! A processor without it (Cortex-A53, A57, A72 or A73, as in a Snapdragon 625 or 665) stops the app
//! at the first such instruction with a signal no error handling can catch, so the engine asks the
//! kernel before any app code runs and refuses the start in words an app can show.
//!
//! The kernel's own capability bits are the only honest answer. `is_aarch64_feature_detected!` is not:
//! with `+fp16` compiled in, the standard library resolves it to a constant `true` at build time and
//! never reads the processor.
//!
//! The decision is plain safe code over the bits, which the host tests hand in. Only the one call that
//! reads them from the kernel is `unsafe`, and it answers a number.

use crate::Result;
use crate::error::CraftError;

/// `HWCAP_FPHP`: the processor does half-precision floating point (Linux's `asm/hwcap.h` for arm64).
pub(crate) const HWCAP_FPHP: u64 = 1 << 9;

/// `HWCAP_ASIMDHP`: the processor does half-precision vector arithmetic (Linux's `asm/hwcap.h` for arm64).
pub(crate) const HWCAP_ASIMDHP: u64 = 1 << 10;

// The bits above are the platform's own, so a device build proves them against the C library's.
#[cfg(all(target_os = "android", target_arch = "aarch64"))]
const _: () = {
    assert!(HWCAP_FPHP == libc::HWCAP_FPHP);
    assert!(HWCAP_ASIMDHP == libc::HWCAP_ASIMDHP);
};

/// Each capability the floor needs, with the words that name it to the person who owns the phone.
const REQUIRED: [(u64, &str); 2] =
    [(HWCAP_FPHP, "half-precision floating point (FEAT_FP16, HWCAP_FPHP)"), (HWCAP_ASIMDHP, "half-precision vector arithmetic (FEAT_FP16, HWCAP_ASIMDHP)")];

/// Refuse a processor that lacks what the library is built for. `hwcap` is the kernel's capability
/// bits, or `None` for a build that takes no half-precision path (every target but Android arm64),
/// which has no floor to meet.
///
/// A word of zero is what `getauxval` answers when the kernel gives no such entry; it names every
/// feature as missing, since a build that needs them cannot be told it has them.
///
/// # Errors
///
/// [`CraftError::Unsupported`], whose message names each missing feature.
pub(crate) fn require_fp16(hwcap: Option<u64>) -> Result<()> {
    let Some(bits) = hwcap else {
        return Ok(());
    };
    let missing: Vec<&str> = REQUIRED.iter().filter(|(bit, _)| bits & bit == 0).map(|(_, name)| *name).collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(CraftError::unsupported(format!(
        "this phone's processor lacks {}; the engine needs an arm64 processor with FP16 (ARMv8.2-A half-precision arithmetic, Cortex-A55 or A75 and newer) and cannot run on this phone",
        missing.join(" and ")
    )))
}

/// The capability bits this build must meet: the kernel's, on Android arm64, and none anywhere else,
/// where no half-precision instruction is compiled in (the x86_64 emulator, iOS and the desktop).
///
/// A host test hands in other bits with [`handed_in`], which every start reads here, so the one check
/// every start goes through is the one the tests drive.
pub(crate) fn kernel_hwcap() -> Option<u64> {
    #[cfg(test)]
    if let Some(bits) = tests::handed_in_here() {
        return bits;
    }
    platform_hwcap()
}

#[cfg(all(target_os = "android", target_arch = "aarch64"))]
fn platform_hwcap() -> Option<u64> {
    // SAFETY: `getauxval` takes a number, reads no memory of ours and has no precondition; it only
    // returns a number, 0 for an entry the kernel does not give.
    Some(unsafe { libc::getauxval(libc::AT_HWCAP) })
}

#[cfg(not(all(target_os = "android", target_arch = "aarch64")))]
fn platform_hwcap() -> Option<u64> {
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Mutex, PoisonError};
    use std::thread::{self, ThreadId};

    /// The capability bits each test hands the starts made on its own thread (the check runs on the
    /// caller's thread, so no other test sees them). The tests run in parallel, so the bits are kept
    /// by thread and never shared.
    static HANDED_IN: Mutex<Vec<(ThreadId, Option<u64>)>> = Mutex::new(Vec::new());

    /// What this thread's test handed in, if it did.
    pub(super) fn handed_in_here() -> Option<Option<u64>> {
        HANDED_IN.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(id, _)| *id == thread::current().id()).map(|(_, bits)| *bits)
    }

    /// Run `body` with every start on this thread reading `hwcap` as the kernel's capability bits, the
    /// way an Android arm64 phone's processor answers them (`None` for a build with no floor). The
    /// platform's own answer comes back when `body` ends, by a panic too.
    pub(crate) fn handed_in<R>(hwcap: Option<u64>, body: impl FnOnce() -> R) -> R {
        struct Restore(ThreadId);
        impl Drop for Restore {
            fn drop(&mut self) {
                HANDED_IN.lock().unwrap_or_else(PoisonError::into_inner).retain(|(id, _)| *id != self.0);
            }
        }
        let id = thread::current().id();
        HANDED_IN.lock().unwrap_or_else(PoisonError::into_inner).push((id, hwcap));
        let _restore = Restore(id);
        body()
    }

    const FPHP_NAME: &str = "half-precision floating point (FEAT_FP16, HWCAP_FPHP)";
    const ASIMDHP_NAME: &str = "half-precision vector arithmetic (FEAT_FP16, HWCAP_ASIMDHP)";

    fn refusal(bits: u64) -> String {
        match require_fp16(Some(bits)) {
            Err(CraftError::Unsupported { message }) => message,
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn the_bits_are_the_kernels_for_arm64() {
        assert_eq!(HWCAP_FPHP, 0x200);
        assert_eq!(HWCAP_ASIMDHP, 0x400);
    }

    #[test]
    fn a_processor_with_both_bits_passes() {
        assert_eq!(require_fp16(Some(HWCAP_FPHP | HWCAP_ASIMDHP)), Ok(()));
        // Other capabilities beside them change nothing.
        assert_eq!(require_fp16(Some(u64::MAX)), Ok(()));
    }

    #[test]
    fn a_build_with_no_half_precision_path_has_no_floor() {
        assert_eq!(require_fp16(None), Ok(()));
    }

    #[test]
    fn a_processor_without_half_precision_scalar_is_refused_by_name() {
        assert_eq!(
            refusal(HWCAP_ASIMDHP | 0b11),
            format!(
                "this phone's processor lacks {FPHP_NAME}; the engine needs an arm64 processor with FP16 (ARMv8.2-A half-precision arithmetic, Cortex-A55 or A75 and newer) and cannot run on this phone"
            )
        );
    }

    #[test]
    fn a_processor_without_half_precision_vectors_is_refused_by_name() {
        assert_eq!(
            refusal(HWCAP_FPHP | 0b11),
            format!(
                "this phone's processor lacks {ASIMDHP_NAME}; the engine needs an arm64 processor with FP16 (ARMv8.2-A half-precision arithmetic, Cortex-A55 or A75 and newer) and cannot run on this phone"
            )
        );
    }

    #[test]
    fn a_processor_without_either_is_refused_naming_both() {
        for bits in [0, 0b11, !(HWCAP_FPHP | HWCAP_ASIMDHP)] {
            let message = refusal(bits);
            assert!(message.starts_with(&format!("this phone's processor lacks {FPHP_NAME} and {ASIMDHP_NAME}; ")), "{message}");
        }
    }

    /// Everywhere but Android arm64 there is no floor to read: the x86_64 emulator, iOS and the
    /// desktop compile in no half-precision instruction.
    #[cfg(not(all(target_os = "android", target_arch = "aarch64")))]
    #[test]
    fn a_build_that_is_not_android_arm64_reads_no_floor() {
        assert_eq!(kernel_hwcap(), None);
    }

    /// On Android arm64 the kernel's word is read, and a phone that has the feature shows both bits.
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    #[test]
    fn android_arm64_reads_the_kernels_word() {
        assert!(kernel_hwcap().is_some_and(|bits| bits != 0));
    }

    /// What a device run proves: the processor these tests run on meets the floor the build needs, read
    /// from the kernel as the engine's own start reads it. On the host there is no floor, so it passes
    /// there as the start does.
    #[test]
    fn the_processor_the_tests_run_on_meets_the_floor() {
        assert_eq!(require_fp16(kernel_hwcap()), Ok(()));
    }

    #[test]
    fn handed_in_bits_are_what_the_start_reads_and_end_with_the_body() {
        let platforms = platform_hwcap();
        assert_eq!(handed_in(Some(HWCAP_FPHP), kernel_hwcap), Some(HWCAP_FPHP));
        assert_eq!(kernel_hwcap(), platforms);
        let panicked = std::panic::catch_unwind(|| handed_in(Some(0), || panic!("a test failed")));
        assert!(panicked.is_err());
        assert_eq!(kernel_hwcap(), platforms);
    }
}
