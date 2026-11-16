//! The engine actor: one thread per engine, calls answered in the order they were made, and a
//! panic inside the app answered as `Internal` with the session kept (plan §16).

use crate::Result;
use crate::app::CraftApp;
use crate::cancel::CancelToken;
use crate::error::CraftError;
use crate::export::{self, ExportCall};
use crate::frames::{FrameBytes, PixelFormat};
use serde_json::Value;
use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};

/// The name every engine thread carries, so a thread listing of the host app shows the engine (plan §16).
/// Crate-visible so an export test checks its progress against this one name, not a copy of it.
pub(crate) const THREAD_NAME: &str = "craftbridge-engine";

/// The reason every call answers after [`Engine::close`], or loses the race to it.
const CLOSED: &str = "engine is closed";

/// The reason a call made from inside one of the engine's own calls is refused: the engine runs one
/// call at a time, so it could answer that call only after the call that made it, which is waiting
/// for the answer, and the two would wait for each other for ever.
const REENTERED: &str = "engine called from inside one of its own calls";

/// The answer when a panic's payload panics again as it is dropped, so its message cannot be read.
const PAYLOAD_PANICKED: &str = "a caught panic's payload panicked as it was dropped";

/// A call to the engine thread and where to answer it. Each call carries its own answer channel,
/// used once: the channel a call is answered on is that call's alone, so an answer cannot reach a
/// waiting caller by mistake. An export carries its payload as one `ExportCall`, so everything the
/// export mechanism needs travels as one item and the export's own file owns it.
pub(crate) enum Message {
    Request { method: String, params: Value, answer: Sender<Result<Value>> },
    Frame { params: Value, format: PixelFormat, answer: Sender<Result<FrameBytes>> },
    Export { call: ExportCall },
    Close,
}

impl Message {
    /// A plain call of `method`, built once the engine gives it the channel its answer goes back on.
    pub(crate) fn request(method: &str, params: Value) -> impl FnOnce(Sender<Result<Value>>) -> Message {
        let method = method.to_string();
        move |answer| Message::Request { method, params, answer }
    }

    /// A frame call, built once the engine gives it the channel its answer goes back on.
    pub(crate) fn frame(params: Value, format: PixelFormat) -> impl FnOnce(Sender<Result<FrameBytes>>) -> Message {
        move |answer| Message::Frame { params, format, answer }
    }
}

/// A handle to one engine: its own thread, and its own session of a [`CraftApp`].
///
/// Two engines in one app are allowed, each with its own thread and session (plan §16). A handle is
/// `Send + Sync`, so every thread of the app calls the same engine through it.
///
/// Dropping a handle closes the engine, as [`Engine::close`] does: the session ends and the thread is
/// joined before the drop returns, so nothing the session holds outlives the handle.
pub struct Engine {
    /// The actor's inbox. `close` takes it, so a call after close is refused here, in the caller's
    /// thread, without waiting for the engine (plan §16: a call after close is `Internal`).
    inbox: Mutex<Option<Sender<Message>>>,
    /// The engine thread, answering with how its session ended. The first close from outside the
    /// engine's own calls takes it and joins it while holding the lock, so a racing close returns only
    /// once the thread has stopped.
    thread: Mutex<Option<JoinHandle<Result<()>>>>,
    /// Set once a close has begun. Every export reads it beside its caller's token, when it starts
    /// and at each progress beat, so a close stops the export running and every export waiting
    /// behind it as a cancel does (plan §16 behaviour 4). It is the engine's own flag rather than the
    /// callers' tokens, which a caller may share with work on another engine that this close must
    /// not stop.
    closing: CancelToken,
    /// The engine thread's id, so a call from inside one of the engine's own calls is told apart.
    id: ThreadId,
}

impl Engine {
    /// Start one engine: spawn its thread and run `app`'s session there, and answer with the handle
    /// every call goes through. The later steps of plan §16's `start` (folders, the GPU, the
    /// decoders) arrive around this spawn, which is the step that defines the thread and the inbox.
    ///
    /// [`Result::Err`] only when the thread itself cannot start; a session that fails to build is the
    /// app crate's own `start` to judge, once the steps around this one exist.
    pub fn start<A: CraftApp + 'static>(app: A) -> Result<Engine> {
        let (inbox, incoming) = mpsc::channel();
        let closing = CancelToken::new();
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_string())
            .spawn({
                let closing = closing.clone();
                move || run(Box::new(app), incoming, closing)
            })
            .map_err(|error| CraftError::internal(format!("cannot start the engine thread: {error}")))?;
        Ok(Engine { id: thread.thread().id(), inbox: Mutex::new(Some(inbox)), thread: Mutex::new(Some(thread)), closing })
    }

    /// Make one call and wait for the engine's answer.
    ///
    /// This blocks the calling thread; the FFI layer hands the wait to its async runtime so an
    /// app's main thread never blocks (plan §14, §20). The call runs on the engine thread in the
    /// order the calls were made, and a panic inside the app is answered as [`CraftError::Internal`]
    /// — this call never returns a panic and never stops the session. A call made from inside one of
    /// this engine's own calls is refused with `Internal` at once rather than waiting for itself.
    pub fn request(&self, method: &str, params: Value) -> Result<Value> {
        answer_of(self.submit(Message::request(method, params))?)
    }

    /// Make one long call that reports progress and can be cancelled, and wait for the engine's
    /// answer (plan §16, §19). It waits its turn as any call does, then runs on the app's
    /// [`CraftApp::request_with_progress`]; while it runs the engine answers only progress, cancel
    /// and `info` (plan §13, L3). Progress arrives on `on_progress`, on the engine thread, as the app
    /// reports it, and a listener that fails is ignored, so a panicking listener never stops the
    /// export (plan §16's edge case). A cancel is `cancel` set from any thread, needing no call to
    /// the engine: the app is told to stop at its next report, and the call answers
    /// [`CraftError::Cancelled`]; a token already set when the export's turn comes answers that
    /// without running it. An `info` call made meanwhile is answered at the export's next report
    /// with the state the app described when the export began. Every other call waits in the order
    /// it was made until the export ends. A call made from inside one of this engine's own calls is
    /// refused with `Internal` at once, as [`Engine::request`] does.
    pub fn request_with_progress<F>(&self, method: &str, params: Value, on_progress: F, cancel: &CancelToken) -> Result<Value>
    where
        F: FnMut(u64, u64, Option<f64>) + Send + 'static,
    {
        answer_of(self.submit(ExportCall::message(method, params, on_progress, cancel))?)
    }

    /// Render one frame as `format` and wait for the engine's answer (plan §14, §16). It is the
    /// call of a frame, which binary pixels need because `request`'s JSON cannot carry them: it
    /// waits its turn as any call does, runs on the app's [`CraftApp::frame`], and so waits behind
    /// an export in the order it was made (plan §13, L3). A panic inside the app is answered as
    /// [`CraftError::Internal`] and the session stays, and a call made from inside one of this
    /// engine's own calls is refused with `Internal` at once, as [`Engine::request`] does.
    pub fn frame(&self, params: Value, format: PixelFormat) -> Result<FrameBytes> {
        answer_of(self.submit(Message::frame(params, format))?)
    }

    /// Hand one call to the engine thread and take the channel its answer comes back on: the one
    /// way every call reaches the engine. A call from inside one of this engine's own calls is
    /// refused with `Internal` at once rather than waiting for itself, and one after close is
    /// refused without waiting for the engine.
    pub(crate) fn submit<T>(&self, message: impl FnOnce(Sender<Result<T>>) -> Message) -> Result<Receiver<Result<T>>> {
        if self.on_engine_thread() {
            return Err(CraftError::internal(REENTERED));
        }
        let Some(inbox) = lock(&self.inbox).clone() else { return Err(closed()) };
        hand_over(&inbox, message)
    }

    /// Stop the engine: stop the thread after the calls already accepted, end the session, and join
    /// the thread. Answers `Internal` when the session's own drop panicked.
    ///
    /// Idempotent: the close that joins the thread answers with how the session ended, and every
    /// other close `Ok`, each only once the thread has stopped. The exception is a close from inside
    /// one of the engine's own calls, which cannot wait for that call to end: it refuses new calls
    /// and answers `Ok` at once, and the thread stops when the call ends. A call still running is
    /// finished before the thread stops, so close waits for it, bar an export: a close stops the
    /// export running and every export waiting as a cancel does, so the wait is the running one's
    /// next progress report, not its whole length, and a waiting one answers `Cancelled` without
    /// running (plan §16 behaviour 4). Any other call handed to the engine before the close is
    /// answered first, in order; one that races the close is either answered whole or refused with
    /// `engine is closed`, never left waiting on a dead engine; and once any close has returned,
    /// every call is refused.
    pub fn close(&self) -> Result<()> {
        // The inbox goes first, so new calls are refused from here on. Close queues behind the calls
        // already accepted, so they are all answered first, and dropping this sender after it
        // disconnects the inbox for good.
        let inbox = lock(&self.inbox).take();
        if let Some(inbox) = inbox {
            let _ = inbox.send(Message::Close);
        }
        // Every export stops as a cancel stops it (plan §16 behaviour 4), so the join below waits for
        // the running one's next progress report, not its end. Set once `Close` is queued, so a call
        // sent by anyone who sees the engine closing lands behind it and is refused.
        self.closing.cancel();
        // A close from inside one of the engine's own calls cannot wait for that call to end, and a
        // thread joining itself panics: the thread stops once the call ends, and a close or drop from
        // outside joins it.
        if self.on_engine_thread() {
            return Ok(());
        }
        let mut thread = lock(&self.thread);
        let Some(handle) = thread.take() else { return Ok(()) };
        // A join error would mean a panic outside every guard, which the loop does not leave room
        // for; the thread is gone either way, and the app hears `Internal`, never a crash.
        handle.join().unwrap_or_else(|_| Err(CraftError::internal("the engine thread stopped unexpectedly")))
    }

    /// Whether the caller is this engine's own thread, which is inside one of its calls.
    fn on_engine_thread(&self) -> bool {
        thread::current().id() == self.id
    }
}

#[cfg(test)]
impl Engine {
    /// Whether a close has begun, by which point its `Close` is queued and every export stops.
    pub(crate) fn is_closing(&self) -> bool {
        self.closing.is_cancelled()
    }

    /// A sender for the inbox, as a caller holds one it took before a close took the inbox, for
    /// the tests of a call that races a close.
    pub(crate) fn racer(&self) -> Option<Sender<Message>> {
        lock(&self.inbox).clone()
    }
}

impl Drop for Engine {
    /// A handle dropped without `close` closes: an unjoined thread would end the session at some
    /// later time, after the app has let go of what the session borrowed from it.
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// The value behind `mutex`. Poisoning can only come from a panic while the lock was held, and none
/// of the engine's locks holds more than an `Option` of a channel or thread handle, which a panic
/// cannot leave half-written, so the poisoned guard is used rather than losing the engine.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hand one call to the engine through `inbox`, giving it the channel its answer comes back on.
///
/// A call the engine stopped before running lost its race to a close: it is refused with `engine is
/// closed` and never answered as though it ran, so a write that never happened cannot read as done.
/// This send fails when the engine had already stopped; [`answer_of`] fails when a close ran ahead
/// of the call in the queue and the engine dropped it unrun.
fn hand_over<T>(inbox: &Sender<Message>, message: impl FnOnce(Sender<Result<T>>) -> Message) -> Result<Receiver<Result<T>>> {
    let (answer, waiting) = mpsc::channel();
    inbox.send(message(answer)).map_err(|_| closed())?;
    Ok(waiting)
}

/// Wait for the answer to a call [`hand_over`] handed over; a call the engine dropped unrun is
/// refused with `engine is closed`.
fn answer_of<T>(waiting: Receiver<Result<T>>) -> Result<T> {
    waiting.recv().map_err(|_| closed())?
}

/// The engine thread's loop: take one call, run it on the app, answer it, and take the next; then
/// end the session. Answers with how the session's drop went.
///
/// One call at a time, in inbox order, because the session is reached only here (plan §16, L1). The
/// loop ends at `Close` or when every handle has dropped the inbox. While an export runs, calls
/// that arrive are taken into `pending` at the export's beats, in order, and run from there before
/// anything made after them (plan §13, L3); `pending` is why the loop takes its next message from
/// it before it waits on the inbox.
fn run(mut app: Box<dyn CraftApp>, incoming: Receiver<Message>, closing: CancelToken) -> Result<()> {
    let mut pending: VecDeque<Message> = VecDeque::new();
    while let Some(message) = next_message(&mut pending, &incoming) {
        match message {
            Message::Close => break,
            Message::Request { method, params, answer } => answer_call(&answer, || app.request(&method, params)),
            Message::Frame { params, format, answer } => answer_call(&answer, || app.frame(params, format)),
            Message::Export { call } => export::run_export(&mut *app, call, export::EngineSide { inbox: &incoming, pending: &mut pending, closing: &closing }),
        }
    }
    // Calls queued behind the close are refused now, not after the session's drop has run: those
    // still in the inbox, and those an export's beats took out of it, which wait in `pending`.
    // Their callers read the disconnect as `engine is closed`.
    drop(incoming);
    drop(pending);
    // The session's drop is app code too, so it runs behind the same guard as a call: a panic there
    // is the close's answer rather than a thread that died outside its guard.
    guarded(move || drop(app))
}

/// Run one call of the app behind the never-crash guard and send what it answered, or the panic it
/// ended in, back to its caller. The caller may already have gone (a UI that stopped waiting);
/// losing the answer is not an error for the engine — the call still ran, in order, and the next
/// call is answered normally.
fn answer_call<T>(answer: &Sender<Result<T>>, call: impl FnOnce() -> Result<T>) {
    let _ = answer.send(guarded(call).flatten());
}

/// The message to run next: one kept waiting during an export first, in the order it arrived, and
/// otherwise the next the inbox holds.
fn next_message(pending: &mut VecDeque<Message>, incoming: &Receiver<Message>) -> Option<Message> {
    match pending.pop_front() {
        Some(message) => Some(message),
        None => incoming.recv().ok(),
    }
}

/// Run `work` behind the never-crash boundary (AGENTS §3): a panic that unwinds out of it becomes
/// `Internal` carrying the panic's message, and the engine thread goes on.
///
/// `AssertUnwindSafe` is deliberate: a panic may leave the session half-changed, and plan §14 and §16
/// keep the session as the panic left it — the document is kept, not rolled back — as EffectCraft's
/// own guard does. Only an unwinding panic can be caught: one raised while another unwinds, by a
/// destructor in the app's own cleanup, is an abort Rust itself makes, before any guard runs.
pub(crate) fn guarded<T>(work: impl FnOnce() -> T) -> Result<T> {
    panic::catch_unwind(AssertUnwindSafe(work)).map_err(|caught| {
        // Reading the payload drops it, and its drop is app code as well, run after the guard above
        // has returned: one that panics again is caught here, and what it panicked with is leaked
        // rather than dropped a third time.
        panic::catch_unwind(AssertUnwindSafe(|| CraftError::from_panic(caught))).unwrap_or_else(|again| {
            std::mem::forget(again);
            CraftError::internal(PAYLOAD_PANICKED)
        })
    })
}

/// The error every refused call answers.
fn closed() -> CraftError {
    CraftError::internal(CLOSED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{DEADLINE, Gate, close_begun, eventually, queue_call, recv};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock, Weak};

    /// The straight-RGBA frame the fake app renders for `params`: one pixel whose bytes are the
    /// call's `v`, so a frame that came back crossed with another call's shows in its bytes.
    fn pixel_of(params: &Value) -> Vec<u8> {
        let v = params.get("v").and_then(Value::as_u64).unwrap_or_default();
        vec![u8::try_from(v).unwrap_or(u8::MAX), 0, 0, 255]
    }

    /// The fake app the runtime is proven on: it keeps a session (a value, plus a log of the calls
    /// it ran in the order it ran them, which is the engine's ordering made visible), and it can be
    /// told to panic, to block until the test releases it, to call its own engine, or to hold its
    /// drop until the test releases it or to panic in it.
    struct FakeApp {
        log: Vec<String>,
        value: i64,
        shared: Arc<Shared>,
        hold_drop: bool,
        drop_panicking: bool,
    }

    /// What the test and its fake app share. `entered` opens when the blocking call has reached the
    /// app and `released` lets it end; `drop_entered` opens when the session's drop has begun and
    /// `drop_released` lets a held drop end; `dropped` says the session has ended; and `engine` is
    /// the app's own engine, held weakly so the session does not keep its own engine alive.
    struct Shared {
        entered: Gate,
        released: Gate,
        drop_entered: Gate,
        drop_released: Gate,
        dropped: AtomicBool,
        engine: OnceLock<Weak<Engine>>,
    }

    /// A panic payload whose own drop panics again.
    struct PanicsOnDrop;

    impl Drop for PanicsOnDrop {
        fn drop(&mut self) {
            panic!("the payload's drop");
        }
    }

    impl FakeApp {
        fn new() -> (FakeApp, Arc<Shared>) {
            let shared = Arc::new(Shared {
                entered: Gate::default(),
                released: Gate::default(),
                drop_entered: Gate::default(),
                drop_released: Gate::default(),
                dropped: AtomicBool::new(false),
                engine: OnceLock::new(),
            });
            (FakeApp { log: Vec::new(), value: 0, shared: shared.clone(), hold_drop: false, drop_panicking: false }, shared)
        }

        fn own_engine(&self) -> Result<Arc<Engine>> {
            self.shared.engine.get().and_then(Weak::upgrade).ok_or_else(|| CraftError::internal("no engine to call"))
        }
    }

    impl CraftApp for FakeApp {
        fn request(&mut self, method: &str, params: Value) -> Result<Value> {
            self.log.push(format!("{method} {params}"));
            match method {
                "echo" => Ok(params),
                "value_set" => {
                    self.value = params.get("v").and_then(Value::as_i64).unwrap_or_default();
                    Ok(Value::Null)
                }
                "value_get" => Ok(Value::from(self.value)),
                "log" => Ok(Value::from(self.log.iter().map(|entry| Value::String(entry.clone())).collect::<Vec<_>>())),
                "panic_str" => panic!("boom"),
                "panic_fmt" => panic!("boom {}", self.value),
                "panic_value" => std::panic::panic_any(42u64),
                "panic_payload_drop" => std::panic::panic_any(PanicsOnDrop),
                "panic_after_write" => {
                    self.value = params.get("v").and_then(Value::as_i64).unwrap_or_default();
                    panic!("boom after write");
                }
                // Blocks the engine thread until the test releases it; fails the test (an error,
                // not a hang) if the release never comes.
                "wait" => {
                    self.shared.entered.open();
                    if self.shared.released.wait() { Ok(Value::Null) } else { Err(CraftError::internal("never released")) }
                }
                "thread" => Ok(Value::from(thread::current().name().unwrap_or("<unnamed>").to_string())),
                "thread_id" => Ok(Value::from(format!("{:?}", thread::current().id()))),
                "reenter" => self.own_engine()?.request("echo", Value::Null),
                "reenter_progress" => self.own_engine()?.request_with_progress("echo", Value::Null, |_, _, _| {}, &CancelToken::new()),
                "close_self" => self.own_engine()?.close().map(|()| Value::Null),
                "hold_drop" => {
                    self.hold_drop = true;
                    Ok(Value::Null)
                }
                "drop_panicking" => {
                    self.drop_panicking = true;
                    Ok(Value::Null)
                }
                other => Err(CraftError::internal(format!("unknown method {other}"))),
            }
        }

        fn frame(&mut self, params: Value, format: PixelFormat) -> Result<FrameBytes> {
            self.log.push(format!("frame {params}"));
            match params.get("how").and_then(Value::as_str) {
                Some("panic") => panic!("boom in frame"),
                Some("reenter") => self.own_engine()?.frame(params, format),
                // A buffer that is not the size it names: the core's own check refuses it.
                Some("short") => FrameBytes::new(2, 2, format, pixel_of(&params)),
                _ => FrameBytes::new(1, 1, format, pixel_of(&params)),
            }
        }
    }

    impl Drop for FakeApp {
        fn drop(&mut self) {
            self.shared.drop_entered.open();
            if self.hold_drop {
                let _ = self.shared.drop_released.wait();
            }
            self.shared.dropped.store(true, Ordering::SeqCst);
            if self.drop_panicking {
                panic!("boom on drop");
            }
        }
    }

    fn call(engine: &Engine, method: &str) -> Result<Value> {
        engine.request(method, Value::Null)
    }

    /// One plain call through a sender a caller already holds, as [`Engine::request`] makes it.
    fn call_on(inbox: &Sender<Message>, method: &str, params: Value) -> Result<Value> {
        hand_over(inbox, Message::request(method, params)).and_then(answer_of)
    }

    /// An engine whose fake app can call its own engine, which the tests of a call made from inside a
    /// call need.
    fn self_aware_engine() -> (Arc<Engine>, Arc<Shared>) {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        assert!(shared.engine.set(Arc::downgrade(&engine)).is_ok());
        (engine, shared)
    }

    /// One call made to `engine` from a thread of its own, with a deadline on the answer, so an engine
    /// waiting for itself fails the test instead of hanging the suite.
    fn from_another_thread<T: Send + 'static>(
        engine: &Arc<Engine>,
        call: impl FnOnce(&Engine) -> Result<T> + Send + 'static,
    ) -> Result<Result<T>, mpsc::RecvTimeoutError> {
        let (answering, answered) = mpsc::channel();
        let calling = engine.clone();
        thread::spawn(move || answering.send(call(&calling)));
        answered.recv_timeout(DEADLINE)
    }

    /// Make the engine busy: a `wait` call that has reached the app and stays there until the test
    /// opens `released`, and the handle of the thread that made it. Nothing lets it end on a timer,
    /// so a call answered, or a close or drop returned, before the test opened the gate did so
    /// without waiting for the call in flight, however slowly the machine runs.
    fn busy(engine: &Arc<Engine>, shared: &Shared) -> JoinHandle<Result<Value>> {
        let waiting = engine.clone();
        let blocked = thread::spawn(move || call(&waiting, "wait"));
        assert!(shared.entered.wait(), "the blocked call never reached the app");
        blocked
    }

    /// Every call runs on the engine's own thread — the one named `craftbridge-engine` — and each
    /// call is answered with its own result, in the order the calls were made (L1).
    #[test]
    fn calls_run_in_order_on_the_engine_thread() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        // The plan's own name for the thread, written out rather than read from the constant, so a
        // renamed thread fails here.
        assert_eq!(call(&engine, "thread").unwrap(), Value::from("craftbridge-engine"), "plan §16 names the thread");
        assert_ne!(
            call(&engine, "thread_id").unwrap(),
            Value::from(format!("{:?}", thread::current().id())),
            "the call ran on the caller's thread, not the engine's"
        );
        // Rounds of set/get/log from one caller: each call sees exactly the session the calls before
        // it left, and the log grows by exactly the calls made, so none is lost, repeated or run
        // twice. One caller never has two calls queued at once; the order of queued calls is the
        // queued-call test's to prove.
        for round in 1..=30u64 {
            engine.request("value_set", json!({ "v": round })).unwrap();
            assert_eq!(engine.request("value_get", Value::Null).unwrap(), Value::from(round), "round {round}");
            let log = engine.request("log", Value::Null).unwrap();
            let entries = log.as_array().unwrap_or_else(|| panic!("round {round}: log answered {log}"));
            // The two opening thread reads, plus three calls per round including this one.
            assert_eq!(entries.len(), 2 + 3 * round as usize, "round {round}");
        }
        engine.close().unwrap();
    }

    /// Calls from many threads are each answered exactly once, with their own answer: nothing is
    /// dropped, duplicated, or crossed over between the answer channels.
    #[test]
    fn calls_from_many_threads_are_each_answered_once_with_their_own_answer() {
        let (app, _shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                let engine = engine.clone();
                thread::spawn(move || {
                    (0..25)
                        .filter(|n| {
                            let value = worker * 100 + n;
                            // `echo` answers with its own params: an answer reaching the wrong
                            // caller, or not reaching its caller at all, fails right here.
                            engine.request("echo", json!({ "v": value })).ok().filter(|answered| *answered == json!({ "v": value })).is_some()
                        })
                        .count()
                })
            })
            .collect();
        let answered: usize = workers.into_iter().map(|worker| worker.join().unwrap_or(0)).sum();
        assert_eq!(answered, 8 * 25);
        engine.close().unwrap();
    }

    /// Calls made while the engine is busy wait for the running call and then run in the order they
    /// were made: the engine's log shows the running call first, then each queued call in turn —
    /// never newest first — and none was answered while the engine was inside the running call (L1).
    #[test]
    fn calls_queued_behind_a_running_call_run_in_the_order_they_were_made() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let blocked = busy(&engine, &shared);
        let queued: Vec<_> = (0..5).map(|n| queue_call(&engine, "echo", json!({ "n": n }))).collect();
        assert!(queued.iter().all(|answer| answer.try_recv().is_err()), "a queued call was answered while the engine was inside the running call");
        shared.released.open();
        assert_eq!(blocked.join().unwrap(), Ok(Value::Null));
        for (n, answer) in queued.iter().enumerate() {
            assert_eq!(recv(answer), Ok(json!({ "n": n })), "the call queued {n} was answered as something else");
        }
        let echoes = (0..5).map(|n| format!("echo {}", json!({ "n": n })));
        let expected: Vec<String> = std::iter::once("wait null".to_string()).chain(echoes).chain(std::iter::once("log null".to_string())).collect();
        assert_eq!(call(&engine, "log").unwrap(), json!(expected));
    }

    /// A frame is a call like any other: it runs on the engine thread in the order the calls were
    /// made, behind a call still running, and each caller gets the frame its own call rendered
    /// (plan §16, L1), in the format it asked for.
    #[test]
    fn frames_run_on_the_engine_thread_in_call_order_each_answered_with_its_own() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let blocked = busy(&engine, &shared);
        let waiting: Vec<_> = (1..=3_u8).map(|n| engine.submit(Message::frame(json!({ "v": n }), PixelFormat::Rgba8Straight)).unwrap()).collect();
        let echo = queue_call(&engine, "echo", json!({ "after": true }));
        assert!(waiting.iter().all(|answer| answer.try_recv().is_err()), "a frame was answered while the engine was inside the running call");
        shared.released.open();
        assert_eq!(blocked.join().unwrap(), Ok(Value::Null));
        for (n, answer) in (1..=3_u8).zip(&waiting) {
            assert_eq!(recv(answer), FrameBytes::new(1, 1, PixelFormat::Rgba8Straight, vec![n, 0, 0, 255]), "frame {n} was answered as another call's");
        }
        assert_eq!(recv(&echo), Ok(json!({ "after": true })));
        let log = call(&engine, "log").unwrap();
        assert_eq!(log, json!(["wait null", "frame {\"v\":1}", "frame {\"v\":2}", "frame {\"v\":3}", "echo {\"after\":true}", "log null"]));
        engine.close().unwrap();
    }

    /// The format the caller asked for reaches the app and shapes what comes back, through the one
    /// `FrameBytes::new` the app calls: premultiplied BGRA for the format that asks for it.
    #[test]
    fn a_frame_is_answered_in_the_format_it_was_asked_in() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        let straight = engine.frame(json!({ "v": 200 }), PixelFormat::Rgba8Straight).unwrap();
        assert_eq!((straight.width, straight.height, straight.format, straight.data), (1, 1, PixelFormat::Rgba8Straight, vec![200, 0, 0, 255]));
        let bgra = engine.frame(json!({ "v": 200 }), PixelFormat::Bgra8Premultiplied).unwrap();
        assert_eq!((bgra.format, bgra.data), (PixelFormat::Bgra8Premultiplied, vec![0, 0, 200, 255]));
        engine.close().unwrap();
    }

    /// A failure of the app's frame is the call's answer and the session goes on: the core's size
    /// check of the bytes, a panic as `Internal` with the document kept (plan §14, §16).
    #[test]
    fn a_failed_frame_is_answered_and_the_session_stays() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        engine.request("value_set", json!({ "v": 5 })).unwrap();
        let refused = engine.frame(json!({ "how": "short" }), PixelFormat::Rgba8Straight).unwrap_err();
        assert_eq!(refused.to_string(), "frame 2x2 needs 16 raw bytes (width x height x 4), the buffer holds 4");
        assert_eq!(engine.frame(json!({ "how": "panic" }), PixelFormat::Png), Err(CraftError::Internal { message: "boom in frame".to_string() }));
        assert_eq!(call(&engine, "value_get").unwrap(), Value::from(5));
        assert!(engine.frame(json!({ "v": 1 }), PixelFormat::Png).is_ok(), "the engine stopped answering frames after a failed one");
        engine.close().unwrap();
    }

    /// An app that keeps the trait's default has no frames: `Unsupported`, and the engine serves on.
    #[test]
    fn an_app_without_frames_answers_unsupported_and_keeps_serving() {
        struct PlainApp;
        impl CraftApp for PlainApp {
            fn request(&mut self, _method: &str, params: Value) -> Result<Value> {
                Ok(params)
            }
        }
        let engine = Engine::start(PlainApp).unwrap();
        assert_eq!(engine.frame(Value::Null, PixelFormat::Png), Err(CraftError::unsupported("this app renders no frames")));
        assert_eq!(engine.request("echo", json!({ "n": 1 })), Ok(json!({ "n": 1 })));
        engine.close().unwrap();
    }

    /// A frame after close is refused as every call is, and one made from inside one of the engine's
    /// own calls is refused at once rather than waiting for itself.
    #[test]
    fn a_frame_after_close_or_from_inside_a_call_is_refused() {
        let (engine, _) = self_aware_engine();
        let reentered = from_another_thread(&engine, |engine| engine.frame(json!({ "how": "reenter" }), PixelFormat::Png));
        assert_eq!(reentered.map(|answer| answer.map(|_| ())), Ok(Err(CraftError::Internal { message: REENTERED.to_string() })));
        engine.close().unwrap();
        assert_eq!(engine.frame(json!({ "v": 1 }), PixelFormat::Png), Err(closed()));
    }

    /// A panic inside the app becomes `Internal` — carrying the panic's message — and the session is
    /// intact: the value set before the panics is still there, and the engine answers calls on.
    #[test]
    fn a_panic_becomes_internal_and_the_session_stays() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        engine.request("value_set", json!({ "v": 7 })).unwrap();
        for (method, expected) in [("panic_str", "boom"), ("panic_fmt", "boom 7")] {
            assert_eq!(call(&engine, method), Err(CraftError::Internal { message: expected.to_string() }), "{method}");
        }
        // A payload that is neither `&str` nor `String` answers what the error mapping makes of it.
        assert_eq!(call(&engine, "panic_value"), Err(CraftError::from_panic(Box::new(42u64))));
        // A payload whose own drop panics again is caught as well, after the guard of the call.
        assert_eq!(call(&engine, "panic_payload_drop"), Err(CraftError::Internal { message: PAYLOAD_PANICKED.to_string() }));
        // The session the panics hit is the session that keeps serving: the value set before them
        // is still there, and calls run on.
        assert_eq!(call(&engine, "value_get").unwrap(), Value::from(7));
        assert_eq!(call(&engine, "echo").unwrap(), Value::Null);
        engine.close().unwrap();
    }

    /// A panic that lands in the middle of a mutation leaves the session as the panic left it — the
    /// document is kept, not rolled back and not lost — and the engine keeps answering.
    #[test]
    fn a_panic_mid_mutation_keeps_the_document_as_it_stood() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        engine.request("value_set", json!({ "v": 3 })).unwrap();
        // This call writes and then panics: the write is part of the kept document.
        let answer = engine.request("panic_after_write", json!({ "v": 4 }));
        assert!(matches!(answer, Err(CraftError::Internal { .. })), "panic mid-write answered {answer:?}");
        assert_eq!(call(&engine, "value_get").unwrap(), Value::from(4));
        engine.request("value_set", json!({ "v": 5 })).unwrap();
        assert_eq!(call(&engine, "value_get").unwrap(), Value::from(5));
        engine.close().unwrap();
    }

    /// After `close` every call answers `Internal` naming the close, without waiting for anything;
    /// a second close answers `Ok`, and closes racing each other all answer `Ok` without joining
    /// the same thread twice.
    #[test]
    fn calls_after_close_answer_internal_and_close_is_idempotent() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        engine.close().unwrap();
        // Plan §16's own words for a call after close, written out rather than read from the
        // constant, so a reworded refusal fails here.
        assert_eq!(call(&engine, "echo"), Err(CraftError::Internal { message: "engine is closed".to_string() }));
        engine.close().unwrap();
        // Four closes racing each other: exactly one takes the thread and joins it, the rest
        // answer `Ok` once it has, and none of them panics or hangs.
        let (app, _shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let closers: Vec<_> = (0..4)
            .map(|_| {
                let engine = engine.clone();
                thread::spawn(move || engine.close())
            })
            .collect();
        for closer in closers {
            let answer = closer.join().unwrap_or_else(|_| panic!("a closer panicked"));
            assert!(answer.is_ok(), "a racing close answered {answer:?}");
        }
    }

    /// `close` waits for the call in flight: it answers only after the blocked call reached its
    /// own end, so nothing is joined mid-call.
    #[test]
    fn close_waits_for_the_call_in_flight() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let blocked = busy(&engine, &shared);
        let closer = close_begun(&engine);
        assert!(!closer.is_finished(), "close answered before the call in flight ended");
        shared.released.open();
        assert_eq!(closer.join().unwrap(), Ok(()));
        assert_eq!(blocked.join().unwrap(), Ok(Value::Null), "the call in flight was not finished before the thread stopped");
        assert_eq!(call(&engine, "echo"), Err(closed()), "the engine answered after it was closed");
    }

    /// A close racing calls and another close keeps every promise `close` makes: a call handed over
    /// before the close is answered whole, a call made once the close has begun is refused at once
    /// without waiting for the engine, and a second close returns only once the session has ended.
    #[test]
    fn a_close_racing_calls_and_a_close_answers_each_as_close_promises() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let blocked = busy(&engine, &shared);
        // Handed over before the close begins, so it is queued ahead of `Close`.
        let queued = queue_call(&engine, "echo", json!({ "n": 1 }));
        let first_close = close_begun(&engine);
        // Answered while the engine is still inside the call in flight, so it never waited for it.
        let (answer, refused) = mpsc::channel();
        let refusing = engine.clone();
        thread::spawn(move || answer.send(call(&refusing, "echo")));
        assert_eq!(recv(&refused), Err(closed()));
        assert!(!blocked.is_finished(), "a call made after close waited for the engine");
        // What the second close saw of the session as it returned.
        let (closing, seen) = (engine.clone(), shared.clone());
        let second_close = thread::spawn(move || (closing.close(), seen.dropped.load(Ordering::SeqCst)));
        shared.released.open();
        assert_eq!(second_close.join().unwrap(), (Ok(()), true), "a second close returned before the session ended");
        assert_eq!(recv(&queued), Ok(json!({ "n": 1 })), "a call handed over before the close was not answered whole");
        assert_eq!(first_close.join().unwrap(), Ok(()));
        assert_eq!(blocked.join().unwrap(), Ok(Value::Null));
    }

    /// A call that loses its race to a close is refused with `engine is closed`, never answered as
    /// though it ran. The racer holds the sender it took before the close took the inbox:
    /// - a call it queues behind the close is refused as soon as the engine stops, while the
    ///   session's drop is still held, and the sender it still holds does not keep the engine
    ///   running;
    /// - a call it sends once the engine has stopped is refused at once.
    #[test]
    fn a_call_that_loses_its_race_to_close_is_refused_never_answered() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        call(&engine, "hold_drop").unwrap();
        let racer = engine.racer().unwrap();
        let blocked = busy(&engine, &shared);
        let closer = close_begun(&engine);
        // Handed over from this thread once the close has begun and before the call in flight is
        // let go, so it is queued behind the `Close` before the engine can stop, however slowly
        // either side runs. Its answer is taken the way a public call takes its own, so what this
        // proves is the refusal the caller gets, `engine is closed`; on a thread of its own only so
        // a broken engine fails at the deadline instead of hanging the suite.
        let raced = hand_over(&racer, Message::request("value_set", json!({ "v": 1 }))).unwrap();
        let waiter = thread::spawn(move || answer_of(raced));
        shared.released.open();
        assert!(eventually(|| waiter.is_finished()), "a call queued behind the close was never refused");
        assert_eq!(waiter.join().unwrap(), Err(closed()), "a call queued behind the close was not refused with `engine is closed`");
        assert!(!shared.dropped.load(Ordering::SeqCst), "a call queued behind the close waited for the session's drop");
        shared.drop_released.open();
        assert_eq!(closer.join().unwrap(), Ok(()));
        assert_eq!(blocked.join().unwrap(), Ok(Value::Null));
        assert_eq!(call_on(&racer, "value_set", json!({ "v": 2 })), Err(closed()), "a call sent after the engine stopped");
    }

    /// Dropping a handle without `close` closes the engine: the session has ended, and its thread
    /// has been joined, by the time the drop returns.
    #[test]
    fn dropping_the_engine_ends_the_session_before_the_drop_returns() {
        let (app, shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        // A session whose drop is held, so a handle's drop that left the thread to end on its own
        // would have returned while the session's drop is still running.
        call(&engine, "hold_drop").unwrap();
        let dropper = thread::spawn(move || drop(engine));
        assert!(shared.drop_entered.wait(), "the session's drop never began");
        assert!(!dropper.is_finished(), "the engine's drop returned while its session was still running");
        shared.drop_released.open();
        dropper.join().unwrap();
        assert!(shared.dropped.load(Ordering::SeqCst));
    }

    /// A panic in the session's own drop is caught like a panic in a call: the close answers it as
    /// `Internal` with the panic's message, and the thread has stopped in good order.
    #[test]
    fn a_panic_in_the_sessions_drop_is_the_closes_answer() {
        let (app, shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        call(&engine, "drop_panicking").unwrap();
        assert_eq!(engine.close(), Err(CraftError::Internal { message: "boom on drop".to_string() }));
        assert!(shared.dropped.load(Ordering::SeqCst));
        assert_eq!(engine.close(), Ok(()));
    }

    /// A call from inside one of the engine's own calls is refused at once rather than waiting for
    /// itself, and a close from there stops the engine after that call without joining its own
    /// thread; a close from outside then joins it.
    #[test]
    fn calls_from_inside_the_engines_own_calls_are_refused_and_never_wait() {
        let (engine, shared) = self_aware_engine();
        let reentered = from_another_thread(&engine, |engine| engine.request("reenter", Value::Null));
        assert_eq!(reentered, Ok(Err(CraftError::Internal { message: REENTERED.to_string() })));
        assert_eq!(call(&engine, "close_self"), Ok(Value::Null));
        assert_eq!(call(&engine, "echo"), Err(closed()));
        assert_eq!(engine.close(), Ok(()));
        assert!(shared.dropped.load(Ordering::SeqCst));
    }

    /// A `request_with_progress` from inside one of the engine's own calls is refused at once too:
    /// the export would wait behind the call that made it, which waits behind it.
    #[test]
    fn an_export_call_from_inside_the_engines_own_calls_is_refused() {
        let (engine, _) = self_aware_engine();
        let reentered = from_another_thread(&engine, |engine| engine.request("reenter_progress", Value::Null));
        assert_eq!(reentered, Ok(Err(CraftError::Internal { message: REENTERED.to_string() })));
        engine.close().unwrap();
    }

    /// A caller that stopped waiting costs the engine nothing: its call still runs, in order, and
    /// the engine answers the next call as usual.
    #[test]
    fn a_call_whose_caller_stopped_waiting_still_runs_and_the_engine_answers_on() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        let (answer, waiting) = mpsc::channel();
        drop(waiting);
        let inbox = engine.racer().unwrap();
        inbox.send(Message::Request { method: "value_set".to_string(), params: json!({ "v": 9 }), answer }).unwrap();
        assert_eq!(call(&engine, "value_get"), Ok(Value::from(9)));
    }

    /// Two engines are two sessions on two threads: each thread carries the engine name, no id is
    /// shared, and one engine's calls cannot see the other's session (plan §16, edge cases).
    #[test]
    fn two_engines_have_their_own_session_and_thread() {
        let (one, _shared) = FakeApp::new();
        let (two, _shared) = FakeApp::new();
        let first = Engine::start(one).unwrap();
        let second = Engine::start(two).unwrap();
        first.request("value_set", json!({ "v": 1 })).unwrap();
        second.request("value_set", json!({ "v": 2 })).unwrap();
        assert_eq!(call(&first, "value_get").unwrap(), Value::from(1));
        assert_eq!(call(&second, "value_get").unwrap(), Value::from(2));
        assert_eq!(call(&first, "thread").unwrap(), Value::from(THREAD_NAME));
        assert_eq!(call(&second, "thread").unwrap(), Value::from(THREAD_NAME));
        assert_ne!(call(&first, "thread_id").unwrap(), call(&second, "thread_id").unwrap());
        first.close().unwrap();
        second.close().unwrap();
    }
}
