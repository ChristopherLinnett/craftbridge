// The one place a `CAMetalLayer` becomes the integer the engine's attach takes.

import CraftBridgeFFI
import QuartzCore

extension Engine {
    /// Previews into `layer`: every later `present` of the engine draws its frame there, until
    /// ``Engine/detachPreview()``, another attach, or ``Engine/suspendGpu()``, which drops the surface with
    /// the device. Attaching replaces the layer the engine previewed before. After ``Engine/resumeGpu()``
    /// the app attaches its layer again; ``BackgroundController`` does so for a layer shown through a
    /// ``PreviewTarget``.
    ///
    /// The engine takes a retain of its own on the layer for as long as it is attached, so the app may
    /// let go of the layer first: the engine keeps it, and every present still draws on it, unseen, until
    /// the preview ends. The parameter is typed `CAMetalLayer` because the engine trusts the class of the
    /// object the address names: a layer of another class would be read as a Metal layer.
    ///
    /// - Throws: ``CraftError/Gpu(detail:)`` when the engine holds no device to preview on, and
    ///   ``CraftError/Unsupported(detail:)`` off Apple platforms.
    @MainActor
    public func attachMetalLayer(_ layer: CAMetalLayer) async throws {
        // `passUnretained` hands over no retain: the engine takes its own (plan §20), and the app's layer is
        // kept alive across the call by the lifetime extension, since the address alone keeps nothing.
        let address = UInt64(UInt(bitPattern: Unmanaged.passUnretained(layer).toOpaque()))
        try await CraftBridgeFFI.attachMetalLayer(engine: self, layer: address)
        withExtendedLifetime(layer) {}
    }
}
