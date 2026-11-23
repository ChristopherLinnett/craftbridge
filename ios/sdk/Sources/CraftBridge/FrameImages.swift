// A rendered frame, and the pictures an iOS app draws with it. The frame's types are UniFFI's own, declared in
// the bindings module, which is no product: the names here are the ones a package outside this one writes
// them by, in a declaration of its own as well as where the compiler infers them.

import CoreGraphics
import CraftBridgeFFI
import Foundation

/// A rendered frame as an engine's `frame` call answers it: the size it was rendered at, in pixels, the
/// layout its bytes are in, and the bytes.
public typealias FrameBytes = CraftBridgeFFI.FrameBytes

/// The layout a `frame` call is asked for and answers its bytes in: a PNG file's bytes, or rows of 8-bit
/// pixels from the top row down with no padding, as straight RGBA or as premultiplied BGRA.
public typealias PixelFormat = CraftBridgeFFI.PixelFormat

extension FrameBytes {
    /// The bytes of one pixel of a ``PixelFormat/bgra8Premultiplied`` frame: blue, green, red, alpha.
    static let bytesPerPixel = 4

    /// The colours of a frame the core converted: sRGB, which is what the compositor's 8-bit output is in.
    static let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()

    /// The frame as a `CGImage`: 8 bits a channel, premultiplied alpha first, little-endian, which is the
    /// order the bytes of ``PixelFormat/bgra8Premultiplied`` are in, so the image shows them as they are
    /// with nothing converted or copied again by Core Graphics. The frame is asked for as
    /// ``PixelFormat/bgra8Premultiplied`` through the engine's `frame`; its bytes are the core's own
    /// conversion of the engine's straight colour, the ones ``pixelBuffer()`` holds.
    ///
    /// - Throws: ``CraftError/InvalidParams(detail:)`` for a frame not asked for as
    ///   ``PixelFormat/bgra8Premultiplied`` or whose bytes are not its size, and ``CraftError/Internal(detail:)``
    ///   when Core Graphics refuses the image.
    public func cgImage() throws -> CGImage {
        let (width, height) = try premultipliedLayout()
        let layout = CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedFirst.rawValue
        guard let provider = CGDataProvider(data: data as CFData),
            let image = CGImage(
                width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 8 * Self.bytesPerPixel,
                bytesPerRow: width * Self.bytesPerPixel, space: Self.colorSpace,
                bitmapInfo: CGBitmapInfo(rawValue: layout), provider: provider, decode: nil,
                shouldInterpolate: false, intent: .defaultIntent)
        else {
            throw CraftError.Internal(detail: "Core Graphics refused a \(width) by \(height) image of the frame")
        }
        return image
    }

    /// The size of a frame that is `Bgra8Premultiplied` rows with nothing missing and nothing over, in the
    /// whole pixels an image takes, or the refusal that says which of those it is not.
    func premultipliedLayout() throws -> (width: Int, height: Int) {
        guard format == .bgra8Premultiplied else {
            throw CraftError.InvalidParams(
                detail: "a frame as an image is asked for as bgra8Premultiplied; this one is \(format)")
        }
        let width = Int(self.width)
        let height = Int(self.height)
        let (pixels, overflowed) = width.multipliedReportingOverflow(by: height)
        let (size, sizeOverflowed) = pixels.multipliedReportingOverflow(by: Self.bytesPerPixel)
        guard width > 0, height > 0, !overflowed, !sizeOverflowed, size == data.count else {
            throw CraftError.InvalidParams(
                detail: "a \(self.width) by \(self.height) frame of bgra8Premultiplied is not \(data.count) bytes")
        }
        return (width, height)
    }
}
