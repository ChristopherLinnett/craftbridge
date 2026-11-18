// The layer an engine previews into, and whether the engine still draws on it.

import CraftBridgeFFI
import QuartzCore

/// The layer an app previews an engine into, kept attached.
///
/// The engine reports whether it holds a preview surface (``Engine/hasPreviewSurface()``), and it
/// drops that surface whenever it gives the GPU up (``Engine/suspendGpu()``) or loses the device,
/// so a layer the app attached before the app went to the background draws nothing after
/// ``Engine/resumeGpu()`` until it is attached again. The target remembers the last layer the app
/// named and ``attach(_:)`` attaches it only when the engine is not previewing into it already —
/// because it holds no surface, or because the one it held was not this layer — and
/// ``reattach()`` is how the controller asks for that after the GPU came back. An app that previews
/// through a target never attaches a layer twice itself.
///
/// The target holds the layer weakly: the engine holds a retain of its own while attached, and an
/// app's view is what decides how long its layer lives. Its attaches and its ending take turns, each
/// once the one asked for before it has ended, so two previews into two layers at once leave the engine
/// on the layer the target remembers, whichever reached it last.
@MainActor
public final class PreviewTarget {
    private let engine: Engine
    private let attachLayer: @MainActor (CAMetalLayer) async throws -> Void
    private weak var layer: CAMetalLayer?
    private let turns = Turns()

    /// A target for previews of `engine`.
    public convenience nonisolated init(engine: Engine) {
        self.init(engine: engine) { try await engine.attachMetalLayer($0) }
    }

    /// A target for previews of `engine` that hands a layer it must attach to `attaching`: a test of a call
    /// that previews through a target, whose engine is a stand-in no layer can be attached to, reads what
    /// the call sends the engine without one.
    package nonisolated init(engine: Engine, attaching: @escaping @MainActor (CAMetalLayer) async throws -> Void) {
        self.engine = engine
        attachLayer = attaching
    }

    /// Attaches `layer` unless the engine is already previewing into it, which an app calls before
    /// every preview, so a layer the engine lost is attached again and one it holds costs no attach.
    ///
    /// A layer whose attach failed is remembered all the same, so ``reattach()`` can try it again
    /// once there is a device to preview on.
    ///
    /// - Throws: what ``Engine/attachMetalLayer(_:)`` throws.
    public func attach(_ layer: CAMetalLayer) async throws {
        try await inTurn {
            if try await self.engine.hasPreviewSurface(), self.layer === layer { return }
            self.layer = layer
            try await self.attachLayer(layer)
        }
    }

    /// Attaches the last layer the app named again, unless the engine is previewing into it already.
    /// Nothing to attach when no layer was shown or the app has let it go.
    ///
    /// - Throws: what ``Engine/attachMetalLayer(_:)`` throws.
    public func reattach() async throws {
        guard let layer else { return }
        try await attach(layer)
    }

    /// Ends the preview: the engine gives its surface up, and the next ``attach(_:)`` attaches afresh.
    ///
    /// - Throws: what ``Engine/detachPreview()`` throws.
    public func detach() async throws {
        try await inTurn {
            try await self.engine.detachPreview()
            self.layer = nil
        }
    }

    /// Runs `step` once the target's attaches and ending asked for before it have ended, and throws what
    /// it threw.
    private func inTurn(_ step: @escaping @MainActor () async throws -> Void) async throws {
        let outcome = await turns.run { () async -> Result<Void, any Error> in
            do {
                try await step()
                return .success(())
            } catch {
                return .failure(error)
            }
        }
        try outcome.get()
    }
}
