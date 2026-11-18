//! The hold that makes a stop in the corpus land before the export it stops can finish.
//!
//! A scenario that cancels an export says at which fraction of its frames the listener stops it, and the
//! listener hears the export's progress only at the runner's beats, a tenth of a second apart. An export
//! that runs on regardless can pass its last frame between two beats, however many frames the scenario
//! gives it, since how many beats it takes depends on how busy the machine is: the answer is then not
//! `Cancelled`, or the stop is met after the file is written. So the export does not run on regardless:
//! once it reports the frames at which the listener stops it, it waits there, asking again, until the
//! stop reaches it. The exporter then sees the stop with frames left to render and removes the file it
//! opened, as it does on a device, and the scenario's result no longer depends on the export's speed.
//!
//! The stop the scenario checks must be the plug-in's own, so a call is answered only once the plug-in's
//! stop has released its export from the hold: a call that answered before any export of it was held, or
//! whose export waited out the hold's limit, fails its step instead, since what it answered was never the
//! plug-in's stop.

use crate::corpus::Progress;
use effectcraft_engine::project::render_queue::{OutputFormat, RenderSettings};
use effectcraft_engine::render_queue::{ExportJob, ExportResult, Exporter};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The longest a held export waits for its stop. A stop that never reaches it is the plug-in's failure to
/// stop the export (or a listener that never stops it), so the hold then ends the export itself and the
/// step fails, naming where it was held, instead of hanging until the step's own timeout.
const LIMIT: Duration = Duration::from_secs(60);

/// How long a held export waits before it asks again whether it has been stopped.
const RECHECK: Duration = Duration::from_millis(5);

/// What became of the stop of the call being answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Met {
    /// No export of the call has been held where its listener stops it.
    #[default]
    Never,
    /// An export was held there until the plug-in's stop reached it.
    Stopped,
    /// An export was held at `done` of its `total` frames for `waited`, and the stop never came.
    Missed { done: u64, total: u64, waited: Duration },
}

/// The call being answered, as its exports read it: where its listener stops it, and what became of that
/// stop.
#[derive(Default)]
struct Watch {
    progress: Option<Progress>,
    met: Met,
}

/// The call being answered, shared by the host, which sets it around each call and judges it once the call
/// has answered, and the exporter, which reads it as a job starts and says how the job's stop came.
#[derive(Clone, Default)]
pub(crate) struct Hold(Arc<Mutex<Watch>>);

impl Hold {
    /// `call`'s answer, with each export it makes held at the frames `progress` stops it at. A call that
    /// stops at a fraction is answered only when the plug-in's stop released an export of it from the hold;
    /// otherwise the step fails, saying why, since the answer would not show the plug-in's stop.
    pub(crate) fn around<T>(&self, progress: &Progress, call: impl FnOnce() -> T) -> Result<T, String> {
        self.replace(Watch { progress: Some(progress.clone()), met: Met::Never });
        let answer = call();
        let met = self.replace(Watch::default()).met;
        match (progress.cancel_at, met) {
            (None, _) | (Some(_), Met::Stopped) => Ok(answer),
            (Some(at), Met::Never) => Err(format!("the call answered without its export being held at {at} of its frames until the plug-in's stop reached it")),
            (Some(_), Met::Missed { done, total, waited }) => {
                Err(format!("the export was held at {done} of its {total} frames for {waited:?}, and the plug-in's stop never reached it"))
            }
        }
    }

    fn replace(&self, watch: Watch) -> Watch {
        std::mem::replace(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner), watch)
    }

    fn progress(&self) -> Option<Progress> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).progress.clone()
    }

    fn met(&self, met: Met) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).met = met;
    }
}

/// An exporter that runs `wrapped`, and holds each job at the frames its call's listener stops it at.
pub(crate) struct Held {
    wrapped: Arc<dyn Exporter>,
    hold: Hold,
}

impl Held {
    /// `wrapped`, held as `hold` says, for as long as [`LIMIT`].
    pub(crate) fn new(wrapped: Arc<dyn Exporter>, hold: Hold) -> Self {
        Self { wrapped, hold }
    }
}

impl Exporter for Held {
    fn formats(&self) -> Vec<OutputFormat> {
        self.wrapped.formats()
    }

    fn supports_skip_existing(&self, settings: &RenderSettings) -> bool {
        self.wrapped.supports_skip_existing(settings)
    }

    fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        self.wrapped.export(job, &mut holding(&self.hold, LIMIT, progress))
    }
}

/// `progress`, as an export held where `hold`'s call stops it reports to it: the frames before are answered
/// as `progress` answers them, and from the first frames the call stops at it asks again, which is how the
/// session is asked whether the stop has reached it, until it says `false`, or until `limit` has passed,
/// and answers `false` too, telling `hold` which it was.
fn holding<'a>(hold: &'a Hold, limit: Duration, progress: &'a mut dyn FnMut(u64, u64) -> bool) -> impl FnMut(u64, u64) -> bool + 'a {
    let stops = hold.progress();
    move |done, total| {
        if !stops.as_ref().is_some_and(|stops| stops.cancels(done, total)) {
            return progress(done, total);
        }
        let waiting = Instant::now();
        loop {
            if !progress(done, total) {
                hold.met(Met::Stopped);
                return false;
            }
            if waiting.elapsed() >= limit {
                hold.met(Met::Missed { done, total, waited: limit });
                return false;
            }
            std::thread::sleep(RECHECK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// An export of `frames` frames that reports after each and ends only if no report stopped it, as the
    /// real one does, made by a call that stops at `at`, with the session answering `true` until the
    /// plug-in's stop reaches it, `stop_after` after the listener asks to stop: the code under test is held
    /// up, never the machine. What it came to: whether it ran to its end, the last frame it reported, and
    /// the call's judgement.
    fn run(frames: u64, at: Option<f64>, stop_after: Duration, limit: Duration) -> (bool, u64, Result<(), String>) {
        let (hold, last) = (Hold::default(), AtomicU64::new(0));
        let (asked, stops) = (Mutex::new(None::<Instant>), at.map(|fraction| fraction * frames as f64));
        let mut session = |done: u64, _: u64| {
            let mut asked = asked.lock().unwrap();
            if stops.is_some_and(|at| done as f64 >= at) {
                return asked.get_or_insert_with(Instant::now).elapsed() < stop_after;
            }
            true
        };
        let mut finished = false;
        let judged = hold.around(&Progress { cancel_at: at }, || {
            let mut hooked = holding(&hold, limit, &mut session);
            finished = (1..=frames).all(|done| {
                last.store(done, Ordering::SeqCst);
                hooked(done, frames)
            });
        });
        (finished, last.load(Ordering::SeqCst), judged)
    }

    #[test]
    fn an_export_the_stop_is_slow_to_reach_waits_for_it_instead_of_finishing() {
        let waiting = Instant::now();
        let (finished, last, judged) = run(60, Some(0.5), Duration::from_millis(300), LIMIT);
        assert!(waiting.elapsed() >= Duration::from_millis(300), "the export gave up its wait for the stop after {:?}", waiting.elapsed());
        assert!(!finished, "the export ran to its end before the stop reached it");
        assert_eq!(last, 30, "it waits at the first frame the listener stops it at, and goes no further");
        assert_eq!(judged, Ok(()), "the plug-in's stop released it");
    }

    #[test]
    fn a_stop_that_comes_at_once_is_met_at_once() {
        assert_eq!(run(60, Some(0.5), Duration::ZERO, LIMIT), (false, 30, Ok(())));
    }

    #[test]
    fn the_hold_is_at_the_fraction_the_call_names_and_nowhere_else() {
        for (at, frame) in [(0.25, 15), (1.0, 60)] {
            assert_eq!(run(60, Some(at), Duration::from_millis(100), LIMIT), (false, frame, Ok(())), "at {at}");
        }
    }

    #[test]
    fn a_call_with_no_stop_runs_to_its_end_unheld() {
        assert_eq!(run(60, None, Duration::from_secs(3600), LIMIT), (true, 60, Ok(())));
    }

    /// A stop that never comes ends the wait at the limit, and the step fails, naming where the export was
    /// held: the stop the hold makes is never taken for the plug-in's.
    #[test]
    fn a_stop_that_never_comes_is_made_by_the_limit_and_fails_the_step() {
        let waited = Instant::now();
        let limit = Duration::from_millis(200);
        let (finished, last, judged) = run(60, Some(0.5), Duration::from_secs(3600), limit);
        assert!(waited.elapsed() >= limit && waited.elapsed() < Duration::from_secs(30), "waited {:?}", waited.elapsed());
        assert_eq!((finished, last), (false, 30));
        assert_eq!(judged, Err("the export was held at 30 of its 60 frames for 200ms, and the plug-in's stop never reached it".to_string()));
    }

    /// A call that answers before an export of it reaches the frames it stops at was never stopped there by
    /// the plug-in, whatever it answered: a fraction no frame reaches, or an export stopped before it.
    #[test]
    fn a_call_whose_export_is_never_held_where_it_stops_fails_the_step() {
        let never = "the call answered without its export being held at 0.5 of its frames until the plug-in's stop reached it";
        assert_eq!(run(60, Some(f64::NAN), Duration::ZERO, LIMIT).2, Err(never.replace("0.5", "NaN")));
        let hold = Hold::default();
        let judged = hold.around(&Progress { cancel_at: Some(0.5) }, || holding(&hold, LIMIT, &mut |_, _| false)(10, 60));
        assert_eq!(judged, Err(never.to_string()), "a stop before the frames the call names is no stop the hold released");
    }

    #[test]
    fn what_a_job_reads_is_the_call_being_answered_and_after_it_nothing() {
        let hold = Hold::default();
        assert!(hold.progress().is_none());
        let mut read = None;
        let _ = hold.around(&Progress { cancel_at: Some(0.5) }, || read = hold.progress().and_then(|progress| progress.cancel_at));
        assert_eq!(read, Some(0.5));
        assert_eq!(hold.around(&Progress { cancel_at: None }, || hold.progress().map(|progress| progress.cancel_at)), Ok(Some(None)));
        assert!(hold.progress().is_none(), "nothing is held once the call has answered");
    }
}
