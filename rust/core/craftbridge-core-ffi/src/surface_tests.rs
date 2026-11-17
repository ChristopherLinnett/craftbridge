//! The exported calls, driven through the surface Swift and Kotlin see: each throws for a closed
//! engine and a caught panic, and never answers with either as a reply.

use crate::crossing::Limits;
use crate::{CancelToken, CraftError, Engine, Folders, FrameBytes, GpuChoice, PixelFormat, ProgressListener, Recovery, RecoveryNote};
use craftbridge_core::CraftApp;
use serde_json::{Value, json};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// The session behind the engine: it answers what a call names, and blows up when told to. Told to
/// with `drop_panicking`, it panics as the engine ends it; `count` answers, from every kind of call,
/// how many `count` calls it ran before this one, which is the order the engine ran them in.
struct Probe {
    panics_on_drop: bool,
    counted: u64,
}

impl Drop for Probe {
    fn drop(&mut self) {
        if self.panics_on_drop {
            panic!("the session blew up on drop");
        }
    }
}

impl CraftApp for Probe {
    fn request(&mut self, method: &str, params: Value) -> craftbridge_core::Result<Value> {
        match method {
            "echo" => Ok(params),
            "count" => {
                self.counted += 1;
                Ok(Value::from(self.counted - 1))
            }
            "drop_panicking" => {
                self.panics_on_drop = true;
                Ok(Value::Null)
            }
            "panic" => panic!("request blew up"),
            _ => Err(CraftError::not_found(format!("no method {method}"))),
        }
    }

    fn request_with_progress(
        &mut self,
        method: &str,
        _params: Value,
        on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool,
        _list_outputs: &mut dyn FnMut(&[PathBuf]) -> craftbridge_core::Result<()>,
    ) -> craftbridge_core::Result<Value> {
        if method == "panic" {
            panic!("export blew up");
        }
        if method == "count" {
            return self.request("count", Value::Null);
        }
        for done in 1..=3_u32 {
            if !on_progress(u64::from(done), 3, Some(f64::from(3 - done))) {
                return Err(CraftError::Cancelled);
            }
        }
        Ok(json!({ "rendered": true }))
    }

    fn frame(&mut self, params: Value, format: PixelFormat) -> craftbridge_core::Result<FrameBytes> {
        if params == json!("panic") {
            panic!("frame blew up");
        }
        if params == json!("count") {
            let ran = self.request("count", Value::Null)?;
            return FrameBytes::new(1, 1, format, vec![u8::try_from(ran.as_u64().unwrap_or(u64::MAX)).unwrap_or(u8::MAX), 0, 0, 255]);
        }
        FrameBytes::new(1, 1, format, vec![10, 20, 30, 255])
    }

    fn mcp(&mut self, line: &str) -> Option<String> {
        match line {
            "panic" => panic!("mcp blew up"),
            "notification" => None,
            "count" => self.request("count", Value::Null).ok().map(|ran| ran.to_string()),
            _ => Some(format!("reply to {line}")),
        }
    }

    /// The probe renders on no GPU: it takes the CPU note and refuses a device.
    fn attach_gpu(&mut self, gpu: &craftbridge_core::GpuDevice) -> craftbridge_core::Result<()> {
        match gpu.parts() {
            Some(_) => Err(CraftError::gpu("the probe renders on no GPU")),
            None => Ok(()),
        }
    }
}

fn engine() -> Arc<Engine> {
    started(GpuChoice::CpuOnly)
}

/// An engine on the probe, started with the GPU `gpu` asks for.
fn started(gpu: GpuChoice) -> Arc<Engine> {
    Engine::from_core(craftbridge_core::Engine::start(Probe { panics_on_drop: false, counted: 0 }, gpu).expect("the engine starts"))
}

fn closed_engine() -> Arc<Engine> {
    let engine = engine();
    pollster::block_on(engine.shut_down()).expect("a first shut down succeeds");
    engine
}

/// A listener that keeps what it hears, and one that panics on the first report it hears.
#[derive(Default)]
struct Heard(Mutex<Vec<(u64, u64, Option<f64>)>>);

impl ProgressListener for Heard {
    fn progress(&self, done: u64, total: u64, seconds_left: Option<f64>) {
        self.0.lock().expect("the list is not poisoned").push((done, total, seconds_left));
    }
}

struct Throws;

impl ProgressListener for Throws {
    fn progress(&self, _: u64, _: u64, _: Option<f64>) {
        panic!("the listener threw");
    }
}

const CLOSED: &str = "engine is closed";

/// Poll `call` once, as a foreign caller does when it makes the call: its answer if the engine was
/// already done, or the call, still waiting.
fn first_poll<F: Future + Unpin>(mut call: F) -> Result<F::Output, F> {
    match Pin::new(&mut call).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(answer) => Ok(answer),
        Poll::Pending => Err(call),
    }
}

/// A call of each exported kind that runs `count`, made on `engine` and first polled as the foreign
/// caller does, answering the number of `count` calls the session ran before it.
fn counting(engine: &Arc<Engine>, kind: usize) -> Pin<Box<dyn Future<Output = Result<u64, CraftError>>>> {
    let engine = Arc::clone(engine);
    match kind % 4 {
        0 => Box::pin(async move { engine.request("count".into(), "null".into()).await.map(|text| text.parse().expect("a number")) }),
        1 => Box::pin(async move { engine.mcp("count".into()).await.map(|line| line.expect("a reply").parse().expect("a number")) }),
        2 => Box::pin(async move { engine.frame(r#""count""#.into(), PixelFormat::Rgba8Straight).await.map(|frame| u64::from(frame.data[0])) }),
        _ => Box::pin(async move {
            engine
                .request_with_progress("count".into(), "null".into(), Arc::new(Heard::default()), CancelToken::new())
                .await
                .map(|text| text.parse().expect("a number"))
        }),
    }
}

#[test]
fn calls_run_in_the_order_they_were_made_however_their_answers_are_awaited() {
    let engine = engine();
    // Each call, of every kind, is made and so first polled in turn, then awaited last first: a call
    // that reached the engine only once awaited, or on a thread of its own, would run out of turn.
    let made: Vec<_> = (0..16).map(|n| first_poll(counting(&engine, n))).collect();
    let answers: Vec<_> = made.into_iter().rev().map(|made| made.unwrap_or_else(pollster::block_on)).collect();
    let expected: Vec<_> = (0..16).rev().map(Ok).collect();
    assert_eq!(answers, expected);
}

#[test]
fn a_resume_with_no_device_answers_the_cpu_note_and_one_the_app_cannot_take_throws() {
    let engine = engine();
    assert_eq!(pollster::block_on(engine.suspend_gpu()), Ok(()));
    assert_eq!(pollster::block_on(engine.resume_gpu()), Ok(Some("the app asked for CPU-only rendering".to_string())), "the CPU note crosses as the answer");
    let on_a_device = started(GpuChoice::Preferred);
    assert_eq!(pollster::block_on(on_a_device.suspend_gpu()), Ok(()));
    assert_eq!(
        pollster::block_on(on_a_device.resume_gpu()),
        Err(CraftError::gpu("the probe renders on no GPU")),
        "the app's refusal crosses as the thrown error"
    );
    assert_eq!(pollster::block_on(on_a_device.request("echo".into(), "1".into())), Ok("1".to_string()), "the engine answers on");
}

#[test]
fn a_call_made_after_shut_down_is_refused_however_late_the_engine_thread_stops() {
    // Repeated, since a shut down that reached the engine on a thread of its own lets a call slip in
    // ahead of it only when the two race.
    for _ in 0..25 {
        let engine = engine();
        let stopping = first_poll(Box::pin(engine.shut_down()));
        let later = first_poll(Box::pin(engine.request("echo".into(), "1".into())));
        assert_eq!(later.map_err(|_| "still waiting"), Ok(Err(CraftError::internal(CLOSED))));
        let stopped = stopping.unwrap_or_else(pollster::block_on);
        assert_eq!(stopped, Ok(()));
    }
}

#[test]
fn a_request_answers_its_result_as_json_text() {
    let answer = pollster::block_on(engine().request("echo".into(), r#"{"a": [1, 2]}"#.into()));
    assert_eq!(answer.map(|text| serde_json::from_str::<Value>(&text).expect("JSON")), Ok(json!({ "a": [1, 2] })));
}

#[test]
fn a_request_with_bad_json_throws_invalid_params_and_the_engine_answers_on() {
    let engine = engine();
    let bad = pollster::block_on(engine.request("echo".into(), "{ not json".into()));
    assert!(matches!(bad, Err(CraftError::InvalidParams { .. })), "{bad:?}");
    assert_eq!(pollster::block_on(engine.request("echo".into(), "1".into())), Ok("1".to_string()));
}

#[test]
fn a_request_the_app_fails_throws_the_apps_error() {
    let answer = pollster::block_on(engine().request("missing".into(), "{}".into()));
    assert_eq!(answer, Err(CraftError::not_found("no method missing")));
}

#[test]
fn a_panic_in_request_throws_internal_and_keeps_the_session() {
    let engine = engine();
    assert_eq!(pollster::block_on(engine.request("panic".into(), "{}".into())), Err(CraftError::internal("request blew up")));
    assert_eq!(pollster::block_on(engine.request("echo".into(), "true".into())), Ok("true".to_string()));
}

#[test]
fn a_request_with_progress_reports_to_the_listener_and_answers_its_result() {
    let heard = Arc::new(Heard::default());
    let answer = pollster::block_on(engine().request_with_progress("export".into(), "{}".into(), heard.clone(), CancelToken::new()));
    assert_eq!(answer.map(|text| serde_json::from_str::<Value>(&text).expect("JSON")), Ok(json!({ "rendered": true })));
    let heard = heard.0.lock().expect("the list is not poisoned").clone();
    assert_eq!(heard, [(1, 3, Some(2.0)), (2, 3, Some(1.0)), (3, 3, Some(0.0))]);
}

#[test]
fn a_cancelled_token_throws_cancelled() {
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(cancel.is_cancelled());
    let answer = pollster::block_on(engine().request_with_progress("export".into(), "{}".into(), Arc::new(Heard::default()), cancel));
    assert_eq!(answer, Err(CraftError::Cancelled));
}

#[test]
fn a_listener_that_throws_does_not_stop_the_export() {
    let answer = pollster::block_on(engine().request_with_progress("export".into(), "{}".into(), Arc::new(Throws), CancelToken::new()));
    assert!(answer.is_ok(), "{answer:?}");
}

/// A listener that calls the engine it listens to with `call`, as an app's progress handler may,
/// through the exported surface, and keeps what each call answered.
struct CallsBack<T> {
    engine: Arc<Engine>,
    call: fn(&Engine) -> T,
    answered: Mutex<Vec<T>>,
}

impl<T: Send> ProgressListener for CallsBack<T> {
    fn progress(&self, _: u64, _: u64, _: Option<f64>) {
        let answer = (self.call)(&self.engine);
        self.answered.lock().expect("the list is not poisoned").push(answer);
    }
}

/// An export of `engine` heard by a listener that makes `call` on it, run on a thread of its own with
/// a bound on its wait: the listener runs on the engine thread, inside the export, so a call that
/// waits for the engine waits for the export, which waits for the listener, and the test fails
/// rather than hangs. Answers the export's answer and what each call the listener made answered.
fn export_heard_by_a_listener_that_calls_back<T: Send + Clone + 'static>(engine: &Arc<Engine>, call: fn(&Engine) -> T) -> (Result<String, CraftError>, Vec<T>) {
    let listener = Arc::new(CallsBack { engine: Arc::clone(engine), call, answered: Mutex::new(Vec::new()) });
    let (answer, answered) = std::sync::mpsc::channel();
    let exporting = Arc::clone(engine);
    let hearing = Arc::clone(&listener);
    std::thread::spawn(move || answer.send(pollster::block_on(exporting.request_with_progress("export".into(), "{}".into(), hearing, CancelToken::new()))));
    let export = answered.recv_timeout(std::time::Duration::from_secs(10)).expect("the export and its listener waited for each other");
    let heard = listener.answered.lock().expect("the list is not poisoned").clone();
    (export, heard)
}

#[test]
fn a_call_from_the_listener_is_refused_at_once_rather_than_waiting_behind_its_export() {
    // A call it makes is refused on the engine thread, where one queued behind the export would wait for it.
    let engine = engine();
    let (export, heard) = export_heard_by_a_listener_that_calls_back(&engine, |engine| pollster::block_on(engine.request("echo".into(), "1".into())));
    assert_eq!(export.map(|text| serde_json::from_str::<Value>(&text).expect("JSON")), Ok(json!({ "rendered": true })));
    let refused = Err(CraftError::internal("engine called from inside one of its own calls"));
    assert_eq!(heard, [refused.clone(), refused.clone(), refused]);
}

#[test]
fn a_shut_down_from_the_listener_answers_at_once_rather_than_waiting_for_its_own_export() {
    // Joining the engine thread from a thread of its own would wait for the export, which waits for the listener.
    let engine = engine();
    let (export, heard) = export_heard_by_a_listener_that_calls_back(&engine, |engine| pollster::block_on(engine.shut_down()));
    assert_eq!(export, Err(CraftError::Cancelled), "the engine stopped the export it was told to end");
    assert!(!heard.is_empty() && heard.iter().all(|answer| answer == &Ok(())), "{heard:?}");
    assert_eq!(pollster::block_on(engine.shut_down()), Ok(()));
}

#[test]
fn a_panic_in_an_export_throws_internal() {
    let answer = pollster::block_on(engine().request_with_progress("panic".into(), "{}".into(), Arc::new(Heard::default()), CancelToken::new()));
    assert_eq!(answer, Err(CraftError::internal("export blew up")));
}

#[test]
fn a_frame_answers_its_bytes_in_the_format_asked() {
    let frame = pollster::block_on(engine().frame("{}".into(), PixelFormat::Rgba8Straight)).expect("a frame");
    assert_eq!(frame, FrameBytes { width: 1, height: 1, format: PixelFormat::Rgba8Straight, data: vec![10, 20, 30, 255] });
}

#[test]
fn an_engine_refuses_values_beyond_what_the_boundary_carries() {
    let limits = engine().limits();
    assert_eq!((limits.frame, limits.answer, limits.mcp_line), (crate::MAX_FRAME_BYTES, crate::MAX_ANSWER_BYTES, crate::MAX_MCP_LINE_BYTES));
}

#[test]
fn a_frame_beyond_the_limit_throws_invalid_params_through_the_exported_call() {
    let engine = engine().with_limits(Limits { frame: 3, ..Limits::BOUNDARY });
    let refused = pollster::block_on(engine.frame("{}".into(), PixelFormat::Rgba8Straight));
    assert_eq!(
        refused,
        Err(CraftError::invalid_params("the frame is 4 bytes, more than the 3 the native boundary carries: ask for a smaller one (maxSide or scale)"))
    );
    let engine = engine.with_limits(Limits { frame: 4, ..Limits::BOUNDARY });
    assert!(pollster::block_on(engine.frame("{}".into(), PixelFormat::Rgba8Straight)).is_ok());
}

#[test]
fn an_answer_beyond_the_limit_throws_invalid_params_from_request_and_from_request_with_progress() {
    // `"abc"` answers as its JSON text, five bytes; an export's `{"rendered":true}` is seventeen.
    let engine = engine().with_limits(Limits { answer: 4, ..Limits::BOUNDARY });
    let refused = pollster::block_on(engine.request("echo".into(), r#""abc""#.into()));
    assert_eq!(refused, Err(CraftError::invalid_params("the answer is 5 bytes, more than the 4 the native boundary carries: ask for less of it")));
    let export = pollster::block_on(engine.request_with_progress("export".into(), "{}".into(), Arc::new(Heard::default()), CancelToken::new()));
    assert_eq!(export, Err(CraftError::invalid_params("the answer is 17 bytes, more than the 4 the native boundary carries: ask for less of it")));
    assert_eq!(pollster::block_on(engine.request("echo".into(), "1".into())), Ok("1".to_string()), "an answer within the limit still crosses");
    let engine = engine.with_limits(Limits { answer: 5, ..Limits::BOUNDARY });
    assert_eq!(pollster::block_on(engine.request("echo".into(), r#""abc""#.into())), Ok(r#""abc""#.to_string()));
    // The limit counts the bytes that cross, not the characters: `"é"` is three characters, four bytes.
    let engine = engine.with_limits(Limits { answer: 3, ..Limits::BOUNDARY });
    assert_eq!(
        pollster::block_on(engine.request("echo".into(), r#""é""#.into())),
        Err(CraftError::invalid_params("the answer is 4 bytes, more than the 3 the native boundary carries: ask for less of it"))
    );
}

#[test]
fn an_mcp_reply_beyond_the_limit_throws_invalid_params_and_a_notification_still_answers_none() {
    // The reply to `ping` is `reply to ping`, thirteen bytes.
    let engine = engine().with_limits(Limits { mcp_line: 12, ..Limits::BOUNDARY });
    assert_eq!(
        pollster::block_on(engine.mcp("ping".into())),
        Err(CraftError::invalid_params("the MCP reply line is 13 bytes, more than the 12 the native boundary carries: ask for less of it"))
    );
    assert_eq!(pollster::block_on(engine.mcp("notification".into())), Ok(None));
    let engine = engine.with_limits(Limits { mcp_line: 13, ..Limits::BOUNDARY });
    assert_eq!(pollster::block_on(engine.mcp("ping".into())), Ok(Some("reply to ping".to_string())));
    // The limit counts the bytes that cross, not the characters: `reply to é` is ten characters, eleven bytes.
    let engine = engine.with_limits(Limits { mcp_line: 10, ..Limits::BOUNDARY });
    assert_eq!(
        pollster::block_on(engine.mcp("é".into())),
        Err(CraftError::invalid_params("the MCP reply line is 11 bytes, more than the 10 the native boundary carries: ask for less of it"))
    );
}

#[test]
fn a_panic_in_frame_throws_internal() {
    let engine = engine();
    assert_eq!(pollster::block_on(engine.frame(r#""panic""#.into(), PixelFormat::Png)), Err(CraftError::internal("frame blew up")));
    assert!(pollster::block_on(engine.frame("{}".into(), PixelFormat::Png)).is_ok());
}

#[test]
fn an_mcp_line_answers_its_reply_or_none() {
    let engine = engine();
    assert_eq!(pollster::block_on(engine.mcp("ping".into())), Ok(Some("reply to ping".to_string())));
    assert_eq!(pollster::block_on(engine.mcp("notification".into())), Ok(None));
}

#[test]
fn a_panic_in_mcp_throws_internal_and_is_no_reply_line() {
    let engine = engine();
    assert_eq!(pollster::block_on(engine.mcp("panic".into())), Err(CraftError::internal("mcp blew up")));
    assert_eq!(pollster::block_on(engine.mcp("ping".into())), Ok(Some("reply to ping".to_string())));
}

#[test]
fn every_call_on_a_closed_engine_throws_internal_and_none_answers_a_reply() {
    let engine = closed_engine();
    let closed = CraftError::internal(CLOSED);
    assert_eq!(pollster::block_on(engine.request("echo".into(), "{}".into())), Err(closed.clone()));
    let progress = pollster::block_on(engine.request_with_progress("export".into(), "{}".into(), Arc::new(Heard::default()), CancelToken::new()));
    assert_eq!(progress, Err(closed.clone()));
    assert_eq!(pollster::block_on(engine.frame("{}".into(), PixelFormat::Png)), Err(closed.clone()));
    assert_eq!(pollster::block_on(engine.mcp("ping".into())), Err(closed.clone()));
    assert_eq!(pollster::block_on(engine.suspend_gpu()), Err(closed.clone()));
    assert_eq!(pollster::block_on(engine.resume_gpu()), Err(closed));
    assert_eq!(pollster::block_on(engine.shut_down()), Ok(()), "shutting down twice is no error");
}

#[test]
fn folders_become_the_cores_paths() {
    let folders = Folders {
        container: "/app".into(),
        bundle: Some("/bundle".into()),
        config: "/app/config".into(),
        cache: "/app/cache".into(),
        models: "/app/models".into(),
        plugins: None,
        fonts: vec!["/fonts/a".into(), "/fonts/b".into()],
    };
    let core = craftbridge_core::folders::Folders::from(folders);
    assert_eq!(core.container, PathBuf::from("/app"));
    assert_eq!(core.bundle, Some(PathBuf::from("/bundle")));
    assert_eq!((core.config, core.cache, core.models), (PathBuf::from("/app/config"), PathBuf::from("/app/cache"), PathBuf::from("/app/models")));
    assert_eq!(core.plugins, None);
    assert_eq!(core.fonts, [PathBuf::from("/fonts/a"), PathBuf::from("/fonts/b")]);
}

#[test]
fn a_shut_down_whose_session_panics_ending_throws_internal() {
    let engine = engine();
    pollster::block_on(engine.request("drop_panicking".into(), "{}".into())).expect("the session is told to");
    assert_eq!(pollster::block_on(engine.shut_down()), Err(CraftError::internal("the session blew up on drop")));
    assert_eq!(pollster::block_on(engine.shut_down()), Ok(()), "the second one finds it already stopped");
}

/// An engine started in a scratch container whose journal lists `card.mp4`, which an export the system
/// killed was writing, so its start deletes the file and reports it.
fn engine_after_a_killed_export(container: &std::path::Path) -> Arc<Engine> {
    let folders = Folders {
        container: container.display().to_string(),
        bundle: None,
        config: container.join("config").display().to_string(),
        cache: container.join("cache").display().to_string(),
        models: container.join("models").display().to_string(),
        plugins: None,
        fonts: vec![],
    };
    let core_folders = craftbridge_core::folders::Folders::from(folders);
    core_folders.prepare().expect("the scratch folders prepare");
    std::fs::write(container.join("card.mp4"), b"half a movie").expect("the half-written output");
    let journal = core_folders.config.join(craftbridge_core::JOURNAL_FILE);
    std::fs::write(journal, json!({ "outputs": [{ "path": "card.mp4" }] }).to_string()).expect("the journal a kill left");
    Engine::from_core(
        craftbridge_core::Engine::start_in(&core_folders, Probe { panics_on_drop: false, counted: 0 }, GpuChoice::CpuOnly).expect("the engine starts"),
    )
}

#[test]
fn recovery_reports_what_the_start_cleaned_up_after_a_killed_export_and_answers_the_same_after_shut_down() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let engine = engine_after_a_killed_export(scratch.path());
    let report = engine.recovery().expect("every path is Unicode");
    assert_eq!(report.removed.len(), 1, "{report:?}");
    assert!(report.removed.first().is_some_and(|path| path.ends_with("card.mp4")), "{report:?}");
    assert!(!scratch.path().join("card.mp4").exists(), "the half-written output was left");
    assert_eq!((report.untouched, report.refused, report.failed), (vec![], Vec::<RecoveryNote>::new(), Vec::<RecoveryNote>::new()));
    pollster::block_on(engine.shut_down()).expect("shut down");
    assert_eq!(engine.recovery().map(|again| again.removed), Ok(report.removed));
}

#[test]
fn an_engine_started_without_folders_recovers_nothing() {
    assert_eq!(engine().recovery(), Ok(Recovery { removed: vec![], untouched: vec![], refused: vec![], failed: vec![], journal: vec![] }));
}
