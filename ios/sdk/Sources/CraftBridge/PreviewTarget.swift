// The layer an engine previews into, and whether the engine still draws on it.

import CraftBridgeFFI
import QuartzCore

/// The layer an app previews an engine into, kept attached.
///
/// The engine drops its preview surface whenever it gives the GPU up (``Engine/suspendGpu()``), so a layer
/// the app attached before the app went to the background draws nothing after ``Engine/resumeGpu()`` until
/// it is attached again. A target remembers the layer and whether the engine still holds it: ``attach(_:)``
/// attaches only when it must, and ``reattach()`` puts the layer back after the GPU came back.
/// ``BackgroundController`` calls both at the right moments, so an app that previews through a target never
/// attaches a layer twice itself.
///
/// The target holds the layer weakly: the engine holds a retain of its own while attached, and an app's
/// view is what decides how long its layer lives.
@MainActor
public final class PreviewTarget {
    private let engine: Engine
    private weak var layer: CAMetalLayer?
    private var attached = false
    /// Counts the times the engine dropped its surface, so an attach that was already under way when it
    /// dropped it is not taken for one that holds.
    private var drops = 0

    /// A target for previews of `engine`.
    public nonisolated init(engine: Engine) {
        self.engine = engine
    }

    /// Attaches `layer` unless it is the layer the engine already holds, which an app calls before every
    /// preview, so a layer that was dropped is attached again and one that is held costs nothing.
    ///
    /// - Throws: what ``Engine/attachMetalLayer(_:)`` throws.
    public func attach(_ layer: CAMetalLayer) async throws {
        if attached, self.layer === layer { return }
        let before = drops
        attached = false
        self.layer = layer
        try await engine.attachMetalLayer(layer)
        attached = drops == before
    }

    /// Attaches the layer again after the GPU came back, when the engine let it go. Nothing to attach
    /// when no layer was shown or the app has let it go.
    ///
    /// - Throws: what ``Engine/attachMetalLayer(_:)`` throws.
    public func reattach() async throws {
        guard !attached, let layer else { return }
        try await attach(layer)
    }

    /// Records that the engine dropped its surface with its device.
    func surfaceDropped() {
        drops += 1
        attached = false
    }

    /// Ends the preview: the engine gives its surface up, and the next ``attach(_:)`` attaches afresh.
    ///
    /// - Throws: what ``Engine/detachPreview()`` throws.
    public func detach() async throws {
        try await engine.detachPreview()
        drops += 1
        attached = false
        layer = nil
    }
}
