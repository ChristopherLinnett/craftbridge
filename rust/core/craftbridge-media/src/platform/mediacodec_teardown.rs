//! How the media crate's test binary ends its process on Android.
//!
//! Android's codec stack keeps threads of its own for the life of the process: the platform's buffer pool
//! library starts a worker, its invalidator, for the first codec any process makes, which waits on a mutex of
//! that library's static data, and nothing in the NDK ends it. `exit` runs the library's static destructors while
//! the worker still runs, and when it next locks that mutex bionic says `FORTIFY: pthread_mutex_lock called on a
//! destroyed mutex` and aborts the process as it ends, after its status is set, so a run whose tests all passed
//! still ends with 0 and only its output tells. The worker is the platform's, so no call of this crate's can end
//! it first; what a test binary can do is leave without running the destructors.
//!
//! So the test binary defines `exit` itself. The dynamic linker looks a symbol up in the executable before any
//! library, so this definition takes the C library's place for every caller: the harness's
//! `std::process::exit` when a test failed, and the C library's own call when `main` returns. It ends the process
//! with the status it is given, which is the harness's verdict however a test failed (a panic, an `Err` returned,
//! a `should_panic` test that returned), and runs no destructor and no `atexit` handler. There is nothing left
//! to flush: Rust's runtime flushes its own streams before each of those calls, and the tests write through
//! nothing else.

use std::ffi::c_int;

unsafe extern "C" {
    /// Ends the process now with `status`: no destructor and no handler runs.
    fn _exit(status: c_int) -> !;
}

/// The process's `exit`, in place of the C library's: ends the process with `status` and runs nothing first.
///
/// # Safety
///
/// It is called as the C library's `exit` is, whose signature it keeps: the compiler holds a definition of a
/// symbol the standard library calls to that symbol's signature.
#[unsafe(no_mangle)]
unsafe extern "C" fn exit(status: c_int) -> ! {
    // SAFETY: `_exit` takes any status and never returns, and nothing of this process runs after it.
    unsafe { _exit(status) }
}
