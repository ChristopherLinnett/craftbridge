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

/// The files of the acceptance's export, at the least the engine can draw: the count is what the start
/// is measured at, so the frames stay the smallest the engine will make a file of.
private let files: UInt64 = 18_000

/// The seconds into the export from which its reports ask the cancel: the first report at or after them
/// asks it. The seam's pause holds the listing's own beats a pause apart at the least, and the acceptance's
/// files make three of the listing's chunks, a beat before each: at the pause as it stands the third beat
/// comes a second or more after the start on any machine, so the cancel a second in asks on one of the
/// listing's beats — the third on a fast machine and an earlier one on a slower.
private let askAfter: TimeInterval = 1

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

        let heard = CancelFromReport(askingAfter: askAfter, printedAs: "export of \(files) files")
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
