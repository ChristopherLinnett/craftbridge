// PNGs of straight 8-bit RGBA, written and read as the files hold them, and two of them held to a
// tolerance as the desktop runner holds them, in its words: the compositor's rule for a render on the GPU
// against the CPU's, which a phone can make against its own CPU's render.

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
