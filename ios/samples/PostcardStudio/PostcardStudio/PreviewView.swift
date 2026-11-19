// The live preview: an MTKView whose layer the engine draws into.

import MetalKit
import SwiftUI

/// A square of the card's frames. The engine draws into the view's `CAMetalLayer` itself, so the view never
/// draws: it only gives the engine a layer that follows the view's size.
struct PreviewView: UIViewRepresentable {
    /// Called once with the view's layer, which is the target of every preview.
    let onLayer: @MainActor (CAMetalLayer) -> Void

    func makeUIView(context: Context) -> MTKView {
        let view = MTKView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        view.isPaused = true
        view.enableSetNeedsDisplay = false
        if let layer = view.layer as? CAMetalLayer {
            // The view is made inside a view update, which may not change the studio's state.
            Task { @MainActor in onLayer(layer) }
        }
        return view
    }

    func updateUIView(_ view: MTKView, context: Context) {}
}
