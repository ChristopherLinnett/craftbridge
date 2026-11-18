// One fake engine the tests share: a UniFFI handle with no Rust behind it, recording every call the
// facade makes on it and answering what the test sets.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import UIKit
import os

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
    /// What the engine reports of holding a preview surface.
    var holdsPreview = false
    /// What a GPU suspend and resume throw, when a test sets it.
    var gpuRefusal: CraftError?
    /// What a GPU resume answers: nil is a device back, a note is the CPU's reason.
    var resumeNote: String?

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

    override func present(params: String) async throws {
        await asked.record("present", try JSONValue(jsonText: params))
    }

    override func hasPreviewSurface() async throws -> Bool {
        holdsPreview
    }

    override func mcp(line: String) async throws -> String? {
        await asked.record("mcp", .string(line))
        return mcpReply
    }

    override func suspendGpu() async throws {
        await asked.record("suspendGpu", .null)
        if let refusal = gpuRefusal { throw refusal }
    }

    override func resumeGpu() async throws -> String? {
        await asked.record("resumeGpu", .null)
        if let refusal = gpuRefusal { throw refusal }
        return resumeNote
    }
}

/// An effects engine whose every request the fake records in `requests` and answers `"done"`.
func engine(recording requests: Requests) -> EffectsEngine {
    let fake = FakeEngine(recording: requests)
    fake.answer = { _, _ in "\"done\"" }
    return EffectsEngine(engine: fake)
}

/// An engine started in `folders`, the app's own unless a test names others, with a comp of `seconds` at
/// 30 frames a second to export, and a folder of its own for each test's output inside `folder`, the cache
/// unless a test names another.
struct Rig {
    let engine: Engine
    let output: URL

    init(seconds: Int, in folder: URL? = nil, folders: Folders = .appDefault) async throws {
        engine = try startEffectsEngine(folders: folders, gpu: .cpuOnly)
        let cache = URL(filePath: folders.cache, directoryHint: .isDirectory)
        output = (folder ?? cache).appending(path: "ExportJobTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let comp: JSONValue = [
            "name": "Card", "width": 1280, "height": 720, "duration": .number(Double(seconds)), "frameRate": 30,
        ]
        try await engine.exec([("comp.new", comp), ("layer.newSolid", ["comp": "Card", "name": "Background"])])
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
final class FirstReport: Sendable {
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

/// The system as a test makes it: its own notification centre, which the test posts the app's lifecycle to,
/// and a background task that is granted, or refused, as the test sets, and recorded either way.
final class FakeHost: BackgroundHost, Sendable {
    let notifications = NotificationCenter()
    private let state = OSAllocatedUnfairLock(initialState: State())

    private struct State {
        var grants = true
        var next = 1
        var begun: [Int] = []
        var ended: [Int] = []
        var expiries: [Int: @Sendable () -> Void] = [:]
    }

    /// The tokens of the tasks begun, in order.
    var begun: [Int] { state.withLock { $0.begun } }
    /// The tokens of the tasks ended, in order.
    var ended: [Int] { state.withLock { $0.ended } }

    /// Whether the system grants time to the tasks begun from now on.
    func grant(_ granted: Bool) {
        state.withLock { $0.grants = granted }
    }

    /// The system's warning that the app is about to be stopped, for the task `token`.
    func expire(_ token: Int) {
        let handler = state.withLock { $0.expiries[token] }
        handler?()
    }

    func post(_ name: Notification.Name) {
        notifications.post(name: name, object: nil)
    }

    @MainActor func beginTask(named name: String, onExpiry: @escaping @Sendable () -> Void) -> Int? {
        state.withLock { state in
            guard state.grants else { return nil }
            let token = state.next
            state.next += 1
            state.begun.append(token)
            state.expiries[token] = onExpiry
            return token
        }
    }

    @MainActor func endTask(_ token: Int) {
        state.withLock { $0.ended.append(token) }
    }
}

/// Waits, a turn at a time, until `condition` holds or a generous number of turns have passed, for what
/// the app's notifications set going on the main actor: they are delivered through a task of their own.
func eventually(_ condition: @MainActor () async -> Bool) async -> Bool {
    for _ in 0..<2000 {
        if await condition() { return true }
        try? await Task.sleep(for: .milliseconds(5))
    }
    return false
}

extension Engine {
    /// Runs the commands in order through the engine's `exec`, the way a test sets a comp up.
    func exec(_ steps: [(command: String, params: JSONValue)]) async throws {
        for (command, params) in steps {
            let step: JSONValue = ["command": .string(command), "params": params]
            _ = try await request(method: "exec", params: try step.jsonText())
        }
    }

    /// A comp `name` of `width` by `height` that is transparent but for one solid of `color` at `opacity` per
    /// cent: every pixel the solid covers is translucent, so a frame of it differs as a straight frame and
    /// as a premultiplied one.
    func veil(_ name: String, width: Int, height: Int, color: String, opacity: Int) async throws {
        try await exec([
            ("comp.new", ["name": .string(name), "width": .number(Double(width)), "height": .number(Double(height))]),
            ("layer.newSolid", ["comp": .string(name), "name": "Veil", "color": .string(color)]),
            (
                "prop.set",
                [
                    "comp": .string(name), "layer": "Veil", "path": "transform/opacity",
                    "value": .number(Double(opacity)),
                ]
            ),
        ])
    }
}

extension Folders {
    /// Folders no other test shares, laid out as the corpus's are in a container of their own under the
    /// simulator's temporary folder, with the test bundle to read: an engine started in the app's own folders
    /// holds their journal's lock and writes a probe file there, which another test's engine meets while the
    /// two run side by side.
    static func isolated() -> Folders {
        engineFolders(container: temporaryFolder("tests"), bundle: Bundle.main.bundleURL)
    }
}

/// Writes a solid-color picture the engine can import, standing in for the photo from the camera.
func picture(_ color: UIColor, to url: URL) throws {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8))
    let data = renderer.pngData { _ in
        color.setFill()
        UIRectFill(CGRect(x: 0, y: 0, width: 8, height: 8))
    }
    try data.write(to: url)
}

/// The running example's card template, made the way Leo makes one: a composition of `width` by `height`, 6
/// seconds at 30 frames a second, with a text layer and a red picture as imported footage, a Name control on
/// the text and a Photo control on the picture, saved as `card.ectemplate` in `folder`. The steps name the
/// layers and items, not their ids, so nothing depends on which numbers the session hands out.
func makeCardTemplate(_ engine: EffectsEngine, in folder: URL, width: Int, height: Int) async throws -> URL {
    let photo = folder.appending(path: "photo.png")
    let template = folder.appending(path: "card.ectemplate")
    try picture(.red, to: photo)
    _ = try await engine.run([
        (
            command: "comp.new",
            params: [
                "name": "Card", "width": .number(Double(width)), "height": .number(Double(height)), "duration": 6,
                "frameRate": 30,
            ]
        ),
        (command: "layer.newSolid", params: ["comp": "Card", "name": "Bg", "color": "#223344"]),
        (command: "layer.newText", params: ["comp": "Card", "text": "Hello", "name": "Name"]),
        (command: "file.import", params: ["paths": [.string(photo.path(percentEncoded: false))]]),
        (command: "layer.addItem", params: ["comp": "Card", "item": "photo.png"]),
        (
            command: "essential.addProperty",
            params: ["comp": "Card", "layer": "Name", "path": "text/sourceText", "name": "Name"]
        ),
        (command: "essential.addMedia", params: ["layer": "photo.png", "name": "Photo"]),
        (
            command: "essential.exportTemplate",
            params: ["comp": "Card", "path": .string(template.path(percentEncoded: false))]
        ),
    ])
    return template
}
