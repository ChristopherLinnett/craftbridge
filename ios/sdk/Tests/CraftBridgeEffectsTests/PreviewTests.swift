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

/// The containers of this suite's engines, removed when the suite ends.
private let previewFolders = "preview"

/// An engine on the GPU, whose active comp is a translucent veil to preview.
private func started() async throws -> Engine {
    let engine = try startEffectsEngine(folders: .isolated(previewFolders), gpu: .preferred)
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

@Suite(Removing(name: previewFolders)) @MainActor struct PreviewTests {
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

    /// The engine is the one that reports whether it holds the preview surface: the preview attaches
    /// its layer only when the engine is not already previewing into it, which shows as the first
    /// present finding the resized layer rather than the preview's attach reconfiguring it. A layer
    /// that is not the held one is attached as soon as it is named, and the preview after an ending
    /// attaches afresh.
    @Test func aPreviewAttachesTheLayerOnlyWhileTheEngineDoesNotHoldIt() async throws {
        let raw = try await started()
        let engine = EffectsEngine(engine: raw, host: FakeHost())
        let target = layer()
        #expect(!(try await raw.hasPreviewSurface()), "nothing is held before the first preview")
        try await engine.preview(into: target)
        #expect(try await raw.hasPreviewSurface(), "the preview attached the layer, and the engine holds it")
        try await drawsOn(target) { try await engine.preview(into: target) }
        let second = layer()
        try await engine.preview(into: second)
        try await drawsOn(second) { try await engine.preview(into: second) }
        try await engine.endPreview()
        #expect(!(try await raw.hasPreviewSurface()), "ending the preview gave the surface up")
        await #expect(throws: noSurface) { try await raw.present(params: "{}") }
        try await engine.preview(into: second)
        try await drawsOn(second) { try await engine.preview(into: second) }
        try await raw.shutDown()
    }

    /// Two previews into two layers made at once take turns, so the engine ends on the layer the target
    /// remembers, the one whose turn came last: a preview into that layer then costs no attach and draws
    /// into it, which shows as the raw present after it finding that layer resized.
    @Test func twoPreviewsIntoTwoLayersAtOnceLeaveTheEngineOnTheLayerTheTargetRemembers() async throws {
        let raw = try await started()
        let engine = EffectsEngine(engine: raw, host: FakeHost())
        let (first, second) = (layer(), layer())
        let early = Task { try await engine.preview(into: first) }
        try await engine.preview(into: second)
        try await early.value
        try await engine.preview(into: first)
        try await drawsOn(first) { try await raw.present(params: "{}") }
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
    /// caller left it out, so the engine's own defaults, the active comp and its current time, apply. The time
    /// is the nearest double to the duration's seconds, the one a `frame` call names the moment by and the one
    /// Kotlin's preview asks for, below a millisecond and below zero too, and a duration past what the engine
    /// reaches is still a time, in whole seconds.
    @Test func aPreviewAsksForTheCompAndTheTimeItWasGiven() {
        let engine = EffectsEngine(engine: FakeEngine(), host: FakeHost())
        #expect(engine.previewRequest(comp: "Veil", at: .seconds(2)) == ["comp": "Veil", "time": 2])
        #expect(engine.previewRequest(comp: 3, at: .milliseconds(500)) == ["comp": 3, "time": 0.5])
        #expect(engine.previewRequest(comp: nil, at: nil) == [:])
        #expect(engine.previewRequest(comp: nil, at: .milliseconds(9)) == ["time": 0.009])
        #expect(engine.previewRequest(comp: nil, at: .microseconds(1)) == ["time": 0.000001])
        #expect(engine.previewRequest(comp: nil, at: .nanoseconds(-1_500_000_001)) == ["time": -1.500000001])
        #expect(engine.previewRequest(comp: nil, at: .seconds(0.1)) == ["time": 0.1])
        #expect(engine.previewRequest(comp: nil, at: .seconds(Int64.max)) == ["time": .number(Double(Int64.max))])
    }

    /// What `preview` hands the engine's `present` is the request the call builds, in every spelling of the time:
    /// no second conversion of the duration sits in the call, which only a read of the call's own request shows,
    /// since the conversion above is pinned on the request builder alone.
    @Test func aPreviewHandsTheEngineTheRequestItBuilds() async throws {
        let requests = Requests()
        let fake = FakeEngine(recording: requests)
        let engine = EffectsEngine(
            engine: fake, host: FakeHost(), previewTarget: PreviewTarget(engine: fake) { _ in })
        let target = layer()
        let named: [(Duration?, JSONValue)] = [
            (.milliseconds(9), ["comp": "Veil", "time": 0.009]),
            (.microseconds(1), ["comp": "Veil", "time": 0.000001]),
            (.nanoseconds(-1_500_000_001), ["comp": "Veil", "time": -1.500000001]),
            (.seconds(2), ["comp": "Veil", "time": 2]),
            (nil, ["comp": "Veil"]),
        ]
        for (time, request) in named {
            try await engine.preview("Veil", into: target, at: time)
            #expect(request == engine.previewRequest(comp: "Veil", at: time), "the test expects the builder's request")
            #expect(await requests.all.last?.method == "present")
            #expect(
                await requests.all.last?.params == request,
                "\(String(describing: time)) reached the engine as another time")
        }
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
