//! Making a call that blocks into one a foreign caller can await without blocking its own thread: what
//! the engine cannot answer on a oneshot, such as stopping, which joins its thread.

use crate::error::CraftError;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;

/// Run `work`, which blocks until the engine answers, on a thread of its own and wait for its answer
/// without blocking the caller's: Swift's and Kotlin's futures are polled on the app's own threads,
/// the main thread among them.
///
/// A panic in `work` is the call's `Internal` error, so none crosses the boundary. A caller that
/// stops waiting leaves the call to finish: the engine answers every call it accepted.
pub(crate) async fn off_thread<T, F>(work: F) -> Result<T, CraftError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CraftError> + Send + 'static,
{
    let (answer, answered) = oneshot::channel();
    thread::Builder::new()
        .name("craftbridge-call".to_string())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|caught| Err(CraftError::from_panic(caught)));
            // The receiver is gone when the caller stopped waiting, and nobody wants the answer then.
            let _ = answer.send(outcome);
        })
        .map_err(|error| CraftError::internal(format!("cannot start a thread for the call: {error}")))?;
    answered.await.unwrap_or_else(|_| Err(CraftError::internal("the call's thread stopped without answering")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_runs_on_another_thread_and_its_answer_comes_back() {
        let caller = thread::current().id();
        let answer = pollster::block_on(off_thread(move || Ok(thread::current().id() != caller)));
        assert_eq!(answer, Ok(true));
    }

    #[test]
    fn the_works_error_comes_back_as_it_is() {
        let answer = pollster::block_on(off_thread(|| Err::<(), _>(CraftError::Cancelled)));
        assert_eq!(answer, Err(CraftError::Cancelled));
    }

    #[test]
    fn a_panic_in_the_work_is_internal() {
        let answer = pollster::block_on(off_thread(|| -> Result<(), CraftError> { panic!("the glue blew up") }));
        assert_eq!(answer, Err(CraftError::internal("the glue blew up")));
    }
}
