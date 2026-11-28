// What the engine's `previewRequest` answers: the frame a preview draws, fitted to its box, and the size of
// the composition behind it.

import CraftBridge
import CraftBridgeFFI

/// The answer of ``EffectsEngine/previewRequest(_:at:width:height:)``.
public struct PreviewRequest: Sendable, Hashable {
    /// The `frame` request that draws the composition with its own alpha, as large as fits the box and no
    /// larger than the composition's pixels. Pass it to ``EffectsEngine/frame(params:format:)``.
    public let request: JSONValue

    /// ``request`` as the JSON text the engine wrote it in, which ``EffectsEngine/frameJSON(params:format:)``
    /// takes: the one text Android and iOS answer for the same composition, time and box, since the engine
    /// writes it and neither platform writes it again.
    public let requestText: String

    /// The composition's width in pixels, which sets the width of the texture or buffer a frame is drawn into.
    public let contentWidth: Int

    /// The composition's height in pixels, which sets the height of the texture or buffer a frame is drawn into.
    public let contentHeight: Int

    /// The answer the engine's `previewRequest` wrote.
    ///
    /// - Throws: ``CraftError/Internal(detail:)`` for a document that is not the answer's shape, which no
    ///   engine of this build writes.
    package init(answer: JSONValue) throws {
        guard case .object(let fields) = answer,
            let request = fields["request"], case .object = request,
            case .string(let requestText) = fields["requestText"],
            case .number(let width) = fields["contentWidth"], let contentWidth = Int(exactly: width),
            case .number(let height) = fields["contentHeight"], let contentHeight = Int(exactly: height)
        else {
            throw CraftError.Internal(
                detail: "the engine's `previewRequest` answered a document without a request and a pixel size")
        }
        self.request = request
        self.requestText = requestText
        self.contentWidth = contentWidth
        self.contentHeight = contentHeight
    }
}
