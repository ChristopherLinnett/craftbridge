// A call that reports progress, listened to and stopped as the scenario says. The engine cannot be told to
// hold an export where the listener stops it, as the desktop runner holds one, so the stop reaches the
// export when the engine's next beat brings it to the listener, and a quick export may have ended by then.
// Neither the answer nor the folder says which: the core answers `Cancelled` and deletes the files an export
// listed once it has been told to stop, whether or not the export ended while the stop was on its way. The
// render queue says it: the exporter that meets a stop answers that it was cancelled, which the queue
// records on the export's item as `UserStopped`, and one that had rendered every frame first records `Done`.
// So a stopped call is judged by its item, never by how far the beats say it had got, which a stop slow to
// reach the export would make it pass.

import CraftBridge
import CraftBridgeEffects
import Foundation
import os

/// One report of the engine's progress.
struct Report: Equatable, Sendable {
    let done: UInt64
    let total: UInt64
}

/// What the listener heard of a call: the report it stopped the call at, and the job it stops.
struct Heard: Sendable {
    var stoppedAt: Report?
    var job: ExportJob?
}

/// How a call that reports progress is listened to, and when it is stopped.
struct Listening: Sendable {
    let stop: Stop

    /// The status the render queue gives the item of an export that met its stop.
    static let stopped: JSONValue = "UserStopped"

    /// The call's answer, with the listener asking the call to stop once the fraction of its frames the
    /// scenario names is done. A call that stops at a fraction is answered only when its export met the
    /// stop, which ``judge(stoppedAt:statuses:)`` decides from the render queue.
    func run(_ method: String, _ params: JSONValue, on engine: EffectsEngine) async throws -> JSONValue {
        let state = OSAllocatedUnfairLock(initialState: Heard())
        let stop = stop
        // The job is made with the lock held, so the listener, which waits on the lock for its first report,
        // always finds the job it is to stop: a stop never comes before there is a job to ask.
        let job = try state.withLock { listening in
            let job = try engine.job(method, params) { done, total, _ in
                state.withLock { heard in
                    if heard.stoppedAt == nil, stop.stops(done: done, total: total) {
                        heard.stoppedAt = Report(done: done, total: total)
                        heard.job?.cancel()
                    }
                }
            }
            listening.job = job
            return job
        }
        let answer: Result<JSONValue, any Error>
        do {
            answer = .success(try await job.result)
        } catch {
            answer = .failure(error)
        }
        let stoppedAt = state.withLock(\.stoppedAt)
        let statuses = stoppedAt == nil ? [] : try await Self.statuses(on: engine)
        if let stoppedAt {
            let said = JSONValue.array(statuses).shown
            print("stopped at \(stoppedAt.done) of \(stoppedAt.total) frames: the render queue says \(said)")
        }
        if let why = judge(stoppedAt: stoppedAt, statuses: statuses) { throw RunnerFailure(description: why) }
        return try answer.get()
    }

    /// The status of each item of the engine's render queue, as `renderQueue.list` gives it.
    static func statuses(on engine: EffectsEngine) async throws -> [JSONValue] {
        let list = try await engine.exec("renderQueue.list")
        guard case .array(let items)? = list.value(at: "/items") else {
            throw RunnerFailure(description: "the render queue lists no items: \(list.shown)")
        }
        return items.map { $0.value(at: "/status") ?? .null }
    }

    /// Why a call the scenario stops at a fraction proves nothing about the stop, or nil when it does, given
    /// the report the listener stopped it at and the statuses of the render queue's items once it answered:
    /// the call must have been stopped there, and an export of it must have met the stop.
    func judge(stoppedAt: Report?, statuses: [JSONValue]) -> String? {
        guard let cancelAt = stop.cancelAt else { return nil }
        let race = "the export won the race, which the simulator has no way to hold"
        guard let stoppedAt else {
            return "the call answered before its export reached \(cancelAt) of its frames, so no stop reached it: "
                + race
        }
        guard statuses.contains(Self.stopped) else {
            return "the stop came at \(stoppedAt.done) of \(stoppedAt.total) frames, and the render queue says "
                + "\(JSONValue.array(statuses).shown), with no export \(Self.stopped.shown): every frame was rendered "
                + "before the stop reached the export, so \(race)"
        }
        return nil
    }
}
