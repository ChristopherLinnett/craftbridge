// One fake engine the tests share: a UniFFI handle with no Rust behind it, recording every call the
// facade makes on it and answering what the test sets.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Darwin
import Foundation
import Testing
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

/// Removes every run folder of the home that no live run holds. One that will not go now is left for the
/// next sweep.
private func sweepTheHome() {
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
