// The error every engine call throws, and the words it shows the app's user.

import CraftBridgeFFI
import Foundation

/// The error every engine call throws: one case for each row of the core's mapping of failures, each
/// carrying the words of whoever met the failure, the engine's for a command and the system's for a file.
public typealias CraftError = CraftBridgeFFI.CraftError

extension CraftError: LocalizedError {
    /// The sentence the app's user sees, in the core's own words, read from the core so no second copy of
    /// them can fall out of step: a case's text as it came, a failed command's name before its text and
    /// its step after, and `cancelled` for a cancelled export.
    public var errorDescription: String? { craftErrorMessage(error: self) }
}
