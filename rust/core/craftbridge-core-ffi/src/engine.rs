//! The engine handle an app calls: one exported object over the core's.

use crate::blocking::off_thread;
use crate::cancel::CancelToken;
use crate::crossing::{Crossing, Limits, carry};
use crate::error::CraftError;
use crate::frames::{FrameBytes, PixelFormat};
use crate::progress::ProgressListener;
use crate::recovery::Recovery;
use serde_json::Value;
use std::sync::Arc;

/// A handle to one engine. Every call but `recovery` is `async`, and every call throws [`CraftError`].
/// A call reaches the engine as it is first polled, which the foreign caller does as it makes the call,
/// and awaits the answer the engine thread sends it, so the engine runs calls in the order they were
/// made and no thread blocks for an answer, bar `shut_down`'s wait for the engine thread to stop, which
/// a thread of its own makes.
#[derive(uniffi::Object)]
pub struct Engine {
    core: Arc<craftbridge_core::Engine>,
    /// The most bytes of each kind of value this engine hands over; a field so a test can lower them.
    limits: Limits,
}

impl Engine {
    /// Wrap a started core engine, which an app crate's constructor makes with
    /// `craftbridge_core::Engine::start_in`.
    #[must_use]
    pub fn from_core(core: craftbridge_core::Engine) -> Arc<Self> {
        Arc::new(Self { core: Arc::new(core), limits: Limits::BOUNDARY })
    }

    /// A request's answer as the JSON text that crosses, refused when the boundary cannot carry it.
    fn carry_answer(&self, answer: Value) -> Result<String, CraftError> {
        let text = answer.to_string();
        let len = text.len();
        carry(Crossing::Answer, text, len, self.limits)
    }
}

#[uniffi::export]
impl Engine {
    /// Run one named request and answer with its result as JSON text.
    pub async fn request(&self, method: String, params: String) -> Result<String, CraftError> {
        let answer = self.core.send_request(&method, parse(&params)?)?;
        self.carry_answer(answer.answered().await?)
    }

    /// Run one request that reports progress to `listener` and stops when `cancel` is set, and answer
    /// with its result as JSON text.
    pub async fn request_with_progress(
        &self,
        method: String,
        params: String,
        listener: Arc<dyn ProgressListener>,
        cancel: Arc<CancelToken>,
    ) -> Result<String, CraftError> {
        let report = move |done, total, seconds_left| listener.progress(done, total, seconds_left);
        let answer = self.core.send_request_with_progress(&method, parse(&params)?, report, &cancel.core())?;
        self.carry_answer(answer.answered().await?)
    }

    /// Render one frame as `format`. The bytes are copied as they cross, and a frame the boundary
    /// cannot carry is refused (see [`crate::MAX_FRAME_BYTES`]).
    pub async fn frame(&self, params: String, format: PixelFormat) -> Result<FrameBytes, CraftError> {
        let answer = self.core.send_frame(parse(&params)?, format)?;
        let frame = answer.answered().await?;
        let len = frame.data.len();
        carry(Crossing::Frame, frame, len, self.limits)
    }

    /// Answer one line of the app's MCP transport with the one line it replies with, or `None` for a
    /// line that needs no reply. A closed engine and a caught panic throw, as `request` does.
    pub async fn mcp(&self, line: String) -> Result<Option<String>, CraftError> {
        let reply = self.core.send_mcp(&line)?.answered().await?;
        let len = reply.as_ref().map_or(0, String::len);
        carry(Crossing::McpLine, reply, len, self.limits)
    }

    /// Give the GPU up for the background: the engine drops its device and the preview surface, so
    /// once this answers nothing is submitted to the GPU until [`Engine::resume_gpu`], and every
    /// render answers on the CPU. It waits its turn as every call does, so one made during an export
    /// answers once the export has ended, which renders on its device until then. Suspending twice is
    /// not an error; a closed engine throws `Internal`.
    pub async fn suspend_gpu(&self) -> Result<(), CraftError> {
        self.core.send_suspend_gpu()?.answered().await
    }

    /// Ask for a device again and hand it to the app, on return to the foreground, answering `None`
    /// once the app holds a device and the CPU note `info.gpu` reports when none came: that is no
    /// failure. An engine that holds a device keeps it. It throws what the app answers when it
    /// cannot take the device, and a closed engine throws `Internal`. The note crosses unchecked, as
    /// an error's text does: no call's parameters make it grow.
    pub async fn resume_gpu(&self) -> Result<Option<String>, CraftError> {
        self.core.send_resume_gpu()?.answered().await
    }

    /// Stop the engine after the calls it has accepted. Stopping twice is not an error.
    ///
    /// The calls made after this one are refused from the moment it is made, as every call is handed
    /// over as it is made. Waiting for the engine thread to stop joins it, which waits for the call it
    /// is running, so that wait is made on a thread of its own rather than the caller's. A call from
    /// inside one of the engine's own calls, a progress listener's, cannot wait for the engine that is
    /// running it, so it answers at once, as the core's `close` does.
    ///
    /// It is not called `close`: Kotlin's generated class is an `AutoCloseable` whose own `close()`
    /// frees the handle, and a `suspend fun close()` beside it is a conflicting overload that does
    /// not compile.
    pub async fn shut_down(&self) -> Result<(), CraftError> {
        if !self.core.begin_close() {
            return Ok(());
        }
        let core = Arc::clone(&self.core);
        off_thread(move || core.close()).await
    }

    /// What the start did about the files an export killed mid-way left behind: the files it deleted,
    /// left and refused to touch, and what it found in the journal. It never changes, and answers the
    /// same after the engine is closed. It throws `Io` only for a path that is not valid Unicode, which
    /// no text can name.
    pub fn recovery(&self) -> Result<Recovery, CraftError> {
        Recovery::try_from(self.core.recovery())
    }
}

#[cfg(test)]
impl Engine {
    /// The limits this engine refuses beyond.
    pub(crate) fn limits(&self) -> Limits {
        self.limits
    }

    /// This engine, refusing values beyond `limits`.
    pub(crate) fn with_limits(self: Arc<Self>, limits: Limits) -> Arc<Self> {
        Arc::new(Self { core: Arc::clone(&self.core), limits })
    }
}

/// A call's parameters: JSON text, which the core takes as a value.
fn parse(params: &str) -> Result<Value, CraftError> {
    Ok(serde_json::from_str(params)?)
}
