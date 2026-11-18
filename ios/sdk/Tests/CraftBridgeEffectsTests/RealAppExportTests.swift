// An export carried into the background on the acceptance iPhone, which no fake host can stand in for: the app
// leaves the foreground while it renders, and what the system then does to it is the system's. Each export ends
// in a `CraftError` or finishes, never a crash; the background task's expiry cancels it before the system stops
// the app; and a GPU export is finished or cancelled with no GPU work submitted once Metal refuses it.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import UIKit

/// The export of the active composition as a sequence of PNGs into a new folder of `folder`.
@MainActor
private func exportingPictures(
    of engine: EffectsEngine,
    into folder: URL,
    progress: @escaping ProgressHandler = { _, _, _ in }
) async throws -> (folder: URL, export: ExportJob) {
    let pictures = folder.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: pictures, withIntermediateDirectories: true)
    return (
        pictures,
        try await engine.export(to: pictures.appending(path: "card.png"), format: .png, progress: progress)
    )
}

/// A composition of `seconds` at 1280 by 720 and 30 frames a second, the engine's active one.
private func addComp(of seconds: Int, to engine: EffectsEngine) async throws {
    _ = try await engine.run([
        (
            command: "comp.new",
            params: [
                "name": "Card", "width": 1280, "height": 720, "duration": .number(Double(seconds)), "frameRate": 30,
            ]
        ),
        (command: "layer.newSolid", params: ["comp": "Card", "name": "Background"]),
    ])
}

@Suite(.serialized) @MainActor struct RealAppExportTests {
    @Test(.needsTheDeviceRunner)
    func anExportRunningAsTheAppGoesToTheBackgroundFinishesOrEndsInACraftErrorAndTheEngineAnswersAfterIt() async throws
    {
        let (engine, folder, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: folder) }
        try await addComp(of: 20, to: engine)
        let headway = Headway()
        let (pictures, export) = try await exportingPictures(of: engine, into: folder, progress: headway.handler)
        let trip = BackgroundTrip(seconds: 8)
        trip.ask()
        let ending = try await Ending(export)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let carried = trip.framesCarried(by: headway)
        print("the export across the trip: \(ending), \(carried) frames still to make when the app left")
        #expect(carried > 0, "the export was still rendering once the app had left the foreground")
        let written = try FileManager.default.contentsOfDirectory(atPath: pictures.path(percentEncoded: false)).count
        if ending.failure == nil {
            #expect(written == 600, "a finished export wrote every picture: \(written)")
        }
        #expect(
            try await engine.request("info", [:]).value(at: "/engine") == "effectcraft",
            "the engine answers after the trip")
        try await shutDown()
    }

    @Test(.needsTheDeviceRunner)
    func theSystemsExpiryOfTheBackgroundTaskCancelsTheExportBeforeTheAppIsStopped() async throws {
        let host = RecordingHost()
        let (engine, folder, shutDown) = try await startedEngine(host: host)
        defer { try? FileManager.default.removeItem(at: folder) }
        // One movie of more frames than the app renders in the background before its time is up, so only
        // the cancel ends it, and one file, so the export is rendering when the warning comes: an export of
        // thousands of files can spend that long before its first frame.
        try await addComp(of: 200, to: engine)
        let headway = Headway()
        let export = try await engine.export(
            to: folder.appending(path: "long.mp4"), format: .h264, progress: headway.handler)
        let trip = BackgroundTrip(seconds: 75)
        trip.ask()
        let ending = try await Ending(export)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let carried = trip.framesCarried(by: headway)
        #expect(carried > 0, "the export was still rendering once the app had left the foreground")
        let (began, expired, ended) = (host.time(of: "began"), host.time(of: "expired"), host.time(of: "ended"))
        print("the task's events: \(host.events.map { "\($0.what) at \($0.at.timeIntervalSince(began ?? $0.at))" })")

        #expect(began != nil, "the export ran under a background task")
        #expect(expired != nil, "the system told the app its time was up")
        #expect(ending.failure == .Cancelled, "the expiry cancelled the export: \(ending)")
        #expect(export.isCancelled)
        if let expired, let ended {
            let done = headway.done(at: expired)
            print("the export had made \(done) frames when the system's warning came")
            #expect(done > 0, "the export was rendering when the system's warning came")
            #expect(ended >= expired, "the task ended after the system's warning")
            #expect(
                ended.timeIntervalSince(expired) < 10,
                "the task ended \(ended.timeIntervalSince(expired)) s after the warning")
        } else {
            Issue.record("the task never ended")
        }
        try await shutDown()
    }

    @Test(.needsTheDeviceRunner)
    func aGpuExportCarriedIntoTheBackgroundFinishesWithEveryPictureOrIsCancelledAndTheGpuIsBackAfterIt() async throws {
        let (engine, folder, shutDown) = try await startedEngine(gpu: .preferred)
        defer { try? FileManager.default.removeItem(at: folder) }
        _ = try await engine.exec("file.openDemoProject")
        let device = try await adapter(of: engine)
        guard case .string = device else {
            Issue.record("the engine renders on the CPU, so no GPU export is carried into the background")
            return
        }
        let (kept, whole) = try await exportingPictures(of: engine, into: folder)
        _ = try await whole.result
        let reference = try fingerprints(ofFilesIn: kept)

        let trip = BackgroundTrip(seconds: 8)
        let headway = Headway()
        let (pictures, export) = try await exportingPictures(of: engine, into: folder, progress: headway.handler)
        trip.ask()
        let ending = try await Ending(export)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let carried = trip.framesCarried(by: headway)
        print(
            "the GPU export across the trip: \(ending), \(carried) frames still to make when the app left, "
                + "\(reference.count) pictures in the foreground")
        #expect(carried > 0, "the export was still rendering once the app had left the foreground")
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil, "the engine's hand-over across the trip failed")
        if let failure = ending.failure {
            #expect(
                failure == .Cancelled,
                "a GPU export that does not finish is cancelled, not refused by Metal: \(failure)")
        } else {
            #expect(try fingerprints(ofFilesIn: pictures) == reference, "every picture is the foreground's")
        }
        #expect(try await adapter(of: engine) == device, "the engine renders on the GPU it had before the trip")
        try await shutDown()
    }
}
