// The entry point of the EffectCraft API. The generated command namespaces in `Generated/` hang off it.

import CraftBridge
import CraftBridgeFFI

/// An EffectCraft engine: the typed command namespaces over its one generic request call.
public final class EffectsEngine: Sendable {
    /// The engine as the UniFFI bindings hand it. Every call of this API is one call on it: the requests
    /// below go through ``requestJSON(_:params:)`` (``request(_:_:)`` is built on it), an export through
    /// ``ExportJob``, a frame and an MCP line through the bindings' own calls. Named `handle` because the
    /// generated namespaces take `engine`.
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
    /// name what each request takes. A binding to another language, which carries every call as a method's
    /// name and its parameters as JSON so that a method the engine gains needs no new Swift, asks
    /// ``requestJSON(_:params:)`` instead, the same door in JSON text. The engine answers each method at one
    /// door alone: a method that reports progress is started through ``job(_:_:progress:)`` and a frame is
    /// asked through ``frame(params:format:)``, and this call refuses both. The package's conformance tests
    /// make the corpus's requests here too, which name the method table's methods, not the typed calls.
    ///
    /// - Throws: the ``CraftError`` the engine answers the request with, which for a method this door does
    ///   not answer is ``CraftError/Unsupported(detail:)``, naming the methods it does;
    ///   ``CraftError/InvalidParams(detail:)`` for parameters that hold NaN or an infinity, which JSON has no
    ///   number for; and ``CraftError/Internal(detail:)`` once the engine has been shut down.
    public func request(_ method: String, _ params: JSONValue) async throws -> JSONValue {
        try JSONValue(jsonText: try await requestJSON(method, params: params.jsonText()))
    }

    /// Sends one request of the engine's method table with its parameters as JSON text, and returns the
    /// engine's reply as the text it wrote: the one door to the engine's requests, which ``request(_:_:)``
    /// and every typed call that makes a request are built on. An app prefers those, whose arguments are
    /// typed; this form is for a binding to another language, which carries every call as a method's name
    /// and JSON text. The parameters reach the engine exactly as given, unparsed on the Swift side, and the
    /// reply comes back exactly as the engine wrote it, which a `JSONValue` cannot promise. Its number is a
    /// `Double`, so a whole number the engine wrote as `2.0` is written back as `2`, which a reader that tells
    /// the two apart, as Dart's does, holds as an `int` where Kotlin's door hands it a `double`, and one above
    /// 2^53 would lose digits. Its object is a `Dictionary`, so keys the engine writes sorted come back in an
    /// order that changes from one process to the next. Nor is the reply built as a tree of values and
    /// written a second time, which a phone with little memory feels.
    ///
    /// As with ``request(_:_:)``, the method is one the table answers at this door: a method that reports
    /// progress is started through ``jobJSON(_:params:progress:)``.
    ///
    /// - Throws: the ``CraftError`` the engine answers the request with, which for a method this door does
    ///   not answer is ``CraftError/Unsupported(detail:)``; ``CraftError/InvalidParams(detail:)`` for
    ///   `params` that is not JSON, which the engine reads and refuses, as Kotlin's `Engine.request` does;
    ///   and ``CraftError/Internal(detail:)`` once the engine has been shut down.
    public func requestJSON(_ method: String, params: String) async throws -> String {
        try await handle.request(method: method, params: params)
    }

    /// Renders one frame as the engine's `frame` is asked with `params`, that request's own keys, and answers
    /// its bytes laid out as `format` names: the generic door under
    /// ``frame(comp:at:frame:maxSide:scale:transparent:format:)``, which asks it with the keys its arguments
    /// name. An app prefers that typed call. A binding to another language, which carries the request's keys
    /// as JSON (see ``requestJSON(_:params:)``), asks ``frameJSON(params:format:)``, and the conformance
    /// tests ask this one with the corpus's own.
    ///
    /// - Throws: the ``CraftError`` the engine refuses the frame with, and, as ``request(_:_:)`` throws them,
    ///   ``CraftError/InvalidParams(detail:)`` for parameters JSON has no number for and
    ///   ``CraftError/Internal(detail:)`` once the engine has been shut down.
    public func frame(params: JSONValue, format: PixelFormat) async throws -> FrameBytes {
        try await frameJSON(params: params.jsonText(), format: format)
    }

    /// Renders one frame as the engine's `frame` is asked with `params`, that request's own keys as JSON
    /// text, which reach the engine exactly as given, and answers its bytes laid out as `format` names: the
    /// one door to a frame, which ``frame(params:format:)`` is built on. An app prefers the typed call
    /// ``frame(comp:at:frame:maxSide:scale:transparent:format:)``; this form is for a binding to another
    /// language, which carries the request as JSON text (see ``requestJSON(_:params:)``).
    ///
    /// - Throws: the ``CraftError`` the engine refuses the frame with, ``CraftError/InvalidParams(detail:)``
    ///   for `params` that is not JSON, and ``CraftError/Internal(detail:)`` once the engine has been shut
    ///   down.
    public func frameJSON(params: String, format: PixelFormat) async throws -> FrameBytes {
        try await handle.frame(params: params, format: format)
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
