// A preview into the app's `CAMetalLayer`, on the simulator's real Metal device: the layer attaches, draws,
// and attaches again once the GPU the app gave up for the background is back.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import QuartzCore
import Testing
import UIKit

/// What a present answers once the layer's size is not the one its surface was configured for: Metal never
/// says so itself, so reading it is the only proof the frame went through a surface made from this layer.
private let outdated = CraftError.Gpu(
    detail: "the preview surface changed size or format; it is configured again, so the next frame draws")
private let noSurface = CraftError.Gpu(detail: "no surface")

/// An engine on the GPU, whose active comp is a translucent veil to preview.
private func started() async throws -> Engine {
    let engine = try startEffectsEngine(folders: .isolated(), gpu: .preferred)
    try await engine.veil("Veil", width: 64, height: 36, color: "#c84d21", opacity: 50)
    return engine
}

/// A layer 32 points wide and as high, at twice the scale, as an app's view makes one.
@MainActor
private func layer(side points: CGFloat = 32) -> CAMetalLayer {
    let layer = CAMetalLayer()
    layer.contentsScale = 2
    layer.bounds = CGRect(x: 0, y: 0, width: points, height: points)
    return layer
}

/// Draws on `layer`, which has been resized, and shows the frame went through its surface: the first present
/// after a resize is the outdated answer, and the next draws.
@MainActor
private func drawsOn(_ layer: CAMetalLayer, through present: () async throws -> Void) async throws {
    layer.bounds = CGRect(
        x: 0, y: 0, width: layer.bounds.width > 28 ? 24 : 32, height: layer.bounds.width > 28 ? 24 : 32)
    await #expect(throws: outdated) { try await present() }
    try await present()
}

/// What `call` refuses with, in the words the app's user is shown, or `no error`.
@MainActor
private func words(_ call: () async throws -> Void) async -> String {
    do {
        try await call()
        return "no error"
    } catch {
        return (error as? CraftError)?.errorDescription ?? "\(error)"
    }
}

@Suite @MainActor struct PreviewTests {
    @Test func aPreviewDrawsOnTheAppsLayer() async throws {
        let engine = try await started()
        let target = layer()
        try await engine.attachMetalLayer(target)
        try await engine.present(params: "{}")
        try await drawsOn(target) { try await engine.present(params: "{}") }
        try await engine.shutDown()
    }

    @Test func theLayerIsAttachedAgainAfterTheGpuIsGivenUpAndAskedForAgain() async throws {
        let engine = try await started()
        let target = layer()
        try await engine.attachMetalLayer(target)
        try await engine.present(params: "{}")
        try await engine.suspendGpu()
        await #expect(throws: noSurface) { try await engine.present(params: "{}") }
        #expect(try await engine.resumeGpu() == nil, "the simulator's device is back")
        await #expect(throws: noSurface) { try await engine.present(params: "{}") }
        try await engine.attachMetalLayer(target)
        try await engine.present(params: "{}")
        try await drawsOn(target) { try await engine.present(params: "{}") }
        try await engine.shutDown()
    }

    @Test func aPreviewThroughTheEngineAttachesTheLayerOnceAndAgainAfterTheBackground() async throws {
        let raw = try await started()
        let host = FakeHost()
        let engine = EffectsEngine(engine: raw, host: host)
        let target = layer()
        try await engine.preview(into: target)
        try await engine.preview("Veil", into: target, at: .seconds(0.5))
        try await drawsOn(target) { try await engine.preview(into: target) }

        host.post(UIApplication.didEnterBackgroundNotification)
        #expect(
            await eventually { (try? await raw.present(params: "{}")) == nil }, "the GPU and the surface were given up")
        await #expect(throws: noSurface) { try await raw.present(params: "{}") }

        host.post(UIApplication.willEnterForegroundNotification)
        #expect(
            await eventually { (try? await raw.present(params: "{}")) != nil },
            "the controller attached the layer again")
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil)
        try await drawsOn(target) { try await raw.present(params: "{}") }
        try await raw.shutDown()
    }

    /// The engine trusts the class of the object the address names, so the wrapper takes a `CAMetalLayer`
    /// and nothing wider: the type of the method is pinned, since no call site could show a wider one.
    @Test func theWrapperTakesACAMetalLayerAndNothingWider() {
        let expected = (@Sendable (Engine) -> @MainActor @Sendable (CAMetalLayer) async throws -> Void).self
        #expect(type(of: Engine.attachMetalLayer) == expected)
    }

    /// The address is passed unretained and the engine takes a retain of its own, so once the app has let go
    /// of the layer and the engine is done with it, nothing holds it: a retain the wrapper took on top would
    /// keep it for ever.
    @Test func aLayerTheEngineAndTheAppAreDoneWithIsFreed() async throws {
        let engine = try await started()
        weak var watched: CAMetalLayer?
        do {
            let target = layer()
            watched = target
            try await engine.attachMetalLayer(target)
            try await engine.present(params: "{}")
        }
        try await engine.shutDown()
        #expect(await eventually { watched == nil }, "the layer is still held")
    }

    /// The request is the `frame` call's own: the composition and the time in seconds, each left out when the
    /// caller left it out, so the engine's own defaults, the active comp and its current time, apply.
    @Test func aPreviewAsksForTheCompAndTheTimeItWasGiven() {
        let engine = EffectsEngine(engine: FakeEngine(), host: FakeHost())
        #expect(engine.previewRequest(comp: "Veil", at: .seconds(2)) == ["comp": "Veil", "time": 2])
        #expect(engine.previewRequest(comp: 3, at: .milliseconds(500)) == ["comp": 3, "time": 0.5])
        #expect(engine.previewRequest(comp: nil, at: nil) == [:])
    }

    /// A comp the engine does not have is refused in the `frame` call's words, which name it.
    @Test func aPreviewOfACompTheEngineLacksIsRefusedNamingIt() async throws {
        let raw = try await started()
        let engine = EffectsEngine(engine: raw, host: FakeHost())
        let target = layer()
        let missing = await words { try await engine.preview("Nowhere", into: target) }
        #expect(missing.contains("Nowhere"), "the comp was named in the request: \(missing)")
        try await raw.shutDown()
    }

    /// Ending a preview empties the surface and is not undone by a trip to the background, and the layer is
    /// attached afresh by the next preview.
    @Test func aPreviewEndedIsAttachedAgainByTheNextOne() async throws {
        let raw = try await started()
        let host = FakeHost()
        let engine = EffectsEngine(engine: raw, host: host)
        let target = layer()
        try await engine.preview(into: target)
        try await engine.endPreview()
        await #expect(throws: noSurface) { try await raw.present(params: "{}") }
        try await engine.endPreview()
        await engine.background.enteredBackground()
        await engine.background.returnedToForeground()
        #expect(engine.background.lastFailure == nil)
        await #expect(throws: noSurface) { try await raw.present(params: "{}") }
        try await engine.preview(into: target)
        try await drawsOn(target) { try await raw.present(params: "{}") }
        try await raw.shutDown()
    }

    @Test func theGpuComingBackWithNoLayerShownAttachesNothing() async throws {
        let raw = try await started()
        let host = FakeHost()
        let engine = EffectsEngine(engine: raw, host: host)
        await engine.background.enteredBackground()
        await engine.background.returnedToForeground()
        #expect(engine.background.lastFailure == nil)
        await #expect(throws: noSurface) { try await raw.present(params: "{}") }
        try await raw.shutDown()
    }
}
