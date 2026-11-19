// The movie the Photos test saves for the owner to watch, made by the same function here, wherever the suite runs,
// and read back with AVFoundation: the flash is on the frames the tone begins in and on no others, and the tone
// begins where the pictures say, so what the owner judged on the phone is pinned without the phone.

import AVFoundation
import Testing

@Suite struct MovieToWatchTests {
    @Test func theMovieToWatchFlashesOnTheFramesItsToneBeginsInAndOnNoOthers() async throws {
        let (engine, folder, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: folder) }
        let movie = try await exportTheMovieToWatch(engine, in: folder)
        try await shutDown()

        let seen = try await read(AVURLAsset(url: movie))
        print("the movie to watch: flash on frames \(seen.whiteFrames), tone from sample \(seen.soundOnset ?? -1)")
        #expect(seen.status == .completed, "both tracks were read to their end: \(seen.status.rawValue)")
        #expect(seen.pictures == 180, "every picture was read: \(seen.pictures)")
        #expect(seen.sound == 288_000, "the sound reads \(seen.sound) samples of 288000")
        // The tone begins 1 s in, at 30 frames a second frame 30, and the flash lasts six frames; the numbers are
        // written out, so a flash or a tone that moves with the constants that place them is caught.
        #expect(seen.whiteFrames == Array(30...35), "the flash is on frames \(seen.whiteFrames)")
        // Sample 48,000 is 1 s at 48 kHz; the tone first rises past ``loud`` on its sixth sample.
        let shift = (seen.soundOnset ?? 0) - 48_005
        #expect(abs(shift) <= 4, "the tone starts \(shift) samples from 1 s, where the flash begins")
    }
}
