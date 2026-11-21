// The export's start on the acceptance iPhone: the listing of every file an export may write, which for
// a sequence of thousands of files is its longest step before the first frame. A start that reports
// nothing through it cannot be reached by a cancel, neither an app's nor the system's when a background
// task's time runs out, and the system ends the export instead; so the start is held to reporting at
// once, before the journal looks at a single file, and to a cancel that stops the export's work and
// leaves none of its files behind. What the cancel owes is that work, not a number of seconds: seconds
// are what a loaded runner spends however it likes, and a test bound on them fails on the runner, not
// on the code.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import os

/// The files of the acceptance's export, at the least the engine can draw: the count is what the start
/// is measured at, so the frames stay the smallest the engine will make a file of.
private let files: UInt64 = 18_000

/// The seconds every report of this export stands still, through the seam this test owns: the export
/// waits on every report it takes, so the pause slows the export's own start — the whole suite's slowness
/// under load, given to this export alone. What the test asserts is what the export does, never how fast
/// the machine was while it did it.
private let reportPause: TimeInterval = 0.5

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
    private let asked = OSAllocatedUnfairLock<Report?>(initialState: nil)
    private let readBy = OSAllocatedUnfairLock<Int?>(initialState: nil)

    /// When the export started, as its reports count from it.
    var start: Date { started }

    /// The first report, or nil when none came.
    func first() -> Report? { kept.withLock { $0.first } }

    /// The cancel, as seconds since the export started, or nil when no report came in time to ask it.
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

    /// The handler to give the export. Each report ends by standing still for ``reportPause``: the export
    /// is mid-report until the handler is done, so the pause runs the export's own steps slow, as a
    /// loaded runner runs them slow, through this test's seam — and what the test reads is the export's
    /// work, which no such slowness moves. The pause also holds the listing's own beats half a second
    /// apart at the least, and the acceptance's files make three of the listing's chunks, a beat before
    /// each, so its third beat comes a second or more after the start on any machine: the cancel a second
    /// in asks on one of the listing's beats, the third on a fast machine and an earlier one on a slower.
    var handler: ProgressHandler {
        { [started, kept, job, asked, readBy] done, total, _ in
            let now = Date()
            let report = Report(at: now, done: done, total: total)
            let count = kept.withLock { kept in
                kept.append(report)
                return kept.count
            }
            let ask = asked.withLock { asked in
                guard asked == nil, now.timeIntervalSince(started) >= 1 else { return false }
                asked = report
                return true
            }
            if ask, let export = job.withLock({ $0 }) {
                export.cancel()
                readBy.withLock { $0 = min($0 ?? count, count) }
            }
            print("export of \(files) files: \(done) of \(total) at \(now.timeIntervalSince(started)) s")
            fflush(stdout)
            Thread.sleep(forTimeInterval: reportPause)
        }
    }

    /// Hands the export to the listener, and asks any cancel a report had already called for: the export
    /// reads that one at its next report.
    func take(_ export: ExportJob) {
        job.withLock { $0 = export }
        guard asked.withLock({ $0 != nil }) else { return }
        export.cancel()
        let next = count() + 1
        readBy.withLock { $0 = min($0 ?? next, next) }
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
        let said = first.map { "\($0.at.timeIntervalSince(heard.start)) s, \($0.done) of \($0.total)" } ?? "none"
        print("first report \(said), \(heard.count()) reports came")
        print("cancel asked \(String(describing: asked)) s, answered \(ended) s")
        fflush(stdout)

        guard let first else {
            Issue.record("an export of \(files) files reported nothing at all before it ended")
            return
        }
        // The first report counts nothing: the export spoke before its render had counted a frame. A
        // render's own first reports, before its first frame, count nothing too, so whether the export
        // spoke from its start, the listing, which reports before the journal looks at a single file, is
        // told by the report that asked the cancel, below.
        #expect(
            first.done == 0 && first.total == 0,
            "the first report came from the render, \(first.done) of \(first.total), not from the export's start")
        #expect(ending.failure == .Cancelled, "the cancel ended the export: \(ending)")
        guard let asked else {
            Issue.record("no report came in time to cancel the export")
            return
        }
        // The cancel reached the export inside its start: the report that asked it counted no frames to
        // draw, as the listing's do. A render counts its item's from its first frame, which on a
        // developer's Mac it has drawn well inside a second, so there a start that cannot be reached is
        // asked from the render and fails here. An export that cannot be stopped until its render is under
        // way is what an app's cancel, and the system's when a background task's time runs out, must never
        // wait out.
        let askedFrom = heard.askedFrom() ?? files
        #expect(
            askedFrom == 0,
            "the cancel was asked in a report of the render, counting \(askedFrom) frames to draw: the export's start could not be reached"
        )
        // What the cancel owes is the export's work stopping, not the answer hurrying: the export stops
        // at the report that reads the cancel and makes none after it, and one that went on listing or
        // rendering despite the cancel went on reporting.
        let lastReport = try #require(heard.lastReport(), "the cancel asked \(asked) s in never reached the export")
        #expect(
            heard.count() <= lastReport,
            "the export made \(heard.count() - lastReport) reports after the one that read its cancel: the cancel did not stop its work"
        )
        let left = (try? FileManager.default.contentsOfDirectory(atPath: into.path(percentEncoded: false))) ?? []
        #expect(left.isEmpty, "the cancelled export left \(left.count) of its \(files) files behind")
        #expect(
            try await engine.request("info", [:]).value(at: "/engine") == "effectcraft",
            "the engine answers after the cancelled export")
        try await shutDown()
    }
}
