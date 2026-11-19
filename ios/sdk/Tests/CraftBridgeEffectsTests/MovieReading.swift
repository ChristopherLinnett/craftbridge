// What AVFoundation, the system's own reader, gives of a movie a test exported: every picture and every sample of
// its sound, read to the end, with where the sound's tone starts and where a white flash is.

import AVFoundation
import CoreVideo
import Testing

/// What AVFoundation reads of a movie, track by track.
struct Reading {
    var pictures = 0
    var lastPicture = CMTime.zero
    var sound = 0
    var loudest = 0
    /// The first sample of the sound past ``loud``, counted from the first the reader gave.
    var soundOnset: Int?
    var soundStarts: CMTime?
    var soundEnds = CMTime.zero
    /// The pictures, counted from 0, whose centre pixel is white: where a white flash is.
    var whiteFrames: [Int] = []
    var status = AVAssetReader.Status.unknown
}

/// Reads every sample of both tracks of `asset` as the system decodes it, to the end.
func read(_ asset: AVURLAsset) async throws -> Reading {
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
        if let image = CMSampleBufferGetImageBuffer(buffer) {
            let centre = pixel(of: image, x: CVPixelBufferGetWidth(image) / 2, y: CVPixelBufferGetHeight(image) / 2)
            // Blue, green and red all near full: the card's own colours leave one of them low.
            if centre.count == 4, centre.prefix(3).allSatisfy({ $0 > 200 }) { seen.whiteFrames.append(seen.pictures) }
        }
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
