// The listener of a call that reports progress, which stops the call where its scenario says.

import CraftBridge
import os

/// One report of the engine's progress.
public struct Report: Equatable, Sendable {
    /// The frames done.
    public let done: UInt64
    /// The frames the call has in all, or none while it has not said.
    public let total: UInt64

    /// A report of `done` frames of `total`.
    public init(done: UInt64, total: UInt64) {
        self.done = done
        self.total = total
    }
}

/// What the listener hears of a call, kept under a lock, since the engine's reports and the caller that holds
/// the job meet here from different threads: the report the listener stopped the call at, and the job it
/// stops.
public final class Hearing: Sendable {
    private struct Heard {
        var stoppedAt: Report?
        var job: ExportJob?
    }

    private let stop: Stop
    private let heard = OSAllocatedUnfairLock(initialState: Heard())

    /// A listener that stops its call where `stop` says.
    public init(stop: Stop) {
        self.stop = stop
    }

    /// The report the listener stopped the call at, if it has.
    public var stoppedAt: Report? { heard.withLock(\.stoppedAt) }

    /// Hears a report: the first one the scenario's fraction is met at stops the job, once there is one.
    public func hear(done: UInt64, total: UInt64) {
        heard.withLock { heard in
            if heard.stoppedAt == nil, stop.stops(done: done, total: total) {
                heard.stoppedAt = Report(done: done, total: total)
                heard.job?.cancel()
            }
        }
    }

    /// Hands the job to the listener. A stop that came before there was a job to ask is asked of it now.
    public func take(_ job: ExportJob) {
        heard.withLock { heard in
            heard.job = job
            if heard.stoppedAt != nil { job.cancel() }
        }
    }
}
