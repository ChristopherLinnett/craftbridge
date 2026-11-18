// A frame as a `CVPixelBuffer` and a `CGImage`: the bytes the core converted, laid out as the system's
// types read them.

import CoreGraphics
import CoreVideo
import CraftBridge
import CraftBridgeFFI
import Foundation
import Testing

/// A frame of a translucent card in `format`: a ramp from black to white down a solid at 30% opacity, in a
/// 32 bpc project, whose pixels are not rounded to 8 bits before they are written, so they hold colours
/// between the 8-bit steps at an alpha that is no whole number of 255ths. There premultiplying the rounded
/// straight bytes, as the core does, and the compositor's own premultiplied write part by one for about a
/// fifth of the pixels, where an 8 bpc project's would agree. A width of 37 pixels is 148 bytes a row, which
/// a pixel buffer pads, so a copy that forgets the padding is read wrong.
private func translucentFrame(format: PixelFormat = .bgra8Premultiplied) async throws -> FrameBytes {
    let engine = try startEffectsEngine(folders: .isolated(), gpu: .cpuOnly)
    try await engine.exec([("file.projectSettings", ["bitDepth": "32"])])
    try await engine.veil("Veil", width: 37, height: 256, color: "#c84d21", opacity: 30)
    try await engine.exec([("effect.apply", ["comp": "Veil", "layer": "Veil", "effect": "Gradient Ramp"])])
    let frame = try await engine.frame(params: #"{"comp":"Veil","transparent":true}"#, format: format)
    try await engine.shutDown()
    return frame
}

/// The core's conversion as Apple makes it from the same card's straight bytes, never as the compositor
/// writes premultiplied colour itself.
private func applesConversion() async throws -> [UInt8] {
    let straight = try await translucentFrame(format: .rgba8Straight)
    return applePremultipliedBGRA(of: [UInt8](straight.data), width: Int(straight.width), height: Int(straight.height))
}

@Suite struct FrameImageTests {
    @Test func aPixelBufferHoldsTheCoresPremultipliedBytesAsTheyAre() async throws {
        let frame = try await translucentFrame()
        let alphas = Set(stride(from: 3, to: frame.data.count, by: 4).map { frame.data[$0] })
        #expect(alphas.contains { $0 > 0 && $0 < 255 }, "the frame is translucent, so premultiplying changes it")
        let buffer = try frame.pixelBuffer()
        #expect(CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA)
        #expect(CVPixelBufferGetWidth(buffer) == 37)
        #expect(CVPixelBufferGetHeight(buffer) == 256)
        #expect(CVPixelBufferGetBytesPerRow(buffer) > 37 * 4, "the rows are padded, so a copy has to go row by row")
        #expect(bytes(of: buffer) == [UInt8](frame.data), "the buffer's bytes are the frame's, nothing else")
        let apple = try await applesConversion()
        #expect(bytes(of: buffer) == apple, "and those are Apple's conversion of the straight bytes")
    }

    @Test func aPixelBufferIsIOSurfaceBackedMetalCompatibleAndTaggedSRGB() async throws {
        let buffer = try await translucentFrame().pixelBuffer()
        #expect(CVPixelBufferGetIOSurface(buffer) != nil, "an IOSurface goes to Core Image and AVFoundation as is")
        let made = CVPixelBufferCopyCreationAttributes(buffer) as? [String: Any]
        #expect(made?[kCVPixelBufferIOSurfacePropertiesKey as String] != nil, "the IOSurface was asked for")
        #expect(made?[kCVPixelBufferMetalCompatibilityKey as String] as? Bool == true, "a Metal texture can wrap it")
        let space = CVBufferCopyAttachment(buffer, kCVImageBufferCGColorSpaceKey, nil)
        #expect(space.map { $0 as! CGColorSpace }?.name == CGColorSpace.sRGB)
    }

    @Test func aCGImageShowsTheCoresPremultipliedBytesAsTheyAre() async throws {
        let frame = try await translucentFrame()
        let image = try frame.cgImage()
        #expect(image.width == 37 && image.height == 256)
        #expect(image.bitsPerComponent == 8 && image.bitsPerPixel == 32 && image.bytesPerRow == 37 * 4)
        #expect(image.alphaInfo == .premultipliedFirst)
        #expect(image.byteOrderInfo == .order32Little)
        #expect(image.colorSpace?.name == CGColorSpace.sRGB)
        #expect(drawn(image) == [UInt8](frame.data), "Core Graphics reads the bytes as the core wrote them")
        let apple = try await applesConversion()
        #expect(drawn(image) == apple, "which are Apple's conversion of the straight bytes")
    }

    @Test func aFrameNotAskedForAsPremultipliedBGRAIsRefused() async throws {
        let straight = try await translucentFrame(format: .rgba8Straight)
        let refusal = CraftError.InvalidParams(
            detail: "a frame as an image is asked for as bgra8Premultiplied; this one is rgba8Straight")
        #expect(throws: refusal) { try straight.pixelBuffer() }
        #expect(throws: refusal) { try straight.cgImage() }
    }

    @Test func aFrameWhoseBytesAreNotItsSizeIsRefused() {
        let short = FrameBytes(width: 2, height: 2, format: .bgra8Premultiplied, data: Data(count: 15))
        let refusal = CraftError.InvalidParams(detail: "a 2 by 2 frame of bgra8Premultiplied is not 15 bytes")
        #expect(throws: refusal) { try short.pixelBuffer() }
        #expect(throws: refusal) { try short.cgImage() }
        let long = FrameBytes(width: 2, height: 2, format: .bgra8Premultiplied, data: Data(count: 17))
        let over = CraftError.InvalidParams(detail: "a 2 by 2 frame of bgra8Premultiplied is not 17 bytes")
        #expect(throws: over) { try long.pixelBuffer() }
        #expect(throws: over) { try long.cgImage() }
        let empty = FrameBytes(width: 0, height: 0, format: .bgra8Premultiplied, data: Data())
        let nothing = CraftError.InvalidParams(detail: "a 0 by 0 frame of bgra8Premultiplied is not 0 bytes")
        #expect(throws: nothing) { try empty.pixelBuffer() }
    }
}
