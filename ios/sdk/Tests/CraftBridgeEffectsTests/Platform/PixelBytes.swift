// The tests' reads of memory a system call owns, a pixel buffer's rows or one of its pixels and a bitmap
// context's, as plain bytes, and Apple's own conversion of straight pixels, which writes into the bytes it is
// handed.

import Accelerate
import CoreGraphics
import CoreVideo
import Metal

/// `straight`, `width` by `height` pixels of straight red, green, blue and alpha, as Apple premultiplies them
/// (`vImagePremultiplyData_RGBA8888`) and orders them for a `kCVPixelFormatType_32BGRA` buffer
/// (`vImagePermuteChannels_ARGB8888`): what the core's own `Bgra8Premultiplied` conversion must write, made
/// by a hand other than the core's. Nothing when either call fails.
func applePremultipliedBGRA(of straight: [UInt8], width: Int, height: Int) -> [UInt8] {
    var pixels = straight
    let converted = pixels.withUnsafeMutableBytes { bytes -> Bool in
        guard let base = bytes.baseAddress else { return false }
        var buffer = vImage_Buffer(
            data: base, height: vImagePixelCount(height), width: vImagePixelCount(width), rowBytes: width * 4)
        let toBGRA: [UInt8] = [2, 1, 0, 3]
        return vImagePremultiplyData_RGBA8888(&buffer, &buffer, vImage_Flags(kvImageNoFlags)) == kvImageNoError
            && vImagePermuteChannels_ARGB8888(&buffer, &buffer, toBGRA, vImage_Flags(kvImageNoFlags)) == kvImageNoError
    }
    return converted ? pixels : []
}

/// The `width * 4` bytes of each of `buffer`'s `height` rows, with the padding that ends each row left out.
func bytes(of buffer: CVPixelBuffer) -> [UInt8] {
    CVPixelBufferLockBaseAddress(buffer, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
    guard let base = CVPixelBufferGetBaseAddress(buffer) else { return [] }
    let stride = CVPixelBufferGetBytesPerRow(buffer)
    let row = CVPixelBufferGetWidth(buffer) * 4
    var bytes: [UInt8] = []
    for line in 0..<CVPixelBufferGetHeight(buffer) {
        bytes += UnsafeRawBufferPointer(start: base + line * stride, count: row)
    }
    return bytes
}

/// The four bytes of the pixel at (`x`, `y`) of `buffer`, a buffer of four bytes a pixel, in its own order: one
/// pixel, where ``bytes(of:)`` copies every row. None when the buffer has no such pixel or no memory to read.
func pixel(of buffer: CVPixelBuffer, x: Int, y: Int) -> [UInt8] {
    CVPixelBufferLockBaseAddress(buffer, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
    guard (0..<CVPixelBufferGetWidth(buffer)) ~= x, (0..<CVPixelBufferGetHeight(buffer)) ~= y,
        let base = CVPixelBufferGetBaseAddress(buffer)
    else { return [] }
    return [UInt8](UnsafeRawBufferPointer(start: base + y * CVPixelBufferGetBytesPerRow(buffer) + x * 4, count: 4))
}

/// The bytes of `image` as a bitmap of the same layout holds them after Core Graphics drew it there: the
/// image's own pixels, read by the system rather than by whoever made the image.
func drawn(_ image: CGImage) -> [UInt8] {
    let layout = CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedFirst.rawValue
    guard
        let context = CGContext(
            data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4,
            space: image.colorSpace ?? CGColorSpaceCreateDeviceRGB(), bitmapInfo: layout),
        let base = context.data
    else { return [] }
    context.setBlendMode(.copy)
    context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    return [UInt8](UnsafeRawBufferPointer(start: base, count: image.width * image.height * 4))
}

/// The four bytes of the pixel at (`x`, `y`) of `texture`, in the texture's own order.
func pixel(of texture: MTLTexture, x: Int, y: Int) -> [UInt8] {
    var bytes = [UInt8](repeating: 0, count: 4)
    bytes.withUnsafeMutableBytes { into in
        guard let base = into.baseAddress else { return }
        texture.getBytes(base, bytesPerRow: 4, from: MTLRegionMake2D(x, y, 1, 1), mipmapLevel: 0)
    }
    return bytes
}
