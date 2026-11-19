// A whole movie with sound, exported on the phone and read back with AVFoundation, the system's own reader:
// the tracks' lengths, the sound and the pictures in step, and both tracks read to their end, the sound's
// AAC stream ending as the code that writes it takes it to end. The sound is a second of silence and then a
// tone, so where the tone starts in what AVFoundation reads says, to the sample, whether the sound is where
// the pictures are.

import AVFoundation
import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing

@Suite struct MovieTracksTests {
    @Test func aMovieWithSoundIsReadByAVFoundationTrackByTrackToItsEndWithSoundAndPicturesInStep() async throws {
        let (engine, folder, shutDown) = try await startedEngine()
        defer { try? FileManager.default.removeItem(at: folder) }
        let sound = folder.appending(path: "tone.wav")
        let movie = folder.appending(path: "show.mp4")
        try tone(seconds: 4).write(to: sound)
        _ = try await engine.run([
            (
                command: "comp.new",
                params: ["name": "Show", "width": 640, "height": 360, "duration": 4, "frameRate": 30]
            ),
            (command: "layer.newSolid", params: ["comp": "Show", "name": "Bg", "color": "#c84d21"]),
            (command: "file.import", params: ["paths": [.string(sound.path(percentEncoded: false))]]),
            (command: "layer.addItem", params: ["comp": "Show", "item": "tone.wav", "time": 0]),
        ])
        _ = try await engine.export("Show", to: movie, format: .h264, extra: ["hardware": true]).result
        try await shutDown()

        let asset = AVURLAsset(url: movie)
        let video = try await asset.loadTracks(withMediaType: .video)
        let audio = try await asset.loadTracks(withMediaType: .audio)
        #expect(video.count == 1, "the movie has one picture track")
        #expect(audio.count == 1, "the movie has one sound track")
        let (pictures, heard) = (
            try await #require(video.first).load(.timeRange), try await #require(audio.first).load(.timeRange)
        )
        // The sound's own 4.000 s, past the encoder's priming, which the track's edit names and its roll
        // sample group lets AVFoundation take from that edit instead of dropping twice.
        #expect(abs(pictures.duration.seconds - 4) < 0.05, "the pictures last \(pictures.duration.seconds) s")
        #expect(abs(heard.duration.seconds - 4) < 0.01, "the sound lasts \(heard.duration.seconds) s")
        #expect(abs(pictures.start.seconds - heard.start.seconds) < 0.01, "both tracks start together")
        #expect(abs(pictures.end.seconds - heard.end.seconds) < 0.01, "both tracks end together")

        let seen = try await read(asset)
        #expect(seen.status == .completed, "both tracks were read to their end: \(seen.status.rawValue)")
        #expect(seen.pictures == 120, "every picture was read: \(seen.pictures)")
        #expect(
            abs(seen.lastPicture.seconds - 4 + 1.0 / 30) < 0.05,
            "the last picture is the 120th: \(seen.lastPicture.seconds) s")
        #expect(seen.loudest > 4_000, "the sound is not silence: its loudest sample is \(seen.loudest)")
        #expect(seen.soundStarts == .zero, "the sound is read from time zero")
        print(
            "the movie: \(seen.pictures) pictures to \(seen.lastPicture.seconds) s, \(seen.sound) samples to \(seen.soundEnds.seconds) s, the tone from sample \(seen.soundOnset ?? -1), status \(seen.status.rawValue)"
        )
        // The read sound sits exactly where the pictures are: the priming taken from the edit as the roll
        // group says, and the sound ending with them rather than running to the encoder's last whole frame.
        let shift = (seen.soundOnset ?? 0) - onset
        #expect(abs(shift) <= 4, "the tone starts \(shift) samples from where it starts beside the pictures")
        #expect(seen.sound == 4 * rate, "the sound reads \(seen.sound) samples of \(4 * rate)")
        #expect(
            abs(seen.soundEnds.seconds - 4) < 0.01,
            "the sound ends at \(seen.soundEnds.seconds) s, with the pictures")
    }
}
