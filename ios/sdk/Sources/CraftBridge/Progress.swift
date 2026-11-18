// An export's progress, handed to a closure.

import CraftBridgeFFI

/// Told how far an export has got, on the engine's thread, once every progress interval.
public typealias ProgressListener = CraftBridgeFFI.ProgressListener

/// What an export's progress is handed to: `done` of `total`, in the export's own units, and the seconds
/// left when the engine can say. It is called on the engine's thread between steps of the export, so it
/// returns quickly and hands anything slow, such as drawing a progress bar, to the main actor.
public typealias ProgressHandler = @Sendable (_ done: UInt64, _ total: UInt64, _ secondsLeft: Double?) -> Void

/// A ``ProgressListener`` made from a closure: it hands every report the engine makes to its handler.
public final class ClosureProgressListener: ProgressListener {
    private let handler: ProgressHandler

    /// A listener that hands every report to `handler`.
    public init(_ handler: @escaping ProgressHandler) {
        self.handler = handler
    }

    /// Hands one report to the handler.
    public func progress(done: UInt64, total: UInt64, secondsLeft: Double?) {
        handler(done, total, secondsLeft)
    }
}
