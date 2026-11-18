// One fake engine the tests share: a UniFFI handle with no Rust behind it, recording every call the
// facade makes on it and answering what the test sets.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation

/// What the engine was asked, in order, kept for the test to read.
actor Requests {
    private(set) var all: [(method: String, params: JSONValue)] = []

    func record(_ method: String, _ params: JSONValue) {
        all.append((method, params))
    }

    /// The params of the `index`th call, or nil when fewer were made: a call that never reached the
    /// handle fails its own test, not the whole run.
    func params(_ index: Int = 0) -> JSONValue? {
        all.indices.contains(index) ? all[index].params : nil
    }

    /// The method of the `index`th call, or nil when fewer were made.
    func method(_ index: Int = 0) -> String? {
        all.indices.contains(index) ? all[index].method : nil
    }
}

/// A UniFFI engine handle with no Rust behind it. A handle made this way is never passed to the real
/// library: every call the facade makes is overridden, records itself in ``asked`` and answers what the
/// test set. A request and a call with progress are recorded under their method and their params; a
/// frame under `frame`, its params beside the format it asked for; an MCP line under `mcp`.
///
/// `@unchecked Sendable`, as the handle it stands in for is: a test sets the answers before it hands the
/// fake to an engine and never after, and every call records through the ``Requests`` actor.
final class FakeEngine: Engine, @unchecked Sendable {
    let asked: Requests
    /// What a request and a call with progress answer, by method and params; `null` unless the test says.
    var answer: @Sendable (_ method: String, _ params: JSONValue) async throws -> String = { _, _ in "null" }
    /// The report a call with progress makes before it answers.
    var report: (done: UInt64, total: UInt64, secondsLeft: Double?) = (1, 2, 0.5)
    /// The frame bytes a frame call answers.
    var frameReply = FrameBytes(width: 2, height: 2, format: .png, data: Data())
    /// The line an MCP call answers.
    var mcpReply: String?

    init(recording asked: Requests = Requests()) {
        self.asked = asked
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle handle: UInt64) {
        asked = Requests()
        super.init(unsafeFromHandle: handle)
    }

    override func request(method: String, params: String) async throws -> String {
        let sent = try JSONValue(jsonText: params)
        await asked.record(method, sent)
        return try await answer(method, sent)
    }

    override func requestWithProgress(
        method: String,
        params: String,
        listener: any ProgressListener,
        cancel: CancelToken
    ) async throws -> String {
        let sent = try JSONValue(jsonText: params)
        await asked.record(method, sent)
        listener.progress(done: report.done, total: report.total, secondsLeft: report.secondsLeft)
        return try await answer(method, sent)
    }

    override func frame(params: String, format: PixelFormat) async throws -> FrameBytes {
        await asked.record("frame", ["params": try JSONValue(jsonText: params), "format": .string("\(format)")])
        return frameReply
    }

    override func mcp(line: String) async throws -> String? {
        await asked.record("mcp", .string(line))
        return mcpReply
    }
}

/// An effects engine whose every request the fake records in `requests` and answers `"done"`.
func engine(recording requests: Requests) -> EffectsEngine {
    let fake = FakeEngine(recording: requests)
    fake.answer = { _, _ in "\"done\"" }
    return EffectsEngine(engine: fake)
}
