//! Export waiting (plan §13, L3; plan §16's `export.rs`): while `request_with_progress` runs, the
//! engine answers progress, cancel and `info` only, and every other call waits in order until the
//! export ends.
//!
//! The shape of this is set by what the engine can do while it is inside the app's export call.
//! The session is reached only on the engine thread (L1), and while `CraftApp::request_with_progress`
//! is on that thread's stack the engine cannot ask the app anything else: a second call into the
//! same session would re-enter a live `&mut` borrow, which is why the engine refuses a call from
//! its own thread. The one code of the engine's that runs during an export is the progress closure
//! handed to that call, run each time the app reports. So that closure is the engine's beat, and
//! everything L3 answers, it answers there:
//!
//! * **progress**: every report the app makes is relayed to the caller's listener as it arrives
//!   (a listener that fails is ignored and the export continues, plan §16's edge case);
//! * **cancel**: a [`CancelToken`] is set without the engine, and the closure answers `false` from
//!   the first beat after it, or after a close began, so an app that reports every
//!   [`PROGRESS_INTERVAL`] and stops at the first `false` stops within one interval (plan §16);
//! * **`info`**: the one request answered during an export. At each beat the engine drains its
//!   inbox, answers queued `info` calls at once, and keeps every other call waiting in order. The
//!   answer is the one the app gave when the export began, asked of it then because it cannot be
//!   asked during the export: the newest state the engine can know. Every call that could change
//!   that state waits behind the export; what the export itself changes, such as the render item it
//!   queues, shows in the first `info` after it ends. `info` takes no parameters (plan §14), so each
//!   call is answered with that one answer whatever parameters it carries.

use crate::Result;
use crate::app::CraftApp;
use crate::cancel::CancelToken;
use crate::engine::{Message, guarded};
use crate::error::CraftError;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// The interval a progress loop is measured on: 100 ms, the interval EffectCraft's own command
/// line waits between progress polls (`apps/effectcraft-cli/src/main.rs:983-1013`), so an export
/// made through CraftBridge reports progress and takes a cancel on the beat the desktop one does
/// (plan §16, §19). An implementation of [`CraftApp::request_with_progress`] reports on this beat
/// for as long as work remains; that is what bounds a cancel to one interval.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// The one call `request_with_progress` does not make the caller wait for, named by plan §13's L3.
pub(crate) const INFO_METHOD: &str = "info";

/// One export handed to the engine thread: the request to run, the token it is cancelled by, where
/// to report progress and where to answer.
pub(crate) struct ExportCall {
    method: String,
    params: Value,
    cancel: CancelToken,
    progress: Box<dyn FnMut(u64, u64, Option<f64>) + Send>,
    answer: Sender<Result<Value>>,
}

impl ExportCall {
    /// The export of `method`, built once the engine gives it the channel its answer goes back on.
    pub(crate) fn message<F>(method: &str, params: Value, on_progress: F, cancel: &CancelToken) -> impl FnOnce(Sender<Result<Value>>) -> Message
    where
        F: FnMut(u64, u64, Option<f64>) + Send + 'static,
    {
        let (method, cancel) = (method.to_string(), cancel.clone());
        move |answer| Message::Export { call: ExportCall { method, params, cancel, progress: Box::new(on_progress), answer } }
    }
}

/// The actor's side of itself that running an export touches: its inbox, the calls it has taken out
/// of the inbox to keep waiting in order, and its flag that a close has begun.
pub(crate) struct EngineSide<'a> {
    pub(crate) inbox: &'a Receiver<Message>,
    pub(crate) pending: &'a mut VecDeque<Message>,
    pub(crate) closing: &'a CancelToken,
}

/// Run one export on the engine thread (plan §16 behaviour 3, plan §19's sequence): take the
/// `info` answer the calls during it will be given, run the app's own loop behind the never-crash
/// guard with the engine's beat as its progress closure, and answer the caller.
pub(crate) fn run_export(app: &mut dyn CraftApp, call: ExportCall, side: EngineSide<'_>) {
    let ExportCall { method, params, cancel, mut progress, answer } = call;
    // A stop is asked by the caller's token, or by a close, which stops every export (plan §16
    // behaviour 4).
    let stop_asked = || cancel.is_cancelled() || side.closing.is_cancelled();
    // An export asked to stop before its turn came is answered without running anything.
    if stop_asked() {
        let _ = answer.send(Err(CraftError::Cancelled));
        return;
    }
    let info = guarded(|| app.request(INFO_METHOD, Value::Null)).flatten();
    let mut stopped = false;
    let ran = guarded(|| {
        app.request_with_progress(&method, params, &mut |done, total, seconds_left| {
            answer_queued(side.inbox, side.pending, &info);
            // The caller's listener is app-side code too: it is run behind the same guard as a
            // call, and its failure is dropped, so the export continues (plan §16's edge case).
            let _ = guarded(|| progress(done, total, seconds_left));
            stopped = stopped || stop_asked();
            !stopped
        })
    });
    let outcome = match ran {
        // A panic is answered as one in any call is: `Internal`, with the session kept.
        Err(panicked) => Err(panicked),
        // Once the app was told to stop, the caller hears `Cancelled` whatever the app answered:
        // the caller asked to stop and the work stopped (`CraftApp::request_with_progress`).
        Ok(_) if stopped => Err(CraftError::Cancelled),
        Ok(answered) => answered,
    };
    let _ = answer.send(outcome);
}

/// The engine's beat during an export: every call that arrived while the app was running it comes
/// out of the inbox here. `info` is answered at once from the export's own answer; everything
/// else goes to `pending`, in order, to run right after the export, ahead of anything sent after
/// this beat, because the inbox only ever holds calls made later than what an earlier beat drained.
fn answer_queued(inbox: &Receiver<Message>, pending: &mut VecDeque<Message>, info: &Result<Value>) {
    while let Ok(message) = inbox.try_recv() {
        match message {
            Message::Request { method: ref name, answer, .. } if name.as_str() == INFO_METHOD => {
                let _ = answer.send(info.clone());
            }
            other => pending.push_back(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, THREAD_NAME, lock};
    use crate::testing::{DEADLINE, Gate, close_begun, eventually, queue_call, recv};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// The polls a fake export runs when nothing stops it.
    const POLLS: u64 = 4;

    /// One progress report as the caller's listener hears it.
    type Report = (u64, u64, Option<f64>);

    /// What the test and its fake app share: the gate that releases the export's next poll one at a
    /// time, the counters and marks the test reads, and the log of what the app ran, in order.
    struct Shared {
        /// The test opens the gate one turn at a time; the export waits here for each next poll.
        release: AtomicUsize,
        /// What `info` answers. A test changes it mid-export to pin the answer an export gives
        /// `info` calls while it runs: the state its export began with, not the state since.
        state: AtomicI64,
        /// `-1` until the progress closure answers `false`; then the poll that saw it.
        stop_poll: AtomicI64,
        /// Set if the progress closure answered `true` again after it had answered `false`.
        resumed: AtomicBool,
        /// When set, `request_with_progress` panics as it is entered, so a test can pin how a
        /// panicking export answers.
        panics_in_export: AtomicBool,
        /// When set, an export asked to stop panics on its way out, so a test can pin that a
        /// caught panic outranks the `Cancelled` the caller asked for.
        panics_when_stopped: AtomicBool,
        /// Whether the app stops at the first `false` (true) or keeps running to its end (false).
        stops_when_stopped: AtomicBool,
        polls: AtomicUsize,
        /// The progress reports the engine has answered, `true` or `false`. The engine reads the
        /// cancel and the close at the end of a beat, after relaying its report, so a test that
        /// cancels once it heard a report could still land in that same beat; once a beat is
        /// counted here the app is waiting for its next release and the next beat is the first
        /// to read anything set now.
        answered: AtomicUsize,
        /// While set, an export that ran to its end opens `at_end` and waits for `end_released`
        /// before it answers, so a test can hand the engine calls when no beat is left to take
        /// them out of the inbox.
        holds_its_end: AtomicBool,
        at_end: Gate,
        end_released: Gate,
        /// While set, the session's drop waits for `drop_released`, so a test can look at the
        /// engine between the end of its loop and the end of its session.
        hold_drop: AtomicBool,
        drop_released: Gate,
        dropped: AtomicBool,
        log: Mutex<Vec<String>>,
    }

    impl Shared {
        /// The export's wait for its next poll: true once released, false if the test never does
        /// and the deadline passes, an error the export answers rather than a hang. Each release
        /// buys exactly one poll: the wait takes one token, so releases a test opens ahead of time
        /// release later polls in turn, never several polls at once.
        fn wait_release(&self, seen: &mut usize) -> bool {
            let released = eventually(|| self.release.load(Ordering::SeqCst) > *seen);
            if released {
                *seen += 1;
            }
            released
        }

        /// Open the gate for `polls` more polls.
        fn open(&self, polls: u64) {
            self.release.fetch_add(usize::try_from(polls).unwrap(), Ordering::SeqCst);
        }

        /// Whether `flag` came to hold `want` before the deadline.
        fn wait_for(&self, flag: &AtomicI64, want: i64) -> bool {
            eventually(|| self.read(flag) == want)
        }

        fn read(&self, flag: &AtomicI64) -> i64 {
            flag.load(Ordering::SeqCst)
        }

        fn log(&self, line: &str) {
            lock(&self.log).push(line.to_string());
        }

        /// The lines of the log that start with `prefix`, in the order the app ran them.
        fn logged(&self, prefix: &str) -> Vec<String> {
            lock(&self.log).iter().filter(|line| line.starts_with(prefix)).cloned().collect()
        }
    }

    /// The fake app an export runs on: `info` answers its shared state, and `request_with_progress`
    /// is a §19 render runner in miniature. It reports a first progress, then waits for each
    /// released poll and reports it, and at the first progress the engine answers `false` it stops
    /// and answers its stop as a failure of its own, as a runner reporting a stopped job does.
    struct ExportApp {
        shared: Arc<Shared>,
        seen: usize,
    }

    fn export_app() -> (ExportApp, Arc<Shared>) {
        let shared = Arc::new(Shared {
            release: AtomicUsize::new(0),
            state: AtomicI64::new(0),
            stop_poll: AtomicI64::new(-1),
            resumed: AtomicBool::new(false),
            panics_in_export: AtomicBool::new(false),
            panics_when_stopped: AtomicBool::new(false),
            stops_when_stopped: AtomicBool::new(true),
            polls: AtomicUsize::new(0),
            answered: AtomicUsize::new(0),
            holds_its_end: AtomicBool::new(false),
            at_end: Gate::default(),
            end_released: Gate::default(),
            hold_drop: AtomicBool::new(false),
            drop_released: Gate::default(),
            dropped: AtomicBool::new(false),
            log: Mutex::new(Vec::new()),
        });
        (ExportApp { shared: shared.clone(), seen: 0 }, shared)
    }

    impl CraftApp for ExportApp {
        fn request(&mut self, method: &str, params: Value) -> Result<Value> {
            self.shared.log(&format!("{method} {params}"));
            match method {
                "info" => Ok(json!({ "state": self.shared.state.load(Ordering::SeqCst) })),
                "echo" => Ok(params),
                other => Err(CraftError::internal(format!("unknown method {other}"))),
            }
        }

        fn request_with_progress(&mut self, method: &str, params: Value, on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool) -> Result<Value> {
            self.shared.log(&format!("export {method} {params}"));
            if self.shared.panics_in_export.load(Ordering::SeqCst) {
                panic!("boom in export");
            }
            for poll in 0..=POLLS {
                if poll != 0 && !self.shared.wait_release(&mut self.seen) {
                    return Err(CraftError::internal("the test never released the export's next poll"));
                }
                self.shared.polls.fetch_add(1, Ordering::SeqCst);
                let go_on = on_progress(poll, POLLS, Some(poll as f64));
                self.shared.answered.fetch_add(1, Ordering::SeqCst);
                if go_on {
                    if self.shared.read(&self.shared.stop_poll) >= 0 {
                        self.shared.resumed.store(true, Ordering::SeqCst);
                    }
                    continue;
                }
                if self.shared.panics_when_stopped.load(Ordering::SeqCst) {
                    panic!("boom after stop");
                }
                // The engine asked this export to stop: the record is for the tests' "within one
                // interval" checks; whether the app honours it now is the fake's switch.
                if self.shared.read(&self.shared.stop_poll) < 0 {
                    self.shared.stop_poll.store(i64::try_from(poll).unwrap_or(i64::MAX), Ordering::SeqCst);
                }
                if self.shared.stops_when_stopped.load(Ordering::SeqCst) {
                    return Err(CraftError::command_failed(method, "stopped by its caller", None));
                }
            }
            if self.shared.holds_its_end.load(Ordering::SeqCst) {
                self.shared.at_end.open();
                if !self.shared.end_released.wait() {
                    return Err(CraftError::internal("the test never let the export end"));
                }
            }
            Ok(json!({ "done": POLLS }))
        }
    }

    impl Drop for ExportApp {
        fn drop(&mut self) {
            if self.shared.hold_drop.load(Ordering::SeqCst) {
                let _ = self.shared.drop_released.wait();
            }
            self.shared.dropped.store(true, Ordering::SeqCst);
        }
    }

    /// Every progress report the engine relays, as an event channel the test waits on.
    struct ProgressRx(mpsc::Receiver<Report>);

    impl ProgressRx {
        /// The next report the engine relayed, waiting for it up to the deadline.
        fn next(&self) -> Option<Report> {
            self.0.recv_timeout(DEADLINE).ok()
        }
        /// Whether no report is waiting now.
        fn none_yet(&self) -> bool {
            self.0.try_recv().is_err()
        }
    }

    /// A listener that sends each report it hears to the channel the test reads.
    fn listener() -> (impl FnMut(u64, u64, Option<f64>) + Send + 'static, ProgressRx) {
        let (events, reports) = mpsc::channel();
        let listen = move |done, total, seconds_left| {
            let _ = events.send((done, total, seconds_left));
        };
        (listen, ProgressRx(reports))
    }

    /// Hand one export to the engine from the test's own thread, so it lands in the inbox behind
    /// everything handed over before it and ahead of everything after, with no thread to schedule.
    fn queue_export(engine: &Engine, cancel: &CancelToken) -> (ProgressRx, mpsc::Receiver<Result<Value>>) {
        let (listen, progress) = listener();
        (progress, engine.submit(ExportCall::message("export", Value::Null, listen, cancel)).unwrap())
    }

    /// Start an engine and one export on it, through the public call a caller makes, with `listen`
    /// as the caller's listener, and take the parts back: from this call's return the engine has
    /// answered the export's first beat, so every test starts at one proven point inside a running
    /// export, between two beats.
    fn start_export(listen: impl FnMut(u64, u64, Option<f64>) + Send + 'static) -> (Arc<Engine>, Arc<Shared>, CancelToken, mpsc::Receiver<Result<Value>>) {
        let (app, shared) = export_app();
        let engine = Arc::new(Engine::start(app).unwrap());
        let cancel = CancelToken::new();
        let (answer, answers) = mpsc::channel();
        let (exporting, token) = (engine.clone(), cancel.clone());
        thread::spawn(move || answer.send(exporting.request_with_progress("export", Value::Null, listen, &token)));
        assert!(eventually(|| shared.answered.load(Ordering::SeqCst) == 1), "the engine never answered the export's first beat");
        (engine, shared, cancel, answers)
    }

    /// [`start_export`] with a listener the test reads, which has heard the export's first report.
    fn started_export() -> (Arc<Engine>, Arc<Shared>, CancelToken, ProgressRx, mpsc::Receiver<Result<Value>>) {
        let (listen, progress) = listener();
        let (engine, shared, cancel, answers) = start_export(listen);
        assert_eq!(progress.next(), Some((0, POLLS, Some(0.0))), "the export never started");
        (engine, shared, cancel, progress, answers)
    }

    /// The plan §16 interval, written out rather than read from the constant, so an interval
    /// retyped to anything else fails here.
    #[test]
    fn the_progress_interval_is_the_clis_interval() {
        assert_eq!(PROGRESS_INTERVAL, Duration::from_millis(100));
    }

    /// While an export runs, an ordinary call waits: it is not answered while the engine is still
    /// inside the export, only after the export ended, and the calls waiting run in the order they
    /// were made (plan §13, L3).
    #[test]
    fn a_call_during_an_export_waits_until_it_ends_then_runs_in_order() {
        let (engine, shared, _cancel, progress, answers) = started_export();
        let waiting: Vec<_> = (1..=3).map(|n| queue_call(&engine, "echo", json!({ "n": n }))).collect();

        // The export moves, its next poll arrives, and still none of the waiting calls has been
        // answered: the engine ran no call while it was inside the export.
        shared.open(1);
        assert_eq!(progress.next(), Some((1, POLLS, Some(1.0))), "the export never polled again");
        assert!(waiting.iter().all(|call| call.try_recv().is_err()), "a call was answered during the export");

        // Let the export end; the waiting calls run after it, in the order they were made.
        shared.open(POLLS - 1);
        assert_eq!(recv(&answers), Ok(json!({ "done": POLLS })));
        for (n, call) in (1..=3).zip(&waiting) {
            assert_eq!(recv(call), Ok(json!({ "n": n })));
        }
        let ran: Vec<String> = lock(&shared.log).iter().filter(|line| !line.starts_with("info")).cloned().collect();
        assert_eq!(ran, ["export export null", "echo {\"n\":1}", "echo {\"n\":2}", "echo {\"n\":3}"], "the calls ran out of order");
        engine.close().unwrap();
    }

    /// A call the export's beats took out of the inbox runs before one still in the inbox when the
    /// export ends: the inbox then holds only calls made after the export's last beat, which are
    /// newer than every call a beat took out (plan §13, L3).
    #[test]
    fn calls_taken_out_during_an_export_run_before_those_made_after_its_last_beat() {
        let (engine, shared, _cancel, _progress, answers) = started_export();
        shared.holds_its_end.store(true, Ordering::SeqCst);
        let earlier = queue_call(&engine, "echo", json!({ "n": 1 }));
        shared.open(POLLS);
        // Past its last report and not yet answered: no beat is left to take a call out.
        assert!(shared.at_end.wait(), "the export never reached its end");
        let later = queue_call(&engine, "echo", json!({ "n": 2 }));
        shared.end_released.open();
        assert_eq!(recv(&answers), Ok(json!({ "done": POLLS })));
        assert_eq!(recv(&earlier), Ok(json!({ "n": 1 })));
        assert_eq!(recv(&later), Ok(json!({ "n": 2 })));
        assert_eq!(shared.logged("echo"), ["echo {\"n\":1}", "echo {\"n\":2}"], "a call made after the export's last beat ran ahead of one made before it");
        engine.close().unwrap();
    }

    /// `info` is the exception L3 makes: an `info` call made while an export runs is answered on
    /// the export's next beat, while the export is still running, with the state the app answered
    /// when the export began (plan §16 behaviour 3).
    #[test]
    fn info_is_answered_during_an_export_from_the_state_at_its_start() {
        let (engine, shared, _cancel, progress, answers) = started_export();

        // A call waits ahead of the ask, so the beat has to look past the calls it keeps waiting.
        let ahead = queue_call(&engine, "echo", json!({ "n": 1 }));
        let asking = queue_call(&engine, "info", Value::Null);
        // Between polls the engine sits inside the app and answers nothing.
        assert!(asking.try_recv().is_err(), "info answered between the engine's beats");

        // The app's state moves while the export runs; the answer the export's `info` calls get is
        // the one its start took.
        shared.state.store(99, Ordering::SeqCst);
        shared.open(1);
        assert_eq!(progress.next(), Some((1, POLLS, Some(1.0))));
        // A beat answers what it takes out of the inbox before it relays its report, so the answer
        // is here by the time the report is.
        let answered = asking.try_recv();
        assert!(answered.is_ok(), "info was not answered at the export's next beat");
        assert_eq!(answered.ok(), Some(Ok(json!({ "state": 0 }))), "info answered live state, not the export's start");
        assert!(ahead.try_recv().is_err(), "the call ahead of info ran during the export");
        assert!(answers.try_recv().is_err(), "the export ended before its info call was answered");
        // Nothing new is drained before the next beat: a second ask waits for it too.
        let later = queue_call(&engine, "info", json!({}));
        assert!(later.try_recv().is_err());
        shared.open(POLLS - 1);
        assert_eq!(recv(&answers), Ok(json!({ "done": POLLS })));
        assert_eq!(recv(&later), Ok(json!({ "state": 0 })));
        assert_eq!(recv(&ahead), Ok(json!({ "n": 1 })));
        // The app was asked for `info` exactly once, at the export's start: the calls waiting
        // behind are answered from that answer, not by re-entering the app.
        assert_eq!(shared.logged("info").len(), 1, "the app was asked for info once per export");
        // After the export, `info` is the app's own live answer again.
        assert_eq!(engine.request("info", Value::Null), Ok(json!({ "state": 99 })));
        engine.close().unwrap();
    }

    /// A cancelled export stops at the very next progress poll, within one progress interval of
    /// the cancel, and the call answers `Cancelled` whatever the stopped app answered (plan §16).
    #[test]
    fn a_cancel_stops_the_export_at_the_next_progress_poll_and_answers_cancelled() {
        let (engine, shared, cancel, progress, answers) = started_export();

        cancel.cancel();
        shared.open(1);
        // The poll that follows the cancel still reports its progress, which it made before the
        // engine answered it `false`, and it is the poll that sees the stop: one interval, no
        // later.
        assert_eq!(progress.next(), Some((1, POLLS, Some(1.0))));
        assert!(shared.wait_for(&shared.stop_poll, 1), "the app never saw the progress closure answer false");
        // The app answered its stop with a failure of its own; the caller asked to stop and hears
        // that it did.
        assert_eq!(recv(&answers), Err(CraftError::Cancelled));
        // The export stopped there: every report it made was relayed before its answer, and the
        // poll that saw the stop was its last.
        assert!(progress.none_yet(), "the export reported past the stop");
        assert_eq!(shared.polls.load(Ordering::SeqCst), 2, "the export ran past the poll that saw the stop");
        // A call after the cancelled export runs normally: the session is intact.
        assert_eq!(engine.request("echo", json!({ "n": 7 })), Ok(json!({ "n": 7 })));
        engine.close().unwrap();
    }

    /// A token set before its export's turn came is answered `Cancelled` without the app running
    /// any of the export.
    #[test]
    fn an_export_already_cancelled_before_it_starts_answers_cancelled_without_running() {
        let (app, shared) = export_app();
        let engine = Engine::start(app).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let (progress, answers) = queue_export(&engine, &cancel);
        assert_eq!(recv(&answers), Err(CraftError::Cancelled));
        assert!(progress.none_yet());
        assert!(lock(&shared.log).is_empty(), "the export ran although it was cancelled first");
        assert_eq!(shared.polls.load(Ordering::SeqCst), 0);
        engine.close().unwrap();
    }

    /// An app that keeps running after the engine answered its progress closure `false`, a fake
    /// that ignores the stop, still answers `Cancelled`: the caller asked to stop and the engine
    /// heard it, whatever the app made of it afterwards.
    #[test]
    fn an_app_that_ignores_the_stop_still_answers_cancelled() {
        let (engine, shared, cancel, _progress, answers) = started_export();
        // The fake reads this flag only when it sees a `false`, which comes after the cancel below.
        shared.stops_when_stopped.store(false, Ordering::SeqCst);
        cancel.cancel();
        shared.open(POLLS);
        // The export ran to its end (`{"done": …}` is what the app tried to answer); the caller
        // hears `Cancelled`, and the stop was still seen at the first poll after the cancel.
        assert_eq!(recv(&answers), Err(CraftError::Cancelled));
        assert!(shared.wait_for(&shared.stop_poll, 1));
        assert_eq!(
            shared.polls.load(Ordering::SeqCst),
            usize::try_from(POLLS + 1).unwrap(),
            "the app did not run the export to its end after ignoring the stop"
        );
        // Every report after the first `false` was answered `false` too.
        assert!(!shared.resumed.load(Ordering::SeqCst), "the engine answered `true` again after it had answered `false`");
        engine.close().unwrap();
    }

    /// A panic inside the app's export is answered as a panic in any call is: `Internal` with the
    /// panic's message, and the session keeps serving (`CraftApp::request_with_progress`, plan §16).
    #[test]
    fn a_panicking_export_answers_internal_and_the_session_stays() {
        let (app, shared) = export_app();
        let engine = Engine::start(app).unwrap();
        shared.panics_in_export.store(true, Ordering::SeqCst);
        let (progress, answers) = queue_export(&engine, &CancelToken::new());
        assert_eq!(recv(&answers), Err(CraftError::Internal { message: "boom in export".to_string() }));
        assert!(progress.none_yet(), "a call that answered `Internal` reported progress");
        assert_eq!(engine.request("echo", json!({ "n": 1 })), Ok(json!({ "n": 1 })), "the session died with the export");
        engine.close().unwrap();
    }

    /// An export that dies in a panic on its way out after being asked to stop answers `Internal`:
    /// the caught panic is what the guard found, `Cancelled` only what the caller asked for. A
    /// wrong answer here would report a crashed export as a clean cancel.
    #[test]
    fn a_panic_after_the_stop_outranks_the_cancel() {
        let (engine, shared, cancel, _progress, answers) = started_export();
        shared.panics_when_stopped.store(true, Ordering::SeqCst);
        cancel.cancel();
        shared.open(1);
        assert_eq!(recv(&answers), Err(CraftError::Internal { message: "boom after stop".to_string() }));
        engine.close().unwrap();
    }

    /// `close` while an export runs stops it as a cancel does, at its next poll, and returns only
    /// once the export has stopped, the session has ended and the thread is joined (plan §16
    /// behaviour 4). The caller's own token is left as it was: it may be shared with other work.
    #[test]
    fn close_during_an_export_stops_it_as_a_cancel_does_and_joins_the_thread() {
        let (engine, shared, cancel, _progress, answers) = started_export();

        let closer = close_begun(&engine);
        shared.open(1);
        assert!(shared.wait_for(&shared.stop_poll, 1), "the close did not stop the export at its next poll");
        assert_eq!(recv(&answers), Err(CraftError::Cancelled), "a close answered the export as something other than a cancel");
        assert!(!cancel.is_cancelled(), "the close set the caller's own token");
        // The close came back only after the thread ended, which is what joining means, and the
        // session had ended by then.
        assert_eq!(closer.join().unwrap(), Ok(()));
        assert!(shared.dropped.load(Ordering::SeqCst), "close returned before the session ended");
        assert_eq!(engine.request("echo", Value::Null), Err(CraftError::internal("engine is closed")));
        engine.close().unwrap();
    }

    /// An export still waiting for its turn when a close begins is stopped too: it answers
    /// `Cancelled` without running, so the close waits for no export to run to its end.
    #[test]
    fn close_stops_an_export_waiting_behind_the_one_running_without_running_it() {
        let (engine, shared, _cancel, _progress, answers) = started_export();
        let (waiting_progress, waiting_answers) = queue_export(&engine, &CancelToken::new());

        let closer = close_begun(&engine);
        shared.open(1);
        assert_eq!(recv(&answers), Err(CraftError::Cancelled));
        assert_eq!(recv(&waiting_answers), Err(CraftError::Cancelled), "the waiting export was not stopped by the close");
        assert!(waiting_progress.none_yet(), "the waiting export ran after the close");
        assert_eq!(shared.logged("export").len(), 1, "the waiting export ran after the close");
        assert_eq!(closer.join().unwrap(), Ok(()));
        assert!(shared.dropped.load(Ordering::SeqCst));
    }

    /// A cancel of an export still waiting for its turn is honoured at that turn: the export answers
    /// `Cancelled` without running, and the export running ahead of it is not touched.
    #[test]
    fn an_export_cancelled_while_it_waits_its_turn_answers_cancelled_without_running() {
        let (engine, shared, _cancel, _progress, answers) = started_export();
        let token = CancelToken::new();
        let (queued_progress, queued_answers) = queue_export(&engine, &token);
        token.cancel();
        shared.open(POLLS);
        assert_eq!(recv(&answers), Ok(json!({ "done": POLLS })), "cancelling the waiting export stopped the running one");
        assert_eq!(recv(&queued_answers), Err(CraftError::Cancelled));
        assert!(queued_progress.none_yet(), "the export cancelled while it waited reported progress");
        assert_eq!(shared.logged("export").len(), 1, "the export cancelled while it waited ran at its turn");
        engine.close().unwrap();
    }

    /// A call that races a close during an export, sent once the close was queued, is taken out of
    /// the inbox by the export's beat like any call, and is refused as soon as the engine stops,
    /// before the session's drop has ended, as a call behind a close always is.
    #[test]
    fn a_call_racing_a_close_during_an_export_is_refused_before_the_session_ends() {
        let (engine, shared, _cancel, _progress, answers) = started_export();
        shared.hold_drop.store(true, Ordering::SeqCst);
        let racer = engine.racer().unwrap();

        let closer = close_begun(&engine);
        let (answer, raced) = mpsc::channel();
        racer.send(Message::request("echo", json!({ "n": 1 }))(answer)).unwrap();
        shared.open(1);
        assert_eq!(recv(&answers), Err(CraftError::Cancelled));
        assert_eq!(raced.recv_timeout(DEADLINE), Err(RecvTimeoutError::Disconnected), "a call behind the close was not refused");
        assert!(!shared.dropped.load(Ordering::SeqCst), "a call behind the close waited for the session's drop");
        shared.drop_released.open();
        assert_eq!(closer.join().unwrap(), Ok(()));
        assert!(shared.logged("echo").is_empty(), "a call behind the close ran");
    }

    /// A progress listener that fails is dropped and the export runs on: a panic out of the
    /// caller's callback is not allowed to stop the export or the engine (plan §16's edge case).
    #[test]
    fn a_progress_listener_that_panics_is_ignored_and_the_export_continues() {
        let (engine, shared, _cancel, answers) = start_export(|_done, _total, _seconds_left| panic!("the listener blew up"));
        shared.open(POLLS);
        assert_eq!(recv(&answers), Ok(json!({ "done": POLLS })), "a failing listener stopped the export");
        assert_eq!(shared.polls.load(Ordering::SeqCst), usize::try_from(POLLS + 1).unwrap());
        assert_eq!(engine.request("echo", json!({ "n": 1 })), Ok(json!({ "n": 1 })), "the engine died with the listener");
        engine.close().unwrap();
    }

    /// Ignoring a failing listener costs the caller nothing else: a cancel still stops the export
    /// at its next report, and the call answers `Cancelled`.
    #[test]
    fn a_progress_listener_that_panics_does_not_keep_a_cancel_from_stopping_the_export() {
        let (engine, shared, cancel, answers) = start_export(|_done, _total, _seconds_left| panic!("the listener blew up"));
        cancel.cancel();
        shared.open(1);
        assert!(shared.wait_for(&shared.stop_poll, 1), "a failing listener kept the cancel from stopping the export");
        assert_eq!(recv(&answers), Err(CraftError::Cancelled));
        engine.close().unwrap();
    }

    /// Progress arrives on the engine thread, as `Engine::request_with_progress` promises: the
    /// export runs on that thread (plan §13, L1), and the engine's beat — where it hands each
    /// report to the caller's listener — runs there with it. A listener cannot assert this from
    /// inside itself, because a failing listener is ignored (plan §16's edge case), so the
    /// listener records the name of the thread that called it and the test reads the record.
    #[test]
    fn progress_arrives_on_the_engine_thread() {
        let (naming, named) = mpsc::channel();
        let (_engine, shared, _cancel, answers) = start_export(move |_done, _total, _seconds_left| {
            let _ = naming.send(thread::current().name().unwrap_or("<unnamed>").to_string());
        });
        shared.open(POLLS);
        assert_eq!(recv(&answers), Ok(json!({ "done": POLLS })));
        let names: Vec<String> = named.try_iter().collect();
        assert_eq!(names.len(), usize::try_from(POLLS + 1).unwrap(), "the listener heard one report per poll");
        assert!(names.iter().all(|name| name == THREAD_NAME), "progress arrived off the engine thread: {names:?}");
    }

    /// A second export is a call like any other: it waits behind the export in flight and runs
    /// after it ends, never beside it (plan §13's one export at a time), and it takes its turn
    /// among the other calls waiting, after those made before it and before those made after.
    #[test]
    fn a_second_export_waits_behind_the_one_running_and_runs_after_it_ends() {
        let (engine, shared, _first_cancel, _first_progress, first_answers) = started_export();
        let before = queue_call(&engine, "echo", json!({ "n": 1 }));
        let (second_progress, second_answers) = queue_export(&engine, &CancelToken::new());
        let after = queue_call(&engine, "echo", json!({ "n": 2 }));

        shared.open(1);
        assert!(eventually(|| shared.polls.load(Ordering::SeqCst) == 2), "the first export never polled again");
        assert!(second_progress.none_yet(), "two exports ran at once");
        shared.open(POLLS - 1);
        assert_eq!(recv(&first_answers), Ok(json!({ "done": POLLS })));
        // Only now does the second export run, and the call made after it still waits.
        assert_eq!(second_progress.next(), Some((0, POLLS, Some(0.0))));
        assert!(after.try_recv().is_err(), "a call made after the second export ran while it did");
        shared.open(POLLS);
        assert_eq!(recv(&second_answers), Ok(json!({ "done": POLLS })));
        assert_eq!(recv(&before), Ok(json!({ "n": 1 })));
        assert_eq!(recv(&after), Ok(json!({ "n": 2 })));
        let ran: Vec<String> = lock(&shared.log).iter().filter(|line| !line.starts_with("info")).cloned().collect();
        assert_eq!(ran, ["export export null", "echo {\"n\":1}", "export export null", "echo {\"n\":2}"], "the second export did not take its turn in order");
        engine.close().unwrap();
    }

    /// An app with no request that runs with progress keeps the trait's default: an export asked
    /// of it answers `Unsupported` naming the method, reports nothing, and the engine serves on.
    #[test]
    fn an_app_without_a_progress_request_answers_unsupported_and_keeps_serving() {
        struct PlainApp;
        impl CraftApp for PlainApp {
            fn request(&mut self, _method: &str, params: Value) -> Result<Value> {
                Ok(params)
            }
        }
        let engine = Engine::start(PlainApp).unwrap();
        let (progress, answers) = queue_export(&engine, &CancelToken::new());
        assert_eq!(recv(&answers), Err(CraftError::unsupported("`export` does not run with progress in this app")));
        assert!(progress.none_yet(), "an app without the request reported progress");
        assert_eq!(engine.request("echo", json!({ "n": 1 })), Ok(json!({ "n": 1 })));
        engine.close().unwrap();
    }
}
