// `previewRequest` on the simulator, through a real engine: the request that fits a 1280 by 720 composition
// in a box, the composition's size, and the frame that request renders, read back by size and alpha.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import Testing

/// The containers of this suite's engines, removed when the suite ends.
private let requestFolders = "previewrequest"

/// An engine whose active composition is 1280 by 720 pixels and holds one half-opaque box in its middle, on a
/// ground that is nothing, so a frame of it has content and transparent pixels.
private func started() async throws -> EffectsEngine {
    let engine = try await EffectsEngine.start(folders: .isolated(requestFolders), gpu: .cpuOnly)
    _ = try await engine.run([
        (
            "comp.new",
            ["name": "Wide", "width": 1280, "height": 720, "duration": 2, "frameRate": 10]
        ),
        (
            "layer.newSolid",
            ["comp": 1, "name": "Box", "color": "#ff8040", "width": 320, "height": 180]
        ),
        ("prop.set", ["layer": "Box", "path": "transform/opacity", "value": 50]),
    ])
    return engine
}

/// The straight alpha of the pixel at (`x`, `y`) of `frame`.
private func alpha(of frame: FrameBytes, x: Int, y: Int) -> UInt8 {
    frame.data[(y * Int(frame.width) + x) * 4 + 3]
}

@Suite(Removing(name: requestFolders)) struct PreviewRequestTests {
    @Test func theFrameARequestHoldsHasTheFittedSizeAndTheCompositionsOwnAlpha() async throws {
        let engine = try await started()
        // The box, the request's text, and the frame's size: a box smaller than the composition, and one larger.
        let cases: [(side: Int, text: String, size: (width: UInt32, height: UInt32))] = [
            (640, #"{"maxSide":640,"time":1,"transparent":true}"#, (640, 360)),
            (4000, #"{"maxSide":1280,"time":1,"transparent":true}"#, (1280, 720)),
        ]
        for (side, text, size) in cases {
            let answer = try await engine.previewRequest(at: .seconds(1), width: side, height: side)
            #expect(answer.requestText == text, "a box of \(side) by \(side)")
            #expect(answer.contentWidth == 1280 && answer.contentHeight == 720)
            let frame = try await engine.frame(params: answer.request, format: .rgba8Straight)
            #expect((frame.width, frame.height) == size, "the frame a box of \(side) by \(side) gets")
            #expect(alpha(of: frame, x: 0, y: 0) == 0, "a corner no layer covers is transparent")
            #expect(alpha(of: frame, x: Int(frame.width) / 2, y: Int(frame.height) / 2) > 0, "the box is drawn")
            let fromText = try await engine.frameJSON(params: answer.requestText, format: .rgba8Straight)
            #expect(fromText == frame, "the request's text asks for the very frame the request does")
        }
        try await engine.shutDown()
    }

    @Test func theTextOfARequestIsTheEnginesWhicheverWayThisPlatformWritesItsTime() async throws {
        let engine = try await started()
        // Swift writes half a millisecond without an exponent and Kotlin with one; the text is the engine's.
        let answer = try await engine.previewRequest(at: .microseconds(500), width: 640, height: 640)
        #expect(answer.requestText == #"{"maxSide":640,"time":0.0005,"transparent":true}"#)
        try await engine.shutDown()
    }

    @Test func aBoxOfAnotherAspectIsMetByTheShorterSide() async throws {
        let engine = try await started()
        let answer = try await engine.previewRequest(.number(1), width: 500, height: 300)
        #expect(answer.requestText == #"{"comp":1,"maxSide":500,"transparent":true}"#)
        let frame = try await engine.frame(params: answer.request, format: .rgba8Straight)
        #expect((frame.width, frame.height) == (500, 281), "inside the box, in the composition's aspect")
        try await engine.shutDown()
    }

    @Test func aBoxWithASideBelowOneAndACompositionThatIsNotThereAreRefused() async throws {
        let engine = try await started()
        for (width, height, key) in [(0, 640, "width"), (640, 0, "height"), (-1, 640, "width")] {
            await #expect(
                throws: CraftError.InvalidParams(
                    detail: "`previewRequest`: `\(key)` must be a whole number from 1 to 4294967295")
            ) {
                try await engine.previewRequest(width: width, height: height)
            }
        }
        // The refusal of a missing composition is a frame's own.
        let framed = await refusal { try await engine.frame(params: ["comp": "nope", "scale": 0.5], format: .png) }
        let previewed = await refusal { try await engine.previewRequest("nope", width: 640, height: 640) }
        #expect(previewed == framed)
        guard case .NotFound? = previewed else {
            Issue.record("a composition that is not there is refused as NotFound, not \(String(describing: previewed))")
            return
        }
        try await engine.shutDown()
    }
}

/// What `call` refuses with, or nil when it does not.
private func refusal(_ call: () async throws -> some Any) async -> CraftError? {
    do {
        _ = try await call()
        return nil
    } catch {
        return error as? CraftError
    }
}
