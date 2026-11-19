// The running example's movie, saved to the acceptance iPhone's Photos library for a person to watch: only a
// person can judge a movie's sound against its pictures. The movie to watch (`exportTheMovieToWatch`) is the card
// at its own size (1080 by 1080, 6 seconds, 180 frames) with a sound of 6 seconds, through the hardware path, and a
// white flash from the frame the sound's tone begins in, so the eye has a picture to match the tone to; it is
// added to Photos with add-only access, where it stays. The test saves a movie and waits for an answer to the
// system's prompt, so it runs only when a device run's filter is exactly its name
// (`tools/ios-device-test.sh --swift -- --filter RealAppPhotosTests`), never in a default run or one whose filter
// only reaches it, and never anywhere that has no Photos library to add to.

import AVFoundation
import CraftBridge
import CraftBridgeEffects
import Foundation
@preconcurrency import Photos
import Testing
import os

/// The name a device run asks for to run this suite: the suite's own, so the two cannot part.
private let photosSuite = String(describing: RealAppPhotosTests.self)

/// How long the person has to answer the system's prompt for access to Photos, in seconds.
private let promptSeconds = 600

/// The movie's tracks and length as AVFoundation reads them.
private struct Movie {
    let pictureTracks: Int
    let soundTracks: Int
    let seconds: Double

    init(at url: URL) async throws {
        let asset = AVURLAsset(url: url)
        pictureTracks = try await asset.loadTracks(withMediaType: .video).count
        soundTracks = try await asset.loadTracks(withMediaType: .audio).count
        seconds = try await asset.load(.duration).seconds
    }
}

/// The answer to the prompt for add-only access to Photos, or nil when nobody answered within ``promptSeconds``.
private func access() async -> PHAuthorizationStatus? {
    let answer = OSAllocatedUnfairLock<PHAuthorizationStatus?>(initialState: nil)
    // The request cannot be cancelled, so it is left to end by itself when nobody answers.
    Task {
        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        answer.withLock { $0 = status }
    }
    _ = await within(seconds: promptSeconds) { answer.withLock { $0 } != nil }
    return answer.withLock { $0 }
}

/// Adds the movie at `url` to Photos and answers the local identifier of the asset it made. Outside the main
/// actor, since Photos runs the change on a queue of its own and the runtime refuses a closure that claims the
/// main actor there.
private func addToPhotos(_ url: URL) async throws -> String? {
    let saved = OSAllocatedUnfairLock<String?>(initialState: nil)
    try await PHPhotoLibrary.shared().performChanges {
        let request = PHAssetChangeRequest.creationRequestForAssetFromVideo(atFileURL: url)
        let identifier = request?.placeholderForCreatedAsset?.localIdentifier
        saved.withLock { $0 = identifier }
    }
    return saved.withLock { $0 }
}

@Suite(.serialized) @MainActor struct RealAppPhotosTests {
    @Test(.needsTheDeviceRunner(askedFor: photosSuite))
    func theRunningExamplesMovieWithSoundIsSavedToPhotosForAPersonToWatch() async throws {
        // The trait is what keeps this test out of the other runs. A test that prompts the owner and saves a
        // movie to their Photos must still stop if the trait is lost or swapped for one a default run passes, so
        // it asks the trait's own question again before anything else.
        try #require(deviceRunAsks(for: photosSuite), "only a device run that asks for it by name saves to Photos")
        let (engine, folder, shutDown) = try await startedEngine(gpu: .preferred)
        defer { try? FileManager.default.removeItem(at: folder) }
        let movie = try await exportTheMovieToWatch(engine, in: folder)
        try await shutDown()

        let made = try await Movie(at: movie)
        #expect(made.pictureTracks == 1 && made.soundTracks == 1, "the movie has one picture track and one sound track")
        #expect(abs(made.seconds - 6) < 0.1, "the movie lasts \(made.seconds) s")

        guard let status = await access() else {
            print("the prompt for access to Photos was not answered in \(promptSeconds) s: nothing was saved")
            return
        }
        #expect(
            status == .authorized || status == .limited,
            "the person allowed the test to add to Photos: the answer was \(status.rawValue)")
        guard status == .authorized || status == .limited else { return }
        let added = Date()
        let saved = try await addToPhotos(movie)
        let identifier = try #require(saved, "Photos named the asset it made")
        print(
            "the movie in Photos: asset \(identifier), \(made.seconds) s, \(made.pictureTracks) picture track and "
                + "\(made.soundTracks) sound track, saved at \(ISO8601DateFormatter().string(from: added))")
    }
}
