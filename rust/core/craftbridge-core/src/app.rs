//! The interface a Craft app's session shows the universal runtime.

use crate::Result;
use crate::error::CraftError;
use crate::frames::{FrameBytes, PixelFormat};
use serde_json::Value;
use std::path::PathBuf;

/// A hosted Craft app's session. The app plugin implements this against its own app's engine; the
/// runtime never knows which app it hosts, only this interface (plan §16).
///
/// A session is `Send` because the runtime moves it onto the engine thread it spawns, and is reached
/// only there: everything that happens to it is a [`crate::Engine`] call, answered in the order the
/// app made it (plan §16, L1). It is dropped there too, behind the same guard as a call, when the
/// engine is closed or its handle dropped.
pub trait CraftApp: Send {
    /// Run one named request with its JSON parameters and answer with the result JSON. A failed
    /// request is an error, never a panic of the caller's: the runtime already guards this call, so
    /// a panic the app itself did not catch becomes [`crate::CraftError::Internal`] and the session
    /// stays.
    fn request(&mut self, method: &str, params: Value) -> Result<Value>;

    /// Run one named request that runs long enough to show progress — an export — reporting its
    /// progress to `on_progress`, stopping when that answers `false`, and listing every file it is
    /// about to write with `list_outputs` before it writes any of them (plan §16, §19, D-17).
    ///
    /// An app with no such request keeps this default, which answers every method
    /// [`crate::CraftError::Unsupported`], as [`CraftApp::request`] answers a method its table does
    /// not hold: the refusal is the same for every such app, so it is written once, here.
    ///
    /// An implementation that runs the request:
    ///
    /// * calls `on_progress(done, total, seconds_left)` from the start of the work to its end, once
    ///   every [`crate::PROGRESS_INTERVAL`] (the CLI's interval, waited between one report and the
    ///   next), in the work's own units, with `seconds_left` `None` while it has no estimate. The
    ///   engine answers progress, cancel and `info` only inside these calls and reaches the
    ///   session through no other call until this one returns (plan §13, L3), so their spacing is
    ///   how late a cancel is seen;
    /// * on the first `false`, stops the work straight away, winding it off as the app's own cancel
    ///   does — for an export, stopping the render so the exporter removes the files it opened for
    ///   that output (L4) — and returns. A `false` means the caller's `CancelToken` was set or the
    ///   engine is closing (plan §16 behaviour 4), and every later call answers `false` again;
    /// * otherwise runs the request to its end and answers as [`CraftApp::request`] does;
    /// * hands `list_outputs` every file the request may write, as the paths its own writer will
    ///   open, once it knows them and before it writes the first: for a render, once its items are
    ///   queued and before the render starts, each output of each item (every file of an image
    ///   sequence, every segment, the render log), since a file the export writes is not always the
    ///   name it was given (a PNG sequence asked for at `card.png` writes `card_00000.png` on).
    ///   `list_outputs` may be called more than once; each call adds to the export's list. It
    ///   answers an error, which the implementation returns without writing those files, when a
    ///   path lies outside the container, is a folder or a link, or the list cannot be written.
    ///
    /// The list is the engine's pending-export journal: when the app is killed mid-export, the next
    /// start deletes each listed file the export wrote, leaving any it never reached as they were,
    /// and the engine clears the list when this call returns, whatever its outcome. An engine
    /// started without folders keeps no journal, and its `list_outputs` lists nothing.
    ///
    /// Once `on_progress` has answered `false`, the call answers [`crate::CraftError::Cancelled`]
    /// whatever the implementation returns, an error included: the caller asked to stop and the
    /// work stopped. A request that ends before any `false` keeps its own answer. A panic is
    /// answered as one in [`CraftApp::request`] is.
    fn request_with_progress(
        &mut self,
        method: &str,
        params: Value,
        on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool,
        list_outputs: &mut dyn FnMut(&[PathBuf]) -> Result<()>,
    ) -> Result<Value> {
        let _ = (params, on_progress, list_outputs);
        Err(CraftError::unsupported(format!("`{method}` does not run with progress in this app")))
    }

    /// Render one frame as `format` (plan §14, §16, §17). Binary pixels cannot come back through
    /// [`CraftApp::request`]'s JSON, so a frame has this call of its own, which the engine makes on
    /// its own thread in the order the calls were made, as it makes every call.
    ///
    /// The implementation builds its answer with [`FrameBytes::new`], handing it the bytes its
    /// render produced, so the format's size check and the premultiplied conversion are the core's
    /// one copy of them and no frame is copied on the way.
    ///
    /// An app with no frames keeps this default, which answers [`crate::CraftError::Unsupported`]:
    /// the refusal is the same for every such app, so it is written once, here.
    fn frame(&mut self, params: Value, format: PixelFormat) -> Result<FrameBytes> {
        let _ = (params, format);
        Err(CraftError::unsupported("this app renders no frames"))
    }

    /// Answer one line of the app's MCP transport (a JSON-RPC message) with the one line it replies
    /// with, or `None` when the line needs no reply, as a notification does. It acts on the same
    /// session [`CraftApp::request`] does, so an agent driving the session through MCP and the app
    /// calling it directly see one document. The default is an app with no MCP endpoint, which
    /// replies to nothing.
    fn mcp(&mut self, _line: &str) -> Option<String> {
        None
    }
}
