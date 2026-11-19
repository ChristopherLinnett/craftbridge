// The validation spine of plan §27 on the acceptance iPhone, at the running example's own size: the card
// template opened and filled, exported as a 1080 by 1080 H.264 movie of 180 frames on the hardware encoder,
// an export cancelled half-way, and the export made again with the app in the background, which the software
// encoder finishes when the system takes the hardware encoder's session, as a movie of the foreground's pictures,
// and a movie of HEVC, which it cannot, that ends in the error naming the lost session.

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
        let before = try await Encoding(of: engine)
        let trip = BackgroundTrip(seconds: 8)
        let headway = Headway()
        let job = try await engine.export(
            card.comp, to: again, format: .h264, progress: headway.handler, extra: ["hardware": true])
        trip.ask()
        let ending = try await Ending(job)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let carried = trip.framesCarried(by: headway)
        let after = try await Encoding(of: engine)
        print("the spine's export across the trip: \(ending), \(carried) frames still to make when the app left")
        print("the encoders across the trip: \(before) before, \(after) after")
        #expect(ending.failure == nil, "the export the system took the hardware encoder from finished: \(ending)")
        #expect(carried > 0, "the export was still rendering once the app had left the foreground")
        #expect(abs(try await seconds(of: again) - 6) < 0.1, "the movie the background export finished lasts 6 s")
        #expect(
            after.fallbacks == before.fallbacks + 1, "the hardware session was lost and the movie went on in software")
        let switched = try #require(after.lastFallbackFrame, "the software encoder made frames of the movie")
        #expect(
            (switched > 0) && (switched < 180)
                && after.hardwarePictures - before.hardwarePictures == switched
                && after.softwarePictures - before.softwarePictures == 180 - switched,
            "the hardware made the frames before \(switched) and the software the rest, each frame once")
        // The foreground's pictures and the switched movie's, where the movie is the hardware's, where the software
        // takes over, and every half second between: within D-12's 40 dB (plan §28).
        let frames = Set(Array(stride(from: 0, to: 180, by: 15)) + [switched - 1, switched, switched + 1])
            .filter { (0..<180).contains($0) }.sorted()
        let lowest = try await lowestPSNR(of: again, against: whole, atFrames: frames)
        print("the lowest PSNR of the pictures of the switched movie against the foreground's: \(lowest) dB")
        #expect(lowest >= 40, "the switched movie's pictures are the foreground's within the encoders' tolerance")
        // A user scrubbing back over the switch in a player, whose decoder has read the software's pictures, sees
        // the pictures before it as a player that starts there shows them: from the parameter sets they were made
        // with, not the software's.
        let scrubbed = try await lowestPSNR(
            of: again, against: again,
            atFrames: [switched, switched - 1, 0, switched + 1, 1].filter { (0..<180).contains($0) },
            reading: .scrubbing, referenceReading: .playerAfresh)
        print("the lowest PSNR of the switched movie's pictures scrubbed back over the switch: \(scrubbed) dB")
        #expect(
            scrubbed >= 60, "the pictures before the switch play back after the pictures after it as they play alone")
        try await shutDown()
    }

    @Test(.needsTheDeviceRunner)
    func anHevcExportWhoseHardwareSessionIsLostInTheBackgroundEndsInTheErrorNamingItAndLeavesNoMovie() async throws {
        let (engine, folder, shutDown) = try await startedEngine(gpu: .preferred)
        defer { try? FileManager.default.removeItem(at: folder) }
        let template = try await makeCardTemplate(engine, in: folder, width: 1080, height: 1080)
        let card = try await engine.templates.open(template)
        let movie = folder.appending(path: "card.mp4")
        let before = try await Encoding(of: engine)
        let trip = BackgroundTrip(seconds: 8)
        let headway = Headway()
        let job = try await engine.export(
            card.comp, to: movie, format: .hevc, progress: headway.handler, extra: ["hardware": true])
        trip.ask()
        let ending = try await Ending(job)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let carried = trip.framesCarried(by: headway)
        print("the HEVC export across the trip: \(ending), \(carried) frames still to make when the app left")
        #expect(carried > 0, "the export was still rendering once the app had left the foreground")
        guard case .CommandFailed(_, let detail, _)? = ending.failure else {
            Issue.record("the HEVC export ended \(ending), not in the error naming the lost session")
            return
        }
        #expect(
            detail.contains("the hardware video encoder's session was lost at frame ")
                && detail.hasSuffix(
                    "and the movie cannot go on in the software encoder: there is no software encoder for HEVC here"),
            "the error names the lost session: \(detail)")
        #expect(
            !FileManager.default.fileExists(atPath: movie.path(percentEncoded: false)), "a failed export left no movie")
        #expect(try await Encoding(of: engine).fallbacks == before.fallbacks, "no movie went on in software")
        try await shutDown()
    }
}
