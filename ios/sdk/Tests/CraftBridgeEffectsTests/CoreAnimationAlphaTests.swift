// What Core Animation shows of a translucent frame the engine drew on a `CAMetalLayer`: a half-transparent
// red over blue must come out as the blend of the two, as Core Animation itself composes a premultiplied
// image of the same red there. The core configures the layer with `PostMultiplied` alpha, which Core
// Animation reads as premultiplied, so the core has to write the frame premultiplied.

import CoreGraphics
import CraftBridge
import CraftBridgeFFI
import Foundation
import Metal
import QuartzCore
import Testing

@Suite @MainActor struct CoreAnimationAlphaTests {
    private static let side = 16

    /// The middle pixel (blue, green, red, alpha) of Core Animation's own composition of `content` over
    /// opaque blue, rendered offscreen into a texture.
    private func middle(of content: CALayer) async throws -> [UInt8] {
        let side = Self.side
        let description = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .bgra8Unorm, width: side, height: side, mipmapped: false)
        description.usage = [.renderTarget, .shaderRead]
        let device = try #require(MTLCreateSystemDefaultDevice())
        let texture = try #require(device.makeTexture(descriptor: description))
        let root = CALayer()
        root.bounds = CGRect(x: 0, y: 0, width: side, height: side)
        root.anchorPoint = .zero
        root.backgroundColor = CGColor(red: 0, green: 0, blue: 1, alpha: 1)
        content.anchorPoint = .zero
        content.bounds = root.bounds
        root.addSublayer(content)
        // A presented drawable reaches the layer when the GPU has finished it, a moment after the call.
        try await Task.sleep(for: .milliseconds(500))
        CATransaction.flush()
        let renderer = CARenderer(mtlTexture: texture, options: nil)
        renderer.layer = root
        renderer.bounds = root.bounds
        // Each frame is rendered on the GPU a moment after the call, and the backdrop under the layer is
        // opaque: until the middle pixel is, nothing has been drawn yet, and a read of it would compare two
        // blanks, so frames are drawn until it is.
        for _ in 0..<100 {
            renderer.beginFrame(atTime: CACurrentMediaTime(), timeStamp: nil)
            renderer.addUpdate(renderer.bounds)
            renderer.render()
            renderer.endFrame()
            try await Task.sleep(for: .milliseconds(50))
            let middle = pixel(of: texture, x: side / 2, y: side / 2)
            if middle[3] == 255 { return middle }
        }
        Issue.record("Core Animation drew nothing into the texture")
        return []
    }

    /// A layer showing a premultiplied image of half-transparent red: the core's own conversion of the
    /// straight colour (255, 0, 0, 128), which is what a correct composition of the preview must equal.
    private func reference() throws -> CALayer {
        let pixels = [UInt8](repeating: 0, count: 4).enumerated().map {
            $0.offset == 2 || $0.offset == 3 ? UInt8(128) : 0
        }
        let frame = FrameBytes(
            width: 1, height: 1, format: .bgra8Premultiplied, data: Data(pixels))
        let layer = CALayer()
        layer.contents = try frame.cgImage()
        layer.magnificationFilter = .nearest
        return layer
    }

    @Test func aTranslucentFrameIsComposedOverTheAppsViewsAsATranslucentPixel() async throws {
        let engine = try startEffectsEngine(folders: .isolated(), gpu: .preferred)
        try await engine.veil("Veil", width: 64, height: 64, color: "#ff0000", opacity: 50)
        let layer = CAMetalLayer()
        layer.contentsScale = 1
        layer.bounds = CGRect(x: 0, y: 0, width: Self.side, height: Self.side)
        try await engine.attachMetalLayer(layer)
        try await engine.present(params: "{}")
        let shown = try await middle(of: layer)
        let expected = try await middle(of: reference())
        try #require(shown.count == 4 && expected.count == 4)
        // Both sides are Core Animation's own drawing, so a frame that drew the blue backdrop alone for each
        // would compare equal: the reference must show its half red over half the blue before the preview
        // is held to it.
        try #require(
            abs(Int(expected[2]) - 128) <= 8 && abs(Int(expected[0]) - 127) <= 8,
            "Core Animation drew the reference as \(expected), not half red over blue")
        // Straight colour read as premultiplied would be red 255 over blue 127 where blend gives red ~128.
        for (channel, name) in zip(0..<4, ["blue", "green", "red", "alpha"]) {
            #expect(
                abs(Int(shown[channel]) - Int(expected[channel])) <= 6,
                "\(name) is \(shown[channel]) where Core Animation composes \(expected[channel])")
        }
        try await engine.shutDown()
    }
}
