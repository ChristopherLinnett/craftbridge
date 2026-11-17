import CraftBridge
import CraftBridgeFFI
import Foundation
import Testing
import os

/// An engine started in the app's own folders, with a comp of `seconds` at 30 frames a second to export,
/// and a folder of its own for each test's output inside `folder`, the cache unless a test names another.
private struct Rig {
    let engine: Engine
    let output: URL

    init(seconds: Int, in folder: URL? = nil) async throws {
        engine = try startEffectsEngine(folders: .appDefault, gpu: .cpuOnly)
        let cache = URL(filePath: Folders.appDefault.cache, directoryHint: .isDirectory)
        output = (folder ?? cache).appending(path: "ExportJobTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let comp: JSONValue = [
            "name": "Card", "width": 1280, "height": 720, "duration": .number(Double(seconds)), "frameRate": 30,
        ]
        for (command, params) in [("comp.new", comp), ("layer.newSolid", ["comp": "Card", "name": "Background"])] {
            let step: JSONValue = ["command": .string(command), "params": params]
            _ = try await engine.request(method: "exec", params: try step.jsonText())
        }
    }

    /// The export of the comp as a PNG sequence into this rig's folder.
    var export: JSONValue {
        [
            "comp": "Card", "output": .string(output.appending(path: "card.png").path(percentEncoded: false)),
            "format": "png",
        ]
    }

    func finish() async throws {
        try await engine.shutDown()
        try FileManager.default.removeItem(at: output)
    }
}

/// A progress handler, and a wait that ends at the first report it hears, once the export is rendering,
/// or once the job has answered, so a job that answers before it reports fails its test rather than
/// leaving it waiting.
private final class FirstReport: Sendable {
    let handler: ProgressHandler
    private let reports: AsyncStream<Void>
    private let heard: AsyncStream<Void>.Continuation

    init() {
        let (reports, heard) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        self.reports = reports
        self.heard = heard
        handler = { _, _, _ in heard.yield() }
    }

    func wait(for job: ExportJob) async {
        let heard = heard
        Task {
            _ = try? await job.result
            heard.finish()
        }
        for await _ in reports { return }
    }
}

@Suite struct ExportJobTests {
    /// Cancelling the task that awaits an export's result sets the job's cancel token, and the engine
    /// stops the export and answers `Cancelled`.
    @Test func cancellingTheTaskAwaitingAnExportCancelsIt() async throws {
        let rig = try await Rig(seconds: 60)
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
        let rig = try await Rig(seconds: 60)
        let first = FirstReport()
        let job = try ExportJob(engine: rig.engine, method: "export", params: rig.export, progress: first.handler)
        await first.wait(for: job)
        job.cancel()
        #expect(job.isCancelled)
        await #expect(throws: CraftError.Cancelled) { try await job.result }
        try await rig.finish()
    }

    @Test func anExportLeftAloneAnswersWhatItRendered() async throws {
        let rig = try await Rig(seconds: 1)
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

    /// The engine writes anywhere in the app's sandbox, so an app exports into `Documents`, which the Files app
    /// shows for an app that shares it, as well as into its `Library`.
    @Test func anExportGoesIntoTheAppsDocuments() async throws {
        let rig = try await Rig(seconds: 1, in: .documentsDirectory)
        let job = try ExportJob(engine: rig.engine, method: "export", params: rig.export)
        await #expect(throws: Never.self) { try await job.result }
        let written = try FileManager.default.contentsOfDirectory(atPath: rig.output.path(percentEncoded: false))
        #expect(written.contains { $0.hasSuffix(".png") }, "the export wrote \(written)")
        try await rig.finish()
    }
}

/// What a fake engine was asked to run.
private actor Calls {
    private(set) var all: [(method: String, params: String)] = []

    func record(_ method: String, _ params: String) {
        all.append((method, params))
    }
}

/// An engine with no native engine behind it, which records each progress call, reports one step of
/// progress and answers `answer`.
private final class ScriptedEngine: Engine, @unchecked Sendable {
    let calls = Calls()
    private let answer: String

    init(answering answer: String) {
        self.answer = answer
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle handle: UInt64) {
        answer = "null"
        super.init(unsafeFromHandle: handle)
    }

    override func requestWithProgress(
        method: String,
        params: String,
        listener: any ProgressListener,
        cancel: CancelToken
    ) async throws -> String {
        await calls.record(method, params)
        listener.progress(done: 1, total: 2, secondsLeft: 0.5)
        return answer
    }
}

@Suite struct ExportJobCrossingTests {
    @Test func aJobHandsItsMethodAndParametersToTheEngineAndItsProgressToItsHandler() async throws {
        let engine = ScriptedEngine(answering: #"{"rendered":[{"comp":"Card"}]}"#)
        let heard = OSAllocatedUnfairLock<[String]>(initialState: [])
        let job = try ExportJob(engine: engine, method: "template.render", params: ["template": "Card"]) {
            done, total, secondsLeft in
            heard.withLock { $0.append("\(done)/\(total) \(secondsLeft.map { "\($0)" } ?? "-")") }
        }
        #expect(try await job.result == ["rendered": [["comp": "Card"]]])
        let calls = await engine.calls.all
        #expect(calls.map(\.method) == ["template.render"])
        #expect(try calls.map { try JSONValue(jsonText: $0.params) } == [["template": "Card"]])
        #expect(heard.withLock { $0 } == ["1/2 0.5"], "the engine reported before it answered")
    }

    @Test func parametersJSONCannotCarryStartNothing() async throws {
        let engine = ScriptedEngine(answering: "null")
        let refusal = CraftError.InvalidParams(
            detail: "the parameters hold NaN or an infinity, which JSON has no number for")
        #expect(throws: refusal) { try ExportJob(engine: engine, method: "export", params: ["end": .number(.nan)]) }
        #expect(await engine.calls.all.isEmpty)
    }

    @Test func anAnswerThatIsNotJSONIsTheEnginesOwnFailure() async throws {
        let job = try ExportJob(engine: ScriptedEngine(answering: "{"), method: "export", params: [:])
        await #expect(throws: CraftError.Internal(detail: "the engine answered with text that is not JSON")) {
            try await job.result
        }
    }
}
