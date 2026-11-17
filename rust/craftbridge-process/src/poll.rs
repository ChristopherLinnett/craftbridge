use std::time::{Duration, Instant};

/// How often a wait looks again.
pub(crate) const POLL: Duration = Duration::from_millis(20);

/// Poll `done` until it holds, and say whether it did before `stuck_after` passed.
pub fn poll_until(stuck_after: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + stuck_after;
    while !done() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}
