// The entry point of the EffectCraft API. The generated command namespaces in `Generated/` hang off it.

import CraftBridge
import CraftBridgeFFI

/// An EffectCraft engine: the typed command namespaces over its one generic request call.
public final class EffectsEngine: Sendable {
    /// The engine as the UniFFI bindings hand it. Every call of this API is one call on it: the requests
    /// below go through ``request(_:_:)``, an export through ``ExportJob``, a frame and an MCP line
    /// through the bindings' own calls. Named `handle` because the generated namespaces take `engine`.
    let handle: Engine

    /// The layer the engine previews into, attached only when the engine does not hold it already:
    /// ``preview(_:into:at:)`` goes through it.
    let previewTarget: PreviewTarget

    /// What the engine does with the GPU and with a running export when the app leaves the foreground:
    /// the policy ``start(folders:gpu:background:)`` was given, applied for as long as the engine lives.
    public let background: BackgroundController

    /// An engine over the UniFFI handle `engine`. ``start`` makes the handle from the folders and GPU the
    /// app names; tests pass a handle of their own, a host of their own for the system, and a preview target
    /// of their own when the handle is a stand-in no layer can be attached to.
    package init(
        engine: Engine,
        background policy: BackgroundPolicy = .default,
        host: any BackgroundHost = UIKitHost(),
        previewTarget: PreviewTarget? = nil
    ) {
        handle = engine
        let target = previewTarget ?? PreviewTarget(engine: engine)
        self.previewTarget = target
        background = BackgroundController(engine: engine, policy: policy, preview: target, host: host)
    }

    /// Sends one request of the engine's method table, such as `"exec"`, and returns its reply: the generic
    /// door under every typed call that makes such a request. An app prefers the typed calls, whose arguments
    /// name what each request takes. The door is for a binding to another language, which carries every call
    /// as a method's name and its parameters as JSON, so that a method the engine gains needs no new Swift:
    /// `JSONValue` is `Codable`, so the binding reads the parameters from JSON text with `JSONDecoder` and
    /// writes the reply back with `JSONEncoder`. The engine answers each method at one door alone: a method
    /// that reports progress is started through ``job(_:_:progress:)`` and a frame is asked through
    /// ``frame(params:format:)``, and this call refuses both. The package's conformance tests make the
    /// corpus's requests here too, which name the method table's methods, not the typed calls.
    ///
    /// - Throws: the ``CraftError`` the engine answers the request with, which for a method this door does
    ///   not answer is ``CraftError/Unsupported(detail:)``, naming the methods it does;
    ///   ``CraftError/InvalidParams(detail:)`` for parameters that hold NaN or an infinity, which JSON has no
    ///   number for; and ``CraftError/Internal(detail:)`` once the engine has been shut down.
    public func request(_ method: String, _ params: JSONValue) async throws -> JSONValue {
        try JSONValue(
            jsonText: try await handle.request(method: method, params: params.jsonText())
        )
    }

    /// Renders one frame as the engine's `frame` is asked with `params`, that request's own keys, and answers
    /// its bytes laid out as `format` names: the generic door under
    /// ``frame(comp:at:frame:maxSide:scale:transparent:format:)``, which asks it with the keys its arguments
    /// name. An app prefers that typed call. This one is for a binding to another language, which carries the
    /// request's keys as JSON (see ``request(_:_:)``), and the conformance tests ask it with the corpus's own.
    ///
    /// - Throws: the ``CraftError`` the engine refuses the frame with, and, as ``request(_:_:)`` throws them,
    ///   ``CraftError/InvalidParams(detail:)`` for parameters JSON has no number for and
    ///   ``CraftError/Internal(detail:)`` once the engine has been shut down.
    public func frame(params: JSONValue, format: PixelFormat) async throws -> FrameBytes {
        try await handle.frame(params: params.jsonText(), format: format)
    }

    /// Runs the command `id` with the keys the caller typed in `params` over the untyped ones in `extra`,
    /// as ``given(_:over:)`` joins them.
    func command(
        _ id: String,
        _ params: KeyValuePairs<String, JSONValue?>,
        extra: [String: JSONValue]
    ) async throws -> JSONValue {
        try await request("exec", ["command": .string(id), "params": given(params, over: extra)])
    }
}

/// The object of the keys a call was given, over the keys of `extra`: a key left out is not sent, since
/// the engine reads a key it does not find as "not named", which a key named with null is not; and a key
/// of `extra` never replaces a typed one, so a typed value is not lost to a stray key. Every call that
/// joins typed keys and untyped ones joins them here, so they all join them alike.
func given(_ keys: KeyValuePairs<String, JSONValue?>, over extra: [String: JSONValue] = [:]) -> JSONValue {
    var object = extra
    for (key, value) in keys {
        if let value {
            object[key] = value
        }
    }
    return .object(object)
}
