//! The engine actor: one thread per engine, calls answered in the order they were made, and a
//! panic inside the app answered as `Internal` with the session kept (plan §16).

use crate::Result;
use crate::app::CraftApp;
use crate::error::CraftError;
use serde_json::Value;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};

/// The name every engine thread carries, so a thread listing of the host app shows the engine (plan §16).
const THREAD_NAME: &str = "craftbridge-engine";

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
/// waiting caller by mistake.
enum Message {
    Request { method: String, params: Value, answer: Sender<Result<Value>> },
    Close,
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
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_string())
            .spawn(move || run(Box::new(app), incoming))
            .map_err(|error| CraftError::internal(format!("cannot start the engine thread: {error}")))?;
        Ok(Engine { id: thread.thread().id(), inbox: Mutex::new(Some(inbox)), thread: Mutex::new(Some(thread)) })
    }

    /// Make one call and wait for the engine's answer.
    ///
    /// This blocks the calling thread; the FFI layer hands the wait to its async runtime so an
    /// app's main thread never blocks (plan §14, §20). The call runs on the engine thread in the
    /// order the calls were made, and a panic inside the app is answered as [`CraftError::Internal`]
    /// — this call never returns a panic and never stops the session. A call made from inside one of
    /// this engine's own calls is refused with `Internal` at once rather than waiting for itself.
    pub fn request(&self, method: &str, params: Value) -> Result<Value> {
        if self.on_engine_thread() {
            return Err(CraftError::internal(REENTERED));
        }
        let Some(inbox) = lock(&self.inbox).clone() else { return Err(closed()) };
        call_on(&inbox, method, params)
    }

    /// Stop the engine: stop the thread after the calls already accepted, end the session, and join
    /// the thread. Answers `Internal` when the session's own drop panicked.
    ///
    /// Idempotent: the close that joins the thread answers with how the session ended, and every
    /// other close `Ok`, each only once the thread has stopped. The exception is a close from inside
    /// one of the engine's own calls, which cannot wait for that call to end: it refuses new calls
    /// and answers `Ok` at once, and the thread stops when the call ends. A call still running is
    /// finished before the thread stops, so close waits for it. A call handed to the engine before
    /// the close is answered first, in order; one that races the close is either answered whole or
    /// refused with `engine is closed`, never left waiting on a dead engine; and once any close has
    /// returned, every call is refused.
    pub fn close(&self) -> Result<()> {
        // The inbox goes first, so new calls are refused from here on. Close queues behind the calls
        // already accepted, so they are all answered first, and dropping this sender after it
        // disconnects the inbox for good.
        let inbox = lock(&self.inbox).take();
        if let Some(inbox) = inbox {
            let _ = inbox.send(Message::Close);
        }
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
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hand one call to the engine through `inbox` and wait for its answer.
///
/// A call the engine stopped before running lost its race to a close: it is refused with `engine is
/// closed` and never answered as though it ran, so a write that never happened cannot read as done.
/// The send fails when the engine had already stopped, and the wait fails when a close ran ahead of
/// the call in the queue and the engine dropped it unrun.
fn call_on(inbox: &Sender<Message>, method: &str, params: Value) -> Result<Value> {
    let (answer, waiting) = mpsc::channel();
    inbox.send(Message::Request { method: method.to_string(), params, answer }).map_err(|_| closed())?;
    waiting.recv().map_err(|_| closed())?
}

/// The engine thread's loop: take one call, run it on the app, answer it, and take the next; then
/// end the session. Answers with how the session's drop went.
///
/// One call at a time, in inbox order, because the session is reached only here (plan §16, L1). The
/// loop ends at `Close` or when every handle has dropped the inbox.
fn run(mut app: Box<dyn CraftApp>, incoming: Receiver<Message>) -> Result<()> {
    while let Ok(message) = incoming.recv() {
        match message {
            Message::Close => break,
            Message::Request { method, params, answer } => {
                let answered = guarded(|| app.request(&method, params)).flatten();
                // The caller may already have gone (a UI that stopped waiting); losing the answer is
                // not an error for the engine — the call still ran, in order, and the next call is
                // answered normally.
                let _ = answer.send(answered);
            }
        }
    }
    // Calls queued behind the close are refused now, not after the session's drop has run.
    drop(incoming);
    // The session's drop is app code too, so it runs behind the same guard as a call: a panic there
    // is the close's answer rather than a thread that died outside its guard.
    guarded(move || drop(app))
}

/// Run `work` behind the never-crash boundary (AGENTS §3): a panic that unwinds out of it becomes
/// `Internal` carrying the panic's message, and the engine thread goes on.
///
/// `AssertUnwindSafe` is deliberate: a panic may leave the session half-changed, and plan §14 and §16
/// keep the session as the panic left it — the document is kept, not rolled back — as EffectCraft's
/// own guard does. Only an unwinding panic can be caught: one raised while another unwinds, by a
/// destructor in the app's own cleanup, is an abort Rust itself makes, before any guard runs.
fn guarded<T>(work: impl FnOnce() -> T) -> Result<T> {
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
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock, Weak};
    use std::time::{Duration, Instant};

    /// The fake app the runtime is proven on: it keeps a session (a value, plus a log of the calls
    /// it ran in the order it ran them, which is the engine's ordering made visible), and it can be
    /// told to panic, to block until the test releases it, to call its own engine, or to drop
    /// slowly or with a panic.
    struct FakeApp {
        log: Vec<String>,
        value: i64,
        shared: Arc<Shared>,
        drop_slowly: bool,
        drop_panicking: bool,
    }

    /// What the test and its fake app share. `entered` says the blocking call has reached the app,
    /// `released` lets it end, `dropped` says the session has ended, and `engine` is the app's own
    /// engine, held weakly so the session does not keep its own engine alive. Waits are polled with
    /// a deadline, so a broken engine fails the test instead of hanging the suite.
    struct Shared {
        entered: AtomicBool,
        released: AtomicBool,
        dropped: AtomicBool,
        engine: OnceLock<Weak<Engine>>,
    }

    impl Shared {
        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
        }
        /// Whether `entered` came true within the deadline.
        fn wait_entered(&self) -> bool {
            self.wait_for(&self.entered)
        }
        fn wait_for(&self, flag: &AtomicBool) -> bool {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !flag.load(Ordering::SeqCst) {
                if Instant::now() > deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(1));
            }
            true
        }
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
                entered: AtomicBool::new(false),
                released: AtomicBool::new(false),
                dropped: AtomicBool::new(false),
                engine: OnceLock::new(),
            });
            (FakeApp { log: Vec::new(), value: 0, shared: shared.clone(), drop_slowly: false, drop_panicking: false }, shared)
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
                    self.shared.entered.store(true, Ordering::SeqCst);
                    if self.shared.wait_for(&self.shared.released) { Ok(Value::Null) } else { Err(CraftError::internal("never released")) }
                }
                "thread" => Ok(Value::from(thread::current().name().unwrap_or("<unnamed>").to_string())),
                "thread_id" => Ok(Value::from(format!("{:?}", thread::current().id()))),
                "reenter" => self.own_engine()?.request("echo", Value::Null),
                "close_self" => self.own_engine()?.close().map(|()| Value::Null),
                "drop_slowly" => {
                    self.drop_slowly = true;
                    Ok(Value::Null)
                }
                "drop_panicking" => {
                    self.drop_panicking = true;
                    Ok(Value::Null)
                }
                other => Err(CraftError::internal(format!("unknown method {other}"))),
            }
        }
    }

    impl Drop for FakeApp {
        fn drop(&mut self) {
            if self.drop_slowly {
                thread::sleep(Duration::from_millis(100));
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

    /// Run `work` against an engine that is busy: a `wait` call has reached the app, and the test
    /// lets it end only 200 ms after `work` begins. Anything that answers without waiting for the
    /// call in flight — a queued call, or a close that skipped its join — answers inside `work`,
    /// long before the release, and the check after it says so.
    fn with_engine_busy(engine: &Engine, shared: &Shared, work: impl FnOnce()) {
        thread::scope(|scope| {
            let blocked = scope.spawn(|| call(engine, "wait"));
            assert!(shared.wait_entered(), "the blocked call never reached the app");
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(200));
                shared.release();
            });
            work();
            assert!(shared.released.load(Ordering::SeqCst), "the engine answered while it was still inside the blocked call");
            blocked.join().unwrap().unwrap();
        });
    }

    /// Start `work` on a thread of its own and return once it has had time to hand its call to the
    /// engine: the thread says when `work` is about to begin, and the 25 ms after that cover the
    /// few steps from there to the handover, so calls started one after another reach the engine
    /// in the order they were started.
    fn spawn_settled<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> JoinHandle<T> {
        let (starting, started) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = starting.send(());
            work()
        });
        let _ = started.recv_timeout(Duration::from_secs(10));
        thread::sleep(Duration::from_millis(25));
        handle
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
        let mut queued = Vec::new();
        with_engine_busy(&engine, &shared, || {
            for n in 0..4 {
                let engine = engine.clone();
                queued.push(spawn_settled(move || engine.request("echo", json!({ "n": n }))));
            }
            // The last call is made here, so the work ends only once the engine has answered it.
            assert_eq!(engine.request("echo", json!({ "n": 4 })).unwrap(), json!({ "n": 4 }));
        });
        for (n, caller) in queued.into_iter().enumerate() {
            assert_eq!(caller.join().unwrap().unwrap(), json!({ "n": n }));
        }
        let echoes = (0..5).map(|n| format!("echo {}", json!({ "n": n })));
        let expected: Vec<String> = std::iter::once("wait null".to_string()).chain(echoes).chain(std::iter::once("log null".to_string())).collect();
        assert_eq!(call(&engine, "log").unwrap(), json!(expected));
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
        let engine = Engine::start(app).unwrap();
        with_engine_busy(&engine, &shared, || {
            let began = Instant::now();
            let closed = engine.close();
            assert!(began.elapsed() >= Duration::from_millis(150), "close answered before the call in flight ended");
            assert!(closed.is_ok(), "close answered {closed:?}");
        });
        assert_eq!(call(&engine, "echo"), Err(closed()), "the engine answered after it was closed");
    }

    /// A close racing calls and another close keeps every promise `close` makes: a call handed over
    /// before the close is answered whole, a call made once the close has begun is refused at once
    /// without waiting for the engine, and a second close returns only once the session has ended.
    #[test]
    fn a_close_racing_calls_and_a_close_answers_each_as_close_promises() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        let mut racers = None;
        with_engine_busy(&engine, &shared, || {
            let (queuing, closing) = (engine.clone(), engine.clone());
            let queued = spawn_settled(move || queuing.request("echo", json!({ "n": 1 })));
            let first_close = spawn_settled(move || closing.close());
            assert_eq!(call(&engine, "echo"), Err(closed()));
            assert!(!shared.released.load(Ordering::SeqCst), "a call made after close waited for the engine");
            assert_eq!(engine.close(), Ok(()));
            assert!(shared.dropped.load(Ordering::SeqCst), "a second close returned before the session ended");
            racers = Some((queued, first_close));
        });
        let (queued, first_close) = racers.unwrap();
        assert_eq!(queued.join().unwrap(), Ok(json!({ "n": 1 })), "a call handed over before the close was not answered whole");
        assert_eq!(first_close.join().unwrap(), Ok(()));
    }

    /// A call that loses its race to a close is refused with `engine is closed`, never answered as
    /// though it ran. The racer holds the sender it took before the close took the inbox:
    /// - a call it queues behind the close is refused as soon as the engine stops, before the
    ///   session's slow drop has ended, and the sender it still holds does not keep the engine
    ///   running;
    /// - a call it sends once the engine has stopped is refused at once.
    #[test]
    fn a_call_that_loses_its_race_to_close_is_refused_never_answered() {
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        call(&engine, "drop_slowly").unwrap();
        let racer = lock(&engine.inbox).clone().unwrap();
        let mut closing = None;
        with_engine_busy(&engine, &shared, || {
            let engine = engine.clone();
            closing = Some(spawn_settled(move || engine.close()));
            assert_eq!(call_on(&racer, "value_set", json!({ "v": 1 })), Err(closed()), "a call queued behind the close");
            assert!(!shared.dropped.load(Ordering::SeqCst), "a call queued behind the close waited for the session's drop");
        });
        assert_eq!(closing.unwrap().join().unwrap(), Ok(()));
        assert_eq!(call_on(&racer, "value_set", json!({ "v": 2 })), Err(closed()), "a call sent after the engine stopped");
    }

    /// Dropping a handle without `close` closes the engine: the session has ended, and its thread
    /// has been joined, by the time the drop returns.
    #[test]
    fn dropping_the_engine_ends_the_session_before_the_drop_returns() {
        let (app, shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        // A session whose drop takes a while, so a drop that left the thread to end on its own
        // would return long before the session ended.
        call(&engine, "drop_slowly").unwrap();
        drop(engine);
        assert!(shared.dropped.load(Ordering::SeqCst), "the engine's drop returned while its session was still running");
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
        let (app, shared) = FakeApp::new();
        let engine = Arc::new(Engine::start(app).unwrap());
        assert!(shared.engine.set(Arc::downgrade(&engine)).is_ok());
        // From another thread with a deadline, so an engine waiting for itself fails the test
        // instead of hanging the suite.
        let (answering, answered) = mpsc::channel();
        let reentering = engine.clone();
        thread::spawn(move || answering.send(reentering.request("reenter", Value::Null)));
        let reentered = answered.recv_timeout(Duration::from_secs(10));
        assert_eq!(reentered, Ok(Err(CraftError::Internal { message: REENTERED.to_string() })));
        assert_eq!(call(&engine, "close_self"), Ok(Value::Null));
        assert_eq!(call(&engine, "echo"), Err(closed()));
        assert_eq!(engine.close(), Ok(()));
        assert!(shared.dropped.load(Ordering::SeqCst));
    }

    /// A caller that stopped waiting costs the engine nothing: its call still runs, in order, and
    /// the engine answers the next call as usual.
    #[test]
    fn a_call_whose_caller_stopped_waiting_still_runs_and_the_engine_answers_on() {
        let (app, _shared) = FakeApp::new();
        let engine = Engine::start(app).unwrap();
        let (answer, waiting) = mpsc::channel();
        drop(waiting);
        let inbox = lock(&engine.inbox).clone().unwrap();
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
