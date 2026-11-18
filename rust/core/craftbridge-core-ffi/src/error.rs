//! The error every exported call throws, and the words it shows its user.

pub use craftbridge_core::CraftError;

// The core's own error, exported as it is defined there, so the boundary has no second copy of its
// variants to fall out of step with the plan's mapping table (§14). The compiler refuses this
// declaration if it stops matching the core's. Each variant's `message` crosses as `detail`
// (`uniffi.toml` says why).
#[uniffi::remote(Error)]
pub enum CraftError {
    InvalidParams { message: String },
    CommandFailed { command: String, message: String, step: Option<u32> },
    NotFound { message: String },
    Unsupported { message: String },
    Cancelled,
    Io { message: String },
    Gpu { message: String },
    Internal { message: String },
}

/// The sentence `error` shows the app's user: the core's own `Display` of it, a `CommandFailed`'s
/// command before its text and its step after. The bindings' messages read it from here — Swift's
/// `LocalizedError`, which `CraftBridge` gives the error — so every language says what the core says,
/// with no second copy of how each variant reads to fall out of step with the core's. A free function,
/// since an error UniFFI exports as `remote` takes no exported method of its own.
#[uniffi::export]
#[must_use]
pub fn craft_error_message(error: CraftError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant reads in the words the core's `Display` gives it: its text, a failed command's
    /// name and step around its text, and `cancelled`.
    #[test]
    fn each_variant_reads_in_the_core_s_own_words() {
        let words = [
            (CraftError::invalid_params("comp.new: unknown parameter(s) widht"), "comp.new: unknown parameter(s) widht"),
            (CraftError::command_failed("effect.apply", "no effect \"Gausian Blur\"", Some(3)), "effect.apply: no effect \"Gausian Blur\" (step 3)"),
            (CraftError::command_failed("effect.apply", "no effect \"Gausian Blur\"", None), "effect.apply: no effect \"Gausian Blur\""),
            (CraftError::not_found("no layer \"#9\" in \"Birthday Card\""), "no layer \"#9\" in \"Birthday Card\""),
            (CraftError::unsupported("external effect helpers are not available on iOS"), "external effect helpers are not available on iOS"),
            (CraftError::Cancelled, "cancelled"),
            (
                CraftError::io("cannot write /Card.ecproj: No space left on device (os error 28)"),
                "cannot write /Card.ecproj: No space left on device (os error 28)",
            ),
            (CraftError::gpu("no Metal adapter"), "no Metal adapter"),
            (CraftError::internal("engine is closed"), "engine is closed"),
        ];
        for (error, expected) in words {
            assert_eq!(craft_error_message(error), expected);
        }
    }
}
