import CraftBridge
import CraftBridgeFFI
import Foundation
import Testing
import os

/// The containers of this suite's engines, removed when the suite ends.
private let jobFolders = "exportjob"

@Suite(Removing(name: jobFolders)) struct ExportJobTests {
    /// Cancelling the task that awaits an export's result sets the job's cancel token, and the engine
    /// stops the export and answers `Cancelled`.
    @Test func cancellingTheTaskAwaitingAnExportCancelsIt() async throws {
        let rig = try await Rig(seconds: 60, folders: .isolated(jobFolders))
        let first = FirstReport()
        let job = try ExportJob(engine: rig.engine, method: "export", params: rig.export, progress: first.handler)
        let awaiting = Task { try await job.result }
        await first.wait(for: job)
        #expect(!job.isCancelled)
        awaiting.cancel()
        await #expect(throws: CraftError.Cancelled) { try await awaiting.value }
        #expect(job.isCancelled)
        try await rig.finish()
    }

    @Test func cancelStopsAnExport() async throws {
        let rig = try await Rig(seconds: 60, folders: .isolated(jobFolders))
        let first = FirstReport()
        let job = try ExportJob(engine: rig.engine, method: "export", params: rig.export, progress: first.handler)
        await first.wait(for: job)
        job.cancel()
        #expect(job.isCancelled)
        await #expect(throws: CraftError.Cancelled) { try await job.result }
        try await rig.finish()
    }

    @Test func anExportLeftAloneAnswersWhatItRendered() async throws {
        let rig = try await Rig(seconds: 1, folders: .isolated(jobFolders))
        let job = try ExportJob(engine: rig.engine, method: "export", params: rig.export)
        let answer = try await job.result
        guard case .object(let fields) = answer, case .array(let rendered) = fields["rendered"] else {
            Issue.record("the export answered \(answer)")
            return
        }
        #expect(rendered.count == 1)
        #expect(!job.isCancelled)
        #expect(try await job.result == answer, "a result can be awaited again")
        try await rig.finish()
    }
}

@Suite struct ExportJobCrossingTests {
    @Test func aJobHandsItsMethodAndParametersToTheEngineAndItsProgressToItsHandler() async throws {
        let engine = FakeEngine()
        engine.answer = { _, _ in #"{"rendered":[{"comp":"Card"}]}"# }
        let heard = OSAllocatedUnfairLock<[String]>(initialState: [])
        let job = try ExportJob(engine: engine, method: "template.render", params: ["template": "Card"]) {
            done, total, secondsLeft in
            heard.withLock { $0.append("\(done)/\(total) \(secondsLeft.map { "\($0)" } ?? "-")") }
        }
        #expect(try await job.result == ["rendered": [["comp": "Card"]]])
        let calls = await engine.asked.all
        #expect(calls.map(\.method) == ["template.render"])
        #expect(calls.map(\.params) == [["template": "Card"]])
        #expect(heard.withLock { $0 } == ["1/2 0.5"], "the engine reported before it answered")
    }

    @Test func parametersJSONCannotCarryStartNothing() async throws {
        let engine = FakeEngine()
        let refusal = CraftError.InvalidParams(
            detail: "the parameters hold NaN or an infinity, which JSON has no number for")
        #expect(throws: refusal) { try ExportJob(engine: engine, method: "export", params: ["end": .number(.nan)]) }
        #expect(await engine.asked.all.isEmpty)
    }

    @Test func anAnswerThatIsNotJSONIsTheEnginesOwnFailure() async throws {
        let engine = FakeEngine()
        engine.answer = { _, _ in "{" }
        let job = try ExportJob(engine: engine, method: "export", params: [:])
        await #expect(throws: CraftError.Internal(detail: "the engine answered with text that is not JSON")) {
            try await job.result
        }
    }
}
