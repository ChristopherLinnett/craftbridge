// The validation spine of plan §27 on the acceptance iPhone, at the running example's own size: the card
// template opened and filled, exported as a 1080 by 1080 H.264 movie of 180 frames on the hardware encoder,
// an export cancelled half-way, and the export made again with the app in the background.

import AVFoundation
import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import UIKit
import os

@Suite(.serialized) @MainActor struct RealAppSpineTests {
    /// The length of a movie's picture track, in seconds.
    private func seconds(of movie: URL) async throws -> Double {
        let track = try #require(try await AVURLAsset(url: movie).loadTracks(withMediaType: .video).first)
        return try await track.load(.timeRange).duration.seconds
    }

    @Test(.needsTheDeviceRunner)
    func theCardIsExportedCancelledAndExportedAgainInTheBackgroundAtTheRunningExamplesSize() async throws {
        let (engine, folder, shutDown) = try await startedEngine(gpu: .preferred)
        defer { try? FileManager.default.removeItem(at: folder) }
        let template = try await makeCardTemplate(engine, in: folder, width: 1080, height: 1080)
        let sam = folder.appending(path: "sam.png")
        try picture(.green, to: sam)

        let card = try await engine.templates.open(template)
        try await card.set("Name", "Sam")
        try await card.set("Photo", media: sam)
        let shot = try await engine.frame(comp: card.comp, at: 2.0, format: .png)
        #expect((shot.width, shot.height) == (1080, 1080), "the frame at 2 s is the card's size")

        let whole = folder.appending(path: "card.mp4")
        let reports = OSAllocatedUnfairLock<UInt64>(initialState: 0)
        _ = try await engine.export(
            card.comp, to: whole, format: .h264, progress: { _, total, _ in reports.withLock { $0 = total } },
            extra: ["hardware": true]
        ).result
        #expect(reports.withLock { $0 } == 180, "the export counts the running example's 180 frames")
        #expect(abs(try await seconds(of: whole) - 6) < 0.1, "the movie lasts 6 s")

        let halted = folder.appending(path: "halted.mp4")
        let cancelled = OSAllocatedUnfairLock<ExportJob?>(initialState: nil)
        let stopping = try await engine.export(
            card.comp, to: halted, format: .h264,
            progress: { done, total, _ in if total > 0, done * 2 >= total { cancelled.withLock { $0 }?.cancel() } },
            extra: ["hardware": true]
        )
        cancelled.withLock { $0 = stopping }
        await #expect(throws: CraftError.Cancelled) { try await stopping.result }
        #expect(
            !FileManager.default.fileExists(atPath: halted.path(percentEncoded: false)),
            "the cancelled export left no movie")

        let again = folder.appending(path: "again.mp4")
        let trip = BackgroundTrip(seconds: 8)
        let headway = Headway()
        let job = try await engine.export(
            card.comp, to: again, format: .h264, progress: headway.handler, extra: ["hardware": true])
        trip.ask()
        let ending = try await Ending(job)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let carried = trip.framesCarried(by: headway)
        print("the spine's export across the trip: \(ending), \(carried) frames still to make when the app left")
        if ending.failure == nil {
            #expect(carried > 0, "the export was still rendering once the app had left the foreground")
            #expect(abs(try await seconds(of: again) - 6) < 0.1, "the movie the background export finished lasts 6 s")
        } else {
            #expect(trip.endedAway(ending), "the export failed once the app had left the foreground")
            #expect(
                !FileManager.default.fileExists(atPath: again.path(percentEncoded: false)),
                "a failed export left no movie")
        }
        try await shutDown()
    }
}
