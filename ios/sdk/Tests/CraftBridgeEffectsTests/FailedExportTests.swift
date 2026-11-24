// A failed or stopped export leaves no output but the renders that finished: a file at an output path always
// means a finished render. The render fails part way, after it wrote pictures of its own, over the engine's
// real exporter, and what is left at the output is only what stood there before the export began, and the
// movie of an item of its queue that finished before the one that failed or was stopped.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import os

/// The picture of the sequence the render cannot write: a read-only file stands at its name, so creating
/// the picture fails there, after the pictures before it were written.
private let blockedPicture = "frame_00150.png"

/// What stood at ``blockedPicture`` before the export, which a failed export must leave as it was.
private let earlierPicture = Data("an earlier picture".utf8)

/// The scene a failing export renders: ten pictures a second for twenty seconds, big enough that the render
/// is still writing when the export reports, so the pictures it wrote are there to be seen as it fails, and
/// the folder `into` holding the read-only file at ``blockedPicture``.
private struct Failing {
    let into: URL
    let blocked: URL

    /// The scene on `engine`, or nil when the permissions do not hold (a user they do not hold back, root,
    /// can write the picture, and no render fails). A scene that is `blocking` no picture has a sequence
    /// that renders for `seconds` seconds and never fails, so a stop has time to reach it.
    init?(on engine: EffectsEngine, in container: URL, seconds: Int = 20, blocking: Bool = true) async throws {
        _ = try await engine.run([
            (
                command: "comp.new",
                params: [
                    "name": "Failing", "width": 1280, "height": 720, "duration": .number(Double(seconds)),
                    "frameRate": 10,
                ]
            )
        ])
        into = container.appending(path: "failing", directoryHint: .isDirectory)
        blocked = into.appending(path: blockedPicture)
        try FileManager.default.createDirectory(at: into, withIntermediateDirectories: true)
        guard blocking else { return }
        try earlierPicture.write(to: blocked)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o444], ofItemAtPath: blocked.path(percentEncoded: false))
        guard !FileManager.default.isWritableFile(atPath: blocked.path(percentEncoded: false)) else { return nil }
    }

    /// The names in the folder now.
    var names: [String] {
        (try? FileManager.default.contentsOfDirectory(atPath: into.path(percentEncoded: false))) ?? []
    }

    /// Makes the blocked picture writable again, so the container can be removed.
    func release() {
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o644], ofItemAtPath: blocked.path(percentEncoded: false))
    }

    /// Queues a movie of one second, then the sequence the render cannot finish.
    func queueMovieThenSequence(on engine: EffectsEngine) async throws -> URL {
        let movie = into.appending(path: "first.mp4")
        _ = try await engine.run([
            (
                command: "renderQueue.add",
                params: [
                    "format": "h264", "output": .string(movie.path(percentEncoded: false)), "end": 1,
                    "resolution": "quarter",
                ]
            ),
            (
                command: "renderQueue.add",
                params: [
                    "format": "png", "output": .string(into.appending(path: "frame.png").path(percentEncoded: false)),
                    "resolution": "half",
                ]
            ),
        ])
        return movie
    }
}

extension Trait where Self == ConditionTrait {
    /// A test that sets a limit on the whole process, which every test running beside it would feel as its own
    /// writes failing: it runs only where asked for, alone (`CRAFTBRIDGE_WHOLE_PROCESS` in the test's
    /// environment, which `xcodebuild test` passes on only as `TEST_RUNNER_CRAFTBRIDGE_WHOLE_PROCESS`, with
    /// `-only-testing` naming it).
    fileprivate static var limitsTheWholeProcess: Self {
        enabled(
            if: ProcessInfo.processInfo.environment["CRAFTBRIDGE_WHOLE_PROCESS"] != nil,
            "only a run asked for, with nothing beside it, limits the whole process")
    }
}

/// Runs `body` under a limit of `bytes` on the size of any file this process writes, and puts the limit and
/// the handling of its signal back however `body` ends, so no test after it inherits either.
@MainActor
private func underFileSizeLimit<T>(of bytes: rlim_t, _ body: () async throws -> T) async rethrows -> T {
    var before = rlimit()
    getrlimit(RLIMIT_FSIZE, &before)
    // The write that crosses the limit raises a signal that would end the process; ignored, it fails.
    let handling = signal(SIGXFSZ, SIG_IGN)
    var limit = rlimit(rlim_cur: bytes, rlim_max: before.rlim_max)
    setrlimit(RLIMIT_FSIZE, &limit)
    defer {
        setrlimit(RLIMIT_FSIZE, &before)
        signal(SIGXFSZ, handling)
    }
    return try await body()
}

@Suite(.serialized) @MainActor struct FailedExportTests {
    @Test
    func anExportThatFailsPartWayLeavesNothingButWhatWasAlreadyThere() async throws {
        let (engine, container, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: container) }
        guard let failing = try await Failing(on: engine, in: container) else {
            try await shutDown()
            return
        }
        defer { failing.release() }

        let seen = OSAllocatedUnfairLock(initialState: Set<String>())
        let names: @Sendable () -> [String] = {
            (try? FileManager.default.contentsOfDirectory(atPath: failing.into.path(percentEncoded: false))) ?? []
        }
        let export = try await engine.export(
            to: failing.into.appending(path: "frame.png"), format: .png,
            progress: { _, _, _ in seen.withLock { $0.formUnion(names()) } })
        let ending = try await Ending(export)

        guard case .CommandFailed? = ending.failure else {
            Issue.record("the render's own failure was expected at the blocked picture, not \(ending)")
            try await shutDown()
            return
        }
        #expect(
            seen.withLock { $0 }.contains { $0 != blockedPicture },
            "the render had written nothing before it failed, so it proves nothing about what a failure leaves")
        #expect(failing.names == [blockedPicture], "the failed export left a file of its own: \(failing.names)")
        #expect(try Data(contentsOf: failing.blocked) == earlierPicture, "the file that was there changed")
        #expect(
            try await engine.request("info", [:]).value(at: "/engine") == "effectcraft",
            "the engine answers after the failed export")
        try await shutDown()
    }

    /// A queue that fails at its second item keeps the movie of the first, which the queue reports as done
    /// at that path, and leaves none of the sequence the second began.
    @Test
    func aQueueThatFailsKeepsTheMovieOfTheItemThatFinished() async throws {
        let (engine, container, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: container) }
        guard let failing = try await Failing(on: engine, in: container) else {
            try await shutDown()
            return
        }
        defer { failing.release() }
        let movie = try await failing.queueMovieThenSequence(on: engine)

        let ending = try await Ending(try await engine.job("export", ["queue": true]))

        guard case .CommandFailed? = ending.failure else {
            Issue.record("the render's own failure was expected at the blocked picture, not \(ending)")
            try await shutDown()
            return
        }
        #expect(
            failing.names.sorted() == ["first.mp4", blockedPicture],
            "a failed queue lost the movie that finished, or left a file of the item that failed: \(failing.names)")
        #expect(
            try Data(contentsOf: movie).count > 0, "the finished item's movie is empty")
        #expect(try Data(contentsOf: failing.blocked) == earlierPicture, "the file that was there changed")
        try await shutDown()
    }

    /// A queue stopped while its second item renders keeps the movie of the first and leaves none of the
    /// second's pictures, and the caller hears `Cancelled`.
    @Test
    func aQueueThatIsStoppedKeepsTheMovieOfTheItemThatFinished() async throws {
        let (engine, container, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: container) }
        guard let failing = try await Failing(on: engine, in: container, seconds: 600, blocking: false) else {
            try await shutDown()
            return
        }
        let movie = try await failing.queueMovieThenSequence(on: engine)

        let running = OSAllocatedUnfairLock<ExportJob?>(initialState: nil)
        let folder = failing.into.path(percentEncoded: false)
        let job = try await engine.job(
            "export", ["queue": true],
            progress: { _, _, _ in
                let names = (try? FileManager.default.contentsOfDirectory(atPath: folder)) ?? []
                if names.contains(where: { $0.hasPrefix("frame_") }) {
                    running.withLock { $0 }?.cancel()
                }
            })
        running.withLock { $0 = job }
        let ending = try await Ending(job)

        guard case .Cancelled? = ending.failure else {
            Issue.record("the stop was expected to answer Cancelled, not \(ending)")
            try await shutDown()
            return
        }
        #expect(
            failing.names == ["first.mp4"],
            "a stopped queue lost the movie that finished, or left a file of the item it stopped: \(failing.names)")
        #expect(try Data(contentsOf: movie).count > 0, "the finished item's movie is empty")
        try await shutDown()
    }

    /// A movie the disk stops part way, by a limit on the size of a file the process may write, which makes
    /// the write that crosses it fail as a full disk does: the render fails after it wrote part of the
    /// movie, and nothing is left at its path.
    @Test(.limitsTheWholeProcess)
    func aMovieTheDiskStopsPartWayLeavesNoFile() async throws {
        let (engine, container, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: container) }
        _ = try await engine.run([
            (
                command: "comp.new",
                params: ["name": "Stopped", "width": 1280, "height": 720, "duration": 20, "frameRate": 10]
            )
        ])
        let movie = container.appending(path: "stopped.mp4")
        let written = OSAllocatedUnfairLock(initialState: 0)
        let path = movie.path(percentEncoded: false)
        let ending = try await underFileSizeLimit(of: 16 * 1024) {
            let job = try await engine.export(
                to: movie, format: .h264,
                progress: { _, _, _ in
                    let size = (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int) ?? 0
                    written.withLock { $0 = max($0, size) }
                },
                extra: ["hardware": true])
            return try await Ending(job)
        }

        #expect(ending.failure != nil, "the movie finished although the disk stopped it: \(ending)")
        #expect(written.withLock { $0 } > 0, "the render had written nothing before it failed, so it proves nothing")
        #expect(!FileManager.default.fileExists(atPath: path), "the failed export left its movie")
        let encoding = try await Encoding(of: engine)
        print("movie stopped: \(encoding), wrote \(written.withLock { $0 }) bytes, ended \(ending)")
        try await shutDown()
    }
}
