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

/// `value` as `count` little-endian bytes.
private func littleEndian(_ value: Int, bytes count: Int) -> [UInt8] {
    (0..<count).map { UInt8(truncatingIfNeeded: value >> (8 * $0)) }
}

/// The sound's rate, in samples a second.
private let rate = 48_000

/// The sample at which the sound's tone first rises past ``loud``: its first second is silent, and the tone
/// rises from zero, so its sixth sample is the first past it.
private let onset = rate + 5

/// A level no sample of the silence, or of the AAC encoder's ringing before the tone, reaches.
private let loud = 3_000

/// A 48 kHz 16-bit stereo WAV of `seconds`: a second of silence, then a 440 Hz tone from zero, loud enough to
/// tell from silence.
private func tone(seconds: Int) -> Data {
    var samples: [UInt8] = []
    samples.reserveCapacity(seconds * rate * 4)
    for index in 0..<(seconds * rate) {
        let level = index < rate ? 0 : Int(sin(Double(index - rate) * 440 * 2 * .pi / Double(rate)) * 12_000)
        samples += littleEndian(level, bytes: 2) + littleEndian(level, bytes: 2)
    }
    let header: [UInt8] =
        Array("RIFF".utf8) + littleEndian(36 + samples.count, bytes: 4) + Array("WAVEfmt ".utf8)
        + littleEndian(16, bytes: 4) + littleEndian(1, bytes: 2) + littleEndian(2, bytes: 2)
        + littleEndian(rate, bytes: 4) + littleEndian(rate * 4, bytes: 4) + littleEndian(4, bytes: 2)
        + littleEndian(16, bytes: 2) + Array("data".utf8) + littleEndian(samples.count, bytes: 4)
    return Data(header + samples)
}

/// What AVFoundation reads of a movie, track by track.
private struct Reading {
    var pictures = 0
    var lastPicture = CMTime.zero
    var sound = 0
    var loudest = 0
    /// The first sample of the sound past ``loud``, counted from the first the reader gave.
    var soundOnset: Int?
    var soundStarts: CMTime?
    var soundEnds = CMTime.zero
    var status = AVAssetReader.Status.unknown
}

/// Reads every sample of both tracks of `asset` as the system decodes it, to the end.
private func read(_ asset: AVURLAsset) async throws -> Reading {
    let reader = try AVAssetReader(asset: asset)
    let pictures = AVAssetReaderTrackOutput(
        track: try #require(try await asset.loadTracks(withMediaType: .video).first),
        outputSettings: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
    let sound = AVAssetReaderTrackOutput(
        track: try #require(try await asset.loadTracks(withMediaType: .audio).first),
        outputSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM, AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false,
        ])
    reader.add(pictures)
    reader.add(sound)
    #expect(reader.startReading(), "the reader started: \(String(describing: reader.error))")
    var seen = Reading()
    while let buffer = pictures.copyNextSampleBuffer() {
        seen.pictures += 1
        seen.lastPicture = CMSampleBufferGetPresentationTimeStamp(buffer)
    }
    while let buffer = sound.copyNextSampleBuffer() {
        seen.soundStarts = seen.soundStarts ?? CMSampleBufferGetPresentationTimeStamp(buffer)
        seen.soundEnds = CMSampleBufferGetPresentationTimeStamp(buffer) + CMSampleBufferGetDuration(buffer)
        let data = bytes(of: buffer)
        for index in stride(from: 0, to: data.count - 1, by: 2) {
            let level = abs(Int(Int16(bitPattern: UInt16(data[index]) | UInt16(data[index + 1]) << 8)))
            seen.loudest = max(seen.loudest, level)
            if seen.soundOnset == nil, level > loud { seen.soundOnset = seen.sound + index / 4 }
        }
        seen.sound += CMSampleBufferGetNumSamples(buffer)
    }
    seen.status = reader.status
    return seen
}

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
