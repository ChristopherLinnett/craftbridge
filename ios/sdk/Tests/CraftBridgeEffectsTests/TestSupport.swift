// One fake engine the tests share: a UniFFI handle with no Rust behind it, recording every call the
// facade makes on it and answering what the test sets.

import AVFoundation
import CraftBridge
import CraftBridgeEffects
import CraftBridgeEffectsConformance
import CraftBridgeFFI
import Darwin
import Foundation
import Testing
import UIKit
import os

/// What the engine was asked, in order, kept for the test to read.
actor Requests {
    private(set) var all: [(method: String, params: JSONValue)] = []
    /// The parameter text of each call made with text, exactly as the handle was handed it, so that a test
    /// can tell text the engine was given unchanged from text read and written again.
    private(set) var texts: [String] = []

    func record(_ method: String, _ params: JSONValue) {
        all.append((method, params))
    }

    /// Keeps the parameter text a call was handed, and answers it read as a value when it is JSON and as a
    /// string of the same characters when it is not, so a test can hand the handle text that is none.
    func read(_ text: String) -> JSONValue {
        texts.append(text)
        return (try? JSONValue(jsonText: text)) ?? .string(text)
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
        let sent = await asked.read(params)
        await asked.record(method, sent)
        return try await answer(method, sent)
    }

    override func requestWithProgress(
        method: String,
        params: String,
        listener: any ProgressListener,
        cancel: CancelToken
    ) async throws -> String {
        let sent = await asked.read(params)
        await asked.record(method, sent)
        listener.progress(done: report.done, total: report.total, secondsLeft: report.secondsLeft)
        return try await answer(method, sent)
    }

    override func frame(params: String, format: PixelFormat) async throws -> FrameBytes {
        let sent = await asked.read(params)
        await asked.record("frame", ["params": sent, "format": .string("\(format)")])
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

/// An engine started in `folders` — a suite's own isolated ones unless the test is about the app's own
/// folders — with a comp of `seconds` at 30 frames a second to export, and a folder of its own for each
/// test's output inside `folder`, the cache unless a test names another.
struct Rig {
    let engine: Engine
    let output: URL

    init(seconds: Int, in folder: URL? = nil, folders: Folders) async throws {
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

/// The seconds every report of a test's own export stands still, through the seam the tests own: the
/// export waits on every report it takes, so the pause slows what the export does on the engine's thread,
/// its start among it — the whole suite's slowness under load, given to one export alone — while a render,
/// on a thread of its own, runs on and only hears a cancel that much later. What a test asserts is what
/// the export does, never how fast the machine was while it did it, so the same test proves itself against
/// an export far slower than this machine makes it by raising this, and against one as quick as this
/// machine makes it by setting it to zero, never by loading the machine.
let reportPause: TimeInterval = 0.5

/// The seconds the first report of a ``CancelFromReport`` waits for its test to hand the job over. The
/// bound is on a hang, never on how long the hand-over takes: the hand-over is the statement after the one
/// that starts the export, so it is reached only by a test that hands its job over nowhere at all.
let handedOverWithin: TimeInterval = 60

/// An export's progress reports, and the cancel its test asks from one of them — which is where an export
/// reads a cancel, since the engine reads the token as each report returns. A cancel handed in from
/// outside a report reaches the export only at its next one, and a quick export can be finished before
/// then and answer `Ok`; a cancel asked inside a report is read by that very report, however quick the
/// export is, and no report follows it. The report that asks is kept, so the test can say how many reports
/// the export still owed it: none.
///
/// The first report waits for ``take(_:)`` to hand the job over, so a test that asks as soon as the export
/// reports is heard at that very report. Every export makes that first report before it writes a single
/// file, so a cancel asked there reaches an export however fast this machine would have rendered it.
final class CancelFromReport: Sendable {
    /// One report: when it came, and how far the export had come.
    struct Report: Sendable {
        let at: Date
        let done: UInt64
        let total: UInt64
    }

    /// The seconds into the export from which a report asks the cancel: the first report at or after them.
    private let askAfter: TimeInterval
    /// The seconds every report stands still for: the seam that runs the export slow or quick through the
    /// test, while everything the test reads stays the export's own work.
    private let pause: TimeInterval
    /// What the reports are printed under, so a run's log says which export spoke.
    private let label: String
    private let started = Date()
    private let kept = OSAllocatedUnfairLock(initialState: [Report]())
    private let job = OSAllocatedUnfairLock<ExportJob?>(initialState: nil)
    private let asked = OSAllocatedUnfairLock<Report?>(initialState: nil)
    private let readBy = OSAllocatedUnfairLock<Int?>(initialState: nil)
    /// Signalled once ``take(_:)`` has handed the job over, which the first report waits for.
    private let handedOver = DispatchSemaphore(value: 0)

    /// A listener that asks the export to stop at its first report at or after `askAfter` seconds into it
    /// — at the very first, for the default — stands each report still for `pause` seconds and prints each
    /// under `label`.
    init(
        askingAfter askAfter: TimeInterval = 0, pausing pause: TimeInterval = reportPause,
        printedAs label: String = "export"
    ) {
        self.askAfter = askAfter
        self.pause = pause
        self.label = label
    }

    /// When the export started, as its reports count from it.
    var start: Date { started }

    /// The first report, or nil when none came.
    func first() -> Report? { kept.withLock { $0.first } }

    /// The cancel, as seconds since the export started, or nil when no report came to ask it in.
    func cancel() -> TimeInterval? { asked.withLock { $0?.at.timeIntervalSince(started) } }

    /// How many frames the report that asked the cancel counted the export as having to draw, or nil when
    /// none asked it: the listing's reports count none, and the render's count its item's once its first
    /// frame is drawn.
    func askedFrom() -> UInt64? { asked.withLock { $0?.total } }

    /// How many reports the export can have made by the one that reads its cancel, or nil when the cancel
    /// never reached it. The engine reads the cancel as each report returns, so a cancel asked inside a
    /// report is read by that very report, and one handed in from outside a report by the next.
    func lastReport() -> Int? { readBy.withLock { $0 } }

    /// How many reports came.
    func count() -> Int { kept.withLock { $0.count } }

    /// The handler to give the export. Each report ends by standing still for the seam's pause: the export
    /// is mid-report until the handler is done, so the pause runs the export's steps on the engine's thread
    /// slow, as a loaded runner runs them slow, and what the test reads is the export's work, which no such
    /// slowness moves.
    var handler: ProgressHandler {
        { [started, askAfter, pause, label, kept, job, asked, readBy, handedOver] done, total, _ in
            let now = Date()
            let report = Report(at: now, done: done, total: total)
            let count = kept.withLock { kept in
                kept.append(report)
                return kept.count
            }
            // The export cannot be stopped before the test has the job to stop it with, and a quick export
            // could be past this report before the line that starts it has handed the job over. Standing
            // still here until it has is what makes a cancel asked at the first report one this very report
            // reads, on any machine, rather than one the export could be done before it ever saw.
            if count == 1, handedOver.wait(timeout: .now() + handedOverWithin) == .timedOut {
                Issue.record(
                    "the export's first report waited \(handedOverWithin) s for its job and none was handed over"
                )
            }
            let ask = asked.withLock { asked in
                guard asked == nil, now.timeIntervalSince(started) >= askAfter else { return false }
                asked = report
                return true
            }
            if ask, let export = job.withLock({ $0 }) {
                export.cancel()
                readBy.withLock { $0 = min($0 ?? count, count) }
            }
            print("\(label): \(done) of \(total) at \(now.timeIntervalSince(started)) s")
            fflush(stdout)
            if pause > 0 { Thread.sleep(forTimeInterval: pause) }
        }
    }

    /// Hands the export to the listener, and asks any cancel a report had already called for: the export
    /// reads that one at its next report.
    func take(_ export: ExportJob) {
        job.withLock { $0 = export }
        handedOver.signal()
        guard asked.withLock({ $0 != nil }) else { return }
        export.cancel()
        let next = count() + 1
        readBy.withLock { $0 = min($0 ?? next, next) }
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
    /// Folders no other test shares, laid out as the corpus's are in a container of their own, a run
    /// folder (``runFolder(_:)``), with the test bundle to read: an engine started in the app's own folders
    /// holds their journal's lock and writes a probe file there, which another test's engine meets while the
    /// two run side by side, and so does another run's on the same simulator. The container carries `name`,
    /// and the suite whose tests start engines here carries ``Removing`` under that name, so the containers
    /// go when its last test has run; a test that removes its own container when it ends — a device test —
    /// may leave `name` at the default and do the removing itself.
    static func isolated(_ name: String = "tests") -> Folders {
        engineFolders(container: runFolder(name), bundle: Bundle.main.bundleURL)
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

/// The length of a movie's picture track, in seconds.
func seconds(of movie: URL) async throws -> Double {
    let track = try #require(try await AVURLAsset(url: movie).loadTracks(withMediaType: .video).first)
    return try await track.load(.timeRange).duration.seconds
}

/// The running example's card's length, in seconds.
let cardSeconds = 6

/// The running example's card's frame rate.
let cardFrameRate = 30

/// The running example's card template, made the way Leo makes one: a composition of `width` by `height`, `seconds`
/// long (``cardSeconds``) at `frameRate` frames a second (``cardFrameRate``), with a text layer and a red picture as
/// imported footage, a Name control on the text and a Photo control on the picture, saved as `card.ectemplate` in
/// `folder`. The steps name the layers and items, not their ids, so nothing depends on which numbers the session
/// hands out.
func makeCardTemplate(
    _ engine: EffectsEngine, in folder: URL, width: Int, height: Int, seconds: Int = cardSeconds,
    frameRate: Int = cardFrameRate
) async throws -> URL {
    let photo = folder.appending(path: "photo.png")
    let template = folder.appending(path: "card.ectemplate")
    try picture(.red, to: photo)
    _ = try await engine.run([
        (
            command: "comp.new",
            params: [
                "name": "Card", "width": .number(Double(width)), "height": .number(Double(height)),
                "duration": .number(Double(seconds)), "frameRate": .number(Double(frameRate)),
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

/// Where every run of the suite makes its run folders: the test runner's temporary folder. A plain run on
/// the simulator has the simulator's own `tmp`, which every plain run there shares and the simulator empties
/// when it shuts down; the `swift_test` stage gives its runs one of their own.
let runFoldersHome = FileManager.default.temporaryDirectory

/// What every run folder's name starts with, so the sweep looks at those folders of the home and no other.
let runFolderPrefix = "craftbridge-"

/// The folders this run has made and still holds, under the name of the suite that removes them, each with
/// the open folder whose lock says that the run making it is alive. They are kept, not looked for again
/// when the suite ends: another run on the same simulator makes folders of the very same names beside
/// these.
private let madeFolders = OSAllocatedUnfairLock<[String: [(folder: URL, lock: Int32)]]>(initialState: [:])

/// A new folder for this run in the home, remembered so that the suite whose name it carries can remove
/// it, and everything inside it, when that suite's last test has run. The run holds each folder it makes
/// with an exclusive lock, which the system lets go however the run ends, so a folder of the home that no
/// open holds is one no live run uses, as a run killed before its suites ended leaves them: each new folder
/// is made after a sweep that removes those, and never one another run on the simulator still holds.
func runFolder(_ name: String) -> URL {
    let folder = newRunFolder(name)
    do {
        let lock = try holdingTheHome {
            sweepTheHome()
            return try madeAndHeld(folder)
        }
        madeFolders.withLock { made in made[name, default: []].append((folder, lock)) }
    } catch {
        Issue.record(error, "\(folder.path(percentEncoded: false)) is not held, so another run's sweep may remove it")
    }
    return folder
}

/// Where a new run folder carrying `name` goes in the home: no folder is there yet.
func newRunFolder(_ name: String) -> URL {
    runFoldersHome.appending(path: "\(runFolderPrefix)\(name)-\(UUID().uuidString)", directoryHint: .isDirectory)
}

/// Runs `body` while this run holds the home's own lock: a run makes and holds a folder, and sweeps, only
/// under it, so no sweep ever finds a folder made and not yet held, which it would take for a killed run's.
func holdingTheHome<T>(_ body: () throws -> T) throws -> T {
    let home = try openedFolder(runFoldersHome)
    defer { release(home) }
    while flock(home, LOCK_EX) != 0 {
        let code = errno
        guard code == EINTR else { throw lockFailure("cannot lock", runFoldersHome, code) }
    }
    return try body()
}

/// The seconds every sweep of the home adds to its own work while a test holds it set. A sweep on a busy
/// runner costs many times what this machine's fastest costs, and a test that waits for a sweep must wait
/// on the sweep itself, so the test that proves its wait sets this and proves it on a sweep far slower
/// than the fastest machine would need a deadline for. Zero while no such test is running.
let sweepPause = OSAllocatedUnfairLock<TimeInterval>(initialState: 0)

/// Removes every run folder of the home that no live run holds. One that will not go now is left for the
/// next sweep.
private func sweepTheHome() {
    // The pause stands under the home lock, where a slow runner holds it for all of the sweep, so a test
    // waiting on a sweep waits on the slowest one rather than on how fast this machine happens to be.
    let pause = sweepPause.withLock { $0 }
    if pause > 0 { Thread.sleep(forTimeInterval: pause) }
    let files = FileManager.default
    let entries = (try? files.contentsOfDirectory(at: runFoldersHome, includingPropertiesForKeys: nil)) ?? []
    for entry in entries where entry.lastPathComponent.hasPrefix(runFolderPrefix) {
        // A file of that name, or a folder its run has just removed, has nothing to sweep.
        let lock = open(entry.path(percentEncoded: false), O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        guard lock >= 0 else { continue }
        defer { release(lock) }
        if (try? lockedAtOnce(lock, entry)) == true {
            try? files.removeItem(at: entry)
        }
    }
}

/// `folder`, made and held by this run.
func madeAndHeld(_ folder: URL) throws -> Int32 {
    let lock = try openedFolder(folder)
    var held = false
    defer { if !held { release(lock) } }
    guard try lockedAtOnce(lock, folder) else {
        throw CraftError.Io(detail: "\(folder.path(percentEncoded: false)) is held by another run as it is made")
    }
    held = true
    return lock
}

/// Removes the folders this run made under `name`, and lets them go: each is removed while it is still
/// held, so no other run's sweep is removing it at the same time, and a folder that would not go is left
/// unheld, for the next run's sweep.
private func removeRunFolders(named name: String) {
    for made in madeFolders.withLock({ made in made.removeValue(forKey: name) ?? [] }) {
        try? FileManager.default.removeItem(at: made.folder)
        release(made.lock)
    }
}

/// Removes every folder the run made under `name` once every test of the suite it is set on has run.
struct Removing: SuiteTrait, TestScoping {
    let name: String

    func provideScope(
        for test: Test,
        testCase: Test.Case?,
        performing function: @Sendable () async throws -> Void
    ) async throws {
        defer { removeRunFolders(named: name) }
        try await function()
    }
}

/// Keeps every other run of the suite out of the app's own folders while the suite it is set on is in
/// them. A Swift package's tests on the simulator have no app container: the simulator's own data folder
/// is their sandbox, so it is one set of app folders for every plain run on that simulator, and the core
/// lets one process hold one container's export journal at a time. Only tests that take `.appDefault`
/// themselves need this — there `.appDefault` is what is under test — and they wait here for a run that
/// got there first. A run given a sandbox of its own, as the gate's stage gives its runs, has its own copy
/// of the folder and waits for no one.
struct AppsFoldersLock: SuiteTrait, TestScoping {
    /// The folder held: the app's own, the container of `.appDefault`.
    static var folder: URL { URL(filePath: Folders.appDefault.container, directoryHint: .isDirectory) }

    func provideScope(
        for test: Test,
        testCase: Test.Case?,
        performing function: @Sendable () async throws -> Void
    ) async throws {
        try await holding(Self.folder) { try await function() }
    }
}

/// Runs `body` while this run holds an exclusive lock on `folder` itself, waiting a quarter of a second at
/// a time while another run holds it. The lock is on the folder, so nothing is made for it and nothing is
/// left behind, and the system lets it go however the run ends.
func holding(_ folder: URL, while body: () async throws -> Void) async throws {
    let lock = try openedFolder(folder)
    defer { release(lock) }
    while try !lockedAtOnce(lock, folder) {
        try await Task.sleep(for: .milliseconds(250))
    }
    try await body()
}

/// An open of `folder` to lock it by, made first when it is not there yet. A program the run starts does
/// not keep it once that program runs.
func openedFolder(_ folder: URL) throws -> Int32 {
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    let lock = open(folder.path(percentEncoded: false), O_RDONLY | O_DIRECTORY | O_CLOEXEC)
    guard lock >= 0 else { throw lockFailure("cannot open", folder, errno) }
    return lock
}

/// Whether `lock`, an open of `folder`, took the folder's exclusive lock at once: false while another
/// open, of this process or another, holds it.
func lockedAtOnce(_ lock: Int32, _ folder: URL) throws -> Bool {
    if flock(lock, LOCK_EX | LOCK_NB) == 0 { return true }
    let code = errno
    guard code == EWOULDBLOCK else { throw lockFailure("cannot lock", folder, code) }
    return false
}

/// Lets `lock` go, and closes it. It is unlocked before it is closed: a program the run starts holds a
/// copy of every open until that program runs, and the lock belongs to the open those copies share.
func release(_ lock: Int32) {
    flock(lock, LOCK_UN)
    close(lock)
}

/// The failure to `what` `folder`, with the system's error number.
private func lockFailure(_ what: String, _ folder: URL, _ code: Int32) -> CraftError {
    CraftError.Io(detail: "\(what) \(folder.path(percentEncoded: false)): errno \(code)")
}
