// The export's start on the acceptance iPhone: the listing of every file an export may write, which for
// a sequence of thousands of files is its longest step before the first frame. A start that reports
// nothing through it cannot be reached by a cancel, neither an app's nor the system's when a background
// task's time runs out, and the system ends the export instead; so the start is held to reporting at
// once and to answering a cancel within seconds, with none of its files left behind.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import os

/// The bound on the first progress report, from an export's start: what an app waits before it can take
/// the export back. It is loose because a phone is a busy machine; the export reports before the journal
/// looks at a single file of it.
private let firstReportWithin: TimeInterval = 5

/// The bound on the export's answer, from the cancel. A cancel met in the listing has no render to
/// wind down and nothing of its own to take back; one that came after the listing has every file the
/// export had written to remove. The bound covers both, and is what an app gives a stop.
private let cancelledWithin: TimeInterval = 10

/// The files of the acceptance's export, at the least the engine can draw: the count is what the start
/// is measured at, so the frames stay the smallest the engine will make a file of.
private let files: UInt64 = 18_000

/// An export's progress reports, and the cancel it asks for a second into the export. The cancel comes
/// from a report, as an app's does: a start that cannot be reached until its first frame is what this
/// is there to find out, and a cancel held back for another thread to deliver could land in the render
/// and prove nothing about the start.
final class StartOfMany: Sendable {
    /// One report: when it came, and how far the export had come.
    struct Report: Sendable {
        let at: Date
        let done: UInt64
        let total: UInt64
    }

    private let started = Date()
    private let kept = OSAllocatedUnfairLock(initialState: [Report]())
    private let job = OSAllocatedUnfairLock<ExportJob?>(initialState: nil)
    private let asked = OSAllocatedUnfairLock<Date?>(initialState: nil)

    /// When the export started, as its reports count from it.
    var start: Date { started }

    /// The first report, as seconds since the export started, or nil when none came.
    func first() -> TimeInterval? {
        kept.withLock { $0.first?.at.timeIntervalSince(started) }
    }

    /// The cancel, as seconds since the export started, or nil when no report came in time to ask it.
    func cancel() -> TimeInterval? { asked.withLock { $0?.timeIntervalSince(started) } }

    /// How many reports came.
    func count() -> Int { kept.withLock { $0.count } }

    /// The handler to give the export.
    var handler: ProgressHandler {
        { [started, kept, job, asked] done, total, _ in
            let now = Date()
            kept.withLock { $0.append(Report(at: now, done: done, total: total)) }
            let ask = asked.withLock { asked in
                guard asked == nil, now.timeIntervalSince(started) >= 1 else { return false }
                asked = now
                return true
            }
            if ask { job.withLock { $0 }?.cancel() }
            print("export of \(files) files: \(done) of \(total) at \(now.timeIntervalSince(started)) s")
            fflush(stdout)
        }
    }

    /// Hands the export to the listener, and asks any cancel a report had already called for.
    func take(_ export: ExportJob) {
        job.withLock { $0 = export }
        if asked.withLock({ $0 != nil }) { export.cancel() }
    }
}

@Suite(.serialized) @MainActor struct ExportStartTests {
    // No trip is asked for: what is measured here is the export's own start, which the app is in the
    // foreground for, so the test runs on a simulator as it does on a phone.
    @Test
    func anExportOfThousandsOfFilesCancelledASecondInHasReportedAlreadyAndLeavesNothing() async throws {
        let (engine, container, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: container) }
        // Ten frames a second of the smallest picture the engine will make, so that every one of the
        // acceptance's files is a file and nothing else: the start is what is being measured.
        _ = try await engine.run([
            (
                command: "comp.new",
                params: [
                    "name": "Many", "width": 16, "height": 16, "duration": .number(Double(files) / 10), "frameRate": 10,
                ]
            )
        ])
        let into = container.appending(path: "many", directoryHint: .isDirectory)

        let heard = StartOfMany()
        let export = try await engine.export(
            to: into.appending(path: "frame.png"), format: .png, progress: heard.handler)
        heard.take(export)
        let ending = try await Ending(export)
        let (first, asked) = (heard.first(), heard.cancel())
        let ended = ending.at.timeIntervalSince(heard.start)
        print("the export of \(files) files: \(ending)")
        print(
            "first report \(String(describing: first)) s, cancel asked \(String(describing: asked)) s, answered \(ended) s, in \(heard.count()) reports"
        )
        fflush(stdout)

        guard let first else {
            Issue.record("an export of \(files) files reported nothing at all before it ended")
            return
        }
        #expect(
            first <= firstReportWithin,
            "the first report came \(first) s after the start, over the bound of \(firstReportWithin) s")
        #expect(ending.failure == .Cancelled, "the cancel ended the export: \(ending)")
        guard let asked else {
            Issue.record("no report came in time to cancel the export")
            return
        }
        #expect(
            ended - asked <= cancelledWithin,
            "the export was answered \(ended - asked) s after the cancel, over the bound of \(cancelledWithin) s")
        let left = (try? FileManager.default.contentsOfDirectory(atPath: into.path(percentEncoded: false))) ?? []
        #expect(left.isEmpty, "the cancelled export left \(left.count) of its \(files) files behind")
        #expect(
            try await engine.request("info", [:]).value(at: "/engine") == "effectcraft",
            "the engine answers after the cancelled export")
        try await shutDown()
    }
}
