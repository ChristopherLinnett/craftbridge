// The entry point of the EffectCraft API. The generated command namespaces in `Generated/` hang off it.

import CraftBridge

/// An EffectCraft engine: the typed command namespaces over its one generic request call.
public final class EffectsEngine: Sendable {
    /// What carries a request to the engine and brings its reply back.
    package typealias Transport = @Sendable (_ method: String, _ params: JSONValue) async throws -> JSONValue

    private let transport: Transport

    /// An engine that sends its requests through `transport`.
    package init(transport: @escaping Transport) {
        self.transport = transport
    }

    /// Sends one request of the engine's method table, such as `"exec"`, and returns its reply. Internal:
    /// the public calls are the typed ones built on it.
    func request(_ method: String, _ params: JSONValue) async throws -> JSONValue {
        try await transport(method, params)
    }

    /// Runs the command `id`. `params` holds the keys the caller typed; `extra` holds keys the type rules
    /// left untyped, which never replace a typed one, so a typed value is not lost to a stray key.
    func command(
        _ id: String,
        _ params: KeyValuePairs<String, JSONValue?>,
        extra: [String: JSONValue]
    ) async throws -> JSONValue {
        var keys = extra
        for (key, value) in params {
            if let value {
                keys[key] = value
            }
        }
        return try await request("exec", ["command": .string(id), "params": .object(keys)])
    }
}
