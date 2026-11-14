//! The interface a Craft app's session shows the universal runtime.

use crate::Result;
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
}
