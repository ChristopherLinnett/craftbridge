// A frame's bytes copied into a pixel buffer: the one place the bytes meet a buffer's memory.

import CoreVideo
import CraftBridgeFFI
import Foundation

extension FrameBytes {
    /// The frame as a `CVPixelBuffer` of `kCVPixelFormatType_32BGRA`, premultiplied, tagged sRGB: the
    /// pixels are the core's own `Bgra8Premultiplied` conversion of the engine's straight colour, byte for
    /// byte. The buffer is IOSurface-backed and Metal-compatible, so it goes to a `CIImage`, an
    /// `AVAssetWriter` or a Metal texture without a copy.
    ///
    /// - Throws: ``CraftError/InvalidParams(detail:)`` for a frame not asked for as
    ///   ``PixelFormat/bgra8Premultiplied`` or whose bytes are not its size, and ``CraftError/Internal(detail:)``
    ///   when the system cannot allocate the buffer.
    public func pixelBuffer() throws -> CVPixelBuffer {
        let (width, height) = try premultipliedLayout()
        let options: [CFString: Any] = [
            kCVPixelBufferIOSurfacePropertiesKey: [String: Any](),
            kCVPixelBufferMetalCompatibilityKey: true,
        ]
        var made: CVPixelBuffer?
        let created = CVPixelBufferCreate(
            kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA, options as CFDictionary, &made)
        guard created == kCVReturnSuccess, let buffer = made else {
            throw CraftError.Internal(
                detail: "the system could not make a \(width) by \(height) pixel buffer (CVReturn \(created))")
        }
        guard CVPixelBufferLockBaseAddress(buffer, []) == kCVReturnSuccess,
            let base = CVPixelBufferGetBaseAddress(buffer)
        else {
            throw CraftError.Internal(detail: "the system gave no memory for the pixel buffer")
        }
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        // A buffer pads each row to what its hardware wants, so the rows are copied one at a time.
        let stride = CVPixelBufferGetBytesPerRow(buffer)
        let row = width * Self.bytesPerPixel
        data.withUnsafeBytes { bytes in
            guard let source = bytes.baseAddress else { return }
            for line in 0..<height {
                (base + line * stride).copyMemory(from: source + line * row, byteCount: row)
            }
        }
        CVBufferSetAttachment(buffer, kCVImageBufferCGColorSpaceKey, Self.colorSpace, .shouldPropagate)
        return buffer
    }
}
