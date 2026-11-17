//! What the crate's tests share: how long a test waits for what the engine must do in the end,
//! and the small handshakes every test that races the engine needs.

use crate::Result;
use crate::engine::{Engine, Message};
use crate::folders::Folders;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long a test waits for anything the engine must eventually deliver, so a broken engine fails
/// with a message instead of hanging the suite. No behaviour of the engine is measured against it:
/// every promise is proven by what arrives, and by what does not arrive until the test lets it,
/// never by how long something took, so a loaded machine slows the tests without failing them.
pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

/// Whether `done` came true within [`DEADLINE`], asked again every millisecond until it does.
pub(crate) fn eventually(mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + DEADLINE;
    while !done() {
        if Instant::now() > until {
            return false;
        }
        thread::sleep(Duration::from_millis(1));
    }
    true
}

/// A gate one side of a test opens and the other waits at: a fake app's call or drop held until
/// the test lets it go, or the test held until the app has got somewhere. Nothing opens on a timer,
/// so what a test checks while a gate is shut holds however slowly the machine runs.
#[derive(Default)]
pub(crate) struct Gate {
    open: AtomicBool,
}

impl Gate {
    /// Let whoever waits at the gate go on.
    pub(crate) fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
    }

    /// Whether the gate opened within [`DEADLINE`]: false means it never did, which the waiter
    /// answers as a failure rather than waiting for ever.
    pub(crate) fn wait(&self) -> bool {
        eventually(|| self.open.load(Ordering::SeqCst))
    }
}

/// Hand one plain call to `engine` from the test's own thread and take the channel its answer
/// comes on: the inbox is first in, first out, so calls handed over one after another from one
/// thread reach the engine in that order, with no thread to schedule.
pub(crate) fn queue_call(engine: &Engine, method: &str, params: Value) -> mpsc::Receiver<Result<Value>> {
    engine.submit(Message::request(method, params)).unwrap()
}

/// Take the answer that arrives on `rx`, failing the test at the deadline if none ever does: an
/// engine that stops answering is a test failure with a message, never a hung suite.
pub(crate) fn recv<T>(rx: &mpsc::Receiver<T>) -> T {
    rx.recv_timeout(DEADLINE).unwrap_or_else(|_| panic!("the call was never answered"))
}

/// Close `engine` from a thread of its own and return once the close has begun, so `Close` is
/// queued and every export has been asked to stop. The close runs apart from the test because it
/// joins the engine thread, which may still be inside a call only the test can let go.
pub(crate) fn close_begun(engine: &Arc<Engine>) -> JoinHandle<Result<()>> {
    let closing = engine.clone();
    let closer = thread::spawn(move || closing.close());
    assert!(eventually(|| engine.is_closing()), "the close never began");
    closer
}

/// The folders of an app's first launch with its sandbox at `container`: the working folders inside it,
/// as an app names them, none created yet.
pub(crate) fn scratch_folders(container: PathBuf) -> Folders {
    Folders {
        config: container.join("config"),
        cache: container.join("cache"),
        models: container.join("models"),
        container,
        bundle: None,
        plugins: None,
        fonts: Vec::new(),
    }
}
