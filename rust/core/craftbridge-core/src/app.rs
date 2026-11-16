//! The interface a Craft app's session shows the universal runtime.

use crate::Result;
use crate::error::CraftError;
use serde_json::Value;

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
    /// progress to `on_progress` and stopping when that answers `false` (plan §16, §19).
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
    /// * otherwise runs the request to its end and answers as [`CraftApp::request`] does.
    ///
    /// Once `on_progress` has answered `false`, the call answers [`crate::CraftError::Cancelled`]
    /// whatever the implementation returns, an error included: the caller asked to stop and the
    /// work stopped. A request that ends before any `false` keeps its own answer. A panic is
    /// answered as one in [`CraftApp::request`] is.
    fn request_with_progress(&mut self, method: &str, params: Value, on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool) -> Result<Value> {
        let _ = (params, on_progress);
        Err(CraftError::unsupported(format!("`{method}` does not run with progress in this app")))
    }
}
