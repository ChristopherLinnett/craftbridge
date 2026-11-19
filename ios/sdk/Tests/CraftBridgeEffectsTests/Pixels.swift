// PNGs of straight 8-bit RGBA, written and read as the files hold them, and two of them held to a
// tolerance as the desktop runner holds them, in its words: the compositor's rule for a render on the GPU
// against the CPU's, which a phone can make against its own CPU's render.

import AVFoundation
import CraftBridge
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// A PNG holding `pixels`, straight RGBA of `width` by `height`, exactly as they are.
func png(straight pixels: Data, width: Int, height: Int) throws -> Data {
    let straight = CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue)
    let output = NSMutableData()
    guard let provider = CGDataProvider(data: pixels as CFData),
        let image = CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: straight, provider: provider, decode: nil,
            shouldInterpolate: false, intent: .defaultIntent),
        let destination = CGImageDestinationCreateWithData(output, UTType.png.identifier as CFString, 1, nil)
    else {
        throw CraftError.Internal(detail: "cannot encode the frame")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        throw CraftError.Internal(detail: "cannot encode the frame")
    }
    return output as Data
}

/// A PNG's pixels as the straight 8-bit RGBA the file holds, with its size. ImageIO hands an RGBA PNG over
/// premultiplied, which keeps a pixel's straight levels only where it is opaque, so a PNG with a translucent
/// pixel is refused rather than read with levels it does not hold; an opaque layout is given full alpha, and
/// any other layout is refused rather than converted, since a conversion would move the levels a tolerance
/// counts.
func straightRGBA(_ file: URL) throws -> (width: Int, height: Int, pixels: [UInt8]) {
    let name = file.path(percentEncoded: false)
    guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
        let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else {
        throw RunnerFailure(description: "cannot read \(name)")
    }
    let alpha = image.alphaInfo
    let channels = image.bitsPerPixel / 8
    let order = image.bitmapInfo.intersection(.byteOrderMask)
    let alphas: [CGImageAlphaInfo] = [.last, .premultipliedLast, .noneSkipLast]
    guard image.bitsPerComponent == 8, order == [] || order == .byteOrder32Big,
        (channels == 4 && alphas.contains(alpha)) || (channels == 3 && alpha == .none),
        let bytes = image.dataProvider?.data as Data?
    else {
        throw RunnerFailure(
            description: "cannot read \(name) as straight 8-bit RGBA: \(image.bitsPerPixel) bits a pixel, "
                + "alpha \(alpha.rawValue), byte order \(order.rawValue)")
    }
    var pixels: [UInt8] = []
    pixels.reserveCapacity(image.width * image.height * 4)
    for row in 0..<image.height {
        let start = row * image.bytesPerRow
        for column in 0..<image.width {
            let at = start + column * channels
            pixels += [
                bytes[at], bytes[at + 1], bytes[at + 2], alpha == .noneSkipLast || channels == 3 ? 255 : bytes[at + 3],
            ]
        }
    }
    if alpha == .premultipliedLast, pixels.enumerated().contains(where: { $0.offset % 4 == 3 && $0.element != 255 }) {
        throw RunnerFailure(
            description: "cannot read \(name) as straight 8-bit RGBA: it holds translucent pixels, which ImageIO "
                + "hands over premultiplied")
    }
    return (image.width, image.height, pixels)
}

/// The failures of two PNGs' pixels, `actual` against `reference`, as the desktop runner words them: each
/// pixel is over when any channel is further than the tolerance's levels from the reference's, no more than
/// its fraction of the pixels may be over, and none may be further than its cap.
func pixels(_ actual: URL, against reference: URL, within tolerance: Tolerance) -> [String] {
    let (ours, theirs): ((width: Int, height: Int, pixels: [UInt8]), (width: Int, height: Int, pixels: [UInt8]))
    do {
        (ours, theirs) = (try straightRGBA(actual), try straightRGBA(reference))
    } catch {
        return ["\(error)"]
    }
    guard (ours.width, ours.height) == (theirs.width, theirs.height) else {
        return ["size \(ours.width)x\(ours.height) is not the reference's \(theirs.width)x\(theirs.height)"]
    }
    var (over, furthest, total) = (0, UInt8(0), 0)
    for start in stride(from: 0, to: ours.pixels.count, by: 4) {
        var worst = UInt8(0)
        for channel in start..<start + 4 {
            let (a, b) = (ours.pixels[channel], theirs.pixels[channel])
            worst = max(worst, a > b ? a - b : b - a)
        }
        furthest = max(furthest, worst)
        over += worst > tolerance.levels ? 1 : 0
        total += 1
    }
    let fraction = total == 0 ? 0 : Double(over) / Double(total)
    var failures: [String] = []
    if fraction > tolerance.allowedFraction {
        let (found, allowed) = (fraction * 100, tolerance.allowedFraction * 100)
        failures.append(
            "\(over) of \(total) pixels are more than \(tolerance.levels) levels from the reference's: "
                + "\(String(format: "%.3f", found))% where \(String(format: "%.3f", allowed))% is allowed")
    }
    if furthest > tolerance.cap {
        failures.append("a pixel is \(furthest) levels from the reference's, past the \(tolerance.cap) allowed")
    }
    return failures
}

/// How a test reads a movie's pictures: as stills, each made afresh from the key picture before it; as a user
/// scrubbing in a player does, one player seeking from each picture to the next with the decoder it kept; or as a
/// player shows each picture when it starts there, a new one for every picture.
enum MovieReading {
    case stills
    case scrubbing
    case playerAfresh
}

/// One picture of a movie as 32-bit pixels, with the place of the alpha byte in each.
private typealias Picture = (data: Data, alphaAt: Int)

/// The pictures of `file` read the way `reading` says, frame by frame of the 30 frames a second of the running
/// example.
private func reader(of file: URL, _ reading: MovieReading) async throws -> (Int) async throws -> Picture {
    switch reading {
    case .stills:
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: file))
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = .zero
        return { frame in
            let (image, _) = try await generator.image(at: CMTime(value: Int64(frame), timescale: 30))
            guard let data = image.dataProvider?.data as Data?, image.bitsPerPixel == 32 else {
                throw RunnerFailure(description: "cannot read frame \(frame) as 32-bit pixels")
            }
            let first: [CGImageAlphaInfo] = [.first, .noneSkipFirst, .premultipliedFirst]
            return (data, first.contains(image.alphaInfo) ? 0 : 3)
        }
    case .scrubbing:
        let output = AVPlayerItemVideoOutput(pixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA
        ])
        let item = AVPlayerItem(url: file)
        item.add(output)
        let player = AVPlayer(playerItem: item)
        for _ in 0..<500 where item.status != .readyToPlay {
            try await Task.sleep(for: .milliseconds(10))
        }
        guard item.status == .readyToPlay else {
            throw RunnerFailure(
                description: "the player never readied \(file.lastPathComponent): \(String(describing: item.error))")
        }
        return { frame in
            let time = CMTime(value: Int64(frame), timescale: 30)
            await player.seek(to: time, toleranceBefore: .zero, toleranceAfter: .zero)
            for _ in 0..<500 {
                if let buffer = output.copyPixelBuffer(forItemTime: time, itemTimeForDisplay: nil) {
                    CVPixelBufferLockBaseAddress(buffer, .readOnly)
                    defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
                    let (width, height) = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer))
                    let stride = CVPixelBufferGetBytesPerRow(buffer)
                    guard let base = CVPixelBufferGetBaseAddress(buffer) else { break }
                    let rows = Data(bytes: base, count: stride * height)
                    var data = Data(capacity: width * height * 4)
                    for row in 0..<height { data.append(rows[(row * stride)..<(row * stride + width * 4)]) }
                    return (data, 3)
                }
                try await Task.sleep(for: .milliseconds(10))
            }
            throw RunnerFailure(
                description: "the player gave no picture for frame \(frame) of \(file.lastPathComponent)")
        }
    case .playerAfresh:
        return { frame in try await reader(of: file, .scrubbing)(frame) }
    }
}

/// The lowest PSNR, in dB, between the pictures of two movies at `frames`, in that order, colour channels only:
/// the alpha a decoded picture carries is a constant that would raise it. `movie` is read as `reading` says, as a
/// user's player reads it, and `reference` the same way unless `referenceReading` names another.
func lowestPSNR(
    of movie: URL, against reference: URL, atFrames frames: [Int], reading: MovieReading = .stills,
    referenceReading: MovieReading? = nil
) async throws -> Double {
    let actual = try await reader(of: movie, reading)
    let expected = try await reader(of: reference, referenceReading ?? reading)
    var lowest = Double.infinity
    for frame in frames {
        let (ours, theirs) = (try await actual(frame), try await expected(frame))
        guard ours.data.count == theirs.data.count, ours.alphaAt == theirs.alphaAt else {
            throw RunnerFailure(description: "frame \(frame) of the two movies differs in size or layout")
        }
        var squares = 0.0
        for (at, (a, b)) in zip(ours.data, theirs.data).enumerated() where at % 4 != ours.alphaAt {
            let difference = Double(a) - Double(b)
            squares += difference * difference
        }
        let error = squares / Double(ours.data.count / 4 * 3)
        lowest = min(lowest, error == 0 ? .infinity : 10 * log10(255 * 255 / error))
    }
    return lowest
}
